
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

pub mod schema {
    pub const WORLD_LOG: u32 = 11;
    pub const RECORDS: u32 = 2;
    pub const FACTS: u32 = 1;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SectionKind {
    WorldLog,
    Records,
    Facts,
    MapRestore,
    FamilyAnswers,
    Catalogue,
    FinalState,
    DerivedVersion,
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
            Self::DerivedVersion => 8,
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
            8 => Self::DerivedVersion,
            _ => {
                return Err(invalid(format!(
                    "The recording holds a section of kind {code}, which this build does not know and has no decoder for"
                )));
            }
        })
    }

    pub fn holds(self, kind: Kind) -> bool {
        match self {
            Self::WorldLog => kind == Kind::RunDirectory,
            Self::Records => kind == Kind::Controls,
            Self::Facts => kind == Kind::Facts,
            Self::MapRestore | Self::FamilyAnswers | Self::Catalogue | Self::FinalState => {
                matches!(kind, Kind::Data | Kind::Stored)
            }
            Self::DerivedVersion => kind == Kind::Stored,
        }
    }

    const fn single(self) -> bool {
        matches!(self, Self::WorldLog | Self::Records | Self::Facts)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SectionKey {
    pub kind: SectionKind,
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

    pub const fn derived(table: u32) -> Self {
        Self::new(SectionKind::DerivedVersion, table)
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
            SectionKind::DerivedVersion => write!(f, "Derived table {}'s version", self.id),
        }
    }
}

pub const FANOUT: usize = 32;
const LEVELS: usize = 12;
pub const PAGE_RAW_BYTES: u64 = 2 + FANOUT as u64 * (1 + 4 * 5 + 3 * 10 + 16);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frozen {
    levels: Vec<Vec<Extent>>,
}

pub enum Place<'a> {
    Extent(&'a Extent),
    Page { page: &'a Extent, level: u32, at: u64 },
}

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

    pub fn levels(&self) -> &[Vec<Extent>] {
        &self.levels
    }

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

    fn follows(&self, extent: &Extent) -> bool {
        self.levels
            .iter()
            .flatten()
            .all(|held| extent.offset > held.offset && extent.offset >= held.offset + held.stored)
    }

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub key: SectionKey,
    pub version: u32,
    pub frozen: Frozen,
    pub open: Vec<Extent>,
}

impl Section {
    pub fn len(&self) -> u64 {
        self.frozen.len() + self.open.len() as u64
    }

