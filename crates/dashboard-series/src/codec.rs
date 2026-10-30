use crate::invalid;
use std::io;

pub struct Encode<'a> {
    bytes: &'a mut [u8],
    at: usize,
}
impl<'a> Encode<'a> {
    pub fn new(bytes: &'a mut [u8]) -> Self {
        Self { bytes, at: 0 }
    }
    pub fn len(&self) -> usize {
        self.at
    }
    pub fn bytes(&mut self, value: &[u8]) -> io::Result<()> {
        let end = self
            .at
            .checked_add(value.len())
            .ok_or_else(|| invalid("column length overflow"))?;
        self.bytes
            .get_mut(self.at..end)
            .ok_or_else(|| invalid("column exceeds page"))?
            .copy_from_slice(value);
        self.at = end;
        Ok(())
    }
    pub fn u8(&mut self, v: u8) -> io::Result<()> {
        self.bytes(&[v])
    }
    pub fn u16(&mut self, v: u16) -> io::Result<()> {
        self.bytes(&v.to_le_bytes())
    }
    pub fn u32(&mut self, v: u32) -> io::Result<()> {
        self.bytes(&v.to_le_bytes())
    }
    pub fn u64(&mut self, v: u64) -> io::Result<()> {
        self.bytes(&v.to_le_bytes())
    }
    pub fn i32(&mut self, v: i32) -> io::Result<()> {
        self.bytes(&v.to_le_bytes())
    }
    pub fn i128(&mut self, v: i128) -> io::Result<()> {
        self.bytes(&v.to_le_bytes())
    }
    pub fn f64(&mut self, v: f64) -> io::Result<()> {
        self.u64(v.to_bits())
    }
}

pub struct Decode<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl<'a> Decode<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }
    pub fn at(bytes: &'a [u8], at: usize) -> Self {
        Self { bytes, at }
    }
    pub fn position(&self) -> usize {
        self.at
    }
    pub fn bytes(&mut self, len: usize) -> io::Result<&'a [u8]> {
        let end = self
            .at
            .checked_add(len)
            .ok_or_else(|| invalid("column length overflow"))?;
        let value = self
            .bytes
            .get(self.at..end)
            .ok_or_else(|| invalid("truncated series column"))?;
        self.at = end;
        Ok(value)
    }
    pub fn array<const N: usize>(&mut self) -> io::Result<[u8; N]> {
        self.bytes(N)?
            .try_into()
            .map_err(|_| invalid("series field width"))
    }
    pub fn u8(&mut self) -> io::Result<u8> {
        Ok(self.array::<1>()?[0])
    }
    pub fn u16(&mut self) -> io::Result<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }
    pub fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    pub fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }
    pub fn i32(&mut self) -> io::Result<i32> {
        Ok(i32::from_le_bytes(self.array()?))
    }
    pub fn i128(&mut self) -> io::Result<i128> {
        Ok(i128::from_le_bytes(self.array()?))
    }
    pub fn f64(&mut self) -> io::Result<f64> {
        let v = f64::from_bits(self.u64()?);
        if !v.is_finite() {
            return Err(invalid("nonfinite series number"));
        }
        Ok(v)
    }
    pub fn flag(&mut self) -> io::Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(invalid("invalid series flag")),
        }
    }
    pub fn var(&mut self) -> io::Result<u64> {
        let value = self.var128()?;
        u64::try_from(value).map_err(|_| invalid("series varint width"))
    }
    pub fn var128(&mut self) -> io::Result<u128> {
        let mut value = 0_u128;
        for shift in (0..128).step_by(7) {
            let byte = self.u8()?;
            let part = u128::from(byte & 0x7f);
            if shift == 126 && part > 3 {
                return Err(invalid("series varint overflow"));
            }
            value |= part << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(invalid("series varint overflow"))
    }
    pub fn zig(&mut self) -> io::Result<i64> {
        let value = self.var()?;
        Ok((value >> 1) as i64 ^ -((value & 1) as i64))
    }
    pub fn zig128(&mut self) -> io::Result<i128> {
        let value = self.var128()?;
        Ok((value >> 1) as i128 ^ -((value & 1) as i128))
    }
    pub fn end(&self) -> io::Result<()> {
        if self.at == self.bytes.len() {
            Ok(())
        } else {
            Err(invalid("trailing series column bytes"))
        }
    }
}

pub trait Binary: Sized {
    const SIZE: usize;
    fn encode(&self, into: &mut Encode<'_>) -> io::Result<()>;
    fn decode(from: &mut Decode<'_>) -> io::Result<Self>;
}

/// A growable byte column whose growth fails instead of aborting.
#[derive(Default)]
pub(crate) struct Sink(Vec<u8>);
impl Sink {
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }
    pub fn clear(&mut self) {
        self.0.clear();
    }
    pub fn put(&mut self, value: &[u8]) -> io::Result<()> {
        engine_allocation::reserve(&mut self.0, value.len())?;
        self.0.extend_from_slice(value);
        Ok(())
    }
    pub fn record<T: Binary>(&mut self, value: &T) -> io::Result<()> {
        let at = self.0.len();
        let end = engine_allocation::add(at, T::SIZE)?;
        engine_allocation::resize(&mut self.0, end, 0)?;
        let mut e = Encode::new(&mut self.0[at..end]);
        value.encode(&mut e)?;
        if e.len() != T::SIZE {
            return Err(invalid("series record codec width"));
        }
        Ok(())
    }
    pub fn u8(&mut self, v: u8) -> io::Result<()> {
        self.put(&[v])
    }
    pub fn var(&mut self, v: u64) -> io::Result<()> {
        self.var128(u128::from(v))
    }
    pub fn var128(&mut self, mut v: u128) -> io::Result<()> {
        let mut bytes = [0_u8; 19];
        let mut len = 0;
        loop {
            let byte = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                bytes[len] = byte;
                len += 1;
                break;
            }
            bytes[len] = byte | 0x80;
            len += 1;
        }
        self.put(&bytes[..len])
    }
    pub fn zig(&mut self, v: i64) -> io::Result<()> {
        self.var(((v << 1) ^ (v >> 63)) as u64)
    }
    pub fn zig128(&mut self, v: i128) -> io::Result<()> {
        self.var128(((v << 1) ^ (v >> 127)) as u128)
    }
}
