//! Decoders for the game's authored image containers.
//!
//! Block-compressed DDS payloads stay compressed so the viewer can upload them
//! to the GPU unchanged. Every other format is converted to 8-bit RGBA with the
//! first row at the top left, because the storage order of a BMP or a TGA is a
//! container detail and never a map coordinate.

pub mod bmp;
pub mod cursor;
pub mod dds;
pub mod native;
pub mod rgba;
pub mod tga;
mod bytes;
pub use bytes::{Bytes, WeakBytes};

use std::io;

/// Names the file and the reason instead of guessing at unsupported bytes.
pub(crate) fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// Straight 8-bit RGBA pixels, row-major from the top-left corner.
#[derive(Clone, Debug)]
pub struct Rgba8Image<P = Vec<u8>> {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes in R, G, B, A order.
    pub pixels: P,
}

pub type SharedRgba8Image = Rgba8Image<Bytes>;

impl<P: AsRef<[u8]>, Q: AsRef<[u8]>> PartialEq<Rgba8Image<Q>> for Rgba8Image<P> {
    fn eq(&self, other: &Rgba8Image<Q>) -> bool {
        self.width == other.width && self.height == other.height && self.pixels.as_ref() == other.pixels.as_ref()
    }
}

impl<P: AsRef<[u8]>> Eq for Rgba8Image<P> {}

/// Palette-indexed pixels, row-major from the top-left corner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexedImage {
    pub width: u32,
    pub height: u32,
    /// `width * height` palette indices.
    pub indices: Bytes,
    /// RGBA entries; entries the file does not declare are all zero.
    pub palette: [[u8; 4]; 256],
}

impl IndexedImage {
    /// Expands the indices through the palette for inspection and comparison.
    #[cfg(test)]
    pub fn to_rgba8(&self) -> Rgba8Image {
        let mut pixels = Vec::with_capacity(self.indices.len() * 4);
        for &index in &self.indices {
            pixels.extend_from_slice(&self.palette[index as usize]);
        }
        Rgba8Image {
            width: self.width,
            height: self.height,
            pixels,
        }
    }
}

/// Reserves without aborting on a hostile or corrupt dimension pair.
pub(crate) fn reserved(count: usize, what: &str) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(count)
        .map_err(|_| invalid(format!("{what} exceeds available memory")))?;
    Ok(bytes)
}
