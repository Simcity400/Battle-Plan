//! A recording file of versioned sections (`H4COLUMN` version [`envelope::VERSION`]).
//!
//! The file is append-only: extents in any order, then a [`Root`] extent, then a fixed
//! [`FOOTER_BYTES`] footer naming it. A revision appends its extents and syncs, writes its
//! resume state to a slot beside the file ([`state_file`]) and syncs that, appends the root and
//! syncs, then the footer, and syncs. A reader takes the last footer whose root verifies
//! ([`Footers`]); a writer taking the file up after a crash takes the newest root whose resume
//! state verifies ([`recover`]) and cuts what follows it.
//!
//! The root names every [`Section`]: its kind, its id, its schema version and its extents.
//! Readers dispatch on kind and version and keep only this build's decoders: a section of
//! another version is refused by name ([`Section::expect`]), and an unknown kind refuses the
//! root. A section's extents are its frozen ones, appended once and paged by an index tree of
//! [`FANOUT`] extents a page ([`Frozen`]), then its open ones, which every revision may replace.

use crate::{
    envelope::{self, Extent, FOOTER_BYTES, FOOTER_MAGIC, HEADER_BYTES, Identity, Kind},
    invalid,
    leaf::{Reader, Writer},
};
use engine_allocation::zstd::{self, Compressor, Decompressor, Holder};
use std::{
    fmt,
    io::{self, Seek, SeekFrom, Write},
};

pub mod catalogue;
pub mod restore_points;
pub mod state_file;
#[cfg(test)]
mod tests;

pub use state_file::StateFile;

/// The schema versions of the sections this build reads and writes itself.
pub mod schema {
    /// H4COLUMN version 8's runs, pages, blocks and restarts, unchanged.
    pub const WORLD_LOG: u32 = 8;
    pub const RECORDS: u32 = 1;
    pub const FACTS: u32 = 1;
}

/// What a section holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SectionKind {
    /// Every run's hours: one [`Kind::RunDirectory`] extent per run, in run order.
    WorldLog,
    /// The campaign's records ([`Kind::Controls`]), opaque here.
    Records,
    /// The game facts packs ([`Kind::Facts`]), oldest first.
    Facts,
    /// Map restore points, each the owner's payload, opaque here ([`restore_points`]).
    MapRestore,
    /// A family's answers, opaque here: its id is the family's.
    FamilyAnswers,
    /// An id map ([`catalogue`]).
    Catalogue,
    /// A family's state once the recording finished, opaque here: its id is the family's.
    FinalState,
}

impl SectionKind {
    pub const fn code(self) -> u16 {
        match self {
            Self::WorldLog => 1,
            Self::Records => 2,
            Self::Facts => 3,
            Self::MapRestore => 4,
            Self::FamilyAnswers => 5,
            Self::Catalogue => 6,
            Self::FinalState => 7,
        }
    }

    pub fn from_code(code: u64) -> io::Result<Self> {
        Ok(match code {
            1 => Self::WorldLog,
            2 => Self::Records,
            3 => Self::Facts,
            4 => Self::MapRestore,
            5 => Self::FamilyAnswers,
            6 => Self::Catalogue,
            7 => Self::FinalState,
            _ => {
                return Err(invalid(format!(
                    "The recording holds a section of kind {code}, which this build does not know and has no decoder for"
                )));
            }
        })
    }

    /// Whether its sections' own extents may be of `kind`.
    pub fn holds(self, kind: Kind) -> bool {
        match self {
            Self::WorldLog => kind == Kind::RunDirectory,
            Self::Records => kind == Kind::Controls,
            Self::Facts => kind == Kind::Facts,
            Self::MapRestore | Self::FamilyAnswers | Self::Catalogue | Self::FinalState => {
                matches!(kind, Kind::Data | Kind::Stored)
            }
        }
    }

    /// Whether it is one per file, of id 0.
    const fn single(self) -> bool {
        matches!(self, Self::WorldLog | Self::Records | Self::Facts)
    }
}

/// A section's name in its file: what holds it and whose it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SectionKey {
    pub kind: SectionKind,
    /// A family's or catalogue's id; 0 for the sections a file holds one of.
    pub id: u32,
}

impl SectionKey {
    pub const WORLD_LOG: Self = Self::single(SectionKind::WorldLog);
    pub const RECORDS: Self = Self::single(SectionKind::Records);
    pub const FACTS: Self = Self::single(SectionKind::Facts);

    const fn single(kind: SectionKind) -> Self {
        Self { kind, id: 0 }
    }

    pub const fn new(kind: SectionKind, id: u32) -> Self {
        Self { kind, id }
    }

    fn check(self) -> io::Result<()> {
        if self.kind.single() && self.id != 0 {
            return Err(invalid(format!("{self} has an id, which its kind has not")));
        }
        Ok(())
    }
}

impl fmt::Display for SectionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            SectionKind::WorldLog => write!(f, "The world log"),
            SectionKind::Records => write!(f, "The campaign records section"),
            SectionKind::Facts => write!(f, "The game facts section"),
            SectionKind::MapRestore => write!(f, "Map restore section {}", self.id),
            SectionKind::FamilyAnswers => write!(f, "Family {}'s answers section", self.id),
            SectionKind::Catalogue => write!(f, "Catalogue {}", self.id),
            SectionKind::FinalState => write!(f, "Family {}'s final state section", self.id),
        }
    }
}

