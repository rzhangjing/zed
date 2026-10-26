//! Micro-benchmarks for `ew_ordpack`.
//!
//! ```text
//! cargo run --release --example bench
//! cargo run --release --example bench -- --check ci/budgets.txt
//! cargo run --release --example bench -- --json target/bench.json
//! cargo run --release --example bench -- --save ci/baseline.json
//! ```
//!
//! Encoding is measured through `to_slice` (pre-sized buffer, no allocation) so
//! the numbers reflect the codec itself rather than the allocator. A hand
//! written codec for `Point` is included as the performance floor.
//!
//! CI uses **ratio budgets** (`ci/budgets.txt`) rather than absolute timings:
//! both sides of every ratio are measured in the same process on the same
//! machine, which makes the gate stable across CI runners.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::hint::black_box;
use std::io;
use std::process;
use std::time::Instant;

use ew_ordpack::{Decode, Encode};

#[derive(Encode, Decode, bincode::Encode, bincode::Decode, Clone, Copy, PartialEq, Debug)]
struct Point {
    x: i32,
    y: i32,
}

#[derive(Encode, Decode, Clone, Copy, PartialEq, Debug)]
#[ordpack(packed)]
struct PackedPoint {
    x: i32,
    y: i32,
}

#[derive(Encode, Decode, bincode::Encode, bincode::Decode, Clone)]
struct User {
    id: u64,
    name: String,
    active: bool,
    score: f64,
}

#[derive(Encode, Decode, Clone)]
enum Event {
    Tick,
    Key(u32),
    Resize { w: u16, h: u16 },
    Message(String),
}

#[derive(Encode, Decode, Clone)]
#[ordpack(compat)]
struct CompatUser {
    id: u64,
    name: String,
    active: bool,
    score: f64,
}

#[derive(Encode, Decode, Clone)]
struct PlainMap {
    entries: HashMap<String, u64>,
}

#[derive(Encode, Decode, Clone)]
struct SortedMap {
    #[ordpack(sort)]
    entries: HashMap<String, u64>,
}

// ---------------------------------------------------------------------------
// Hand written codec for Point (same wire format), used as the floor.
// ---------------------------------------------------------------------------

#[inline]
fn zigzag(value: i64) -> u64 {
    ((value as u64) << 1) ^ ((value >> 63) as u64)
}

#[inline]
fn write_varint(mut value: u64, out: &mut [u8], pos: &mut usize) {
    while value >= 0x80 {
        out[*pos] = (value as u8) | 0x80;
        *pos += 1;
        value >>= 7;
    }
    out[*pos] = value as u8;
    *pos += 1;
}

#[inline]
fn read_varint(input: &[u8], pos: &mut usize) -> u64 {
    let mut result = 0u64;
    let mut shift = 0;
    loop {
        let byte = input[*pos];
        *pos += 1;
        result |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return result;
        }
        shift += 7;
    }
}

#[inline]
fn unzigzag(value: u64) -> i64 {
    ((value >> 1) as i64) ^ -((value & 1) as i64)
}

#[inline]
fn manual_encode_point(point: &Point, out: &mut [u8]) -> usize {
    out[0] = 2;
    out[1] = 0;
    let mut pos = 2;
    write_varint(zigzag(point.x as i64), out, &mut pos);
    write_varint(zigzag(point.y as i64), out, &mut pos);
    pos
}

#[inline]
fn manual_decode_point(input: &[u8]) -> Point {
    let mut pos = 2;
    let x = unzigzag(read_varint(input, &mut pos)) as i32;
    let y = unzigzag(read_varint(input, &mut pos)) as i32;
    Point { x, y }
}

// ---------------------------------------------------------------------------
// Result collection
// ---------------------------------------------------------------------------

struct Row {
    name: &'static str,
    size: usize,
    enc_ns: f64,
    dec_ns: f64,
    to_vec_ns: Option<f64>,
}

impl Row {
    fn enc_mb_s(&self) -> f64 {
        throughput(self.size, self.enc_ns)
    }

