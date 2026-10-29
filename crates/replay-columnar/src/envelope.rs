//! A columnar recording's saved-file envelope (`H4COLUMN` version 3).
//!
//! The file is append-only: zstd extents in any order, then a [`Root`] extent, then a
//! fixed [`FOOTER_BYTES`] footer naming the root. A writer appends extents, syncs, appends
//! the root, syncs, then appends the footer and syncs; a reader takes the last footer whose
//! root verifies ([`last_root`]), so a torn tail after it is ignored, and a writer reopening
//! the file truncates that tail first. Every extent is described where it is referenced, by
//! an [`Extent`] envelope carrying a digest of its stored bytes, so a reader checks an
//! extent's place and length before allocating and its bytes before decoding.
//!
//! A run is one continuous span of one branch and observation epoch, on its own grid of
//! [`UNIT_ORDINALS`]-ordinal units: one [`IndexPage`] per unit position names the unit
//! extents of every stream, the restart that may end the position (one extent per table
//! and the run's dictionary chunk, under one ordinal barrier), which tables the position
//! touched, and the hour digest chain after it. Dictionary ids are run-local. Until a
//! position seals, a stream's unit may be a chain of contiguous sub-extents that later
//! revisions extend; readers concatenate their raw bytes in order. Each run's pages are
//! listed by its own [`RunDirectory`] extent and the recording's control records by
//! [`Kind::Controls`] extents, so a revision rewrites only the active run's directory and a
//! small root.
//!
//! The chain covers raw bytes ([`chain_hour`]), so recompressing extents never changes a
//! recording's logical identity.

use crate::{
    invalid,
    leaf::{Reader, Writer},
    state::Bits,
};
use std::io;

#[cfg(test)]
mod tests;

pub const MAGIC: &[u8; 8] = b"H4COLUMN";
pub const VERSION: u32 = 3;
pub const HEADER_BYTES: usize = 12;
pub const FOOTER_MAGIC: &[u8; 8] = b"H4COLEND";
/// Root offset, root stored length, root digest and the footer magic.
pub const FOOTER_BYTES: usize = 8 + 8 + 16 + 8;
/// Recorded ordinals per unit; only a run's last unit may hold fewer.
pub const UNIT_ORDINALS: u32 = 720;
/// The fewest bytes an [`Extent`] envelope encodes in: its kind, seven varints, its digest.
const EXTENT_BYTES: usize = 1 + 7 + 16;

/// A count read from `bytes`, refused when it exceeds `limit` or more entries of
/// `entry_bytes` each than the bytes could hold.
fn bounded(count: usize, limit: usize, bytes: &[u8], entry_bytes: usize) -> io::Result<usize> {
    if count > limit || count > bytes.len() / entry_bytes.max(1) {
        return Err(invalid("A count exceeds what its bytes or schema can hold"));
    }
    Ok(count)
}

/// BLAKE3 of `bytes`, truncated to 128 bits: binds stored bytes to their envelope.
pub fn digest(bytes: &[u8]) -> [u8; 16] {
    let hash = blake3::hash(bytes);
    hash.as_bytes()[..16].try_into().expect("sixteen bytes")
}

/// The digest of a schema: each table's column kinds, in table order.
pub fn schema_digest(schema: &[Vec<crate::state::Kind>]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    for table in schema {
        hasher.update(&(table.len() as u64).to_le_bytes());
        for &kind in table {
            hasher.update(&[kind as u8]);
        }
    }
    *hasher.finalize().as_bytes()
}

/// The chain value before a run's first hour: the identity block and the run.
pub fn chain_start(identity: &[u8], run: u32) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(identity);
    hasher.update(&run.to_le_bytes());
    *hasher.finalize().as_bytes()
}

/// The chain value after one hour: the value before it, the hour's raw manifest, then
/// each of its sections' raw bytes in manifest order.
pub fn chain_hour<'b>(
    before: &[u8; 32],
    manifest: &[u8],
    sections: impl IntoIterator<Item = &'b [u8]>,
) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(before);
    hasher.update(&(manifest.len() as u64).to_le_bytes());
    hasher.update(manifest);
    for section in sections {
        hasher.update(&(section.len() as u64).to_le_bytes());
        hasher.update(section);
    }
    *hasher.finalize().as_bytes()
}

