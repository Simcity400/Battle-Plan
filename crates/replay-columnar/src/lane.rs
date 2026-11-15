//! A block's clock ([`Clock`]): every hour's flags and metrics, what is truly per hour.

use crate::{
    block::{put_varint, unzig, zig},
    invalid,
    leaf::Reader,
};
use std::io;

/// One hour's flags and metrics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClockHour {
    pub flags: u8,
    pub total_ns: u64,
    pub pool_bytes: u64,
    pub dropped_frames: u64,
}

/// A block's clock: every hour's flags, then its metrics.
#[derive(Default)]
pub struct Clock {
    flags: Vec<u8>,
    metrics: Vec<u8>,
    pool: u64,
}

impl Clock {
    pub fn hour(&mut self, hour: &ClockHour) {
        self.flags.push(hour.flags);
        put_varint(&mut self.metrics, hour.total_ns);
        put_varint(&mut self.metrics, zig(hour.pool_bytes.wrapping_sub(self.pool) as i64));
        self.pool = hour.pool_bytes;
        put_varint(&mut self.metrics, hour.dropped_frames);
    }

    pub fn finish(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(5 + self.flags.len() + self.metrics.len());
        put_varint(&mut out, self.flags.len() as u64);
        out.extend_from_slice(&self.flags);
        out.extend_from_slice(&self.metrics);
        out
    }

    /// Every hour of the clock `payload` of a block of `hours` ordinals.
    pub fn read(payload: &[u8], hours: u32) -> io::Result<Vec<ClockHour>> {
        let mut reader = Reader::new(payload);
        if reader.varint_u32()? != hours {
            return Err(invalid("A clock holds other hours than its block"));
        }
        let flags = reader.raw(hours as usize)?;
        let mut out = engine_allocation::vec(hours as usize)?;
        let mut pool = 0_u64;
        for &flags in flags {
            let total_ns = reader.varint()?;
            pool = pool.wrapping_add(unzig(reader.varint()?) as u64);
            out.push(ClockHour {
                flags,
                total_ns,
                pool_bytes: pool,
                dropped_frames: reader.varint()?,
            });
        }
        reader.finish()?;
        Ok(out)
    }
}
