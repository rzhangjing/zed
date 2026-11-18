# ew_ordpack 设计方案 v0.1

> 目标：一个**编译期 schema 驱动**的二进制序列化框架。线格式里**不存任何字段名/字段 ID**，
> 结构体只带一个**字段个数**，其余全部依赖类型定义中的**声明顺序**。
> 设计取向是「速度优先、存储场景、单向演进」，性能目标是逼近手写 codec。

---

## 0. 一句话概括

```rust
#[derive(Encode, Decode)]
struct User { id: u64, name: String, active: bool }

// 线格式（7 字节）：
// 03 00 | 01 | 02 61 62 | 01
//  ^^^^^   ^    ^         ^
//  字段数   id  "ab"     active
//  (u16 LE, 2 字节)
```

对比 bincode 2（standard，varint）的 5 字节：多出的 2 字节就是字段个数，
换来的是「缺失的尾部字段可以填默认值」。**本设计根本不写字段名/字段 ID。**

---

## 1. 目标、非目标与假设

### 1.1 目标

| 编号 | 目标 | 验收方式 |
|---|---|---|
| G1 | 编解码速度与 bincode 2 同级或更好 | 基准：解码 ≤ 1.0× bincode 2；编码受 +2 字节计数影响 |
| G2 | 覆盖 Rust 标准类型 | 独立 roundtrip 测试矩阵 |
| G3 | 覆盖容器类型 | Vec/HashMap/BTreeMap/集合/数组/元组/Cow/智能指针 |
| G4 | 覆盖自定义类型 | derive 支持 struct/tuple struct/unit/newtype/enum/泛型/生命周期 |
| G5 | 线格式零字段元数据 | 字节级 golden test 冻结 |
| G6 | 单向可演进（append-only） | v1 数据可由 v2 类型解码 |
| G7 | 默认零 `unsafe`、可处理不可信输入 | 不 panic、不越界、有界分配 |
| G8 | `no_std + alloc` | `--no-default-features` 编译通过 |

### 1.2 非目标

- **N1 网络协议**：不做双向跨版本兼容。未知字段无法跳过（除非显式开启 `compat`）。
- **N2 自描述格式**：没有类型定义就无法解码。不提供 `Value` 动态树（可作为可选 feature）。
- **N3 跨语言**：不追求与 protobuf/flatbuffers 互通。
- **N4 字段重排 / 中间插入**：这是本设计的硬约束，见 §4.2。
- **N5 归档式零拷贝**（rkyv 那种）：「读出即用、无需反序列化」不是目标；目标是把反序列化本身做快。

### 1.3 假设（请确认）

1. 使用场景是**存储/快照/进程内缓存/日志**，读端通常和写端一起升级，或读端先升级。
2. 生产者和消费者共享同一份 Rust 类型定义（同一个 crate 或同一版本协议）。
3. 主流平台为小端；大端通过 `native-endian` feature 或固定小端兜底。
4. 单条记录的 schema 相对稳定，演进以「尾部追加字段」为主。

---

## 2. 设计总览

### 2.1 分层

```
┌──────────────────────────────────────────────────────────┐
│ 类型层   #[derive(Encode, Decode)]                        │
│         把「字段顺序 + 字段个数」直接编译成直线代码         │
├──────────────────────────────────────────────────────────┤
│ 格式层   Encoder<W> / Decoder<'de, R>                     │
│         varint / zigzag / tag / 字段个数 / 长度            │
├──────────────────────────────────────────────────────────┤
│ 字节层   Writer / Reader                                   │
│         Vec<u8>、&mut [u8]、io::Write（v2）                 │
└──────────────────────────────────────────────────────────┘
```

与 serde 最大的区别：**没有 visitor，没有 `serialize_struct(name, len)`，没有运行期形状表**。
派生宏生成的就是 `write_varint(N)` + 依次 `field.encode(e)`，全部内联后可被 LLVM 消成几条 `mov`。

### 2.2 关键决策

| 编号 | 决策 | 代价 | 收益 |
|---|---|---|---|
| D1 | 结构体只写 `u16le(字段个数)`，不写名称/ID | 每结构体 2 字节 | 缺失尾字段可填默认值 |
| D2 | schema 固化在类型里，不落盘 | 需要类型定义 | 解码全是静态分派 |
| D3 | 整数默认 varint + zigzag；`fixed-int` feature 切定长 | 解码多几个分支 | 典型数据体积小 2–4 倍 |
| D4 | 枚举用**声明顺序**的变体序号 | 中间插入变体会破坏 | 1 字节即可表达 |
| D5 | `Option`/`Result` 单字节 tag | 1 字节 | 无歧义、可跳过判断 |
| D6 | 可选 `compat`：结构体额外写 `varint(字节长度)` | 1–3 字节 | 可跳过尾部未知字段 |
| D7 | 可选 `FINGERPRINT` 常量 + 文件头 | 每文件 14 字节 | 加载时立刻发现 schema 不匹配 |
| D8 | 默认零 `unsafe`；`unchecked` feature 提供快速路径 | 少量边界检查 | 内存安全 |

**为什么是「字段个数」而不是「字段 ID」**：字段 ID 至少要 1 字节/字段（10 个字段 = 10 字节），
而且必须在每个字段前读取、比较、跳转；字段个数只有 2 字节/结构体（u16），还能在编译期折叠成常量。
它换来两件 ID 方案才有的事：字段重排、跳过未知字段。本设计明确放弃前者，后者用可选的 `compat` 兜底。

---

## 3. 线格式规范 v1

### 3.1 基础编码

#### 3.1.1 varint（LEB128，无符号）

- 每个字节低 7 位有效，最高位是「还有后续」标志。
- 编码 0..=127 → 1 字节；300 → `AC 02`；800 → `A0 06`。
- 解码限制：最多 10 字节；第 10 字节只允许 `0x01`，否则 `ErrorKind::VarintOverflow`。
- 默认接受非规范（overlong）编码；`strict` feature 下拒绝，以支持「字节唯一」承诺。

#### 3.1.2 zigzag（有符号）

```
encode: (n << 1) ^ (n >> 63)      // i64，算术右移
decode: (u >> 1) as i64 ^ -((u & 1) as i64)
```

`0 → 0`，`-1 → 1`，`1 → 2`，`-2 → 3`。小负数不会膨胀成 10 字节。

#### 3.1.3 定长整数（`fixed-int` feature 或按类型属性）

