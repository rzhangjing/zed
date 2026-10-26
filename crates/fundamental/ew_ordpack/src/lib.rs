//! # ew_ordpack
//!
//! Schema-driven binary serialization with **zero field metadata on the wire**.
//!
//! The wire format of a struct is `u16le(field_count)` followed by the fields in
//! declaration order. Field names and field ids are never written; the decoder
//! recovers the layout entirely from the type definition.
//!
//! ```no_run
//! use ew_ordpack::{Decode, Encode};
//!
//! #[derive(Encode, Decode)]
//! struct User {
//!     id: u64,
//!     name: String,
//!     active: bool,
//! }
//!
//! let user = User { id: 1, name: "ab".into(), active: true };
//! let bytes = ew_ordpack::to_vec(&user).unwrap();
//! // 03 00 | 01 | 02 61 62 | 01
//! let decoded: User = ew_ordpack::from_slice(&bytes).unwrap();
//! ```
//!
//! ## Wire format (v1)
//!
//! | Rust type | Encoding |
//! |---|---|
//! | `u8` / `i8` | raw byte |
//! | `u16`..`u128` | LEB128 varint |
//! | `i16`..`i128` | zigzag varint |
//! | `f32` / `f64` | IEEE-754, little endian |
//! | `bool` | `0x00` / `0x01` |
//! | `char` | varint scalar value |
//! | `String` / `&str` | `varint(len)` + UTF-8 |
//! | `Vec<T>` / `&[T]` | `varint(len)` + elements |
//! | `[T; N]` | elements only |
//! | `Option<T>` | `0x00` / `0x01` + `T` |
//! | struct | `u16le(field_count)` + fields |
//! | enum | `varint(variant_tag)` + `[u16le(field_count) + fields]` |
//!
//! Iteration order of hash maps / sets is not stable, so byte-for-byte output
//! is only guaranteed for deterministic containers (`BTreeMap`, `BTreeSet`).

#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(not(feature = "unchecked"), forbid(unsafe_code))]
#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(feature = "alloc")]
extern crate alloc;

use core::fmt;
use core::marker::PhantomData;

#[cfg(feature = "derive")]
pub use ew_ordpack_derive::{Decode, Encode};

mod impls;

/// Wire format version. Participates in [`fingerprint`] so that a format change
/// invalidates every stored fingerprint.
pub const FORMAT_VERSION: u8 = 1;

/// Maximum nesting depth accepted by the decoder.
///
/// Every sequence, map, optional value and enum payload counts as one level.
pub const MAX_DEPTH: u16 = 128;

/// Maximum number of zero-sized elements accepted in a single sequence.
const MAX_ZST_SEQ: usize = 1 << 24;

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Error produced by encoding or decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    offset: u32,
}

