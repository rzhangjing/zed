//! Full coverage of every `Encode` / `Decode` implementation and the edge
//! cases around them.

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap, HashSet, LinkedList, VecDeque};
use std::ffi::{CStr, CString, OsStr, OsString};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::num::{
    NonZeroI128, NonZeroI16, NonZeroI32, NonZeroI64, NonZeroI8, NonZeroIsize, NonZeroU128,
    NonZeroU16, NonZeroU32, NonZeroU64, NonZeroU8, NonZeroUsize, Wrapping,
};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, UNIX_EPOCH};

use ew_ordpack::{Bytes, Decode, Encode, ErrorKind};

fn roundtrip<T>(value: &T) -> T
where
    T: Encode + for<'de> Decode<'de>,
{
    let bytes = ew_ordpack::to_vec(value).unwrap();
    ew_ordpack::from_slice(&bytes).unwrap()
}

// ---------------------------------------------------------------------------
// Primitives and edge values
// ---------------------------------------------------------------------------

#[test]
fn unit_and_zero_sized() {
    assert_eq!(roundtrip(&()), ());
    assert_eq!(
        roundtrip(&core::marker::PhantomData::<String>),
        core::marker::PhantomData
    );
    assert_eq!(roundtrip(&Vec::<()>::new()), Vec::<()>::new());
    assert_eq!(roundtrip(&vec![(), (), ()]), vec![(), (), ()]);
    assert_eq!(roundtrip(&[(); 4]), [(); 4]);
    assert_eq!(roundtrip(&[0u8; 0]), [0u8; 0]);
}

#[test]
fn integer_boundaries() {
    macro_rules! check {
        ($($ty:ty),* $(,)?) => {
            $(
                for value in [<$ty>::MIN, <$ty>::MAX, 0 as $ty, 1 as $ty] {
                    assert_eq!(roundtrip(&value), value);
                }
            )*
        };
    }
    check!(u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize);
}

#[test]
fn float_bit_patterns() {
    for value in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        -0.0,
        f64::MIN_POSITIVE,
        5e-324,
        f64::MAX,
        f64::MIN,
    ] {
        let bytes = ew_ordpack::to_vec(&value).unwrap();
        let decoded: f64 = ew_ordpack::from_slice(&bytes).unwrap();
        assert_eq!(decoded.to_bits(), value.to_bits());
    }
    for value in [
        f32::NAN,
        f32::NEG_INFINITY,
        -0.0,
        f32::MIN_POSITIVE,
        f32::MAX,
    ] {
        let bytes = ew_ordpack::to_vec(&value).unwrap();
        let decoded: f32 = ew_ordpack::from_slice(&bytes).unwrap();
        assert_eq!(decoded.to_bits(), value.to_bits());
    }
}

#[test]
fn char_boundaries() {
    for value in ['\0', '\u{10FFFF}', 'é', '🦀', '\u{7F}'] {
        assert_eq!(roundtrip(&value), value);
    }
    // A UTF-16 surrogate is not a valid Unicode scalar value.
    let err = ew_ordpack::from_slice::<char>(&[0x80, 0xB0, 0x03]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidChar(0xD800));
}

#[test]
fn bool_and_unit_tags() {
    assert_eq!(ew_ordpack::to_vec(&true).unwrap(), vec![1]);
    assert_eq!(ew_ordpack::to_vec(&false).unwrap(), vec![0]);
    assert_eq!(ew_ordpack::to_vec(&()).unwrap(), Vec::<u8>::new());
    assert_eq!(
        ew_ordpack::from_slice::<bool>(&[2]).unwrap_err().kind(),
        ErrorKind::InvalidBool(2)
    );
}

#[test]
fn references() {
    let value = 7u32;
    assert_eq!(ew_ordpack::to_vec(&&value).unwrap(), vec![7]);

    let mut owned = 7u32;
    let mut_ref: &mut u32 = &mut owned;
    assert_eq!(ew_ordpack::to_vec(&mut_ref).unwrap(), vec![7]);

    assert_eq!(ew_ordpack::to_vec::<str>("abc").unwrap(), b"\x03abc");
    assert_eq!(
        ew_ordpack::to_vec::<[u32]>(&[1, 2, 3][..]).unwrap(),
        vec![3, 1, 2, 3]
    );
}

