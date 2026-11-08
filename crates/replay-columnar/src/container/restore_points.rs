//! A map restore section's framing: one extent per point, each the owner's payload, opaque
//! here, taken every [`Schedule::spacing_hours`] the owner chooses. A point's tags name its hour
//! (`first`), its ordinal in the section (`stream`) and the spacing it was taken at (`hours`).

use super::{ContainerWriter, SectionKey, SectionKind, Tags};
use crate::{
    envelope::{Extent, Kind},
    invalid,
};
use std::io::{self, Seek, Write};

/// When points are taken: the owner's choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Schedule {
    /// Hours from one point to the next.
    pub spacing_hours: u32,
}

impl Schedule {
    /// Whether a point is due at `hour` of a run starting at `first_hour`.
    pub fn due(&self, first_hour: i32, hour: i32) -> bool {
        let since = i64::from(hour) - i64::from(first_hour);
        since >= 0 && since % i64::from(self.spacing_hours.max(1)) == 0
    }
}

/// How a point's payload is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Payload {
    /// As it is, compressed within by the owner's codec ([`Kind::Stored`]).
    Stored,
    /// As one zstd frame at this level ([`Kind::Data`]).
    Zstd(i32),
}

/// A point's hour, ordinal and spacing, as its extent's tags name them.
pub fn point_of(extent: &Extent) -> (i32, u64, u32) {
    (extent.first as i32, u64::from(extent.stream), extent.hours)
}

/// Writes a map restore section's points.
pub struct MapRestoreWriter {
    key: SectionKey,
    schedule: Schedule,
    points: u64,
}

impl MapRestoreWriter {
    /// Declares map restore section `id` of the owner's schema `version` in `writer`, going on
    /// after the points it holds.
    pub fn new<W: Write + Seek>(writer: &mut ContainerWriter<W>, id: u32, version: u32, schedule: Schedule) -> io::Result<Self> {
        if schedule.spacing_hours == 0 {
            return Err(invalid("A map restore schedule spaces its points"));
        }
        let key = SectionKey::new(SectionKind::MapRestore, id);
        writer.section(key, version)?;
        let points = writer.section_of(key).map_or(0, |section| section.len());
        Ok(Self { key, schedule, points })
    }

    pub fn key(&self) -> SectionKey {
        self.key
    }

    pub fn schedule(&self) -> Schedule {
        self.schedule
    }

    /// The points the section holds.
    pub fn points(&self) -> u64 {
        self.points
    }

    /// Appends the point at `hour` holding `payload`, stored as `stored` says.
    pub fn add<W: Write + Seek>(
        &mut self,
        writer: &mut ContainerWriter<W>,
        hour: i32,
        payload: &[u8],
        stored: Payload,
    ) -> io::Result<Extent> {
        let tags = Tags {
            stream: u32::try_from(self.points).map_err(|_| invalid("A map restore section passes 2^32 points"))?,
            first: hour as u32,
            hours: self.schedule.spacing_hours,
            ..Tags::default()
        };
        let extent = match stored {
            Payload::Stored => writer.put(Kind::Stored, tags, payload.len() as u64, payload)?,
            Payload::Zstd(level) => writer.put_zstd(Kind::Data, tags, payload, level)?,
        };
        writer.append(self.key, extent)?;
        self.points += 1;
        Ok(extent)
    }
}
