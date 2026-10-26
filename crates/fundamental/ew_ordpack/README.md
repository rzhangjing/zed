# ew_ordpack

**编译期 schema 驱动的二进制序列化：线格式里没有任何字段元数据，只有 2 字节的字段个数。**

[用户手册](docs/manual.md) · [设计文档](docs/design.md) · [基准](examples/bench.rs)

```rust
use ew_ordpack::{Decode, Encode};

#[derive(Encode, Decode, PartialEq, Debug)]
struct User {
    id: u64,
    name: String,
    active: bool,
}

let user = User { id: 1, name: "ab".into(), active: true };
let bytes = ew_ordpack::to_vec(&user).unwrap();

// 03 00 | 01 | 02 61 62 | 01
// 字段数   id    "ab"    active
let decoded: User = ew_ordpack::from_slice(&bytes).unwrap();
assert_eq!(decoded, user);
```

## 设计要点

- **不写字段名、不写字段 ID**：结构体头部只有 `u16le(字段个数)`，字段按声明顺序排列。
- **解码全靠类型定义**：没有运行期形状表、没有 visitor，全部单态化后被 LLVM 内联。
- **唯一的安全演进方式：尾部追加字段**。旧数据解码到新类型时，缺失字段用 `#[ordpack(default)]` 补齐。
- **零拷贝**：`&'de str` / `&'de [u8]` 直接借用输入切片。
- **默认零 `unsafe`**；`no_std` 可用（core-only 或 +alloc）。

## 线格式（v1）

| Rust 类型 | 编码 |
|---|---|
| `u8` / `i8` | 原始字节 |
| `u16`..`u128` | LEB128 varint |
| `i16`..`i128` | zigzag varint |
| `f32` / `f64` | IEEE-754 小端 |
| `bool` | `0x00` / `0x01` |
| `char` | varint 标量 |
| `String` / `&str` | `varint(len)` + UTF-8 |
| `Vec<T>` / `&[T]` | `varint(len)` + 元素 |
| `[T; N]` | 元素（无长度） |
| `Option` / `Result` | 1 字节 tag |
| 结构体 | `u16le(字段数)` + 字段（声明顺序） |
| 枚举 | `varint(变体号)` + `[u16le(字段数) + 字段]` |
| newtype / `#[ordpack(transparent)]` | 透明，无字段数 |

## 属性

```rust
#[derive(Encode, Decode)]
struct Config {
    id: u64,
    #[ordpack(default)]              // 旧数据缺这个字段时用 Default 补齐
    retries: u32,
    #[ordpack(skip)]                 // 不编码、不计入字段数
    cache: Option<String>,
    #[ordpack(with = "chrono_codec")] // 自定义字段编解码
    timestamp: MyTime,
}

#[derive(Encode, Decode)]
enum Event {
    Tick,
    #[ordpack(tag = 7)]              // 钉死变体号
    Key(u32),
    Resize { w: u16, h: u16 },      // 变体内部也带字段数
}

#[derive(Encode, Decode)]
#[ordpack(packed)]                   // 不写字段数：版本锁定，最省最快
struct Header { magic: u32, version: u16 }

#[derive(Encode, Decode)]
#[ordpack(compat)]                   // 额外写负载长度：新数据可被旧类型解码
struct Record {
    id: u64,
    #[ordpack(default)]
    score: u32,
}

#[derive(Encode, Decode)]
struct Registry {
    #[ordpack(sort)]                 // 按编码后的键排序，输出与哈希顺序无关
    entries: HashMap<String, u64>,
}
```

完整清单：`default`、`skip`、`with`、`tag`、`transparent`、`packed`、`compat`、`sort`、`rename`、`bound`、`crate`。

## 性能

`cargo run --release --example bench`（x86_64，release + thin LTO，多轮中位）：

| 负载 | 大小 | encode | decode |
|---|---|---|---|
| `Point { i32, i32 }` | 4 B | **1.2 ns** | **4.3 ns** |
| `Point`（`#[ordpack(packed)]`，无字段计数） | 2 B | 1.0 ns | 4.1 ns |
| `User { u64, String, bool, f64 }` | 22 B | 3.0 ns | 29.8 ns |
| `Vec<u32>` × 1000 | 1874 B | 2.9 GB/s | 1.0 GB/s |
| `Vec<u8>` × 1 KiB | 1026 B | 44 GB/s | 1.6 GB/s |
| `Bytes<Vec<u8>>` × 1 KiB | 1026 B | 124 GB/s | **29 GB/s** |
| `Bytes<Vec<u8>>` × 64 KiB | 65539 B | 69 GB/s | **3.8 GB/s** |
| `Point`（bincode 2 对照） | 2 B | 1.3 ns | 5.6 ns |
| `Point`（无检查手写） | 4 B | 1.4 ns | 1.6 ns |

- 编码 **0.92×** bincode 2（略快），解码 **0.72×**（快约 30%）。
- 与无检查手写相比：编码 **0.85×**（更快），解码 2.5×（差在 `Result` 传播与边界检查）。
- `packed` 版本与 bincode 同样是 2 字节：编码 0.81×、解码 0.84×。
- 二进制 blob 用 `Bytes<T>`：解码比 `Vec<u8>` 快 7–16 倍（小对象更明显）。
- 需要再压编码时可用 `unchecked` feature + `to_slice_unchecked`（约再快 25%，但需保证缓冲区足够大）。