/// What an extent holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// One stream's sections (or manifests) for one unit position.
    Unit = 1,
    /// The run-local strings first used since the run's previous restart.
    DictionaryChunk = 2,
    /// One table's state at a restart.
    RestartTable = 3,
    IndexPage = 4,
    Root = 5,
    /// One run's place in the campaign and its pages ([`RunDirectory`]).
    RunDirectory = 6,
    /// Control records, opaque here and read by `replay`; each revision appends one.
    Controls = 7,
}

impl Kind {
    fn read(byte: u8) -> io::Result<Self> {
        Ok(match byte {
            1 => Self::Unit,
            2 => Self::DictionaryChunk,
            3 => Self::RestartTable,
            4 => Self::IndexPage,
            5 => Self::Root,
            6 => Self::RunDirectory,
            7 => Self::Controls,
            _ => return Err(invalid(format!("Unknown extent kind {byte}"))),
        })
    }
}

/// Why a run starts. Readers keep codes they do not know rather than refuse them.
pub mod epoch {
    /// The first run of a session.
    pub const START: u8 = 0;
    /// A save was loaded: a new game branch.
    pub const LOAD: u8 = 1;
    /// A rewind or branch: the branch tracker's parent changed.
    pub const BRANCH: u8 = 2;
    /// Dropped or unknown hours.
    pub const GAP: u8 = 3;
    /// The world epoch was rebuilt within a session.
    pub const WORLD_DISCONTINUITY: u8 = 4;
    /// A later session resuming the same campaign.
    pub const CONTINUATION: u8 = 5;
    /// A new hour-log generation after a crash.
    pub const RECOVERY_RESUME: u8 = 6;
    /// Hours refused by load admission, then resumed.
    pub const ADMISSION_REFUSAL: u8 = 7;
}

/// Where one extent is and what it holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extent {
    pub kind: Kind,
    pub run: u32,
    /// A unit's stream (0 manifests, `1 + table`); a restart table's table; else 0.
    pub stream: u32,
    /// A unit's or page's first ordinal; a restart's barrier (the last ordinal it holds).
    pub first: u32,
    /// Ordinals a unit or page covers; 0 otherwise.
    pub hours: u32,
    pub offset: u64,
    pub stored: u64,
    pub raw: u64,
    pub digest: [u8; 16],
}

impl Extent {
    pub fn write(&self, writer: &mut Writer) -> io::Result<()> {
        writer.u8(self.kind as u8)?;
        writer.varint(u64::from(self.run))?;
        writer.varint(u64::from(self.stream))?;
        writer.varint(u64::from(self.first))?;
        writer.varint(u64::from(self.hours))?;
        writer.varint(self.offset)?;
        writer.varint(self.stored)?;
        writer.varint(self.raw)?;
        writer.raw(&self.digest)
    }

    pub fn read(reader: &mut Reader<'_>) -> io::Result<Self> {
        Ok(Self {
            kind: Kind::read(reader.u8()?)?,
            run: reader.varint_u32()?,
            stream: reader.varint_u32()?,
            first: reader.varint_u32()?,
            hours: reader.varint_u32()?,
            offset: reader.varint()?,
            stored: reader.varint()?,
            raw: reader.varint()?,
            digest: reader.raw(16)?.try_into().expect("sixteen bytes"),
        })
    }

