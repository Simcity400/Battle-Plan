
use crate::invalid;
use std::io;

#[cfg(test)]
mod tests;

pub trait Le: Copy + Default {
    const WIDTH: usize;
    fn put(self, out: &mut Vec<u8>);
    fn put_slice(values: &[Self], out: &mut Vec<u8>) {
        for &value in values {
            value.put(out);
        }
    }
    fn get(bytes: &[u8]) -> Self;
}

macro_rules! le {
    ($($kind:ty),+) => {
        $(impl Le for $kind {
            const WIDTH: usize = std::mem::size_of::<$kind>();
            fn put(self, out: &mut Vec<u8>) {
                out.extend_from_slice(&self.to_le_bytes());
            }
            fn put_slice(values: &[Self], out: &mut Vec<u8>) {
                #[cfg(target_endian = "little")]
                {
                    let bytes = values.len() * Self::WIDTH;
                    // SAFETY: primitive integer slices are contiguous initialized little-endian bytes.
                    let raw = unsafe {
                        std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), bytes)
                    };
                    out.extend_from_slice(raw);
                }
                #[cfg(not(target_endian = "little"))]
                for &value in values {
                    value.put(out);
                }
            }
            fn get(bytes: &[u8]) -> Self {
                Self::from_le_bytes(bytes.try_into().expect("width"))
            }
        })+
    };
}
le!(u8, i8, u16, i16, u32, i32, u64, i64);

pub trait Int: Copy + Default {
    fn wire(self) -> u64;
    fn unwire(wire: u64) -> io::Result<Self>;
}

macro_rules! unsigned_int {
    ($($kind:ty),+) => {
        $(impl Int for $kind {
            fn wire(self) -> u64 {
                self as u64
            }
            fn unwire(wire: u64) -> io::Result<Self> {
                <$kind>::try_from(wire).map_err(|_| invalid("Varint exceeds its column's width"))
            }
        })+
    };
}
macro_rules! signed_int {
    ($($kind:ty),+) => {
        $(impl Int for $kind {
            fn wire(self) -> u64 {
                let value = self as i64;
                ((value << 1) ^ (value >> 63)) as u64
            }
            fn unwire(wire: u64) -> io::Result<Self> {
                let value = ((wire >> 1) as i64) ^ -((wire & 1) as i64);
                <$kind>::try_from(value).map_err(|_| invalid("Varint exceeds its column's width"))
            }
        })+
    };
}
unsigned_int!(u16, u32, u64);
signed_int!(i16, i32, i64);

const SLOT_DELTAS: u8 = 0;
const SLOT_BITMAP: u8 = 1;
pub const SLOT_WORDS: usize = u32::MAX as usize / 64 + 1;
const POSITION_BITMAP: u8 = 0;
const POSITION_RUNS: u8 = 1;

pub(crate) fn varint_len(value: u64) -> usize {
    (64 - value.max(1).leading_zeros() as usize).div_ceil(7)
}