/// The kind of a [`Error`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// Input ended before a value was fully decoded.
    UnexpectedEof,
    /// A `bool` was not encoded as `0` or `1`.
    InvalidBool(u8),
    /// A `char` was not a valid Unicode scalar value.
    InvalidChar(u32),
    /// A string was not valid UTF-8.
    InvalidUtf8,
    /// An enum tag was outside the expected range.
    InvalidVariant {
        /// Tag that was read.
        got: u64,
        /// Number of variants the decoder knows about.
        variants: u64,
    },
    /// A one-byte tag was outside the expected range.
    InvalidTag {
        /// Tag that was read.
        got: u8,
        /// Largest accepted tag.
        max: u8,
    },
    /// A struct or variant carried an unexpected field count.
    FieldCount {
        /// Field count that was read.
        got: u16,
        /// Field count the decoder expects.
        expected: u16,
    },
    /// A field required for decoding was absent.
    MissingField(&'static str),
    /// A length did not fit into `usize`.
    LengthOverflow,
    /// Nesting exceeded [`MAX_DEPTH`].
    NestingLimit,
    /// A varint overflowed its target integer.
    VarintOverflow,
    /// The output buffer was too small.
    BufferFull,
    /// Input bytes remained after decoding.
    TrailingBytes,
    /// A `NonZero*` value decoded to zero.
    NonZero,
    /// A value could not be represented in the wire format.
    Unsupported(&'static str),
    /// A user supplied codec failed.
    Custom(&'static str),
    /// The underlying reader failed.
    #[cfg(feature = "std")]
    Io(std::io::ErrorKind),
}

impl Error {
    /// Create an error without position information.
    #[inline]
    pub const fn new(kind: ErrorKind) -> Self {
        Self { kind, offset: 0 }
    }

    /// Create an error at a specific byte offset.
    #[inline]
    pub const fn at(kind: ErrorKind, offset: u32) -> Self {
        Self { kind, offset }
    }

    /// The kind of this error.
    #[inline]
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Byte offset in the input where the error was detected, if known.
    #[inline]
    pub const fn offset(&self) -> u32 {
        self.offset
    }

    #[cold]
    fn with_offset(self, offset: u32) -> Self {
        if self.offset == 0 {
            Self {
                kind: self.kind,
                offset,
            }
        } else {
            self
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            ErrorKind::UnexpectedEof => write!(f, "unexpected end of input"),
            ErrorKind::InvalidBool(v) => write!(f, "invalid bool tag {v}"),
            ErrorKind::InvalidChar(v) => write!(f, "invalid char scalar {v:#x}"),
            ErrorKind::InvalidUtf8 => write!(f, "invalid UTF-8"),
            ErrorKind::InvalidVariant { got, variants } => {
                write!(f, "invalid variant tag {got} (expected < {variants})")
            }
            ErrorKind::InvalidTag { got, max } => {
                write!(f, "invalid tag {got} (expected <= {max})")
            }
            ErrorKind::FieldCount { got, expected } => {
                write!(f, "field count {got} does not match expected {expected}")
            }
            ErrorKind::MissingField(name) => write!(f, "missing field `{name}`"),
            ErrorKind::LengthOverflow => write!(f, "length does not fit in usize"),
            ErrorKind::NestingLimit => write!(f, "nesting limit exceeded"),
            ErrorKind::VarintOverflow => write!(f, "varint overflow"),
            ErrorKind::BufferFull => write!(f, "output buffer too small"),
            ErrorKind::TrailingBytes => write!(f, "trailing bytes after value"),
            ErrorKind::NonZero => write!(f, "expected non-zero value"),
            ErrorKind::Unsupported(what) => write!(f, "unsupported: {what}"),
            ErrorKind::Custom(what) => write!(f, "custom codec error: {what}"),
            #[cfg(feature = "std")]
            ErrorKind::Io(kind) => write!(f, "I/O error: {kind:?}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}

// ---------------------------------------------------------------------------
// Writer / Reader
// ---------------------------------------------------------------------------

/// A sink of bytes.
pub trait Writer {
    /// Write all bytes.
    fn write(&mut self, bytes: &[u8]) -> Result<(), Error>;

    /// Write a single byte.
    #[inline]
    fn write_u8(&mut self, value: u8) -> Result<(), Error> {
        self.write(&[value])
    }

    /// Remaining capacity, when known.
    #[inline]
    fn remaining(&self) -> Option<usize> {
        None
    }
}

#[cfg(feature = "alloc")]
impl Writer for alloc::vec::Vec<u8> {
    #[inline]
    fn write(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.extend_from_slice(bytes);
        Ok(())
    }

    #[inline]
    fn write_u8(&mut self, value: u8) -> Result<(), Error> {
        self.push(value);
        Ok(())
    }

    #[inline]
    fn remaining(&self) -> Option<usize> {
        None
    }
}

impl<W: Writer + ?Sized> Writer for &mut W {
    #[inline]
    fn write(&mut self, bytes: &[u8]) -> Result<(), Error> {
        (**self).write(bytes)
    }

    #[inline]
    fn write_u8(&mut self, value: u8) -> Result<(), Error> {
        (**self).write_u8(value)
    }

    #[inline]
    fn remaining(&self) -> Option<usize> {
        (**self).remaining()
    }
}

/// A writer over a mutable slice with a cursor.
///
/// Returns [`ErrorKind::BufferFull`] once the slice is exhausted.
pub struct SliceWriter<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl<'a> SliceWriter<'a> {
    /// Create a writer over `buf`.
    #[inline]
    pub fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    /// Number of bytes written so far.
    #[inline]
    pub fn position(&self) -> usize {
        self.pos
    }

    /// The bytes written so far.
    #[inline]
    pub fn written(&self) -> &[u8] {
        &self.buf[..self.pos]
    }
}

impl<'a> Writer for SliceWriter<'a> {
    #[inline]
    fn write(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let end = self.pos + bytes.len();
        if end > self.buf.len() {
            return Err(Error::at(ErrorKind::BufferFull, self.pos as u32));
        }
        self.buf[self.pos..end].copy_from_slice(bytes);
        self.pos = end;
        Ok(())
    }

    #[inline]
    fn write_u8(&mut self, value: u8) -> Result<(), Error> {
        if self.pos >= self.buf.len() {
            return Err(Error::at(ErrorKind::BufferFull, self.pos as u32));
        }
        self.buf[self.pos] = value;
        self.pos += 1;
        Ok(())
    }

    #[inline]
    fn remaining(&self) -> Option<usize> {
        Some(self.buf.len() - self.pos)
    }
}

/// A writer used by `to_slice` when the `unchecked` feature is on.
///
/// The total length has been validated against [`Encode::encoded_len`] once,
/// so per-write bounds checks are skipped.
#[cfg(feature = "unchecked")]
struct UncheckedWriter<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

#[cfg(feature = "unchecked")]
impl<'a> UncheckedWriter<'a> {
    #[inline]
    fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    #[inline]
    fn position(&self) -> usize {
        self.pos
    }
}

#[cfg(feature = "unchecked")]
impl Writer for UncheckedWriter<'_> {
    #[inline]
    fn write(&mut self, bytes: &[u8]) -> Result<(), Error> {
        // SAFETY: `to_slice` verified that `buf` holds `encoded_len` bytes and
        // the type's encoder never writes past that length.
        unsafe {
            core::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                self.buf.as_mut_ptr().add(self.pos),
                bytes.len(),
            );
        }
        self.pos += bytes.len();
        Ok(())
    }

    #[inline]
    fn write_u8(&mut self, value: u8) -> Result<(), Error> {
        // SAFETY: see `write`.
        unsafe {
            *self.buf.get_unchecked_mut(self.pos) = value;
        }
        self.pos += 1;
        Ok(())
    }
}

/// A source of bytes that can lend out its contents.
pub trait Reader<'de> {
    /// Read exactly `len` bytes.
    fn read(&mut self, len: usize) -> Result<&'de [u8], Error>;

    /// Read a single byte.
    #[inline]
    fn read_u8(&mut self) -> Result<u8, Error> {
        Ok(self.read(1)?[0])
    }

    /// Number of bytes left.
    fn remaining(&self) -> usize;

    /// Skip `len` bytes.
    #[inline]
    fn skip(&mut self, len: usize) -> Result<(), Error> {
        self.read(len)?;
        Ok(())
    }

    /// Number of bytes consumed so far.
    fn position(&self) -> usize {
        0
    }
}

/// A reader over a borrowed slice.
pub struct SliceReader<'de> {
    buf: &'de [u8],
    pos: usize,
}

impl<'de> SliceReader<'de> {
    /// Create a reader over `buf`.
    #[inline]
    pub fn new(buf: &'de [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    /// Number of bytes consumed so far.
    #[inline]
    pub fn position(&self) -> usize {
        self.pos
    }
}

impl<'de> Reader<'de> for SliceReader<'de> {
    #[inline]
    fn read(&mut self, len: usize) -> Result<&'de [u8], Error> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| Error::at(ErrorKind::LengthOverflow, self.pos as u32))?;
        if end > self.buf.len() {
            return Err(Error::at(ErrorKind::UnexpectedEof, self.pos as u32));
        }
        let out = &self.buf[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    #[inline]
    fn read_u8(&mut self) -> Result<u8, Error> {
        match self.buf.get(self.pos) {
            Some(&byte) => {
                self.pos += 1;
                Ok(byte)
            }
            None => Err(Error::at(ErrorKind::UnexpectedEof, self.pos as u32)),
        }
    }

    #[inline]
    fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    #[inline]
    fn skip(&mut self, len: usize) -> Result<(), Error> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| Error::at(ErrorKind::LengthOverflow, self.pos as u32))?;
        if end > self.buf.len() {
            return Err(Error::at(ErrorKind::UnexpectedEof, self.pos as u32));
        }
        self.pos = end;
        Ok(())
    }

    #[inline]
    fn position(&self) -> usize {
        self.pos
    }
}

// ---------------------------------------------------------------------------
// Encoder
// ---------------------------------------------------------------------------

/// Encodes values into a [`Writer`].
pub struct Encoder<W> {
    writer: W,
    pos: usize,
}

impl<W: Writer> Encoder<W> {
    /// Create an encoder around `writer`.
    #[inline]
    pub fn new(writer: W) -> Self {
        Self { writer, pos: 0 }
    }

    /// Unwrap the underlying writer.
    #[inline]
    pub fn into_inner(self) -> W {
        self.writer
    }

    /// Number of bytes written so far.
    #[inline]
    pub fn position(&self) -> usize {
        self.pos
    }

    #[inline]
    fn put(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.writer.write(bytes)?;
        self.pos += bytes.len();
        Ok(())
    }

    /// Write one raw byte.
    #[inline]
    pub fn write_u8(&mut self, value: u8) -> Result<(), Error> {
        self.writer.write_u8(value)?;
        self.pos += 1;
        Ok(())
    }

    /// Write a fixed little-endian `u16` (used for field counts).
    #[inline]
    pub fn write_u16(&mut self, value: u16) -> Result<(), Error> {
        self.put(&value.to_le_bytes())
    }

    /// Write an unsigned LEB128 varint.
    #[inline]
    pub fn write_varint(&mut self, mut value: u64) -> Result<(), Error> {
        if value < 0x80 {
            return self.write_u8(value as u8);
        }
        while value >= 0x80 {
            self.write_u8((value as u8) | 0x80)?;
            value >>= 7;
        }
        self.write_u8(value as u8)
    }

    /// Write a 128-bit unsigned LEB128 varint.
    #[inline]
    pub fn write_varint_u128(&mut self, mut value: u128) -> Result<(), Error> {
        if value < 0x80 {
            return self.write_u8(value as u8);
        }
        while value >= 0x80 {
            self.write_u8((value as u8) | 0x80)?;
            value >>= 7;
        }
        self.write_u8(value as u8)
    }

    /// Write a zigzag encoded signed integer.
    #[inline]
    pub fn write_zigzag(&mut self, value: i64) -> Result<(), Error> {
        self.write_varint(((value as u64) << 1) ^ ((value >> 63) as u64))
    }

    /// Write a zigzag encoded signed 128-bit integer.
    #[inline]
    pub fn write_zigzag_i128(&mut self, value: i128) -> Result<(), Error> {
        self.write_varint_u128(((value as u128) << 1) ^ ((value >> 127) as u128))
    }

    /// Write an `f32` as little-endian IEEE-754 bits.
    #[inline]
    pub fn write_f32(&mut self, value: f32) -> Result<(), Error> {
        self.put(&value.to_bits().to_le_bytes())
    }

    /// Write an `f64` as little-endian IEEE-754 bits.
    #[inline]
    pub fn write_f64(&mut self, value: f64) -> Result<(), Error> {
        self.put(&value.to_bits().to_le_bytes())
    }

    /// Write a length-prefixed byte string.
    #[inline]
    pub fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.write_varint(bytes.len() as u64)?;
        self.put(bytes)
    }

    /// Write raw bytes without a length prefix.
    #[inline]
    pub fn write_raw(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.put(bytes)
    }

    /// Write a length-prefixed UTF-8 string.
    #[inline]
    pub fn write_str(&mut self, value: &str) -> Result<(), Error> {
        self.write_bytes(value.as_bytes())
    }

    /// Write a sequence length.
    #[inline]
    pub fn write_len(&mut self, len: usize) -> Result<(), Error> {
        self.write_varint(len as u64)
    }

    /// Write a struct / variant field count.
    #[inline]
    pub fn write_field_count(&mut self, count: u16) -> Result<(), Error> {
        self.write_u16(count)
    }

    /// Write an enum variant tag.
    #[inline]
    pub fn write_tag(&mut self, tag: u64) -> Result<(), Error> {
        self.write_varint(tag)
    }
}