/// Extents an index page holds, and how many a root holds of each level.
pub const FANOUT: usize = 32;
/// The most levels a section's index tree takes: 32^12 extents is 2^60.
const LEVELS: usize = 12;
/// The most raw bytes an index page takes: its level and count, then [`FANOUT`] envelopes of
/// their kind, four u32 and three u64 varints and a digest each.
pub const PAGE_RAW_BYTES: u64 = 2 + FANOUT as u64 * (1 + 4 * 5 + 3 * 10 + 16);

/// A section's frozen extents: level 0 holds extents of the section, each level `k` above it
/// [`Kind::Index`] pages each holding [`FANOUT`] of level `k - 1`, every level fewer than
/// [`FANOUT`]. In order, the highest level's first; in that order the section's extents lie in
/// the file one after another ([`check_follows`]), so reading them all is bounded by the file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frozen {
    levels: Vec<Vec<Extent>>,
}

/// Where a frozen position lies.
pub enum Place<'a> {
    /// In the root: the extent at it.
    Extent(&'a Extent),
    /// In a page of `level`, at `at` of the positions it covers.
    Page { page: &'a Extent, level: u32, at: u64 },
}

/// The positions one entry of `level` covers.
pub const fn span(level: u32) -> u64 {
    (FANOUT as u64).pow(level)
}

impl Frozen {
    pub fn len(&self) -> u64 {
        (0..)
            .zip(&self.levels)
            .map(|(level, held)| held.len() as u64 * span(level))
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        self.levels.iter().all(Vec::is_empty)
    }

    /// Each level's extents, from level 0.
    pub fn levels(&self) -> &[Vec<Extent>] {
        &self.levels
    }

    /// Where position `at` lies.
    pub fn locate(&self, at: u64) -> Option<Place<'_>> {
        let mut start = 0_u64;
        for (level, held) in self.levels.iter().enumerate().rev() {
            let level = level as u32;
            let covered = held.len() as u64 * span(level);
            if at < start + covered {
                let within = at - start;
                let entry = &held[(within / span(level)) as usize];
                return Some(match level {
                    0 => Place::Extent(entry),
                    _ => Place::Page {
                        page: entry,
                        level,
                        at: within % span(level),
                    },
                });
            }
            start += covered;
        }
        None
    }

    /// Whether `extent`, appended next, lies past every extent it holds, pages included.
    fn follows(&self, extent: &Extent) -> bool {
        self.levels
            .iter()
            .flatten()
            .all(|held| extent.offset > held.offset && extent.offset >= held.offset + held.stored)
    }

    /// Appends `extent`; each level it fills goes into a page `page` writes, of the level above.
    fn push(&mut self, extent: Extent, page: &mut dyn FnMut(u32, &[Extent]) -> io::Result<Extent>) -> io::Result<()> {
        let mut carry = extent;
        for level in 0..LEVELS {
            if self.levels.len() == level {
                engine_allocation::reserve(&mut self.levels, 1)?;
                self.levels.push(engine_allocation::vec(FANOUT)?);
            }
            engine_allocation::reserve(&mut self.levels[level], 1)?;
            self.levels[level].push(carry);
            if self.levels[level].len() < FANOUT {
                return Ok(());
            }
            carry = page(level as u32 + 1, &self.levels[level])?;
            self.levels[level].clear();
        }
        Err(invalid("A section passes its index tree's levels"))
    }

    fn validate(&self, key: SectionKey) -> io::Result<()> {
        if self.levels.len() > LEVELS || self.levels.last().is_some_and(Vec::is_empty) {
            return Err(invalid(format!("{key}'s index levels are malformed")));
        }
        for (level, held) in (0_u32..).zip(&self.levels) {
            if held.len() >= FANOUT {
                return Err(invalid(format!("{key} holds a whole level unpaged")));
            }
            for extent in held {
                check_entry(key, level, extent)?;
            }
        }
        Ok(())
    }
}

/// Refuses `next`, a frozen extent of `key` read after `previous`, unless it starts past the
/// start and end of `previous`, and ends within `length`, the file's committed bytes: frozen
/// extents are appended in the order they are written, so a section names no more of them than
/// its file holds bytes.
pub fn check_follows(key: SectionKey, previous: Option<&Extent>, next: &Extent, length: u64) -> io::Result<()> {
    let after = previous.is_none_or(|previous| {
        next.offset > previous.offset && next.offset >= previous.offset.saturating_add(previous.stored)
    });
    let within = next.offset.checked_add(next.stored).is_some_and(|end| end <= length);
    if !after || !within {
        return Err(next.damaged(&format!("does not follow the extent of {key} before it in the file")));
    }
    Ok(())
}

/// Refuses `extent` unless it may be an entry of `key`'s level `level`.
fn check_entry(key: SectionKey, level: u32, extent: &Extent) -> io::Result<()> {
    let fits = match level {
        0 => key.kind.holds(extent.kind),
        _ => extent.kind == Kind::Index && extent.stream == level && extent.raw <= PAGE_RAW_BYTES,
    };
    if !fits {
        return Err(extent.damaged(&format!("is no level {level} entry of {key}")));
    }
    Ok(())
}

