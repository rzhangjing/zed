//! 版本演进：尾部追加字段、`compat` 跳过未知尾部、schema 指纹。
//!
//! 运行：`cargo run --example demo_versioning`

use ew_ordpack::{Decode, Encode};

#[derive(Encode, Decode, PartialEq, Debug)]
struct V1 {
    id: u64,
    name: String,
}

#[derive(Encode, Decode, PartialEq, Debug)]
struct V2 {
    id: u64,
    name: String,
    #[ordpack(default)]
    score: u32,
}

#[derive(Encode, Decode, PartialEq, Debug)]
#[ordpack(compat)]
struct C1 {
    id: u64,
    name: String,
}

#[derive(Encode, Decode, PartialEq, Debug)]
#[ordpack(compat)]
struct C2 {
    id: u64,
    name: String,
    #[ordpack(default)]
    score: u32,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1) 旧数据 -> 新类型：追加字段用 default 补齐
    let old = V1 {
        id: 1,
        name: "a".into(),
    };
    let bytes = ew_ordpack::to_vec(&old)?;
    let upgraded: V2 = ew_ordpack::from_slice(&bytes)?;
    println!("V1 -> V2 : {upgraded:?} (score 默认 0)");

    // 2) 新数据 -> 旧类型：严格模式必须失败
    let new = V2 {
        id: 2,
        name: "b".into(),
        score: 9,
    };
    let bytes = ew_ordpack::to_vec(&new)?;
    match ew_ordpack::from_slice::<V1>(&bytes) {
        Ok(value) => println!("unexpected success: {value:?}"),
        Err(err) => println!("V2 -> V1 : rejected with {:?}", err.kind()),
    }

    // 3) compat：旧类型可以跳过新版本追加的尾部字段
    let c2 = C2 {
        id: 3,
        name: "c".into(),
        score: 5,
    };
    let bytes = ew_ordpack::to_vec(&c2)?;
    let c1: C1 = ew_ordpack::from_slice(&bytes)?;
    println!("C2 -> C1 : {c1:?} (compat 跳过 score)");

    // 4) 指纹：只哈希结构，不含字段名
    println!("V1 fingerprint = {:#018x}", V1::FINGERPRINT);
    println!("V2 fingerprint = {:#018x}", V2::FINGERPRINT);
    println!("C1 fingerprint = {:#018x}", C1::FINGERPRINT);
    assert_ne!(V1::FINGERPRINT, V2::FINGERPRINT);

    // 5) 枚举变体号可以钉死，避免以后插入变体导致漂移
    #[derive(Encode, Decode, PartialEq, Debug)]
    enum Event {
        Tick,
        #[ordpack(tag = 7)]
        Key(u32),
    }

    println!("Event::Tick  = {:?}", ew_ordpack::to_vec(&Event::Tick)?);
    println!("Event::Key(1)= {:?}", ew_ordpack::to_vec(&Event::Key(1))?);
    assert_eq!(ew_ordpack::to_vec(&Event::Key(1))?[0], 7);

    Ok(())
}