// ---------------------------------------------------------------------------
// Decoder
// ---------------------------------------------------------------------------

/// Decodes values from a [`Reader`].
pub struct Decoder<'de, R> {
    reader: R,
    depth: u16,
    marker: PhantomData<&'de [u8]>,
}

impl<'de, R: Reader<'de>> Decoder<'de, R> {
    /// Create a decoder around `reader`.
    #[inline]
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            depth: 0,
            marker: PhantomData,
        }
    }

    /// Unwrap the underlying reader.
    #[inline]
    pub fn into_inner(self) -> R {
        self.reader
    }

    /// Number of bytes consumed so far.
    #[inline]
    pub fn consumed(&self) -> usize {
        self.reader.position()
    }

    /// Number of bytes left in the input.
    #[inline]
    pub fn remaining(&self) -> usize {
        self.reader.remaining()
    }

    /// Build an error positioned at the current offset.
    #[cold]
    pub fn err(&self, kind: ErrorKind) -> Error {
        Error::at(kind, self.reader.position() as u32)
    }

    /// Build a [`ErrorKind::MissingField`] error.
    #[cold]
    pub fn missing(&self, name: &'static str) -> Error {
        self.err(ErrorKind::MissingField(name))
    }

    #[inline]
    fn read_exact(&mut self, len: usize) -> Result<&'de [u8], Error> {
        self.reader
            .read(len)
            .map_err(|err| err.with_offset(self.reader.position() as u32))
    }

    /// Read one byte.
    #[inline]
    pub fn read_u8(&mut self) -> Result<u8, Error> {
        self.reader
            .read_u8()
            .map_err(|err| err.with_offset(self.reader.position() as u32))
    }

    /// Read a fixed little-endian `u16`.
    #[inline]
    pub fn read_u16(&mut self) -> Result<u16, Error> {
        let bytes = self.read_exact(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    /// Read an unsigned LEB128 varint.
    #[inline]
    pub fn read_varint(&mut self) -> Result<u64, Error> {
        let first = self.read_u8()?;
        if first < 0x80 {
            return Ok(first as u64);
        }
        let mut result = (first & 0x7f) as u64;
        let mut shift = 7u32;
        loop {
            let byte = self.read_u8()?;
            if shift == 63 && byte > 1 {
                return Err(self.err(ErrorKind::VarintOverflow));
            }
            result |= ((byte & 0x7f) as u64) << shift;
            if byte & 0x80 == 0 {
                return Ok(result);
            }
            shift += 7;
            if shift > 63 {
                return Err(self.err(ErrorKind::VarintOverflow));
            }
        }
    }

    /// Read a 128-bit unsigned LEB128 varint.
    #[inline]
    pub fn read_varint_u128(&mut self) -> Result<u128, Error> {
        let first = self.read_u8()?;
        if first < 0x80 {
            return Ok(first as u128);
        }
        let mut result = (first & 0x7f) as u128;
        let mut shift = 7u32;
        loop {
            let byte = self.read_u8()?;
            if shift == 126 && byte > 3 {
                return Err(self.err(ErrorKind::VarintOverflow));
            }
            result |= ((byte & 0x7f) as u128) << shift;
            if byte & 0x80 == 0 {
                return Ok(result);
            }
            shift += 7;
            if shift > 126 {
                return Err(self.err(ErrorKind::VarintOverflow));
            }
        }
    }

    /// Read a zigzag encoded signed integer.
    #[inline]
    pub fn read_zigzag(&mut self) -> Result<i64, Error> {
        let value = self.read_varint()?;
        Ok(((value >> 1) as i64) ^ -((value & 1) as i64))
    }

    /// Read a zigzag encoded signed 128-bit integer.
    #[inline]
    pub fn read_zigzag_i128(&mut self) -> Result<i128, Error> {
        let value = self.read_varint_u128()?;
        Ok(((value >> 1) as i128) ^ -((value & 1) as i128))
    }

    /// Read an `f32` from little-endian IEEE-754 bits.
    #[inline]
    pub fn read_f32(&mut self) -> Result<f32, Error> {
        let bytes = self.read_exact(4)?;
        Ok(f32::from_bits(u32::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3],
        ])))
    }

    /// Read an `f64` from little-endian IEEE-754 bits.
    #[inline]
    pub fn read_f64(&mut self) -> Result<f64, Error> {
        let bytes = self.read_exact(8)?;
        Ok(f64::from_bits(u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ])))
    }

    /// Read a sequence length, validating it against the remaining input.
    ///
    /// `min_elem_bytes` is the minimum number of bytes one element occupies on
    /// the wire (use `0` for zero-sized elements).
    #[inline]
    pub fn read_seq_len(&mut self, min_elem_bytes: usize) -> Result<usize, Error> {
        let len = self.read_len()?;
        let max = self
            .remaining()
            .checked_div(min_elem_bytes)
            .unwrap_or(MAX_ZST_SEQ);
        if len > max {
            return Err(self.err(ErrorKind::UnexpectedEof));
        }
        Ok(len)
    }

    /// Read a length prefix.
    #[inline]
    pub fn read_len(&mut self) -> Result<usize, Error> {
        let value = self.read_varint()?;
        usize::try_from(value).map_err(|_| self.err(ErrorKind::LengthOverflow))
    }

    /// Read a length-prefixed byte string.
    #[inline]
    pub fn read_bytes(&mut self) -> Result<&'de [u8], Error> {
        let len = self.read_len()?;
        if len > self.remaining() {
            return Err(self.err(ErrorKind::UnexpectedEof));
        }
        self.read_exact(len)
    }

    /// Read raw bytes without a length prefix.
    #[inline]
    pub fn read_raw(&mut self, len: usize) -> Result<&'de [u8], Error> {
        self.read_exact(len)
    }

    /// Read a length-prefixed UTF-8 string.
    #[inline]
    pub fn read_str(&mut self) -> Result<&'de str, Error> {
        let bytes = self.read_bytes()?;
        core::str::from_utf8(bytes).map_err(|_| self.err(ErrorKind::InvalidUtf8))
    }

    /// Read a struct / variant field count.
    #[inline]
    pub fn read_field_count(&mut self) -> Result<u16, Error> {
        self.read_u16()
    }

    /// Read an enum variant tag.
    #[inline]
    pub fn read_tag(&mut self) -> Result<u64, Error> {
        self.read_varint()
    }

    /// Current byte position in the input.
    #[inline]
    pub fn position(&self) -> usize {
        self.reader.position()
    }

    /// Skip `len` bytes of input.
    #[inline]
    pub fn skip(&mut self, len: usize) -> Result<(), Error> {
        self.reader
            .skip(len)
            .map_err(|err| err.with_offset(self.reader.position() as u32))
    }

    #[inline]
    pub(crate) fn enter(&mut self) -> Result<(), Error> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.err(ErrorKind::NestingLimit));
        }
        Ok(())
    }

    #[inline]
    pub(crate) fn leave(&mut self) {
        self.depth -= 1;
    }
}

