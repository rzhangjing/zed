//! 内存预算的**离线**判据：流式取数不随行数增长、分块读不随值放大。
//!
//! 这两条判据在文档里挂了很久：`docs/设计方案.md` §12.2 的 G4（T4.7「`dhat` 内存检查」）、
//! G5（T5.2「`fetch_stream` 10 万行 RSS 平坦」）与 G6（T6.3「分块读无放大」）都记着
//! 「未做——要挂全局分配器＝加依赖」。`dhat` 确实要加依赖，但**这件事本身**
//! （「峰值活字节」「窗口内分配总量」）用三十行 `GlobalAlloc` 包装就能量，见 `tests/meter`。
//!
//! 与真机用例（`tests/query_real.rs` 里三条 `alloc_budget` 用例）的分工：
//!
//! * 这里跑在假后端上，量的是**中间件自己的账**：`RowBuf` 跨批复用、`LobStream` 只按窗口
//!   追加、交付完的东西不被留着。窗口大小、行数、批大小都是夹具的参数，可以随便摆。
//! * 真机那一半量的是**驱动真的分块之后**我们的账（`SQLGetData`/`SQLPutData` 的窗口由
//!   驱动决定），并且能跑到 10 万行 / 4 MiB。两边都只能量到**我们自己的**分配：
//!   驱动 DLL 内部的 `malloc` 走它自己的 CRT，进程内分配器看不见。
//!
//! 两条纪律，也是这份用例自己的「仪器说明书」：
//!
//! 1. **夹具的流水账是噪声。** 假后端每批/每趟都 `note(format!(..))` 记一笔（`op_log`），
//!    这些字符串在窗口内累加，量级约 64 B × 批数。所以基线增长的容差给到 64 KiB，
//!    并且另设**阳性对照**：把批大小提到整张表、或让消费方自己把值攒起来，仪器必须
//!    看见那份内存——否则「峰值很小」只说明仪器瞎，不说明代码好。
//! 2. **只在窗口里放被测负载。** 夹具、计划、连接都建在 `Window::open()` 之前。

mod common;
mod meter;

use std::future::poll_fn;
use std::pin::Pin;

use common::{FakeBackend, Fixture, columns};
use dm_core::{DataType, DmValue};
use dm_query::{LobRef, RowStream};
use futures_core::Stream;

use crate::meter::Window;

/// 行表里每行的文本载荷（字节）：`行长 × 行数` 就是「全留在内存里」的下界。
const PAYLOAD: usize = 120;
/// 行表规模：`FEW` 是基准，`MANY` 是它的 8 倍——差值才是「会不会随行数长」。
const FEW: usize = 512;
const MANY: usize = 4096;
/// 基准测量的批大小：`MANY / BATCH` = 512 批。
const BATCH: usize = 8;
/// 分块读的窗口（夹具参数，与真驱动的 `LOB_CHUNK_BYTES` 无关；用 64 KiB 是为了
/// 让夹具自己的流水日志（每趟记一笔）不成为噪声）。
const WINDOW: usize = 64 * 1024;

/// 一次测量的读数。
struct Measured {
    rows: usize,
    sum: i64,
    peak: usize,
    total: usize,
    net: isize,
}

/// 抽干行流：只累加 `ID` 与行数。
///
/// 刻意**不**把文本列 `get::<String>` 出来——那是消费方的分配，会把窗口里的账算歪。
/// 被测的是中间件：驱动把行写进 `RowBuf`，`RowStream` 一批一批交出来。
async fn drain(stream: &mut RowStream<FakeBackend>) -> (usize, i64) {
    let mut rows = 0usize;
    let mut sum = 0i64;
    while let Some(row) = poll_fn(|cx| Pin::new(&mut *stream).poll_next(cx)).await {
        let row = row.expect("行流不该在这里报错");
        sum += i64::from(row.get::<i32>(0).expect("ID 列应当能取成 i32"));
        rows += 1;
    }
    (rows, sum)
}

