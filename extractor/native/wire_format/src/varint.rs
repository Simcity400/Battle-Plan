//! LEB128 varints and zigzag, per `WIRE_FORMAT.md` §0.
//!
//! Shortest form is **required**, not merely produced: a decoder that accepts a
//! redundant trailing zero group would let two writers emit different bytes for
//! the same value, which is exactly what the dedup floor rests on not happening.

/// Longest legal encoding of a `u64`: ten groups of seven bits.
pub const MAX_VARINT_LEN: usize = 10;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WireError {
    /// Ran out of bytes mid-value.
    Truncated,
    /// An eleventh continuation byte, or a value that cannot fit `u64`.
    Overlong,
    /// Decodable, but not the shortest encoding of its value.
    NotShortest,
    /// A field, bitmap or frame that does not satisfy a stated invariant.
    Corrupt(&'static str),
}

pub type Result<T> = core::result::Result<T, WireError>;

pub fn write_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// Decode at `pos`, returning the value and the new position.
pub fn read_varint(buf: &[u8], pos: usize) -> Result<(u64, usize)> {
    let mut value: u64 = 0;
    let mut shift = 0u32;
    let mut i = pos;
    loop {
        if i >= buf.len() {
            return Err(WireError::Truncated);
        }
        if i - pos >= MAX_VARINT_LEN {
            return Err(WireError::Overlong);
        }
        let byte = buf[i];
        i += 1;
        let payload = (byte & 0x7f) as u64;
        // The tenth group carries only the single remaining bit of a u64.
        if shift == 63 && payload > 1 {
            return Err(WireError::Overlong);
        }
        value |= payload << shift;
        if byte & 0x80 == 0 {
            // Shortest form: a final group of zero is only legal as the sole group.
            if payload == 0 && i - pos > 1 {
                return Err(WireError::NotShortest);
            }
            return Ok((value, i));
        }
        shift += 7;
    }
}

pub fn zigzag_encode(value: i64) -> u64 {
    ((value << 1) ^ (value >> 63)) as u64
}

pub fn zigzag_decode(value: u64) -> i64 {
    ((value >> 1) as i64) ^ -((value & 1) as i64)
}

pub fn write_zigzag(out: &mut Vec<u8>, value: i64) {
    write_varint(out, zigzag_encode(value));
}

pub fn read_zigzag(buf: &[u8], pos: usize) -> Result<(i64, usize)> {
    let (raw, next) = read_varint(buf, pos)?;
    Ok((zigzag_decode(raw), next))
}

/// Compare two canonical encoded payloads: unsigned, bytewise, shorter-is-smaller
/// on a prefix. §8 pins this for gap-cause resolution because the answer differs
/// from numeric order — counter ids 255 and 256 encode as `FF 01` and `80 02`.
pub fn compare_canonical_bytes(a: &[u8], b: &[u8]) -> core::cmp::Ordering {
    a.cmp(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(value: u64) {
        let mut out = Vec::new();
        write_varint(&mut out, value);
        let (got, pos) = read_varint(&out, 0).expect("decodes");
        assert_eq!(got, value, "value roundtrips");
        assert_eq!(pos, out.len(), "consumes exactly its own bytes");
    }

    #[test]
    fn varint_roundtrips_across_group_boundaries() {
        for value in [0u64, 1, 127, 128, 255, 256, 16383, 16384, u32::MAX as u64, u64::MAX] {
            roundtrip(value);
        }
    }

    #[test]
    fn one_byte_holds_up_to_127() {
        let mut out = Vec::new();
        write_varint(&mut out, 127);
        assert_eq!(out.len(), 1);
        out.clear();
        write_varint(&mut out, 128);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn the_255_256_case_that_pins_the_comparator() {
        // §8 cites these exact encodings: numeric order and byte order disagree.
        let mut a = Vec::new();
        write_varint(&mut a, 255);
        let mut b = Vec::new();
        write_varint(&mut b, 256);
        assert_eq!(a, vec![0xFF, 0x01]);
        assert_eq!(b, vec![0x80, 0x02]);
        assert!(255u64 < 256u64, "numerically 255 is smaller");
        assert_eq!(
            compare_canonical_bytes(&a, &b),
            core::cmp::Ordering::Greater,
            "but bytewise 255 sorts AFTER 256, which is why the representation is pinned"
        );
    }

    #[test]
    fn non_shortest_encoding_is_rejected() {
        // 0 written as two groups: legal LEB128, illegal here.
        assert_eq!(read_varint(&[0x80, 0x00], 0), Err(WireError::NotShortest));
        // 1 written as two groups.
        assert_eq!(read_varint(&[0x81, 0x00], 0), Err(WireError::NotShortest));
        // A single zero group is the shortest encoding of 0 and stays legal.
        assert_eq!(read_varint(&[0x00], 0), Ok((0, 1)));
    }

    #[test]
    fn truncated_and_overlong_are_distinguished() {
        assert_eq!(read_varint(&[0x80], 0), Err(WireError::Truncated));
        let eleven = [0x80u8; 11];
        assert_eq!(read_varint(&eleven, 0), Err(WireError::Overlong));
    }

    #[test]
    fn zigzag_keeps_small_negatives_one_byte() {
        for value in [0i64, -1, 1, -64, 63] {
            let mut out = Vec::new();
            write_zigzag(&mut out, value);
            assert_eq!(out.len(), 1, "{value} fits one byte");
            assert_eq!(read_zigzag(&out, 0).unwrap().0, value);
        }
    }

    #[test]
    fn zigzag_roundtrips_at_the_extremes() {
        for value in [i64::MIN, i64::MAX, -1, 0] {
            assert_eq!(zigzag_decode(zigzag_encode(value)), value);
        }
    }
}
