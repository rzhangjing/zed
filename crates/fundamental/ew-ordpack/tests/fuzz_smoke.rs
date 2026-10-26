//! Deterministic "fuzz-like" property tests.
//!
//! These run on stable in CI; the cargo-fuzz harness lives in `fuzz/`.

use std::collections::HashMap;

use ew_ordpack::{Decode, Encode};

#[derive(Encode, Decode, Clone, PartialEq, Debug)]
struct Payload {
    id: u64,
    name: String,
    flag: bool,
    scores: Vec<u32>,
    tags: Vec<u8>,
    nested: (i32, char),
    lookup: HashMap<String, u64>,
}

#[derive(Encode, Decode, Clone, PartialEq, Debug)]
#[ordpack(compat)]
struct CompatPayload {
    id: u32,
    name: String,
    #[ordpack(default)]
    extra: Option<u64>,
}

#[derive(Encode, Decode, Clone, PartialEq, Debug)]
enum Message {
    Quit,
    Move { x: i32, y: i32 },
    Write(String),
    Batch(Vec<Message>),
}

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn byte(&mut self) -> u8 {
        self.next() as u8
    }

    fn range(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }

    fn string(&mut self) -> String {
        let len = self.range(24);
        (0..len)
            .map(|_| (b'a' + (self.byte() % 26)) as char)
            .collect()
    }

    fn message(&mut self, depth: usize) -> Message {
        match self.range(if depth == 0 { 3 } else { 4 }) {
            0 => Message::Quit,
            1 => Message::Move {
                x: self.next() as i32,
                y: self.next() as i32,
            },
            2 => Message::Write(self.string()),
            _ => {
                let len = self.range(4);
                Message::Batch((0..len).map(|_| self.message(depth - 1)).collect())
            }
        }
    }

    fn payload(&mut self) -> Payload {
        let mut lookup = HashMap::new();
        for _ in 0..self.range(6) {
            lookup.insert(self.string(), self.next());
        }
        Payload {
            id: self.next(),
            name: self.string(),
            flag: self.next() & 1 == 1,
            scores: (0..self.range(10)).map(|_| self.next() as u32).collect(),
            tags: (0..self.range(16)).map(|_| self.byte()).collect(),
            nested: (self.next() as i32, (b'a' + (self.byte() % 26)) as char),
            lookup,
        }
    }
}

#[test]
fn arbitrary_bytes_never_panic() {
    let mut rng = Rng::new(0x1234_5678_9abc_def0);
    for _ in 0..20_000 {
        let len = rng.range(96);
        let bytes: Vec<u8> = (0..len).map(|_| rng.byte()).collect();

        let _ = ew_ordpack::from_slice::<Payload>(&bytes);
        let _ = ew_ordpack::from_slice::<Message>(&bytes);
        let _ = ew_ordpack::from_slice::<CompatPayload>(&bytes);
        let _ = ew_ordpack::from_slice::<String>(&bytes);
        let _ = ew_ordpack::from_slice::<Vec<u8>>(&bytes);
        let _ = ew_ordpack::from_slice::<HashMap<String, Vec<u32>>>(&bytes);
        let _ = ew_ordpack::from_slice::<Option<Box<Payload>>>(&bytes);
        let _ = ew_ordpack::from_slice::<(u8, String, Vec<i64>)>(&bytes);
    }
}

#[test]
fn decoded_values_reencode() {
    let mut rng = Rng::new(0xdead_beef_cafe_babe);
    for _ in 0..5_000 {
        let len = rng.range(64);
        let bytes: Vec<u8> = (0..len).map(|_| rng.byte()).collect();

        if let Ok(value) = ew_ordpack::from_slice::<CompatPayload>(&bytes) {
            let encoded = ew_ordpack::to_vec(&value).unwrap();
            assert_eq!(
                ew_ordpack::from_slice::<CompatPayload>(&encoded).unwrap(),
                value
            );
        }
        if let Ok(value) = ew_ordpack::from_slice::<String>(&bytes) {
            let encoded = ew_ordpack::to_vec(&value).unwrap();
            assert_eq!(ew_ordpack::from_slice::<String>(&encoded).unwrap(), value);
        }
        if let Ok(value) = ew_ordpack::from_slice::<Message>(&bytes) {
            let encoded = ew_ordpack::to_vec(&value).unwrap();
            assert_eq!(ew_ordpack::from_slice::<Message>(&encoded).unwrap(), value);
        }
    }
}

#[test]
fn random_values_roundtrip() {
    let mut rng = Rng::new(0x0bad_f00d_1234_5678);
    for _ in 0..2_000 {
        let value = rng.payload();
        let bytes = ew_ordpack::to_vec(&value).unwrap();
        let decoded: Payload = ew_ordpack::from_slice(&bytes).unwrap();
        assert_eq!(decoded, value);

        let message = rng.message(3);
        let bytes = ew_ordpack::to_vec(&message).unwrap();
        let decoded: Message = ew_ordpack::from_slice(&bytes).unwrap();
        assert_eq!(decoded, message);
    }
}

#[test]
fn prefix_decoding_consumes_only_its_value() {
    let mut rng = Rng::new(42);
    for _ in 0..1_000 {
        let value = rng.payload();
        let bytes = ew_ordpack::to_vec(&value).unwrap();

        let mut extended = bytes.clone();
        extended.extend_from_slice(&[0xAA; 8]);

        let (decoded, used): (Payload, usize) = ew_ordpack::from_slice_prefix(&extended).unwrap();
        assert_eq!(decoded, value);
        assert_eq!(used, bytes.len());
    }
}