/// The entries of `page`, an index page of `key` of the level its envelope names, from its raw
/// bytes: every one checked to be of the level below.
pub fn read_page(raw: &[u8], page: &Extent, key: SectionKey) -> io::Result<Vec<Extent>> {
    if page.kind != Kind::Index || page.stream == 0 || page.stream as usize > LEVELS {
        return Err(page.damaged(&format!("is no index page of {key}")));
    }
    let mut reader = Reader::new(raw);
    if reader.varint()? != u64::from(page.stream) || reader.count()? != FANOUT {
        return Err(page.damaged("holds another level or count than its envelope's"));
    }
    let mut entries = engine_allocation::vec(FANOUT)?;
    for _ in 0..FANOUT {
        let entry = Extent::read(&mut reader)?;
        check_entry(key, page.stream - 1, &entry)?;
        entries.push(entry);
    }
    reader.finish()?;
    Ok(entries)
}

fn write_page(level: u32, entries: &[Extent]) -> io::Result<Vec<u8>> {
    let mut writer = Writer::new();
    writer.varint(u64::from(level))?;
    writer.varint(entries.len() as u64)?;
    for entry in entries {
        entry.write(&mut writer)?;
    }
    Ok(writer.into_bytes())
}

/// One section: its name, schema version and extents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub key: SectionKey,
    pub version: u32,
    pub frozen: Frozen,
    /// Its last extents, which the next revision may replace.
    pub open: Vec<Extent>,
}

impl Section {
    pub fn len(&self) -> u64 {
        self.frozen.len() + self.open.len() as u64
    }

    pub fn is_empty(&self) -> bool {
        self.frozen.is_empty() && self.open.is_empty()
    }

    /// Refuses a section of another schema version than `version`, naming both: a build keeps
    /// only its own version's decoder and has no upgrader for another's.
    pub fn expect(&self, version: u32) -> io::Result<()> {
        let held = self.version;
        match held.cmp(&version) {
            std::cmp::Ordering::Equal => Ok(()),
            std::cmp::Ordering::Less => Err(invalid(format!(
                "{} is schema version {held}, older than this build's version {version}; this build reads version {version} only and has no upgrader for it",
                self.key
            ))),
            std::cmp::Ordering::Greater => Err(invalid(format!(
                "{} is schema version {held}, newer than this build's version {version}",
                self.key
            ))),
        }
    }

    fn validate(&self) -> io::Result<()> {
        self.key.check()?;
        self.frozen.validate(self.key)?;
        let first = self.frozen.len();
        for (at, extent) in (first..).zip(&self.open) {
            check_entry(self.key, 0, extent)?;
            check_owner(self.key, at, extent)?;
        }
        Ok(())
    }
}

/// Refuses `extent` at position `at` of `key` unless its envelope names that place.
pub fn check_owner(key: SectionKey, at: u64, extent: &Extent) -> io::Result<()> {
    let placed = match key.kind {
        SectionKind::WorldLog => u64::from(extent.run) == at,
        SectionKind::Records | SectionKind::Facts => extent.run == 0,
        _ => true,
    };
    if !placed {
        return Err(extent.damaged(&format!("is not entry {at} of {key}")));
    }
    Ok(())
}

/// The resume state slot a root was written with ([`state_file`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlotRef {
    pub sequence: u64,
    /// Of the slot's stored payload.
    pub digest: [u8; 16],
}

/// What the footer names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Root {
    pub identity: Identity,
    /// Increases with every published revision of the file; chains never include it.
    pub revision: u64,
    /// The resume state slot written with it; none once the file is finished.
    pub state: Option<SlotRef>,
    /// Every section, ascending by key.
    pub sections: Vec<Section>,
}

impl Root {
    pub fn write(&self) -> io::Result<Vec<u8>> {
        write_root(&self.identity, self.revision, self.state, &self.sections)
    }

    pub fn read(bytes: &[u8]) -> io::Result<Self> {
        let mut reader = Reader::new(bytes);
        let length = reader.count()?;
        let identity = Identity::read(reader.raw(length)?)?;
        let revision = reader.varint()?;
        let state = match reader.u8()? {
            0 => None,
            1 => Some(SlotRef {
                sequence: reader.varint()?,
                digest: reader.raw(16)?.try_into().expect("sixteen bytes"),
            }),
            _ => return Err(invalid("The root's state marker is malformed")),
        };
        let count = bounded(reader.count()?, bytes, 3)?;
        let mut sections = engine_allocation::vec(count)?;
        for _ in 0..count {
            let kind = SectionKind::from_code(reader.varint()?)?;
            let key = SectionKey {
                kind,
                id: reader.varint_u32()?,
            };
            let version = reader.varint_u32()?;
            let levels = reader.count()?;
            if levels > LEVELS {
                return Err(invalid(format!("{key}'s index levels are malformed")));
            }
            let mut frozen = Frozen {
                levels: engine_allocation::vec(levels)?,
            };
            for _ in 0..levels {
                frozen.levels.push(read_extents(&mut reader, bytes)?);
            }
            let open = read_extents(&mut reader, bytes)?;
            sections.push(Section {
                key,
                version,
                frozen,
                open,
            });
        }
        reader.finish()?;
        let root = Self {
            identity,
            revision,
            state,
            sections,
        };
        validate_sections(&root.sections)?;
        Ok(root)
    }

    pub fn section(&self, key: SectionKey) -> Option<&Section> {
        find(&self.sections, key).ok().map(|at| &self.sections[at])
    }

