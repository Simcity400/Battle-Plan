//! A recording's index, written last: its hours, every compressed unit, and every
//! restart with its dictionary chunk, ending in a checksum of everything before it.
//!
//! A restart holds the state after hour ordinal `after`; the walker that starts there
//! reads the dictionary chunks of every restart up to and including it (the directory),
//! that restart's table states, then the units from hour `after + 1`.
//!
//! Layout: hours (varint) | unit hours (varint) | units (count, then each
//! [`Unit`]) | restarts (count, then each: after, hour (zigzag), chunk offset, raw and
//! stored, state offset, raw and stored, all varints) | FNV-1a 64 of the bytes before it
//! (u64 little-endian).

use crate::{
    framing::Unit,
    invalid,
    leaf::{Reader, Writer},
};
use std::io;

#[cfg(test)]
mod tests;

/// One compressed record's place in the file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Record {
    pub offset: u64,
    pub raw: u64,
    pub stored: u64,
}

impl Record {
    fn write(&self, writer: &mut Writer) -> io::Result<()> {
        writer.varint(self.offset)?;
        writer.varint(self.raw)?;
        writer.varint(self.stored)
    }

    fn read(reader: &mut Reader<'_>) -> io::Result<Self> {
        Ok(Self {
            offset: reader.varint()?,
            raw: reader.varint()?,
            stored: reader.varint()?,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Restart {
    /// The ordinal of the last hour whose changes the state holds.
    pub after: u32,
    /// That hour.
    pub hour: i32,
    /// The dictionary strings first used since the previous restart.
    pub chunk: Record,
    /// Every table's restart bytes.
    pub state: Record,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Index {
    pub hours: u32,
    /// The hours every unit covers but a recording's last.
    pub unit_hours: u32,
    pub units: Vec<Unit>,
    pub restarts: Vec<Restart>,
}

fn zigzag(value: i32) -> u64 {
    u64::from(((value << 1) ^ (value >> 31)) as u32)
}

fn unzigzag(value: u64) -> io::Result<i32> {
    let value = u32::try_from(value).map_err(|_| invalid("Hour exceeds 32 bits"))?;
    Ok(((value >> 1) as i32) ^ -((value & 1) as i32))
}

/// FNV-1a 64: a check against a torn or damaged index, not a signature.
fn checksum(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

impl Index {
    /// Every unit starts on the unit grid and covers the grid's hours, but a unit ending the
    /// recording covers fewer; each stream's units are in hour order; every restart ends a
    /// unit within the hours, in hour order.
    fn validate(&self) -> io::Result<()> {
        if self.unit_hours == 0 {
            return Err(invalid("Units cover no hours"));
        }
        let mut last: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
        for unit in &self.units {
            let end = u64::from(unit.first) + u64::from(unit.hours);
            if unit.first % self.unit_hours != 0
                || unit.hours == 0
                || unit.hours > self.unit_hours
                || end > u64::from(self.hours)
                || (unit.hours != self.unit_hours && end != u64::from(self.hours))
            {
                return Err(invalid("A unit does not cover its place on the unit grid"));
            }
            if last
                .insert(unit.stream, unit.first)
                .is_some_and(|before| before >= unit.first)
            {
                return Err(invalid("A stream's units are not in hour order"));
            }
        }
        let mut previous: Option<u32> = None;
        for restart in &self.restarts {
            if restart.after >= self.hours
                || (u64::from(restart.after) + 1) % u64::from(self.unit_hours) != 0
                || previous.is_some_and(|before| before >= restart.after)
            {
                return Err(invalid(
                    "A restart does not end a unit within the hours, in order",
                ));
            }
            previous = Some(restart.after);
        }
        Ok(())
    }

    pub fn write(&self) -> io::Result<Vec<u8>> {
        self.validate()?;
        let mut writer = Writer::new();
        writer.varint(u64::from(self.hours))?;
        writer.varint(u64::from(self.unit_hours))?;
        writer.varint(self.units.len() as u64)?;
        for unit in &self.units {
            unit.write(&mut writer)?;
        }
        writer.varint(self.restarts.len() as u64)?;
        for restart in &self.restarts {
            writer.varint(u64::from(restart.after))?;
            writer.varint(zigzag(restart.hour))?;
            restart.chunk.write(&mut writer)?;
            restart.state.write(&mut writer)?;
        }
        let mut bytes = writer.into_bytes();
        let sum = checksum(&bytes);
        bytes.extend_from_slice(&sum.to_le_bytes());
        Ok(bytes)
    }

    pub fn read(bytes: &[u8]) -> io::Result<Self> {
        let split = bytes
            .len()
            .checked_sub(8)
            .ok_or_else(|| invalid("Index is shorter than its checksum"))?;
        let (body, sum) = bytes.split_at(split);
        if checksum(body).to_le_bytes() != sum {
            return Err(invalid("Index checksum differs"));
        }
        let mut reader = Reader::new(body);
        let hours = reader.varint_u32()?;
        let unit_hours = reader.varint_u32()?;
        if unit_hours == 0 {
            return Err(invalid("Units cover no hours"));
        }
        let count = reader.count()?;
        let mut units = engine_allocation::vec(count.min(body.len()))?;
        for _ in 0..count {
            units.push(Unit::read(&mut reader)?);
        }
        let count = reader.count()?;
        let mut restarts = engine_allocation::vec(count.min(body.len()))?;
        for _ in 0..count {
            restarts.push(Restart {
                after: reader.varint_u32()?,
                hour: unzigzag(reader.varint()?)?,
                chunk: Record::read(&mut reader)?,
                state: Record::read(&mut reader)?,
            });
        }
        reader.finish()?;
        let index = Self {
            hours,
            unit_hours,
            units,
            restarts,
        };
        index.validate()?;
        Ok(index)
    }

    /// The units of `stream` from hour ordinal `from`, in order.
    pub fn stream_from(&self, stream: u32, from: u32) -> impl Iterator<Item = &Unit> {
        self.units.iter().filter(move |unit| {
            unit.stream == stream && u64::from(unit.first) + u64::from(unit.hours) > u64::from(from)
        })
    }
}
