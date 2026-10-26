//! Implementations for standard library types.

use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use core::time::Duration;

use crate::{
    varint_len, varint_len_u128, zigzag_len, zigzag_len_i128, Decode, Decoder, Encode, Encoder,
    Error, ErrorKind, Reader, Writer,
};

#[cfg(feature = "alloc")]
use alloc::{
    borrow::{Cow, ToOwned},
    boxed::Box,
    collections::{BTreeMap, BTreeSet, BinaryHeap, LinkedList, VecDeque},
    rc::Rc,
    string::String,
    sync::Arc,
    vec::Vec,
};

#[cfg(feature = "std")]
use std::{
    collections::{HashMap, HashSet},
    ffi::{CStr, CString, OsStr, OsString},
    path::{Path, PathBuf},
    sync::{Mutex, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};

// ---------------------------------------------------------------------------
// Primitives
// ---------------------------------------------------------------------------

impl Encode for () {
    const KIND: u64 = crate::fingerprint_init("()");

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        Some(0)
    }

    #[inline]
    fn encode<W: Writer>(&self, _encoder: &mut Encoder<W>) -> Result<(), Error> {
        Ok(())
    }
}

impl<'de> Decode<'de> for () {
    #[inline]
    fn decode<R: Reader<'de>>(_decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        Ok(())
    }
}

impl Encode for bool {
    const KIND: u64 = crate::fingerprint_init("bool");

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        Some(1)
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        encoder.write_u8(*self as u8)
    }
}

impl<'de> Decode<'de> for bool {
    #[inline]
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        match decoder.read_u8()? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(decoder.err(ErrorKind::InvalidBool(other))),
        }
    }
}

impl Encode for u8 {
    const KIND: u64 = crate::fingerprint_init("u8");

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        Some(1)
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        encoder.write_u8(*self)
    }
}

impl<'de> Decode<'de> for u8 {
    #[inline]
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        decoder.read_u8()
    }
}

impl Encode for i8 {
    const KIND: u64 = crate::fingerprint_init("i8");

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        Some(1)
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        encoder.write_u8(*self as u8)
    }
}

impl<'de> Decode<'de> for i8 {
    #[inline]
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        Ok(decoder.read_u8()? as i8)
    }
}

macro_rules! impl_varint_unsigned {
    ($($ty:ty),* $(,)?) => {
        $(
            impl Encode for $ty {
                const KIND: u64 = crate::fingerprint_init(stringify!($ty));

                #[inline]
                fn encoded_len(&self) -> Option<usize> {
                    Some(varint_len(*self as u64))
                }

                #[inline]
                fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
                    encoder.write_varint(*self as u64)
                }
            }

            impl<'de> Decode<'de> for $ty {
                #[inline]
                fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
                    let value = decoder.read_varint()?;
                    <$ty>::try_from(value).map_err(|_| decoder.err(ErrorKind::VarintOverflow))
                }
            }
        )*
    };
}

impl_varint_unsigned!(u16, u32, u64);

impl Encode for u128 {
    const FINGERPRINT: u64 = crate::fingerprint("u128", &[]);
    const KIND: u64 = crate::fingerprint_init("u128");

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        Some(varint_len_u128(*self))
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        encoder.write_varint_u128(*self)
    }
}

impl<'de> Decode<'de> for u128 {
    #[inline]
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        decoder.read_varint_u128()
    }
}

impl Encode for usize {
    const KIND: u64 = crate::fingerprint_init("usize");

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        Some(varint_len(*self as u64))
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        encoder.write_varint(*self as u64)
    }
}

impl<'de> Decode<'de> for usize {
    #[inline]
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        let value = decoder.read_varint()?;
        usize::try_from(value).map_err(|_| decoder.err(ErrorKind::LengthOverflow))
    }
}

macro_rules! impl_varint_signed {
    ($($ty:ty),* $(,)?) => {
        $(
            impl Encode for $ty {
                const KIND: u64 = crate::fingerprint_init(stringify!($ty));

                #[inline]
                fn encoded_len(&self) -> Option<usize> {
                    Some(zigzag_len(*self as i64))
                }

                #[inline]
                fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
                    encoder.write_zigzag(*self as i64)
                }
            }

            impl<'de> Decode<'de> for $ty {
                #[inline]
                fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
                    let value = decoder.read_zigzag()?;
                    <$ty>::try_from(value).map_err(|_| decoder.err(ErrorKind::VarintOverflow))
                }
            }
        )*
    };
}

impl_varint_signed!(i16, i32, i64);

impl Encode for i128 {
    const FINGERPRINT: u64 = crate::fingerprint("i128", &[]);
    const KIND: u64 = crate::fingerprint_init("i128");

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        Some(zigzag_len_i128(*self))
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        encoder.write_zigzag_i128(*self)
    }
}

impl<'de> Decode<'de> for i128 {
    #[inline]
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        decoder.read_zigzag_i128()
    }
}

impl Encode for isize {
    const KIND: u64 = crate::fingerprint_init("isize");

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        Some(zigzag_len(*self as i64))
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        encoder.write_zigzag(*self as i64)
    }
}

impl<'de> Decode<'de> for isize {
    #[inline]
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        let value = decoder.read_zigzag()?;
        isize::try_from(value).map_err(|_| decoder.err(ErrorKind::VarintOverflow))
    }
}

impl Encode for f32 {
    const KIND: u64 = crate::fingerprint_init("f32");

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        Some(4)
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        encoder.write_f32(*self)
    }
}