    /// `key`'s section, refused by name when it is not of `version`; none when it holds none.
    pub fn open(&self, key: SectionKey, version: u32) -> io::Result<Option<&Section>> {
        let section = self.section(key);
        if let Some(section) = section {
            section.expect(version)?;
        }
        Ok(section)
    }
}

/// The fewest bytes an [`Extent`] envelope encodes in.
const EXTENT_BYTES: usize = 1 + 7 + 16;

fn bounded(count: usize, bytes: &[u8], entry_bytes: usize) -> io::Result<usize> {
    if count > bytes.len() / entry_bytes {
        return Err(invalid("A count exceeds what its bytes can hold"));
    }
    Ok(count)
}

fn read_extents(reader: &mut Reader<'_>, bytes: &[u8]) -> io::Result<Vec<Extent>> {
    let count = bounded(reader.count()?, bytes, EXTENT_BYTES)?;
    let mut extents = engine_allocation::vec(count)?;
    for _ in 0..count {
        extents.push(Extent::read(reader)?);
    }
    Ok(extents)
}

fn write_extents(writer: &mut Writer, extents: &[Extent]) -> io::Result<()> {
    writer.varint(extents.len() as u64)?;
    for extent in extents {
        extent.write(writer)?;
    }
    Ok(())
}

fn write_root(identity: &Identity, revision: u64, state: Option<SlotRef>, sections: &[Section]) -> io::Result<Vec<u8>> {
    validate_sections(sections)?;
    let mut writer = Writer::new();
    let identity = identity.chain_bytes()?;
    writer.varint(identity.len() as u64)?;
    writer.raw(&identity)?;
    writer.varint(revision)?;
    match state {
        None => writer.u8(0)?,
        Some(slot) => {
            writer.u8(1)?;
            writer.varint(slot.sequence)?;
            writer.raw(&slot.digest)?;
        }
    }
    writer.varint(sections.len() as u64)?;
    for section in sections {
        writer.varint(u64::from(section.key.kind.code()))?;
        writer.varint(u64::from(section.key.id))?;
        writer.varint(u64::from(section.version))?;
        writer.varint(section.frozen.levels.len() as u64)?;
        for level in &section.frozen.levels {
            write_extents(&mut writer, level)?;
        }
        write_extents(&mut writer, &section.open)?;
    }
    Ok(writer.into_bytes())
}

fn validate_sections(sections: &[Section]) -> io::Result<()> {
    if sections.windows(2).any(|pair| pair[0].key >= pair[1].key) {
        return Err(invalid("The root's sections are not each named once, in order"));
    }
    sections.iter().try_for_each(Section::validate)
}

fn find(sections: &[Section], key: SectionKey) -> Result<usize, usize> {
    sections.binary_search_by_key(&key, |section| section.key)
}

/// One published revision: its number, the file's length through its footer and its root's
/// digest. A file grows only by later revisions, so one holding this length and root holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Revision {
    pub number: u64,
    pub length: u64,
    pub root: [u8; 16],
}

impl Revision {
    /// A digest naming the revision, for records kept beside a recording.
    pub fn digest(self) -> u64 {
        u64::from_le_bytes(self.root[..8].try_into().expect("eight bytes"))
    }
}

/// One owner's resume state: what it needs to go on after the revision it was written with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateEntry {
    /// The section whose writer it is.
    pub owner: SectionKey,
    /// The owner's schema version of these bytes.
    pub version: u32,
    pub bytes: Vec<u8>,
}

impl StateEntry {
    /// Refuses an entry of another schema version than `version`, naming both.
    pub fn expect(&self, version: u32) -> io::Result<()> {
        if self.version != version {
            return Err(invalid(format!(
                "The resume state of {} is schema version {}; this build reads version {version} only",
                self.owner, self.version
            )));
        }
        Ok(())
    }
}

/// `entries` as a state slot's payload holds them, each owner once.
pub fn write_entries(entries: &[StateEntry]) -> io::Result<Vec<u8>> {
    let mut writer = Writer::new();
    writer.varint(entries.len() as u64)?;
    for entry in entries {
        writer.varint(u64::from(entry.owner.kind.code()))?;
        writer.varint(u64::from(entry.owner.id))?;
        writer.varint(u64::from(entry.version))?;
        writer.varint(entry.bytes.len() as u64)?;
        writer.raw(&entry.bytes)?;
    }
    Ok(writer.into_bytes())
}

pub fn read_entries(bytes: &[u8]) -> io::Result<Vec<StateEntry>> {
    let mut reader = Reader::new(bytes);
    let count = bounded(reader.count()?, bytes, 4)?;
    let mut entries: Vec<StateEntry> = engine_allocation::vec(count)?;
    for _ in 0..count {
        let owner = SectionKey {
            kind: SectionKind::from_code(reader.varint()?)?,
            id: reader.varint_u32()?,
        };
        let version = reader.varint_u32()?;
        let length = reader.count()?;
        let bytes = engine_allocation::copy_vec(reader.raw(length)?)?;
        if entries.iter().any(|entry| entry.owner == owner) {
            return Err(invalid(format!("The resume state holds {owner}'s twice")));
        }
        entries.push(StateEntry { owner, version, bytes });
    }
    reader.finish()?;
    Ok(entries)
}

