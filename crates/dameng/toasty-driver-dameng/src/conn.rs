//! `DamengConnection`：`toasty_core::driver::Connection` 的达梦实现（T8.5）。
//!
//! # 这一层在做什么
//!
//! 引擎把**已 lower 的 `stmt::Statement` AST**（标量值已换成 `Expr::Arg(n)`）交给我们，
//! 驱动负责三件事：
//!
//! 1. **渲染**：`toasty_sql::Serializer` 把 AST 渲染成 SQL。用的是 **vendored fork**
//!    里新增的 `Serializer::dameng`（§9.8）：双引号标识符、`IDENTITY(1, 1)`、
//!    `CLOB`/`VARBINARY(n)`/`NUMBER` 等达梦原生类型名，`?` 占位符与 MySQL 一致。
//! 2. **绑参**：`Expr::Arg(n)` 是**位置索引**，而 DM 的 ODBC 绑参是 `?`（无索引）——
//!    `serialize_with_arg_order` 给出 `n` 在 SQL 里出现的顺序，据此把参数重排一遍。
//!    同一个 `Arg(n)` 可以出现多次（`?` 不能重复引用），所以用「引用计数 + 最后一次 take」
//!    的办法展开（照抄上游 MySQL 驱动 `lib.rs:440-472` 的做法）。
//! 3. **解码**：`QuerySql.ret` / `RawSqlRet` / `Insert.ret` 指明要还原成哪些
//!    `stmt::Type`；`dm-query` 已经把 ODBC 缓冲解成 `DmValue`，这里按目标类型再解一层。
//!
//! # 会话与事务
//!
//! 事务把「连接的所有权」交给 `dm_query::Transaction`（设计上就是吃掉的），所以本类型
//! 用 [`SessionKind`] 表示「空闲连接 / 在事务里」两种状态；事务结束后连接归还池子，
//! 下一次操作再借一条（`ensure_session`）。达梦的 `SQL_ATTR_AUTOCOMMIT` 复位、保存点
//! 丢事务的坑都由 `dm-query` 那层兜住（§6.6、§12.2）。
//!
//! # `is_valid()` / `ping()` 的硬契约
//!
//! `is_valid()` 必须**廉价且不做 IO**（池在每次回收与每次 `exec` 之后都会问一次）；
//! 本类型用一个 `bool` 记住「这条连接上是否出现过连接级故障」。`ping()` 的失败
//! **必须**归成 `Error::connection_lost`，否则死连接会被放回空闲集（§9.5）。

use std::sync::Arc;

use async_trait::async_trait;
use dm_core::DmValue;
use dm_error::DmError;
use dm_pool::DmPool;
use dm_query::{PooledConn, Row, RowSet, Transaction};
use toasty_core::{
    driver::{
        Connection, ExecResponse,
        log::{QueryLog, QueryLogConfig},
        operation::{Operation, RawSqlRet, Transaction as TxOp, TransactionMode, TypedValue},
    },
    schema::{
        Schema,
        db::{self, AppliedMigration, Migration},
    },
    stmt,
};

use crate::capability;
use crate::error::{to_toasty, to_toasty_with_sql, unsupported_operation};
use crate::value;

/// 查询日志里的 `db.system` 值（与 `Capability.driver_name` 一致）。
pub const DRIVER_NAME: &str = "dameng";

/// 迁移记账表名（DM 会把未加引号的标识符折成大写）。
const MIGRATIONS_TABLE: &str = "__toasty_migrations";

/// 「这次执行要什么返回」——把上游三种 `ret` 形态归一。
#[derive(Debug, Clone)]
enum Ret {
    /// 只关心影响行数。
    Count,
    /// 按这些类型解码结果行。
    Types(Vec<stmt::Type>),
    /// 按 ODBC 元数据推断类型（`RawSqlRet::Infer`）。
    Infer,
    /// 单行 insert 的自增主键：没有 `RETURNING`，只能回查（`SELECT @@IDENTITY`）。
    GeneratedId(stmt::Type),
}

/// 连接当前的状态：握手完的空闲连接，或者一个开着的事务。
#[derive(Debug)]
enum SessionKind {
    Idle(PooledConn),
    Txn(Transaction),
}

/// 三种保存点操作（`Transaction::Savepoint` / `ReleaseSavepoint` / `RollbackToSavepoint`）。
#[derive(Debug, Clone, Copy)]
enum SavepointOp {
    Create,
    Release,
    RollbackTo,
}

/// 一条到 DM 的逻辑连接（`Box<dyn Connection>` 的具体类型）。
#[derive(Debug)]
pub struct DamengConnection {
    /// 池的共享句柄：事务提交后连接归还池子，下一次操作要再借一条。
    pool: Arc<DmPool>,
    /// `None` 表示当前没有借出的连接（事务刚结束，或还没借）。
    session: Option<SessionKind>,
    /// 引擎下发的查询日志配置（`ConnectContext.query_log`）。
    query_log: QueryLogConfig,
    /// 出现过连接级故障后置 `false`，让池把这条连接丢弃。
    valid: bool,
}

impl DamengConnection {
    /// 新建一条逻辑连接。`pool` 是共享的自研连接池。
    #[must_use]
    pub fn new(pool: Arc<DmPool>, query_log: QueryLogConfig) -> Self {
        Self {
            pool,
            session: None,
            query_log,
            valid: true,
        }
    }

    /// 把 `DmError` 翻成 Toasty 错误，并记住「连接已经不可复用」。
    fn record_err(&mut self, err: DmError) -> toasty_core::Error {
        if err.is_connection_lost() {
            self.valid = false;
        }
        to_toasty(err)
    }

