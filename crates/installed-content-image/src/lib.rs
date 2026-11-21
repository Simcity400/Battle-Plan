
pub mod bmp;
pub mod cursor;
pub mod dds;
pub mod native;
pub mod rgba;
pub mod tga;
mod bytes;
pub use bytes::{Bytes, WeakBytes};

use std::io;

pub(crate) fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[derive(Clone, Debug)]
pub struct Rgba8Image<P = Vec<u8>> {
    pub width: u32,
    pub height: u32,
    pub pixels: P,
}

pub type SharedRgba8Image = Rgba8Image<Bytes>;

impl<P: AsRef<[u8]>, Q: AsRef<[u8]>> PartialEq<Rgba8Image<Q>> for Rgba8Image<P> {
    fn eq(&self, other: &Rgba8Image<Q>) -> bool {
        self.width == other.width && self.height == other.height && self.pixels.as_ref() == other.pixels.as_ref()
    }
}

impl<P: AsRef<[u8]>> Eq for Rgba8Image<P> {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexedImage {
    pub width: u32,
    pub height: u32,
    pub indices: Bytes,
    pub palette: [[u8; 4]; 256],
}

impl IndexedImage {
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

pub(crate) fn reserved(count: usize, what: &str) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(count)
        .map_err(|_| invalid(format!("{what} exceeds available memory")))?;
    Ok(bytes)
}