#[test]
fn strings_and_bytes() {
    for value in ["", "a", "héllo", "日本語", "🦀🦀"] {
        assert_eq!(roundtrip(&value.to_string()), value);
    }
    let long = "x".repeat(100_000);
    assert_eq!(roundtrip(&long), long);

    assert_eq!(
        ew_ordpack::from_slice::<String>(&[0x01, 0xFF])
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidUtf8
    );
    // Declared length exceeds the remaining input.
    assert_eq!(
        ew_ordpack::from_slice::<String>(&[0x05, b'a'])
            .unwrap_err()
            .kind(),
        ErrorKind::UnexpectedEof
    );
}

// ---------------------------------------------------------------------------
// Containers
// ---------------------------------------------------------------------------

#[test]
fn sequences() {
    assert_eq!(roundtrip(&Vec::<u8>::new()), Vec::<u8>::new());
    assert_eq!(roundtrip(&vec![1u32, 2, 3]), vec![1, 2, 3]);
    assert_eq!(roundtrip(&[1u16, 2, 3]), [1, 2, 3]);
    assert_eq!(roundtrip(&Vec::<String>::new()), Vec::<String>::new());

    let mut deque = VecDeque::new();
    deque.push_back(1u8);
    deque.push_front(0);
    assert_eq!(roundtrip(&deque), deque);
    assert_eq!(roundtrip(&VecDeque::<u8>::new()), VecDeque::new());

    let mut list = LinkedList::new();
    list.push_back(1u32);
    list.push_back(2);
    assert_eq!(roundtrip(&list), list);
    assert_eq!(roundtrip(&LinkedList::<u8>::new()), LinkedList::new());

    let mut heap = BinaryHeap::new();
    heap.push(3u32);
    heap.push(1);
    heap.push(2);
    assert_eq!(roundtrip(&heap).into_sorted_vec(), vec![1, 2, 3]);
    assert_eq!(
        roundtrip(&BinaryHeap::<u8>::new()).into_sorted_vec(),
        Vec::<u8>::new()
    );
}

#[test]
fn tuples_all_arities() {
    macro_rules! check {
        ($($value:expr),+ $(,)?) => {{
            let tuple = ($($value,)+);
            assert_eq!(roundtrip(&tuple), tuple);
        }};
    }

    check!(1u8);
    check!(1u8, 2u16);
    check!(1u8, 2u16, 3u32);
    check!(1u8, 2u16, 3u32, 4u64);
    check!(1u8, 2u16, 3u32, 4u64, 5i8);
    check!(1u8, 2u16, 3u32, 4u64, 5i8, 6i16);
    check!(1u8, 2u16, 3u32, 4u64, 5i8, 6i16, 7i32);
    check!(1u8, 2u16, 3u32, 4u64, 5i8, 6i16, 7i32, 8i64);
    check!(1u8, 2u16, 3u32, 4u64, 5i8, 6i16, 7i32, 8i64, 9f32);
    check!(1u8, 2u16, 3u32, 4u64, 5i8, 6i16, 7i32, 8i64, 9f32, 10f64);
    check!(1u8, 2u16, 3u32, 4u64, 5i8, 6i16, 7i32, 8i64, 9f32, 10f64, 'x');
    check!(1u8, 2u16, 3u32, 4u64, 5i8, 6i16, 7i32, 8i64, 9f32, 10f64, 'x', true);
}

#[test]
fn maps_and_sets() {
    let mut btree = BTreeMap::new();
    btree.insert("a".to_string(), 1u32);
    btree.insert("b".to_string(), 2);
    assert_eq!(roundtrip(&btree), btree);
    assert_eq!(roundtrip(&BTreeMap::<u8, u8>::new()), BTreeMap::new());

    let mut hash = HashMap::new();
    hash.insert("key".to_string(), 42u64);
    assert_eq!(roundtrip(&hash), hash);
    assert_eq!(roundtrip(&HashMap::<u8, u8>::new()), HashMap::new());

    let bset: BTreeSet<u32> = [3, 1, 2].into_iter().collect();
    assert_eq!(roundtrip(&bset), bset);
    assert_eq!(roundtrip(&BTreeSet::<u8>::new()), BTreeSet::new());

    let hset: HashSet<String> = ["x".to_string()].into_iter().collect();
    assert_eq!(roundtrip(&hset), hset);
    assert_eq!(roundtrip(&HashSet::<u8>::new()), HashSet::new());
}