    /// 同 [`Self::record_err`]，额外把**触发错误的语句**带进诊断链。
    ///
    /// 达梦的失败消息常常只有「SQL 语法错误」这种级别（`diags` 里才有 `SQLSTATE` 与
    /// 原生码），没有语句文本根本定位不到是哪一条 DDL（T8.1a 的教训）。
    fn record_err_with_sql(&mut self, err: DmError, sql: &str) -> toasty_core::Error {
        if err.is_connection_lost() {
            self.valid = false;
        }
        to_toasty_with_sql(err, sql)
    }

    /// 确保手上有一条连接（用完即还的语义由池子负责）。
    async fn ensure_session(&mut self) -> toasty_core::Result<()> {
        if self.session.is_none() {
            let acquired = PooledConn::acquire(&self.pool).await;
            let conn = match acquired {
                Ok(conn) => conn,
                Err(err) => return Err(self.record_err(err)),
            };
            self.session = Some(SessionKind::Idle(conn));
        }
        Ok(())
    }

    /// 在「当前会话」（空闲连接或事务）上执行一条语句，返回影响行数。
    async fn execute_sql(&mut self, sql: &str, args: &[DmValue]) -> toasty_core::Result<u64> {
        trace_sql(sql, args);
        self.ensure_session().await?;
        let result = match self.session.as_mut().expect("ensure_session 刚保证有连接") {
            SessionKind::Idle(conn) => conn.execute(sql, args).await,
            SessionKind::Txn(txn) => txn.execute(sql, args).await,
        };
        match result {
            Ok(rows) => Ok(rows),
            Err(err) => Err(self.record_err_with_sql(err, sql)),
        }
    }

    /// 在「当前会话」上跑一条查询。
    async fn query_sql(&mut self, sql: &str, args: &[DmValue]) -> toasty_core::Result<RowSet> {
        trace_sql(sql, args);
        self.ensure_session().await?;
        let result = match self.session.as_mut().expect("ensure_session 刚保证有连接") {
            SessionKind::Idle(conn) => conn.query(sql, args).await,
            SessionKind::Txn(txn) => txn.query(sql, args).await,
        };
        match result {
            Ok(rowset) => Ok(rowset),
            Err(err) => Err(self.record_err_with_sql(err, sql)),
        }
    }

    /// 跑一条查询并只取第一行（`SELECT @@IDENTITY`、`SELECT 1` 这类）。
    async fn query_one_sql(
        &mut self,
        sql: &str,
        args: &[DmValue],
    ) -> toasty_core::Result<Option<Row>> {
        let rowset = self.query_sql(sql, args).await?;
        Ok(rowset.first().cloned())
    }

    /// 把参数按 `?` 的出现顺序展开成 `Vec<DmValue>`。
    ///
    /// `arg_order[k] = n` 表示「SQL 里第 k 个 `?` 绑的是 `params[n]`」；同一个 `n` 出现
    /// 多次时，前几次 clone、最后一次 take（上游 MySQL 驱动的做法，`lib.rs:440-472`）。
    fn expand_params(
        params: &[TypedValue],
        arg_order: &[usize],
    ) -> toasty_core::Result<Vec<DmValue>> {
        let mut remaining = vec![0usize; params.len()];
        for &pos in arg_order {
            let slot = remaining.get_mut(pos).ok_or_else(|| {
                toasty_core::Error::invalid_statement(format!(
                    "渲染器引用了不存在的参数序号 {pos}（只有 {} 个参数）",
                    params.len()
                ))
            })?;
            *slot += 1;
        }

        let encoded: Vec<DmValue> = value::encode_params(params)?;

        let mut args: Vec<DmValue> = Vec::with_capacity(arg_order.len());
        let mut used: Vec<DmValue> = encoded;
        for &pos in arg_order {
            remaining[pos] -= 1;
            let value = if remaining[pos] == 0 {
                std::mem::replace(&mut used[pos], DmValue::Null)
            } else {
                used[pos].clone()
            };
            args.push(value);
        }
        Ok(args)
    }

    /// 渲染 AST → SQL，并执行。
    async fn exec_stmt(
        &mut self,
        db_schema: &toasty_core::schema::db::Schema,
        stmt: &stmt::Statement,
        params: &[TypedValue],
        ret: Ret,
    ) -> toasty_core::Result<ExecResponse> {
        // 渲染用 fork 出来的达梦方言（`?` 占位符 + 双引号标识符 + 达梦类型名，§9.8）。
        let sql_stmt = toasty_sql::stmt::Statement::from(stmt.clone());
        let serializer = toasty_sql::Serializer::dameng(db_schema);
        let (sql, arg_order) = serializer.serialize_with_arg_order(&sql_stmt);
        let args = Self::expand_params(params, &arg_order)?;
        let logged: Vec<stmt::Value> = if self.query_log.params {
            params.iter().map(|p| p.value.clone()).collect()
        } else {
            Vec::new()
        };
        self.run(&sql, &args, ret, &logged).await
    }

    /// 执行已经渲染好的 SQL（`RawSql` 走这条）。
    async fn exec_raw(
        &mut self,
        sql: &str,
        params: &[TypedValue],
        ret: RawSqlRet,
        logged: Vec<stmt::Value>,
    ) -> toasty_core::Result<ExecResponse> {
        let ret = match ret {
            RawSqlRet::None => Ret::Count,
            RawSqlRet::Infer => Ret::Infer,
            RawSqlRet::Types(types) => Ret::Types(types),
        };
        let args: Vec<DmValue> = value::encode_params(params)?;
        self.run(sql, &args, ret, &logged).await
    }