impl<'de> Decode<'de> for f32 {
    #[inline]
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        decoder.read_f32()
    }
}

impl Encode for f64 {
    const KIND: u64 = crate::fingerprint_init("f64");

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        Some(8)
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        encoder.write_f64(*self)
    }
}

impl<'de> Decode<'de> for f64 {
    #[inline]
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        decoder.read_f64()
    }
}

impl Encode for char {
    const KIND: u64 = crate::fingerprint_init("char");

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        Some(varint_len(*self as u32 as u64))
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        encoder.write_varint(*self as u32 as u64)
    }
}

impl<'de> Decode<'de> for char {
    #[inline]
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        let value = decoder.read_varint()?;
        let value =
            u32::try_from(value).map_err(|_| decoder.err(ErrorKind::InvalidChar(u32::MAX)))?;
        char::from_u32(value).ok_or_else(|| decoder.err(ErrorKind::InvalidChar(value)))
    }
}

// ---------------------------------------------------------------------------
// References
// ---------------------------------------------------------------------------

impl<T: Encode + ?Sized> Encode for &T {
    const FINGERPRINT: u64 = T::FINGERPRINT;
    const KIND: u64 = T::KIND;

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        (**self).encoded_len()
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        (**self).encode(encoder)
    }
}

impl<T: Encode + ?Sized> Encode for &mut T {
    const FINGERPRINT: u64 = T::FINGERPRINT;
    const KIND: u64 = T::KIND;

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        (**self).encoded_len()
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        (**self).encode(encoder)
    }
}

// ---------------------------------------------------------------------------
// Strings and bytes
// ---------------------------------------------------------------------------

impl Encode for str {
    const KIND: u64 = crate::fingerprint_init("str");

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        let len = self.len();
        varint_len(len as u64).checked_add(len)
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        encoder.write_str(self)
    }
}

impl<'de: 'a, 'a> Decode<'de> for &'a str {
    #[inline]
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        decoder.read_str()
    }
}

impl<'de: 'a, 'a> Decode<'de> for &'a [u8] {
    #[inline]
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        decoder.read_bytes()
    }
}

#[cfg(feature = "alloc")]
impl Encode for String {
    const FINGERPRINT: u64 = crate::fingerprint("String", &[]);
    const KIND: u64 = crate::fingerprint_init("String");

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        self.as_str().encoded_len()
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        encoder.write_str(self)
    }
}

#[cfg(feature = "alloc")]
impl<'de> Decode<'de> for String {
    #[inline]
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        Ok(decoder.read_str()?.into())
    }
}

// ---------------------------------------------------------------------------
// Sequences
// ---------------------------------------------------------------------------

impl<T: Encode> Encode for [T] {
    const KIND: u64 = crate::fingerprint_combine(crate::fingerprint_init("slice"), &[T::KIND]);

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        let mut len = varint_len(self.len() as u64);
        for item in self {
            len = len.checked_add(item.encoded_len()?)?;
        }
        Some(len)
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        encoder.write_len(self.len())?;
        for item in self {
            item.encode(encoder)?;
        }
        Ok(())
    }
}

#[cfg(feature = "alloc")]
impl<T: Encode> Encode for Vec<T> {
    const FINGERPRINT: u64 = crate::fingerprint("Vec", &[T::FINGERPRINT]);
    const KIND: u64 = crate::fingerprint_combine(crate::fingerprint_init("Vec"), &[T::KIND]);

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        self.as_slice().encoded_len()
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        self.as_slice().encode(encoder)
    }
}

#[cfg(feature = "alloc")]
impl<'de, T: Decode<'de>> Decode<'de> for Vec<T> {
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        let len = decoder.read_seq_len(min_element_size::<T>())?;
        let capacity = core::cmp::min(len, decoder.remaining().saturating_add(1));
        decoder.enter()?;
        let mut out = Vec::with_capacity(capacity);
        for _ in 0..len {
            out.push(T::decode(decoder)?);
        }
        decoder.leave();
        Ok(out)
    }
}

/// Minimum number of bytes an element of `T` occupies on the wire.
///
/// This is only used to bound pre-allocation against the remaining input, so a
/// conservative `0` for zero-sized types and `1` otherwise is enough.
#[cfg(feature = "alloc")]
#[inline]
const fn min_element_size<T>() -> usize {
    if core::mem::size_of::<T>() == 0 {
        0
    } else {
        1
    }
}

impl<T: Encode, const N: usize> Encode for [T; N] {
    const FINGERPRINT: u64 = crate::fingerprint("Array", &[T::FINGERPRINT]);
    const KIND: u64 = crate::fingerprint_combine(crate::fingerprint_init("Array"), &[T::KIND]);

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        let mut len = 0usize;
        for item in self {
            len = len.checked_add(item.encoded_len()?)?;
        }
        Some(len)
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        for item in self {
            item.encode(encoder)?;
        }
        Ok(())
    }
}

#[cfg(feature = "alloc")]
impl<'de, T: Decode<'de>, const N: usize> Decode<'de> for [T; N] {
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        decoder.enter()?;
        let mut out = Vec::with_capacity(N);
        for _ in 0..N {
            out.push(T::decode(decoder)?);
        }
        decoder.leave();
        out.try_into()
            .map_err(|_| decoder.err(ErrorKind::Custom("array length mismatch")))
    }
}