    fn dec_mb_s(&self) -> f64 {
        throughput(self.size, self.dec_ns)
    }
}

fn throughput(size: usize, ns: f64) -> f64 {
    if ns <= 0.0 {
        0.0
    } else {
        size as f64 / ns * 1000.0
    }
}

#[derive(Default)]
struct Bench {
    rows: Vec<Row>,
}

impl Bench {
    fn measure<T>(&mut self, name: &'static str, value: &T, iters: usize) -> (f64, f64)
    where
        T: Encode + for<'de> Decode<'de>,
    {
        let bytes = ew_ordpack::to_vec(value).unwrap();
        let size = bytes.len();
        let mut buf = vec![0u8; size];

        for _ in 0..1_000 {
            black_box(ew_ordpack::to_slice(black_box(value), &mut buf).unwrap());
            black_box(ew_ordpack::from_slice::<T>(black_box(&bytes)).unwrap());
        }

        let start = Instant::now();
        for _ in 0..iters {
            // Observe the buffer every iteration so repeated identical writes
            // cannot be folded into a single one by the optimizer.
            black_box(&mut buf);
            black_box(ew_ordpack::to_slice(black_box(value), &mut buf).unwrap());
        }
        let enc = start.elapsed().as_secs_f64();

        let start = Instant::now();
        for _ in 0..iters {
            black_box(ew_ordpack::from_slice::<T>(black_box(&bytes)).unwrap());
        }
        let dec = start.elapsed().as_secs_f64();

        let enc_ns = enc * 1e9 / iters as f64;
        let dec_ns = dec * 1e9 / iters as f64;
        self.push(Row {
            name,
            size,
            enc_ns,
            dec_ns,
            to_vec_ns: None,
        });
        (enc_ns, dec_ns)
    }

    fn measure_to_vec<T: Encode>(&mut self, name: &'static str, value: &T, iters: usize) -> f64 {
        let size = ew_ordpack::to_vec(value).unwrap().len();
        for _ in 0..1_000 {
            black_box(ew_ordpack::to_vec(black_box(value)).unwrap());
        }
        let start = Instant::now();
        for _ in 0..iters {
            black_box(ew_ordpack::to_vec(black_box(value)).unwrap());
        }
        let ns = start.elapsed().as_secs_f64() * 1e9 / iters as f64;
        self.push(Row {
            name,
            size,
            enc_ns: ns,
            dec_ns: 0.0,
            to_vec_ns: Some(ns),
        });
        ns
    }

    fn push(&mut self, row: Row) {
        match row.to_vec_ns {
            Some(ns) => println!("{:<24} {:>6} B | to_vec {:>7.1} ns", row.name, row.size, ns),
            None => println!(
                "{:<24} {:>6} B | enc {:>7.1} ns ({:>6.0} MB/s) | dec {:>7.1} ns ({:>6.0} MB/s)",
                row.name,
                row.size,
                row.enc_ns,
                row.enc_mb_s(),
                row.dec_ns,
                row.dec_mb_s()
            ),
        }
        self.rows.push(row);
    }

    fn get(&self, name: &str) -> &Row {
        self.rows
            .iter()
            .find(|row| row.name == name)
            .unwrap_or_else(|| panic!("missing benchmark row `{name}`"))
    }

    /// Machine-independent ratios used as CI budgets.
    fn ratios(&self) -> Vec<(&'static str, f64)> {
        let point = self.get("Point");
        let packed = self.get("Point (packed)");
        let hand = self.get("Point (hand written)");
        let bincode = self.get("Point (bincode 2)");
        let vec_1k = self.get("Vec<u8> 1KiB");
        let bytes_1k = self.get("Bytes<Vec<u8>> 1KiB");
        let vec_64k = self.get("Vec<u8> 64KiB");
        let bytes_64k = self.get("Bytes<Vec<u8>> 64KiB");

        vec![
            ("point_enc_vs_hand", point.enc_ns / hand.enc_ns),
            ("point_dec_vs_hand", point.dec_ns / hand.dec_ns),
            ("point_enc_vs_bincode", point.enc_ns / bincode.enc_ns),
            ("point_dec_vs_bincode", point.dec_ns / bincode.dec_ns),
            (
                "packed_point_enc_vs_bincode",
                packed.enc_ns / bincode.enc_ns,
            ),
            (
                "packed_point_dec_vs_bincode",
                packed.dec_ns / bincode.dec_ns,
            ),
            ("bytes_1k_dec_speedup", vec_1k.dec_ns / bytes_1k.dec_ns),
            ("bytes_64k_dec_speedup", vec_64k.dec_ns / bytes_64k.dec_ns),
        ]
    }
}

fn bench_bincode_point(point: &Point, iters: usize) -> (f64, f64) {
    let config = bincode::config::standard();
    let bytes = bincode::encode_to_vec(point, config).unwrap();
    let mut out = [0u8; 16];

    for _ in 0..1_000 {
        black_box(&mut out);
        black_box(bincode::encode_into_slice(*black_box(point), &mut out, config).unwrap());
        black_box(bincode::decode_from_slice::<Point, _>(black_box(&bytes), config).unwrap());
    }

    let start = Instant::now();
    for _ in 0..iters {
        black_box(&mut out);
        black_box(bincode::encode_into_slice(*black_box(point), &mut out, config).unwrap());
    }
    let enc = start.elapsed().as_secs_f64() * 1e9 / iters as f64;

    let start = Instant::now();
    for _ in 0..iters {
        black_box(bincode::decode_from_slice::<Point, _>(black_box(&bytes), config).unwrap());
    }
    let dec = start.elapsed().as_secs_f64() * 1e9 / iters as f64;

    (enc, dec)
}

// ---------------------------------------------------------------------------
// CI output
// ---------------------------------------------------------------------------

fn write_json(bench: &Bench, ratios: &[(&'static str, f64)], path: &str) -> io::Result<()> {
    let mut out = String::from(
        "{\n  \"version\": 1,\n  \"note\": \"Absolute timings are machine-specific; CI gates on ci/budgets.txt ratios.\",\n  \"rows\": [\n",
    );
    for (index, row) in bench.rows.iter().enumerate() {
        if index > 0 {
            out.push_str(",\n");
        }
        let to_vec = match row.to_vec_ns {
            Some(value) => format!("{value:.3}"),
            None => "null".to_string(),
        };
        out.push_str(&format!(
            "    {{\"name\": \"{}\", \"size\": {}, \"enc_ns\": {:.3}, \"dec_ns\": {:.3}, \"enc_mb_s\": {:.1}, \"dec_mb_s\": {:.1}, \"to_vec_ns\": {}}}",
            row.name,
            row.size,
            row.enc_ns,
            row.dec_ns,
            row.enc_mb_s(),
            row.dec_mb_s(),
            to_vec
        ));
    }
    out.push_str("\n  ],\n  \"ratios\": {");
    for (index, (name, value)) in ratios.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&format!("\n    \"{name}\": {value:.3}"));
    }
    out.push_str("\n  }\n}\n");
    fs::write(path, out)
}

fn parse_bound(text: &str) -> Result<f64, io::Error> {
    if text.eq_ignore_ascii_case("inf") {
        return Ok(f64::INFINITY);
    }
    text.parse::<f64>().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid bound `{text}`"),
        )
    })
}