// ---------------------------------------------------------------------------
// Traits
// ---------------------------------------------------------------------------

/// Types that can be encoded.
pub trait Encode {
    /// Structural fingerprint of this type: a hash of the type path and the
    /// wire-relevant shape of its fields, **excluding field names**.
    ///
    /// Used to detect incompatible schemas at load time.
    const FINGERPRINT: u64 = 0;

    /// Shallow type tag used to build [`Encode::FINGERPRINT`] without
    /// recursing into the type's own fields.
    ///
    /// This keeps fingerprints computable for recursive types such as
    /// `enum Tree { Leaf, Node(Vec<Tree>) }`: `Vec<Tree>::KIND` only needs
    /// `Tree::KIND`, which is a plain hash of the type path.
    const KIND: u64 = 0;

    /// Exact number of bytes [`Encode::encode`] will produce, if cheaply known.
    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        None
    }

    /// Encode `self`.
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error>;
}

/// Types that can be decoded from a byte slice borrowed for `'de`.
pub trait Decode<'de>: Sized {
    /// Decode a value.
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error>;
}

/// Deterministic encoding for unordered containers.
///
/// Used by `#[ordpack(sort)]`: entries are sorted by their encoded key bytes, so
/// the output does not depend on hash iteration order.
#[cfg(feature = "alloc")]
pub trait SortedEncode {
    /// Write the container with entries sorted by encoded key bytes.
    fn encode_sorted<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error>;
}