    /// 真正落到中间件上的一步：执行 + 解码 + 查询日志。
    async fn run(
        &mut self,
        sql: &str,
        args: &[DmValue],
        ret: Ret,
        logged_params: &[stmt::Value],
    ) -> toasty_core::Result<ExecResponse> {
        let mut log = QueryLog::sql(&self.query_log, DRIVER_NAME, sql, logged_params.iter());
        let result = match &ret {
            Ret::Count => match self.execute_sql(sql, args).await {
                Ok(rows) => {
                    log.rows(rows);
                    Ok(ExecResponse::count(rows))
                }
                Err(err) => Err(err),
            },
            Ret::GeneratedId(ty) => match self.execute_sql(sql, args).await {
                // 达梦没有 `RETURNING`（`returning_from_insert = false`），单行 insert 的自增
                // 主键只能回查；`@@IDENTITY` 是会话级的「最近一次自增」。⚠️ 待真机确认（V-29）。
                Ok(_) => match self.query_one_sql("SELECT @@IDENTITY", &[]).await {
                    Ok(Some(row)) => {
                        let raw = row.values().first().cloned().unwrap_or(DmValue::Null);
                        match value::decode(&raw, ty) {
                            Ok(id) => {
                                log.rows(1);
                                Ok(ExecResponse::value_stream(stmt::ValueStream::from_vec(
                                    vec![record(vec![id])],
                                )))
                            }
                            Err(err) => Err(err),
                        }
                    }
                    Ok(None) => Err(toasty_core::Error::invalid_result(format!(
                        "单行 insert 之后 `SELECT @@IDENTITY` 没有返回任何行（目标类型 {ty:?}）"
                    ))),
                    Err(err) => Err(err),
                },
                Err(err) => Err(err),
            },
            Ret::Types(types) => match self.query_sql(sql, args).await {
                Ok(rowset) => decode_rowset(rowset, types),
                Err(err) => Err(err),
            },
            Ret::Infer => match self.query_sql(sql, args).await {
                Ok(rowset) => infer_rowset(rowset),
                Err(err) => Err(err),
            },
        };
        log.finish(&result);
        result
    }

    /// 执行一条 insert 写入，必要时用 `SET IDENTITY_INSERT <表> ON/OFF` 包一层。
    ///
    /// 自增列要写显式值时达梦强制要求这个开关（真机探针 15k–15p：不开就报
    /// `[HY000/-2723] 仅当指定列列表，且SET IDENTITY_INSERT为ON时，才能对自增列赋值`）。
    /// `OFF` 失败不改写入结果，但必须留痕：否则这条连接上后续的 insert 会一直处在
    /// `IDENTITY_INSERT` 打开的状态。
    async fn run_insert(
        &mut self,
        db: &db::Schema,
        write: &stmt::Statement,
        params: &[TypedValue],
        ret: Ret,
        identity: Option<&str>,
    ) -> toasty_core::Result<ExecResponse> {
        if let Some(table) = identity {
            self.execute_sql(&format!("SET IDENTITY_INSERT {table} ON"), &[])
                .await?;
        }
        let written = self.exec_stmt(db, write, params, ret).await;
        if let Some(table) = identity {
            self.identity_insert_off(table).await;
        }
        written
    }

    /// 关闭 `IDENTITY_INSERT`。失败**不改**写入结果，但必须留痕：否则这条连接上后续的
    /// insert 会一直处在打开状态（达梦的自增列显式赋值会被静默允许）。
    async fn identity_insert_off(&mut self, table: &str) {
        let off = format!("SET IDENTITY_INSERT {table} OFF");
        if let Err(err) = self.execute_sql(&off, &[]).await {
            tracing::warn!(target: DRIVER_NAME, "SET IDENTITY_INSERT … OFF 失败：{err}");
        }
    }

