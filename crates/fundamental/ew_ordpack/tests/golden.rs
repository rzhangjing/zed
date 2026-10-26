use ew_ordpack::{Decode, Encode};

// ---------------------------------------------------------------------------
// Golden bytes: these freeze the wire format. If they change, the format
// version must change too.
// ---------------------------------------------------------------------------

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
}

#[derive(Encode, Decode, PartialEq, Debug)]
enum Event {
    Tick,
    Key(u8),
    Resize { w: u16, h: u16 },
}

#[test]
fn golden_point() {
    let bytes = ew_ordpack::to_vec(&Point { x: 1, y: -2 }).unwrap();
    assert_eq!(bytes, vec![0x02, 0x00, 0x02, 0x03]);
}

#[test]
fn golden_user() {
    let user = User {
        id: 1,
        name: "ab".into(),
        active: true,
    };
    let bytes = ew_ordpack::to_vec(&user).unwrap();
    assert_eq!(bytes, vec![0x03, 0x00, 0x01, 0x02, 0x61, 0x62, 0x01]);
}

#[test]
fn golden_enum() {
    assert_eq!(ew_ordpack::to_vec(&Event::Tick).unwrap(), vec![0x00]);
    assert_eq!(
        ew_ordpack::to_vec(&Event::Key(65)).unwrap(),
        vec![0x01, 0x01, 0x00, 0x41]
    );
    assert_eq!(
        ew_ordpack::to_vec(&Event::Resize { w: 800, h: 600 }).unwrap(),
        vec![0x02, 0x02, 0x00, 0xA0, 0x06, 0xD8, 0x04]
    );
}

#[test]
fn golden_primitives() {
    assert_eq!(
        ew_ordpack::to_vec(&Some(300u32)).unwrap(),
        vec![0x01, 0xAC, 0x02]
    );
    assert_eq!(ew_ordpack::to_vec(&300u32).unwrap(), vec![0xAC, 0x02]);
    assert_eq!(ew_ordpack::to_vec(&-2i32).unwrap(), vec![0x03]);
    assert_eq!(ew_ordpack::to_vec(&"hello").unwrap(), b"\x05hello");
    assert_eq!(
        ew_ordpack::to_vec(&vec![1u16, 2, 3]).unwrap(),
        vec![0x03, 0x01, 0x02, 0x03]
    );
    assert_eq!(
        ew_ordpack::to_vec(&[1u8, 2, 3]).unwrap(),
        vec![0x01, 0x02, 0x03]
    );
    assert_eq!(ew_ordpack::to_vec(&()).unwrap(), Vec::<u8>::new());
    assert_eq!(ew_ordpack::to_vec(&true).unwrap(), vec![0x01]);
    assert_eq!(ew_ordpack::to_vec(&'A').unwrap(), vec![0x41]);
}
