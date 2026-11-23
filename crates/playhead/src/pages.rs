
use crate::{Piece, invalid};
use replay_recording::columnar_frames::{Damage, Open, Recording};
use replay_columnar::{
    state::Kind,
    unit::{BLOCK, BlockPage, reader::Extents},
};
use replay_data::{BranchId, Hour};
use replay_data::rows::TableId;
use std::{
    io,
    path::Path,
    sync::{Arc, Mutex, PoisonError, Weak},
    time::Instant,
};

#[cfg(test)]
mod tests;

#[derive(Debug)]
pub struct Run {
    pub first_hour: Hour,
    pub pages: Vec<BlockPage>,
}

impl Run {
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

    pub fn last_hour(&self) -> Hour {
        self.first_hour + self.hours() as Hour - 1
    }
}

pub trait Pages: Send + Sync {
    fn extents(&self, run: usize) -> &dyn Extents;
    fn runs(&self) -> &[Run];
    fn kinds(&self, table: usize) -> Option<&[Kind]>;
    fn held(&self, table: usize) -> usize {
        self.kinds(table).map_or(0, <[Kind]>::len)
    }
    fn format(&self) -> u16 {
        replay_data::FORMAT_VERSION
    }
    fn withheld_at(&self, _run: usize, _table: usize, _hour: Hour) -> Option<&str> {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Span {
    Whole,
    Run(usize),
    Hours { from: Hour, hours: u32 },
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Opened {
    pub ns: u64,
    pub blocks: usize,
    pub parts: usize,
    pub frames: usize,
    pub packed: usize,
}

pub struct WorldPages {
    recording: Recording,
    runs: Vec<Run>,
    recorded: Vec<u32>,
    branches: Vec<BranchId>,
    pieces: Vec<Piece>,
    kinds: Vec<Vec<Kind>>,
    opened: Opened,
}

impl WorldPages {
    pub fn open(path: &Path, span: Span) -> io::Result<Self> {
        let started = Instant::now();
        let mut pages = Self::over(Recording::open_path_as(path, Open::Saved)?, span)?;
        pages.opened.ns = started.elapsed().as_nanos() as u64;
        Ok(pages)
    }

    pub fn over(recording: Recording, span: Span) -> io::Result<Self> {
        let started = Instant::now();
        let mut kinds = replay_recording::schema::schema()?;
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
        let (mut runs, mut branches, mut pieces) = (Vec::new(), Vec::new(), Vec::new());
        let recorded = wanted.clone();
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
            branches.push(held.branch);
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
        Ok(Self { recording, runs, recorded, branches, pieces, kinds, opened })
    }

    pub fn shared(recording: &Recording) -> io::Result<Arc<Self>> {
        static HELD: Mutex<Vec<(String, Weak<WorldPages>)>> = Mutex::new(Vec::new());
        static BUILDING: Mutex<()> = Mutex::new(());
        let name = name(recording);
        let held = || HELD.lock().unwrap_or_else(PoisonError::into_inner).iter().find(|(held, _)| *held == name).and_then(|(_, pages)| pages.upgrade());
        if let Some(pages) = held() {
            return Ok(pages);
        }
        let _building = BUILDING.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(pages) = held() {
            return Ok(pages);
        }
        let pages = Arc::new(Self::over(recording.selecting(&TableId::ALL)?, Span::Whole)?);
        let mut opened = HELD.lock().unwrap_or_else(PoisonError::into_inner);
        opened.retain(|(_, pages)| pages.strong_count() > 0);
        opened.push((name, Arc::downgrade(&pages)));
        Ok(pages)
    }

    pub fn run_of(&self, recorded: u32) -> Option<usize> {
        self.recorded.iter().position(|&run| run == recorded)
    }

    pub fn pieces(&self) -> &[Piece] {
        &self.pieces
    }

    pub fn branches(&self) -> &[BranchId] {
        &self.branches
    }

    pub fn opened(&self) -> Opened {
        self.opened
    }

    pub fn schema(&self) -> &[Vec<Kind>] {
        &self.kinds
    }

    pub fn recording(&self) -> &Recording {
        &self.recording
    }

    pub fn withheld(&self, table: usize) -> Option<&str> {
        self.recording.withheld_reason(table)
    }

    pub fn damage(&self) -> &[Damage] {
        self.recording.damage()
    }
}

pub fn name(recording: &Recording) -> String {
    let revision = recording.revision();
    let digest: String = revision.root.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{digest}:{}", revision.length)
}

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

    fn held(&self, table: usize) -> usize {
        self.recording.columns().get(table).copied().unwrap_or(0)
    }

    fn format(&self) -> u16 {
        self.recording.format()
    }

    fn withheld_at(&self, run: usize, table: usize, hour: Hour) -> Option<&str> {
        let &recorded = self.recorded.get(run)?;
        match self.recording.table_coverage(table, recorded) {
            replay_recording::columnar_frames::TableCoverage::Valid => None,
            replay_recording::columnar_frames::TableCoverage::WithheldAll(reason) => Some(reason),
            replay_recording::columnar_frames::TableCoverage::WithheldBefore { ordinal, reason } => {
                let first = self.recording.run(recorded).ok()?.first_hour;
                (hour < first + ordinal as i32).then_some(reason)
            }
        }
    }
}
