use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::time::{Duration, UNIX_EPOCH};

use ew_ordpack::{Decode, Encode};

fn roundtrip<T>(value: &T) -> T
where
    T: Encode + for<'de> Decode<'de>,
{
    let bytes = ew_ordpack::to_vec(value).unwrap();
    ew_ordpack::from_slice(&bytes).unwrap()
}

#[test]
fn primitives_roundtrip() {
    macro_rules! check {
        ($($value:expr => $ty:ty),* $(,)?) => {
            $(
                let value: $ty = $value;
                let decoded: $ty = roundtrip(&value);
                assert_eq!(decoded, value);
            )*
        };
    }

    check! {
        0u8 => u8, 255u8 => u8,
        0i8 => i8, -128i8 => i8, 127i8 => i8,
        0u16 => u16, u16::MAX => u16,
        0u32 => u32, u32::MAX => u32,
        0u64 => u64, u64::MAX => u64,
        0u128 => u128, u128::MAX => u128,
        0usize => usize, usize::MAX => usize,
        0i16 => i16, i16::MIN => i16, i16::MAX => i16,
        0i32 => i32, i32::MIN => i32, i32::MAX => i32,
        0i64 => i64, i64::MIN => i64, i64::MAX => i64,
        0i128 => i128, i128::MIN => i128, i128::MAX => i128,
        0isize => isize, isize::MIN => isize, isize::MAX => isize,
        0.0f32 => f32, -1.5f32 => f32, f32::MIN => f32, f32::MAX => f32,
        0.0f64 => f64, -1.5f64 => f64, f64::MIN => f64, f64::MAX => f64,
        true => bool, false => bool,
        'a' => char, '中' => char, '🦀' => char,
    }
}

#[test]
fn strings_roundtrip() {
    for value in ["", "a", "hello world", "中文 UTF-8", "🦀"] {
        let decoded: String = roundtrip(&value.to_string());
        assert_eq!(decoded, value);
    }
}

#[test]
fn borrowed_roundtrip() {
    #[derive(Encode, Decode, PartialEq, Debug)]
    struct Borrowed<'a> {
        name: &'a str,
        data: &'a [u8],
    }

    let value = Borrowed {
        name: "borrowed",
        data: &[1, 2, 3, 4],
    };
    let bytes = ew_ordpack::to_vec(&value).unwrap();
    let decoded: Borrowed<'_> = ew_ordpack::from_slice(&bytes).unwrap();
    assert_eq!(decoded, value);
    // The decoded string is a slice of the input buffer, not a new allocation.
    // Layout: count(2) + len(1) + "borrowed"
    assert_eq!(decoded.name.as_ptr(), bytes[3..].as_ptr());
}

#[test]
fn containers_roundtrip() {
    assert_eq!(roundtrip(&vec![1u32, 2, 3]), vec![1, 2, 3]);
    assert_eq!(roundtrip(&Vec::<String>::new()), Vec::<String>::new());
    assert_eq!(roundtrip(&vec![Some(1u8), None]), vec![Some(1), None]);
    assert_eq!(roundtrip(&[1u16, 2, 3]), [1, 2, 3]);
    assert_eq!(
        roundtrip(&(1u8, "two".to_string(), 3.5f64)),
        (1, "two".into(), 3.5)
    );

    let mut deque = VecDeque::new();
    deque.push_back(1u8);
    deque.push_back(2);
    assert_eq!(roundtrip(&deque), deque);

    let mut map = BTreeMap::new();
    map.insert("a".to_string(), 1u32);
    map.insert("b".to_string(), 2);
    assert_eq!(roundtrip(&map), map);

    let mut hash = HashMap::new();
    hash.insert("key".to_string(), 42u64);
    assert_eq!(roundtrip(&hash), hash);

    let set: BTreeSet<u32> = [1, 2, 3].into_iter().collect();
    assert_eq!(roundtrip(&set), set);

    let hset: HashSet<String> = ["x".to_string()].into_iter().collect();
    assert_eq!(roundtrip(&hset), hset);
}

#[test]
fn misc_types_roundtrip() {
    assert_eq!(roundtrip(&Duration::new(3, 500)), Duration::new(3, 500));
    assert_eq!(
        roundtrip(&(UNIX_EPOCH + Duration::new(10, 20))),
        UNIX_EPOCH + Duration::new(10, 20)
    );
    assert_eq!(
        roundtrip(&(UNIX_EPOCH - Duration::new(10, 500))),
        UNIX_EPOCH - Duration::new(10, 500)
    );
    assert_eq!(
        roundtrip(&IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4))),
        IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4))
    );
    assert_eq!(
        roundtrip(&IpAddr::V6(Ipv6Addr::LOCALHOST)),
        IpAddr::V6(Ipv6Addr::LOCALHOST)
    );
    assert_eq!(
        roundtrip(&"127.0.0.1:8080".parse::<SocketAddr>().unwrap()),
        "127.0.0.1:8080".parse::<SocketAddr>().unwrap()
    );
    assert_eq!(
        roundtrip(&NonZeroU32::new(7).unwrap()),
        NonZeroU32::new(7).unwrap()
    );
    assert_eq!(roundtrip(&PathBuf::from("/tmp/a")), PathBuf::from("/tmp/a"));
}

