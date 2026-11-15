//! Where a recording's page entries come from: a format-12 recording ([`WorldPages`]), its runs
//! whole or one span of game hours, read through [`replay::columnar_frames::Recording`].

use crate::{Piece, invalid};
use replay::columnar_frames::{Damage, Open, Recording};
use replay_columnar::{
    state::Kind,
    unit::{BLOCK, BlockPage, reader::Extents},
};
use replay_data::Hour;
use std::{io, path::Path, time::Instant};

#[cfg(test)]
mod tests;

/// One run of a recording as read: the first game hour of its first page and its blocks' pages in
/// order, each a whole block but the last.
#[derive(Debug)]
pub struct Run {
    pub first_hour: Hour,
    pub pages: Vec<BlockPage>,
}

impl Run {
    /// A run of `pages`, refused by name unless they are its blocks in order.
    pub fn new(first_hour: Hour, pages: Vec<BlockPage>) -> io::Result<Self> {
        for (index, page) in pages.iter().enumerate() {
            if page.block as usize != index {
                return Err(invalid(format!("Block page {index} names block {}", page.block)));
            }
            let whole = u32::from(page.hours) == BLOCK;
            if page.hours == 0 || (!whole && index + 1 != pages.len()) {
                return Err(invalid(format!("Block {index} holds {} hours", page.hours)));
            }
        }
        if pages.is_empty() {
            return Err(invalid("A run holds no block"));
        }
        Ok(Self { first_hour, pages })
    }

    pub fn hours(&self) -> u32 {
        self.pages.iter().map(|page| u32::from(page.hours)).sum()
    }

    /// Its last game hour.
    pub fn last_hour(&self) -> Hour {
        self.first_hour + self.hours() as Hour - 1
    }
}

/// A recording's pages: each run's frames, its runs, and each table's column kinds.
pub trait Pages: Send + Sync {
    fn extents(&self, run: usize) -> &dyn Extents;
    fn runs(&self) -> &[Run];
    fn kinds(&self, table: usize) -> Option<&[Kind]>;
}

/// What of a recording is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Span {
    /// Every run whole.
    Whole,
    /// One run whole.
    Run(usize),
    /// Game hours `from` to `from + hours - 1` of the run holding `from`, clipped to its end.
    Hours { from: Hour, hours: u32 },
}

/// What opening read and holds.
#[derive(Clone, Copy, Debug, Default)]
pub struct Opened {
    pub ns: u64,
    /// Block pages held, and the parts and frame entries they name.
    pub blocks: usize,
    pub parts: usize,
    pub frames: usize,
    /// Tables' block ends packed (the live rows moved to the first slots).
    pub packed: usize,
}

/// A format-12 recording's pages as the playhead reads them. A run read for a span starts at the
/// block holding every table's last restore before the span's first hour, so a seek anywhere in
/// the span finds its restore; its pages are numbered from there. Kinds are the build's schema;
/// columns the recording's format did not write read as not recorded, and a table it holds as
/// this build reads it otherwise has no kinds ([`Self::withheld`]). A run whose directory cannot
/// be read is left out of [`Span::Whole`] ([`Recording::damage`] names it).
pub struct WorldPages {
    recording: Recording,
    runs: Vec<Run>,
    pieces: Vec<Piece>,
    kinds: Vec<Vec<Kind>>,
    opened: Opened,
}

impl WorldPages {
    /// The newest saved revision of the recording at `path`.
    pub fn open(path: &Path, span: Span) -> io::Result<Self> {
        let started = Instant::now();
        let mut pages = Self::over(Recording::open_path_as(path, Open::Saved)?, span)?;
        pages.opened.ns = started.elapsed().as_nanos() as u64;
        Ok(pages)
    }