#[derive(Default)]
pub struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn reusing(mut bytes: Vec<u8>) -> Self {
        bytes.clear();
        Self { bytes }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    #[inline]
    fn reserve(&mut self, bytes: usize) -> io::Result<()> {
        if self.bytes.capacity() - self.bytes.len() < bytes {
            engine_allocation::reserve(&mut self.bytes, bytes)?;
        }
        Ok(())
    }

    #[inline]
    pub fn u8(&mut self, value: u8) -> io::Result<()> {
        self.reserve(1)?;
        self.bytes.push(value);
        Ok(())
    }

    #[inline]
    pub fn varint(&mut self, mut value: u64) -> io::Result<()> {
        if self.bytes.capacity() - self.bytes.len() < 10 {
            self.reserve(varint_len(value))?;
        }
        while value >= 0x80 {
            self.bytes.push(value as u8 | 0x80);
            value >>= 7;
        }
        self.bytes.push(value as u8);
        Ok(())
    }

    pub fn raw(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.reserve(bytes.len())?;
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }

    pub fn ints<T: Int>(&mut self, values: &[T]) -> io::Result<()> {
        self.reserve(values.len())?;
        for &value in values {
            self.varint(value.wire())?;
        }
        Ok(())
    }

    pub fn lengths(&mut self, ends: &[u32]) -> io::Result<()> {
        self.reserve(ends.len())?;
        let mut start = 0;
        for &end in ends {
            let length = end
                .checked_sub(start)
                .ok_or_else(|| invalid("Cell ends decrease"))?;
            self.varint(u64::from(length))?;
            start = end;
        }
        Ok(())
    }

    pub fn keys(&mut self, keys: &[u128]) -> io::Result<()> {
        self.reserve(keys.len())?;
        let mut previous = 0u128;
        for &key in keys {
            let difference = key.wrapping_sub(previous) as i128;
            let mut value = ((difference << 1) ^ (difference >> 127)) as u128;
            self.reserve(19)?;
            while value >= 0x80 {
                self.bytes.push(value as u8 | 0x80);
                value >>= 7;
            }
            self.bytes.push(value as u8);
            previous = key;
        }
        Ok(())
    }

    pub fn align(&mut self, width: usize) -> io::Result<()> {
        let padding = self.bytes.len().next_multiple_of(width) - self.bytes.len();
        self.reserve(padding)?;
        self.bytes.resize(self.bytes.len() + padding, 0);
        Ok(())
    }

    pub fn values<T: Le>(&mut self, values: &[T]) -> io::Result<()> {
        self.align(T::WIDTH)?;
        self.reserve(values.len() * T::WIDTH)?;
        T::put_slice(values, &mut self.bytes);
        Ok(())
    }

    pub fn slots(&mut self, slots: &[u32]) -> io::Result<()> {
        if slots.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(invalid("Slot list is not strictly ascending"));
        }
        let deltas: usize = slots
            .iter()
            .enumerate()
            .map(|(index, &slot)| match index {
                0 => varint_len(u64::from(slot)),
                _ => varint_len(u64::from(slot - slots[index - 1] - 1)),
            })
            .sum();
        let words = slots.last().map_or(0, |&last| last as usize / 64 + 1);
        self.varint(slots.len() as u64)?;
        if words * 8 + varint_len(words as u64) + 7 < deltas {
            self.u8(SLOT_BITMAP)?;
            self.varint(words as u64)?;
            self.bitmap(words, slots)
        } else {
            self.u8(SLOT_DELTAS)?;
            self.reserve(deltas)?;
            for (index, &slot) in slots.iter().enumerate() {
                self.varint(u64::from(match index {
                    0 => slot,
                    _ => slot - slots[index - 1] - 1,
                }))?;
            }
            Ok(())
        }
    }

    pub fn positions(&mut self, of: usize, positions: &[u32]) -> io::Result<()> {
        if positions.windows(2).any(|pair| pair[0] >= pair[1])
            || positions.last().is_some_and(|&last| last as usize >= of)
        {
            return Err(invalid("Positions are not ascending within their list"));
        }
        let (mut count, mut end, mut run_bytes) = (0u64, 0, 0);
        for (start, length) in runs(positions) {
            count += 1;
            run_bytes += varint_len(u64::from(start - end)) + varint_len(u64::from(length - 1));
            end = start + length;
        }
        run_bytes += varint_len(count);
        let words = of.div_ceil(64);
        if words * 8 + 7 < run_bytes {
            self.u8(POSITION_BITMAP)?;
            self.bitmap(words, positions)
        } else {
            self.u8(POSITION_RUNS)?;
            self.reserve(run_bytes)?;
            self.varint(count)?;
            let mut end = 0;
            for (start, length) in runs(positions) {
                self.varint(u64::from(start - end))?;
                self.varint(u64::from(length - 1))?;
                end = start + length;
            }
            Ok(())
        }
    }

    fn bitmap(&mut self, words: usize, bits: &[u32]) -> io::Result<()> {
        self.align(u64::WIDTH)?;
        let len = words * u64::WIDTH;
        self.reserve(len)?;
        let start = self.bytes.len();
        self.bytes.resize(start + len, 0);
        let out = &mut self.bytes[start..start + len];
        for &bit in bits {
            out[bit as usize / 8] |= 1 << (bit % 8);
        }
        Ok(())
    }
}

fn runs(positions: &[u32]) -> impl Iterator<Item = (u32, u32)> + '_ {
    let mut at = 0;
    std::iter::from_fn(move || {
        let &start = positions.get(at)?;
        let mut length = 1;
        while positions
            .get(at + length as usize)
            .is_some_and(|&next| next - start == length)
        {
            length += 1;
        }
        at += length as usize;
        Some((start, length))
    })
}

