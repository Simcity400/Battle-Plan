//! The records a lens hands the emit ([`super::lens`]), MessagePack as [`codec`] writes them,
//! packed exactly: a map whose keys are all strings as its shape, its header and keys, which a
//! lens names once, then its values; and a 16-byte binary, an i128 as rmp writes one, as its
//! integer. Each stands after 0xc1, a byte MessagePack never holds: then a varint, 0 for an
//! integer, which a zigzag varint follows, or a shape's number past 1.
//!
//! [`codec`]: dashboard_base::prepared::codec

use dashboard_series::hash::FastMap;
use std::io;

#[cfg(test)]
mod tests;

const MARK: u8 = 0xc1;
/// The length a 16-byte binary's head gives.
const BIN16: [u8; 2] = [0xc4, 16];

fn damaged() -> io::Error {
    io::Error::other("A politics lens record is not MessagePack")
}

/// What a value's head is: a map's or array's entries, or the whole length of a value without
/// any, and whether it is a 16-byte binary.
enum Head {
    Map(usize),
    Array(usize),
    Leaf(bool),
}

/// The head of the value at `bytes[at..]`, and its length.
fn head(bytes: &[u8], at: usize) -> io::Result<(Head, usize)> {
    let byte = *bytes.get(at).ok_or_else(damaged)?;
    let size = |width: usize| -> io::Result<usize> {
        let field = bytes.get(at + 1..at + 1 + width).ok_or_else(damaged)?;
        Ok(field
            .iter()
            .fold(0, |value, &byte| value << 8 | usize::from(byte)))
    };
    let leaf = |length: usize| (Head::Leaf(false), length);
    Ok(match byte {
        0x00..=0x7f | 0xc0 | 0xc2 | 0xc3 | 0xe0..=0xff => leaf(1),
        0x80..=0x8f => (Head::Map(usize::from(byte & 0x0f)), 1),
        0x90..=0x9f => (Head::Array(usize::from(byte & 0x0f)), 1),
        0xa0..=0xbf => leaf(1 + usize::from(byte & 0x1f)),
        0xc4 => {
            let length = size(1)?;
            (Head::Leaf(length == 16), 2 + length)
        }
        0xc5 | 0xda => leaf(3 + size(2)?),
        0xc6 | 0xdb => leaf(5 + size(4)?),
        0xc7 => leaf(3 + size(1)?),
        0xc8 => leaf(4 + size(2)?),
        0xc9 => leaf(6 + size(4)?),
        0xd9 => leaf(2 + size(1)?),
        0xca | 0xce | 0xd2 => leaf(5),
        0xcb | 0xcf | 0xd3 => leaf(9),
        0xcc | 0xd0 => leaf(2),
        0xcd | 0xd1 => leaf(3),
        0xd4 => leaf(3),
        0xd5 => leaf(4),
        0xd6 => leaf(6),
        0xd7 => leaf(10),
        0xd8 => leaf(18),
        0xdc => (Head::Array(size(2)?), 3),
        0xdd => (Head::Array(size(4)?), 5),
        0xde => (Head::Map(size(2)?), 3),
        0xdf => (Head::Map(size(4)?), 5),
        0xc1 => return Err(damaged()),
    })
}

fn is_string(byte: u8) -> bool {
    matches!(byte, 0xa0..=0xbf | 0xd9..=0xdb)
}

/// Where the value at `bytes[at..]`, its head `length` long, ends.
fn end(bytes: &[u8], at: usize, length: usize) -> io::Result<usize> {
    at.checked_add(length)
        .filter(|&end| end <= bytes.len())
        .ok_or_else(damaged)
}

/// Where the value at `bytes[at..]` ends, past every value it holds.
fn skip(bytes: &[u8], at: usize) -> io::Result<usize> {
    let (head, length) = head(bytes, at)?;
    let mut at = end(bytes, at, length)?;
    let values = match head {
        Head::Leaf(_) => 0,
        Head::Array(entries) => entries,
        Head::Map(entries) => entries.checked_mul(2).ok_or_else(damaged)?,
    };
    for _ in 0..values {
        at = skip(bytes, at)?;
    }
    Ok(at)
}