/// The tags the owner of a [`Kind::Data`] or [`Kind::Stored`] extent gives it, which the
/// container never reads; the world log's extents keep their envelopes' meaning.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tags {
    pub run: u32,
    pub stream: u32,
    pub first: u32,
    pub hours: u32,
}

static CONTAINER: Holder = Holder::new("zstd: container");

/// The window `raw` bytes are compressed with: what they need, within what every decoder takes.
fn window(raw: usize) -> u32 {
    let needed = raw.max(2);
    (usize::BITS - (needed - 1).leading_zeros()).clamp(10, 27)
}

/// `raw` as one zstd frame at `level`, by `compressor`.
pub fn compress(compressor: &mut Compressor, level: i32, raw: &[u8]) -> io::Result<Vec<u8>> {
    compressor.level(level)?;
    compressor.window_log(window(raw.len()))?;
    let mut stored = engine_allocation::vec(zstd::bound(raw.len()))?;
    compressor
        .compress_append(raw, &mut stored)
        .map_err(zstd::Error::invalid)?;
    Ok(stored)
}

/// The most bytes [`decompress`] grows its output by ahead of what it decoded: a zstd block.
const DECODE_STEP: usize = 128 << 10;

/// `stored`, one zstd frame, decoded, its declared length `raw` when given. The output grows
/// only as the frame fills it, never past its declared length: a small frame declaring more
/// than it holds costs no more than it decodes to.
pub fn decompress(stored: &[u8], raw: Option<u64>) -> io::Result<Vec<u8>> {
    let declared = zstd::content_size(stored).ok_or_else(|| invalid("A frame declares no length"))?;
    if raw.is_some_and(|raw| raw != declared) {
        return Err(invalid("differs from its frame's length"));
    }
    if zstd::frame_size(stored) != Some(stored.len()) {
        return Err(invalid("holds bytes past its frame"));
    }
    let declared = usize::try_from(declared).map_err(|_| invalid("A frame declares more than the address space"))?;
    let mut context = Decompressor::new(&CONTAINER)?;
    let mut out = Vec::new();
    let mut read = 0;
    loop {
        if out.len() == out.capacity() && out.len() < declared {
            let more = out.len().max(DECODE_STEP).min(declared - out.len());
            engine_allocation::reserve_exact(&mut out, more)?;
        }
        let before = (out.len(), read);
        if context.stream(&mut out, stored, &mut read).map_err(zstd::Error::invalid)? {
            break;
        }
        if (out.len(), read) == before {
            return Err(invalid("decompresses to another length"));
        }
    }
    if out.len() != declared || read != stored.len() {
        return Err(invalid("decompresses to another length"));
    }
    Ok(out)
}

/// Bytes read at an offset: a file's, or memory's.
pub trait ReadAt {
    fn len(&self) -> io::Result<u64>;
    /// Fills `out` from `at`.
    fn read_at(&self, at: u64, out: &mut [u8]) -> io::Result<()>;
}

impl ReadAt for [u8] {
    fn len(&self) -> io::Result<u64> {
        Ok(<[u8]>::len(self) as u64)
    }

    fn read_at(&self, at: u64, out: &mut [u8]) -> io::Result<()> {
        let bytes = usize::try_from(at)
            .ok()
            .and_then(|at| self.get(at..)?.get(..out.len()))
            .ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))?;
        out.copy_from_slice(bytes);
        Ok(())
    }
}

impl ReadAt for std::fs::File {
    fn len(&self) -> io::Result<u64> {
        Ok(self.metadata()?.len())
    }

    fn read_at(&self, mut at: u64, mut out: &mut [u8]) -> io::Result<()> {
        while !out.is_empty() {
            #[cfg(windows)]
            let count = std::os::windows::fs::FileExt::seek_read(self, out, at)?;
            #[cfg(unix)]
            let count = std::os::unix::fs::FileExt::read_at(self, out, at)?;
            if count == 0 {
                // The file changed under the reader: no damage of the recording it opened.
                return Err(io::Error::other("The recording's file ends before its committed length"));
            }
            at += count as u64;
            out = &mut out[count..];
        }
        Ok(())
    }
}

/// The blocks a footer is searched for and a root hashed in.
const SCAN: usize = 64 << 10;

/// The digest of `length` bytes of `file` from `at`, as [`envelope::digest`] makes it, read
/// through `block`.
fn digest_at<F: ReadAt + ?Sized>(file: &F, at: u64, length: u64, block: &mut [u8]) -> io::Result<[u8; 16]> {
    let mut hasher = blake3::Hasher::new();
    let mut done = 0;
    while done < length {
        let count = (length - done).min(block.len() as u64) as usize;
        file.read_at(at + done, &mut block[..count])?;
        hasher.update(&block[..count]);
        done += count as u64;
    }
    Ok(hasher.finalize().as_bytes()[..16].try_into().expect("sixteen bytes"))
}

/// A file's footers whose roots verify, from its end back ([`Self::next`]).
pub struct Footers<'f, F: ReadAt + ?Sized> {
    file: &'f F,
    /// The block being searched holds the bytes from `first` to `stop`, its candidates below
    /// `cursor`.
    first: u64,
    stop: u64,
    cursor: usize,
    loaded: bool,
    done: bool,
    work: u64,
    budget: u64,
    block: Vec<u8>,
    hashed: Vec<u8>,
}