小端（或 `native-endian`）原样写入，`u16 → 2B`、`u64 → 8B`。解码是一次 `read_unaligned`。

#### 3.1.4 浮点

`f32`/`f64` 始终是 4/8 字节 IEEE-754 位模式，小端。**不规范化 NaN**（保留 payload，保证 roundtrip 位级相等）。

#### 3.1.5 字段个数（u16，固定 2 字节）

`nfields` 固定用 **2 字节小端 u16**，不用 varint：

| 方案 | 1–127 字段 | 128–16383 字段 | >16383 字段 | 解码成本 |
|---|---|---|---|---|
| varint | 1 B | 2 B | 3 B+ | 循环 + 分支 |
| **u16le** | **2 B** | **2 B** | **不支持** | **一次 2 字节读** |

选择固定宽度：

- 字段个数位于每条结构体/变体的头部热路径，固定宽度没有 varint 循环和分支；
- 上限 65535，足够覆盖任何现实结构体；
- derive 在编译期检查 `字段数 <= 65535`，超出直接编译失败；解码端读 u16 天然有界。

代价：小结构体比 varint 多 1 字节。这是明确取舍，换来的是头部的可预测性与上限保障。

### 3.2 线格式文法

```
value    := primitive | option | seq | map | tuple | structure | enumeration
option   := 0x00 | 0x01 value
seq      := varint(len) value{len}
map      := varint(len) (value value){len}
tuple    := value{N}                      ; N 由类型决定，无长度
structure:= u16le(nfields) value{nfields}  ; 声明顺序，2 字节计数
packed   := value{nfields}                 ; 无 nfields，版本锁定
enum     := varint(tag) [ u16le(nfields) value{nfields} ]
str      := varint(byte_len) utf8{byte_len}
bytes    := varint(byte_len) u8{byte_len}  ; 用于 &[u8] / Vec<u8>
```

> 注意：`enum` 只有「带字段的变体」才写 `nfields`。unit 变体编码后只有 1 字节 tag。

### 3.3 结构与枚举的精确规则

**结构体（named / tuple，字段数 ≠ 1）**

```
u16le(nfields)       ; 所有「参与编码」的字段个数；skip / PhantomData 不计
value, value, ...    ; 严格按声明顺序
```

**newtype 结构体（单字段 tuple struct）**：透明，等价于内部类型，**不写 nfields**。
这是为了 `struct UserId(u64)` 这种类型零开销，也为了泛型包装器不引入额外字节。

**单字段 named struct**：默认仍写 `nfields`，需要透明时加 `#[ordpack(transparent)]`。

**0 字段结构体 / unit struct**：写 `u16le(0)`（2 字节）；`#[ordpack(packed)]` 时 0 字节。

**枚举**

```
variant := varint(index)              ; index 是声明顺序，从 0 开始
         [ u16le(nfields) field* ]    ; 仅当变体至少有一个参与编码的字段
```

`#[ordpack(tag = 7)]` 可以把某个变体的序号钉死为 7，用于「以后要在前面插变体」的场景。
未标注 tag 的变体按声明顺序编号；一旦有显式 tag，建议全部显式，避免隐式编号漂移。

**字段个数上限**：65535。derive 在编译期检查，超出直接编译失败；解码端读 u16 天然有界，
只需在 `n > EXPECTED` 时立即报错，不会出现长循环。

### 3.4 完整类型映射表

| Rust 类型 | 线格式 | 字节数 |
|---|---|---|
| `bool` | `0x00`/`0x01`，其余值报错 | 1 |
| `u8` / `i8` | 原样 | 1 |
| `u16` `u32` `u64` `u128` | `varint`（`fixed-int` 时定长 LE） | 1..19 |
| `i16` `i32` `i64` `i128` | `zigzag(varint)` | 1..19 |
| `usize` / `isize` | 按 `u64` / `i64` 编码（跨平台稳定） | 1..10 |
| `f32` / `f64` | IEEE-754 位模式，小端 | 4 / 8 |
| `char` | `varint` 标量值，解码校验 `char::from_u32` | 1..4 |
| `()` / `PhantomData<T>` | 空 | 0 |
| `Option<T>` | `0x00` \| `0x01 + T` | 1+ |
| `Result<T, E>` | `0x00 + T`（Ok）\| `0x01 + E`（Err） | 1+ |
| `String` `&str` `Box<str>` `Cow<str>` | `varint(字节长)` + UTF-8 | 1+ |
| `[u8; N]` | 原始字节，无长度前缀 | N |
| `&[u8]` `Vec<u8>` `Box<[u8]>` | `varint(len)` + 原始字节 | 1+ |
| `Vec<T>` `VecDeque<T>` `LinkedList<T>` `BinaryHeap<T>` | `varint 个数` + 元素 | 1+ |
| `&[T]` `[T; N]` | 序列（`&[T]` 带长度；数组不带） | — |
| `(A, B, ...)` | 依次编码，无计数 | — |
| `HashMap<K,V>` `BTreeMap<K,V>` | `varint 条目数` + `(K,V)*` | 1+ |
| `HashSet<T>` `BTreeSet<T>` | `varint 个数` + 元素 | 1+ |
| `Box<T>` `Rc<T>` `Arc<T>` `Cow<T>` | 同 `T` | — |
| `Cell<T>` `RefCell<T>` `Mutex<T>` `RwLock<T>` | 同 `T`（解码重建容器） | — |
| `Wrapping<T>` `Reverse<T>` | 同 `T` | — |
| `Duration` | `varint secs` + `varint nanos` | 2+ |
| `SystemTime` | `zigzag secs`（相对 UNIX_EPOCH）+ `varint nanos` | 2+ |
| `Range<T>` `RangeInclusive<T>` 等 | 两个字段（同 struct） | — |
| `NonZeroU8..U128` 等 | 基础整数，解码校验非零 | 1+ |
| `IpAddr` | `0x04/0x06` + 地址字节 | 5 / 17 |
| `SocketAddr` | 地址 + `u16` 端口 | 7 / 19 |
| `Path` `PathBuf` `OsStr` `OsString` | `varint` + 平台字节（**不跨平台**） | 1+ |
| `CStr` `CString` | `varint` + 不含 NUL 的字节 | 1+ |
| `NonNull<T>` `*const T` `*mut T` | **不实现**（地址无意义），用 `with` 逃逸 | — |
| `Instant` | **不实现**（无稳定表示） | — |

### 3.5 字节示例