    /// The stored bytes in `file`, checked against the envelope: inside the extents,
    /// before the footer, and with this digest.
    pub fn stored_in<'f>(&self, file: &'f [u8]) -> io::Result<&'f [u8]> {
        let end = file.len().saturating_sub(FOOTER_BYTES);
        let start = usize::try_from(self.offset)
            .ok()
            .filter(|&start| start >= HEADER_BYTES);
        let length = usize::try_from(self.stored).ok();
        let bytes = start
            .zip(length)
            .and_then(|(start, length)| Some(start..start.checked_add(length)?))
            .filter(|range| range.end <= end)
            .map(|range| &file[range])
            .ok_or_else(|| self.damaged("lies outside the recording's extents"))?;
        if digest(bytes) != self.digest {
            return Err(self.damaged("differs from its digest"));
        }
        Ok(bytes)
    }

    /// A refusal naming this extent's scope.
    pub fn damaged(&self, what: &str) -> io::Error {
        invalid(format!(
            "{:?} extent of run {} stream {} from ordinal {} ({} hours) {what}",
            self.kind, self.run, self.stream, self.first, self.hours
        ))
    }

    fn expect(&self, kind: Kind, run: u32) -> io::Result<()> {
        if self.kind != kind || self.run != run {
            return Err(self.damaged(&format!("is not a {kind:?} extent of run {run}")));
        }
        Ok(())
    }
}

/// One table's state at a restart, with the run-local dictionary ids it references.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestartTable {
    pub table: u32,
    pub extent: Extent,
    /// The run's dictionary chunks its state references, by their restarts' ordinals in
    /// the run, ascending: a damaged chunk refuses exactly the tables listing it.
    pub chunks: Vec<u32>,
}

/// The restart ending a unit position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestartDirectory {
    /// The last ordinal whose changes the restart holds.
    pub after: u32,
    pub hour: i32,
    /// This restart's ordinal among its run's restarts.
    pub ordinal: u32,
    pub chunk: Extent,
    /// The run-local dictionary ids the chunk holds: its first, and how many.
    pub ids: (u32, u32),
    /// Every table's state, ascending by table.
    pub tables: Vec<RestartTable>,
}

/// One unit position of a run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexPage {
    pub run: u32,
    pub first: u32,
    pub hours: u32,
    pub first_hour: i32,
    pub last_hour: i32,
    /// Each present stream's unit, ascending by stream, stream 0 first: one extent, or until
    /// the position seals, a chain of contiguous sub-extents in ordinal order.
    pub units: Vec<Extent>,
    pub restart: Option<RestartDirectory>,
    /// Tables some hour of the position has a section for.
    pub sectioned: Bits,
    /// Tables some hour of the position read.
    pub read: Bits,
    /// Tables whose status changed within the position or at its first ordinal.
    pub status_changed: Bits,
    /// The hour digest chain after the position's last ordinal.
    pub chain: [u8; 32],
}

fn zigzag(value: i32) -> u64 {
    u64::from(((value << 1) ^ (value >> 31)) as u32)
}

fn unzigzag(value: u64) -> io::Result<i32> {
    let value = u32::try_from(value).map_err(|_| invalid("Hour exceeds 32 bits"))?;
    Ok(((value >> 1) as i32) ^ -((value & 1) as i32))
}

fn write_bits(writer: &mut Writer, bits: &Bits) -> io::Result<()> {
    writer.varint(bits.words().len() as u64)?;
    for word in bits.words() {
        writer.raw(&word.to_le_bytes())?;
    }
    Ok(())
}

fn read_bits(reader: &mut Reader<'_>, tables: usize) -> io::Result<Bits> {
    let count = reader.count()?;
    if count != tables.div_ceil(64) {
        return Err(invalid(
            "A page's table bitmap differs from the schema's tables",
        ));
    }
    let mut words = engine_allocation::vec(count)?;
    for _ in 0..count {
        words.push(u64::from_le_bytes(
            reader.raw(8)?.try_into().expect("eight bytes"),
        ));
    }
    Bits::from_words(words)
}