#[test]
fn struct_forms_roundtrip() {
    #[derive(Encode, Decode, PartialEq, Debug)]
    struct Named {
        a: u32,
        b: String,
    }

    #[derive(Encode, Decode, PartialEq, Debug)]
    struct Tuple(u8, i16);

    #[derive(Encode, Decode, PartialEq, Debug)]
    struct Newtype(u64);

    #[derive(Encode, Decode, PartialEq, Debug)]
    struct Unit;

    #[derive(Encode, Decode, PartialEq, Debug)]
    struct Nested {
        point: Named,
        tuple: Tuple,
    }

    assert_eq!(
        roundtrip(&Named {
            a: 1,
            b: "x".into()
        }),
        Named {
            a: 1,
            b: "x".into()
        }
    );
    assert_eq!(roundtrip(&Tuple(1, -2)), Tuple(1, -2));
    assert_eq!(roundtrip(&Newtype(7)), Newtype(7));
    assert_eq!(roundtrip(&Unit), Unit);
    // Newtype is transparent: 7 encodes as a bare varint.
    assert_eq!(ew_ordpack::to_vec(&Newtype(7)).unwrap(), vec![7]);
    assert_eq!(
        roundtrip(&Nested {
            point: Named {
                a: 1,
                b: "x".into()
            },
            tuple: Tuple(1, -2)
        }),
        Nested {
            point: Named {
                a: 1,
                b: "x".into()
            },
            tuple: Tuple(1, -2)
        }
    );
}

#[test]
fn enum_roundtrip() {
    #[derive(Encode, Decode, PartialEq, Debug)]
    enum Message {
        Quit,
        Move { x: i32, y: i32 },
        Write(String),
        Change(i32, i32, i32),
    }

    for value in [
        Message::Quit,
        Message::Move { x: 1, y: -2 },
        Message::Write("hello".into()),
        Message::Change(1, 2, 3),
    ] {
        assert_eq!(roundtrip(&value), value);
    }
}

#[test]
fn generics_roundtrip() {
    #[derive(Encode, Decode, PartialEq, Debug)]
    struct Wrapper<T> {
        value: T,
        count: u32,
    }

    assert_eq!(
        roundtrip(&Wrapper {
            value: vec!["a".to_string()],
            count: 1
        }),
        Wrapper {
            value: vec!["a".to_string()],
            count: 1
        }
    );
}

mod hex {
    use ew_ordpack::{Decoder, Encoder, Error, Reader, Writer};

    pub fn encode<W: Writer>(value: &u32, e: &mut Encoder<W>) -> Result<(), Error> {
        e.write_raw(&value.to_le_bytes())
    }