    /// 模拟 `INSERT … RETURNING`：写入之后按**回查键**把投影读回来（达梦没有 `RETURNING`）。
    ///
    /// 回查键 = 主键（普通 insert 与主键目标的 upsert），或 **upsert 的冲突目标列**
    /// （唯一键目标：命中冲突时保留的是**已有行的主键**，按主键回查会 0 行；Round 28）。
    ///
    /// 接受「非空 `VALUES` 字面值」形状（单行、多行都行）——这正是上游 `create!` /
    /// `upsert_by_*` / 批量 `create!` 的形状；其它形状（`SELECT` 源、`DEFAULT VALUES`）如实报
    /// 不支持，**不猜** SQL（§15.11）。
    ///
    /// 多行时**逐行写入**，两个理由：①自增主键只能按行回查 `@@IDENTITY`；②返回顺序必须与输入
    /// 顺序一致，而「一条 `OR` 链 + 无 `ORDER BY`」的 `SELECT` 行序不可依赖。
    async fn insert_and_reload(
        &mut self,
        db: &db::Schema,
        write: &stmt::Statement,
        params: &[TypedValue],
        returning: &stmt::Returning,
        types: &[stmt::Type],
        identity: Option<&str>,
    ) -> toasty_core::Result<ExecResponse> {
        let insert = write
            .as_insert()
            .expect("`Operation::Insert` 携带的一定是 `Statement::Insert`");
        let target = insert.target.as_table_unwrap();
        let table = db.table(target.table);

        let rows: Vec<stmt::ExprRecord> = match &insert.source.body {
            stmt::ExprSet::Values(values) if !values.rows.is_empty() => values
                .rows
                .iter()
                .map(|row| row.as_record_unwrap().clone())
                .collect(),
            other => {
                return Err(toasty_core::Error::unsupported_feature(format!(
                    "达梦的 `INSERT … RETURNING` 模拟只支持「非空 `VALUES` 字面值」插入；表 {} 的源是 {other:?}",
                    table.name
                )));
            }
        };

        let pk_columns: Vec<&db::Column> = table.primary_key_columns().collect();

        // 回查用哪几列做键（Round 28）：
        //   - 普通 insert / **主键目标**的 upsert ⇒ 主键（旧行为不变）；
        //   - **唯一键目标**的 upsert ⇒ 冲突目标列。命中冲突时保留下来的是**已有行的主键**，
        //     而写入语句里带的是本次源行的主键（`upsert_by_email` 那种新生成的 UUID）⇒
        //     按主键回查必然 0 行。冲突目标列一定出现在插入列列表里（达梦的 `ON (...)`
        //     只引用 `USING` 源列），所以键值能从源行直接取到。
        let (reload_columns, key_label): (Vec<&db::Column>, &str) = match &insert.upsert {
            Some(upsert) => match &upsert.target {
                stmt::UpsertTarget::Columns(columns) => (
                    columns
                        .iter()
                        .map(|column_id| table.column(*column_id))
                        .collect(),
                    "冲突目标列",
                ),
                other => {
                    return Err(toasty_core::Error::unsupported_feature(format!(
                        "达梦的 `INSERT … RETURNING` 模拟要求 upsert 的冲突目标已 lowering 成列；表 {} 的目标是 {other:?}",
                        table.name
                    )));
                }
            },
            None => (pk_columns.clone(), "主键"),
        };

        if reload_columns.is_empty() {
            return Err(toasty_core::Error::unsupported_feature(format!(
                "达梦的 `INSERT … RETURNING` 模拟要靠主键回查，但表 {} 没有主键",
                table.name
            )));
        }

        // `or_ignore()`（`UpsertAction::Ignore`）的语义是「冲突就什么都不做，而且**不算
        // 写过**」：上游用 `assert_none!(…or_ignore().exec())` 要求这种情形返回**空结果**。
        // fork 把 Ignore 渲染成「命中分支条件恒假的 MERGE」⇒ 命中已有行时受影响行数是
        // **0**、真插进去时是 **1**，驱动据此决定这一行要不要回查（真机探针 16m/16n）。
        let ignore = insert
            .upsert
            .as_deref()
            .is_some_and(|upsert| matches!(upsert.action, stmt::UpsertAction::Ignore));

        let mut result = Vec::with_capacity(rows.len());
        for row in &rows {
            // 本行回查键的值：能从写入语句里取就取；取不到的（自增列）写完回查 `@@IDENTITY`。
            let mut keys: Vec<Option<TypedValue>> = reload_columns
                .iter()
                .map(|column| insert_column_value(insert, row, column, params))
                .collect();
            let missing: Vec<usize> = keys
                .iter()
                .enumerate()
                .filter(|(_, key)| key.is_none())
                .map(|(index, _)| index)
                .collect();

            let single = single_row_insert(write, row);
            match missing.as_slice() {
                [] => {
                    let written = self
                        .run_insert(db, &single, params, Ret::Count, identity)
                        .await?;
                    if ignore && written.values.into_count() == 0 {
                        // 冲突被忽略：这一行没有写进去，也不该出现在结果里。
                        continue;
                    }
                }
                [index] if reload_columns[*index].auto_increment => {
                    self.run_insert(db, &single, params, Ret::Count, identity)
                        .await?;
                    let column = reload_columns[*index];
                    let raw = self
                        .query_one_sql("SELECT @@IDENTITY", &[])
                        .await?
                        .and_then(|row| row.values().first().cloned())
                        .ok_or_else(|| {
                            toasty_core::Error::invalid_result(format!(
                                "`INSERT … RETURNING` 模拟：`SELECT @@IDENTITY` 没有返回值（表 {}）",
                                table.name
                            ))
                        })?;
                    keys[*index] = Some(TypedValue {
                        value: value::decode(&raw, &column.ty)?,
                        ty: column.storage_ty.clone(),
                    });
                }
                _ => {
                    return Err(toasty_core::Error::unsupported_feature(format!(
                        "达梦的 `INSERT … RETURNING` 模拟取不到 {key_label} {} 的值（表 {}）：只有「唯一一列自增主键缺省」这一种形状能靠 `@@IDENTITY` 补",
                        missing
                            .iter()
                            .map(|index| format!("`{}`", reload_columns[*index].name))
                            .collect::<Vec<_>>()
                            .join("、"),
                        table.name
                    )));
                }
            }

            let keys: Vec<TypedValue> = keys
                .into_iter()
                .map(|key| key.expect("上一步已把缺的回查键补齐"))
                .collect();

            // 第二步：按回查键把这一行的投影读回来。
            let mut follow = Vec::with_capacity(keys.len());
            let mut filter = None;
            for (column, key) in reload_columns.iter().zip(&keys) {
                let position = follow.len();
                follow.push(key.clone());
                let predicate = stmt::Expr::eq(
                    stmt::Expr::column(stmt::ExprReference::column(0, column.id.index)),
                    stmt::Expr::arg(position),
                );
                filter = Some(match filter {
                    Some(previous) => stmt::Expr::and(previous, predicate),
                    None => predicate,
                });
            }

            let select = stmt::Select {
                returning: returning.clone(),
                source: stmt::Source::table(target.table),
                filter: stmt::Filter::new(filter.expect("回查键至少一列")),
                distinct: false,
            };
            let query =
                stmt::Statement::Query(stmt::Query::new(stmt::ExprSet::Select(Box::new(select))));
            let response = self
                .exec_stmt(db, &query, &follow, Ret::Types(types.to_vec()))
                .await?;

            let mut fetched = match response.values.collect_as_value().await? {
                stmt::Value::List(rows) => rows,
                other => {
                    return Err(toasty_core::Error::invalid_result(format!(
                        "`INSERT … RETURNING` 模拟：回查返回的不是行流（{other:?}）"
                    )));
                }
            };
            match fetched.len() {
                1 => result.push(fetched.pop().expect("长度已判为 1")),
                0 => {
                    return Err(toasty_core::Error::invalid_result(format!(
                        "`INSERT … RETURNING` 模拟：按{key_label}回查不到刚写入的行（表 {}）",
                        table.name
                    )));
                }
                n => {
                    return Err(toasty_core::Error::invalid_result(format!(
                        "`INSERT … RETURNING` 模拟：按{key_label}回查到 {n} 行（表 {}）",
                        table.name
                    )));
                }
            }
        }

        Ok(ExecResponse::value_stream(stmt::ValueStream::from_vec(
            result,
        )))
    }

