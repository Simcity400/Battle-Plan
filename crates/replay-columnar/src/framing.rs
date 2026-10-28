//! How a recording's hours are cut into compressed units, byte in and byte out; the
//! caller compresses and decompresses each unit.
//!
//! Stream 0 holds every hour's manifest; stream `1 + t` holds table `t`'s sections, in
//! hour order, for the hours whose manifests list one. A unit is one stream's bytes for
//! a run of whole hours. Every stream's units start at the same hour ordinals, so a
//! walker that starts at a restart starts every stream at a unit's first byte.
//!
//! Inside a manifest unit each manifest is its byte length (varint), then its bytes.
//! Inside a table unit each section starts at a multiple of 8 bytes from the unit's
//! start, zero-padded, so its value arrays stay aligned; its length is the manifest's.

use crate::{
    invalid,
    leaf::{Reader, Writer},
};
use std::io;

#[cfg(test)]
mod tests;

/// The stream holding every hour's manifest.
pub const MANIFESTS: u32 = 0;

/// The stream holding `table`'s sections.
pub fn table_stream(table: usize) -> u32 {
    1 + table as u32
}

/// Appends one hour's manifest to a manifest unit.
pub fn put_manifest(unit: &mut Writer, manifest: &[u8]) -> io::Result<()> {
    unit.varint(manifest.len() as u64)?;
    unit.raw(manifest)
}

/// The next manifest of a manifest unit; None at its end.
pub fn next_manifest<'a>(unit: &mut Reader<'a>, length: usize) -> io::Result<Option<&'a [u8]>> {
    if unit.at() == length {
        return Ok(None);
    }
    let bytes = unit.count()?;
    unit.raw(bytes).map(Some)
}

/// Appends one hour's section to a table unit.
pub fn put_section(unit: &mut Writer, section: &[u8]) -> io::Result<()> {
    unit.align(8)?;
    unit.raw(section)
}

/// The next section of a table unit, `length` bytes as its hour's manifest says.
pub fn next_section<'a>(unit: &mut Reader<'a>, length: usize) -> io::Result<&'a [u8]> {
    unit.align(8)?;
    unit.raw(length)
}

/// Where one compressed unit is: its stream, the ordinal of its first hour and how many
/// hours it covers, and its bytes in the file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Unit {
    pub stream: u32,
    pub first: u32,
    pub hours: u32,
    pub offset: u64,
    pub raw: u64,
    pub stored: u64,
}

impl Unit {
    pub(crate) fn write(&self, writer: &mut Writer) -> io::Result<()> {
        writer.varint(u64::from(self.stream))?;
        writer.varint(u64::from(self.first))?;
        writer.varint(u64::from(self.hours))?;
        writer.varint(self.offset)?;
        writer.varint(self.raw)?;
        writer.varint(self.stored)
    }

    pub(crate) fn read(reader: &mut Reader<'_>) -> io::Result<Self> {
        let unit = Self {
            stream: reader.varint_u32()?,
            first: reader.varint_u32()?,
            hours: reader.varint_u32()?,
            offset: reader.varint()?,
            raw: reader.varint()?,
            stored: reader.varint()?,
        };
        if unit.hours == 0 {
            return Err(invalid("A unit covers no hours"));
        }
        Ok(unit)
    }
}