/// Byte-buffer wrapper that encodes and decodes with bulk copies.
///
/// The wire format is identical to the wrapped type, so it is interchangeable
/// with `Vec<u8>`, `&[u8]` and `[u8; N]`:
///
/// ```rust
/// use ew_ordpack::Bytes;
///
/// let plain = ew_ordpack::to_vec(&vec![1u8, 2, 3]).unwrap();
/// let bulk = ew_ordpack::to_vec(&Bytes(vec![1u8, 2, 3])).unwrap();
/// assert_eq!(plain, bulk);
/// ```
///
/// Use it for binary blobs (hashes, compressed payloads, file chunks) where the
/// generic per-element path would cost a bounds check per byte.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Bytes<T>(pub T);

impl<T> Bytes<T> {
    /// Wrap `value`.
    #[inline]
    pub const fn new(value: T) -> Self {
        Self(value)
    }

    /// Unwrap the inner value.
    #[inline]
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> core::ops::Deref for Bytes<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> core::ops::DerefMut for Bytes<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T> From<T> for Bytes<T> {
    #[inline]
    fn from(value: T) -> Self {
        Self(value)
    }
}

/// Combine two optional lengths, returning `None` if either is unknown.
#[inline]
pub const fn optional_add(left: Option<usize>, right: Option<usize>) -> Option<usize> {
    match (left, right) {
        (Some(left), Some(right)) => left.checked_add(right),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Encode `value` into a new `Vec`.
#[cfg(feature = "alloc")]
#[inline]
pub fn to_vec<T: Encode + ?Sized>(value: &T) -> Result<alloc::vec::Vec<u8>, Error> {
    let mut out = alloc::vec::Vec::with_capacity(value.encoded_len().unwrap_or(0));
    {
        let mut encoder = Encoder::new(&mut out);
        value.encode(&mut encoder)?;
    }
    Ok(out)
}

/// Encode `value` into `out`, returning the number of bytes written.
#[inline(always)]
pub fn to_slice<T: Encode + ?Sized>(value: &T, out: &mut [u8]) -> Result<usize, Error> {
    let mut writer = SliceWriter::new(out);
    {
        let mut encoder = Encoder::new(&mut writer);
        value.encode(&mut encoder)?;
    }
    Ok(writer.position())
}

/// Encode `value` into `out` without per-write bounds checks.
///
/// # Safety
///
/// `out` must be at least `Encode::encoded_len(value)` bytes long, and
/// `encoded_len` must be exact for the value's type. Prefer [`to_slice`]
/// unless profiling shows this matters.
#[cfg(feature = "unchecked")]
#[inline]
pub unsafe fn to_slice_unchecked<T: Encode + ?Sized>(
    value: &T,
    out: &mut [u8],
) -> Result<usize, Error> {
    debug_assert!(value.encoded_len().map_or(true, |len| out.len() >= len));
    let mut writer = UncheckedWriter::new(out);
    {
        let mut encoder = Encoder::new(&mut writer);
        value.encode(&mut encoder)?;
    }
    Ok(writer.position())
}

/// Decode a `T` from `bytes`, requiring the whole input to be consumed.
#[inline(always)]
pub fn from_slice<'de, T: Decode<'de>>(bytes: &'de [u8]) -> Result<T, Error> {
    let mut decoder = Decoder::new(SliceReader::new(bytes));
    let value = T::decode(&mut decoder)?;
    if decoder.remaining() != 0 {
        return Err(decoder.err(ErrorKind::TrailingBytes));
    }
    Ok(value)
}

/// Decode a `T` from the front of `bytes`, returning the value and the number
/// of bytes consumed.
pub fn from_slice_prefix<'de, T: Decode<'de>>(bytes: &'de [u8]) -> Result<(T, usize), Error> {
    let mut decoder = Decoder::new(SliceReader::new(bytes));
    let value = T::decode(&mut decoder)?;
    Ok((value, decoder.consumed()))
}

// ---------------------------------------------------------------------------
// Streaming
// ---------------------------------------------------------------------------

/// Types that can be decoded from a byte stream.
///
/// This is implemented for every `T: for<'de> Decode<'de>`, i.e. types that do
/// not borrow from the input. Types like `&'de str` deliberately do not
/// implement it, so [`from_reader`] cannot be instantiated with them.
#[cfg(feature = "std")]
pub trait DecodeOwned: Sized {
    /// Decode one value from `decoder`.
    fn decode_owned<R: std::io::Read>(decoder: &mut StreamDecoder<R>) -> Result<Self, Error>;
}

#[cfg(feature = "std")]
impl<T> DecodeOwned for T
where
    T: for<'de> Decode<'de>,
{
    #[inline]
    fn decode_owned<R: std::io::Read>(decoder: &mut StreamDecoder<R>) -> Result<Self, Error> {
        decoder.decode()
    }
}

/// Buffered decoder over a [`std::io::Read`] source.
///
/// Bytes are pulled in chunks and decoded with the same wire format as
/// [`from_slice`]. One `StreamDecoder` can decode several concatenated values;
/// bytes that were read but not consumed stay buffered for the next call.
///
/// ```no_run
/// use std::io::Cursor;
/// use ew_ordpack::{Decode, Encode, StreamDecoder};
///
/// #[derive(Encode, Decode)]
/// struct Row { id: u32, name: String }
///
/// let mut bytes = ew_ordpack::to_vec(&Row { id: 1, name: "a".into() }).unwrap();
/// bytes.extend(ew_ordpack::to_vec(&Row { id: 2, name: "b".into() }).unwrap());
///
/// let mut decoder = StreamDecoder::new(Cursor::new(bytes));
/// let first: Row = decoder.decode().unwrap();
/// let second: Row = decoder.decode().unwrap();
/// assert_eq!((first.id, second.id), (1, 2));
/// ```
#[cfg(feature = "std")]
pub struct StreamDecoder<R> {
    reader: R,
    buf: alloc::vec::Vec<u8>,
    start: usize,
    eof: bool,
}

#[cfg(feature = "std")]
impl<R> StreamDecoder<R> {
    /// Create a stream decoder around `reader`.
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            buf: alloc::vec::Vec::new(),
            start: 0,
            eof: false,
        }
    }

    /// Bytes that have been read but not decoded yet.
    pub fn buffered(&self) -> &[u8] {
        &self.buf[self.start..]
    }

    /// Whether the reader has signalled end-of-file.
    pub fn is_eof(&self) -> bool {
        self.eof
    }

    /// Consume the decoder, returning the reader and the bytes not yet decoded.
    pub fn into_parts(mut self) -> (R, alloc::vec::Vec<u8>) {
        let rest = self.buf.split_off(self.start);
        (self.reader, rest)
    }
}

#[cfg(feature = "std")]
impl<R: std::io::Read> StreamDecoder<R> {
    /// Decode one value, pulling more bytes from the reader as needed.
    ///
    /// Returns [`ErrorKind::UnexpectedEof`] if the stream ends in the middle of
    /// a value. Decoding a value consumes only its own bytes; anything after it
    /// stays buffered for the next call.
    pub fn decode<T>(&mut self) -> Result<T, Error>
    where
        T: for<'de> Decode<'de>,
    {
        loop {
            match from_slice_prefix::<T>(&self.buf[self.start..]) {
                Ok((value, used)) => {
                    self.consume(used);
                    return Ok(value);
                }
                Err(err) => {
                    if err.kind() == ErrorKind::UnexpectedEof && !self.eof {
                        self.fill()?;
                        continue;
                    }
                    return Err(err);
                }
            }
        }
    }