    /// 事务控制：`Start` / `Commit` / `Rollback` / 三个保存点操作。
    async fn exec_transaction(&mut self, op: TxOp) -> toasty_core::Result<ExecResponse> {
        match op {
            TxOp::Start {
                isolation,
                read_only,
                mode,
            } => {
                // `TransactionMode` 是 SQLite 系的加锁时机；达梦没有等价物，照上游 MySQL
                // 的做法对 `Immediate`/`Exclusive` 报不支持，其余按隔离级别开事务。
                if matches!(
                    mode,
                    TransactionMode::Immediate | TransactionMode::Exclusive
                ) {
                    return Err(toasty_core::Error::unsupported_feature(format!(
                        "达梦不支持 TransactionMode::{mode:?}"
                    )));
                }
                self.begin(isolation, read_only).await?;
                Ok(ExecResponse::count(0))
            }
            TxOp::Commit => {
                self.commit().await?;
                Ok(ExecResponse::count(0))
            }
            TxOp::Rollback => {
                self.rollback().await?;
                Ok(ExecResponse::count(0))
            }
            TxOp::Savepoint(name) => {
                self.savepoint_op(SavepointOp::Create, &name).await?;
                Ok(ExecResponse::count(0))
            }
            TxOp::ReleaseSavepoint(name) => {
                // 达梦没有 `RELEASE SAVEPOINT` 语法：`dm-query` 会把语法拒绝翻成
                // `Unsupported`（§6.6），这里原样透传 ⇒ 上层看到的是能力缺口而不是 -2007。
                self.savepoint_op(SavepointOp::Release, &name).await?;
                Ok(ExecResponse::count(0))
            }
            TxOp::RollbackToSavepoint(name) => {
                self.savepoint_op(SavepointOp::RollbackTo, &name).await?;
                Ok(ExecResponse::count(0))
            }
        }
    }

    /// 在事务里跑一个保存点操作。
    ///
    /// 不写成「接收闭包」的泛型：`dm_query::Transaction` 的方法都借 `&mut self`，
    /// 让闭包把它借出去会把 `Fut` 的生命周期绑到闭包的 `FnOnce` 上（编译器 E0700/E0521）。
    async fn savepoint_op(&mut self, op: SavepointOp, name: &str) -> toasty_core::Result<()> {
        let result = match self.session.as_mut() {
            Some(SessionKind::Txn(txn)) => match op {
                SavepointOp::Create => txn.savepoint(name).await,
                // 达梦**没有** `RELEASE SAVEPOINT` 语法（真机实测：`RELEASE SAVEPOINT sp_raw`
                // 报 `-2007 语法分析出错`，而 `SAVEPOINT sp`、`ROLLBACK TO sp`、
                // `ROLLBACK TO SAVEPOINT sp` 三种都通过）。引擎在**嵌套事务成功结束**时发这个
                // 操作，所以把「释放」当无操作是安全的：
                //   * 达梦的保存点在 COMMIT/ROLLBACK 时自动释放；
                //   * 引擎给每个保存点用唯一名字（`engine/**/tx.rs` 的 `format!("tx_{}", …)`），
                //     残留的旧保存点不会被复用；
                //   * 回滚到更外层的保存点时，达梦会连同内层保存点一起丢弃。
                // 反过来，如果这里报 `unsupported_feature`，嵌套事务的用例会整片失败。
                SavepointOp::Release => {
                    tracing::debug!("达梦无 RELEASE SAVEPOINT，把释放保存点 {name} 按无操作处理");
                    Ok(())
                }
                SavepointOp::RollbackTo => txn.rollback_to(name).await,
            },
            _ => {
                return Err(toasty_core::Error::invalid_statement(format!(
                    "保存点 {name} 只能在事务里使用，但当前没有开着的事务"
                )));
            }
        };
        match result {
            Ok(()) => Ok(()),
            Err(err) => Err(self.record_err(err)),
        }
    }

    /// 开事务（把空闲连接的所有权交给 `dm_query::Transaction`）。
    async fn begin(
        &mut self,
        isolation: Option<toasty_core::driver::operation::IsolationLevel>,
        read_only: bool,
    ) -> toasty_core::Result<()> {
        use toasty_core::driver::operation::IsolationLevel as Upstream;

        // 上游 `IsolationLevel` 与 `dm_core::IsolationLevel` 逐变体同名：映射是恒等的，
        // 但两边是不同类型，只能显式转。
        let level = match isolation {
            None => None,
            Some(Upstream::ReadUncommitted) => Some(dm_core::IsolationLevel::ReadUncommitted),
            Some(Upstream::ReadCommitted) => Some(dm_core::IsolationLevel::ReadCommitted),
            Some(Upstream::RepeatableRead) => Some(dm_core::IsolationLevel::RepeatableRead),
            Some(Upstream::Serializable) => Some(dm_core::IsolationLevel::Serializable),
        };
        // 只读事务：DM 支持 `SET TRANSACTION READ ONLY`，真机实测它**真的**拦住写操作
        // （`UPDATE` 被服务端以 `[25000/-6506] 试图在只读事务中修改数据` 拒绝）⇒ 如实实现，
        // 不再报 `unsupported_feature`。
        //
        // 隔离级别与只读**必须分两条发**：达梦不接受 SQL:92 的合并形态
        // `SET TRANSACTION ISOLATION LEVEL <l> READ ONLY` —— T8.9 真机探针 12a 实测
        // `SQL 语法错误 … [42000/-2007] 第 1 行, 第 89 列[READ]附近出现错误`，官方套件
        // `tx_interactive::builder_with_all_options` 就是撞在这条上。分两条则
        // 12b（先隔离级别、再只读）与 12c（先只读、再隔离级别）都成功 ⇒ 这里按 12b 的顺序。
        // 两条都紧接着 `begin()`（autocommit=OFF）发，仍是该事务最早的语句。

        self.ensure_session().await?;
        let conn = match self.session.take() {
            Some(SessionKind::Idle(conn)) => conn,
            Some(txn @ SessionKind::Txn(_)) => {
                self.session = Some(txn);
                return Err(toasty_core::Error::invalid_statement(
                    "达梦驱动收到嵌套的事务开始（事务里的嵌套应该用保存点）",
                ));
            }
            None => {
                return Err(toasty_core::Error::invalid_statement(
                    "达梦驱动没有可用的连接来开启事务",
                ));
            }
        };
        // `begin()` 只做 `autocommit = OFF`；`SET TRANSACTION …` 由下面按需发。
        let begun = conn.begin().await;
        let mut txn = match begun {
            Ok(txn) => txn,
            Err(err) => return Err(self.record_err(err)),
        };

        let mut setup = Vec::new();
        if let Some(level) = level {
            setup.push(format!(
                "SET TRANSACTION ISOLATION LEVEL {}",
                level.as_str()
            ));
        }
        if read_only {
            setup.push("SET TRANSACTION READ ONLY".to_string());
        }
        for sql in &setup {
            if let Err(err) = txn.execute(sql, ()).await {
                // 隔离级别/只读被服务端拒绝：先把事务收掉，别把半开的事务还给池。
                let _ = txn.rollback().await;
                return Err(self.record_err(err));
            }
        }

        self.session = Some(SessionKind::Txn(txn));
        Ok(())
    }