/// 跑一次「`row_count` 行、每批 `batch_rows` 行」的流式取数，返回窗口内的读数。
async fn stream_rows(row_count: usize, batch_rows: usize) -> Measured {
    let fx = Fixture::open(2).await;
    fx.shared.configure(move |plan| {
        plan.columns = columns(&[("ID", DataType::Integer), ("NAME", DataType::LongVarChar)]);
        plan.rows = (0..row_count)
            .map(|i| {
                let id = i32::try_from(i).expect("行号放得进 i32");
                vec![DmValue::Int(id), DmValue::Text("x".repeat(PAYLOAD))]
            })
            .collect();
        plan.batch_rows = batch_rows;
    });

    let conn = fx.conn().await;
    // 窗口开在 `fetch_stream` **之前**：`RowStream` 打开时就会取回第一批（阳性对照那一跑
    // 的一整批正好在这里），只把随后的 `drain` 圈进窗口会让整表那一跑量到 0 字节。
    let window = Window::open();
    let mut stream = conn
        .fetch_stream("select ID, NAME from T", ())
        .await
        .expect("打开行流失败");
    let (rows, sum) = drain(&mut stream).await;
    drop(stream);
    let measured = Measured {
        rows,
        sum,
        peak: window.peak_growth(),
        total: window.total_growth(),
        net: window.net_growth(),
    };
    fx.shutdown().await;
    measured
}

/// 消费方怎么处置读到的块。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Consume {
    /// 正确用法：读一趟、看完就丢（复用同一个缓冲，`clear()` 保容量）。
    PerTrip,
    /// **阳性对照**：自己把整个值攒进一个 `Vec`。它必须让仪器看见整值那么大——
    /// 否则 `PerTrip` 报出来的小峰值只能说明仪器瞎。
    Whole,
}

/// 一次分块读的读数。
struct LobRead {
    payload: usize,
    chunks: usize,
    bytes: usize,
    checksum: u64,
    peak: usize,
    total: usize,
    net: isize,
}

/// 跑一次「`payload` 字节的 CLOB 分块读」。
async fn read_lob(payload: usize, consume: Consume) -> LobRead {
    let fx = Fixture::open(2).await;
    fx.shared.configure(move |plan| {
        plan.columns = columns(&[("BODY", DataType::LongVarChar)]);
        plan.rows = vec![vec![DmValue::Null]];
        plan.lob_rows = vec![Some(vec![0xAB; payload])];
        plan.lob_chunk_bytes = WINDOW;
    });

    let conn = fx.conn().await;
    let mut stream = conn
        .lob_stream("select BODY from T", (), LobRef::name("BODY"))
        .await
        .expect("打开分块读失败");
    assert!(
        stream.next_row().await.expect("定位第一行失败"),
        "计划里有一行，`next_row` 必须返回 true"
    );

    let mut buf: Vec<u8> = Vec::new();
    let mut bytes = 0usize;
    let mut checksum = 0u64;
    let mut chunks = 0usize;

    let window = Window::open();
    loop {
        if consume == Consume::PerTrip {
            buf.clear();
        }
        let state = stream.read_chunk(&mut buf).await.expect("分块读失败");
        bytes += buf.len();
        checksum = checksum.wrapping_add(buf.iter().map(|b| u64::from(*b)).sum::<u64>());
        chunks += 1;
        if state.is_done() {
            break;
        }
    }
    drop(stream);
    let measured = LobRead {
        payload,
        chunks,
        bytes,
        checksum,
        peak: window.peak_growth(),
        total: window.total_growth(),
        net: window.net_growth(),
    };
    fx.shutdown().await;
    measured
}

/// G5/T5.2：行流交付过的行**不被留着**，峰值由「一批」决定、不由「多少行」决定。
#[test]
fn a_row_stream_does_not_hold_the_rows_it_delivered() {
    let _gate = meter::serialize();
    let rt = meter::runtime();
    let few = rt.block_on(stream_rows(FEW, BATCH));
    let many = rt.block_on(stream_rows(MANY, BATCH));
    // 阳性对照：一批就是整张表 ⇒ 整张表必须同时活着。
    let whole = rt.block_on(stream_rows(MANY, MANY));

    assert_eq!(few.rows, FEW, "512 行必须全部送到");
    assert_eq!(many.rows, MANY, "4096 行必须全部送到");
    let expected = i64::try_from(MANY).expect("行数放得进 i64");
    assert_eq!(many.sum, expected * (expected - 1) / 2, "行内容必须按序");
    assert_eq!(whole.rows, MANY);

    // 4096 行 × 120 字节的载荷全留下就是 480 KiB 量级。
    assert!(
        many.peak < 256 * 1024,
        "峰值 {} 太大：行表一共 {} 字节，峰值不该到同一量级",
        meter::kib(many.peak),
        meter::kib(MANY * PAYLOAD)
    );
    // 行数翻 8 倍，峰值不该跟着长（容差里含夹具的流水账，见文件头第 1 条）。
    assert!(
        many.peak <= few.peak + 64 * 1024,
        "行数 ×8 把峰值从 {} 抬到了 {}",
        meter::kib(few.peak),
        meter::kib(many.peak)
    );
    // 阳性对照：批次放开之后峰值必须真的上来，否则上面的「小」不算证据。
    assert!(
        whole.peak > 256 * 1024,
        "阳性对照失败：一批就是整张表（{} 字节），峰值却只有 {}——仪器没有灵敏度",
        meter::kib(MANY * PAYLOAD),
        meter::kib(whole.peak)
    );
    // 交付完的行不该被留着：净增只是池里那条连接的复用缓冲。
    assert!(
        many.net < 128 * 1024,
        "取完 {} 行之后净增了 {}",
        MANY,
        meter::kib(many.net.unsigned_abs())
    );

    eprintln!(
        "行流（批 {BATCH}）：{FEW} 行 → 峰值 {}、总分配 {}、净增 {}；{MANY} 行 → 峰值 {}、\
         总分配 {}、净增 {}；阳性对照（批 {MANY}）→ 峰值 {}",
        meter::kib(few.peak),
        meter::kib(few.total),
        few.net,
        meter::kib(many.peak),
        meter::kib(many.total),
        many.net,
        meter::kib(whole.peak)
    );
}

