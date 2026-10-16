//! Bounded explicit little-endian decoding. No Rust memory layout is on disk.

use std::io;

/// Diagnostic excerpts never copy a recording-sized string. Data itself is
/// preserved; only the diagnostic shown to a caller is shortened.
pub fn diagnostic_text(text: &str) -> &str {
    let mut end = text.len().min(1024);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Formatting with checked growth, including errors whose fields are recorded.
pub fn try_format(args: std::fmt::Arguments<'_>) -> io::Result<String> {
    struct Writer(String);
    impl std::fmt::Write for Writer {
        fn write_str(&mut self, text: &str) -> std::fmt::Result {
            engine::allocation::reserve_string(&mut self.0, text.len())
                .map_err(|_| std::fmt::Error)?;
            self.0.push_str(text);
            Ok(())
        }
    }
    let mut writer = Writer(String::new());
    std::fmt::write(&mut writer, args).map_err(|_| engine::allocation::oom())?;
    Ok(writer.0)
}

pub fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// Collector error paths cannot allocate even for a static diagnostic.
pub fn invalid_static(message: &'static str) -> io::Error {
    if engine::allocation::capture_active() {
        io::Error::from(io::ErrorKind::InvalidData)
    } else {
        invalid(message)
    }
}

pub fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) -> io::Result<()> {
    let length =
        u32::try_from(bytes.len()).map_err(|_| invalid_static("byte field exceeds u32"))?;
    let additional = bytes
        .len()
        .checked_add(4)
        .ok_or_else(engine::allocation::oom)?;
    engine::allocation::reserve(out, additional)?;
    out.extend_from_slice(&length.to_le_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}

pub fn put_string(out: &mut Vec<u8>, value: &str) -> io::Result<()> {
    put_bytes(out, value.as_bytes())
}

/// Canonical unsigned LEB128, including the 65 bits needed by native numeric differences.
pub fn put_varint(out: &mut Vec<u8>, mut value: u128) {
    while value >= 0x80 {
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

pub struct Decoder<'a> {
    bytes: &'a [u8],
    pub format: u16,
    pub recorded_fields: usize,
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
        Self {
            bytes,
            format: crate::FORMAT_VERSION,
            recorded_fields: usize::MAX,
        }
    }
    pub fn remaining(&self) -> usize {
        self.bytes.len()
    }
    /// The unread bytes, for callers that report the exact span a read consumed.
    pub fn rest(&self) -> &'a [u8] {
        self.bytes
    }
    pub fn take(&mut self, length: usize) -> io::Result<&'a [u8]> {
        if length > self.bytes.len() {
            return Err(invalid_static("truncated field"));
        }
        let (head, tail) = self.bytes.split_at(length);
        self.bytes = tail;
        Ok(head)
    }
    pub fn finish(self) -> io::Result<()> {
        if self.bytes.is_empty() {
            Ok(())
        } else {
            Err(invalid_static("trailing bytes"))
        }
    }
    number!(u8, u8);
    number!(u16, u16);
    number!(u32, u32);
    number!(u64, u64);
    number!(i32, i32);
    number!(i64, i64);
    /// Decode one canonical unsigned LEB128 value without consuming the next field.
    pub fn varint(&mut self) -> io::Result<u128> {
        let mut value = 0_u128;
        for index in 0..19 {
            let byte = self.u8()?;
            let payload = byte & 0x7f;
            if index == 18 && (payload > 3 || byte & 0x80 != 0) {
                return Err(invalid_static("varint exceeds u128"));
            }
            value |= u128::from(payload) << (index * 7);
            if byte & 0x80 == 0 {
                if index != 0 && payload == 0 {
                    return Err(invalid_static("noncanonical overlong varint"));
                }
                return Ok(value);
            }
        }
        Err(invalid_static("varint exceeds u128"))
    }

    pub fn bool(&mut self) -> io::Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(invalid_static("invalid boolean")),
        }
    }
    /// Bound collection allocation by the bytes actually present before allocating.
    pub fn count(&mut self, minimum_item_bytes: usize) -> io::Result<usize> {
        let n = self.u32()? as usize;
        if minimum_item_bytes == 0 || n > self.remaining() / minimum_item_bytes {
            return Err(invalid_static(
                "collection length exceeds remaining payload",
            ));
        }
        Ok(n)
    }
    pub fn bytes(&mut self) -> io::Result<Vec<u8>> {
        let n = self.u32()? as usize;
        engine::allocation::copy_vec(self.take(n)?)
    }
    pub fn string(&mut self) -> io::Result<String> {
        String::from_utf8(self.bytes()?).map_err(|_| invalid_static("invalid UTF-8 string"))
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

    #[test]
    fn varints_preserve_full_width_values_with_canonical_bytes() {
        let mut u64_max_bytes = vec![0xff; 9];
        u64_max_bytes.push(1);
        let mut difference_max_bytes = vec![0xff; 9];
        difference_max_bytes.push(3);
        let mut u128_max_bytes = vec![0xff; 18];
        u128_max_bytes.push(3);
        for (value, expected) in [
            (0, vec![0]),
            (127, vec![127]),
            (128, vec![0x80, 1]),
            (u128::from(u64::MAX), u64_max_bytes),
            ((1_u128 << 65) - 1, difference_max_bytes),
            (u128::MAX, u128_max_bytes),
        ] {
            let mut encoded = Vec::new();
            put_varint(&mut encoded, value);
            assert_eq!(encoded, expected);
            encoded.push(0xff);
            let mut decoder = Decoder::new(&encoded);
            assert_eq!(decoder.varint().unwrap(), value);
            assert_eq!(decoder.u8().unwrap(), 0xff);
            decoder.finish().unwrap();
        }
    }

    #[test]
    fn varints_refuse_truncation_overflow_and_redundant_groups() {
        let mut overflow = vec![0xff; 18];
        overflow.push(4);
        let mut continued_final_group = vec![0xff; 18];
        continued_final_group.push(0x83);
        let mut overlong_zero = vec![0x80; 18];
        overlong_zero.push(0);
        for malformed in [
            Vec::new(),
            vec![0x80],
            vec![0xff; 9],
            vec![0x80, 0],
            vec![0x81, 0],
            overflow,
            continued_final_group,
            vec![0x80; 19],
            overlong_zero,
        ] {
            assert_eq!(
                Decoder::new(&malformed).varint().unwrap_err().kind(),
                io::ErrorKind::InvalidData,
                "accepted malformed varint {malformed:02x?}",
            );
        }
    }
}
