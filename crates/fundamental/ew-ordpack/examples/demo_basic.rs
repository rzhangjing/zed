//! 基础用法：结构体、枚举、容器、借用解码、批量字节。
//!
//! 运行：`cargo run --example demo_basic`

use std::collections::BTreeMap;

use ew_ordpack::{Bytes, Decode, Encode};

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Encode, Decode, PartialEq, Debug)]
struct Point {
    x: i32,
    y: i32,
}

#[derive(Encode, Decode, PartialEq, Debug)]
struct User {
    id: u64,
    name: String,
    active: bool,
    tags: Vec<String>,
}

#[derive(Encode, Decode, PartialEq, Debug)]
enum Message {
    Ping,
    Move { x: i32, y: i32 },
    Text(String),
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1) 结构体：线格式 = u16 字段数 + 按声明顺序的字段
    let point = Point { x: 1, y: -2 };
    let bytes = ew_ordpack::to_vec(&point)?;
    println!("Point   : {}", hex(&bytes));
    assert_eq!(ew_ordpack::from_slice::<Point>(&bytes)?, point);

    // 2) 嵌套与容器
    let user = User {
        id: 1,
        name: "ab".into(),
        active: true,
        tags: vec!["x".into()],
    };
    let bytes = ew_ordpack::to_vec(&user)?;
    println!(
        "User    : {} bytes, 前 6 字节 {}",
        bytes.len(),
        hex(&bytes[..6])
    );
    assert_eq!(ew_ordpack::from_slice::<User>(&bytes)?, user);

    // 3) 编码到已有缓冲区：零分配
    let mut buf = [0u8; 128];
    let used = ew_ordpack::to_slice(&user, &mut buf)?;
    assert_eq!(&buf[..used], bytes.as_slice());
    println!("to_slice: {used} bytes（无堆分配）");

    // 4) 零拷贝借用解码
    let wire = ew_ordpack::to_vec(&"hello".to_string())?;
    let borrowed: &str = ew_ordpack::from_slice(&wire)?;
    println!(
        "borrowed: {borrowed}，零拷贝 = {}",
        borrowed.as_ptr() == wire[1..].as_ptr()
    );

    // 5) 枚举：变体序号 + 变体内的字段
    for message in [
        Message::Ping,
        Message::Move { x: 1, y: 2 },
        Message::Text("hi".into()),
    ] {
        let bytes = ew_ordpack::to_vec(&message)?;
        println!("Enum    : {}", hex(&bytes));
        assert_eq!(ew_ordpack::from_slice::<Message>(&bytes)?, message);
    }

    // 6) 批量字节：与 Vec<u8> 线格式相同，但走批量 memcpy
    let blob = Bytes(vec![0u8, 1, 2, 3]);
    let wire = ew_ordpack::to_vec(&blob)?;
    println!("Bytes   : {}", hex(&wire));

    // 7) 有序 map（输出确定性）
    let mut map = BTreeMap::new();
    map.insert("a".to_string(), 1u32);
    map.insert("b".to_string(), 2);
    let wire = ew_ordpack::to_vec(&map)?;
    println!("BTreeMap: {}", hex(&wire));

    // 8) 错误带类型和偏移
    let err = ew_ordpack::from_slice::<u32>(&[0xFF]).unwrap_err();
    println!("error   : {:?} at byte {}", err.kind(), err.offset());

    println!("OK");
    Ok(())
}
