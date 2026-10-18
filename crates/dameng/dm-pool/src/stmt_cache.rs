//! 语句缓存：每条连接一份的 LRU 预处理语句表（设计方案 §5.4 / CAP-C-11）。
//!
//! ## 为什么这一层在 `dm-pool` 而不是 `dm-odbc`
//!
//! 「复用预处理语句」看起来像驱动层的事，但它在驱动层**没法表达**：
//!
//! - `Backend` 是对象安全的（`crates/dm-core/src/backend.rs` 的
//!   `fn assert_object_safe(_: &mut dyn Backend<Stmt = MockStmt>)` 就是 T2.4 的落地判据），
//!   而「借出一条语句、跑完再还回来」要求调用方同时持有 `&mut B` 与 `&mut B::Stmt`；
//!   这条路只能靠泛型闭包，泛型方法会把对象安全性直接打掉；
//! - 缓存是**每条物理连接**的资源，而连接的生命周期由池持有（`ConnInner`）。
//!   放在池里，缓存随连接创建/销毁/复位，天然与借还、取消、毒化这些路径对齐。
//!
//! 还有一个实打实的好处：`dm-pool` 的假后端也能用上同一个缓存，于是
//! 「同一条 SQL 只 `prepare` 一次」可以在**离线**用例里断言（`tests/stmt_cache.rs`），
//! 不必依赖真机。
//!
//! ## 语义
//!
//! - 键是 **SQL 文本本身**（精确匹配）。不做归一化：`where id = 1` 与 `where id = 2`
//!   是两条不同的语句（参数化应由调用方用 `?` 完成），这也让「缓存命中」这件事
//!   可预测——不会出现「看起来一样、其实被合并」的意外。
//! - 命中即把条目**取出来**（移出表），跑完再由调用方放回，放回即成为最近使用。
//!   取出来而不是借用，是为了让「执行失败就丢掉这条语句」这件事不需要在
//!   借用状态下做二次 `remove`（ODBC 语句在错误之后状态未知，不该再复用）。
//! - 超出容量按 LRU 淘汰队尾。

use std::collections::VecDeque;

/// 语句缓存的观测数据（`PoolGuard::stmt_cache_stats`）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StmtCacheStats {
    /// 命中次数：复用了已缓存的语句句柄（省掉一次 `SQLPrepare`）。
    pub hits: u64,
    /// 未命中次数：新建了一条语句。
    pub misses: u64,
    /// 因容量上限被淘汰的条数。
    pub evictions: u64,
    /// 当前缓存条数。
    pub size: usize,
    /// 容量上限（`0` 表示缓存关闭）。
    pub capacity: usize,
}

/// 一条缓存项：SQL 文本（键）+ 语句句柄。
pub(crate) struct StmtEntry<S> {
    sql: String,
    stmt: S,
}

impl<S> StmtEntry<S> {
    pub(crate) fn new(sql: String, stmt: S) -> Self {
        Self { sql, stmt }
    }

    /// 取出语句给闭包用。
    pub(crate) fn stmt_mut(&mut self) -> &mut S {
        &mut self.stmt
    }

    /// 放回缓存之前问一次「这条语句还能不能执行」（`Backend::statement_reusable`）。
    pub(crate) fn stmt(&self) -> &S {
        &self.stmt
    }
}

impl<S> std::fmt::Debug for StmtEntry<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StmtEntry").field("sql", &self.sql).finish()
    }
}

/// 每条连接一份的 LRU 语句缓存。
///
/// `S` 是后端的语句类型（`Backend::Stmt`）。**不做任何 IO**：全部动作都是
/// 表操作，所以它可以安全地藏在 `Mutex` 里被阻塞闭包短锁持有（§5.8）。
pub(crate) struct StmtCache<S> {
    capacity: usize,
    /// 队首 = 最近使用，队尾 = 最久未用。
    entries: VecDeque<StmtEntry<S>>,
    hits: u64,
    misses: u64,
    evictions: u64,
}