pub struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

fn short() -> io::Error {
    invalid("Section ends inside a value")
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    pub fn at(&self) -> usize {
        self.at
    }

    pub fn left(&self) -> usize {
        self.bytes.len() - self.at
    }

    pub fn rest(&self) -> &'a [u8] {
        &self.bytes[self.at..]
    }

    pub fn finish(&self) -> io::Result<()> {
        if self.at != self.bytes.len() {
            return Err(invalid(format!(
                "Section has {} bytes after its last value",
                self.bytes.len() - self.at
            )));
        }
        Ok(())
    }

    pub fn finish_padded(&self, width: usize) -> io::Result<()> {
        let rest = &self.bytes[self.at..];
        if rest.is_empty()
            || (self.bytes.len() == self.at.next_multiple_of(width)
                && rest.iter().all(|&byte| byte == 0))
        {
            return Ok(());
        }
        self.finish()
    }

    fn take(&mut self, count: usize) -> io::Result<&'a [u8]> {
        let end = self.at.checked_add(count).ok_or_else(short)?;
        let bytes = self.bytes.get(self.at..end).ok_or_else(short)?;
        self.at = end;
        Ok(bytes)
    }

    pub fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn raw(&mut self, length: usize) -> io::Result<&'a [u8]> {
        self.take(length)
    }

    fn reserve_for<T>(&self, into: &mut Vec<T>, count: usize) -> io::Result<()> {
        engine_allocation::reserve(into, count.min(self.bytes.len() - self.at))
    }

    pub fn ints<T: Int>(&mut self, count: usize, into: &mut Vec<T>) -> io::Result<()> {
        into.clear();
        self.reserve_for(into, count)?;
        for _ in 0..count {
            into.push(T::unwire(self.varint()?)?);
        }
        Ok(())
    }

    pub fn lengths(&mut self, count: usize, into: &mut Vec<u32>) -> io::Result<()> {
        into.clear();
        self.reserve_for(into, count)?;
        let mut end = 0u32;
        for _ in 0..count {
            end = end
                .checked_add(self.varint_u32()?)
                .ok_or_else(|| invalid("Cell lengths pass 4 GiB of items"))?;
            into.push(end);
        }
        Ok(())
    }

    pub fn keys(&mut self, count: usize, into: &mut Vec<u128>) -> io::Result<()> {
        into.clear();
        self.reserve_for(into, count)?;
        let mut previous = 0u128;
        for _ in 0..count {
            let mut value = 0u128;
            let mut shift = 0;
            loop {
                let byte = self.u8()?;
                let part = u128::from(byte & 0x7f);
                if shift == 126 && part > 3 || shift > 126 {
                    return Err(invalid("Key varint overflows 128 bits"));
                }
                value |= part << shift;
                if byte & 0x80 == 0 {
                    if byte == 0 && shift != 0 {
                        return Err(invalid("Key varint is not in its shortest form"));
                    }
                    break;
                }
                shift += 7;
            }
            let difference = ((value >> 1) as i128) ^ -((value & 1) as i128);
            previous = previous.wrapping_add(difference as u128);
            into.push(previous);
        }
        Ok(())
    }

    pub fn varint(&mut self) -> io::Result<u64> {
        if let Some(&byte) = self.bytes.get(self.at)
            && byte < 0x80
        {
            self.at += 1;
            return Ok(u64::from(byte));
        }
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = self.u8()?;
            let part = u64::from(byte & 0x7f);
            if shift == 63 && part > 1 {
                return Err(invalid("Varint overflows 64 bits"));
            }
            value |= part << shift;
            if byte & 0x80 == 0 {
                if byte == 0 && shift != 0 {
                    return Err(invalid("Varint is not in its shortest form"));
                }
                return Ok(value);
            }
        }
        Err(invalid("Varint overflows 64 bits"))
    }

    pub fn varint_u32(&mut self) -> io::Result<u32> {
        u32::try_from(self.varint()?).map_err(|_| invalid("Varint exceeds 32 bits"))
    }

    pub fn count(&mut self) -> io::Result<usize> {
        let count = self.varint()?;
        usize::try_from(count).map_err(|_| invalid("Count exceeds this platform"))
    }

    pub fn align(&mut self, width: usize) -> io::Result<()> {
        let padding = self.at.next_multiple_of(width) - self.at;
        if self.take(padding)?.iter().any(|&byte| byte != 0) {
            return Err(invalid("Alignment padding is not zero"));
        }
        Ok(())
    }

    pub fn values<T: Le>(&mut self, count: usize, into: &mut Vec<T>) -> io::Result<()> {
        self.align(T::WIDTH)?;
        let bytes = self.take(count.checked_mul(T::WIDTH).ok_or_else(short)?)?;
        into.clear();
        engine_allocation::reserve(into, count)?;
        into.extend(bytes.chunks_exact(T::WIDTH).map(T::get));
        Ok(())
    }

    fn words(&mut self, count: usize) -> io::Result<&'a [u8]> {
        self.align(u64::WIDTH)?;
        self.take(count.checked_mul(u64::WIDTH).ok_or_else(short)?)
    }

    pub fn slots(&mut self, into: &mut Vec<u32>) -> io::Result<()> {
        self.slots_below(u32::MAX as usize + 1, into)
    }

    pub fn slots_below(&mut self, end: usize, into: &mut Vec<u32>) -> io::Result<()> {
        let count = self.count()?;
        if count > end {
            return Err(invalid("Slot list holds more slots than it may"));
        }
        into.clear();
        match self.u8()? {
            SLOT_DELTAS => {
                engine_allocation::reserve(into, count.min(self.bytes.len() - self.at))?;
                let mut next = 0u64;
                for _ in 0..count {
                    let slot = next
                        .checked_add(self.varint()?)
                        .filter(|&slot| slot < end as u64)
                        .ok_or_else(|| invalid("Slot list passes the last slot"))?;
                    into.push(slot as u32);
                    next = slot + 1;
                }
            }
            SLOT_BITMAP => {
                let words = self.count()?;
                if words > SLOT_WORDS || words > end.div_ceil(64) {
                    return Err(invalid("Slot bitmap passes the last slot"));
                }
                let bytes = self.words(words)?;
                if bytes.rchunks_exact(u64::WIDTH).next().map(u64::get) == Some(0) {
                    return Err(invalid("Slot bitmap ends in an empty word"));
                }
                let bits = bytes.chunks_exact(u64::WIDTH).map(u64::get);
                let set: usize = bits.clone().map(|word| word.count_ones() as usize).sum();
                if set != count {
                    return Err(invalid("Slot bitmap disagrees with its count"));
                }
                engine_allocation::reserve(into, count)?;
                for (index, word) in bits.enumerate() {
                    let mut rest = word;
                    while rest != 0 {
                        let slot = index * 64 + rest.trailing_zeros() as usize;
                        if slot >= end {
                            return Err(invalid("Slot bitmap passes the last slot"));
                        }
                        into.push(slot as u32);
                        rest &= rest - 1;
                    }
                }
            }
            form => return Err(invalid(format!("Slot list form {form}"))),
        }
        Ok(())
    }

    pub fn positions(&mut self, of: usize, into: &mut Vec<u32>) -> io::Result<()> {
        into.clear();
        match self.u8()? {
            POSITION_BITMAP => {
                let bits = self.words(of.div_ceil(64))?.chunks_exact(u64::WIDTH);
                for (index, word) in bits.map(u64::get).enumerate() {
                    let mut rest = word;
                    while rest != 0 {
                        let position = index * 64 + rest.trailing_zeros() as usize;
                        if position >= of {
                            return Err(invalid("Position bitmap passes its list"));
                        }
                        engine_allocation::reserve(into, 1)?;
                        into.push(position as u32);
                        rest &= rest - 1;
                    }
                }
            }
            POSITION_RUNS => {
                let runs = self.count()?;
                let mut end = 0u64;
                for _ in 0..runs {
                    let start = end
                        .checked_add(self.varint()?)
                        .ok_or_else(|| invalid("Position run overflows"))?;
                    let length = self
                        .varint()?
                        .checked_add(1)
                        .ok_or_else(|| invalid("Position run overflows"))?;
                    end = start
                        .checked_add(length)
                        .filter(|&end| end <= of as u64)
                        .ok_or_else(|| invalid("Position run passes its list"))?;
                    engine_allocation::reserve(into, length as usize)?;
                    into.extend((start..end).map(|position| position as u32));
                }
            }
            form => return Err(invalid(format!("Position form {form}"))),
        }
        Ok(())
    }
}