#[cfg(feature = "alloc")]
impl<T: Encode> Encode for VecDeque<T> {
    const FINGERPRINT: u64 = crate::fingerprint("VecDeque", &[T::FINGERPRINT]);
    const KIND: u64 = crate::fingerprint_combine(crate::fingerprint_init("VecDeque"), &[T::KIND]);

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        let mut len = varint_len(self.len() as u64);
        for item in self {
            len = len.checked_add(item.encoded_len()?)?;
        }
        Some(len)
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        encoder.write_len(self.len())?;
        for item in self {
            item.encode(encoder)?;
        }
        Ok(())
    }
}

#[cfg(feature = "alloc")]
impl<'de, T: Decode<'de>> Decode<'de> for VecDeque<T> {
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        let len = decoder.read_seq_len(min_element_size::<T>())?;
        let capacity = core::cmp::min(len, decoder.remaining().saturating_add(1));
        decoder.enter()?;
        let mut out = VecDeque::with_capacity(capacity);
        for _ in 0..len {
            out.push_back(T::decode(decoder)?);
        }
        decoder.leave();
        Ok(out)
    }
}

#[cfg(feature = "alloc")]
impl<T: Encode> Encode for LinkedList<T> {
    const FINGERPRINT: u64 = crate::fingerprint("LinkedList", &[T::FINGERPRINT]);
    const KIND: u64 = crate::fingerprint_combine(crate::fingerprint_init("LinkedList"), &[T::KIND]);

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        let mut len = varint_len(self.len() as u64);
        for item in self {
            len = len.checked_add(item.encoded_len()?)?;
        }
        Some(len)
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        encoder.write_len(self.len())?;
        for item in self {
            item.encode(encoder)?;
        }
        Ok(())
    }
}

#[cfg(feature = "alloc")]
impl<'de, T: Decode<'de>> Decode<'de> for LinkedList<T> {
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        let len = decoder.read_seq_len(min_element_size::<T>())?;
        decoder.enter()?;
        let mut out = LinkedList::new();
        for _ in 0..len {
            out.push_back(T::decode(decoder)?);
        }
        decoder.leave();
        Ok(out)
    }
}

#[cfg(feature = "alloc")]
impl<T: Encode + Ord> Encode for BinaryHeap<T> {
    const FINGERPRINT: u64 = crate::fingerprint("BinaryHeap", &[T::FINGERPRINT]);
    const KIND: u64 = crate::fingerprint_combine(crate::fingerprint_init("BinaryHeap"), &[T::KIND]);

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        let mut len = varint_len(self.len() as u64);
        for item in self {
            len = len.checked_add(item.encoded_len()?)?;
        }
        Some(len)
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        encoder.write_len(self.len())?;
        for item in self {
            item.encode(encoder)?;
        }
        Ok(())
    }
}

#[cfg(feature = "alloc")]
impl<'de, T: Decode<'de> + Ord> Decode<'de> for BinaryHeap<T> {
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        let len = decoder.read_seq_len(min_element_size::<T>())?;
        decoder.enter()?;
        let mut out =
            BinaryHeap::with_capacity(core::cmp::min(len, decoder.remaining().saturating_add(1)));
        for _ in 0..len {
            out.push(T::decode(decoder)?);
        }
        decoder.leave();
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Maps and sets
// ---------------------------------------------------------------------------

#[cfg(feature = "alloc")]
macro_rules! impl_btree_map {
    ($ty:ident) => {
        #[cfg(feature = "alloc")]
        impl<K: Encode, V: Encode> Encode for $ty<K, V> {
            const FINGERPRINT: u64 =
                crate::fingerprint(stringify!($ty), &[K::FINGERPRINT, V::FINGERPRINT]);
            const KIND: u64 = crate::fingerprint_combine(
                crate::fingerprint_init(stringify!($ty)),
                &[K::KIND, V::KIND],
            );

            #[inline]
            fn encoded_len(&self) -> Option<usize> {
                let mut len = varint_len(self.len() as u64);
                for (key, value) in self {
                    len = len.checked_add(key.encoded_len()?)?;
                    len = len.checked_add(value.encoded_len()?)?;
                }
                Some(len)
            }

            #[inline]
            fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
                encoder.write_len(self.len())?;
                for (key, value) in self {
                    key.encode(encoder)?;
                    value.encode(encoder)?;
                }
                Ok(())
            }
        }
    };
}

#[cfg(feature = "alloc")]
impl_btree_map!(BTreeMap);

#[cfg(feature = "alloc")]
impl<'de, K: Decode<'de> + Ord, V: Decode<'de>> Decode<'de> for BTreeMap<K, V> {
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        let len = decoder.read_seq_len(1)?;
        decoder.enter()?;
        let mut out = BTreeMap::new();
        for _ in 0..len {
            let key = K::decode(decoder)?;
            let value = V::decode(decoder)?;
            out.insert(key, value);
        }
        decoder.leave();
        Ok(out)
    }
}

#[cfg(feature = "alloc")]
macro_rules! impl_btree_set {
    ($ty:ident) => {
        #[cfg(feature = "alloc")]
        impl<T: Encode> Encode for $ty<T> {
            const FINGERPRINT: u64 = crate::fingerprint(stringify!($ty), &[T::FINGERPRINT]);
            const KIND: u64 =
                crate::fingerprint_combine(crate::fingerprint_init(stringify!($ty)), &[T::KIND]);

            #[inline]
            fn encoded_len(&self) -> Option<usize> {
                let mut len = varint_len(self.len() as u64);
                for item in self {
                    len = len.checked_add(item.encoded_len()?)?;
                }
                Some(len)
            }

            #[inline]
            fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
                encoder.write_len(self.len())?;
                for item in self {
                    item.encode(encoder)?;
                }
                Ok(())
            }
        }
    };
}

