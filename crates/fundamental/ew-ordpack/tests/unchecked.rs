#![cfg(feature = "unchecked")]

use ew_ordpack::{Decode, Encode};

#[derive(Encode, Decode, PartialEq, Debug)]
struct Payload {
    id: u64,
    name: String,
    data: Vec<u8>,
}

#[test]
fn unchecked_matches_checked_encoding() {
    let value = Payload {
        id: 42,
        name: "unchecked".into(),
        data: vec![1, 2, 3, 4, 5],
    };
    let len = value.encoded_len().unwrap();
    assert_eq!(ew_ordpack::to_vec(&value).unwrap().len(), len);

    let mut checked = vec![0u8; len];
    let mut unchecked = vec![0u8; len];
    let checked_len = ew_ordpack::to_slice(&value, &mut checked).unwrap();
    // SAFETY: the buffer is exactly `encoded_len` bytes.
    let unchecked_len = unsafe { ew_ordpack::to_slice_unchecked(&value, &mut unchecked) }.unwrap();

    assert_eq!(checked_len, unchecked_len);
    assert_eq!(checked, unchecked);
    let decoded: Payload = ew_ordpack::from_slice(&unchecked).unwrap();
    assert_eq!(decoded, value);
}