/// G6/T6.3：分块读的峰值由**一趟窗口**决定，不由值的大小决定。
#[test]
fn a_chunked_read_does_not_follow_the_value() {
    let _gate = meter::serialize();
    let rt = meter::runtime();
    let small = rt.block_on(read_lob(512 * 1024, Consume::PerTrip));
    let large = rt.block_on(read_lob(4 * 1024 * 1024, Consume::PerTrip));
    // 阳性对照：消费方自己攒下整个值。
    let whole = rt.block_on(read_lob(4 * 1024 * 1024, Consume::Whole));

    for read in [&small, &large] {
        assert_eq!(read.bytes, read.payload, "拼回来的字节数必须与值一致");
        assert_eq!(
            read.checksum,
            0xABu64 * u64::try_from(read.payload).expect("值长度放得进 u64"),
            "每一块的内容都要对"
        );
        assert_eq!(
            read.chunks,
            read.payload.div_ceil(WINDOW) + 1,
            "趟数 = ceil(值/窗口) + 1（最后一趟是 Done）"
        );
    }

    // 值长大 8 倍，峰值必须还在「一趟窗口」这个量级：真攒下来就是 4 MiB。
    assert!(
        large.peak < 192 * 1024,
        "4 MiB 的值的峰值是 {}，远超一趟窗口",
        meter::kib(large.peak)
    );
    assert!(
        large.peak <= small.peak + 64 * 1024,
        "值 ×8 把峰值从 {} 抬到了 {}",
        meter::kib(small.peak),
        meter::kib(large.peak)
    );
    // 阳性对照：同一个仪器、同一条路径，消费方一攒就必须看见整值。
    assert!(
        whole.peak >= whole.payload,
        "阳性对照失败：消费方攒下了整个值（{}），峰值却只有 {}——仪器没有灵敏度",
        meter::kib(whole.payload),
        meter::kib(whole.peak)
    );
    assert!(
        large.net < 128 * 1024,
        "读完 4 MiB 之后净增了 {}",
        meter::kib(large.net.unsigned_abs())
    );
    // 「峰值平」说的是没有累积；再钉一条**每趟**的账：坏写法是「每趟把剩下的整个值复制
    // 一遍」，那样 65 趟就是几百 MiB，这条会当场炸。
    assert!(
        large.total <= large.chunks * 2 * WINDOW,
        "{} 趟共分配 {}，平均每趟 {}——超过了一趟窗口的量级",
        large.chunks,
        meter::kib(large.total),
        meter::kib(large.total / large.chunks.max(1))
    );

    eprintln!(
        "分块读（窗口 {}）：{} → 峰值 {}、总分配 {}、{} 趟；{} → 峰值 {}、总分配 {}、{} 趟、\
         净增 {}；阳性对照（自己攒）→ 峰值 {}",
        meter::kib(WINDOW),
        meter::kib(small.payload),
        meter::kib(small.peak),
        meter::kib(small.total),
        small.chunks,
        meter::kib(large.payload),
        meter::kib(large.peak),
        meter::kib(large.total),
        large.chunks,
        large.net,
        meter::kib(whole.peak)
    );
}
