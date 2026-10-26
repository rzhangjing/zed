//! 流式读写：多条记录追加、逐条解码、干净结束与截断检测。
//!
//! 运行：`cargo run --example demo_streaming`

use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Write};

use ew_ordpack::{Decode, Encode, ErrorKind, StreamDecoder};

#[derive(Encode, Decode, PartialEq, Debug)]
struct Row {
    id: u32,
    name: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::temp_dir().join("ew_ordpack_demo_stream.bin");
    let path = path.to_str().expect("temporary path must be UTF-8");

    let rows = vec![
        Row {
            id: 1,
            name: "alpha".into(),
        },
        Row {
            id: 2,
            name: "beta".into(),
        },
        Row {
            id: 3,
            name: "gamma".into(),
        },
    ];

    // 1) 逐条追加到文件
    {
        let mut writer = BufWriter::new(File::create(path)?);
        for row in &rows {
            let bytes = ew_ordpack::to_vec(row)?;
            writer.write_all(&bytes)?;
        }
        writer.flush()?;
    }
    println!(
        "wrote {} records, {} bytes",
        rows.len(),
        fs::metadata(path)?.len()
    );

    // 2) 逐条解码；只有"EOF 且缓冲区为空"才算干净结束
    let mut decoder = StreamDecoder::new(BufReader::new(File::open(path)?));
    let mut read_back = Vec::new();
    loop {
        match decoder.decode::<Row>() {
            Ok(row) => read_back.push(row),
            Err(err)
                if err.kind() == ErrorKind::UnexpectedEof
                    && decoder.is_eof()
                    && decoder.buffered().is_empty() =>
            {
                break;
            }
            Err(err) => return Err(err.into()),
        }
    }
    assert_eq!(read_back, rows);
    println!("streamed back: {read_back:?}");

    // 3) 截断一条记录后，必须报错而不是静默结束
    let mut bytes = fs::read(path)?;
    bytes.pop();
    fs::write(path, &bytes)?;
    let mut decoder = StreamDecoder::new(BufReader::new(File::open(path)?));
    let mut seen = 0usize;
    let result: Result<(), ew_ordpack::Error> = loop {
        match decoder.decode::<Row>() {
            Ok(_) => seen += 1,
            Err(err) => break Err(err),
        }
    };
    println!(
        "truncated file: decoded {seen} records then {:?}",
        result.map_err(|err| err.kind()).err()
    );

    fs::remove_file(path).ok();
    Ok(())
}