impl IndexPage {
    pub fn write(&self) -> io::Result<Vec<u8>> {
        let mut writer = Writer::new();
        writer.varint(u64::from(self.run))?;
        writer.varint(u64::from(self.first))?;
        writer.varint(u64::from(self.hours))?;
        writer.varint(zigzag(self.first_hour))?;
        writer.varint(zigzag(self.last_hour))?;
        writer.varint(self.units.len() as u64)?;
        for unit in &self.units {
            unit.write(&mut writer)?;
        }
        match &self.restart {
            None => writer.u8(0)?,
            Some(restart) => {
                writer.u8(1)?;
                writer.varint(u64::from(restart.after))?;
                writer.varint(zigzag(restart.hour))?;
                writer.varint(u64::from(restart.ordinal))?;
                restart.chunk.write(&mut writer)?;
                writer.varint(u64::from(restart.ids.0))?;
                writer.varint(u64::from(restart.ids.1))?;
                writer.varint(restart.tables.len() as u64)?;
                for table in &restart.tables {
                    writer.varint(u64::from(table.table))?;
                    table.extent.write(&mut writer)?;
                    writer.varint(table.chunks.len() as u64)?;
                    for &chunk in &table.chunks {
                        writer.varint(u64::from(chunk))?;
                    }
                }
            }
        }
        write_bits(&mut writer, &self.sectioned)?;
        write_bits(&mut writer, &self.read)?;
        write_bits(&mut writer, &self.status_changed)?;
        writer.raw(&self.chain)?;
        Ok(writer.into_bytes())
    }

    /// A page of a schema with `tables` tables, checked against the envelope `page` it
    /// was read through.
    pub fn read(bytes: &[u8], page: &Extent, tables: usize) -> io::Result<Self> {
        let mut reader = Reader::new(bytes);
        let run = reader.varint_u32()?;
        let first = reader.varint_u32()?;
        let hours = reader.varint_u32()?;
        let first_hour = unzigzag(reader.varint()?)?;
        let last_hour = unzigzag(reader.varint()?)?;
        // A stream's unit is at most one piece per ordinal of the position.
        let limit = (tables + 1) * UNIT_ORDINALS as usize;
        let count = bounded(reader.count()?, limit, bytes, EXTENT_BYTES)?;
        let mut units = engine_allocation::vec(count)?;
        for _ in 0..count {
            units.push(Extent::read(&mut reader)?);
        }
        let restart = match reader.u8()? {
            0 => None,
            1 => {
                let after = reader.varint_u32()?;
                let hour = unzigzag(reader.varint()?)?;
                let ordinal = reader.varint_u32()?;
                let chunk = Extent::read(&mut reader)?;
                let ids = (reader.varint_u32()?, reader.varint_u32()?);
                if reader.count()? != tables {
                    return Err(invalid("A restart does not hold every table once"));
                }
                let mut restored = engine_allocation::vec(tables)?;
                for _ in 0..tables {
                    let table = reader.varint_u32()?;
                    let extent = Extent::read(&mut reader)?;
                    let limit = ordinal as usize + 1;
                    let count = bounded(reader.count()?, limit, bytes, 1)?;
                    let mut chunks = engine_allocation::vec(count)?;
                    for _ in 0..count {
                        chunks.push(reader.varint_u32()?);
                    }
                    restored.push(RestartTable {
                        table,
                        extent,
                        chunks,
                    });
                }
                Some(RestartDirectory {
                    after,
                    hour,
                    ordinal,
                    chunk,
                    ids,
                    tables: restored,
                })
            }
            _ => return Err(invalid("A page's restart marker is malformed")),
        };
        let sectioned = read_bits(&mut reader, tables)?;
        let read = read_bits(&mut reader, tables)?;
        let status_changed = read_bits(&mut reader, tables)?;
        let chain = reader.raw(32)?.try_into().expect("thirty-two bytes");
        reader.finish()?;
        let page_read = Self {
            run,
            first,
            hours,
            first_hour,
            last_hour,
            units,
            restart,
            sectioned,
            read,
            status_changed,
            chain,
        };
        if (page.run, page.first, page.hours) != (run, first, hours) {
            return Err(page.damaged("holds another unit position"));
        }
        page_read.validate(tables)?;
        Ok(page_read)
    }

    /// The extents of stream `stream`'s unit, in ordinal order.
    pub fn stream(&self, stream: u32) -> impl Iterator<Item = &Extent> {
        self.units.iter().filter(move |unit| unit.stream == stream)
    }