#[test]
fn option_and_result() {
    assert_eq!(roundtrip(&None::<u32>), None);
    assert_eq!(roundtrip(&Some(Some(5u32))), Some(Some(5)));
    assert_eq!(roundtrip(&Ok::<u32, String>(1)), Ok(1));
    assert_eq!(roundtrip(&Err::<u32, String>("e".into())), Err("e".into()));

    assert_eq!(
        ew_ordpack::from_slice::<Option<u8>>(&[2])
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidTag { got: 2, max: 1 }
    );
    assert_eq!(
        ew_ordpack::from_slice::<Result<u8, u8>>(&[2])
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidTag { got: 2, max: 1 }
    );
}

#[test]
fn nested_containers() {
    type Nested = HashMap<String, Vec<Option<Result<u32, String>>>>;
    let mut value = Nested::new();
    value.insert(
        "k".into(),
        vec![Some(Ok(1)), Some(Err("e".into())), None, Some(Ok(u32::MAX))],
    );
    assert_eq!(roundtrip(&value), value);
}

// ---------------------------------------------------------------------------
// Pointers, wrappers and smart constructors
// ---------------------------------------------------------------------------

#[test]
fn smart_pointers() {
    assert_eq!(*roundtrip(&Box::new(5u32)), 5);
    assert_eq!(*roundtrip(&Rc::new("x".to_string())), "x");
    assert_eq!(*roundtrip(&Arc::new([1u8, 2, 3])), [1, 2, 3]);

    // Encode-only unsized contents.
    let boxed: Box<str> = "boxed".into();
    let rc: Rc<str> = Rc::from("rc");
    let arc: Arc<str> = Arc::from("arc");
    assert_eq!(ew_ordpack::to_vec(&boxed).unwrap(), b"\x05boxed");
    assert_eq!(ew_ordpack::to_vec(&rc).unwrap(), b"\x02rc");
    assert_eq!(ew_ordpack::to_vec(&arc).unwrap(), b"\x03arc");
}

#[test]
fn cow() {
    for value in [Cow::Borrowed("cow"), Cow::Owned(String::from("owned"))] {
        let bytes = ew_ordpack::to_vec(&value).unwrap();
        let decoded: Cow<'_, str> = ew_ordpack::from_slice(&bytes).unwrap();
        assert_eq!(decoded, value);
    }
}

#[test]
fn wrappers() {
    assert_eq!(roundtrip(&Wrapping(5u32)), Wrapping(5));
    assert_eq!(roundtrip(&Reverse(5u32)), Reverse(5));
    assert_eq!(roundtrip(&Cell::new(5u32)).get(), 5);
    assert_eq!(*roundtrip(&RefCell::new(5u32)).borrow(), 5);
    assert_eq!(*roundtrip(&Mutex::new(5u32)).lock().unwrap(), 5);
    assert_eq!(*roundtrip(&RwLock::new(5u32)).read().unwrap(), 5);
}

#[test]
fn non_zero_integers() {
    macro_rules! check {
        ($($ty:ident => $inner:ty),* $(,)?) => {
            $(
                let value = <$ty>::new(1 as $inner).unwrap();
                assert_eq!(roundtrip(&value).get(), 1 as $inner);
                assert_eq!(
                    ew_ordpack::from_slice::<$ty>(&[0]).unwrap_err().kind(),
                    ErrorKind::NonZero
                );
            )*
        };
    }

    check!(
        NonZeroU8 => u8,
        NonZeroU16 => u16,
        NonZeroU32 => u32,
        NonZeroU64 => u64,
        NonZeroU128 => u128,
        NonZeroUsize => usize,
        NonZeroI8 => i8,
        NonZeroI16 => i16,
        NonZeroI32 => i32,
        NonZeroI64 => i64,
        NonZeroI128 => i128,
        NonZeroIsize => isize,
    );
}

// ---------------------------------------------------------------------------
// Time, networking, paths and FFI strings
// ---------------------------------------------------------------------------

