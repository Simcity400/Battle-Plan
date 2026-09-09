//! Bounded explicit little-endian decoding. No Rust memory layout is on disk.

use std::io;

pub fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

pub fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) -> io::Result<()> {
    let length = u32::try_from(bytes.len()).map_err(|_| invalid("byte field exceeds u32"))?;
    out.extend_from_slice(&length.to_le_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}

pub fn put_string(out: &mut Vec<u8>, value: &str) -> io::Result<()> {
    put_bytes(out, value.as_bytes())
}

pub struct Decoder<'a> {
    bytes: &'a [u8],
}

macro_rules! number {
    ($name:ident, $ty:ty) => {
        pub fn $name(&mut self) -> io::Result<$ty> {
            Ok(<$ty>::from_le_bytes(
                self.take(size_of::<$ty>())?.try_into().unwrap(),
            ))
        }
    };
}

impl<'a> Decoder<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }
    pub fn remaining(&self) -> usize {
        self.bytes.len()
    }
    pub fn take(&mut self, length: usize) -> io::Result<&'a [u8]> {
        if length > self.bytes.len() {
            return Err(invalid("truncated field"));
        }
        let (head, tail) = self.bytes.split_at(length);
        self.bytes = tail;
        Ok(head)
    }
    pub fn finish(self) -> io::Result<()> {
        if self.bytes.is_empty() {
            Ok(())
        } else {
            Err(invalid("trailing bytes"))
        }
    }
    number!(u8, u8);
    number!(u16, u16);
    number!(u32, u32);
    number!(u64, u64);
    number!(i32, i32);
    number!(i64, i64);
    pub fn bool(&mut self) -> io::Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(invalid("invalid boolean")),
        }
    }
    /// Bound collection allocation by the bytes actually present before allocating.
    pub fn count(&mut self, minimum_item_bytes: usize) -> io::Result<usize> {
        let n = self.u32()? as usize;
        if minimum_item_bytes == 0 || n > self.remaining() / minimum_item_bytes {
            return Err(invalid("collection length exceeds remaining payload"));
        }
        Ok(n)
    }
    pub fn bytes(&mut self) -> io::Result<Vec<u8>> {
        let n = self.u32()? as usize;
        Ok(self.take(n)?.to_vec())
    }
    pub fn string(&mut self) -> io::Result<String> {
        String::from_utf8(self.bytes()?).map_err(|_| invalid("invalid UTF-8 string"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hostile_lengths_and_tags_fail_before_allocation() {
        assert!(Decoder::new(&u32::MAX.to_le_bytes()).bytes().is_err());
        assert!(Decoder::new(&u32::MAX.to_le_bytes()).count(1).is_err());
        assert!(Decoder::new(&[2]).bool().is_err());
        assert!(Decoder::new(&[0]).finish().is_err());
    }
}