fn load_budgets(path: &str) -> io::Result<Vec<(String, f64, f64)>> {
    let mut out = Vec::new();
    let text = fs::read_to_string(path)?;
    // Windows editors may prepend a UTF-8 BOM; it must not become part of the
    // first budget name.
    for (line_no, line) in text.trim_start_matches('\u{feff}').lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let (Some(name), Some(min), Some(max)) = (parts.next(), parts.next(), parts.next()) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{path}:{}: expected `name min max`", line_no + 1),
            ));
        };
        out.push((name.to_string(), parse_bound(min)?, parse_bound(max)?));
    }
    Ok(out)
}

fn check_budgets(bench: &Bench, path: &str) -> io::Result<bool> {
    let budgets = load_budgets(path)?;
    let ratios = bench.ratios();
    let mut failures = 0;
    println!("\nperformance budgets ({path})");
    for (name, min, max) in &budgets {
        match ratios.iter().find(|(ratio, _)| ratio == name) {
            Some((_, value)) => {
                let ok = *value >= *min && *value <= *max;
                if !ok {
                    failures += 1;
                }
                println!(
                    "{name:<26} {value:>8.2}x  budget [{min:.2}, {max:.2}]  {}",
                    if ok { "ok" } else { "FAIL" }
                );
            }
            None => {
                failures += 1;
                println!("{name:<26}      ???  missing ratio");
            }
        }
    }
    if failures == 0 {
        println!("all performance budgets met");
        Ok(true)
    } else {
        println!("{failures} performance budget(s) violated");
        Ok(false)
    }
}

