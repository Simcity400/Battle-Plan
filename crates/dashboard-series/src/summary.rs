use crate::{
    add,
    codec::{Binary, Decode, Encode},
    invalid,
    types::{Run, Span},
};
use std::io;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct U256(pub [u64; 4]);
impl U256 {
    pub fn checked_mul(self, factor: u64) -> io::Result<Self> {
        let mut result = [0; 4];
        let mut carry = 0_u128;
        for (index, limb) in result.iter_mut().enumerate() {
            let value = u128::from(self.0[index]) * u128::from(factor) + carry;
            *limb = value as u64;
            carry = value >> 64;
        }
        if carry != 0 {
            return Err(invalid("series exact contribution overflow"));
        }
        Ok(Self(result))
    }
    pub fn contribution(value: i128, hours: u64) -> Self {
        let magnitude = value.unsigned_abs();
        let low = (magnitude as u64 as u128) * u128::from(hours);
        let high = (magnitude >> 64) * u128::from(hours) + (low >> 64);
        Self([low as u64, high as u64, (high >> 64) as u64, 0])
    }
    pub fn checked_add(self, other: Self) -> io::Result<Self> {
        let mut result = [0; 4];
        let mut carry = false;
        for (i, out) in result.iter_mut().enumerate() {
            let (v, a) = self.0[i].overflowing_add(other.0[i]);
            let (v, b) = v.overflowing_add(u64::from(carry));
            *out = v;
            carry = a || b;
        }
        if carry {
            return Err(invalid("series exact contribution overflow"));
        }
        Ok(Self(result))
    }
}
impl Binary for U256 {
    const SIZE: usize = 32;
    fn encode(&self, e: &mut Encode<'_>) -> io::Result<()> {
        for v in self.0 {
            e.u64(v)?;
        }
        Ok(())
    }
    fn decode(d: &mut Decode<'_>) -> io::Result<Self> {
        Ok(Self([d.u64()?, d.u64()?, d.u64()?, d.u64()?]))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Summary {
    pub absolute_total: U256,
    pub covered: u64,
    pub first_hour: i32,
    pub last_hour: i32,
    pub last_value_hour: i32,
    pub min_hour: i32,
    pub max_hour: i32,
    pub first_value: f64,
    pub last_value: f64,
    pub min_value: f64,
    pub max_value: f64,
    pub raw_min: i128,
    pub raw_max: i128,
    pub first_break: bool,
    pub has_break: bool,
}
impl Summary {
    pub fn run(run: Run, span: Span, decimals: u8) -> Self {
        let value = run.value as f64 / 10_f64.powi(i32::from(decimals));
        Self {
            absolute_total: U256::contribution(run.value, span.hours()),
            covered: span.hours(),
            first_hour: span.first,
            last_hour: span.last,
            last_value_hour: span.first,
            min_hour: span.first,
            max_hour: span.first,
            first_value: value,
            last_value: value,
            min_value: value,
            max_value: value,
            raw_min: run.value,
            raw_max: run.value,
            first_break: run.break_before && span.first == run.span.first,
            has_break: false,
        }
    }
    /// A run continued unchanged through `hours`, as its piece there sums.
    pub(crate) fn held(value: i128, hours: Span, decimals: u8) -> Self {
        Self::run(
            Run {
                span: hours,
                value,
                break_before: false,
                supplying_boundary: false,
                canonical: 0,
                sequence: 0,
            },
            hours,
            decimals,
        )
    }
    pub fn merge(self, b: Self) -> io::Result<Self> {
        let adjacent = i64::from(self.last_hour) + 1 == i64::from(b.first_hour);
        Ok(Self {
            absolute_total: self.absolute_total.checked_add(b.absolute_total)?,
            covered: add(self.covered, b.covered)?,
            first_hour: self.first_hour,
            last_hour: b.last_hour,
            first_value: self.first_value,
            last_value: b.last_value,
            // A break after `b`'s one stretch is a coverage gap: the value still began in `self`.
            last_value_hour: if b.last_value_hour == b.first_hour
                && !b.first_break
                && adjacent
                && b.min_value == b.max_value
                && self.last_value == b.first_value
            {
                self.last_value_hour
            } else {
                b.last_value_hour
            },
            min_hour: if self.min_value <= b.min_value {
                self.min_hour
            } else {
                b.min_hour
            },
            max_hour: if self.max_value >= b.max_value {
                self.max_hour
            } else {
                b.max_hour
            },
            min_value: self.min_value.min(b.min_value),
            max_value: self.max_value.max(b.max_value),
            raw_min: self.raw_min.min(b.raw_min),
            raw_max: self.raw_max.max(b.raw_max),
            first_break: self.first_break,
            has_break: self.has_break || b.has_break || b.first_break || !adjacent,
        })
    }
    pub fn constant(self) -> bool {
        !self.has_break
            && self.raw_min == self.raw_max
            && self.first_hour <= self.last_hour
            && self.covered == (i64::from(self.last_hour) - i64::from(self.first_hour) + 1) as u64
    }
    pub fn clip_constant(self, span: Span, decimals: u8) -> io::Result<Self> {
        if !self.constant() {
            return Err(invalid("nonconstant summary clip"));
        }
        let original = Span::new(self.first_hour, self.last_hour)?;
        let span = original
            .intersect(span)
            .ok_or_else(|| invalid("empty constant clip"))?;
        Ok(Self::run(
            Run {
                span: original,
                value: self.raw_min,
                break_before: self.first_break,
                supplying_boundary: false,
                canonical: 0,
                sequence: 0,
            },
            span,
            decimals,
        ))
    }
}
impl Binary for Summary {
    const SIZE: usize = 126;
    fn encode(&self, e: &mut Encode<'_>) -> io::Result<()> {
        self.absolute_total.encode(e)?;
        e.u64(self.covered)?;
        for h in [
            self.first_hour,
            self.last_hour,
            self.last_value_hour,
            self.min_hour,
            self.max_hour,
        ] {
            e.i32(h)?;
        }
        for v in [
            self.first_value,
            self.last_value,
            self.min_value,
            self.max_value,
        ] {
            e.f64(v)?;
        }
        e.i128(self.raw_min)?;
        e.i128(self.raw_max)?;
        e.u8(u8::from(self.first_break))?;
        e.u8(u8::from(self.has_break))
    }
    fn decode(d: &mut Decode<'_>) -> io::Result<Self> {
        let v = Self {
            absolute_total: U256::decode(d)?,
            covered: d.u64()?,
            first_hour: d.i32()?,
            last_hour: d.i32()?,
            last_value_hour: d.i32()?,
            min_hour: d.i32()?,
            max_hour: d.i32()?,
            first_value: d.f64()?,
            last_value: d.f64()?,
            min_value: d.f64()?,
            max_value: d.f64()?,
            raw_min: d.i128()?,
            raw_max: d.i128()?,
            first_break: flag(d)?,
            has_break: flag(d)?,
        };
        if v.covered == 0 || v.raw_min > v.raw_max || v.min_value > v.max_value {
            return Err(invalid("invalid series summary"));
        }
        Ok(v)
    }
}
fn flag(d: &mut Decode<'_>) -> io::Result<bool> {
    match d.u8()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(invalid("invalid summary flag")),
    }
}