    /// 提交：事务对象被吃掉，连接归还池子，下一次操作再借。
    async fn commit(&mut self) -> toasty_core::Result<()> {
        match self.session.take() {
            Some(SessionKind::Txn(txn)) => match txn.commit().await {
                Ok(()) => Ok(()),
                Err(err) => Err(self.record_err(err)),
            },
            Some(other) => {
                self.session = Some(other);
                Err(toasty_core::Error::invalid_statement(
                    "达梦驱动收到 COMMIT，但当前没有开着的事务",
                ))
            }
            None => Err(toasty_core::Error::invalid_statement(
                "达梦驱动收到 COMMIT，但当前没有连接",
            )),
        }
    }

    /// 回滚（同上）。
    async fn rollback(&mut self) -> toasty_core::Result<()> {
        match self.session.take() {
            Some(SessionKind::Txn(txn)) => match txn.rollback().await {
                Ok(()) => Ok(()),
                Err(err) => Err(self.record_err(err)),
            },
            Some(other) => {
                self.session = Some(other);
                Err(toasty_core::Error::invalid_statement(
                    "达梦驱动收到 ROLLBACK，但当前没有开着的事务",
                ))
            }
            None => Err(toasty_core::Error::invalid_statement(
                "达梦驱动收到 ROLLBACK，但当前没有连接",
            )),
        }
    }

    /// 建迁移记账表（幂等）。
    ///
    /// 不用 `CREATE TABLE IF NOT EXISTS`：达梦是否支持这个前缀还没实测（§9.4 规则 9 只实测了
    /// `DROP TABLE IF EXISTS`），所以先查 `USER_TABLES` 再决定建不建。
    async fn ensure_migrations_table(&mut self) -> toasty_core::Result<()> {
        let sql = format!(
            "SELECT COUNT(*) FROM USER_TABLES WHERE TABLE_NAME = '{}'",
            MIGRATIONS_TABLE.to_uppercase()
        );
        let exists = match self.query_one_sql(&sql, &[]).await? {
            Some(row) => row.get::<i64>(0).map_err(|e| self.record_err(e))? > 0,
            None => false,
        };
        if !exists {
            let ddl = format!(
                "CREATE TABLE {MIGRATIONS_TABLE} (id BIGINT PRIMARY KEY, name VARCHAR(255) NOT NULL, applied_at TIMESTAMP NOT NULL)"
            );
            self.execute_sql(&ddl, &[]).await?;
        }
        Ok(())
    }
}

#[async_trait]
impl Connection for DamengConnection {
    async fn exec(
        &mut self,
        schema: &Arc<Schema>,
        plan: Operation,
    ) -> toasty_core::Result<ExecResponse> {
        match plan {
            Operation::Transaction(op) => self.exec_transaction(op).await,

            Operation::QuerySql(op) => {
                let ret = match op.ret {
                    Some(types) => Ret::Types(types),
                    None => Ret::Count,
                };
                self.exec_stmt(&schema.db, &op.stmt, &op.params, ret).await
            }

            Operation::Insert(op) => {
                // 达梦没有 `RETURNING`（§9.4 第 6 条）：把投影从写入语句上摘下来，先执行写入，
                // 再按主键把投影读回来。`capability().returning_from_insert = true` 声明的正是
                // 「驱动会替上游把 RETURNING 模拟出来」，而不是「达梦支持 RETURNING」。
                let mut write = op.stmt.clone();
                let returning = write.take_returning();

                // 自增列要写显式值时，达梦要求 `SET IDENTITY_INSERT <表> ON/OFF` 包一层
                // （真机探针 15k–15p：不开就报 `[HY000/-2723]`）。MERGE 形态最容易踩到。
                let identity = identity_insert_table(&schema.db, &write);

                match (returning, op.ret) {
                    // 上游声明了结果类型 ⇒ 走「先写入、再按主键回查」的模拟。
                    (Some(returning), Some(types)) => {
                        self.insert_and_reload(
                            &schema.db,
                            &write,
                            &op.params,
                            &returning,
                            &types,
                            identity.as_deref(),
                        )
                        .await
                    }
                    // 没有 `RETURNING` 的单行 insert：上游只允许「一个自增主键」这种精确结果
                    // （`engine/plan/statement.rs:1036-1079`），回查 `@@IDENTITY` 补上。
                    (None, Some(types)) => {
                        let ret = match types.as_slice() {
                            [ty] => Ret::GeneratedId(ty.clone()),
                            other => {
                                return Err(toasty_core::Error::invalid_result(format!(
                                    "单行 insert 的结果类型必须恰好 1 个，实际 {other:?}"
                                )));
                            }
                        };
                        self.run_insert(&schema.db, &write, &op.params, ret, identity.as_deref())
                            .await
                    }
                    // 上游不需要结果：只执行写入。
                    (_, None) => {
                        self.run_insert(
                            &schema.db,
                            &write,
                            &op.params,
                            Ret::Count,
                            identity.as_deref(),
                        )
                        .await
                    }
                }
            }

            Operation::RawSql(op) => {
                let logged: Vec<stmt::Value> = if self.query_log.params {
                    op.params.iter().map(|p| p.value.clone()).collect()
                } else {
                    Vec::new()
                };
                self.exec_raw(&op.sql, &op.params, op.ret, logged).await
            }

            // 键值驱动专用的变体：SQL 驱动收不到（`scan` 我们声明为 false），
            // 真收到就如实报能力缺口——**不要** `todo!()`（§15.11 CAP-K-26）。
            other => Err(unsupported_operation(other.name())),
        }
    }

