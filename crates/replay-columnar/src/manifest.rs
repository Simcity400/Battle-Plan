
use crate::{
    invalid,
    leaf::{Reader, Writer},
};
use std::io;

#[cfg(test)]
mod tests;

pub const BLOCK_START: u8 = 1;
pub const AFTER_GAP: u8 = 2;
pub const AFTER_LOAD: u8 = 4;
pub const SAMPLES: u8 = 8;
const FLAGS: u8 = BLOCK_START | AFTER_GAP | AFTER_LOAD | SAMPLES;

pub const INSERTS: u8 = 1;
pub const DELETES: u8 = 2;
pub const FILLS: u8 = 4;
pub const TOUCHES: u8 = 8;
const OPS: u8 = INSERTS | DELETES | FILLS | TOUCHES;

fn within_columns(bits: &[u64], columns: usize) -> bool {
    columns % 64 == 0 || bits.last().is_none_or(|&last| last >> (columns % 64) == 0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub enum Status {
    NotRecorded,
    Complete,
    Truncated {
        missing: u32,
    },
    Unavailable {
        reason: u32,
    },
}

impl Status {
    pub(crate) fn write(self, writer: &mut Writer) -> io::Result<()> {
        match self {
            Self::NotRecorded => writer.u8(0),
            Self::Complete => writer.u8(1),
            Self::Truncated { missing } => {
                writer.u8(2)?;
                writer.varint(u64::from(missing))
            }
            Self::Unavailable { reason } => {
                writer.u8(3)?;
                writer.varint(u64::from(reason))
            }
        }
    }

    pub(crate) fn read(reader: &mut Reader<'_>) -> io::Result<Self> {
        Ok(match reader.u8()? {
            0 => Self::NotRecorded,
            1 => Self::Complete,
            2 => Self::Truncated {
                missing: reader.varint_u32()?,
            },
            3 => Self::Unavailable {
                reason: reader.varint_u32()?,
            },
            other => return Err(invalid(format!("Table status {other}"))),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub table: usize,
    pub bytes: u64,
    pub columns: Vec<u64>,
    pub ops: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest<'a> {
    pub branch: u64,
    pub hour: i32,
    pub flags: u8,
    pub total_ns: u64,
    pub pool_bytes: u64,
    pub dropped_frames: u64,
    pub appends: Vec<&'a [u8]>,
    pub statuses: Vec<Status>,
    pub read: Vec<u64>,
    pub samples: Vec<(usize, Option<i32>)>,
    pub sections: Vec<Section>,
}

fn put_sample(writer: &mut Writer, hour: Option<i32>) -> io::Result<()> {
    writer.varint(hour.map_or(0, |hour| zigzag(hour) + 1))
}

fn get_sample(reader: &mut Reader<'_>) -> io::Result<Option<i32>> {
    match reader.varint()? {
        0 => Ok(None),
        value => unzigzag(value - 1).map(Some),
    }
}

fn zigzag(value: i32) -> u64 {
    u64::from(((value << 1) ^ (value >> 31)) as u32)
}

fn unzigzag(value: u64) -> io::Result<i32> {
    let value = u32::try_from(value).map_err(|_| invalid("Hour exceeds 32 bits"))?;
    Ok(((value >> 1) as i32) ^ -((value & 1) as i32))
}

impl Manifest<'_> {
    pub fn write(&self, columns: &[usize]) -> io::Result<Vec<u8>> {
        let tables = columns.len();
        if self.statuses.len() != tables || self.read.len() != tables.div_ceil(64) {
            return Err(invalid("Manifest disagrees with the schema's tables"));
        }
        if self.flags & !FLAGS != 0 {
            return Err(invalid(format!("Manifest flags {:#x}", self.flags)));
        }
        if tables % 64 != 0
            && self
                .read
                .last()
                .is_some_and(|&last| last >> (tables % 64) != 0)
        {
            return Err(invalid("Manifest read bits pass the schema's tables"));
        }
        if (self.flags & SAMPLES != 0) == self.samples.is_empty()
            || self.samples.windows(2).any(|pair| pair[0].0 >= pair[1].0)
            || self
                .samples
                .last()
                .is_some_and(|&(table, _)| table >= tables)
        {
            return Err(invalid("Manifest samples disagree with its flag or tables"));
        }
        let mut writer = Writer::new();
        writer.varint(self.branch)?;
        writer.varint(zigzag(self.hour))?;
        writer.u8(self.flags)?;
        writer.varint(self.total_ns)?;
        writer.varint(self.pool_bytes)?;
        writer.varint(self.dropped_frames)?;
        writer.varint(self.appends.len() as u64)?;
        for append in &self.appends {
            writer.varint(append.len() as u64)?;
            writer.raw(append)?;
        }
        for &status in &self.statuses {
            status.write(&mut writer)?;
        }
        writer.values(&self.read)?;
        if !self.samples.is_empty() {
            writer.varint(self.samples.len() as u64)?;
            for &(table, hour) in &self.samples {
                writer.varint(table as u64)?;
                put_sample(&mut writer, hour)?;
            }
        }
        let mut present = engine_allocation::filled(tables.div_ceil(64), 0u64)?;
        for pair in self.sections.windows(2) {
            if pair[0].table >= pair[1].table {
                return Err(invalid("Manifest sections are not in table order"));
            }
        }
        for section in &self.sections {
            if section.table >= tables
                || section.columns.len() != columns[section.table].div_ceil(64)
                || !within_columns(&section.columns, columns[section.table])
                || section.ops & !OPS != 0
            {
                return Err(invalid("Manifest section disagrees with its table"));
            }
            present[section.table / 64] |= 1 << (section.table % 64);
        }
        writer.values(&present)?;
        for section in &self.sections {
            writer.varint(section.bytes)?;
            writer.values(&section.columns)?;
            writer.u8(section.ops)?;
        }
        Ok(writer.into_bytes())
    }
}

impl<'a> Manifest<'a> {
    pub fn read(bytes: &'a [u8], columns: &[usize]) -> io::Result<Self> {
        let (manifest, read) = Self::read_prefix(bytes, columns)?;
        if read != bytes.len() {
            return Err(invalid(format!(
                "Manifest has {} bytes after its end",
                bytes.len() - read
            )));
        }
        Ok(manifest)
    }

    pub fn read_prefix(bytes: &'a [u8], columns: &[usize]) -> io::Result<(Self, usize)> {
        let tables = columns.len();
        let mut reader = Reader::new(bytes);
        let branch = reader.varint()?;
        let hour = unzigzag(reader.varint()?)?;
        let flags = reader.u8()?;
        if flags & !FLAGS != 0 {
            return Err(invalid(format!("Manifest flags {flags:#x}")));
        }
        let total_ns = reader.varint()?;
        let pool_bytes = reader.varint()?;
        let dropped_frames = reader.varint()?;
        let count = reader.count()?;
        let mut appends = engine_allocation::vec(count.min(bytes.len()))?;
        for _ in 0..count {
            let length = reader.count()?;
            appends.push(reader.raw(length)?);
        }
        let mut statuses = engine_allocation::vec(tables)?;
        for _ in 0..tables {
            statuses.push(Status::read(&mut reader)?);
        }
        let mut read: Vec<u64> = Vec::new();
        reader.values(tables.div_ceil(64), &mut read)?;
        if tables % 64 != 0 && read.last().is_some_and(|&last| last >> (tables % 64) != 0) {
            return Err(invalid("Manifest read bits pass the schema's tables"));
        }
        let mut samples = Vec::new();
        if flags & SAMPLES != 0 {
            let count = reader.count()?;
            if count == 0 || count > tables {
                return Err(invalid("Manifest samples disagree with its tables"));
            }
            engine_allocation::reserve(&mut samples, count)?;
            for _ in 0..count {
                let table = reader.count()?;
                if table >= tables || samples.last().is_some_and(|&(last, _)| last >= table) {
                    return Err(invalid("Manifest samples are not tables in order"));
                }
                samples.push((table, get_sample(&mut reader)?));
            }
        }
        let mut present: Vec<u64> = Vec::new();
        reader.values(tables.div_ceil(64), &mut present)?;
        let mut sections = Vec::new();
        for (index, &word) in present.iter().enumerate() {
            let mut rest = word;
            while rest != 0 {
                let table = index * 64 + rest.trailing_zeros() as usize;
                rest &= rest - 1;
                let count = *columns
                    .get(table)
                    .ok_or_else(|| invalid("Manifest names a table past the schema"))?;
                let bytes = reader.varint()?;
                let mut changed = Vec::new();
                reader.values(count.div_ceil(64), &mut changed)?;
                if !within_columns(&changed, count) {
                    return Err(invalid("Manifest section names a column past its table"));
                }
                let ops = reader.u8()?;
                if ops & !OPS != 0 {
                    return Err(invalid(format!("Manifest section op flags {ops:#x}")));
                }
                engine_allocation::reserve(&mut sections, 1)?;
                sections.push(Section {
                    table,
                    bytes,
                    columns: changed,
                    ops,
                });
            }
        }
        Ok((
            Self {
                branch,
                hour,
                flags,
                total_ns,
                pool_bytes,
                dropped_frames,
                appends,
                statuses,
                read,
                samples,
                sections,
            },
            reader.at(),
        ))
    }
}