#[test]
fn time_types() {
    for value in [
        Duration::ZERO,
        Duration::new(0, 999_999_999),
        Duration::new(3, 500),
        Duration::MAX,
    ] {
        assert_eq!(roundtrip(&value), value);
    }

    for value in [
        UNIX_EPOCH,
        UNIX_EPOCH + Duration::new(10, 20),
        UNIX_EPOCH - Duration::new(10, 500),
        UNIX_EPOCH - Duration::new(0, 1),
    ] {
        assert_eq!(roundtrip(&value), value);
    }
}

#[test]
fn networking() {
    for value in [
        IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)),
        IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        IpAddr::V6(Ipv6Addr::LOCALHOST),
        IpAddr::V6(Ipv6Addr::UNSPECIFIED),
    ] {
        assert_eq!(roundtrip(&value), value);
    }

    let v4 = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 8080);
    assert_eq!(roundtrip(&SocketAddr::V4(v4)), SocketAddr::V4(v4));

    let v6 = SocketAddrV6::new(Ipv6Addr::LOCALHOST, 443, 7, 9);
    assert_eq!(roundtrip(&SocketAddr::V6(v6)), SocketAddr::V6(v6));

    assert_eq!(
        ew_ordpack::from_slice::<IpAddr>(&[5]).unwrap_err().kind(),
        ErrorKind::InvalidTag { got: 5, max: 6 }
    );
    assert_eq!(
        ew_ordpack::from_slice::<SocketAddr>(&[5])
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidTag { got: 5, max: 6 }
    );
}

#[test]
fn paths_and_os_strings() {
    let path = PathBuf::from("some/dir/file.txt");
    assert_eq!(roundtrip(&path), path);
    assert_eq!(
        ew_ordpack::to_vec(&path).unwrap(),
        ew_ordpack::to_vec::<Path>(path.as_path()).unwrap()
    );

    let os = OsString::from("hello");
    assert_eq!(roundtrip(&os), os);
    assert_eq!(
        ew_ordpack::to_vec(&os).unwrap(),
        ew_ordpack::to_vec::<OsStr>(os.as_os_str()).unwrap()
    );
}

#[test]
fn c_strings() {
    let cstring = CString::new("hello").unwrap();
    assert_eq!(roundtrip(&cstring), cstring);
    assert_eq!(
        ew_ordpack::to_vec(&cstring).unwrap(),
        ew_ordpack::to_vec::<CStr>(cstring.as_c_str()).unwrap()
    );

    // Wire bytes with an interior NUL cannot become a CString.
    assert_eq!(
        ew_ordpack::from_slice::<CString>(&[0x02, b'a', 0x00])
            .unwrap_err()
            .kind(),
        ErrorKind::Custom("interior NUL in CString")
    );
}

#[cfg(windows)]
#[test]
fn non_utf8_os_string_is_rejected() {
    use std::os::windows::ffi::OsStringExt;

    // An unpaired surrogate is a legal `OsString` but not valid UTF-8.
    let bad = OsString::from_wide(&[0xD800]);
    assert!(bad.to_str().is_none());
    assert_eq!(
        ew_ordpack::to_vec(&bad).unwrap_err().kind(),
        ErrorKind::Unsupported("non-UTF-8 OsStr")
    );

    let bad_path = PathBuf::from(bad);
    assert_eq!(
        ew_ordpack::to_vec(&bad_path).unwrap_err().kind(),
        ErrorKind::Unsupported("non-UTF-8 path")
    );
}

// ---------------------------------------------------------------------------
// Derived types
// ---------------------------------------------------------------------------

#[derive(Encode, Decode, PartialEq, Debug)]
enum Tree {
    Leaf,
    Node(Vec<Tree>),
}

#[test]
fn recursive_enum() {
    let value = Tree::Node(vec![
        Tree::Leaf,
        Tree::Node(vec![Tree::Leaf, Tree::Node(vec![])]),
    ]);
    assert_eq!(roundtrip(&value), value);

    // The fingerprint of a recursive type is still computable.
    assert_ne!(Tree::FINGERPRINT, 0);
}

#[test]
fn nesting_limit_is_enforced() {
    fn deep(depth: usize) -> Tree {
        if depth == 0 {
            Tree::Leaf
        } else {
            Tree::Node(vec![deep(depth - 1)])
        }
    }

    let bytes = ew_ordpack::to_vec(&deep(140)).unwrap();
    assert_eq!(
        ew_ordpack::from_slice::<Tree>(&bytes).unwrap_err().kind(),
        ErrorKind::NestingLimit
    );

    // Within the limit it decodes fine.
    let bytes = ew_ordpack::to_vec(&deep(10)).unwrap();
    assert!(ew_ordpack::from_slice::<Tree>(&bytes).is_ok());
}