#[cfg(feature = "alloc")]
impl_btree_set!(BTreeSet);

#[cfg(feature = "alloc")]
impl<'de, T: Decode<'de> + Ord> Decode<'de> for BTreeSet<T> {
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        let len = decoder.read_seq_len(min_element_size::<T>())?;
        decoder.enter()?;
        let mut out = BTreeSet::new();
        for _ in 0..len {
            out.insert(T::decode(decoder)?);
        }
        decoder.leave();
        Ok(out)
    }
}

#[cfg(feature = "std")]
impl<K: Encode, V: Encode> Encode for HashMap<K, V> {
    const FINGERPRINT: u64 = crate::fingerprint("HashMap", &[K::FINGERPRINT, V::FINGERPRINT]);
    const KIND: u64 =
        crate::fingerprint_combine(crate::fingerprint_init("HashMap"), &[K::KIND, V::KIND]);

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        let mut len = varint_len(self.len() as u64);
        for (key, value) in self {
            len = len.checked_add(key.encoded_len()?)?;
            len = len.checked_add(value.encoded_len()?)?;
        }
        Some(len)
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        encoder.write_len(self.len())?;
        for (key, value) in self {
            key.encode(encoder)?;
            value.encode(encoder)?;
        }
        Ok(())
    }
}

#[cfg(feature = "std")]
impl<'de, K, V> Decode<'de> for HashMap<K, V>
where
    K: Decode<'de> + Eq + core::hash::Hash,
    V: Decode<'de>,
{
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        let len = decoder.read_seq_len(1)?;
        decoder.enter()?;
        let mut out =
            HashMap::with_capacity(core::cmp::min(len, decoder.remaining().saturating_add(1)));
        for _ in 0..len {
            let key = K::decode(decoder)?;
            let value = V::decode(decoder)?;
            out.insert(key, value);
        }
        decoder.leave();
        Ok(out)
    }
}

#[cfg(feature = "std")]
impl<T: Encode> Encode for HashSet<T> {
    const FINGERPRINT: u64 = crate::fingerprint("HashSet", &[T::FINGERPRINT]);
    const KIND: u64 = crate::fingerprint_combine(crate::fingerprint_init("HashSet"), &[T::KIND]);

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        let mut len = varint_len(self.len() as u64);
        for item in self {
            len = len.checked_add(item.encoded_len()?)?;
        }
        Some(len)
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        encoder.write_len(self.len())?;
        for item in self {
            item.encode(encoder)?;
        }
        Ok(())
    }
}

#[cfg(feature = "std")]
impl<'de, T> Decode<'de> for HashSet<T>
where
    T: Decode<'de> + Eq + core::hash::Hash,
{
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        let len = decoder.read_seq_len(min_element_size::<T>())?;
        decoder.enter()?;
        let mut out =
            HashSet::with_capacity(core::cmp::min(len, decoder.remaining().saturating_add(1)));
        for _ in 0..len {
            out.insert(T::decode(decoder)?);
        }
        decoder.leave();
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Deterministic encoding (`#[ordpack(sort)]`)
// ---------------------------------------------------------------------------

#[cfg(feature = "std")]
impl<K: Encode, V: Encode> crate::SortedEncode for HashMap<K, V> {
    fn encode_sorted<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        // Buffer only the encoded keys, sort spans of that buffer, then write
        // each key followed by its value straight into the output. Two
        // allocations total, independent of map size.
        let mut key_buf = Vec::new();
        let mut entries: Vec<(usize, usize, &V)> = Vec::with_capacity(self.len());
        for (key, value) in self {
            let start = key_buf.len();
            {
                let mut inner = Encoder::new(&mut key_buf);
                key.encode(&mut inner)?;
            }
            entries.push((start, key_buf.len(), value));
        }

        let keys = &key_buf;
        entries.sort_unstable_by(|left, right| keys[left.0..left.1].cmp(&keys[right.0..right.1]));

        encoder.write_len(entries.len())?;
        for &(start, end, value) in &entries {
            encoder.write_raw(&keys[start..end])?;
            value.encode(encoder)?;
        }
        Ok(())
    }
}

#[cfg(feature = "std")]
impl<T: Encode> crate::SortedEncode for HashSet<T> {
    fn encode_sorted<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        let mut key_buf = Vec::new();
        let mut spans: Vec<(usize, usize)> = Vec::with_capacity(self.len());
        for item in self {
            let start = key_buf.len();
            {
                let mut inner = Encoder::new(&mut key_buf);
                item.encode(&mut inner)?;
            }
            spans.push((start, key_buf.len()));
        }

        let keys = &key_buf;
        spans.sort_unstable_by(|left, right| keys[left.0..left.1].cmp(&keys[right.0..right.1]));

        encoder.write_len(spans.len())?;
        for &(start, end) in &spans {
            encoder.write_raw(&keys[start..end])?;
        }
        Ok(())
    }
}

#[cfg(feature = "alloc")]
impl<K: Encode, V: Encode> crate::SortedEncode for BTreeMap<K, V> {
    #[inline]
    fn encode_sorted<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        self.encode(encoder)
    }
}

#[cfg(feature = "alloc")]
impl<T: Encode> crate::SortedEncode for BTreeSet<T> {
    #[inline]
    fn encode_sorted<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        self.encode(encoder)
    }
}

// ---------------------------------------------------------------------------
// Option / Result
// ---------------------------------------------------------------------------

