//! Support for the code that runtime macros generate.

pub use serde;

use crate::{deserialize_tagged, serialize_tagged};

/// Encodes record fields in the format expected by the browser runtime.
///
/// # Errors
///
/// Fails when a field cannot be serialized.
pub fn serialize_record<T, S>(serializer: S, fields: &T) -> Result<S::Ok, S::Error>
where
    T: serde::Serialize,
    S: serde::Serializer,
{
    serialize_tagged(serializer, "Record", fields)
}

/// Decodes record fields from the browser runtime's format.
///
/// # Errors
///
/// Fails when the input lacks the record tag or contains an invalid field value.
pub fn deserialize_record<'de, T, D>(deserializer: D) -> Result<T, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    deserialize_tagged(deserializer, "Record")
}