    pub fn is_empty(&self) -> bool {
        self.frozen.is_empty() && self.open.is_empty()
    }

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlotRef {
    pub sequence: u64,
    pub digest: [u8; 16],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Saved {
    Yes,
    No(Option<Revision>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Root {
    pub identity: Identity,
    pub revision: u64,
    pub state: Option<SlotRef>,
    pub saved: Saved,
    pub sections: Vec<Section>,
    pub level: Option<i32>,
}

impl Root {
    pub fn write(&self) -> io::Result<Vec<u8>> {
        write_root(&self.identity, self.revision, self.state, self.saved, self.level, &self.sections)
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
        let saved = match reader.u8()? {
            0 => Saved::Yes,
            1 => Saved::No(None),
            2 => Saved::No(Some(Revision {
                number: reader.varint()?,
                length: reader.varint()?,
                root: reader.raw(16)?.try_into().expect("sixteen bytes"),
            })),
            _ => return Err(invalid("The root's saved marker is malformed")),
        };
        if matches!(saved, Saved::No(Some(before)) if before.number >= revision) {
            return Err(invalid("The root names a saved revision that is not before it"));
        }
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
        let level = match reader.left() {
            0 => None,
            _ => {
                let zigged = reader.varint()?;
                let level = (zigged >> 1) as i64 ^ -((zigged & 1) as i64);
                Some(i32::try_from(level).map_err(|_| invalid("The root's level passes 32 bits"))?)
            }
        };
        reader.finish()?;
        let root = Self {
            identity,
            revision,
            state,
            saved,
            sections,
            level,
        };
        validate_sections(&root.sections)?;
        Ok(root)
    }

    pub fn section(&self, key: SectionKey) -> Option<&Section> {
        find(&self.sections, key).ok().map(|at| &self.sections[at])
    }

    pub fn open(&self, key: SectionKey, version: u32) -> io::Result<Option<&Section>> {
        let section = self.section(key);
        if let Some(section) = section {
            section.expect(version)?;
        }
        Ok(section)
    }
}

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

fn write_root(
    identity: &Identity,
    revision: u64,
    state: Option<SlotRef>,
    saved: Saved,
    level: Option<i32>,
    sections: &[Section],
) -> io::Result<Vec<u8>> {
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
    match saved {
        Saved::Yes => writer.u8(0)?,
        Saved::No(None) => writer.u8(1)?,
        Saved::No(Some(before)) => {
            writer.u8(2)?;
            writer.varint(before.number)?;
            writer.varint(before.length)?;
            writer.raw(&before.root)?;
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
    if let Some(level) = level {
        let level = i64::from(level);
        writer.varint(((level << 1) ^ (level >> 63)) as u64)?;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Revision {
    pub number: u64,
    pub length: u64,
    pub root: [u8; 16],
}

impl Revision {
    pub fn digest(self) -> u64 {
        u64::from_le_bytes(self.root[..8].try_into().expect("eight bytes"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateEntry {
    pub owner: SectionKey,
    pub version: u32,
    pub bytes: Vec<u8>,
}

impl StateEntry {
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tags {
    pub run: u32,
    pub stream: u32,
    pub first: u32,
    pub hours: u32,
}

static CONTAINER: Holder = Holder::new("zstd: container");

fn window(raw: usize) -> u32 {
    let needed = raw.max(2);
    (usize::BITS - (needed - 1).leading_zeros()).clamp(10, 27)
}

pub fn compress(compressor: &mut Compressor, level: i32, raw: &[u8]) -> io::Result<Vec<u8>> {
    compressor.level(level)?;
    compressor.window_log(window(raw.len()))?;
    let mut stored = engine_allocation::vec(zstd::bound(raw.len()))?;
    compressor
        .compress_append(raw, &mut stored)
        .map_err(zstd::Error::invalid)?;
    Ok(stored)
}

const DECODE_STEP: usize = 128 << 10;

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

pub trait ReadAt {
    fn len(&self) -> io::Result<u64>;
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
                return Err(io::Error::other("The recording's file ends before its committed length"));
            }
            at += count as u64;
            out = &mut out[count..];
        }
        Ok(())
    }
}

const SCAN: usize = 64 << 10;

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

pub struct Footers<'f, F: ReadAt + ?Sized> {
    file: &'f F,
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
            budget: 2 * length,
            block: engine_allocation::filled(SCAN, 0)?,
            hashed: engine_allocation::filled(SCAN, 0)?,
        })
    }

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
            self.stop = self.first + magic as u64 - 1;
        }
    }
}

pub fn read_root<F: ReadAt + ?Sized>(file: &F, extent: &Extent) -> io::Result<Root> {
    let mut stored = engine_allocation::filled(usize::try_from(extent.stored).map_err(|_| engine_allocation::oom())?, 0)?;
    file.read_at(extent.offset, &mut stored)?;
    if envelope::digest(&stored) != extent.digest {
        return Err(invalid("The recording's root differs from its digest"));
    }
    Root::read(&decompress(&stored, None)?)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Open {
    Saved,
    Newest,
}

pub fn locate<F: ReadAt + ?Sized>(file: &F, open: Open) -> io::Result<Option<(Extent, u64)>> {
    let (newest, length) = Footers::new(file)?
        .next()?
        .ok_or_else(|| invalid("The recording has no footer whose root verifies"))?;
    if open == Open::Newest {
        return Ok(Some((newest, length)));
    }
    match read_root(file, &newest)?.saved {
        Saved::Yes => Ok(Some((newest, length))),
        Saved::No(None) => Ok(None),
        Saved::No(Some(saved)) => root_at(file, saved).map(|root| Some((root, saved.length))),
    }
}

pub fn root_at<F: ReadAt + ?Sized>(file: &F, revision: Revision) -> io::Result<Extent> {
    let header = HEADER_BYTES as u64;
    let footer = FOOTER_BYTES as u64;
    let missing = || invalid("The recording does not hold the revision its root names as saved");
    if revision.length < header + footer || revision.length > file.len()? {
        return Err(missing());
    }
    let mut bytes = [0; FOOTER_BYTES];
    file.read_at(revision.length - footer, &mut bytes)?;
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
    if bytes[32..] != *FOOTER_MAGIC
        || root.digest != revision.root
        || root.offset < header
        || root.offset.checked_add(root.stored) != Some(revision.length - footer)
    {
        return Err(missing());
    }
    Ok(root)
}

pub struct Recovered {
    pub root: Root,
    pub root_extent: Extent,
    pub length: u64,
    pub state: Vec<StateEntry>,
    pub skipped: u32,
}

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Framing {
    pub roots: u64,
    pub pages: u64,
    pub slots: u64,
    pub footers: u64,
}

const STATE_LEVEL: i32 = 1;

enum Slot<'a> {
    None,
    Carried,
    Written(&'a [StateEntry]),
}

impl<'a> From<Option<&'a [StateEntry]>> for Slot<'a> {
    fn from(state: Option<&'a [StateEntry]>) -> Self {
        state.map_or(Self::None, Self::Written)
    }
}

pub struct ContainerWriter<W: Write + Seek> {
    file: W,
    identity: Identity,
    revision: u64,
    sections: Vec<Section>,
    state: Option<StateFile>,
    slot: Option<SlotRef>,
    saved: Option<Revision>,
    sequence: u64,
    level: i32,
    recorded: Option<i32>,
    compressor: Compressor,
    poisoned: bool,
    framing: Framing,
}

impl<W: Write + Seek> ContainerWriter<W> {
    pub fn create(file: W, identity: Identity, level: i32, state: Option<StateFile>) -> io::Result<Self> {
        Self::create_after(file, identity, level, 0, state)
    }

    pub fn create_after(mut file: W, identity: Identity, level: i32, revision: u64, state: Option<StateFile>) -> io::Result<Self> {
        file.write_all(envelope::MAGIC)?;
        file.write_all(&envelope::VERSION.to_le_bytes())?;
        Self::with(file, identity, revision, Vec::new(), (level, Some(level)), state, None, None)
    }

    pub fn resume(mut file: W, root: Root, at: Revision, level: i32, state: Option<StateFile>) -> io::Result<Self> {
        if root.revision != at.number {
            return Err(invalid("A file is taken up at its root's own revision"));
        }
        file.seek(SeekFrom::Start(at.length))?;
        let saved = match root.saved {
            Saved::Yes => Some(at),
            Saved::No(before) => before,
        };
        let recorded = root.level.filter(|&held| held == level);
        Self::with(file, root.identity, root.revision, root.sections, (level, recorded), state, root.state, saved)
    }

    #[allow(clippy::too_many_arguments, reason = "one private constructor of two public ones")]
    fn with(
        file: W,
        identity: Identity,
        revision: u64,
        sections: Vec<Section>,
        (level, recorded): (i32, Option<i32>),
        state: Option<StateFile>,
        slot: Option<SlotRef>,
        saved: Option<Revision>,
    ) -> io::Result<Self> {
        let sequence = slot.map_or(0, |slot| slot.sequence);
        Ok(Self {
            file,
            identity,
            revision,
            sections,
            state,
            slot,
            saved,
            sequence,
            level,
            recorded,
            compressor: Compressor::new(&CONTAINER, level)?,
            poisoned: false,
            framing: Framing::default(),
        })
    }

    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn saved(&self) -> Option<Revision> {
        self.saved
    }

    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    pub fn section_of(&self, key: SectionKey) -> Option<&Section> {
        find(&self.sections, key).ok().map(|at| &self.sections[at])
    }

    pub fn framing(&self) -> Framing {
        self.framing
    }

    pub fn position(&mut self) -> io::Result<u64> {
        self.file.stream_position()
    }

    pub fn file_mut(&mut self) -> &mut W {
        &mut self.file
    }

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

    pub fn put(&mut self, kind: Kind, tags: Tags, raw: u64, stored: &[u8]) -> io::Result<Extent> {
        self.usable()?;
        let result = self.write(kind, tags, raw, stored);
        self.poison(result)
    }

    pub fn put_from(&mut self, kind: Kind, tags: Tags, raw: u64, source: &mut dyn io::Read, length: u64) -> io::Result<Extent> {
        self.usable()?;
        let result = self.copy(kind, tags, raw, source, length);
        self.poison(result)
    }

    fn copy(&mut self, kind: Kind, tags: Tags, raw: u64, source: &mut dyn io::Read, length: u64) -> io::Result<Extent> {
        let offset = self.file.stream_position()?;
        let mut hasher = blake3::Hasher::new();
        let mut buffer = vec![0; (1 << 20).min(length as usize).max(1)];
        let mut left = length;
        while left > 0 {
            let piece = &mut buffer[..left.min(1 << 20) as usize];
            source.read_exact(piece)?;
            hasher.update(piece);
            self.file.write_all(piece)?;
            left -= piece.len() as u64;
        }
        Ok(Extent {
            kind,
            run: tags.run,
            stream: tags.stream,
            first: tags.first,
            hours: tags.hours,
            offset,
            stored: length,
            raw,
            digest: hasher.finalize().as_bytes()[..16].try_into().expect("sixteen bytes"),
        })
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

    pub fn put_zstd(&mut self, kind: Kind, tags: Tags, raw: &[u8], level: i32) -> io::Result<Extent> {
        self.usable()?;
        let result = compress(&mut self.compressor, level, raw).and_then(|stored| self.write(kind, tags, raw.len() as u64, &stored));
        self.poison(result)
    }

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

    pub fn commit(
        &mut self,
        state: Option<&[StateEntry]>,
        saved: bool,
        sync: impl FnMut(&mut W) -> io::Result<()>,
    ) -> io::Result<Revision> {
        self.commit_with(state, saved, sync, |_| Ok(()))
    }

    pub fn commit_with(
        &mut self,
        state: Option<&[StateEntry]>,
        saved: bool,
        mut sync: impl FnMut(&mut W) -> io::Result<()>,
        prepare: impl FnOnce(Revision) -> io::Result<()>,
    ) -> io::Result<Revision> {
        self.usable()?;
        let result = self.publish(state, saved, &mut sync, prepare);
        self.poison(result)
    }

    pub fn commit_carrying(&mut self, saved: bool, mut sync: impl FnMut(&mut W) -> io::Result<()>) -> io::Result<Revision> {
        self.usable()?;
        let result = self.publish(Slot::Carried, saved, &mut sync, |_| Ok(()));
        self.poison(result)
    }

    fn publish<'a>(
        &mut self,
        state: impl Into<Slot<'a>>,
        saved: bool,
        sync: &mut dyn FnMut(&mut W) -> io::Result<()>,
        prepare: impl FnOnce(Revision) -> io::Result<()>,
    ) -> io::Result<Revision> {
        sync(&mut self.file)?;
        let slot = match state.into() {
            Slot::None => None,
            Slot::Carried => self.slot,
            Slot::Written(entries) => {
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
        let mark = match saved {
            true => Saved::Yes,
            false => Saved::No(self.saved),
        };
        let root = write_root(&self.identity, self.revision + 1, slot, mark, self.recorded, &self.sections)?;
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
        if saved {
            self.saved = Some(revision);
        }
        Ok(revision)
    }

    pub fn finish(
        mut self,
        final_state: &[StateEntry],
        saved: bool,
        sync: impl FnMut(&mut W) -> io::Result<()>,
    ) -> io::Result<(W, Revision, Framing)> {
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
        let revision = self.commit(None, saved, sync)?;
        if let Some(state) = self.state.take() {
            state.remove()?;
        }
        Ok((self.file, revision, self.framing))
    }
}