```rust
struct Point { x: i32, y: i32 }
Point { x: 1, y: -2 }
// 02 00 | 02 | 03
//  ^^^^^   ^    └ zigzag(-2) = 3
//  |       └ zigzag(1) = 2
//  └ 字段数 = 2（u16 LE）

struct User { id: u64, name: String, active: bool }
User { id: 1, name: "ab", active: true }
// 03 00 | 01 | 02 61 62 | 01        → 7 字节（bincode 2 standard: 5 字节）
//   前 2 字节是字段个数（u16 LE），bincode 没有它
```

```rust
enum Event { Tick, Key(u8), Resize { w: u16, h: u16 } }

Event::Tick                     → 00
Event::Key(65)                  → 01 01 00 41
Event::Resize { w: 800, h: 600 } → 02 02 00 A0 06 D8 04
```

```rust
Option<u32> = Some(300)  → 01 AC 02
Vec<u16> = [1, 2, 3]     → 03 01 02 03
Vec<i16> = [1, 2, 3]     → 03 02 04 06     （zigzag）
String = "hello"         → 05 68 65 6C 6C 6F
[u8; 3] = [1, 2, 3]      → 01 02 03        （无长度）
```

### 3.6 不变量

- **I1 唯一性**：同一格式版本 + 同一类型定义 ⇒ 同一字节序列。例外：`HashMap`/`HashSet` 的迭代顺序、NaN payload。
- **I2 自足性**：解码只依赖类型定义，不读数据内元数据。
- **I3 安全性**：任意输入不 panic、不越界、不无界分配。
- **I4 无额外分配**：编码除输出 buffer 外零分配。

---

## 4. 版本演进：字段个数到底能做什么

### 4.1 机制

结构体开头写 `u16le(nfields)`（2 字节），解码端知道自己的 `EXPECTED`：

| 情况 | 语义 | 处理 |
|---|---|---|
| `n == EXPECTED` | 正常 | 依次解码 |
| `n < EXPECTED` | 写端是旧版本 | 尾部字段用 `#[ordpack(default)]` 填默认值；未标注则 `MissingField` 错误 |
| `n > EXPECTED` | 写端是新版本 | 严格模式报 `FieldCount`；`compat` 模式按字节长度跳过尾部 |

### 4.2 允许 / 禁止的变更

| 变更 | 结果 | 说明 |
|---|---|---|
| 尾部追加字段 | ✅ 旧数据可读 | 新字段必须 `#[ordpack(default)]` |
| 删除尾部字段 | ✅ 新代码可读旧/新数据 | 旧数据多出的字段需要 `compat`，否则报错 |
| 给 `Option<T>` 字段加默认值 | ✅ | `Option` 的 `Default` 是 `None` |
| 修改字段类型 | ❌ | 线格式完全不兼容 |
| 交换字段顺序 | ❌ | 声明顺序即线格式 |
| 中间插入字段 | ❌ | 没有字段名/ID，无法对位 |
| 重命名字段 | ✅ | 名字不上线，仅影响错误信息 |
| 枚举尾部追加变体 | ✅ | tag 从 0 递增 |
| 枚举中间插入变体 | ❌ | 后续 tag 全部漂移（除非全部显式 `tag`） |
| 变体内部追加字段 | ✅ | 变体也带 `nfields` |

> 这是本设计最重要的一条纪律：**只允许在尾部追加**。重排字段 = 换格式。

### 4.3 `compat` 模式（可选，仅结构体）

> 已实现：`#[ordpack(compat)]` 结构与枚举不可混用；`compat` 与 `packed` 互斥，
> 且要求所有字段都能精确计算长度（不能有 `with` 字段）。

```rust
#[ordpack(compat)]
struct User { id: u64, name: String }
```

线格式变为：

```
u16le(nfields) | varint(byte_len) | fields...
```

解码时先记下 `end = pos + byte_len`，解完自己认识的字段后：

- 若 `n > EXPECTED`：`seek(end)` 跳过尾部未知字段（要求未知字段**只在尾部**）。
- 最后断言 `pos == end`，顺带做一次廉价的一致性校验。

代价：每结构体多 1–3 字节 + 一次位置记录。收益：读端可以比写端旧，适合「先发布新写端、再滚动升级读端」的场景。
默认关闭；对顶层记录或跨进程边界上的类型建议开启。

**实现约束（重要）**：`byte_len` 必须在写字段之前确定，而它是变长 varint。三种做法：

| 做法 | 成本 | 适用 |
|---|---|---|
| 精确长度预计算（`encoded_len()`） | 对字符串/容器多一次 O(n) 遍历 | 推荐，无额外缓冲 |
| 计数 writer 先跑一遍 `encode` | 约 1.5–2× 编码耗时 | 派生代码零改动 |
| 写进临时 `Vec` 再拷贝 | 一次额外分配 + memcpy | 兜底 |

因此 `compat` 只对**确实需要前向兼容**的类型开启，且优先用在嵌套结构的「外壳」上；
内层小结构体不加 `compat`，避免每层都做长度预计算。若 `encoded_len()` 返回 `None`，
编码器自动退化到临时缓冲方案，语义不变、速度变慢。

> ⚠️ 长度前缀只保证「能跳到结尾」，**不保证字段对位正确**：如果新版本在**中间**插入字段，
> 旧解码器会把后续字段读错位，而长度检查仍然通过（字节数正好对上）。
> 这种错误只能靠 §4.4 的指纹在加载时拦截——所以 `compat` 应当和指纹一起用，
> 并且把「只允许尾部追加」写进团队规范。

### 4.4 指纹（可选但强烈建议）

```rust
const FINGERPRINT: u64 = combine(
    base_hash("my_crate::User", FORMAT_VERSION),
    &[u64::FINGERPRINT, String::FINGERPRINT, bool::FINGERPRINT],
);
```

- 只哈希**结构**：类型路径 + 字段个数 + 每个字段类型的指纹（递归折叠成 u64），**不含字段名**。
- 重命名字段不改变指纹；改字段类型/顺序/个数会改变。
- 用法：文件/记录头 14 字节 `"MSER" | format:u8 | flags:u8 | fingerprint:u64`。
- 加载时比对失败 → `ErrorKind::SchemaMismatch`，比解码到一半才报错清晰得多。

### 4.5 演进流程示例

```rust
// v1
#[derive(Encode, Decode)]
struct Record { id: u64, name: String }

// v2：尾部追加，并给默认值
#[derive(Encode, Decode)]
struct Record {
    id: u64,
    name: String,
    #[ordpack(default)]
    score: u32,          // v1 数据解码为 0
}
```

