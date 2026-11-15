//! A recording's extents (`H4COLUMN` version 12): one campaign, every session in it, in
//! sections ([`crate::container`]).
//!
//! Every extent is described where it is referenced, by an [`Extent`] envelope carrying a
//! digest of its stored bytes, so a reader checks an extent's place and length before allocating
//! and its bytes before decoding. The file's header, footer and root are [`crate::container`]'s;
//! the world log's pages are [`crate::pages`]'.

use crate::{
    invalid,
    leaf::{Reader, Writer},
};
use std::io;

#[cfg(test)]
mod tests;

pub const MAGIC: &[u8; 8] = b"H4COLUMN";
/// 12: each table's unit of a block its parts of per-column streams entity by entity
/// ([`crate::unit`]), restores per column by replay work, the world log's pages
/// [`crate::pages`]; 11: each table a lane, with its own presence, statuses, reads, sections and
/// strings per block, a clock unit per block, span pages and summaries; 10: each root says whether
/// its revision is saved ([`crate::container::Saved`]).
pub const VERSION: u32 = 12;
pub const HEADER_BYTES: usize = 12;
pub const FOOTER_MAGIC: &[u8; 8] = b"H4COLEND";
/// Root offset, root stored length, root digest and the footer magic.
pub const FOOTER_BYTES: usize = 8 + 8 + 16 + 8;
/// Recorded ordinals per block; only a run's last block may hold fewer.
pub const UNIT_ORDINALS: u32 = 720;
/// Blocks a span page indexes.
pub const SPAN_BLOCKS: u32 = 16;
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

/// What an extent holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A block's per-hour flags and metrics ([`crate::lane::Clock`]).
    Clock = 1,
    /// One table's unit of one block ([`crate::lane`]).
    Block = 2,
    /// One table's state at a block's end ([`crate::restart`]).
    Restore = 3,
    /// One table's summary piece of one level ([`crate::summary`]).
    Summary = 4,
    BlockPage = 5,
    Root = 6,
    /// One run's place in the campaign and its pages ([`RunDirectory`]).
    RunDirectory = 7,
    /// Campaign records, opaque here and read by `replay`; each revision appends one.
    Controls = 8,
    /// One pack of the game facts the recording carries, opaque here.
    Facts = 9,
    /// A page of a section's index tree ([`crate::container::Frozen`]); its stream is its level.
    Index = 10,
    /// A section owner's bytes as one zstd frame; its other fields are the owner's tags.
    Data = 11,
    /// A section owner's bytes as they are; its other fields are the owner's tags.
    Stored = 12,
    /// One span's directory: its blocks' clocks and each table's row ([`SpanDirectory`]).
    SpanDirectory = 13,
    /// One span's table rows, stored as they are ([`SpanRow`]).
    SpanIndex = 14,
}

impl Kind {
    fn read(byte: u8) -> io::Result<Self> {
        Ok(match byte {
            1 => Self::Clock,
            2 => Self::Block,
            3 => Self::Restore,
            4 => Self::Summary,
            5 => Self::BlockPage,
            6 => Self::Root,
            7 => Self::RunDirectory,
            8 => Self::Controls,
            9 => Self::Facts,
            10 => Self::Index,
            11 => Self::Data,
            12 => Self::Stored,
            13 => Self::SpanDirectory,
            14 => Self::SpanIndex,
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
    /// A unit's, restore's or summary's table; else 0.
    pub stream: u32,
    /// A unit's, clock's or page's first ordinal; a restore's last ordinal; a summary's first.
    pub first: u32,
    /// Ordinals a unit, clock, page or summary covers; 0 otherwise.
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
}

/// Where one table's row lies in a span index, and the digest of its bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowPlace {
    pub table: u32,
    pub offset: u64,
    pub length: u32,
    pub digest: [u8; 16],
}

/// One closed span of a run: its directory and its index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpanPages {
    pub directory: Extent,
    pub index: Extent,
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
        let (version, identity) = Self::read_any(bytes)?;
        if version != u64::from(VERSION) {
            return Err(invalid("The identity names another format version"));
        }
        Ok(identity)
    }

    /// The identity a capture begins with, of whichever recording format its collector wrote: a
    /// capture's compatibility is its row format, and the recording made of it writes its own.
    pub fn read_capture(bytes: &[u8]) -> io::Result<Self> {
        Ok(Self::read_any(bytes)?.1)
    }

    fn read_any(bytes: &[u8]) -> io::Result<(u64, Self)> {
        let mut identity = Reader::new(bytes);
        let version = identity.varint()?;
        let schema = identity.raw(32)?.try_into().expect("thirty-two bytes");
        let length = identity.count()?;
        let campaign = identity.raw(length)?.to_vec();
        let length = identity.count()?;
        let content = identity.raw(length)?.to_vec();
        let capture_mode = identity.u8()?;
        identity.finish()?;
        Ok((
            version,
            Self {
                schema,
                campaign,
                content,
                capture_mode,
            },
        ))
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
            "An H4COLUMN recording of format {version}; this build reads and writes format {VERSION} only and has no upgrader for it"
        ))),
        std::cmp::Ordering::Greater => Err(invalid(format!(
            "An H4COLUMN recording of format {version}, newer than this build's; this build reads and writes format {VERSION}"
        ))),
    }
}

/// The last footer of a file of this version whose root verifies: its root's envelope, and
/// the file's length through that footer. Anything after it is a torn tail.
pub fn last_root(file: &[u8]) -> io::Result<(Extent, usize)> {
    crate::container::Footers::new(file)?
        .next()?
        .map(|(root, end)| (root, end as usize))
        .ok_or_else(|| invalid("The recording has no footer whose root verifies"))
}