    /// Units of this position by stream, each stream's extents in ordinal order without
    /// overlap and within the position, the manifests' covering it whole; a table stream's
    /// extent followed by another keeps its sections aligned (a multiple of 8 raw bytes); a
    /// restart ending the position holds every table's state once, in order.
    fn validate(&self, tables: usize) -> io::Result<()> {
        if self.hours == 0 || self.hours > UNIT_ORDINALS || self.first % UNIT_ORDINALS != 0 {
            return Err(invalid("A page does not cover a unit position"));
        }
        if self.units.first().is_none_or(|unit| unit.stream != 0) {
            return Err(invalid("A page has no manifest unit"));
        }
        // Ordinal arithmetic in u64: untrusted values never overflow.
        let stop = |first: u32, hours: u32| u64::from(first) + u64::from(hours);
        let end = stop(self.first, self.hours);
        if end > u64::from(u32::MAX) + 1 {
            return Err(invalid("A page ends past the last ordinal"));
        }
        let mut previous: Option<&Extent> = None;
        for unit in &self.units {
            unit.expect(Kind::Unit, self.run)?;
            let continues = previous.filter(|before| before.stream == unit.stream);
            let placed = unit.hours > 0
                && unit.first >= self.first
                && stop(unit.first, unit.hours) <= end
                && unit.stream as usize <= tables
                && previous.is_none_or(|before| before.stream <= unit.stream)
                && continues.is_none_or(|before| {
                    stop(before.first, before.hours) <= u64::from(unit.first)
                        && (unit.stream == 0 || before.raw % 8 == 0)
                })
                && (unit.stream != 0
                    || continues.map_or(unit.first == self.first, |before| {
                        stop(before.first, before.hours) == u64::from(unit.first)
                    }));
            if !placed {
                return Err(unit.damaged("does not belong to its page's position"));
            }
            previous = Some(unit);
        }
        let covered = self
            .stream(0)
            .last()
            .is_some_and(|last| stop(last.first, last.hours) == end);
        if !covered {
            return Err(invalid("A page's manifests do not cover its position"));
        }
        if let Some(restart) = &self.restart {
            restart.chunk.expect(Kind::DictionaryChunk, self.run)?;
            if u64::from(restart.after) + 1 != end || restart.chunk.first != restart.after {
                return Err(invalid("A restart does not end its page's position"));
            }
            if u64::from(restart.ids.0) + u64::from(restart.ids.1) > u64::from(u32::MAX) + 1 {
                return Err(invalid("A dictionary chunk's ids pass the last id"));
            }
            if restart.tables.len() != tables
                || restart
                    .tables
                    .iter()
                    .enumerate()
                    .any(|(at, table)| table.table as usize != at)
            {
                return Err(invalid(
                    "A restart does not hold every table once, in order",
                ));
            }
            for table in &restart.tables {
                table.extent.expect(Kind::RestartTable, self.run)?;
                if table.extent.stream != table.table || table.extent.first != restart.after {
                    return Err(table.extent.damaged("does not belong to its restart"));
                }
                let ordered = table.chunks.windows(2).all(|pair| pair[0] < pair[1]);
                if !ordered
                    || table
                        .chunks
                        .last()
                        .is_some_and(|&last| last > restart.ordinal)
                {
                    return Err(table
                        .extent
                        .damaged("names chunks its restart cannot reference"));
                }
            }
        }
        Ok(())
    }
}

/// What every chain of the recording starts from: what no revision changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    /// BLAKE3 over each table's column kinds, in table order.
    pub schema: [u8; 32],
    pub campaign: Vec<u8>,
    /// The content and mod-stack identity, in its recorded form.
    pub content: Vec<u8>,
    pub capture_mode: u8,
}

impl Identity {
    /// The bytes chains start from ([`chain_start`]).
    pub fn chain_bytes(&self) -> io::Result<Vec<u8>> {
        let mut writer = Writer::new();
        writer.varint(u64::from(VERSION))?;
        writer.raw(&self.schema)?;
        writer.varint(self.campaign.len() as u64)?;
        writer.raw(&self.campaign)?;
        writer.varint(self.content.len() as u64)?;
        writer.raw(&self.content)?;
        writer.u8(self.capture_mode)?;
        Ok(writer.into_bytes())
    }
}

