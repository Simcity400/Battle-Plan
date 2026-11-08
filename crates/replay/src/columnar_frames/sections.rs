//! A recording's sections as readers take them ([`replay_columnar::container`]): each one's
//! extents listed through its index pages, and read with their places and digests checked.

use super::{
    input::{Decoder, Source},
    invalid,
};
use replay_columnar::{
    container::{PAGE_RAW_BYTES, Place, decompress, Section, SectionKey, check_follows, check_owner, read_page, span},
    envelope::{Extent, Kind},
};
use std::io;

/// Every extent of `section`, in order: its frozen ones read down through their pages, then its
/// open ones.
pub(super) fn extents(source: &Source, decoder: &mut Decoder, section: &Section) -> io::Result<Vec<Extent>> {
    let mut out = Vec::new();
    for (level, held) in section.frozen.levels().iter().enumerate().rev() {
        for extent in held {
            if level == 0 {
                push(source, section.key, extent, &mut out)?;
            } else {
                expand(source, decoder, section.key, extent, &mut out)?;
            }
        }
    }
    engine_allocation::reserve(&mut out, section.open.len())?;
    out.extend_from_slice(&section.open);
    for (at, extent) in (0_u64..).zip(&out) {
        check_owner(section.key, at, extent)?;
    }
    Ok(out)
}

/// Appends `extent`, the next frozen extent of `key`, once it follows the one before it: a
/// section names no more of them than its file holds bytes.
fn push(source: &Source, key: SectionKey, extent: &Extent, out: &mut Vec<Extent>) -> io::Result<()> {
    check_follows(key, out.last(), extent, source.len())?;
    engine_allocation::reserve(out, 1)?;
    out.push(*extent);
    Ok(())
}

/// Appends the extents `page`, an index page of `key`, covers, each checked as it is read.
fn expand(source: &Source, decoder: &mut Decoder, key: SectionKey, page: &Extent, out: &mut Vec<Extent>) -> io::Result<()> {
    for entry in entries(source, decoder, key, page)? {
        if page.stream == 1 {
            push(source, key, &entry, out)?;
        } else {
            expand(source, decoder, key, &entry, out)?;
        }
    }
    Ok(())
}

fn entries(source: &Source, decoder: &mut Decoder, key: SectionKey, page: &Extent) -> io::Result<Vec<Extent>> {
    // Its envelope sizes what decodes it: no page takes more.
    if page.raw > PAGE_RAW_BYTES {
        return Err(page.damaged(&format!("is larger than any index page of {key}")));
    }
    let mut raw = Vec::new();
    decoder.append(source, page, &mut raw)?;
    read_page(&raw, page, key)
}

/// One section of a recording, opened at its schema version.
pub struct SectionReader<'r> {
    source: &'r Source,
    section: &'r Section,
}

impl<'r> SectionReader<'r> {
    pub(super) fn new(source: &'r Source, section: &'r Section) -> Self {
        Self { source, section }
    }

    pub fn key(&self) -> SectionKey {
        self.section.key
    }

    pub fn version(&self) -> u32 {
        self.section.version
    }

    pub fn len(&self) -> u64 {
        self.section.len()
    }

    pub fn is_empty(&self) -> bool {
        self.section.is_empty()
    }

    /// Its extent at `at`, read down through the pages holding it.
    pub fn extent(&self, at: u64) -> io::Result<Extent> {
        let frozen = self.section.frozen.len();
        if at >= frozen {
            let open = usize::try_from(at - frozen).ok();
            return open
                .and_then(|open| self.section.open.get(open).copied())
                .ok_or_else(|| invalid(format!("{} holds no extent {at}", self.section.key)));
        }
        let mut decoder = Decoder::new()?;
        let (mut page, mut level, mut within) = match self.section.frozen.locate(at) {
            Some(Place::Extent(extent)) => return Ok(*extent),
            Some(Place::Page { page, level, at }) => (*page, level, at),
            None => return Err(invalid(format!("{} holds no extent {at}", self.section.key))),
        };
        loop {
            let entries = entries(self.source, &mut decoder, self.section.key, &page)?;
            let below = span(level - 1);
            let entry = entries[(within / below) as usize];
            if level == 1 {
                check_owner(self.section.key, at, &entry)?;
                return Ok(entry);
            }
            (page, level, within) = (entry, level - 1, within % below);
        }
    }

    /// Every extent, in order.
    pub fn extents(&self) -> io::Result<Vec<Extent>> {
        extents(self.source, &mut Decoder::new()?, self.section)
    }

    /// `extent`'s bytes, its place and digest checked: a [`Kind::Stored`] or facts extent's as
    /// stored, any other's decoded from its frame.
    pub fn read(&self, extent: &Extent) -> io::Result<Vec<u8>> {
        read(self.source, extent)
    }

    /// `out.len()` stored bytes of `extent`, a [`Kind::Stored`] or facts extent, from `offset`
    /// within it, its place checked; its digest is the caller's to check, once, by
    /// [`Self::read`].
    pub fn read_at(&self, extent: &Extent, offset: u64, out: &mut [u8]) -> io::Result<()> {
        if !matches!(extent.kind, Kind::Stored | Kind::Facts) {
            return Err(invalid("Only an extent stored as it is reads in parts"));
        }
        self.source.stored_at(extent, offset, out)
    }
}

/// `extent`'s bytes in `source`, its place and digest checked: as stored for one stored as it
/// is, else decoded from its frame.
pub(super) fn read(source: &Source, extent: &Extent) -> io::Result<Vec<u8>> {
    if matches!(extent.kind, Kind::Records | Kind::Block) {
        return Err(invalid("A page's unit is read in its page's chunks"));
    }
    let mut buffer = Vec::new();
    match extent.kind {
        Kind::Stored | Kind::Facts => return engine_allocation::copy_vec(source.stored_checked(extent, &mut buffer)?),
        // An owner's frame grows its output only as it decodes.
        Kind::Data => return decompress(source.stored_checked(extent, &mut buffer)?, Some(extent.raw)),
        _ => {}
    }
    let mut raw = Vec::new();
    Decoder::new()?.append(source, extent, &mut raw)?;
    Ok(raw)
}
