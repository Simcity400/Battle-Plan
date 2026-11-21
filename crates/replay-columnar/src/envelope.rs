
use crate::{
    invalid,
    leaf::{Reader, Writer},
};
use std::io;

#[cfg(test)]
mod tests;

pub const MAGIC: &[u8; 8] = b"H4COLUMN";
pub const VERSION: u32 = 12;
pub const HEADER_BYTES: usize = 12;
pub const FOOTER_MAGIC: &[u8; 8] = b"H4COLEND";
pub const FOOTER_BYTES: usize = 8 + 8 + 16 + 8;
pub const UNIT_ORDINALS: u32 = 720;
pub const SPAN_BLOCKS: u32 = 16;
pub fn digest(bytes: &[u8]) -> [u8; 16] {
    let hash = blake3::hash(bytes);
    hash.as_bytes()[..16].try_into().expect("sixteen bytes")
}

pub fn schema_digest(schema: &[Vec<crate::state::Kind>]) -> [u8; 32] {
    tables_digest(schema.iter().map(Vec::as_slice))
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Clock = 1,
    Block = 2,
    Restore = 3,
    Summary = 4,
    BlockPage = 5,
    Root = 6,
    RunDirectory = 7,
    Controls = 8,
    Facts = 9,
    Index = 10,
    Data = 11,
    Stored = 12,
    SpanDirectory = 13,
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

pub mod epoch {
    pub const START: u8 = 0;
    pub const LOAD: u8 = 1;
    pub const BRANCH: u8 = 2;
    pub const GAP: u8 = 3;
    pub const WORLD_DISCONTINUITY: u8 = 4;
    pub const CONTINUATION: u8 = 5;
    pub const RECOVERY_RESUME: u8 = 6;
    pub const ADMISSION_REFUSAL: u8 = 7;
    pub const WRITE_FAILURE: u8 = 8;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extent {
    pub kind: Kind,
    pub run: u32,
    pub stream: u32,
    pub first: u32,
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

    pub fn damaged(&self, what: &str) -> io::Error {
        invalid(format!(
            "{:?} extent of run {} stream {} from ordinal {} ({} hours) {what}",
            self.kind, self.run, self.stream, self.first, self.hours
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowPlace {
    pub table: u32,
    pub offset: u64,
    pub length: u32,
    pub digest: [u8; 16],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpanPages {
    pub directory: Extent,
    pub index: Extent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub schema: [u8; 32],
    pub campaign: Vec<u8>,
    pub content: Vec<u8>,
    pub capture_mode: u8,
}

impl Identity {
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

    pub fn read(bytes: &[u8]) -> io::Result<Self> {
        let (version, identity) = Self::read_any(bytes)?;
        if version != u64::from(VERSION) {
            return Err(invalid("The identity names another format version"));
        }
        Ok(identity)
    }

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

pub fn footer(root: u64, stored: u64, digest: &[u8; 16]) -> [u8; FOOTER_BYTES] {
    let mut bytes = [0; FOOTER_BYTES];
    bytes[..8].copy_from_slice(&root.to_le_bytes());
    bytes[8..16].copy_from_slice(&stored.to_le_bytes());
    bytes[16..32].copy_from_slice(digest);
    bytes[32..].copy_from_slice(FOOTER_MAGIC);
    bytes
}

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

pub fn last_root(file: &[u8]) -> io::Result<(Extent, usize)> {
    crate::container::Footers::new(file)?
        .next()?
        .map(|(root, end)| (root, end as usize))
        .ok_or_else(|| invalid("The recording has no footer whose root verifies"))
}