impl<T: Encode> Encode for Option<T> {
    const FINGERPRINT: u64 = crate::fingerprint("Option", &[T::FINGERPRINT]);
    const KIND: u64 = crate::fingerprint_combine(crate::fingerprint_init("Option"), &[T::KIND]);

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        match self {
            None => Some(1),
            Some(value) => Some(value.encoded_len()?.checked_add(1)?),
        }
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        match self {
            None => encoder.write_u8(0),
            Some(value) => {
                encoder.write_u8(1)?;
                value.encode(encoder)
            }
        }
    }
}

impl<'de, T: Decode<'de>> Decode<'de> for Option<T> {
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        match decoder.read_u8()? {
            0 => Ok(None),
            1 => {
                decoder.enter()?;
                let value = T::decode(decoder)?;
                decoder.leave();
                Ok(Some(value))
            }
            other => Err(decoder.err(ErrorKind::InvalidTag { got: other, max: 1 })),
        }
    }
}

impl<T: Encode, E: Encode> Encode for Result<T, E> {
    const FINGERPRINT: u64 = crate::fingerprint("Result", &[T::FINGERPRINT, E::FINGERPRINT]);
    const KIND: u64 =
        crate::fingerprint_combine(crate::fingerprint_init("Result"), &[T::KIND, E::KIND]);

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        match self {
            Ok(value) => Some(value.encoded_len()?.checked_add(1)?),
            Err(error) => Some(error.encoded_len()?.checked_add(1)?),
        }
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        match self {
            Ok(value) => {
                encoder.write_u8(0)?;
                value.encode(encoder)
            }
            Err(error) => {
                encoder.write_u8(1)?;
                error.encode(encoder)
            }
        }
    }
}

impl<'de, T: Decode<'de>, E: Decode<'de>> Decode<'de> for Result<T, E> {
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        match decoder.read_u8()? {
            0 => Ok(Ok(T::decode(decoder)?)),
            1 => Ok(Err(E::decode(decoder)?)),
            other => Err(decoder.err(ErrorKind::InvalidTag { got: other, max: 1 })),
        }
    }
}

// ---------------------------------------------------------------------------
// Tuples
// ---------------------------------------------------------------------------

macro_rules! impl_tuple {
    ($(($idx:tt, $ty:ident)),+ $(,)?) => {
        impl<$($ty: Encode),+> Encode for ($($ty,)+) {
            const FINGERPRINT: u64 = crate::fingerprint(
                concat!("tuple", stringify!($($ty),+)),
                &[$($ty::FINGERPRINT),+],
            );
            const KIND: u64 = crate::fingerprint_combine(
                crate::fingerprint_init("tuple"),
                &[$($ty::KIND),+],
            );

            #[inline]
            fn encoded_len(&self) -> Option<usize> {
                let mut len = 0usize;
                $(len = len.checked_add(self.$idx.encoded_len()?)?;)+
                Some(len)
            }

            #[inline]
            fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
                $(self.$idx.encode(encoder)?;)+
                Ok(())
            }
        }

        impl<'de, $($ty: Decode<'de>),+> Decode<'de> for ($($ty,)+) {
            #[inline]
            fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
                Ok(($($ty::decode(decoder)?,)+))
            }
        }
    };
}

impl_tuple!((0, A));
impl_tuple!((0, A), (1, B));
impl_tuple!((0, A), (1, B), (2, C));
impl_tuple!((0, A), (1, B), (2, C), (3, D));
impl_tuple!((0, A), (1, B), (2, C), (3, D), (4, E));
impl_tuple!((0, A), (1, B), (2, C), (3, D), (4, E), (5, F));
impl_tuple!((0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G));
impl_tuple!(
    (0, A),
    (1, B),
    (2, C),
    (3, D),
    (4, E),
    (5, F),
    (6, G),
    (7, H)
);
impl_tuple!(
    (0, A),
    (1, B),
    (2, C),
    (3, D),
    (4, E),
    (5, F),
    (6, G),
    (7, H),
    (8, I)
);
impl_tuple!(
    (0, A),
    (1, B),
    (2, C),
    (3, D),
    (4, E),
    (5, F),
    (6, G),
    (7, H),
    (8, I),
    (9, J)
);
impl_tuple!(
    (0, A),
    (1, B),
    (2, C),
    (3, D),
    (4, E),
    (5, F),
    (6, G),
    (7, H),
    (8, I),
    (9, J),
    (10, K)
);
impl_tuple!(
    (0, A),
    (1, B),
    (2, C),
    (3, D),
    (4, E),
    (5, F),
    (6, G),
    (7, H),
    (8, I),
    (9, J),
    (10, K),
    (11, L)
);

// ---------------------------------------------------------------------------
// Smart pointers and wrappers
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Bytes (bulk byte buffers)
// ---------------------------------------------------------------------------

#[cfg(feature = "alloc")]
impl Encode for crate::Bytes<Vec<u8>> {
    const FINGERPRINT: u64 = <Vec<u8> as Encode>::FINGERPRINT;
    const KIND: u64 = <Vec<u8> as Encode>::KIND;

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        let len = self.0.len();
        Some(varint_len(len as u64) + len)
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        encoder.write_bytes(&self.0)
    }
}

#[cfg(feature = "alloc")]
impl<'de> Decode<'de> for crate::Bytes<Vec<u8>> {
    #[inline]
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        Ok(crate::Bytes(decoder.read_bytes()?.to_vec()))
    }
}

