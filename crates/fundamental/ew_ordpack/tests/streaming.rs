use std::io::{self, Cursor, Read};

use ew_ordpack::{Decode, DecodeOwned, Encode, ErrorKind, StreamDecoder};

#[derive(Encode, Decode, Clone, PartialEq, Debug)]
struct User {
    id: u64,
    name: String,
    active: bool,
}

#[derive(Encode, Decode, PartialEq, Debug)]
enum Message {
    Quit,
    Write(String),
    Batch(Vec<Message>),
}

fn sample(index: u64) -> User {
    User {
        id: index,
        name: format!("user-{index}"),
        active: index % 2 == 0,
    }
}

/// A reader that hands out at most `chunk` bytes per `read` call.
struct Chunked<'a> {
    data: &'a [u8],
    pos: usize,
    chunk: usize,
}

impl Read for Chunked<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.data.len() || buf.is_empty() {
            return Ok(0);
        }
        let n = self.chunk.min(buf.len()).min(self.data.len() - self.pos);
        buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

struct Failing;

impl Read for Failing {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "boom"))
    }
}

#[test]
fn from_reader_decodes_value() {
    let value = sample(7);
    let bytes = ew_ordpack::to_vec(&value).unwrap();
    let decoded: User = ew_ordpack::from_reader(Cursor::new(bytes)).unwrap();
    assert_eq!(decoded, value);
}

#[test]
fn from_reader_rejects_trailing_bytes() {
    let value = sample(1);
    let mut bytes = ew_ordpack::to_vec(&value).unwrap();
    bytes.extend(ew_ordpack::to_vec(&value).unwrap());
    let err = ew_ordpack::from_reader::<_, User>(Cursor::new(bytes)).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::TrailingBytes);
}

#[test]
fn stream_decoder_reads_multiple_records() {
    let values: Vec<User> = (0..3).map(sample).collect();
    let mut bytes = Vec::new();
    for value in &values {
        bytes.extend(ew_ordpack::to_vec(value).unwrap());
    }

    let mut decoder = StreamDecoder::new(Cursor::new(bytes));
    for expected in &values {
        let decoded: User = decoder.decode().unwrap();
        assert_eq!(&decoded, expected);
    }
    let err = decoder.decode::<User>().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::UnexpectedEof);
}

#[test]
fn stream_decoder_handles_tiny_chunks() {
    let value = sample(42);
    let bytes = ew_ordpack::to_vec(&value).unwrap();
    let reader = Chunked {
        data: &bytes,
        pos: 0,
        chunk: 3,
    };
    let mut decoder = StreamDecoder::new(reader);
    let decoded: User = decoder.decode().unwrap();
    assert_eq!(decoded, value);
    assert!(decoder.is_eof() || decoder.buffered().is_empty());
}

#[test]
fn stream_decoder_handles_large_value() {
    let size = 256 * 1024;
    let chunk = 4096;
    let value = User {
        id: 9,
        name: "x".repeat(size),
        active: true,
    };
    let bytes = ew_ordpack::to_vec(&value).unwrap();
    let reader = Chunked {
        data: &bytes,
        pos: 0,
        chunk,
    };
    let mut decoder = StreamDecoder::new(reader);
    let decoded: User = decoder.decode().unwrap();
    assert_eq!(decoded, value);
}

#[test]
fn stream_decoder_nested_enum() {
    let value = Message::Batch(vec![
        Message::Quit,
        Message::Write("hello".into()),
        Message::Batch(vec![Message::Write("inner".into())]),
    ]);
    let bytes = ew_ordpack::to_vec(&value).unwrap();
    let mut decoder = StreamDecoder::new(Cursor::new(bytes));
    let decoded: Message = decoder.decode().unwrap();
    assert_eq!(decoded, value);
}

#[test]
fn stream_decoder_surfaces_io_errors() {
    let mut decoder = StreamDecoder::new(Failing);
    let err = decoder.decode::<User>().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Io(io::ErrorKind::BrokenPipe));
}

#[test]
fn decode_owned_trait() {
    let value = sample(5);
    let bytes = ew_ordpack::to_vec(&value).unwrap();
    let mut decoder = StreamDecoder::new(Cursor::new(bytes));
    let decoded = User::decode_owned(&mut decoder).unwrap();
    assert_eq!(decoded, value);
}

#[test]
fn into_parts_returns_unconsumed_bytes() {
    let first = sample(1);
    let second = sample(2);
    let mut bytes = ew_ordpack::to_vec(&first).unwrap();
    let first_len = bytes.len();
    bytes.extend(ew_ordpack::to_vec(&second).unwrap());

    let mut decoder = StreamDecoder::new(Cursor::new(bytes));
    let decoded: User = decoder.decode().unwrap();
    assert_eq!(decoded, first);

    let (reader, rest) = decoder.into_parts();
    // The reader is read ahead (buffered), so its position is past the first
    // record; the logical remainder is returned separately.
    assert!(reader.position() >= first_len as u64);
    assert_eq!(ew_ordpack::from_slice::<User>(&rest).unwrap(), second);
}