#[derive(Encode, Decode, PartialEq, Debug)]
struct FixedArray<const N: usize> {
    data: [u8; N],
    tag: u8,
}

#[test]
fn const_generics() {
    let value = FixedArray::<4> {
        data: [1, 2, 3, 4],
        tag: 9,
    };
    assert_eq!(roundtrip(&value), value);
    assert_eq!(roundtrip(&FixedArray::<0> { data: [], tag: 1 }).tag, 1);
}

#[derive(Encode, Decode, PartialEq, Debug)]
struct Generic<T> {
    value: T,
    count: u32,
}

#[test]
fn generic_struct() {
    let value = Generic {
        value: vec![Some("x".to_string())],
        count: 2,
    };
    assert_eq!(roundtrip(&value), value);
}

#[derive(Encode, Decode, PartialEq, Debug)]
#[ordpack(bound = "T: Copy")]
struct Bounded<T> {
    value: T,
}

#[derive(Encode, Decode, PartialEq, Debug)]
#[ordpack(crate = "ew_ordpack")]
struct WithCrate {
    value: u8,
}

#[derive(Encode, Decode, PartialEq, Debug)]
struct Renamed {
    #[ordpack(rename = "wire_name_is_not_written")]
    value: u32,
}

#[test]
fn attributes() {
    assert_eq!(roundtrip(&Bounded { value: 5u32 }), Bounded { value: 5 });
    assert_eq!(roundtrip(&WithCrate { value: 7 }), WithCrate { value: 7 });

    // Renaming is diagnostic only; bytes are unchanged.
    assert_eq!(
        ew_ordpack::to_vec(&Renamed { value: 7 }).unwrap(),
        vec![1, 0, 7]
    );
}

// ---------------------------------------------------------------------------
// Robustness
// ---------------------------------------------------------------------------

#[derive(Encode, Decode, PartialEq, Debug)]
struct Complex {
    a: u64,
    b: String,
    c: Vec<u16>,
    d: Option<char>,
    e: (i8, bool),
}

#[test]
fn truncation_sweep_never_panics() {
    let value = Complex {
        a: u64::MAX,
        b: "hello".into(),
        c: vec![1, 2, 3],
        d: Some('x'),
        e: (-1, true),
    };
    let bytes = ew_ordpack::to_vec(&value).unwrap();
    for prefix in 0..bytes.len() {
        let result = ew_ordpack::from_slice::<Complex>(&bytes[..prefix]);
        assert!(result.is_err(), "prefix of {prefix} bytes decoded");
    }
}

#[test]
fn oversized_lengths_are_rejected() {
    // Declared vector length far exceeds the remaining input.
    let err = ew_ordpack::from_slice::<Vec<u8>>(&[0xFF, 0xFF, 0xFF, 0xFF, 0x0F]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::UnexpectedEof);

    // Length that does not fit in usize.
    let err = ew_ordpack::from_slice::<String>(&[
        0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01,
    ])
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::UnexpectedEof);
}

#[test]
fn field_count_mismatch_is_rejected() {
    #[derive(Encode, Decode, PartialEq, Debug)]
    struct Pair {
        a: u8,
        b: u8,
    }

    assert_eq!(
        ew_ordpack::from_slice::<Pair>(&[0x09, 0x00])
            .unwrap_err()
            .kind(),
        ErrorKind::FieldCount {
            got: 9,
            expected: 2
        }
    );
}

#[test]
fn trailing_bytes_are_rejected() {
    assert_eq!(
        ew_ordpack::from_slice::<u8>(&[1, 2]).unwrap_err().kind(),
        ErrorKind::TrailingBytes
    );
    let (value, used) = ew_ordpack::from_slice_prefix::<u8>(&[1, 2]).unwrap();
    assert_eq!((value, used), (1, 1));
}