impl<'f, F: ReadAt + ?Sized> Footers<'f, F> {
    /// The footers of `file`, a recording of this build's version.
    pub fn new(file: &'f F) -> io::Result<Self> {
        let length = file.len()?;
        let mut start = [0; HEADER_BYTES];
        let held = length.min(HEADER_BYTES as u64) as usize;
        file.read_at(0, &mut start[..held])?;
        envelope::check_header(&start[..held])?;
        if length < (HEADER_BYTES + FOOTER_BYTES) as u64 {
            return Err(invalid("An H4COLUMN recording holds no footer"));
        }
        Ok(Self {
            file,
            first: length,
            stop: length,
            cursor: 0,
            loaded: false,
            done: false,
            work: 0,
            // All candidates' roots together are hashed at most twice the file's length: a tail
            // of footer-shaped bytes cannot cost more.
            budget: 2 * length,
            block: engine_allocation::filled(SCAN, 0)?,
            hashed: engine_allocation::filled(SCAN, 0)?,
        })
    }

    /// The next footer back whose root verifies: its root's envelope, and the file's length
    /// through the footer. None once no earlier one does.
    pub fn next(&mut self) -> io::Result<Option<(Extent, u64)>> {
        let header = HEADER_BYTES as u64;
        let footer = FOOTER_BYTES as u64;
        let magic = FOOTER_MAGIC.len();
        loop {
            if !self.loaded {
                if self.done || self.stop <= header {
                    return Ok(None);
                }
                self.first = self.stop.saturating_sub(SCAN as u64).max(header);
                let count = (self.stop - self.first) as usize;
                self.file.read_at(self.first, &mut self.block[..count])?;
                self.cursor = (count + 1).saturating_sub(magic);
                self.loaded = true;
            }
            while self.cursor > 0 {
                self.cursor -= 1;
                let at = self.cursor;
                if self.block[at..at + magic] != *FOOTER_MAGIC {
                    continue;
                }
                let end = self.first + (at + magic) as u64;
                if end < header + footer {
                    continue;
                }
                let mut bytes = [0; FOOTER_BYTES];
                self.file.read_at(end - footer, &mut bytes)?;
                let root = Extent {
                    kind: Kind::Root,
                    run: 0,
                    stream: 0,
                    first: 0,
                    hours: 0,
                    offset: u64::from_le_bytes(bytes[..8].try_into().expect("eight bytes")),
                    stored: u64::from_le_bytes(bytes[8..16].try_into().expect("eight bytes")),
                    raw: 0,
                    digest: bytes[16..32].try_into().expect("sixteen bytes"),
                };
                if root.offset < header || root.offset.checked_add(root.stored) != Some(end - footer) {
                    continue;
                }
                self.work += root.stored;
                if self.work > self.budget {
                    return Err(invalid("Recovering the recording's footer exceeds its work bound"));
                }
                if digest_at(self.file, root.offset, root.stored, &mut self.hashed)? == root.digest {
                    return Ok(Some((root, end)));
                }
            }
            self.loaded = false;
            if self.first == header {
                self.done = true;
                return Ok(None);
            }
            // Blocks overlap by a magic less one byte, so one across their boundary is seen.
            self.stop = self.first + magic as u64 - 1;
        }
    }
}

/// The root `extent` names in `file`, read and decoded.
pub fn read_root<F: ReadAt + ?Sized>(file: &F, extent: &Extent) -> io::Result<Root> {
    let mut stored = engine_allocation::filled(usize::try_from(extent.stored).map_err(|_| engine_allocation::oom())?, 0)?;
    file.read_at(extent.offset, &mut stored)?;
    if envelope::digest(&stored) != extent.digest {
        return Err(invalid("The recording's root differs from its digest"));
    }
    Root::read(&decompress(&stored, None)?)
}

/// Where a writer takes a file up after a crash ([`recover`]).
pub struct Recovered {
    pub root: Root,
    pub root_extent: Extent,
    /// The file's length through the root's footer: what follows it is cut.
    pub length: u64,
    /// The resume state the root was written with; empty when it names none.
    pub state: Vec<StateEntry>,
    /// The later roots that verify but whose resume state does not.
    pub skipped: u32,
}

/// The newest root of `file` whose resume state slot in `state` verifies, or which names none:
/// a crash before a revision's root leaves the slot it wrote unnamed, and the root before it
/// names the other slot, which that write never touches.
pub fn recover<F: ReadAt + ?Sized>(file: &F, state: Option<&StateFile>) -> io::Result<Recovered> {
    let mut footers = Footers::new(file)?;
    let mut skipped = 0;
    while let Some((root_extent, length)) = footers.next()? {
        let root = read_root(file, &root_extent)?;
        let entries = match root.state {
            None => Some(Vec::new()),
            Some(slot) => match state.map(|state| state.read(slot)).transpose()?.flatten() {
                Some(stored) => Some(read_entries(&decompress(&stored, None)?)?),
                None => None,
            },
        };
        if let Some(state) = entries {
            return Ok(Recovered {
                root,
                root_extent,
                length,
                state,
                skipped,
            });
        }
        skipped += 1;
    }
    Err(invalid(
        "No root of the recording names resume state that verifies beside it, so it cannot be taken up",
    ))
}