/// One run: its place in the campaign and its pages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunDirectory {
    pub branch: u64,
    /// Why the run starts ([`epoch`]).
    pub epoch: u8,
    pub first_hour: i32,
    pub ordinals: u32,
    /// The chain after the run's last ordinal.
    pub chain: [u8; 32],
    /// Every unit position's page, in order.
    pub pages: Vec<Extent>,
}

impl RunDirectory {
    pub fn write(&self, run: u32) -> io::Result<Vec<u8>> {
        self.validate(run)?;
        let mut writer = Writer::new();
        writer.varint(self.branch)?;
        writer.u8(self.epoch)?;
        writer.varint(zigzag(self.first_hour))?;
        writer.varint(u64::from(self.ordinals))?;
        writer.raw(&self.chain)?;
        writer.varint(self.pages.len() as u64)?;
        for page in &self.pages {
            page.write(&mut writer)?;
        }
        Ok(writer.into_bytes())
    }

    /// Run `run`'s directory, read through its envelope `directory`.
    pub fn read(bytes: &[u8], directory: &Extent, run: u32) -> io::Result<Self> {
        directory.expect(Kind::RunDirectory, run)?;
        let mut reader = Reader::new(bytes);
        let branch = reader.varint()?;
        let epoch = reader.u8()?;
        let first_hour = unzigzag(reader.varint()?)?;
        let ordinals = reader.varint_u32()?;
        let chain = reader.raw(32)?.try_into().expect("thirty-two bytes");
        let count = bounded(reader.count()?, usize::MAX, bytes, EXTENT_BYTES)?;
        let mut pages = engine_allocation::vec(count)?;
        for _ in 0..count {
            pages.push(Extent::read(&mut reader)?);
        }
        reader.finish()?;
        let read = Self {
            branch,
            epoch,
            first_hour,
            ordinals,
            chain,
            pages,
        };
        read.validate(run)?;
        Ok(read)
    }

    /// Its pages cover its ordinals on the unit grid, in order, the last alone possibly short.
    fn validate(&self, run: u32) -> io::Result<()> {
        let mut next = 0_u64;
        for page in &self.pages {
            page.expect(Kind::IndexPage, run)?;
            let end = u64::from(page.first) + u64::from(page.hours);
            let full = u64::from(page.first) == next
                && page.hours > 0
                && page.hours <= UNIT_ORDINALS
                && (page.hours == UNIT_ORDINALS || end == u64::from(self.ordinals));
            if !full {
                return Err(page.damaged("is not the run's next unit position"));
            }
            next = end;
        }
        if next != u64::from(self.ordinals) {
            return Err(invalid(format!(
                "Run {run}'s pages do not cover its ordinals"
            )));
        }
        Ok(())
    }
}

/// What the footer names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Root {
    pub identity: Identity,
    /// Increases with every published revision of the file; chains never include it.
    pub revision: u64,
    /// Each run's directory, in run order.
    pub runs: Vec<Extent>,
    /// The control records' extents, in the order `replay` reads them.
    pub controls: Vec<Extent>,
}

impl Root {
    pub fn write(&self) -> io::Result<Vec<u8>> {
        self.validate()?;
        let mut writer = Writer::new();
        let identity = self.identity.chain_bytes()?;
        writer.varint(identity.len() as u64)?;
        writer.raw(&identity)?;
        writer.varint(self.revision)?;
        for list in [&self.runs, &self.controls] {
            writer.varint(list.len() as u64)?;
            for extent in list {
                extent.write(&mut writer)?;
            }
        }
        Ok(writer.into_bytes())
    }