    fn consume(&mut self, used: usize) {
        self.start += used;
        if self.start == self.buf.len() {
            self.buf.clear();
            self.start = 0;
        } else if self.start >= 64 * 1024 && self.start * 2 >= self.buf.len() {
            self.buf.drain(..self.start);
            self.start = 0;
        }
    }

    fn fill(&mut self) -> Result<(), Error> {
        let old_len = self.buf.len();
        // Grow geometrically so that a value spanning several reads costs
        // O(log n) decode attempts instead of O(n).
        let extra = core::cmp::max(old_len, 8192);
        self.buf.resize(old_len + extra, 0);
        match std::io::Read::read(&mut self.reader, &mut self.buf[old_len..]) {
            Ok(0) => {
                self.buf.truncate(old_len);
                self.eof = true;
                Ok(())
            }
            Ok(n) => {
                self.buf.truncate(old_len + n);
                Ok(())
            }
            Err(err) => {
                self.buf.truncate(old_len);
                Err(io_error(err))
            }
        }
    }
}

/// Read `reader` to the end and decode a single value.
///
/// The whole input must be consumed by the value; for a stream of concatenated
/// values use [`StreamDecoder`] instead.
#[cfg(feature = "std")]
pub fn from_reader<R, T>(mut reader: R) -> Result<T, Error>
where
    R: std::io::Read,
    T: for<'de> Decode<'de>,
{
    let mut buf = alloc::vec::Vec::new();
    std::io::Read::read_to_end(&mut reader, &mut buf).map_err(io_error)?;
    from_slice(&buf)
}

#[cfg(feature = "std")]
#[cold]
fn io_error(err: std::io::Error) -> Error {
    Error::new(ErrorKind::Io(err.kind()))
}

// ---------------------------------------------------------------------------
// Fingerprints
// ---------------------------------------------------------------------------

/// FNV-1a offset basis.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Begin a fingerprint for a named type.
///
/// The wire format version is mixed in, so any format change invalidates every
/// stored fingerprint.
pub const fn fingerprint_init(name: &str) -> u64 {
    let mut hash = FNV_OFFSET ^ (FORMAT_VERSION as u64);
    let bytes = name.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
        i += 1;
    }
    hash
}