impl<S> StmtCache<S> {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: VecDeque::new(),
            hits: 0,
            misses: 0,
            evictions: 0,
        }
    }

    /// 缓存是否开启（`stmt_cache_size == 0` 即关闭）。
    pub(crate) fn is_enabled(&self) -> bool {
        self.capacity > 0
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// 命中则取出条目（记一次 `hits`），未命中返回 `None`（**不记账**：
    /// 未命中由调用方在真正 `prepare` 之后用 [`StmtCache::note_miss`] 记，
    /// 因为「查了表但没有」与「压根没查（缓存关闭）」是两件事）。
    pub(crate) fn take(&mut self, sql: &str) -> Option<StmtEntry<S>> {
        let pos = self.entries.iter().position(|e| e.sql == sql)?;
        self.hits += 1;
        self.entries.remove(pos)
    }

    /// 记一次未命中。
    pub(crate) fn note_miss(&mut self) {
        self.misses += 1;
    }

    /// 放回缓存（成为最近使用）。超出容量时从队尾淘汰。
    ///
    /// 容量为 `0` 时**直接丢掉**条目：关闭缓存必须真的不攒句柄。
    pub(crate) fn put(&mut self, entry: StmtEntry<S>) {
        if !self.is_enabled() {
            return;
        }
        // 同一个 SQL 不该有两份（正常路径不会发生：`take` 已经把它移走了；
        // 这里是防御，免得将来多一条调用路径就攒出重复句柄）。
        if let Some(pos) = self.entries.iter().position(|e| e.sql == entry.sql) {
            self.entries.remove(pos);
        }
        self.entries.push_front(entry);
        while self.entries.len() > self.capacity {
            self.entries.pop_back();
            self.evictions += 1;
        }
    }

    /// 全部丢掉（连接复位时用：复位是「把连接拉回干净状态」，
    /// 缓存的语句句柄未必还带着干净的游标状态，全丢最省事也最安全）。
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(crate) fn stats(&self) -> StmtCacheStats {
        StmtCacheStats {
            hits: self.hits,
            misses: self.misses,
            evictions: self.evictions,
            size: self.len(),
            capacity: self.capacity,
        }
    }
}

impl<S> std::fmt::Debug for StmtCache<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StmtCache")
            .field("capacity", &self.capacity)
            .field("size", &self.entries.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(sql: &str) -> StmtEntry<u32> {
        StmtEntry::new(sql.to_owned(), sql.len() as u32)
    }

    /// 关闭（容量 0）：查也不查、攒也不攒。
    #[test]
    fn a_disabled_cache_never_stores_anything() {
        let mut c = StmtCache::new(0);
        assert!(!c.is_enabled());
        assert!(c.take("select 1").is_none());
        c.put(entry("select 1"));
        assert_eq!(c.len(), 0, "容量 0 必须真的丢掉句柄");
        let s = c.stats();
        assert_eq!(s.hits, 0);
        assert_eq!(s.misses, 0, "没查表就不该记未命中");
        assert_eq!(s.capacity, 0);
    }

    /// 命中计数与「取出来再放回」的往返。
    #[test]
    fn hit_and_miss_are_counted_and_the_entry_round_trips() {
        let mut c = StmtCache::new(4);
        assert!(c.take("select 1").is_none());
        c.note_miss();
        c.put(entry("select 1"));
        assert_eq!(c.len(), 1);

        let mut got = c.take("select 1").expect("第二次应该命中");
        assert_eq!(*got.stmt_mut(), 8, "取回的必须是原来那条语句");
        assert_eq!(c.len(), 0, "命中即移出，等着调用方放回");
        c.put(got);

        let s = c.stats();
        assert_eq!(s.hits, 1);
        assert_eq!(s.misses, 1);
        assert_eq!(s.size, 1);
        assert_eq!(s.capacity, 4);
    }

    /// SQL 文本是精确键：不同的文本互不命中。
    #[test]
    fn the_key_is_the_exact_sql_text() {
        let mut c = StmtCache::new(4);
        c.put(entry("select 1"));
        assert!(c.take("select 1 ").is_none(), "多一个空格就是另一条语句");
        assert!(c.take("SELECT 1").is_none(), "大小写不归一化");
        assert!(c.take("select 1").is_some());
    }

    /// 超容量按 LRU 淘汰：最近用过的留下，最久没用的先走。
    #[test]
    fn the_least_recently_used_entry_is_evicted_first() {
        let mut c = StmtCache::new(2);
        c.put(entry("a"));
        c.put(entry("b"));
        // 用一下 a：它应当变成最近使用，于是下一个被淘汰的是 b。
        let a = c.take("a").expect("a 在表里");
        c.put(a);
        c.put(entry("c"));

        assert_eq!(c.len(), 2);
        assert!(c.take("a").is_some(), "刚用过的 a 应该还在");
        assert!(c.take("b").is_none(), "b 被淘汰了");
        assert_eq!(c.stats().evictions, 1);
    }

    /// 同一个 SQL 被放回两次也只留一条（防御性断言）。
    #[test]
    fn a_duplicate_put_does_not_grow_the_table() {
        let mut c = StmtCache::new(4);
        c.put(entry("a"));
        c.put(entry("a"));
        assert_eq!(c.len(), 1);
    }

    /// 复位路径会 `clear`：条数归零，但累计计数保留（历史账不能丢）。
    #[test]
    fn clear_drops_the_entries_but_keeps_the_counters() {
        let mut c = StmtCache::new(4);
        c.put(entry("a"));
        let _ = c.take("a").unwrap();
        c.note_miss();
        c.put(entry("a"));
        c.clear();

        assert_eq!(c.len(), 0);
        let s = c.stats();
        assert_eq!(s.size, 0);
        assert_eq!(s.hits, 1);
        assert_eq!(s.misses, 1);
    }
}