// ---------------------------------------------------------------------------

fn main() {
    let mut json_path: Option<String> = None;
    let mut save_path: Option<String> = None;
    let mut check_path: Option<String> = None;

    let args: Vec<String> = env::args().skip(1).collect();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => {
                json_path = args.get(index + 1).cloned();
                index += 2;
            }
            "--save" => {
                save_path = args.get(index + 1).cloned();
                index += 2;
            }
            "--check" => {
                check_path = args.get(index + 1).cloned();
                index += 2;
            }
            "--help" | "-h" => {
                println!("usage: bench [--json <path>] [--save <path>] [--check <budgets>]");
                return;
            }
            other => {
                eprintln!("unknown argument `{other}` (try --help)");
                process::exit(2);
            }
        }
    }

    let point = Point { x: 1, y: -2 };
    let user = User {
        id: 42,
        name: "Aristotle".into(),
        active: true,
        score: 61.5,
    };
    let points: Vec<Point> = (0..1_000).map(|i| Point { x: i, y: -i }).collect();
    let numbers: Vec<u32> = (0..1_000).collect();
    let mut map = HashMap::new();
    for i in 0..100 {
        map.insert(format!("key-{i}"), i as u64);
    }
    let events: Vec<Event> = (0..100)
        .map(|i| match i % 4 {
            0 => Event::Tick,
            1 => Event::Key(i),
            2 => Event::Resize {
                w: i as u16,
                h: i as u16,
            },
            _ => Event::Message(format!("msg-{i}")),
        })
        .collect();

    let blob: Vec<u8> = (0..1024).map(|i| (i % 251) as u8).collect();
    let big_blob: Vec<u8> = (0..64 * 1024).map(|i| (i % 251) as u8).collect();
    let bulk_1k = ew_ordpack::Bytes(blob.clone());
    let bulk_64k = ew_ordpack::Bytes(big_blob.clone());
    let text = "x".repeat(1024);
    let compat_user = CompatUser {
        id: 42,
        name: "Aristotle".into(),
        active: true,
        score: 61.5,
    };
    let plain_map = PlainMap {
        entries: map.clone(),
    };
    let sorted_map = SortedMap {
        entries: map.clone(),
    };

    println!("ew_ordpack micro-benchmarks (encode = to_slice, no allocation)");
    println!("{}", "-".repeat(100));

    let mut bench = Bench::default();

    bench.measure("Point", &point, 5_000_000);
    bench.measure("Point (packed)", &PackedPoint { x: 1, y: -2 }, 5_000_000);
    bench.measure("User", &user, 2_000_000);
    bench.measure("Vec<Point> x1000", &points, 50_000);
    bench.measure("Vec<u32> x1000", &numbers, 200_000);
    bench.measure("HashMap<String,u64>", &map, 50_000);
    bench.measure("Vec<Event> x100", &events, 200_000);
    bench.measure("Vec<u8> 1KiB", &blob, 200_000);
    bench.measure("Vec<u8> 64KiB", &big_blob, 5_000);
    bench.measure("Bytes<Vec<u8>> 1KiB", &bulk_1k, 200_000);
    bench.measure("Bytes<Vec<u8>> 64KiB", &bulk_64k, 5_000);
    bench.measure("String 1KiB", &text, 200_000);
    bench.measure("u32 (one varint)", &300u32, 20_000_000);
    bench.measure("Vec<u32> x1", &vec![300u32], 10_000_000);
    bench.measure("User (compat)", &compat_user, 2_000_000);
    bench.measure("HashMap plain", &plain_map, 50_000);
    bench.measure("HashMap sorted", &sorted_map, 20_000);
    bench.measure_to_vec("User (to_vec)", &user, 1_000_000);
    bench.measure_to_vec("Vec<u32> x1000 (to_vec)", &numbers, 50_000);

    // Hand written floor.
    let iters = 5_000_000;
    let mut buf = [0u8; 8];
    let bytes = ew_ordpack::to_vec(&point).unwrap();

    for _ in 0..1_000 {
        black_box(&mut buf);
        black_box(manual_encode_point(black_box(&point), &mut buf));
        black_box(manual_decode_point(black_box(&bytes)));
    }

    let start = Instant::now();
    for _ in 0..iters {
        black_box(&mut buf);
        black_box(manual_encode_point(black_box(&point), &mut buf));
    }
    let manual_enc = start.elapsed().as_secs_f64() * 1e9 / iters as f64;

    let start = Instant::now();
    for _ in 0..iters {
        black_box(manual_decode_point(black_box(&bytes)));
    }
    let manual_dec = start.elapsed().as_secs_f64() * 1e9 / iters as f64;

    bench.push(Row {
        name: "Point (hand written)",
        size: 4,
        enc_ns: manual_enc,
        dec_ns: manual_dec,
        to_vec_ns: None,
    });

    let (bincode_enc, bincode_dec) = bench_bincode_point(&point, iters);
    bench.push(Row {
        name: "Point (bincode 2)",
        size: bincode::encode_to_vec(point, bincode::config::standard())
            .unwrap()
            .len(),
        enc_ns: bincode_enc,
        dec_ns: bincode_dec,
        to_vec_ns: None,
    });

    #[cfg(feature = "unchecked")]
    {
        let mut ubuf = [0u8; 8];
        for _ in 0..1_000 {
            // SAFETY: the buffer is larger than Point::encoded_len().
            unsafe {
                black_box(ew_ordpack::to_slice_unchecked(black_box(&point), &mut ubuf).unwrap())
            };
        }
        let start = Instant::now();
        for _ in 0..iters {
            // SAFETY: same as above.
            unsafe {
                black_box(ew_ordpack::to_slice_unchecked(black_box(&point), &mut ubuf).unwrap())
            };
        }
        let ns = start.elapsed().as_secs_f64() * 1e9 / iters as f64;
        println!(
            "{:<24} {:>6} B | enc {ns:>7.1} ns    | to_slice_unchecked",
            "Point", 4
        );
    }

    let ratios = bench.ratios();
    println!();
    for (name, value) in &ratios {
        println!("{name:<26} {value:>8.2}x");
    }

    if let Some(path) = save_path {
        if let Err(err) = write_json(&bench, &ratios, &path) {
            eprintln!("failed to write `{path}`: {err}");
            process::exit(2);
        }
        println!("\nbaseline written to {path}");
    }

    if let Some(path) = json_path {
        if let Err(err) = write_json(&bench, &ratios, &path) {
            eprintln!("failed to write `{path}`: {err}");
            process::exit(2);
        }
        println!("results written to {path}");
    }

    if let Some(path) = check_path {
        match check_budgets(&bench, &path) {
            Ok(true) => {}
            Ok(false) => process::exit(1),
            Err(err) => {
                eprintln!("failed to read budgets `{path}`: {err}");
                process::exit(2);
            }
        }
    }
}