    pub fn decode<'de, R: Reader<'de>>(d: &mut Decoder<'de, R>) -> Result<u32, Error> {
        let bytes = d.read_raw(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
}

#[test]
fn custom_with_roundtrip() {
    #[derive(Encode, Decode, PartialEq, Debug)]
    struct Custom {
        #[ordpack(with = "hex")]
        value: u32,
        tail: u8,
    }

    let value = Custom {
        value: 0x1234_5678,
        tail: 9,
    };
    let bytes = ew_ordpack::to_vec(&value).unwrap();
    // 2 byte count + 4 raw LE bytes + 1 byte tail
    assert_eq!(bytes.len(), 7);
    assert_eq!(&bytes[2..6], &[0x78, 0x56, 0x34, 0x12]);
    assert_eq!(ew_ordpack::from_slice::<Custom>(&bytes).unwrap(), value);
}

#[test]
fn attributes_roundtrip() {
    #[derive(Encode, Decode, PartialEq, Debug)]
    struct Skipping {
        a: u32,
        #[ordpack(skip)]
        cache: u64,
        b: u32,
    }

    let value = Skipping {
        a: 1,
        cache: 999,
        b: 2,
    };
    let bytes = ew_ordpack::to_vec(&value).unwrap();
    // Field count is 2 (the skipped field is not counted).
    assert_eq!(bytes, vec![0x02, 0x00, 0x01, 0x02]);
    assert_eq!(
        ew_ordpack::from_slice::<Skipping>(&bytes).unwrap(),
        Skipping {
            a: 1,
            cache: 0,
            b: 2
        }
    );

    #[derive(Encode, Decode, PartialEq, Debug)]
    #[ordpack(transparent)]
    struct Transparent {
        inner: u32,
    }

    assert_eq!(
        ew_ordpack::to_vec(&Transparent { inner: 5 }).unwrap(),
        vec![5]
    );

    #[derive(Encode, Decode, PartialEq, Debug)]
    #[ordpack(packed)]
    struct Packed {
        a: u16,
        b: u16,
    }

    assert_eq!(
        ew_ordpack::to_vec(&Packed { a: 1, b: 2 }).unwrap(),
        vec![1, 2]
    );
    assert_eq!(
        ew_ordpack::from_slice::<Packed>(&[1, 2]).unwrap(),
        Packed { a: 1, b: 2 }
    );
}

#[test]
fn enum_tags_roundtrip() {
    #[derive(Encode, Decode, PartialEq, Debug)]
    enum Tagged {
        #[ordpack(tag = 7)]
        Seven,
        #[ordpack(tag = 9)]
        Nine,
    }

    assert_eq!(ew_ordpack::to_vec(&Tagged::Seven).unwrap(), vec![7]);
    assert_eq!(ew_ordpack::to_vec(&Tagged::Nine).unwrap(), vec![9]);
    assert_eq!(
        ew_ordpack::from_slice::<Tagged>(&[7]).unwrap(),
        Tagged::Seven
    );
    assert_eq!(
        ew_ordpack::from_slice::<Tagged>(&[9]).unwrap(),
        Tagged::Nine
    );
    assert!(ew_ordpack::from_slice::<Tagged>(&[8]).is_err());
}

#[test]
fn version_compatibility() {
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

    let v1 = V1 {
        id: 1,
        name: "a".into(),
    };
    let bytes = ew_ordpack::to_vec(&v1).unwrap();
    let upgraded: V2 = ew_ordpack::from_slice(&bytes).unwrap();
    assert_eq!(
        upgraded,
        V2 {
            id: 1,
            name: "a".into(),
            score: 0
        }
    );

    let v2 = V2 {
        id: 1,
        name: "a".into(),
        score: 5,
    };
    let bytes = ew_ordpack::to_vec(&v2).unwrap();
    assert!(ew_ordpack::from_slice::<V1>(&bytes).is_err());
}

#[test]
fn errors_are_reported() {
    use ew_ordpack::ErrorKind;

    let err = ew_ordpack::from_slice::<bool>(&[2]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidBool(2));

    let err = ew_ordpack::from_slice::<u32>(&[0x80]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::UnexpectedEof);

    let err = ew_ordpack::from_slice::<u8>(&[1, 2]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::TrailingBytes);

    let err = ew_ordpack::from_slice::<String>(&[0x01, 0xFF]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidUtf8);

    let err = ew_ordpack::from_slice::<Point>(&[0x05, 0x00]).unwrap_err();
    assert_eq!(
        err.kind(),
        ErrorKind::FieldCount {
            got: 5,
            expected: 2
        }
    );

    let err = ew_ordpack::from_slice::<Vec<u8>>(&[0x03, 0x01]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::UnexpectedEof);
}

#[derive(Encode, Decode, PartialEq, Debug)]
struct Point {
    x: i32,
    y: i32,
}

#[test]
fn to_slice_matches_to_vec() {
    let value = Point { x: 1, y: -2 };
    let mut buf = [0u8; 16];
    let used = ew_ordpack::to_slice(&value, &mut buf).unwrap();
    assert_eq!(&buf[..used], ew_ordpack::to_vec(&value).unwrap().as_slice());

    let mut too_small = [0u8; 2];
    let err = ew_ordpack::to_slice(&value, &mut too_small).unwrap_err();
    assert_eq!(err.kind(), ew_ordpack::ErrorKind::BufferFull);
}

#[test]
fn fingerprints_differ() {
    #[derive(Encode, Decode)]
    struct A {
        x: u32,
    }

    #[derive(Encode, Decode)]
    struct B {
        x: u64,
    }

    assert_ne!(A::FINGERPRINT, B::FINGERPRINT);
    assert_eq!(A::FINGERPRINT, A::FINGERPRINT);
}

#[test]
fn borrowed_str_is_zero_copy() {
    let bytes = ew_ordpack::to_vec(&"hello".to_string()).unwrap();
    let decoded: &str = ew_ordpack::from_slice(&bytes).unwrap();
    assert_eq!(decoded, "hello");
    assert_eq!(decoded.as_ptr(), bytes[1..].as_ptr());
}
