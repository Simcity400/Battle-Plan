//! A columnar recording's saved-file envelope (`H4COLUMN` version 4).
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
//! [`UNIT_ORDINALS`]-ordinal blocks, each written once and never revised: one [`IndexPage`]
//! per block names its hour records ([`crate::records`]) and each table's block
//! ([`crate::block`]) for the tables with hours in it, the restart that may end it (one extent
//! per table and the run's dictionary chunk, under one ordinal barrier), which tables the block
//! touched, and the chain after it. A table's block may be stored as a change from its block
//! of the page before ([`IndexPage::chained`]), never past a restart, which walks start from.
//! A restart's table is stored as a change from the run's
//! previous restart's ([`RestartTable::base`]), every [`KEYFRAME_RESTARTS`]th whole. Dictionary
//! ids are run-local. Each run's pages are listed by its own [`RunDirectory`] extent and the
//! recording's control records by [`Kind::Controls`] extents, so a revision rewrites only the
//! active run's directory and a small root.
//!
//! The chain covers each block's extents by their digests ([`chain_block`]).

use crate::{
    invalid,
    leaf::{Reader, Writer},
    state::Bits,
};
use std::io;

#[cfg(test)]
mod tests;

pub const MAGIC: &[u8; 8] = b"H4COLUMN";
/// 5: each table's hours stored once per block of [`UNIT_ORDINALS`]; 4: restarts hold their
/// kept tables' last complete reads ([`crate::restart::Last`]).
pub const VERSION: u32 = 5;
pub const HEADER_BYTES: usize = 12;
pub const FOOTER_MAGIC: &[u8; 8] = b"H4COLEND";
/// Root offset, root stored length, root digest and the footer magic.
pub const FOOTER_BYTES: usize = 8 + 8 + 16 + 8;
/// Recorded ordinals per block; only a run's last block may hold fewer.
pub const UNIT_ORDINALS: u32 = 720;
/// Blocks from a run's start or restart to its next restart.
pub const RESTART_BLOCKS: u32 = 3;
/// Every how many of a run's restarts one is stored whole; the others are changes from the
/// restart before them.
pub const KEYFRAME_RESTARTS: u32 = 8;
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
    tables_digest(schema.iter().map(Vec::as_slice))
}

/// The digest of `schema` were each table only its first `columns[table]` columns; none when
/// `columns` names other tables or more columns.
pub fn schema_digest_prefix(
    schema: &[Vec<crate::state::Kind>],
    columns: &[usize],
) -> Option<[u8; 32]> {
    let held = schema.len() == columns.len()
        && schema.iter().zip(columns).all(|(kinds, &held)| held <= kinds.len());
    held.then(|| {
        tables_digest(
            schema
                .iter()
                .zip(columns)
                .map(|(kinds, &held)| &kinds[..held]),
        )
    })
}

fn tables_digest<'k>(tables: impl Iterator<Item = &'k [crate::state::Kind]>) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    for table in tables {
        hasher.update(&(table.len() as u64).to_le_bytes());
        for &kind in table {
            hasher.update(&[kind as u8]);
        }
    }
    *hasher.finalize().as_bytes()
}

/// The chain value after one block: the value before it, then its hour records' and each of
/// its tables' extents, by digest, in table order.
pub fn chain_block(before: &[u8; 32], records: &Extent, tables: &[Extent]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(before);
    for extent in std::iter::once(records).chain(tables) {
        hasher.update(&extent.stream.to_le_bytes());
        hasher.update(&extent.stored.to_le_bytes());
        hasher.update(&extent.digest);
    }
    *hasher.finalize().as_bytes()
}

/// What an extent holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A block's hour records ([`crate::records`]).
    Records = 1,
    /// One table's hours of one block ([`crate::block`]).
    Block = 2,
    /// The run-local strings first used since the run's previous restart.
    DictionaryChunk = 3,
    /// One table's state at a restart.
    RestartTable = 4,
    IndexPage = 5,
    Root = 6,
    /// One run's place in the campaign and its pages ([`RunDirectory`]).
    RunDirectory = 7,
    /// Control records, opaque here and read by `replay`; each revision appends one.
    Controls = 8,
}

