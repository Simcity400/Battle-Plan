//! The byte form prepared rows are stored in: MessagePack, structs as maps, so the serde shapes
//! replies use (skipped and defaulted fields) store as they are, read without text parsing.

use serde::{Serialize, de::DeserializeOwned};
use std::io;

/// `value` as a stored row, appended to `out`.
pub fn encode_into<T: Serialize + ?Sized>(out: &mut Vec<u8>, value: &T) -> io::Result<()> {
    value
        .serialize(&mut rmp_serde::Serializer::new(out).with_struct_map())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// `value` as a stored row.
pub fn encode<T: Serialize + ?Sized>(value: &T) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    encode_into(&mut out, value)?;
    Ok(out)
}

/// A stored row's value.
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> io::Result<T> {
    rmp_serde::from_slice(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}