- v1 数据 → v2 类型：✅ `n=2 < 3`，`score = 0`
- v2 数据 → v1 类型：❌（严格模式）或 ✅（`compat`，跳过尾部 1 个字段）

---

## 5. 借用与零拷贝

### 5.1 `Decode<'de>`

```rust
pub trait Decode<'de>: Sized {
    fn decode<R: Reader<'de>>(d: &mut Decoder<'de, R>) -> Result<Self, Error>;
}

impl<'de> Decode<'de> for &'de str {
    fn decode<R: Reader<'de>>(d: &mut Decoder<'de, R>) -> Result<Self, Error> {
        d.read_str()          // 校验 UTF-8 后直接返回切片，零拷贝
    }
}

impl<'de> Decode<'de> for String {
    fn decode<R: Reader<'de>>(d: &mut Decoder<'de, R>) -> Result<Self, Error> {
        Ok(d.read_str()?.to_owned())
    }
}
```

- `&'de str`、`&'de [u8]`：零拷贝。
- `&'de [u16]`：**不支持**（需要对齐 + 端序转换），解码成 `Vec<u16>`。
- `Cow<'de, str>`：输入是切片时借用，否则 `to_owned`。

### 5.2 流式解码

`Reader::read` 返回 `&'de [u8]` 只对切片成立；流式输入无法借用，因此提供两条路径：

```rust
// 读到结尾，解一个值（要求全部消费）
let value: User = ew_ordpack::from_reader(file)?;

// 逐条解码，缓冲区按需增长
let mut decoder = StreamDecoder::new(file);
let first: User = decoder.decode()?;
let second: User = decoder.decode()?;
```

```rust
pub trait DecodeOwned: Sized {
    fn decode_owned<R: Read>(d: &mut StreamDecoder<R>) -> Result<Self, Error>;
}

// 对所有权类型自动成立；借用类型不满足 for<'de> Decode<'de>，编译期即被拒绝
impl<T: for<'de> Decode<'de>> DecodeOwned for T { ... }
```

- `StreamDecoder` 在内部缓冲已读字节；每次 `decode` 只消费属于自己的字节，剩余留给下一次调用。
- 解码返回 `UnexpectedEof` 且尚未到文件尾时，继续 `read` 并重试。
- 缓冲区按几何增长（8 KiB 起，每次翻倍），把「跨多次读取的大值」从 O(n²) 重解码降到 O(n log n)。
- `into_parts()` 返回未消费的字节，便于把解码器交还给其他协议层。
- I/O 错误映射为 `ErrorKind::Io(std::io::ErrorKind)`。

`Reader::read` 返回 `&'de [u8]` 只对切片成立。流式 reader 无法返回借用，因此：

```rust
pub trait DecodeOwned: Sized {
    fn decode_owned<R: Read>(d: &mut StreamDecoder<R>) -> Result<Self, Error>;
}
```

- `from_slice` 需要 `T: Decode<'de>`（可借用）。
- `from_reader` 需要 `T: DecodeOwned`（`&'de str` 没有实现，编译期就能拦住）。
- 派生宏同时生成两个 impl；`DecodeOwned` 只是把 `read_str` 换成 `read_string`。

v1 只做 `from_slice` + `to_slice`/`to_vec`；流式放 v2。

---

## 6. API 与代码结构

### 6.1 crate 划分

```
ew_ordpack/                # facade：to_vec/from_slice + 常用类型 impl
ew_ordpack_core/           # 无 derive 的核心：trait、Encoder/Decoder、Error
ew_ordpack_derive/         # proc-macro：#[derive(Encode, Decode)]
```

v1 也可以先做成单 crate + `derive` feature，稳定后再拆。

### 6.2 核心 trait

```rust
pub trait Encode {
    /// 结构指纹：只依赖结构，不依赖字段名
    const FINGERPRINT: u64 = 0;

    /// 精确的编码后长度；`Some` 表示可在不编码的前提下算出。
    /// 用于一次分配到位，以及 `compat` 模式的长度前缀。
    #[inline]
    fn encoded_len(&self) -> Option<usize> { None }

    fn encode<W: Writer>(&self, e: &mut Encoder<W>) -> Result<(), Error>;
}

pub trait Decode<'de>: Sized {
    fn decode<R: Reader<'de>>(d: &mut Decoder<'de, R>) -> Result<Self, Error>;
}
```

### 6.3 Writer / Reader

```rust
pub trait Writer {
    fn write(&mut self, bytes: &[u8]) -> Result<(), Error>;
    #[inline] fn write_u8(&mut self, v: u8) -> Result<(), Error> { self.write(&[v]) }
    fn remaining(&self) -> Option<usize> { None }   // 定长 buffer 用
}

pub trait Reader<'de> {
    fn read(&mut self, len: usize) -> Result<&'de [u8], Error>;
    fn read_u8(&mut self) -> Result<u8, Error>;
    fn remaining(&self) -> usize;                   // 切片 reader 用
}
```

`Encoder` 持有 `W`，所有方法 `#[inline]`：

```rust
impl<W: Writer> Encoder<W> {
    #[inline] pub fn write_varint(&mut self, mut v: u64) -> Result<(), Error> {
        while v >= 0x80 {
            self.w.write_u8((v as u8) | 0x80)?;
            v >>= 7;
        }
        self.w.write_u8(v as u8)
    }
    #[inline] pub fn write_tag(&mut self, tag: u8) -> Result<(), Error> {
        self.w.write_u8(tag)
    }
    #[inline] pub fn write_u16(&mut self, v: u16) -> Result<(), Error> {
        self.w.write(&v.to_le_bytes())      // 字段个数：固定 2 字节
    }
    #[inline] pub fn write_bytes(&mut self, b: &[u8]) -> Result<(), Error> {
        self.write_varint(b.len() as u64)?;
        self.w.write(b)
    }
}
```

`Decoder` 对应提供 `read_u16()`（一次 2 字节读，无循环）与 `read_varint()`。

### 6.4 derive 展开

```rust
#[derive(Encode, Decode)]
struct User { id: u64, name: String, active: bool }
```

展开（简化）：