#[cfg(feature = "alloc")]
impl Encode for crate::Bytes<&[u8]> {
    const FINGERPRINT: u64 = <Vec<u8> as Encode>::FINGERPRINT;
    const KIND: u64 = <Vec<u8> as Encode>::KIND;

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        let len = self.0.len();
        Some(varint_len(len as u64) + len)
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        encoder.write_bytes(self.0)
    }
}

#[cfg(feature = "alloc")]
impl<'de> Decode<'de> for crate::Bytes<&'de [u8]> {
    #[inline]
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        Ok(crate::Bytes(decoder.read_bytes()?))
    }
}

#[cfg(feature = "alloc")]
impl Encode for crate::Bytes<Box<[u8]>> {
    const FINGERPRINT: u64 = <Vec<u8> as Encode>::FINGERPRINT;
    const KIND: u64 = <Vec<u8> as Encode>::KIND;

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        let len = self.0.len();
        Some(varint_len(len as u64) + len)
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        encoder.write_bytes(&self.0)
    }
}

#[cfg(feature = "alloc")]
impl<'de> Decode<'de> for crate::Bytes<Box<[u8]>> {
    #[inline]
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        Ok(crate::Bytes(
            decoder.read_bytes()?.to_vec().into_boxed_slice(),
        ))
    }
}

impl<const N: usize> Encode for crate::Bytes<[u8; N]> {
    const FINGERPRINT: u64 = <[u8; N] as Encode>::FINGERPRINT;
    const KIND: u64 = <[u8; N] as Encode>::KIND;

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        Some(N)
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        encoder.write_raw(&self.0)
    }
}

impl<'de, const N: usize> Decode<'de> for crate::Bytes<[u8; N]> {
    #[inline]
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        let bytes = decoder.read_raw(N)?;
        let mut out = [0u8; N];
        out.copy_from_slice(bytes);
        Ok(crate::Bytes(out))
    }
}

#[cfg(feature = "alloc")]
impl<T: Encode + ?Sized> Encode for Box<T> {
    const FINGERPRINT: u64 = T::FINGERPRINT;
    const KIND: u64 = T::KIND;

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        (**self).encoded_len()
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        (**self).encode(encoder)
    }
}

#[cfg(feature = "alloc")]
impl<'de, T: Decode<'de>> Decode<'de> for Box<T> {
    #[inline]
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        Ok(Box::new(T::decode(decoder)?))
    }
}

#[cfg(feature = "alloc")]
impl<T: Encode + ?Sized> Encode for Rc<T> {
    const FINGERPRINT: u64 = T::FINGERPRINT;
    const KIND: u64 = T::KIND;

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        (**self).encoded_len()
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        (**self).encode(encoder)
    }
}

#[cfg(feature = "alloc")]
impl<'de, T: Decode<'de>> Decode<'de> for Rc<T> {
    #[inline]
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        Ok(Rc::new(T::decode(decoder)?))
    }
}

#[cfg(feature = "alloc")]
impl<T: Encode + ?Sized> Encode for Arc<T> {
    const FINGERPRINT: u64 = T::FINGERPRINT;
    const KIND: u64 = T::KIND;

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        (**self).encoded_len()
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        (**self).encode(encoder)
    }
}

#[cfg(feature = "alloc")]
impl<'de, T: Decode<'de>> Decode<'de> for Arc<T> {
    #[inline]
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        Ok(Arc::new(T::decode(decoder)?))
    }
}

#[cfg(feature = "alloc")]
impl<'a, T> Encode for Cow<'a, T>
where
    T: ToOwned + ?Sized + Encode,
{
    const FINGERPRINT: u64 = T::FINGERPRINT;
    const KIND: u64 = T::KIND;

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        self.as_ref().encoded_len()
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        self.as_ref().encode(encoder)
    }
}

#[cfg(feature = "alloc")]
impl<'de, T> Decode<'de> for Cow<'de, T>
where
    T: ToOwned + ?Sized,
    T::Owned: Decode<'de>,
{
    #[inline]
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        Ok(Cow::Owned(T::Owned::decode(decoder)?))
    }
}

impl<T: Encode> Encode for core::marker::PhantomData<T> {
    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        Some(0)
    }

    #[inline]
    fn encode<W: Writer>(&self, _encoder: &mut Encoder<W>) -> Result<(), Error> {
        Ok(())
    }
}

impl<'de, T> Decode<'de> for core::marker::PhantomData<T> {
    #[inline]
    fn decode<R: Reader<'de>>(_decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        Ok(core::marker::PhantomData)
    }
}

impl<T: Encode> Encode for core::num::Wrapping<T> {
    const FINGERPRINT: u64 = T::FINGERPRINT;

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        self.0.encoded_len()
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        self.0.encode(encoder)
    }
}

impl<'de, T: Decode<'de>> Decode<'de> for core::num::Wrapping<T> {
    #[inline]
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        Ok(core::num::Wrapping(T::decode(decoder)?))
    }
}

impl<T: Encode> Encode for core::cmp::Reverse<T> {
    const FINGERPRINT: u64 = T::FINGERPRINT;

    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        self.0.encoded_len()
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        self.0.encode(encoder)
    }
}

impl<'de, T: Decode<'de>> Decode<'de> for core::cmp::Reverse<T> {
    #[inline]
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        Ok(core::cmp::Reverse(T::decode(decoder)?))
    }
}

impl<T: Copy + Encode> Encode for core::cell::Cell<T> {
    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        self.get().encoded_len()
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        self.get().encode(encoder)
    }
}

impl<'de, T: Decode<'de>> Decode<'de> for core::cell::Cell<T> {
    #[inline]
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        Ok(core::cell::Cell::new(T::decode(decoder)?))
    }
}