/// Stored bytes a container wrote of its own, apart from its sections' extents.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Framing {
    pub roots: u64,
    pub pages: u64,
    /// The resume state's slots, beside the file.
    pub slots: u64,
    pub footers: u64,
}

/// The level resume state slots are compressed at: written every revision, read only after a crash.
const STATE_LEVEL: i32 = 1;

/// Writes a recording file of sections, a revision at a time.
pub struct ContainerWriter<W: Write + Seek> {
    file: W,
    identity: Identity,
    /// The last revision published; zero before the first.
    revision: u64,
    sections: Vec<Section>,
    state: Option<StateFile>,
    /// The slot the last root names.
    slot: Option<SlotRef>,
    /// The last slot sequence written.
    sequence: u64,
    /// The level roots and index pages are compressed at.
    level: i32,
    compressor: Compressor,
    /// A write failed partway: what the file holds past its last revision is not to be named.
    poisoned: bool,
    framing: Framing,
}

impl<W: Write + Seek> ContainerWriter<W> {
    /// A new file at `file`'s start, of `identity`, its roots and index pages compressed at
    /// `level`, its resume state kept in `state`.
    pub fn create(mut file: W, identity: Identity, level: i32, state: Option<StateFile>) -> io::Result<Self> {
        file.write_all(envelope::MAGIC)?;
        file.write_all(&envelope::VERSION.to_le_bytes())?;
        Self::with(file, identity, 0, Vec::new(), level, state, None)
    }

    /// Takes up a file after `root`, its last revision, which ends at `length`: what follows it
    /// must be cut already.
    pub fn resume(mut file: W, root: Root, length: u64, level: i32, state: Option<StateFile>) -> io::Result<Self> {
        file.seek(SeekFrom::Start(length))?;
        Self::with(file, root.identity, root.revision, root.sections, level, state, root.state)
    }