```rust
impl Encode for User {
    const FINGERPRINT: u64 = combine(base_hash("User", FORMAT_VERSION), &[
        u64::FINGERPRINT,
        String::FINGERPRINT,
        bool::FINGERPRINT,
    ]);

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        Some(2
            + self.id.encoded_len()?
            + self.name.encoded_len()?
            + self.active.encoded_len()?)
    }

    #[inline]
    fn encode<W: Writer>(&self, e: &mut Encoder<W>) -> Result<(), Error> {
        e.write_u16(3)?;                // 常量折叠：一条 2 字节写
        self.id.encode(e)?;
        self.name.encode(e)?;
        self.active.encode(e)?;
        Ok(())
    }
}

impl<'de> Decode<'de> for User {
    #[inline]
    fn decode<R: Reader<'de>>(d: &mut Decoder<'de, R>) -> Result<Self, Error> {
        let n = d.read_u16()?;
        if n == 3 {                     // 快路径：一次比较，直接构造
            Ok(User {
                id:     Decode::decode(d)?,
                name:   Decode::decode(d)?,
                active: Decode::decode(d)?,
            })
        } else {
            decode_user_slow(d, n)      // #[cold]：处理缺失/多余字段
        }
    }
}

    let __n = d.read_field_count()?;
    if __n == 3 {
        // 快路径：一次比较，直线解码
        ::core::result::Result::Ok(User {
            id:     Decode::decode(d)?,
            name:   Decode::decode(d)?,
            active: Decode::decode(d)?,
        })
    } else {
        // 冷路径：字段缺失时填默认值 / 报错
        if __n > 3 {
            return Err(d.err(ErrorKind::FieldCount { got: __n, expected: 3 }));
        }
        let id     = if __n > 0 { Decode::decode(d)? } else { return Err(d.missing("id")) };
        let name   = if __n > 1 { Decode::decode(d)? } else { return Err(d.missing("name")) };
        let active = if __n > 2 { Decode::decode(d)? } else { return Err(d.missing("active")) };
        ::core::result::Result::Ok(User { id, name, active })
    }
}
```

这样热路径只有「读计数 + 与常量比较 + 顺序解码」，慢路径虽然与快路径在同一个函数里，
但处于一个永不进入的分支，LLVM 会把它排到函数尾部（v0.1 未再拆成独立的 `#[cold]` 函数，
以避免派生代码翻倍；后续可改为按类型生成冷函数）。

带默认值时，慢路径对应字段换成：

```rust
let score = if __n > 3 { Decode::decode(d)? } else { <u32 as Default>::default() };
```

枚举展开：

```rust
impl Encode for Event {
    fn encode<W: Writer>(&self, e: &mut Encoder<W>) -> Result<(), Error> {
        match self {
            Event::Tick => e.write_varint(0),
            Event::Key(v0) => {
                e.write_varint(1)?;
                e.write_varint(1)?;
                v0.encode(e)
            }
            Event::Resize { w, h } => {
                e.write_varint(2)?;
                e.write_varint(2)?;
                w.encode(e)?;
                h.encode(e)
            }
        }
    }
}
```

### 6.5 属性清单

| 属性 | 作用域 | 语义 |
|---|---|---|
| `#[ordpack(default)]` | 字段 | 缺失时用 `Default`，并允许旧数据解码 |
| `#[ordpack(skip)]` | 字段 | 不编码、不计入 `nfields`，解码用 `Default` |
| `#[ordpack(default = "path")]` | 字段 | 自定义缺失值 |
| `#[ordpack(with = "mod")]` | 字段 | 自定义 `encode`/`decode` 函数 |
| `#[ordpack(transparent)]` | 类型 | 单字段结构体透明编码 |
| `#[ordpack(packed)]` | 类型 | 不写 `nfields`，版本锁定，最快最省；与 `compat` 互斥 |
| `#[ordpack(compat)]` | 类型 | 额外写字节长度，可跳过尾部未知字段；仅结构体，与 `packed` 互斥 |
| `#[ordpack(sort)]` | 字段（map/set） | 按编码后的键字节排序，保证确定性输出 |
| `#[ordpack(sort)]` | 字段（map/set） | 按编码后的键字节排序，保证确定性输出 |
| `#[ordpack(tag = N)]` | 变体 | 固定变体序号 |
| `#[ordpack(rename = "s")]` | 字段/变体 | **仅用于错误信息**，不上线 |
| `#[ordpack(bound = "...")]` | 类型 | 覆盖自动生成的 where 子句 |
| `#[ordpack(crate = "...")]` | 类型 | 重命名生成的路径 |
| `#[ordpack(flatten)]` | — | **不支持**（需要字段名做键） |