    /// 廉价、无 IO：只看「这条连接上是否出现过连接级故障」。
    fn is_valid(&self) -> bool {
        self.valid
    }

    /// 后台健康检查：失败**必须**归 `connection_lost`，否则死连接会漏回空闲集。
    async fn ping(&mut self) -> toasty_core::Result<()> {
        // `SELECT 1 FROM DUAL` 是达梦上最便宜的一次往返（§6.6）。
        match self.query_sql("SELECT 1 FROM DUAL", &[]).await {
            Ok(_) => Ok(()),
            Err(err) => {
                self.valid = false;
                // 这里**故意**不再走 `to_toasty` 的分类：上游文档要求 ping 的失败一定是
                // `connection_lost`，哪怕根因是「语句写错了」也要按连接不可用来处理。
                Err(toasty_core::Error::connection_lost(PingFailed(format!(
                    "达梦 ping 失败：{err}"
                ))))
            }
        }
    }

    /// 建表（+ 非主键索引）。DDL 用 `serialize`（值内联，DDL 不支持绑定参数）。
    async fn push_schema(&mut self, schema: &Schema) -> toasty_core::Result<()> {
        for table in &schema.db.tables {
            // 一条 serializer 服务一张表及其索引（与上游 MySQL 驱动一致）。
            let serializer = toasty_sql::Serializer::dameng(&schema.db);

            let create_table =
                toasty_sql::stmt::Statement::create_table(table, &capability::DAMENG);
            let sql = serializer.serialize(&create_table);
            // DDL 不走 `QueryLog`（上游只把 DML 接进去），这里补一条同 target 的调试日志：
            // 真机联调时必须能看见「达梦到底收到了什么语句」。
            tracing::debug!(target: "toasty::query", "push_schema: {sql}");
            self.execute_sql(&sql, &[]).await?;

            for index in &table.indices {
                if index.primary_key {
                    continue;
                }
                let create_index = toasty_sql::stmt::Statement::create_index(index);
                let sql = serializer.serialize(&create_index);
                tracing::debug!(target: "toasty::query", "push_schema: {sql}");
                self.execute_sql(&sql, &[]).await?;
            }
        }
        Ok(())
    }

    /// 已应用的迁移：只读记账表的 `id`（0.11.0 的 `AppliedMigration` 只有 `id`）。
    async fn applied_migrations(&mut self) -> toasty_core::Result<Vec<AppliedMigration>> {
        self.ensure_migrations_table().await?;
        let sql = format!("SELECT id FROM {MIGRATIONS_TABLE} ORDER BY applied_at");
        let rowset = self.query_sql(&sql, &[]).await?;
        let mut applied = Vec::with_capacity(rowset.len());
        for row in rowset.rows() {
            let id = row.get::<i64>(0).map_err(|e| self.record_err(e))?;
            let id = u64::try_from(id).map_err(|_| {
                toasty_core::Error::invalid_result(format!("迁移表里的 id={id} 不是合法的主键"))
            })?;
            applied.push(AppliedMigration::new(id));
        }
        Ok(applied)
    }

    /// 应用一个迁移：**一个事务**里跑完所有语句再写记账行；失败回滚。
    async fn apply_migration(
        &mut self,
        id: u64,
        name: &str,
        migration: &Migration,
    ) -> toasty_core::Result<()> {
        self.ensure_migrations_table().await?;
        self.begin(None, false).await?;

        let result = async {
            for statement in migration.statements() {
                self.execute_sql(statement, &[]).await?;
            }
            let sql = format!(
                "INSERT INTO {MIGRATIONS_TABLE} (id, name, applied_at) VALUES (?, ?, NOW())"
            );
            let args = [
                DmValue::BigInt(i64::try_from(id).map_err(|_| {
                    toasty_core::Error::invalid_result(format!("迁移 id={id} 超出 BIGINT"))
                })?),
                DmValue::Text(name.to_owned()),
            ];
            self.execute_sql(&sql, &args).await?;
            Ok(())
        }
        .await;

        match result {
            Ok(()) => self.commit().await,
            Err(err) => {
                // 回滚失败不能掩盖原始错误：先记回滚结果，再返回原错。
                if let Err(rollback_err) = self.rollback().await {
                    tracing::warn!(
                        %rollback_err,
                        "迁移失败后的回滚也失败了；原始错误：{err}"
                    );
                }
                Err(err)
            }
        }
    }
}

/// 单行 `INSERT` 里某个主键列的绑定值（用于模拟 `RETURNING` 的回查条件）。
///
/// 只认两种形状：`Expr::Arg(n)`（引擎的绑定参数）与 `Expr::Value(v)`（内联常量）。其它形状
/// （例如 `Expr::Incoming`、子查询）返回 `None`，由调用方按能力缺口报错。
fn insert_column_value(
    insert: &stmt::Insert,
    row: &stmt::ExprRecord,
    column: &db::Column,
    params: &[TypedValue],
) -> Option<TypedValue> {
    let target = insert.target.as_table_unwrap();
    let position = target
        .columns
        .iter()
        .position(|candidate| candidate.index == column.id.index)?;

    match row.fields.get(position)? {
        stmt::Expr::Arg(arg) if arg.nesting == 0 => params.get(arg.position).cloned(),
        stmt::Expr::Value(value) => Some(TypedValue {
            value: value.clone(),
            ty: column.storage_ty.clone(),
        }),
        _ => None,
    }
}