fn put_varint(out: &mut Vec<u8>, mut value: u128) {
    while value >= 0x80 {
        out.push(value as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn varint(bytes: &[u8], at: &mut usize) -> io::Result<u128> {
    let mut value = 0_u128;
    for shift in (0..128).step_by(7) {
        let byte = *bytes.get(*at).ok_or_else(damaged)?;
        *at += 1;
        value |= u128::from(byte & 0x7f) << shift;
        if byte < 0x80 {
            return Ok(value);
        }
    }
    Err(damaged())
}

/// The shapes a lens names, by their bytes: a map's head, then its keys.
#[derive(Default)]
pub(super) struct Shapes {
    numbers: FastMap<Vec<u8>, u64>,
    /// Those named since last taken, in their numbers' order.
    named: Vec<Vec<u8>>,
    scratch: Vec<u8>,
    starts: Vec<usize>,
}

impl Shapes {
    /// `bytes`, one MessagePack value, packed onto `out`.
    pub(super) fn pack(&mut self, bytes: &[u8], out: &mut Vec<u8>) -> io::Result<()> {
        if self.value(bytes, 0, out)? != bytes.len() {
            return Err(damaged());
        }
        Ok(())
    }

    /// The shapes named since last asked.
    pub(super) fn take(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.named)
    }

    /// The value at `bytes[at..]`, packed onto `out`; where it ends.
    fn value(&mut self, bytes: &[u8], at: usize, out: &mut Vec<u8>) -> io::Result<usize> {
        let (head, length) = head(bytes, at)?;
        let mut after = end(bytes, at, length)?;
        match head {
            Head::Leaf(true) => {
                let value = i128::from_be_bytes(
                    bytes[at + 2..after].try_into().map_err(|_| damaged())?,
                );
                out.push(MARK);
                out.push(0);
                put_varint(out, ((value << 1) ^ (value >> 127)) as u128);
            }
            Head::Leaf(false) => out.extend_from_slice(&bytes[at..after]),
            Head::Array(entries) => {
                out.extend_from_slice(&bytes[at..after]);
                for _ in 0..entries {
                    after = self.value(bytes, after, out)?;
                }
            }
            Head::Map(entries) => {
                // The keys, and where each value starts.
                let first = self.starts.len();
                self.scratch.clear();
                self.scratch.extend_from_slice(&bytes[at..after]);
                let mut keyed = entries > 0;
                let mut cursor = after;
                for _ in 0..entries {
                    let key = cursor;
                    cursor = skip(bytes, key)?;
                    keyed &= is_string(bytes[key]);
                    self.scratch.extend_from_slice(&bytes[key..cursor]);
                    self.starts.push(cursor);
                    cursor = skip(bytes, cursor)?;
                }
                if keyed {
                    let number = match self.numbers.get(&self.scratch) {
                        Some(&number) => number,
                        None => {
                            let number = self.numbers.len() as u64;
                            self.numbers.insert(self.scratch.clone(), number);
                            self.named.push(self.scratch.clone());
                            number
                        }
                    };
                    out.push(MARK);
                    put_varint(out, u128::from(number) + 1);
                    for index in first..first + entries {
                        let start = self.starts[index];
                        self.value(bytes, start, out)?;
                    }
                } else {
                    out.extend_from_slice(&bytes[at..after]);
                    for _ in 0..entries {
                        after = self.value(bytes, after, out)?;
                        after = self.value(bytes, after, out)?;
                    }
                }
                self.starts.truncate(first);
                after = cursor;
            }
        }
        Ok(after)
    }
}

/// A shape as the emit holds it: its map's head and keys, and where each key ends.
struct Shape {
    bytes: Vec<u8>,
    head: usize,
    keys: Vec<usize>,
}

/// The shapes a lens named, as the emit holds them.
#[derive(Default)]
pub(super) struct Named(Vec<Shape>);

impl Named {
    pub(super) fn clear(&mut self) {
        self.0.clear();
    }

    /// Takes up `shapes`, as [`Shapes::take`] gave them.
    pub(super) fn extend(&mut self, shapes: Vec<Vec<u8>>) -> io::Result<()> {
        for bytes in shapes {
            let (Head::Map(entries), head) = head(&bytes, 0)? else {
                return Err(damaged());
            };
            let mut keys = Vec::with_capacity(entries);
            let mut at = head;
            for _ in 0..entries {
                if !bytes.get(at).copied().is_some_and(is_string) {
                    return Err(damaged());
                }
                at = skip(&bytes, at)?;
                keys.push(at);
            }
            if at != bytes.len() {
                return Err(damaged());
            }
            self.0.push(Shape { bytes, head, keys });
        }
        Ok(())
    }

    /// The record `packed` holds, as [`Shapes::pack`] packed it, into `out`.
    pub(super) fn unpack(&self, packed: &[u8], out: &mut Vec<u8>) -> io::Result<()> {
        out.clear();
        if self.value(packed, 0, out)? != packed.len() {
            return Err(damaged());
        }
        Ok(())
    }

    fn value(&self, packed: &[u8], at: usize, out: &mut Vec<u8>) -> io::Result<usize> {
        if packed.get(at) == Some(&MARK) {
            let mut at = at + 1;
            let code = varint(packed, &mut at)?;
            if code == 0 {
                let zigzag = varint(packed, &mut at)?;
                let value = (zigzag >> 1) as i128 ^ -((zigzag & 1) as i128);
                out.extend_from_slice(&BIN16);
                out.extend_from_slice(&value.to_be_bytes());
                return Ok(at);
            }
            let shape = usize::try_from(code - 1)
                .ok()
                .and_then(|number| self.0.get(number))
                .ok_or_else(damaged)?;
            out.extend_from_slice(&shape.bytes[..shape.head]);
            let mut from = shape.head;
            for &key in &shape.keys {
                out.extend_from_slice(&shape.bytes[from..key]);
                from = key;
                at = self.value(packed, at, out)?;
            }
            return Ok(at);
        }
        let (head, length) = head(packed, at)?;
        let mut after = end(packed, at, length)?;
        out.extend_from_slice(&packed[at..after]);
        let values = match head {
            Head::Leaf(_) => 0,
            Head::Array(entries) => entries,
            Head::Map(entries) => entries.checked_mul(2).ok_or_else(damaged)?,
        };
        for _ in 0..values {
            after = self.value(packed, after, out)?;
        }
        Ok(after)
    }
}