    fn with(
        file: W,
        identity: Identity,
        revision: u64,
        sections: Vec<Section>,
        level: i32,
        state: Option<StateFile>,
        slot: Option<SlotRef>,
    ) -> io::Result<Self> {
        let sequence = state
            .as_ref()
            .and_then(StateFile::newest)
            .max(slot.map(|slot| slot.sequence))
            .unwrap_or(0);
        Ok(Self {
            file,
            identity,
            revision,
            sections,
            state,
            slot,
            sequence,
            level,
            compressor: Compressor::new(&CONTAINER, level)?,
            poisoned: false,
            framing: Framing::default(),
        })
    }

    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    /// The last revision published; zero before the first.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    pub fn section_of(&self, key: SectionKey) -> Option<&Section> {
        find(&self.sections, key).ok().map(|at| &self.sections[at])
    }

    /// The stored bytes it wrote of its own.
    pub fn framing(&self) -> Framing {
        self.framing
    }

    /// The file's length so far.
    pub fn position(&mut self) -> io::Result<u64> {
        self.file.stream_position()
    }

    pub fn file_mut(&mut self) -> &mut W {
        &mut self.file
    }

    /// The file it writes.
    pub fn into_inner(self) -> W {
        self.file
    }

    fn usable(&self) -> io::Result<()> {
        if self.poisoned {
            return Err(io::Error::other(
                "A write to the recording failed partway; it is rolled back to its last revision",
            ));
        }
        Ok(())
    }

    fn poison<T>(&mut self, result: io::Result<T>) -> io::Result<T> {
        self.poisoned |= result.is_err();
        result
    }

    /// Declares section `key` of schema `version`, which a file holding it must hold at that
    /// version.
    pub fn section(&mut self, key: SectionKey, version: u32) -> io::Result<()> {
        key.check()?;
        match find(&self.sections, key) {
            Ok(at) => self.sections[at].expect(version),
            Err(at) => {
                engine_allocation::reserve(&mut self.sections, 1)?;
                self.sections.insert(
                    at,
                    Section {
                        key,
                        version,
                        frozen: Frozen::default(),
                        open: Vec::new(),
                    },
                );
                Ok(())
            }
        }
    }

    fn held(&self, key: SectionKey) -> io::Result<usize> {
        find(&self.sections, key).map_err(|_| invalid(format!("{key} is not declared")))
    }

    /// `stored`, of `raw` bytes, appended as an extent of `kind`, named by nothing yet.
    pub fn put(&mut self, kind: Kind, tags: Tags, raw: u64, stored: &[u8]) -> io::Result<Extent> {
        self.usable()?;
        let result = self.write(kind, tags, raw, stored);
        self.poison(result)
    }

    fn write(&mut self, kind: Kind, tags: Tags, raw: u64, stored: &[u8]) -> io::Result<Extent> {
        let offset = self.file.stream_position()?;
        self.file.write_all(stored)?;
        Ok(Extent {
            kind,
            run: tags.run,
            stream: tags.stream,
            first: tags.first,
            hours: tags.hours,
            offset,
            stored: stored.len() as u64,
            raw,
            digest: envelope::digest(stored),
        })
    }

    /// `raw` compressed at `level` as one zstd frame, appended as an extent of `kind`.
    pub fn put_zstd(&mut self, kind: Kind, tags: Tags, raw: &[u8], level: i32) -> io::Result<Extent> {
        self.usable()?;
        let result = compress(&mut self.compressor, level, raw).and_then(|stored| self.write(kind, tags, raw.len() as u64, &stored));
        self.poison(result)
    }

    /// Names `extent` the next of `key`'s frozen extents: before its open ones, never replaced.
    pub fn append(&mut self, key: SectionKey, extent: Extent) -> io::Result<()> {
        self.usable()?;
        let at = self.held(key)?;
        let section = &self.sections[at];
        if !section.open.is_empty() {
            return Err(invalid(format!("{key}'s frozen extents precede its open ones")));
        }
        check_entry(key, 0, &extent)?;
        check_owner(key, section.frozen.len(), &extent)?;
        if !section.frozen.follows(&extent) {
            return Err(invalid(format!("{key}'s frozen extents are appended in the order they are written")));
        }
        let mut frozen = std::mem::take(&mut self.sections[at].frozen);
        let result = frozen.push(extent, &mut |level, entries| {
            let raw = write_page(level, entries)?;
            let stored = compress(&mut self.compressor, self.level, &raw)?;
            self.framing.pages += stored.len() as u64;
            let tags = Tags {
                stream: level,
                ..Tags::default()
            };
            self.write(Kind::Index, tags, raw.len() as u64, &stored)
        });
        self.sections[at].frozen = frozen;
        self.poison(result)
    }

    /// Replaces `key`'s open extents, its last, with `extents`.
    pub fn set_open(&mut self, key: SectionKey, extents: &[Extent]) -> io::Result<()> {
        self.usable()?;
        let at = self.held(key)?;
        let first = self.sections[at].frozen.len();
        for (position, extent) in (first..).zip(extents) {
            check_entry(key, 0, extent)?;
            check_owner(key, position, extent)?;
        }
        let open = engine_allocation::copy_vec(extents)?;
        self.sections[at].open = open;
        Ok(())
    }

    /// Publishes everything named so far as the next revision, written with resume state
    /// `state` when given. `sync` makes what is written durable before what names it. A failure
    /// leaves the file at its last revision.
    pub fn commit(&mut self, state: Option<&[StateEntry]>, sync: impl FnMut(&mut W) -> io::Result<()>) -> io::Result<Revision> {
        self.commit_with(state, sync, |_| Ok(()))
    }

    /// [`Self::commit`], `prepare` told the revision once everything but its footer is
    /// durable, before the footer names it: a crash after `prepare` leaves either revision.
    pub fn commit_with(
        &mut self,
        state: Option<&[StateEntry]>,
        mut sync: impl FnMut(&mut W) -> io::Result<()>,
        prepare: impl FnOnce(Revision) -> io::Result<()>,
    ) -> io::Result<Revision> {
        self.usable()?;
        let result = self.publish(state, &mut sync, prepare);
        self.poison(result)
    }

    fn publish(
        &mut self,
        state: Option<&[StateEntry]>,
        sync: &mut dyn FnMut(&mut W) -> io::Result<()>,
        prepare: impl FnOnce(Revision) -> io::Result<()>,
    ) -> io::Result<Revision> {
        sync(&mut self.file)?;
        let slot = match state {
            None => None,
            Some(entries) => {
                let raw = write_entries(entries)?;
                let stored = compress(&mut self.compressor, STATE_LEVEL, &raw)?;
                let file = self
                    .state
                    .as_mut()
                    .ok_or_else(|| invalid("A revision with resume state needs its state file"))?;
                let sequence = self.sequence + 1;
                let slot = file.write(sequence, &stored, self.slot)?;
                self.sequence = sequence;
                self.framing.slots += stored.len() as u64;
                Some(slot)
            }
        };
        let root = write_root(&self.identity, self.revision + 1, slot, &self.sections)?;
        let stored = compress(&mut self.compressor, self.level, &root)?;
        let extent = self.write(Kind::Root, Tags::default(), root.len() as u64, &stored)?;
        self.framing.roots += extent.stored;
        sync(&mut self.file)?;
        let length = self.file.stream_position()? + FOOTER_BYTES as u64;
        let revision = Revision {
            number: self.revision + 1,
            length,
            root: extent.digest,
        };
        prepare(revision)?;
        self.file
            .write_all(&envelope::footer(extent.offset, extent.stored, &extent.digest))?;
        self.file.flush()?;
        sync(&mut self.file)?;
        self.framing.footers += FOOTER_BYTES as u64;
        self.revision = revision.number;
        self.slot = slot;
        Ok(revision)
    }

    /// Publishes the last revision: each family's state of `final_state` written into the file
    /// once, as its final state section, and a root naming no resume state, after which the
    /// state file goes: the file, its last revision and what it wrote of its own.
    pub fn finish(mut self, final_state: &[StateEntry], sync: impl FnMut(&mut W) -> io::Result<()>) -> io::Result<(W, Revision, Framing)> {
        self.usable()?;
        for entry in final_state {
            if entry.owner.kind != SectionKind::FamilyAnswers {
                return Err(invalid(format!(
                    "{} keeps no final state; only a family's is kept",
                    entry.owner
                )));
            }
            let key = SectionKey::new(SectionKind::FinalState, entry.owner.id);
            self.section(key, entry.version)?;
            let extent = self.put_zstd(Kind::Data, Tags::default(), &entry.bytes, self.level)?;
            self.set_open(key, &[extent])?;
        }
        let revision = self.commit(None, sync)?;
        if let Some(state) = self.state.take() {
            state.remove()?;
        }
        Ok((self.file, revision, self.framing))
    }
}
