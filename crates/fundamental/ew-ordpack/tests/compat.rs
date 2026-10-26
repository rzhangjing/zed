use ew_ordpack::{Decode, Encode};

// V1 and V2 share the same prefix; V2 appended a field with a default.
#[derive(Encode, Decode, PartialEq, Debug)]
#[ordpack(compat)]
struct V1 {
    id: u64,
    name: String,
}

#[derive(Encode, Decode, PartialEq, Debug)]
#[ordpack(compat)]
struct V2 {
    id: u64,
    name: String,
    #[ordpack(default)]
    score: u32,
}

#[test]
fn compat_wire_format() {
    let v1 = V1 {
        id: 1,
        name: "a".into(),
    };
    let bytes = ew_ordpack::to_vec(&v1).unwrap();
    // count = 2, payload length = 1 (id) + 1 + 1 (name) = 3
    assert_eq!(bytes, vec![0x02, 0x00, 0x03, 0x01, 0x01, 0x61]);
    assert_eq!(ew_ordpack::from_slice::<V1>(&bytes).unwrap(), v1);
}

#[test]
fn compat_old_to_new() {
    let v1 = V1 {
        id: 7,
        name: "old".into(),
    };
    let bytes = ew_ordpack::to_vec(&v1).unwrap();
    let v2: V2 = ew_ordpack::from_slice(&bytes).unwrap();
    assert_eq!(
        v2,
        V2 {
            id: 7,
            name: "old".into(),
            score: 0
        }
    );
}

#[test]
fn compat_new_to_old_skips_unknown_tail() {
    let v2 = V2 {
        id: 9,
        name: "new".into(),
        score: 42,
    };
    let bytes = ew_ordpack::to_vec(&v2).unwrap();
    let v1: V1 = ew_ordpack::from_slice(&bytes).unwrap();
    assert_eq!(
        v1,
        V1 {
            id: 9,
            name: "new".into()
        }
    );
}

#[derive(Encode, Decode, PartialEq, Debug)]
#[ordpack(compat)]
struct Outer {
    inner: V2,
    tail: u8,
}

#[test]
fn compat_nested() {
    let value = Outer {
        inner: V2 {
            id: 1,
            name: "x".into(),
            score: 3,
        },
        tail: 4,
    };
    let bytes = ew_ordpack::to_vec(&value).unwrap();
    assert_eq!(ew_ordpack::from_slice::<Outer>(&bytes).unwrap(), value);
}

#[test]
fn compat_length_is_checked() {
    // count says 2, payload length says 1 byte, but 2 fields follow; the
    // trailing check must reject it instead of silently misreading.
    let mut bytes = ew_ordpack::to_vec(&V1 {
        id: 1,
        name: "a".into(),
    })
    .unwrap();
    bytes[2] = 1;
    assert!(ew_ordpack::from_slice::<V1>(&bytes).is_err());
}

#[test]
fn compat_truncated_payload_is_rejected() {
    let bytes = ew_ordpack::to_vec(&V1 {
        id: 1,
        name: "hello".into(),
    })
    .unwrap();
    let truncated = &bytes[..bytes.len() - 2];
    assert!(ew_ordpack::from_slice::<V1>(truncated).is_err());
}