impl<T: Encode> Encode for core::cell::RefCell<T> {
    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        self.borrow().encoded_len()
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        self.borrow().encode(encoder)
    }
}

impl<'de, T: Decode<'de>> Decode<'de> for core::cell::RefCell<T> {
    #[inline]
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        Ok(core::cell::RefCell::new(T::decode(decoder)?))
    }
}

// ---------------------------------------------------------------------------
// Non-zero integers
// ---------------------------------------------------------------------------

macro_rules! impl_nonzero {
    ($($ty:ty => $inner:ty),* $(,)?) => {
        $(
            impl Encode for $ty {
                #[inline]
                fn encoded_len(&self) -> Option<usize> {
                    self.get().encoded_len()
                }

                #[inline]
                fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
                    self.get().encode(encoder)
                }
            }

            impl<'de> Decode<'de> for $ty {
                #[inline]
                fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
                    let value = <$inner>::decode(decoder)?;
                    <$ty>::new(value).ok_or_else(|| decoder.err(ErrorKind::NonZero))
                }
            }
        )*
    };
}

impl_nonzero!(
    core::num::NonZeroU8 => u8,
    core::num::NonZeroU16 => u16,
    core::num::NonZeroU32 => u32,
    core::num::NonZeroU64 => u64,
    core::num::NonZeroU128 => u128,
    core::num::NonZeroUsize => usize,
    core::num::NonZeroI8 => i8,
    core::num::NonZeroI16 => i16,
    core::num::NonZeroI32 => i32,
    core::num::NonZeroI64 => i64,
    core::num::NonZeroI128 => i128,
    core::num::NonZeroIsize => isize,
);

// ---------------------------------------------------------------------------
// Time
// ---------------------------------------------------------------------------

impl Encode for Duration {
    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        Some(varint_len(self.as_secs()) + varint_len(self.subsec_nanos() as u64))
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        encoder.write_varint(self.as_secs())?;
        encoder.write_varint(self.subsec_nanos() as u64)
    }
}

impl<'de> Decode<'de> for Duration {
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        let secs = decoder.read_varint()?;
        let nanos = decoder.read_varint()?;
        if nanos >= 1_000_000_000 {
            return Err(decoder.err(ErrorKind::Custom("invalid Duration nanoseconds")));
        }
        Ok(Duration::new(secs, nanos as u32))
    }
}

#[cfg(feature = "std")]
impl Encode for SystemTime {
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        match self.duration_since(UNIX_EPOCH) {
            Ok(duration) => {
                encoder.write_zigzag(duration.as_secs() as i64)?;
                encoder.write_varint(duration.subsec_nanos() as u64)
            }
            Err(err) => {
                let duration = err.duration();
                let secs = duration.as_secs() as i64;
                let nanos = duration.subsec_nanos();
                if nanos == 0 {
                    encoder.write_zigzag(-secs)?;
                    encoder.write_varint(0)
                } else {
                    encoder.write_zigzag(-secs - 1)?;
                    encoder.write_varint((1_000_000_000 - nanos) as u64)
                }
            }
        }
    }
}

#[cfg(feature = "std")]
impl<'de> Decode<'de> for SystemTime {
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        let secs = decoder.read_zigzag()?;
        let nanos = decoder.read_varint()?;
        if nanos >= 1_000_000_000 {
            return Err(decoder.err(ErrorKind::Custom("invalid SystemTime nanoseconds")));
        }
        let base = if secs >= 0 {
            UNIX_EPOCH.checked_add(Duration::new(secs as u64, 0))
        } else {
            let magnitude = (-(secs + 1)) as u64 + 1;
            UNIX_EPOCH.checked_sub(Duration::new(magnitude, 0))
        };
        let base = base.ok_or_else(|| decoder.err(ErrorKind::Custom("SystemTime overflow")))?;
        base.checked_add(Duration::from_nanos(nanos))
            .ok_or_else(|| decoder.err(ErrorKind::Custom("SystemTime overflow")))
    }
}

// ---------------------------------------------------------------------------
// Networking
// ---------------------------------------------------------------------------

impl Encode for IpAddr {
    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        Some(match self {
            IpAddr::V4(_) => 5,
            IpAddr::V6(_) => 17,
        })
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        match self {
            IpAddr::V4(addr) => {
                encoder.write_u8(4)?;
                encoder.write_raw(&addr.octets())
            }
            IpAddr::V6(addr) => {
                encoder.write_u8(6)?;
                encoder.write_raw(&addr.octets())
            }
        }
    }
}

impl<'de> Decode<'de> for IpAddr {
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        match decoder.read_u8()? {
            4 => {
                let bytes = decoder.read_raw(4)?;
                Ok(IpAddr::V4(Ipv4Addr::new(
                    bytes[0], bytes[1], bytes[2], bytes[3],
                )))
            }
            6 => {
                let bytes = decoder.read_raw(16)?;
                let mut octets = [0u8; 16];
                octets.copy_from_slice(bytes);
                Ok(IpAddr::V6(Ipv6Addr::from(octets)))
            }
            other => Err(decoder.err(ErrorKind::InvalidTag { got: other, max: 6 })),
        }
    }
}