impl Kind {
    fn read(byte: u8) -> io::Result<Self> {
        Ok(match byte {
            1 => Self::Records,
            2 => Self::Block,
            3 => Self::DictionaryChunk,
            4 => Self::RestartTable,
            5 => Self::IndexPage,
            6 => Self::Root,
            7 => Self::RunDirectory,
            8 => Self::Controls,
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
    /// The row recording's write of the hours before failed; the writer started again.
    pub const WRITE_FAILURE: u8 = 8;
}

/// Where one extent is and what it holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extent {
    pub kind: Kind,
    pub run: u32,
    /// A block's or restart's table; else 0.
    pub stream: u32,
    /// A block's, its records' or page's first ordinal; a restart's barrier (the last ordinal
    /// it holds).
    pub first: u32,
    /// Ordinals a block, its records or page cover; 0 otherwise.
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
    /// Its bytes are zstd with the run's previous restart's raw bytes of the same table as
    /// their dictionary: the restart holds only what changed since.
    pub base: bool,
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

/// One block of a run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexPage {
    pub run: u32,
    pub first: u32,
    pub hours: u32,
    pub first_hour: i32,
    pub last_hour: i32,
    pub records: Extent,
    /// The block of each table with hours in it, ascending by table.
    pub tables: Vec<Extent>,
    /// The tables whose block is zstd with their block of the page before, raw, as its
    /// dictionary; never in a run's first page or the page after a restart.
    pub chained: Bits,
    pub restart: Option<RestartDirectory>,
    /// Tables some hour of the position has a section for.
    pub sectioned: Bits,
    /// Tables some hour of the position read.
    pub read: Bits,
    /// Tables whose status changed within the position or at its first ordinal.
    pub status_changed: Bits,
    /// The chain after the block ([`chain_block`]).
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
        self.records.write(&mut writer)?;
        writer.varint(self.tables.len() as u64)?;
        for table in &self.tables {
            table.write(&mut writer)?;
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
                    writer.u8(u8::from(table.base))?;
                    writer.varint(table.chunks.len() as u64)?;
                    for &chunk in &table.chunks {
                        writer.varint(u64::from(chunk))?;
                    }
                }
            }
        }
        write_bits(&mut writer, &self.chained)?;
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
        let records = Extent::read(&mut reader)?;
        let count = bounded(reader.count()?, tables, bytes, EXTENT_BYTES)?;
        let mut blocks = engine_allocation::vec(count)?;
        for _ in 0..count {
            blocks.push(Extent::read(&mut reader)?);
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
                    let base = match reader.u8()? {
                        0 => false,
                        1 => true,
                        _ => return Err(invalid("A restart table's base marker is malformed")),
                    };
                    let limit = ordinal as usize + 1;
                    let count = bounded(reader.count()?, limit, bytes, 1)?;
                    let mut chunks = engine_allocation::vec(count)?;
                    for _ in 0..count {
                        chunks.push(reader.varint_u32()?);
                    }
                    restored.push(RestartTable {
                        table,
                        extent,
                        base,
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
        let chained = read_bits(&mut reader, tables)?;
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
            records,
            tables: blocks,
            chained,
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

    /// Table `table`'s block, when it has hours in this one.
    pub fn table(&self, table: u32) -> Option<&Extent> {
        self.tables
            .binary_search_by_key(&table, |extent| extent.stream)
            .ok()
            .map(|at| &self.tables[at])
    }

    /// A block of the run, its records and tables' blocks covering it whole, the tables in
    /// order; a restart ending it holds every table's state once, in order, changes only from
    /// a restart before it.
    fn validate(&self, tables: usize) -> io::Result<()> {
        if self.hours == 0 || self.hours > UNIT_ORDINALS || self.first % UNIT_ORDINALS != 0 {
            return Err(invalid("A page does not cover a block"));
        }
        // Ordinal arithmetic in u64: untrusted values never overflow.
        let end = u64::from(self.first) + u64::from(self.hours);
        if end > u64::from(u32::MAX) + 1 {
            return Err(invalid("A page ends past the last ordinal"));
        }
        self.records.expect(Kind::Records, self.run)?;
        if (self.records.stream, self.records.first, self.records.hours) != (0, self.first, self.hours) {
            return Err(self.records.damaged("does not cover its page's block"));
        }
        let mut previous: Option<u32> = None;
        for block in &self.tables {
            block.expect(Kind::Block, self.run)?;
            let placed = block.first == self.first
                && block.hours == self.hours
                && (block.stream as usize) < tables
                && previous.is_none_or(|before| before < block.stream);
            if !placed {
                return Err(block.damaged("does not belong to its page's block"));
            }
            previous = Some(block.stream);
        }
        if self.chained.ones().any(|table| self.table(table).is_none()) {
            return Err(invalid("A page chains a table it holds no block of"));
        }
        if self.first == 0 && self.chained.count() != 0 {
            return Err(invalid("A run's first page chains a block"));
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
                if table.base && restart.ordinal % KEYFRAME_RESTARTS == 0 {
                    return Err(table.extent.damaged("is a change in a restart stored whole"));
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

    /// The identity [`Self::chain_bytes`] wrote.
    pub fn read(bytes: &[u8]) -> io::Result<Self> {
        let mut identity = Reader::new(bytes);
        if identity.varint()? != u64::from(VERSION) {
            return Err(invalid("The identity names another format version"));
        }
        let schema = identity.raw(32)?.try_into().expect("thirty-two bytes");
        let length = identity.count()?;
        let campaign = identity.raw(length)?.to_vec();
        let length = identity.count()?;
        let content = identity.raw(length)?.to_vec();
        let capture_mode = identity.u8()?;
        identity.finish()?;
        Ok(Self {
            schema,
            campaign,
            content,
            capture_mode,
        })
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
        let identity = Identity::read(reader.raw(length)?)?;
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
            identity,
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

/// Refuses a file whose header is not this version's, naming the version it holds: earlier
/// versions have no upgrader.
pub fn check_header(header: &[u8]) -> io::Result<()> {
    if header.len() < HEADER_BYTES || &header[..8] != MAGIC {
        return Err(invalid("Not an H4COLUMN recording"));
    }
    let version = u32::from_le_bytes(header[8..12].try_into().expect("four bytes"));
    match version.cmp(&VERSION) {
        std::cmp::Ordering::Equal => Ok(()),
        std::cmp::Ordering::Less => Err(invalid(format!(
            "An H4COLUMN version {version} recording, older than this build's version {VERSION};              this build reads version {VERSION} only and has no upgrader for it"
        ))),
        std::cmp::Ordering::Greater => Err(invalid(format!(
            "An H4COLUMN version {version} recording, newer than this build's version {VERSION}"
        ))),
    }
}

/// The last footer of a file of this version whose root verifies: its root's envelope, and
/// the file's length through that footer. Anything after it is a torn tail.
pub fn last_root(file: &[u8]) -> io::Result<(Extent, usize)> {
    check_header(file)?;
    if file.len() < HEADER_BYTES + FOOTER_BYTES {
        return Err(invalid("An H4COLUMN recording holds no footer"));
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