/// Extend a fingerprint with the fingerprints of the type's fields (or variant
/// tags and field fingerprints, for enums).
pub const fn fingerprint_combine(base: u64, fields: &[u64]) -> u64 {
    let mut hash = base;
    let mut i = 0;
    while i < fields.len() {
        hash ^= fields[i];
        hash = hash.wrapping_mul(FNV_PRIME);
        i += 1;
    }
    hash
}

/// Convenience: fingerprint a named type from its field fingerprints.
pub const fn fingerprint(name: &str, fields: &[u64]) -> u64 {
    fingerprint_combine(fingerprint_init(name), fields)
}

/// Number of bytes an unsigned 64-bit varint occupies.
#[inline]
pub const fn varint_len(value: u64) -> usize {
    if value < 0x80 {
        1
    } else if value < 0x4000 {
        2
    } else if value < 0x20_0000 {
        3
    } else if value < 0x1000_0000 {
        4
    } else if value < 0x0080_0000_0000 {
        5
    } else if value < 0x4_0000_0000 {
        6
    } else if value < 0x200_0000_0000 {
        7
    } else if value < 0x1_0000_0000_0000 {
        8
    } else if value < 0x80_0000_0000_0000 {
        9
    } else {
        10
    }
}

/// Number of bytes an unsigned 128-bit varint occupies.
#[inline]
pub const fn varint_len_u128(mut value: u128) -> usize {
    let mut len = 1;
    while value >= 0x80 {
        value >>= 7;
        len += 1;
    }
    len
}

/// Number of bytes a zigzag varint occupies.
#[inline]
pub const fn zigzag_len(value: i64) -> usize {
    varint_len(((value as u64) << 1) ^ ((value >> 63) as u64))
}

/// Number of bytes a 128-bit zigzag varint occupies.
#[inline]
pub const fn zigzag_len_i128(value: i128) -> usize {
    varint_len_u128(((value as u128) << 1) ^ ((value >> 127) as u128))
}