## 性能技巧

```rust
// 1. 编码到已有缓冲区：零分配
let mut buf = [0u8; 32];
let used = ew_ordpack::to_slice(&value, &mut buf)?;

// 2. 二进制数据用批量包装
let frame = Frame { payload: Bytes(bytes), .. };

// 3. 极端热路径（需自行保证 out.len() >= encoded_len）
#[cfg(feature = "unchecked")]
let used = unsafe { ew_ordpack::to_slice_unchecked(&value, &mut out)? };
```

## 流式解码

```rust
use std::io::Cursor;
use ew_ordpack::StreamDecoder;

let mut bytes = ew_ordpack::to_vec(&a).unwrap();
bytes.extend(ew_ordpack::to_vec(&b).unwrap());

let mut decoder = StreamDecoder::new(Cursor::new(bytes));
let first: Row = decoder.decode()?;   // 只消费自己的字节
let second: Row = decoder.decode()?;  // 剩下的仍在缓冲区里
```

- `from_reader(reader)`：读到结尾解一个值，要求全部字节被消费。
- `StreamDecoder::decode()`：逐条解码，内部缓冲区按需几何增长。
- `DecodeOwned` 对所有权类型自动成立；`&str` / `&[u8]` 不能满足 `for<'de> Decode<'de>`，编译期即被拒绝。
- I/O 失败映射为 `ErrorKind::Io(io::ErrorKind)`。

## 当前限制

- `compat` 目前只支持结构体（枚举会编译报错）。
- 字段重排 / 中间插入 = 破坏性变更；请用 `FINGERPRINT` 或 golden test 守住。
- `HashMap` / `HashSet` 默认按哈希顺序输出；需要确定性时加 `#[ordpack(sort)]` 或改用 `BTreeMap`。
- `Path` / `OsStr` 仅支持 UTF-8。

## 模糊测试

```bash
cargo test                       # 含 tests/fuzz_smoke.rs（稳定版可跑的随机属性测试）

# 可选的 cargo-fuzz 深度模糊测试（不是构建/测试的必要依赖）
cargo install cargo-fuzz
cargo +nightly fuzz run decode
cargo +nightly fuzz run roundtrip
```

> `fuzz/` 是独立的可选工程，只有运行它才需要 nightly；库本身、`cargo test` 和 CI 全部在 stable 上运行。
> Windows 上 `libfuzzer-sys` 需要 MSVC 工具链；本仓库用
> `cargo +nightly-x86_64-pc-windows-msvc check --manifest-path fuzz/Cargo.toml` 验证过目标可编译。

`fuzz/fuzz_targets/decode.rs` 对任意字节输入解码多种类型；`roundtrip.rs` 对成功解码的值重新编码并比对。

## CI 性能基线

CI **不用绝对耗时**做门槛（跨机器不可比），而是用**同进程内的比值预算**：
每个比值的分子分母都在同一次运行、同一台机器上测得。

- [`ci/budgets.txt`](ci/budgets.txt)：比值的上下界，`--check` 据此判定，违反则退出码 1。
- [`ci/baseline.json`](ci/baseline.json)：一次完整测量的快照，供对比与归档（绝对数字会随机器变化）。
- 每次 CI 运行产出 `target/bench.json` 工件，保留完整指标。

```bash
cargo run --release --example bench -- --check ci/budgets.txt
cargo run --release --example bench -- --save ci/baseline.json   # 更新快照
```

当前预算（本机实测值 → 门槛）：

| 比值 | 实测 | 预算 |
|---|---|---|
| `point_enc_vs_hand` | 0.85× | ≤ 1.50 |
| `point_dec_vs_hand` | 2.5× | ≤ 4.00 |
| `point_enc_vs_bincode` | 0.92× | ≤ 1.50 |
| `point_dec_vs_bincode` | 0.72× | ≤ 1.20 |
| `packed_point_enc_vs_bincode` | 0.81× | ≤ 1.20 |
| `packed_point_dec_vs_bincode` | 0.84× | ≤ 1.20 |
| `bytes_1k_dec_speedup` | 16× | ≥ 5.0 |
| `bytes_64k_dec_speedup` | 7× | ≥ 1.5 |

调整预算时改 `ci/budgets.txt`，并在 PR 中说明原因。

## 示例

| 示例 | 内容 |
|---|---|
| `cargo run --example demo_basic` | 结构体、枚举、容器、`to_slice`、零拷贝、`Bytes`、错误信息 |
| `cargo run --example demo_file` | 单条记录存文件：magic + 格式版本 + schema 指纹 |
| `cargo run --example demo_streaming` | 多条记录流式追加与逐条解码、截断检测 |
| `cargo run --example demo_versioning` | 尾部追加字段、`compat` 跳过未知尾部、变体 tag |

完整说明见[用户手册](docs/manual.md)。

## 测试

```bash
cargo test                                                 # 85 个测试
cargo test --features unchecked                            # +1
cargo build --no-default-features --features derive        # core-only
cargo build --no-default-features --features alloc,derive  # no_std + alloc
```

## License

MIT OR Apache-2.0
