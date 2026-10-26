use ew_ordpack::{Bytes, Decode, Encode};

#[test]
fn bytes_wire_matches_vec() {
    let plain = ew_ordpack::to_vec(&vec![1u8, 2, 3]).unwrap();
    let bulk = ew_ordpack::to_vec(&Bytes(vec![1u8, 2, 3])).unwrap();
    assert_eq!(plain, bulk);
    assert_eq!(bulk, vec![3, 1, 2, 3]);
}

#[test]
fn bytes_array_matches_array() {
    let plain = ew_ordpack::to_vec(&[1u8, 2, 3]).unwrap();
    let bulk = ew_ordpack::to_vec(&Bytes([1u8, 2, 3])).unwrap();
    assert_eq!(plain, bulk);
    assert_eq!(bulk, vec![1, 2, 3]);
}

#[test]
fn bytes_roundtrip_large() {
    let size = 1_000_000;
    let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
    let bytes = ew_ordpack::to_vec(&Bytes(data.clone())).unwrap();

    let decoded: Bytes<Vec<u8>> = ew_ordpack::from_slice(&bytes).unwrap();
    assert_eq!(decoded.0, data);

    // Interchangeable with the plain `Vec<u8>` wire format.
    let plain: Vec<u8> = ew_ordpack::from_slice(&bytes).unwrap();
    assert_eq!(plain, data);
}

#[test]
fn bytes_borrowed_is_zero_copy() {
    let bytes = ew_ordpack::to_vec(&Bytes(vec![9u8, 8, 7])).unwrap();
    let decoded: Bytes<&[u8]> = ew_ordpack::from_slice(&bytes).unwrap();
    assert_eq!(&decoded.0, &[9u8, 8, 7]);
    // Layout: varint length (1 byte) + payload.
    assert_eq!(decoded.0.as_ptr(), bytes[1..].as_ptr());
}

#[test]
fn bytes_in_struct() {
    #[derive(Encode, Decode, PartialEq, Debug)]
    struct Frame {
        id: u32,
        payload: Bytes<Vec<u8>>,
        checksum: Bytes<[u8; 4]>,
    }

    let value = Frame {
        id: 7,
        payload: Bytes(vec![1, 2, 3, 4, 5]),
        checksum: Bytes([0xDE, 0xAD, 0xBE, 0xEF]),
    };
    let bytes = ew_ordpack::to_vec(&value).unwrap();
    let decoded: Frame = ew_ordpack::from_slice(&bytes).unwrap();
    assert_eq!(decoded, value);
}

#[test]
fn bytes_is_interchangeable_with_boxed_slice() {
    let value: Bytes<Box<[u8]>> = Bytes(vec![5u8, 6, 7].into_boxed_slice());
    let bytes = ew_ordpack::to_vec(&value).unwrap();
    let decoded: Bytes<Box<[u8]>> = ew_ordpack::from_slice(&bytes).unwrap();
    assert_eq!(&decoded.0[..], &[5, 6, 7]);
}