#[test]
fn bulk_and_element_wire_formats_match() {
    assert_eq!(
        ew_ordpack::to_vec(&vec![1u8, 2, 3]).unwrap(),
        ew_ordpack::to_vec(&Bytes(vec![1u8, 2, 3])).unwrap()
    );
    assert_eq!(
        ew_ordpack::to_vec(&[1u8, 2, 3]).unwrap(),
        ew_ordpack::to_vec(&Bytes([1u8, 2, 3])).unwrap()
    );

    let slice: &[u8] = &[1, 2, 3];
    assert_eq!(
        ew_ordpack::to_vec(&slice).unwrap(),
        ew_ordpack::to_vec(&Bytes(slice)).unwrap()
    );
}

#[test]
fn decode_into_borrowed_slices_is_zero_copy() {
    let bytes = ew_ordpack::to_vec(&"borrowed".to_string()).unwrap();
    let text: &str = ew_ordpack::from_slice(&bytes).unwrap();
    assert_eq!(text, "borrowed");
    assert_eq!(text.as_ptr(), bytes[1..].as_ptr());
}

// ---------------------------------------------------------------------------
// Encoding path consistency: `to_vec` vs `to_slice` vs `encoded_len`
// ---------------------------------------------------------------------------

mod raw_u32 {
    use ew_ordpack::{Decoder, Encoder, Error, Reader, Writer};

    pub fn encode<W: Writer>(value: &u32, encoder: &mut Encoder<W>) -> Result<(), Error> {
        encoder.write_raw(&value.to_le_bytes())
    }

    pub fn decode<'de, R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<u32, Error> {
        let bytes = decoder.read_raw(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
}

#[derive(Encode, Decode, PartialEq, Debug)]
struct WithCustom {
    #[ordpack(with = "raw_u32")]
    value: u32,
}

#[derive(Encode, Decode, PartialEq, Debug)]
#[ordpack(compat)]
struct CompatRecord {
    id: u64,
    name: String,
    #[ordpack(default)]
    score: u32,
}

#[test]
fn encoded_len_is_exact() {
    macro_rules! check {
        ($($value:expr),* $(,)?) => {$(
            {
                let value = $value;
                let bytes = ew_ordpack::to_vec(&value).unwrap();
                assert_eq!(
                    value.encoded_len(),
                    Some(bytes.len()),
                    "encoded_len mismatch for {}",
                    stringify!($value)
                );
            }
        )*};
    }

    check!(
        0u8,
        u64::MAX,
        i64::MIN,
        1.5f64,
        true,
        '🦀',
        String::new(),
        "hello".to_string(),
        vec![1u8, 2, 3],
        vec![Some(1u32), None],
        Bytes(vec![1u8, 2, 3]),
        [1u8, 2, 3],
        (1u8, 2u16, "x".to_string()),
        Some(300u32),
        Ok::<u8, String>(1),
        Err::<u8, String>("e".into()),
        Complex {
            a: 1,
            b: "x".into(),
            c: vec![1],
            d: Some('y'),
            e: (1, false),
        },
        CompatRecord {
            id: 1,
            name: "n".into(),
            score: 2,
        },
        {
            let mut map = BTreeMap::new();
            map.insert("k".to_string(), 1u64);
            map
        },
    );

    // A `with` field has no pre-computable length.
    assert!(WithCustom { value: 7 }.encoded_len().is_none());
}

#[test]
fn to_vec_and_to_slice_agree() {
    macro_rules! check {
        ($($value:expr),* $(,)?) => {$(
            {
                let value = $value;
                let bytes = ew_ordpack::to_vec(&value).unwrap();
                let mut buf = vec![0u8; bytes.len()];
                let used = ew_ordpack::to_slice(&value, &mut buf).unwrap();
                assert_eq!(used, bytes.len(), "used mismatch for {}", stringify!($value));
                assert_eq!(buf, bytes, "bytes mismatch for {}", stringify!($value));
            }
        )*};
    }

    check!(
        0u8,
        u64::MAX,
        "hello".to_string(),
        vec![1u8, 2, 3],
        Bytes(vec![1u8, 2, 3]),
        [1u8, 2, 3],
        (1u8, "x".to_string()),
        Some(300u32),
        Complex {
            a: 1,
            b: "x".into(),
            c: vec![1],
            d: Some('y'),
            e: (1, false),
        },
        CompatRecord {
            id: 1,
            name: "n".into(),
            score: 2,
        },
        WithCustom { value: 7 },
    );
}