    pub fn read(bytes: &[u8]) -> io::Result<Self> {
        let mut reader = Reader::new(bytes);
        let length = reader.count()?;
        let mut identity = Reader::new(reader.raw(length)?);
        if identity.varint()? != u64::from(VERSION) {
            return Err(invalid("The root names another format version"));
        }
        let schema = identity.raw(32)?.try_into().expect("thirty-two bytes");
        let length = identity.count()?;
        let campaign = identity.raw(length)?.to_vec();
        let length = identity.count()?;
        let content = identity.raw(length)?.to_vec();
        let capture_mode = identity.u8()?;
        identity.finish()?;
        let revision = reader.varint()?;
        let mut lists = [Vec::new(), Vec::new()];
        for list in &mut lists {
            let count = bounded(reader.count()?, usize::MAX, bytes, EXTENT_BYTES)?;
            *list = engine_allocation::vec(count)?;
            for _ in 0..count {
                list.push(Extent::read(&mut reader)?);
            }
        }
        reader.finish()?;
        let [runs, controls] = lists;
        let root = Self {
            identity: Identity {
                schema,
                campaign,
                content,
                capture_mode,
            },
            revision,
            runs,
            controls,
        };
        root.validate()?;
        Ok(root)
    }

    /// Run `n`'s directory is the `n`th; controls are controls.
    fn validate(&self) -> io::Result<()> {
        for (at, run) in self.runs.iter().enumerate() {
            run.expect(
                Kind::RunDirectory,
                u32::try_from(at).map_err(|_| invalid("Too many runs"))?,
            )?;
        }
        for controls in &self.controls {
            controls.expect(Kind::Controls, 0)?;
        }
        Ok(())
    }
}

/// The footer of a file whose root was written at `root` with `stored` bytes and `digest`.
pub fn footer(root: u64, stored: u64, digest: &[u8; 16]) -> [u8; FOOTER_BYTES] {
    let mut bytes = [0; FOOTER_BYTES];
    bytes[..8].copy_from_slice(&root.to_le_bytes());
    bytes[8..16].copy_from_slice(&stored.to_le_bytes());
    bytes[16..32].copy_from_slice(digest);
    bytes[32..].copy_from_slice(FOOTER_MAGIC);
    bytes
}

/// The last footer of a version 3 file whose root verifies: its root's envelope, and the
/// file's length through that footer. Anything after it is a torn tail.
pub fn last_root(file: &[u8]) -> io::Result<(Extent, usize)> {
    if file.len() < HEADER_BYTES + FOOTER_BYTES
        || !file.starts_with(MAGIC)
        || file[8..12] != VERSION.to_le_bytes()
    {
        return Err(invalid("Not a version 3 H4COLUMN recording"));
    }
    // A root sits right before its footer. All candidates' roots together are hashed at
    // most twice the file's length: a tail of footer-shaped bytes cannot cost more.
    let mut work = 0_u64;
    let budget = 2 * file.len() as u64;
    let mut end = file.len();
    while end >= HEADER_BYTES + FOOTER_BYTES {
        let Some(at) = file[HEADER_BYTES..end]
            .windows(FOOTER_MAGIC.len())
            .rposition(|window| window == FOOTER_MAGIC)
            .map(|at| at + HEADER_BYTES)
        else {
            break;
        };
        let stop = at + FOOTER_MAGIC.len();
        if stop >= HEADER_BYTES + FOOTER_BYTES {
            let footer = &file[stop - FOOTER_BYTES..stop];
            let root = Extent {
                kind: Kind::Root,
                run: 0,
                stream: 0,
                first: 0,
                hours: 0,
                offset: u64::from_le_bytes(footer[..8].try_into().expect("eight bytes")),
                stored: u64::from_le_bytes(footer[8..16].try_into().expect("eight bytes")),
                raw: 0,
                digest: footer[16..32].try_into().expect("sixteen bytes"),
            };
            let body = &file[..stop];
            let placed = usize::try_from(root.offset)
                .ok()
                .zip(usize::try_from(root.stored).ok())
                .and_then(|(start, length)| Some(start..start.checked_add(length)?))
                .filter(|range| {
                    range.start >= HEADER_BYTES && range.end == body.len() - FOOTER_BYTES
                });
            if let Some(range) = placed {
                work += root.stored;
                if work > budget {
                    return Err(invalid(
                        "Recovering the recording's footer exceeds its work bound",
                    ));
                }
                if digest(&body[range]) == root.digest {
                    return Ok((root, stop));
                }
            }
        }
        end = at + FOOTER_MAGIC.len() - 1;
    }
    Err(invalid("The recording has no footer whose root verifies"))
}