### 6.6 错误类型

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Error { kind: ErrorKind, offset: u32 }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    UnexpectedEof,
    InvalidBool(u8),
    InvalidChar(u32),
    InvalidUtf8,
    InvalidTag { got: u64, max: u64 },
    InvalidVariant { got: u64, variants: u64 },
    FieldCount { got: u16, expected: u16 },
    MissingField(&'static str),
    LengthOverflow,
    NestingLimit,
    VarintOverflow,
    SchemaMismatch,
    TrailingBytes,
    BufferFull,
    Custom(&'static str),
}
```

- `Copy` + 无 `String`：`no_std` 友好，错误路径不分配。
- `Display` 只在用户打印时格式化。
- 每个错误构造点标 `#[cold]`，避免污染热路径的 icache / 抑制内联。

### 6.7 feature flags

| feature | 默认 | 说明 |
|---|---|---|
| `std` | ✅ | `Vec`/`HashMap`/`io` |
| `alloc` | ✅ | 仅 `alloc` 环境 |
| `derive` | ✅ | 派生宏 |
| `fixed-int` | ❌ | 整数用定长 LE 而非 varint |
| `native-endian` | ❌ | 浮点/定长整数用本机端序（仅同架构读写） |
| `simdutf8` | ❌ | 用 SIMD 校验 UTF-8 |
| `unchecked` | ❌ | 提供 `to_slice_unchecked`（unsafe，仅编码；解码仍保留检查） |
| `compat` | ❌ | 全局默认开启 `compat` |
| `strict` | ❌ | 拒绝非规范 varint，保证字节唯一 |
| `trace` | ❌ | 解码错误带路径（`people[1].age`） |

### 6.8 对外 API（最小集合）

```rust
use ew_ordpack::{Encode, Decode};

#[derive(Encode, Decode, PartialEq, Debug)]
struct User { id: u64, name: String, active: bool }

let user = User { id: 1, name: "ab".into(), active: true };

// 编码到 Vec（一次分配）
let bytes = ew_ordpack::to_vec(&user)?;

// 编码到已有缓冲区（零分配，最快路径）
let mut buf = [0u8; 64];
let used = ew_ordpack::to_slice(&user, &mut buf)?;

// 解码：借用版（可零拷贝）
let decoded: User = ew_ordpack::from_slice(&bytes)?;

// 可选：带指纹头的记录格式
let record = ew_ordpack::to_record(&user)?;          // "MSER" + 指纹 + 长度 + payload
let user2: User = ew_ordpack::from_record(&record)?; // 指纹不匹配则 SchemaMismatch
```

---

## 7. 性能工程

### 7.1 为什么能快

1. **单态化**：`Encode<W>`、`Decode<'de, R>` 全泛型，无 `dyn`、无 visitor、无 `Serializer` 抽象层。
2. **常量折叠**：`nfields`、变体 tag、字段偏移都是编译期常量；`write_u16(3)` 退化成一条 2 字节写。
3. **零元数据**：没有 `serialize_struct("User", 3)`，没有字段名比较，没有字符串 hash。
4. **一次分配**：`encoded_len()` 让 `to_vec` 一次 `with_capacity`；编码过程零 realloc、零临时分配。
5. **零拷贝解码**：`&'de str` / `&'de [u8]` 直接切片，不做 `to_owned`。
6. **少分支**：varint 常见值 1 字节；`fixed-int` 模式下整数是纯 `mov`。
7. **冷错误路径**：错误构造全部 `#[cold]` + 单独函数，热路径只剩 `test + jcc`。
8. **缓冲区直写**：`to_slice` 用 `&mut [u8]`，容量检查在入口做一次，后续 `copy_nonoverlapping`。

### 7.2 预期代码形态

```rust
// struct Point { x: i32, y: i32 }，fixed-int + native-endian
#[inline]
fn encode(&self, e: &mut Encoder<W>) -> Result<(), Error> {
    // 编译器可以把下面三次写合并成一次 10 字节拷贝（2 + 4 + 4）
    e.write_u16(2)?;
    e.write_i32(self.x)?;
    e.write_i32(self.y)?;
    Ok(())
}
```

### 7.3 基准计划

工作负载：

| 负载 | 说明 |
|---|---|
| `Point { x: i32, y: i32 }` | 纯定长小结构体，测极限吞吐 |
| `Vec<u32>` × 1000 | 测序列路径与主循环 |
| `HashMap<String, u64>` × 100 | 测字符串与 map |
| 嵌套树（3 层，混合 Option/Vec/String） | 测真实业务形态 |

对比对象：`bincode 2`、`postcard`、`musli`（storage / packed）、`rkyv`（归档访问）、`serde_json`（基线）。

指标：encode ns/op、decode ns/op、字节数、峰值分配次数、`cargo bloat` 代码体积。

**验收门槛（v0.3 实测，多轮取中位）**

在 `x86_64-pc-windows-gnu`、`release` + thin LTO + `codegen-units = 1`
（见 `Cargo.toml` 的 `[profile.release]`）、`cargo run --release --example bench` 下：

| 负载 | 大小 | encode | decode | 备注 |
|---|---|---|---|---|
| `Point { i32, i32 }` | 4 B | **1.2 ns** | **4.3 ns** | 核心路径 |
| `Point`（`packed`） | 2 B | 1.0 ns | 4.1 ns | 去掉字段计数，与 bincode 同字节数 |
| `User { u64, String, bool, f64 }` | 22 B | 3.0 ns | 29.8 ns | 字符串解码含一次 `to_owned` |
| `Vec<Point>` × 1000 | 5873 B | 1.9 GB/s | 1.3 GB/s | |
| `Vec<u32>` × 1000 | 1874 B | 2.9 GB/s | 1.0 GB/s | |
| `Vec<u8>` × 1 KiB | 1026 B | 44 GB/s | 1.6 GB/s | 编码被向量化；解码逐字节 |
| `Bytes<Vec<u8>>` × 1 KiB | 1026 B | 124 GB/s | **29 GB/s** | 批量 memcpy |
| `Bytes<Vec<u8>>` × 64 KiB | 65539 B | 69 GB/s | **3.8–15 GB/s** | 分配器相关波动 |
| `String` × 1 KiB | 1026 B | 128 GB/s | 17.8 GB/s | 批量 memcpy |
| `HashMap<String,u64>` × 100 | 793 B | 2.4 GB/s | 0.16 GB/s | 解码受 SipHash + String 分配支配 |
| `HashMap`（加 `sort`） | 793 B | 0.23 GB/s | — | 慢约 10×，确定性输出的代价 |
| `User`（`compat`） | 23 B | 3.5 ns | 31.7 ns | 比普通模式慢 ~15% |
| `Point`（bincode 2 对照） | 2 B | 1.3 ns | 5.6 ns | |
| `Point`（无检查手写） | 4 B | 1.4 ns | 1.6 ns | 无 `Result`、无边界检查 |
| `Point`（`unchecked` feature） | 4 B | **0.8 ns** | — | 仅编码，解码保持安全 |

- **v0.3 的关键改动是内联**：派生出的 `encode`/`decode` 与 `to_slice`/`from_slice` 改为
  `#[inline(always)]` 后，`Point` 从 4.3 ns / 6.4 ns 降到 1.2 ns / 4.3 ns。
- 编码 **0.92×** bincode 2（略快），解码 **0.72×**（快约 30%）；`packed` 版本与 bincode
  同为 2 字节时是 0.81× / 0.84×。
- 对无检查手写：编码 **0.85×**（更快），解码 2.5×。解码差在 `Result` 传播与边界检查，
  而不是算法；这两项都不能为不可信输入去掉。
- `Bytes<T>` 让 `Vec<u8>` 解码从 1.6 GB/s 提升到 3.8–29 GB/s（7–16×，小对象更明显），
  编码持平或更快。
- `unchecked` feature 只对编码生效（`to_slice_unchecked`，约再快 25%）；解码不能跳过边界检查，
  因为输入不可信——这是刻意保留的安全边界。
- 体积：`Point` 4 B vs bincode 2 B（+2 B 字段计数）；字段越多、整数越小，varint 越能抵消。
- `to_vec` 的 32 ns 几乎全是分配开销（`Point` 只有 22 字节），不是 codec 成本。

> 基准本身的两处修正：编码循环每轮 `black_box(&mut buf)`（避免跨迭代死代码消除），
> 且对被测值与对照组使用同样的 `black_box`（此前 bincode 入参未隔离，会被常量折叠）。

**CI 基线**：`ci/budgets.txt` 定义**同进程比值**门槛（例如 `point_dec_vs_bincode ≤ 1.2`、
`bytes_1k_dec_speedup ≥ 5`），由 `cargo run --release --example bench -- --check ci/budgets.txt`
判定；绝对耗时只写入 `ci/baseline.json` 快照和 `target/bench.json` 工件，不做跨机器比较。

**CI 基线**：`ci/budgets.txt` 定义**同进程比值**门槛（例如 `point_dec_vs_bincode ≤ 1.2`、
`bytes_1k_dec_speedup ≥ 5`），由 `cargo run --release --example bench -- --check ci/budgets.txt`
判定；绝对耗时只写入 `ci/baseline.json` 快照和 `target/bench.json` 工件，不做跨机器比较。

### 7.4 代码体积

不要为每个类型生成 `Shape` 静态表（那是 musli/reflection 路线）。派生只生成函数体，链接器可裁剪未用类型。
代价是失去运行期自省；需要调试时用 `trace` feature 或单独的 `describe` 工具（v3）。

---

## 8. 安全与鲁棒性

| 威胁 | 防御 |
|---|---|
| 超长 `len` 触发巨量分配 | 先 `len as usize` 溢出检查；`len > remaining` 直接报错；`Vec::try_reserve` 失败返回错误 |
| 长度 × 元素大小溢出 | `checked_mul` |
| 深层递归爆栈 | 解码器维护 `depth: u16`，上限 128（可配置） |
| 非法 UTF-8 | `str::from_utf8` / `simdutf8` |
| 非法 bool / char / enum tag / Option tag | 逐一校验，越界即错 |
| 尾部垃圾数据 | `from_slice` 默认要求消费完所有字节；`from_slice_prefix` 返回已用长度 |
| 恶意 `nfields` | u16 天然有界；`n > EXPECTED` 立即报错 |
| 解码 panic | 全 `Result`；fuzz 验证 |

---

## 9. 与现有方案对比

| 方案 | 字段元数据 | 缺失字段 | 未知字段 | 零拷贝 | 备注 |
|---|---|---|---|---|---|
| bincode 2 | 无 | ✗ | ✗ | ✗ | 最快之一，但完全版本锁定 |
| postcard | 无 | ✗ | ✗ | 部分 | 面向嵌入式，varint |
| rkyv | 无（归档布局） | ✗ | ✗ | ✅ | 访问零拷贝，但需要校验、布局受限 |
| musli storage | 字段 ID（默认数字索引） | ✅ | ✗ | ✅ | 可容忍重排；packed 模式去掉 ID 但也放弃升级 |
| **本设计** | **仅字段个数（u16）** | ✅ | ✗（compat 可跳过尾部） | ✅（`&str`/`&[u8]`） | 每结构体 2 字节，换取版本检测 + 缺失填充 |

与 musli 的关系：

- 同样走「schema 驱动 + 无 visitor」路线；
- musli 的 `storage` 保留字段标识以支持重排，`packed` 去掉标识但彻底放弃演进；
- 本设计取中间点：**保留 2 字节的字段个数**（u16，常量折叠，几乎零成本），不保留任何标识。
  这正好是「存储场景、只追加字段」的最优折中。

---

## 10. 风险与缓解

| 风险 | 影响 | 缓解 |
|---|---|---|
| 无法跳过未知字段 | 读端不能比写端旧 | `compat` 模式；发布顺序约定（先升读端）；版本号 + 指纹 |
| 字段重排/中间插入 | 静默解错（最危险） | 指纹头 + golden test + CI 中跑「上一版本 fixture」 |
| `HashMap` 字节不稳定 | 哈希/去重/对比失效 | 文档明确；提供 `#[ordpack(sort)]`；推荐 `BTreeMap` |
| `usize`/`Path` 平台差异 | 不可移植 | `usize` 固定按 u64；路径类类型文档标注 |
| derive 泛型/生命周期复杂 | 编译错误难懂 | `trybuild` 快照测试 + 清晰 span |
| 安全与极致性能冲突 | 边界检查开销 | 默认安全；`unchecked` feature 给需要的人 |
| 无自描述导致排障困难 | 线上脏数据难分析 | `trace` feature + 可选 `Value` 树（feature） |

---

## 11. 路线图

| 阶段 | 内容 | 产出 | 估时 |
|---|---|---|---|
| M0 | workspace、trait 骨架、Error、Writer/Reader | `cargo test` 绿 | 0.5d |
| M1 | 基础类型 + struct/tuple/unit + `to_vec`/`from_slice` | 第一版可用 | 1d |
| M2 | enum、泛型、集合、`Decode<'de>` 借用 | 类型矩阵全覆盖 | 1d |
| M3 | 属性（default/skip/with/tag/packed/compat/transparent）+ 版本 fixture 测试 | 演进能力闭环 | 1d |
| M4 | 基准 + 针对性优化（encoded_len、冷路径、buffer 直写） | 基准报告 | 1d |
| M5 | `no_std`、fuzz、文档、CI | v0.1 发布 | 1d |

每个阶段的验收：`cargo test` + `cargo clippy -D warnings` + 该阶段基准不回退。

---

## 12. 待你拍板的问题

1. **整数默认**：varint（体积小，推荐）还是定长 LE（速度略快、可预测）？
2. **`compat` 模式**：要不要作为一等公民？它会改变默认线格式（每结构体多 1–3 字节）。
3. **目标场景**：纯本地存储，还是也要跨进程/跨版本滚动升级？后者建议默认开 `compat` + 指纹。
4. **`no_std`**：现在就要，还是 v1 之后再加？
5. **字段重排**：确认接受「重排 = 破坏性变更」吗？如果不能接受，就必须存字段 ID（回到 musli storage 路线）。
6. **指纹**：是否接受在文件头放 14 字节 schema 指纹？

以上 1–6 若都按推荐值（varint / compat 可选 / 存储场景 / 暂不 no_std / 接受不可重排 / 要指纹），
可以直接进入 M0。

---

## 13. 实现状态（v0.1）

### 13.1 已实现

**核心**
- `Error` / `ErrorKind`（含字节偏移；`Copy`，无分配）
- `Writer`：`Vec<u8>`、`&mut W`、`SliceWriter`（定长缓冲区）
- `Reader` / `SliceReader`（借用切片，零拷贝）
- `Encoder` / `Decoder`：varint、zigzag、定长 `u16` 字段计数、长度前缀、嵌套深度限制
- `Encode` / `Decode` trait，`to_vec` / `to_slice` / `from_slice` / `from_slice_prefix`
- `FINGERPRINT`：结构指纹（类型路径 + 字段类型，**不含字段名**）

**类型矩阵**
- 整数（含 128 位）、浮点、`bool`、`char`、`()`
- `String` / `&str` / `&[u8]` / `Vec` / 数组 / 元组（1–12）/ 切片
- `Option` / `Result`
- `VecDeque` / `LinkedList` / `BinaryHeap` / `BTreeMap` / `BTreeSet`
- `HashMap` / `HashSet`（std）
- `Box` / `Rc` / `Arc` / `Cow` / `Wrapping` / `Reverse` / `Cell` / `RefCell` / `Mutex` / `RwLock` / `PhantomData`
- `NonZero*` / `Duration` / `SystemTime` / `IpAddr` / `SocketAddr`
- `Path` / `PathBuf` / `OsStr` / `OsString` / `CString` / `CStr`（std，要求 UTF-8）

**derive**
- 具名 / 元组 / unit / newtype 结构体
- 三种枚举变体（unit / tuple / struct），显式 `tag`
- 泛型、生命周期、const 泛型（透传）
- 属性：`default`、`skip`、`with`、`tag`、`transparent`、`packed`、`rename`、`bound`、`crate`

**版本与安全**
- 字段数快路径 + 缺失字段填默认值（`#[ordpack(default)]`）
- `compat` 模式：`u16le(count) | varint(payload_len) | fields`，旧类型可跳过新版本追加的尾部字段
- `#[ordpack(sort)]`：HashMap / HashSet 按编码后的键字节排序，输出与哈希种子无关
- 流式解码：`from_reader`、`StreamDecoder`（逐条、按需增长）、`DecodeOwned`、`ErrorKind::Io`
- 批量字节：`Bytes<Vec<u8>>` / `Bytes<&[u8]>` / `Bytes<[u8; N]>` / `Bytes<Box<[u8]>>`（线格式与未包装类型一致）
- `unchecked` feature：`to_slice_unchecked`（`unsafe fn`，仅编码路径）
- 长度前缀先校验剩余字节，避免超长分配
- 嵌套深度上限 128
- 默认 `#![forbid(unsafe_code)]`
- `no_std`：core-only 与 `alloc` 两种构建均通过

### 13.2 尚未实现（与本文档的差距）

| 功能 | 状态 |
|---|---|
| `fixed-int` / `native-endian` / `simdutf8` / `trace` feature | 未实现 |
| `Value` 动态树、`describe` 调试工具 | 未实现 |
| `compat` 用于枚举 | 未实现（结构体已支持） |
| cargo-fuzz 接入 CI | 目标已就绪，未接入（需要 nightly 与较长运行时间） |

### 13.3 与设计的偏差

- **快路径未拆冷函数**：在同一函数内用 `if __n == N { 快 } else { 慢 }`，避免派生代码翻倍；
  慢路径分支不会进入，LLVM 会把它排到函数尾部。
- **错误保留字段名**：`ErrorKind::MissingField(&'static str)` 让 `Error` 变成 32 字节。
  实测（对照 8 字节错误）对热点路径无影响，因为错误分支从不执行。
- **指纹改为浅层 `KIND` 组合**：若直接递归 `T::FINGERPRINT`，递归类型
  （如 `enum Tree { Node(Vec<Tree>) }`）会导致 const 求值循环。现在每个类型提供
  一个不依赖自身字段的 `KIND`，`FINGERPRINT = hash(路径, 字段数, 各字段 KIND)`。
  代价是：字段类型替换只有在浅层标签不同时才能被发现（例如两个同为 `u32` 的字段互换位置无法识别）。
- **`compat` 仅结构体**：枚举变体的长度前缀尚未实现。
- **`unchecked` 只覆盖编码**：解码输入不可信，边界检查是安全前提，不能跳过；
  实测跳过检查的收益也主要来自内联而非去检查（见 §7.3）。
- **不做 UB 解释器验证**：构建、测试与 CI 全部使用 stable；`unchecked` 的 unsafe 路径由
  `tests/unchecked.rs` 和 `tests/coverage.rs` 的运行时断言覆盖。
- **networking 类型来自 `core::net`**：因此 `no_std` 下也可用，要求 Rust ≥ 1.77。

### 13.4 测试与模糊测试

- `cargo test`：83 个测试——6 个 bytes、6 个 compat、**32 个 coverage**、4 个 golden、
  4 个 fuzz smoke、16 个 roundtrip、3 个 sort、9 个 streaming、3 个文档测试；
  `--features unchecked` 额外 1 个。
- `tests/coverage.rs` 逐项覆盖所有 `Encode`/`Decode` 实现：全部整数边界、浮点位模式
  （含 NaN/-0.0/subnormal）、char 边界与非法标量、引用/切片/str、四种 `Bytes`、1–12 元组、
  LinkedList/BinaryHeap、全部 12 种 NonZero、SystemTime 纪元前后、SocketAddrV6、
  OsString/CString/Path 及非法 UTF-8（Windows）、递归枚举、const 泛型、`bound`/`crate`/`rename`、
  截断前缀扫描、超长长度、字段数不匹配、嵌套深度上限。
- `tests/fuzz_smoke.rs` 用确定性 PRNG 覆盖：任意字节解码不 panic、解码值可重编码、
  随机值 roundtrip、前缀解码只消费自身字节。
- `tests/streaming.rs` 覆盖：`from_reader`、逐条解码、3 字节小块的碎片读取、256 KiB 大值、
  嵌套枚举、I/O 错误映射、`into_parts`。
- `tests/bytes.rs` 覆盖：与 `Vec<u8>` / `[u8; N]` 的线格式互通、零拷贝借用、1 MB 往返。
- `fuzz/` 提供可选的 cargo-fuzz 目标（`decode`、`roundtrip`），需要 nightly + cargo-fuzz；
  不参与构建、测试与 CI。
- CI：`.github/workflows/ci.yml` 全部使用 stable，跑 fmt / clippy / 测试 / no_std 构建 / 性能预算。
  性能门槛是同进程比值（`ci/budgets.txt`），不是绝对耗时；完整指标作为工件上传。

### 13.5 复现

```bash
cargo test
cargo run --release --example bench
cargo build --no-default-features --features derive        # core-only
cargo build --no-default-features --features alloc,derive  # no_std + alloc
```
