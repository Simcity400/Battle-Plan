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
//! start, zero-padded, so its value arrays stay aligned; its length is the manifest's. A
//! table unit may end with the padding to a multiple of 8, as a unit piece cut to be
//! continued does.

use crate::leaf::{Reader, Writer};
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

/// Refuses a table unit's bytes after its last section but the padding it may end with.
pub fn finish_sections(unit: &Reader) -> io::Result<()> {
    unit.finish_padded(8)
}
