# ew-ordpack 用户手册

> 版本：0.1.0  
> 适用：Rust 1.77+（`std` 默认开启；支持 `no_std` + `alloc`）  
> 配套示例：`examples/demo_basic.rs`、`demo_file.rs`、`demo_streaming.rs`、`demo_versioning.rs`

---

## 目录

1. [它是什么](#1-它是什么)
2. [安装](#2-安装)
3. [快速开始](#3-快速开始)
4. [核心概念](#4-核心概念)
5. [线格式](#5-线格式)
6. [类型支持](#6-类型支持)
7. [属性参考](#7-属性参考)
8. [API 参考](#8-api-参考)
9. [使用场景与示例](#9-使用场景与示例)
10. [错误处理](#10-错误处理)
11. [版本演进](#11-版本演进)
12. [性能](#12-性能)
13. [no_std 使用](#13-no_std-使用)
14. [限制与 FAQ](#14-限制与-faq)
15. [命令速查](#15-命令速查)

---

## 1. 它是什么

**ew-ordpack** 是一个 schema 驱动的二进制序列化库：

- 线格式里**不写字段名、不写字段 ID**，结构体只带一个 **2 字节的字段个数**；
- 字段按**声明顺序**排列，解码完全依赖类型定义；
- 编码/解码在编译期展开成直线代码，无 visitor、无运行期形状表；
- 默认零 `unsafe`，支持 `no_std`；
- 版本演进规则是**只在尾部追加字段**（缺失字段用默认值补齐）。

它适合：本地存储、快照、缓存、日志、进程间传递等「读写双方共享同一份类型定义」的场景。

它不适合：需要自描述、需要旧读端理解新字段、需要跨语言互操作的协议。

---

## 2. 安装

```toml
[dependencies]
ew-ordpack = { path = "../ew-ordpack" }
```

发布后可写 `ew-ordpack = "0.1"`。

### Feature 一览

| feature | 默认 | 说明 |
|---|---|---|
| `std` | ✅ | `HashMap`、`Path`、`SystemTime`、`Mutex`、流式 `from_reader` 等 |
| `alloc` | ✅（由 std 引入） | `Vec`、`String`、`BTreeMap` 等 |
| `derive` | ✅ | `#[derive(Encode, Decode)]` |
| `unchecked` | ❌ | 提供 `unsafe fn to_slice_unchecked` |

```toml
# no_std + alloc
ew-ordpack = { path = "../ew-ordpack", default-features = false, features = ["alloc", "derive"] }
```

---

## 3. 快速开始

```rust
use ew_ordpack::{Decode, Encode};

#[derive(Encode, Decode, PartialEq, Debug)]
struct User {
    id: u64,
    name: String,
    active: bool,
}

fn main() -> Result<(), ew_ordpack::Error> {
    let user = User { id: 1, name: "ab".into(), active: true };

    let bytes = ew_ordpack::to_vec(&user)?;          // 7 字节
    let back: User = ew_ordpack::from_slice(&bytes)?;

    assert_eq!(back, user);
    Ok(())
}
```

编码后的字节：

```
03 00 | 01 | 02 61 62 | 01
字段数   id    "ab"     active
```

运行完整示例：

```bash
cargo run --example demo_basic
```

---

## 4. 核心概念

### 4.1 Schema 就是类型本身

没有 `.proto`、没有注册表、没有字段名表。解码器从 `T` 的类型定义直接知道你期望多少个字段、每个字段是什么类型。

### 4.2 字段数 + 声明顺序

- 结构体开头写 `u16le(字段数)`（小端 2 字节）；
- 之后按声明顺序依次写字段，没有任何分隔符或标记；
- 解码时先读字段数，与类型期望值比较：
  - 相等 → 正常解码；
  - 小于期望 → 尾部字段缺失，用 `#[ordpack(default)]` 补齐；
  - 大于期望 → 报 `FieldCount`（严格模式）或跳过（`compat` 模式）。

### 4.3 零拷贝

`&'de str` 和 `&'de [u8]` 直接借用输入切片，不复制。只适用于切片输入（`from_slice`）。

### 4.4 安全的演进方式

唯一安全的变更：**在尾部追加字段**。不能重排、不能在中间插入、不能改字段类型。详见[版本演进](#11-版本演进)。

---

## 5. 线格式

### 5.1 基础编码

| 编码 | 规则 |
|---|---|
| `varint` | LEB128 无符号，1–10 字节（`u128` 最多 19 字节） |
| `zigzag` | 有符号整数先 zigzag 再 varint |
| `f32`/`f64` | IEEE-754 位模式，小端 |
| `bool` | `0x00` / `0x01` |
| `char` | UTF-32 标量值的 varint |
| 字符串/字节串 | `varint(字节长)` + 内容 |
| 序列/映射 | `varint(元素数)` + 元素 |
| 数组 `[T; N]` | 恰好 N 个元素，无长度 |
| `Option` / `Result` | 1 字节 tag |
| 结构体 | `u16le(字段数)` + 字段 |
| 枚举 | `varint(变体号)` + 负载 |

### 5.2 结构体与枚举

```
struct Point { x: i32, y: i32 }
Point { x: 1, y: -2 }        → 02 00 | 02 | 03

enum Message { Ping, Move { x: i32, y: i32 }, Text(String) }
Ping                          → 00
Move { x: 1, y: 2 }           → 01 | 02 00 | 02 | 04
Text("hi")                    → 02 | 01 00 | 02 68 69
```

- 枚举的变体号按声明顺序从 0 开始，可用 `#[ordpack(tag = N)]` 钉死；
- 带字段的变体在 tag 之后写 `u16le(字段数)`；
- unit 变体只写 tag。

### 5.3 `compat` 模式

```rust
#[ordpack(compat)]
struct Record {
    id: u64,
    #[ordpack(default)]
    score: u32,
}
```

线格式变为：

```
u16le(字段数) | varint(负载长度) | 字段...
```

旧类型解码新数据时，可以按负载长度跳过尾部未知字段。代价是每结构体多 1–3 字节。

---

## 6. 类型支持

### 6.1 标准类型

| 类别 | 类型 |
|---|---|
| 整数 | `u8` `u16` `u32` `u64` `u128` `usize` `i8` `i16` `i32` `i64` `i128` `isize` |
| 浮点 | `f32` `f64` |
| 其他标量 | `bool` `char` `()` |
| 字符串 | `String` `&str` `Box<str>`（仅编码） |
| 引用 | `&T`（仅编码）`&mut T`（仅编码） |
| 智能指针 | `Box<T>` `Rc<T>` `Arc<T>` `Cow<'_, T>` |
| 包装 | `PhantomData<T>` `Wrapping<T>` `Reverse<T>` `Cell<T>` `RefCell<T>` |
| 同步 | `Mutex<T>` `RwLock<T>`（std） |
| 非零整数 | `NonZeroU8`…`NonZeroU128`、`NonZeroI8`…`NonZeroI128`、`NonZeroUsize` `NonZeroIsize` |
| 时间 | `Duration` `SystemTime`（std） |
| 网络 | `IpAddr` `SocketAddr`（含 V6 的 flowinfo/scope_id） |
| 路径/OS | `Path`（仅编码）`PathBuf` `OsStr`（仅编码）`OsString` |
| C 字符串 | `CStr`（仅编码）`CString` |
| 批量字节 | `Bytes<Vec<u8>>` `Bytes<&[u8]>` `Bytes<[u8; N]>` `Bytes<Box<[u8]>>` |

### 6.2 容器类型

`Vec<T>`、`&[T]`、`[T; N]`、`VecDeque<T>`、`LinkedList<T>`、`BinaryHeap<T>`、`BTreeMap<K,V>`、`BTreeSet<T>`、`HashMap<K,V>`、`HashSet<T>`、`Option<T>`、`Result<T,E>`、1–12 元组。

### 6.3 自定义类型

`#[derive(Encode, Decode)]` 支持：

- 具名结构体、元组结构体、unit 结构体、newtype；
- 枚举的 unit / tuple / struct 变体；
- 泛型、生命周期、const 泛型；
- `#[ordpack(...)]` 属性。

---

## 7. 属性参考

### 7.1 类型级

| 属性 | 作用 | 约束 |
|---|---|---|
| `#[ordpack(transparent)]` | 单字段透明编码，等同内部类型 | 恰好一个参与编码的字段 |
| `#[ordpack(packed)]` | 不写字段计数，版本锁定 | 与 `compat` 互斥 |
| `#[ordpack(compat)]` | 额外写负载长度，旧类型可跳过新尾部 | **仅结构体**；与 `packed` 互斥；不能含 `with` |
| `#[ordpack(bound = "T: Copy")]` | 追加 where 约束 | 字符串里写谓词 |
| `#[ordpack(crate = "ew_ordpack")]` | 重命名生成的 crate 路径 | 字符串 |

> newtype（单字段元组结构体）**默认透明**，不需要显式标注。

### 7.2 字段级

| 属性 | 作用 |
|---|---|
| `#[ordpack(default)]` | 缺失时用 `Default::default()` |
| `#[ordpack(default = "path")]` | 缺失时调用 `path()` |
| `#[ordpack(skip)]` | 不编码、不计入字段数，解码用默认值 |
| `#[ordpack(with = "module")]` | 自定义编码/解码函数 |
| `#[ordpack(rename = "x")]` | 仅影响错误信息，不上线 |
| `#[ordpack(sort)]` | map/set 按编码后的键字节排序（确定性输出） |

### 7.3 变体级

| 属性 | 作用 |
|---|---|
| `#[ordpack(tag = N)]` | 固定变体序号 |
| `#[ordpack(rename = "x")]` | 仅影响错误信息 |

### 7.4 `with` 模块的写法

```rust
mod timestamp {
    use ew_ordpack::{Decoder, Encoder, Error, Reader, Writer};

    pub fn encode<W: Writer>(value: &MyTime, e: &mut Encoder<W>) -> Result<(), Error> {
        e.write_varint(value.secs)
    }

    pub fn decode<'de, R: Reader<'de>>(d: &mut Decoder<'de, R>) -> Result<MyTime, Error> {
        Ok(MyTime { secs: d.read_varint()? })
    }
}

#[derive(Encode, Decode)]
struct Record {
    #[ordpack(with = "timestamp")]
    at: MyTime,
}
```

含 `with` 字段的类型，`encoded_len()` 返回 `None`（无法预计算长度）。

---

## 8. API 参考

### 8.1 编码

```rust
pub fn to_vec<T: Encode + ?Sized>(value: &T) -> Result<Vec<u8>, Error>;
pub fn to_slice<T: Encode + ?Sized>(value: &T, out: &mut [u8]) -> Result<usize, Error>;
```

- `to_vec`：一次分配到位（使用 `encoded_len`），最省事；
- `to_slice`：写入已有缓冲区，编码过程零分配；缓冲区不足返回 `BufferFull`；
- 两者输出逐字节一致。

```rust
#[cfg(feature = "unchecked")]
pub unsafe fn to_slice_unchecked<T: Encode + ?Sized>(value: &T, out: &mut [u8]) -> Result<usize, Error>;
```

`unchecked` 版本跳过逐次边界检查，调用方必须保证 `out.len() >= encoded_len`。

### 8.2 解码

```rust
pub fn from_slice<'de, T: Decode<'de>>(bytes: &'de [u8]) -> Result<T, Error>;
pub fn from_slice_prefix<'de, T: Decode<'de>>(bytes: &'de [u8]) -> Result<(T, usize), Error>;
pub fn from_reader<R: std::io::Read, T>(reader: R) -> Result<T, Error>
where T: for<'de> Decode<'de>;
```

- `from_slice` 要求**消费完所有字节**，有剩余报 `TrailingBytes`；
- `from_slice_prefix` 只解一个值，返回 `(值, 已用字节数)`；
- `from_reader` 读到 EOF 后解一个值，同样要求全部消费；借用类型（`&str`/`&[u8]`）不满足 `for<'de>`，编译期即被拒绝。

### 8.3 流式解码

```rust
pub struct StreamDecoder<R> { /* ... */ }

impl<R> StreamDecoder<R> {
    pub fn new(reader: R) -> Self;
    pub fn buffered(&self) -> &[u8];
    pub fn is_eof(&self) -> bool;
    pub fn into_parts(self) -> (R, Vec<u8>);
}

impl<R: Read> StreamDecoder<R> {
    pub fn decode<T>(&mut self) -> Result<T, Error>
    where T: for<'de> Decode<'de>;
}
```

用于「多条记录拼接」的流。每次 `decode` 只消费一条记录，剩余字节留在内部缓冲区。

### 8.4 `Bytes<T>`

```rust
pub struct Bytes<T>(pub T);
```

线格式与未包装类型一致，但编码/解码走批量 memcpy：

- `Bytes<Vec<u8>>`、`Bytes<Box<[u8]>>`、`Bytes<&[u8]>`；
- `Bytes<[u8; N]>`。

### 8.5 其他

```rust
pub const FORMAT_VERSION: u8;   // 当前为 1
pub const MAX_DEPTH: u16;       // 128
pub trait Encode {
    const FINGERPRINT: u64;     // 结构指纹（不含字段名）
    const KIND: u64;            // 指纹组合用的浅层标签
    fn encoded_len(&self) -> Option<usize>;
    fn encode<W: Writer>(&self, e: &mut Encoder<W>) -> Result<(), Error>;
}
pub trait DecodeOwned { fn decode_owned<R: Read>(d: &mut StreamDecoder<R>) -> Result<Self, Error>; }
pub trait SortedEncode { fn encode_sorted<W: Writer>(&self, e: &mut Encoder<W>) -> Result<(), Error>; }
```

`FINGERPRINT` 只包含类型路径、字段数和每个字段的浅层 `KIND`，**不含字段名**。重命名字段不改变指纹；追加/删除字段、改字段类型会改变指纹。两个同类型字段互换位置**不会**被发现，需要靠 golden 测试守住。

---

## 9. 使用场景与示例

所有示例都可以直接运行。

### 9.1 基础读写

```bash
cargo run --example demo_basic
```

覆盖：结构体、嵌套容器、`to_slice`、零拷贝 `&str`、枚举、`Bytes`、`BTreeMap`、错误信息。

### 9.2 单条记录存文件

```bash
cargo run --example demo_file
```

文件布局：

```
[4B magic][1B 格式版本][8B schema 指纹][payload]
```

加载时先校验 magic/版本/指纹，再解码 payload。示例输出：

```
saved: 22 bytes (header 13 + payload 9)
loaded: Config { id: 7, name: "demo", retries: 3 }
tampered header -> Some("schema fingerprint mismatch")
```

### 9.3 多条记录流式读写

```bash
cargo run --example demo_streaming
```

写端：逐条 `to_vec` 后追加。

读端：

```rust
let mut decoder = StreamDecoder::new(BufReader::new(file));
loop {
    match decoder.decode::<Row>() {
        Ok(row) => process(row),
        Err(e) if e.kind() == ErrorKind::UnexpectedEof
            && decoder.is_eof()
            && decoder.buffered().is_empty() => break,   // 干净结束
        Err(e) => return Err(e.into()),                  // 截断/损坏
    }
}
```

示例还会演示「文件被截断」时能正确报错，而不是静默结束。

### 9.4 版本迁移

```bash
cargo run --example demo_versioning
```

覆盖：

- 旧数据 → 新类型：追加字段用 `default` 补齐；
- 新数据 → 旧类型：严格模式报 `FieldCount`；
- `compat` 模式：旧类型跳过新版本追加的尾部；
- schema 指纹对比；
- 枚举变体用 `tag` 钉死。

### 9.5 确定性输出

```rust
#[derive(Encode, Decode)]
struct Registry {
    #[ordpack(sort)]
    entries: std::collections::HashMap<String, u64>,
}
```

`sort` 会按键的**编码后字节**排序。代价是编码时多一次分配和排序（约比无序慢 10 倍），只在需要哈希/去重/对比时使用。

### 9.6 大块二进制数据

```rust
#[derive(Encode, Decode)]
struct Frame {
    id: u32,
    payload: Bytes<Vec<u8>>,
}
```

比 `Vec<u8>` 解码快约 10–19 倍（1 KiB 数据）。

---

## 10. 错误处理

```rust
pub struct Error { /* kind, offset */ }

impl Error {
    pub fn kind(&self) -> ErrorKind;
    pub fn offset(&self) -> u32;   // 出错的字节位置
}
```

| `ErrorKind` | 含义 |
|---|---|
| `UnexpectedEof` | 输入提前结束 |
| `InvalidBool(u8)` | bool tag 不是 0/1 |
| `InvalidChar(u32)` | 非法 Unicode 标量 |
| `InvalidUtf8` | 字符串不是合法 UTF-8 |
| `InvalidTag { got, max }` | 单字节 tag 越界（Option/Result/IpAddr 等） |
| `InvalidVariant { got, variants }` | 枚举变体号越界 |
| `FieldCount { got, expected }` | 字段数不匹配 |
| `MissingField(&'static str)` | 缺失且无默认值 |
| `LengthOverflow` | 长度放不进 `usize` |
| `NestingLimit` | 嵌套超过 128 层 |
| `VarintOverflow` | varint 超出目标整数 |
| `BufferFull` | `to_slice` 缓冲区不足 |
| `TrailingBytes` | 解码后仍有剩余字节 |
| `NonZero` | `NonZero*` 解出 0 |
| `Unsupported(&'static str)` | 无法表示（如非 UTF-8 路径） |
| `Custom(&'static str)` | 自定义 codec 错误 |
| `Io(std::io::ErrorKind)` | 读流失败（std） |

```rust
match ew_ordpack::from_slice::<T>(&bytes) {
    Ok(value) => { /* ... */ }
    Err(err) => eprintln!("decode failed: {:?} at byte {}", err.kind(), err.offset()),
}
```

---

## 11. 版本演进

### 11.1 允许与禁止

| 变更 | 结果 |
|---|---|
| 尾部追加字段 + `#[ordpack(default)]` | ✅ 旧数据可读 |
| 删除尾部字段 | ✅ 新代码可读；旧数据多出的字段需 `compat`，否则报错 |
| 重命名字段 | ✅ 名字不上线 |
| 修改字段类型 | ❌ |
| 交换字段顺序 | ❌ |
| 中间插入字段 | ❌ |
| 枚举尾部追加变体 | ✅ |
| 枚举中间插入变体 | ❌（除非全部显式 `tag`） |
| 变体内部追加字段 | ✅（配合 `compat` 可让旧读端跳过） |

### 11.2 推荐流程

1. 新字段永远加在最后，并标 `#[ordpack(default)]`；
2. 需要「旧读端读新数据」时，给结构体加 `#[ordpack(compat)]`；
3. 文件头存 `T::FINGERPRINT`，加载前比对；
4. 用 golden 字节测试锁定格式（见 `tests/golden.rs`）。

### 11.3 指纹的边界

`FINGERPRINT` 不含字段名，也不区分「两个同类型字段互换位置」。指纹能发现结构变化，但不能替代 golden 测试。

---

## 12. 性能

环境：x86_64 Windows，`--release` + thin LTO，三次运行中位数。

| 负载 | encode | decode |
|---|---|---|
| `Point { i32, i32 }` | **1.24 ns** | **4.14 ns** |
| `Point`（`packed`） | 1.05 ns | 4.09 ns |
| `User { u64, String, bool, f64 }` | 3.11 ns | 30.38 ns |
| `Vec<u32>` × 1000 | 2.87 GB/s | 0.99 GB/s |
| `Bytes<Vec<u8>>` × 1 KiB | 89 GB/s | **30.5 GB/s** |
| `String` × 1 KiB | 126 GB/s | 15.5 GB/s |
| `HashMap<String,u64>` × 100 | 2.59 GB/s | 0.16 GB/s |
| bincode 2（对照） | 1.29 ns | 5.33 ns |

结论：

- 编码约为 bincode 2 的 **0.92×**（略快），解码 **0.78×**（快约 22%）；
- 与无检查手写 codec 相比：编码更快，解码约 2.5×（差在 `Result` 与边界检查）；
- 基准门禁见 `ci/budgets.txt`，用同进程比值判定，不依赖绝对耗时。

### 性能技巧

```rust
// 1) 热路径用 to_slice，避免每次分配
let mut buf = vec![0u8; value.encoded_len().unwrap_or(256)];
let used = ew_ordpack::to_slice(&value, &mut buf)?;

// 2) 二进制数据用 Bytes<T>
let frame = Frame { payload: Bytes(bytes), .. };

// 3) 固定布局、版本锁定的内层类型用 packed
#[ordpack(packed)]
struct Header { magic: u32, version: u16 }
```

---

## 13. no_std 使用

```toml
ew-ordpack = { path = "../ew-ordpack", default-features = false, features = ["alloc", "derive"] }
```

- 纯 `core`（无 `Vec`/`String`）：`features = ["derive"]`；
- `std` 专属类型在 no_std 下不可用：`HashMap`、`HashSet`、`Path`、`PathBuf`、`OsString`、`SystemTime`、`Mutex`、`RwLock`、`CString`、`CStr`、`from_reader`、`StreamDecoder`。

---

## 14. 限制与 FAQ

### 已知限制

- `compat` 只支持结构体，枚举暂不支持；
- 枚举没有未知变体的兜底（未知 tag 直接报错）；
- `Path` / `OsStr` 只接受 UTF-8；
- `Box<[u8]>` 没有 `Decode`，请用 `Vec<u8>` 或 `Bytes<Box<[u8]>>`；
- `usize`/`isize` 按 u64/i64 编码，跨平台稳定但会占用更多空间；
- 流式解码只能得到拥有所有权的值；
- `HashMap`/`HashSet` 默认输出顺序随机，需要确定性时用 `sort` 或 `BTreeMap`。

### FAQ

**Q：为什么 `Point { i32, i32 }` 是 4 字节，而 bincode 是 2 字节？**  
A：多了 2 字节字段计数。这是版本演进能力的成本；需要极限紧凑时用 `#[ordpack(packed)]`。

**Q：能和 bincode/protobuf 互通吗？**  
A：不能。没有字段名/ID，格式只对同一份 Rust 类型定义有意义。

**Q：`from_slice` 报 `TrailingBytes` 怎么办？**  
A：输入里有多个值。用 `from_slice_prefix`，或 `StreamDecoder` 逐条解码。

**Q：为什么解码比手写慢 2.5 倍？**  
A：`Result` 传播 + 边界检查。解码输入不可信，这些不能省。

**Q：如何保证不同进程读到的字节一致？**  
A：用 `BTreeMap`，或给 `HashMap` 字段加 `#[ordpack(sort)]`。

**Q：字段改名会影响旧数据吗？**  
A：不会。名字不上线，只影响错误信息。

**Q：如何检测加载的 schema 与写入时不一致？**  
A：在文件头存 `T::FINGERPRINT` 并在加载时比对（见 `demo_file`）。

---

## 15. 命令速查

```bash
# 构建
cargo build
cargo build --release

# 测试（85 个）
cargo test
cargo test --release
cargo test --features unchecked

# no_std 构建
cargo build --no-default-features --features derive
cargo build --no-default-features --features alloc,derive

# 示例
cargo run --example demo_basic
cargo run --example demo_file
cargo run --example demo_streaming
cargo run --example demo_versioning

# 性能基准与预算
cargo run --release --example bench
cargo run --release --example bench -- --check ci/budgets.txt
cargo run --release --example bench -- --json target/bench.json
cargo run --release --example bench -- --save ci/baseline.json

# 代码质量
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
```

---

## 相关文档

- [设计文档](design.md)：线格式规范、演进规则、性能分析与偏差记录
- [README](../README.md)：项目概览
- [基准](../examples/bench.rs) 与 [预算](../ci/budgets.txt)