impl Encode for SocketAddr {
    fn encoded_len(&self) -> Option<usize> {
        Some(match self {
            SocketAddr::V4(addr) => 1 + 4 + varint_len(addr.port() as u64),
            SocketAddr::V6(addr) => {
                1 + 16
                    + varint_len(addr.port() as u64)
                    + varint_len(addr.flowinfo() as u64)
                    + varint_len(addr.scope_id() as u64)
            }
        })
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        match self {
            SocketAddr::V4(addr) => {
                encoder.write_u8(4)?;
                encoder.write_raw(&addr.ip().octets())?;
                encoder.write_varint(addr.port() as u64)
            }
            SocketAddr::V6(addr) => {
                encoder.write_u8(6)?;
                encoder.write_raw(&addr.ip().octets())?;
                encoder.write_varint(addr.port() as u64)?;
                encoder.write_varint(addr.flowinfo() as u64)?;
                encoder.write_varint(addr.scope_id() as u64)
            }
        }
    }
}

impl<'de> Decode<'de> for SocketAddr {
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        match decoder.read_u8()? {
            4 => {
                let bytes = decoder.read_raw(4)?;
                let ip = Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3]);
                let port = u16::try_from(decoder.read_varint()?)
                    .map_err(|_| decoder.err(ErrorKind::VarintOverflow))?;
                Ok(SocketAddr::V4(SocketAddrV4::new(ip, port)))
            }
            6 => {
                let bytes = decoder.read_raw(16)?;
                let mut octets = [0u8; 16];
                octets.copy_from_slice(bytes);
                let ip = Ipv6Addr::from(octets);
                let port = u16::try_from(decoder.read_varint()?)
                    .map_err(|_| decoder.err(ErrorKind::VarintOverflow))?;
                let flowinfo = u32::try_from(decoder.read_varint()?)
                    .map_err(|_| decoder.err(ErrorKind::VarintOverflow))?;
                let scope_id = u32::try_from(decoder.read_varint()?)
                    .map_err(|_| decoder.err(ErrorKind::VarintOverflow))?;
                Ok(SocketAddr::V6(SocketAddrV6::new(
                    ip, port, flowinfo, scope_id,
                )))
            }
            other => Err(decoder.err(ErrorKind::InvalidTag { got: other, max: 6 })),
        }
    }
}

// ---------------------------------------------------------------------------
// Paths and OS strings
// ---------------------------------------------------------------------------

#[cfg(feature = "std")]
impl Encode for Path {
    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        self.to_str().and_then(|s| s.encoded_len())
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        let s = self
            .to_str()
            .ok_or_else(|| Error::new(ErrorKind::Unsupported("non-UTF-8 path")))?;
        encoder.write_str(s)
    }
}

#[cfg(feature = "std")]
impl Encode for PathBuf {
    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        self.as_path().encoded_len()
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        self.as_path().encode(encoder)
    }
}

#[cfg(feature = "std")]
impl<'de> Decode<'de> for PathBuf {
    #[inline]
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        Ok(PathBuf::from(decoder.read_str()?))
    }
}

#[cfg(feature = "std")]
impl Encode for OsStr {
    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        self.to_str().and_then(|s| s.encoded_len())
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        let s = self
            .to_str()
            .ok_or_else(|| Error::new(ErrorKind::Unsupported("non-UTF-8 OsStr")))?;
        encoder.write_str(s)
    }
}

#[cfg(feature = "std")]
impl Encode for OsString {
    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        self.as_os_str().encoded_len()
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        self.as_os_str().encode(encoder)
    }
}

#[cfg(feature = "std")]
impl<'de> Decode<'de> for OsString {
    #[inline]
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        Ok(OsString::from(decoder.read_str()?))
    }
}

#[cfg(feature = "std")]
impl Encode for CStr {
    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        let len = self.to_bytes().len();
        varint_len(len as u64).checked_add(len)
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        encoder.write_bytes(self.to_bytes())
    }
}

#[cfg(feature = "std")]
impl Encode for CString {
    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        self.as_c_str().encoded_len()
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        self.as_c_str().encode(encoder)
    }
}

#[cfg(feature = "std")]
impl<'de> Decode<'de> for CString {
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        let bytes = decoder.read_bytes()?;
        CString::new(bytes).map_err(|_| decoder.err(ErrorKind::Custom("interior NUL in CString")))
    }
}

// ---------------------------------------------------------------------------
// Locks
// ---------------------------------------------------------------------------

#[cfg(feature = "std")]
impl<T: Encode> Encode for Mutex<T> {
    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        self.lock().ok()?.encoded_len()
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        let guard = self
            .lock()
            .map_err(|_| Error::new(ErrorKind::Custom("poisoned Mutex")))?;
        guard.encode(encoder)
    }
}

#[cfg(feature = "std")]
impl<'de, T: Decode<'de>> Decode<'de> for Mutex<T> {
    #[inline]
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        Ok(Mutex::new(T::decode(decoder)?))
    }
}

#[cfg(feature = "std")]
impl<T: Encode> Encode for RwLock<T> {
    #[inline]
    fn encoded_len(&self) -> Option<usize> {
        self.read().ok()?.encoded_len()
    }

    #[inline]
    fn encode<W: Writer>(&self, encoder: &mut Encoder<W>) -> Result<(), Error> {
        let guard = self
            .read()
            .map_err(|_| Error::new(ErrorKind::Custom("poisoned RwLock")))?;
        guard.encode(encoder)
    }
}

#[cfg(feature = "std")]
impl<'de, T: Decode<'de>> Decode<'de> for RwLock<T> {
    #[inline]
    fn decode<R: Reader<'de>>(decoder: &mut Decoder<'de, R>) -> Result<Self, Error> {
        Ok(RwLock::new(T::decode(decoder)?))
    }
}