    pub fn over(recording: Recording, span: Span) -> io::Result<Self> {
        let started = Instant::now();
        let mut kinds = replay::columnar_frames::schema()?;
        for &(table, _) in recording.withheld() {
            kinds[table].clear();
        }
        let readable: Vec<u32> = (0..recording.run_count()).filter(|&run| recording.run(run).is_ok()).collect();
        let wanted: Vec<u32> = match span {
            Span::Whole => readable,
            Span::Run(run) => {
                recording.run(u32::try_from(run).unwrap_or(u32::MAX))?;
                vec![run as u32]
            }
            Span::Hours { from, .. } => {
                let run = readable.into_iter().find(|&run| recording.run(run).is_ok_and(|run| run.ordinal(from).is_some()));
                vec![run.ok_or_else(|| invalid(format!("No readable run of the recording holds game hour {from}")))?]
            }
        };
        let (mut runs, mut pieces) = (Vec::new(), Vec::new());
        for index in wanted {
            let held = recording.run(index)?;
            let (first_hour, blocks) = (held.first_hour, held.pages() as u32);
            let last = held.last_hour();
            let (first, to) = match span {
                Span::Hours { from, hours } => (from, (i64::from(from) + i64::from(hours.max(1)) - 1).min(i64::from(last)) as Hour),
                _ => (first_hour, last),
            };
            let (from_block, pages) = pages_for(&recording, index, blocks, ((first - first_hour) as u32 / BLOCK, (to - first_hour) as u32 / BLOCK + 1))?;
            let run = Run::new(first_hour + (from_block * BLOCK) as Hour, pages)?;
            pieces.push(Piece { run: runs.len(), first, last: to });
            runs.push(run);
        }
        let mut opened = Opened { ns: 0, ..Opened::default() };
        for page in runs.iter().flat_map(|run| &run.pages) {
            opened.blocks += 1;
            for entry in &page.tables {
                opened.packed += usize::from(entry.packed);
                opened.parts += entry.parts.len();
                opened.frames += entry.parts.iter().map(|part| part.frames.len()).sum::<usize>();
            }
        }
        opened.ns = started.elapsed().as_nanos() as u64;
        Ok(Self { recording, runs, pieces, kinds, opened })
    }

    /// The pieces of the timeline what was opened plays: one a run, its span's hours.
    pub fn pieces(&self) -> &[Piece] {
        &self.pieces
    }

    pub fn opened(&self) -> Opened {
        self.opened
    }

    /// Every table's column kinds, none for a table withheld.
    pub fn schema(&self) -> &[Vec<Kind>] {
        &self.kinds
    }

    pub fn recording(&self) -> &Recording {
        &self.recording
    }

    /// Why the recording's table `table` is not read, when it is withheld.
    pub fn withheld(&self, table: usize) -> Option<&str> {
        self.recording.withheld_reason(table)
    }

    /// The runs whose directories cannot be read, each with why.
    pub fn damage(&self) -> &[Damage] {
        self.recording.damage()
    }
}

/// Blocks `from..to` of run `run`, and the blocks before `from` back to the one holding every
/// table's last restore before it: the first block read, and the pages numbered from it.
fn pages_for(recording: &Recording, run: u32, blocks: u32, (from, to): (u32, u32)) -> io::Result<(u32, Vec<BlockPage>)> {
    let to = to.min(blocks);
    let mut before = Vec::new();
    let mut found = vec![false; recording.tables()];
    let mut first = from;
    while first > 0 && found.iter().any(|found| !found) {
        first -= 1;
        let page = recording.block(run, first)?.unit_page();
        for (table, entry) in page.tables.iter().enumerate() {
            found[table] |= entry.restore.is_some() || entry.first.is_some();
        }
        before.push(page);
    }
    before.reverse();
    let mut pages = before;
    pages.extend(recording.unit_pages(run, from, to)?);
    for (index, page) in pages.iter_mut().enumerate() {
        page.block = index as u32;
    }
    Ok((first, pages))
}

impl Pages for WorldPages {
    fn extents(&self, _run: usize) -> &dyn Extents {
        self.recording.extents()
    }

    fn runs(&self) -> &[Run] {
        &self.runs
    }

    fn kinds(&self, table: usize) -> Option<&[Kind]> {
        self.kinds.get(table).filter(|kinds| !kinds.is_empty()).map(Vec::as_slice)
    }
}