/// 把「多行 `INSERT`」改写成「只含指定那一行的 `INSERT`」。
///
/// 用于 `INSERT … RETURNING` 模拟的逐行写入。参数是**位置索引**（`Expr::Arg(n)`），
/// 只改行数不动索引 ⇒ 同一份 `params` 可以原样复用。
fn single_row_insert(write: &stmt::Statement, row: &stmt::ExprRecord) -> stmt::Statement {
    let mut single = write.clone();
    let stmt::Statement::Insert(insert) = &mut single else {
        unreachable!("`Operation::Insert` 携带的一定是 `Statement::Insert`");
    };
    insert.source = stmt::Query::new(stmt::ExprSet::Values(stmt::Values::new(vec![
        stmt::Expr::Record(row.clone()),
    ])));
    single
}

/// 需要 `SET IDENTITY_INSERT <表> ON/OFF` 包裹时，返回**已折大写并带双引号**的表名。
///
/// 表名带模式前缀（`#[table = "DBTESTA.T_ORDER"]`）时返回 `"DBTESTA"."T_ORDER"`：
/// 用 `toasty_sql::dameng_table_name`，与 DML/DDL 的渲染规则共用一份实现（模式名原样、
/// 表名折大写），否则这里渲染出 `"DBTESTA.T_ORDER"` 会让开关作用在一个不存在的表上。
///
/// 触发条件：upsert（`MERGE INTO`）形态，且显式给某个自增列赋了值。真机依据（引擎探针第 15 节）：
/// 不开这个开关就给自增列赋值 ⇒ `[HY000/-2723] 仅当指定列列表，且SET IDENTITY_INSERT为ON时，
/// 才能对自增列赋值`；打开后 MERGE 的「插入新行 / 命中已有行」两种分支都能写显式值。
fn identity_insert_table(db: &db::Schema, write: &stmt::Statement) -> Option<String> {
    let insert = write.as_insert()?;
    // 只有 upsert（MERGE 形态）会带显式自增值：普通 insert 那边上游一律用 `Expr::Default`。
    insert.upsert.as_ref()?;

    let target = insert.target.as_table_unwrap();
    let table = db.table(target.table);
    let row = match &insert.source.body {
        stmt::ExprSet::Values(values) if values.rows.len() == 1 => {
            values.rows[0].as_record_unwrap()
        }
        _ => return None,
    };

    for (position, column_id) in target.columns.iter().enumerate() {
        let column = table.column(*column_id);
        if !column.auto_increment {
            continue;
        }
        if matches!(row.fields.get(position), Some(stmt::Expr::Default) | None) {
            continue;
        }
        return Some(toasty_sql::dameng_table_name(&table.name));
    }

    None
}

/// 一行结果 = 一个 `Value::Record`（与上游各驱动一致；引擎侧按记录投影取列）。
fn record(values: Vec<stmt::Value>) -> stmt::Value {
    stmt::Value::Record(stmt::ValueRecord::from_vec(values))
}

/// 按 `ret` 给的目标类型解码整个结果集。
fn decode_rowset(rowset: RowSet, types: &[stmt::Type]) -> toasty_core::Result<ExecResponse> {
    let mut records = Vec::with_capacity(rowset.len());
    for row in rowset.rows() {
        if row.len() != types.len() {
            return Err(toasty_core::Error::invalid_result(format!(
                "结果集有 {} 列，但引擎声明了 {} 个返回类型 {:?}",
                row.len(),
                types.len(),
                types
            )));
        }
        let mut values = Vec::with_capacity(types.len());
        for (value, ty) in row.values().iter().zip(types) {
            values.push(value::decode(value, ty)?);
        }
        records.push(record(values));
    }
    Ok(ExecResponse::value_stream(stmt::ValueStream::from_vec(
        records,
    )))
}

/// `RawSqlRet::Infer`：按 ODBC 元数据推断出来的 `DmValue` 变体挑 `stmt::Value`。
fn infer_rowset(rowset: RowSet) -> toasty_core::Result<ExecResponse> {
    let mut records = Vec::with_capacity(rowset.len());
    for row in rowset.rows() {
        let mut values = Vec::with_capacity(row.len());
        for value in row.values() {
            values.push(value::infer(value)?);
        }
        records.push(record(values));
    }
    Ok(ExecResponse::value_stream(stmt::ValueStream::from_vec(
        records,
    )))
}

/// `ping` 的失败载体：上游要求 ping 的失败一定是 `connection_lost`，而 `connection_lost`
/// 收的是 `impl std::error::Error`，所以这里包一层把 `DmError` 带进去。
#[derive(Debug)]
struct PingFailed(String);

impl std::fmt::Display for PingFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PingFailed {}

/// `DM_TRACE_SQL=1` 时把每条语句与它的参数打到 stderr。
///
/// 默认关闭，且**不进 `tracing`**：`dm-query` 那一层刻意只记 `sql_fp` 指纹（不落原文，
/// 免得把业务数据写进日志），这个开关是排查「引擎到底发了什么 SQL」时的临时放大镜
/// （T8.9 对齐官方套件 `paginate_multi_column` 时加的）。用法：
/// `$env:DM_TRACE_SQL='1'; cargo test … -- --nocapture`。
fn trace_sql(sql: &str, args: &[DmValue]) {
    if std::env::var_os("DM_TRACE_SQL").is_some() {
        eprintln!("[dm-sql] {sql}  -- args={args:?}");
    }
}
