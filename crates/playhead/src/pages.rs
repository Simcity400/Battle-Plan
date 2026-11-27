use crate::{Piece, invalid};
use replay_recording::columnar_frames::{Damage, Open, Recording, SharedEntry};
use replay_columnar::{
    envelope::SPAN_BLOCKS,
    manifest::Status,
    state::Kind,
    unit::{
        BLOCK, BlockPage, TableEntry,
        reader::{self, Extents, PageEntry},
    },
};
use replay_data::{BranchId, Hour};
use replay_data::rows::TableId;
use std::{
    io,
    ops::Deref,
    path::Path,
    sync::{Arc, Mutex, OnceLock, PoisonError, Weak},
    time::Instant,
};

#[cfg(test)]
mod tests;

#[derive(Clone)]
pub struct EntryRef(EntryInner);

#[derive(Clone)]
enum EntryInner {
    Recorded(SharedEntry),
    Held(Arc<BlockPage>, usize),
    Empty,
}

impl EntryRef {
    pub fn recorded(entry: SharedEntry) -> Self {
        Self(EntryInner::Recorded(entry))
    }

    pub fn held(page: Arc<BlockPage>, table: usize) -> Self {
        Self(EntryInner::Held(page, table))
    }

    pub fn empty() -> Self {
        Self(EntryInner::Empty)
    }
}

impl Deref for EntryRef {
    type Target = TableEntry;

    fn deref(&self) -> &TableEntry {
        static EMPTY: OnceLock<TableEntry> = OnceLock::new();
        match &self.0 {
            EntryInner::Recorded(entry) => &entry.unit,
            EntryInner::Held(page, table) => &page.tables[*table],
            EntryInner::Empty => EMPTY.get_or_init(TableEntry::default),
        }
    }
}

#[derive(Clone)]
pub struct PageView {
    pub block: u32,
    pub hours: u16,
    pub table: usize,
    pub entry: EntryRef,
}

impl PageEntry for PageView {
    fn block(&self) -> u32 {
        self.block
    }

    fn hours(&self) -> u16 {
        self.hours
    }

    fn table_entry(&self, table: usize) -> Option<&TableEntry> {
        (table == self.table).then_some(&*self.entry)
    }
}

#[derive(Debug)]
enum RunBlocks {
    Held(Vec<Arc<BlockPage>>),
    Recorded(Box<[u16]>),
}

#[derive(Debug)]
pub struct Run {
    pub first_hour: Hour,
    blocks: RunBlocks,
    hours: u32,
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
        let hours = pages.iter().map(|page| u32::from(page.hours)).sum();
        Ok(Self { first_hour, blocks: RunBlocks::Held(pages.into_iter().map(Arc::new).collect()), hours })
    }

    fn recorded(first_hour: Hour, block_hours: Box<[u16]>) -> io::Result<Self> {
        if block_hours.is_empty() {
            return Err(invalid("A run holds no block"));
        }
        for (index, &hours) in block_hours.iter().enumerate() {
            let whole = u32::from(hours) == BLOCK;
            if hours == 0 || (!whole && index + 1 != block_hours.len()) {
                return Err(invalid(format!("Block {index} holds {hours} hours")));
            }
        }
        let hours = block_hours.iter().map(|&hours| u32::from(hours)).sum();
        Ok(Self { first_hour, blocks: RunBlocks::Recorded(block_hours), hours })
    }

    pub fn hours(&self) -> u32 {
        self.hours
    }

    pub fn last_hour(&self) -> Hour {
        self.first_hour + self.hours as Hour - 1
    }

    pub fn blocks(&self) -> usize {
        match &self.blocks {
            RunBlocks::Held(pages) => pages.len(),
            RunBlocks::Recorded(hours) => hours.len(),
        }
    }

    pub fn block_hours(&self, block: usize) -> Option<u16> {
        match &self.blocks {
            RunBlocks::Held(pages) => pages.get(block).map(|page| page.hours),
            RunBlocks::Recorded(hours) => hours.get(block).copied(),
        }
    }

    pub fn held_pages(&self) -> Option<&[Arc<BlockPage>]> {
        match &self.blocks {
            RunBlocks::Held(pages) => Some(pages),
            RunBlocks::Recorded(_) => None,
        }
    }

    pub(crate) fn held_entry(&self, block: usize, table: usize) -> io::Result<EntryRef> {
        let RunBlocks::Held(pages) = &self.blocks else {
            return Err(invalid("A recorded run's entries are read through its Pages implementation"));
        };
        let page = pages.get(block).ok_or_else(|| invalid(format!("Run holds no block {block}")))?;
        if table >= page.tables.len() {
            return Err(invalid(format!("Block {block} names no table {table}")));
        }
        Ok(EntryRef::held(Arc::clone(page), table))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TableWritten {
    Written,
    Unwritten,
    Withheld(String),
    Damaged(String),
}

pub fn entry_recorded(entry: &TableEntry, first_hour: Hour) -> bool {
    let mut hour = 0;
    loop {
        match reader::status(entry, hour, first_hour) {
            Ok((status, ..)) if status != Status::NotRecorded => return true,
            Ok((_, _, _, Some(next))) if next > hour => hour = next,
            _ => return false,
        }
    }
}

pub trait Pages: Send + Sync {
    fn extents(&self, run: usize) -> &dyn Extents;
    fn runs(&self) -> &[Run];
    fn table_entry(&self, run: usize, block: usize, table: usize) -> io::Result<EntryRef> {
        self.runs()
            .get(run)
            .ok_or_else(|| invalid(format!("No run {run}")))?
            .held_entry(block, table)
    }
    fn table_pages(&self, run: usize, table: usize, from: usize, to: usize) -> io::Result<Vec<PageView>> {
        let held = self.runs().get(run).ok_or_else(|| invalid(format!("No run {run}")))?;
        let end = to.min(held.blocks());
        let mut pages = Vec::with_capacity(end.saturating_sub(from));
        for block in from..end {
            let hours = held.block_hours(block).ok_or_else(|| invalid(format!("Run {run} holds no block {block}")))?;
            let entry = self.table_entry(run, block, table)?;
            pages.push(PageView { block: block as u32, hours, table, entry });
        }
        Ok(pages)
    }
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
    fn held_tables(&self, tables: usize) -> Vec<bool> {
        let runs = self.runs();
        (0..tables)
            .map(|table| {
                let unwithheld = self.kinds(table).is_some_and(|kinds| !kinds.is_empty())
                    && runs.iter().enumerate().any(|(run, held)| self.withheld_at(run, table, held.last_hour()).is_none());
                if !unwithheld {
                    return false;
                }
                let restored = runs.iter().enumerate().all(|(run_idx, run)| {
                    (0..run.blocks()).any(|block| {
                        self.table_entry(run_idx, block, table)
                            .is_ok_and(|entry| entry.first.is_some() || entry.restore.is_some())
                    })
                });
                if !restored {
                    return false;
                }
                runs.iter().enumerate().any(|(run_idx, run)| {
                    (0..run.blocks()).any(|block| {
                        self.table_entry(run_idx, block, table)
                            .is_ok_and(|entry| entry_recorded(&entry, run.first_hour + (block as u32 * BLOCK) as Hour))
                    })
                })
            })
            .collect()
    }
    fn written_tables(&self, runs: &[usize], tables: usize) -> Vec<TableWritten> {
        let total_runs = self.runs().len();
        (0..tables)
            .map(|table| {
                if self.kinds(table).is_none() {
                    if self.held(table) == 0 {
                        return TableWritten::Unwritten;
                    }
                    return runs
                        .iter()
                        .copied()
                        .filter(|&run| run < total_runs)
                        .find_map(|run| self.withheld_at(run, table, self.runs()[run].first_hour).map(|reason| TableWritten::Withheld(reason.to_owned())))
                        .unwrap_or(TableWritten::Unwritten);
                }
                let mut damaged = None;
                for &run in runs {
                    if run >= total_runs {
                        continue;
                    }
                    match self.table_entry(run, 0, table) {
                        Ok(entry) if entry.first.is_some() => return TableWritten::Written,
                        Ok(_) => {}
                        Err(error) if damaged.is_none() => damaged = Some(error.to_string()),
                        Err(_) => {}
                    }
                }
                damaged.map_or(TableWritten::Unwritten, TableWritten::Damaged)
            })
            .collect()
    }
    fn table_has_activity(&self, table: usize) -> bool {
        self.kinds(table).is_some()
            && self.runs().iter().enumerate().any(|(run_idx, run)| {
                (0..run.blocks()).any(|block| {
                    self.table_entry(run_idx, block, table)
                        .is_ok_and(|entry| !entry.parts.is_empty() || entry.first.is_some() || entry.restore.is_some())
                })
            })
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
}

pub struct WorldPages {
    recording: Recording,
    runs: Vec<Run>,
    first_blocks: Vec<u32>,
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
        let (mut runs, mut first_blocks, mut branches, mut pieces) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let recorded = wanted.clone();
        let mut total_blocks = 0usize;
        for index in wanted {
            let held = recording.run(index)?;
            let (first_hour, blocks) = (held.first_hour, held.pages() as u32);
            let last = held.last_hour();
            let (first, to) = match span {
                Span::Hours { from, hours } => (from, (i64::from(from) + i64::from(hours.max(1)) - 1).min(i64::from(last)) as Hour),
                _ => (first_hour, last),
            };
            let from_block = first_block_for(&recording, index, (first - first_hour) as u32 / BLOCK)?;
            let to_block = ((to - first_hour) as u32 / BLOCK + 1).min(blocks);
            let block_hours: Box<[u16]> = (from_block..to_block).map(|block| held.block(block as usize).1 as u16).collect();
            total_blocks += block_hours.len();
            let run = Run::recorded(first_hour + (from_block * BLOCK) as Hour, block_hours)?;
            pieces.push(Piece { run: runs.len(), first, last: to });
            runs.push(run);
            first_blocks.push(from_block);
            branches.push(held.branch);
        }
        let opened = Opened { ns: started.elapsed().as_nanos() as u64, blocks: total_blocks };
        Ok(Self { recording, runs, first_blocks, recorded, branches, pieces, kinds, opened })
    }

    pub fn shared(recording: &Recording) -> io::Result<Arc<Self>> {
        static HELD: Mutex<Vec<(String, Weak<WorldPages>)>> = Mutex::new(Vec::new());
        static BUILDING: Mutex<()> = Mutex::new(());
        let key = name(recording);
        let held = || HELD.lock().unwrap_or_else(PoisonError::into_inner).iter().find(|(held, _)| *held == key).and_then(|(_, pages)| pages.upgrade());
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
        opened.push((key, Arc::downgrade(&pages)));
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
    format!("{digest}:{}:{:x}", revision.length, recording.pages_id())
}

fn first_block_for(recording: &Recording, run: u32, from: u32) -> io::Result<u32> {
    if from == 0 {
        return Ok(0);
    }
    let tables = recording.tables();
    let mut found = vec![false; tables];
    let mut remaining = tables;
    let mut first = from;
    while first > 0 && remaining > 0 {
        first -= 1;
        if let Some(open) = recording.open_block_shared(run, first)? {
            for (table, entry) in open.tables.iter().enumerate() {
                if !found[table] && (entry.unit.restore.is_some() || entry.unit.first.is_some()) {
                    found[table] = true;
                    remaining -= 1;
                }
            }
        } else {
            for (table, seen) in found.iter_mut().enumerate() {
                if *seen {
                    continue;
                }
                let entry = recording.table_entry(run, first, table)?;
                if entry.unit.restore.is_some() || entry.unit.first.is_some() {
                    *seen = true;
                    remaining -= 1;
                }
            }
        }
    }
    Ok(first)
}

impl Pages for WorldPages {
    fn extents(&self, _run: usize) -> &dyn Extents {
        self.recording.extents()
    }

    fn runs(&self) -> &[Run] {
        &self.runs
    }

    fn table_entry(&self, run: usize, block: usize, table: usize) -> io::Result<EntryRef> {
        let &recorded = self.recorded.get(run).ok_or_else(|| invalid(format!("No run {run}")))?;
        let held = &self.runs[run];
        if block >= held.blocks() {
            return Err(invalid(format!("Run {run} holds no block {block}")));
        }
        let rec_block = self.first_blocks[run] + block as u32;
        self.recording.table_entry(recorded, rec_block, table).map(EntryRef::recorded)
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

    fn held_tables(&self, tables: usize) -> Vec<bool> {
        let available_tables = tables.min(self.recording.tables());
        let mut candidates: Vec<bool> = (0..tables)
            .map(|table| {
                table < available_tables
                    && self.kinds(table).is_some_and(|kinds| !kinds.is_empty())
                    && self.runs.iter().enumerate().any(|(run, held)| self.withheld_at(run, table, held.last_hour()).is_none())
            })
            .collect();
        let mut any_recorded = vec![false; tables];
        let mut run_restored = vec![false; tables];
        for (run_idx, run) in self.runs.iter().enumerate() {
            let recorded_run = self.recorded[run_idx];
            let first_block = self.first_blocks[run_idx];
            let end_block = first_block + run.blocks() as u32;
            let closed_blocks = self.recording.run(recorded_run).map_or(0, |held| held.directory().spans.len() as u32 * SPAN_BLOCKS);
            run_restored.fill(false);

            let closed_end = end_block.min(closed_blocks);
            if first_block < closed_end {
                let first_span = first_block / SPAN_BLOCKS;
                let last_span = (closed_end - 1) / SPAN_BLOCKS;
                for span in first_span..=last_span {
                    let span_start = (span * SPAN_BLOCKS).max(first_block);
                    let span_end = ((span + 1) * SPAN_BLOCKS).min(closed_end);
                    let mut any_needed = false;
                    for table in 0..available_tables {
                        if !candidates[table] || (run_restored[table] && any_recorded[table]) {
                            continue;
                        }
                        any_needed = true;
                        let Ok(row) = self.recording.table_span(recorded_run, span, table) else {
                            continue;
                        };
                        for rec_block in span_start..span_end {
                            let entry = &row.blocks[(rec_block % SPAN_BLOCKS) as usize].unit;
                            if entry.first.is_some() || entry.restore.is_some() {
                                run_restored[table] = true;
                            }
                            if !any_recorded[table] {
                                let local_block = rec_block - first_block;
                                if entry_recorded(entry, run.first_hour + (local_block * BLOCK) as Hour) {
                                    any_recorded[table] = true;
                                }
                            }
                            if run_restored[table] && any_recorded[table] {
                                break;
                            }
                        }
                    }
                    if !any_needed {
                        break;
                    }
                }
            }

            let open_start = first_block.max(closed_blocks);
            for rec_block in open_start..end_block {
                if !(0..available_tables).any(|table| candidates[table] && (!run_restored[table] || !any_recorded[table])) {
                    break;
                }
                let Ok(Some(open)) = self.recording.open_block_shared(recorded_run, rec_block) else {
                    continue;
                };
                let block_first = run.first_hour + ((rec_block - first_block) * BLOCK) as Hour;
                for table in 0..available_tables {
                    if !candidates[table] || (run_restored[table] && any_recorded[table]) {
                        continue;
                    }
                    let Some(entry) = open.tables.get(table) else {
                        continue;
                    };
                    if entry.unit.first.is_some() || entry.unit.restore.is_some() {
                        run_restored[table] = true;
                    }
                    if !any_recorded[table] && entry_recorded(&entry.unit, block_first) {
                        any_recorded[table] = true;
                    }
                }
            }

            for table in 0..available_tables {
                if !run_restored[table] {
                    candidates[table] = false;
                }
            }
        }
        (0..tables).map(|table| candidates[table] && any_recorded[table]).collect()
    }

    fn written_tables(&self, runs: &[usize], tables: usize) -> Vec<TableWritten> {
        let available_tables = tables.min(self.recording.tables());
        let mut written = vec![TableWritten::Unwritten; tables];
        let mut remaining = available_tables;
        for (table, state) in written[..available_tables].iter_mut().enumerate() {
            if self.kinds(table).is_none() {
                if let Some(reason) = self.recording.withheld_reason(table) {
                    *state = TableWritten::Withheld(reason.to_owned());
                }
                remaining -= 1;
            }
        }
        for &run_idx in runs {
            if remaining == 0 {
                break;
            }
            let Some(run) = self.runs.get(run_idx) else {
                continue;
            };
            if run.blocks() == 0 {
                continue;
            }
            let recorded_run = self.recorded[run_idx];
            let rec_block = self.first_blocks[run_idx];
            match self.recording.open_block_shared(recorded_run, rec_block) {
                Ok(Some(open)) => {
                    for table in 0..available_tables {
                        if matches!(written[table], TableWritten::Unwritten | TableWritten::Damaged(_))
                            && self.kinds(table).is_some()
                            && open.tables.get(table).is_some_and(|entry| entry.unit.first.is_some())
                        {
                            written[table] = TableWritten::Written;
                            remaining -= 1;
                        }
                    }
                }
                Ok(None) => {
                    for table in 0..available_tables {
                        if matches!(written[table], TableWritten::Written | TableWritten::Withheld(_)) || self.kinds(table).is_none() {
                            continue;
                        }
                        match self.recording.table_entry(recorded_run, rec_block, table) {
                            Ok(entry) if entry.unit.first.is_some() => {
                                written[table] = TableWritten::Written;
                                remaining -= 1;
                            }
                            Ok(_) => {}
                            Err(error) if matches!(written[table], TableWritten::Unwritten) => {
                                written[table] = TableWritten::Damaged(error.to_string());
                            }
                            Err(_) => {}
                        }
                    }
                }
                Err(error) => {
                    let reason = error.to_string();
                    for (table, state) in written[..available_tables].iter_mut().enumerate() {
                        if matches!(state, TableWritten::Unwritten) && self.kinds(table).is_some() {
                            *state = TableWritten::Damaged(reason.clone());
                        }
                    }
                }
            }
        }
        written
    }

    fn table_has_activity(&self, table: usize) -> bool {
        if table >= self.recording.tables() || self.kinds(table).is_none() {
            return false;
        }
        for (run_idx, run) in self.runs.iter().enumerate() {
            let recorded_run = self.recorded[run_idx];
            let first_block = self.first_blocks[run_idx];
            let end_block = first_block + run.blocks() as u32;
            let closed_blocks = self.recording.run(recorded_run).map_or(0, |held| held.directory().spans.len() as u32 * SPAN_BLOCKS);
            let closed_end = end_block.min(closed_blocks);
            if first_block < closed_end {
                let first_span = first_block / SPAN_BLOCKS;
                let last_span = (closed_end - 1) / SPAN_BLOCKS;
                for span in first_span..=last_span {
                    let span_start = (span * SPAN_BLOCKS).max(first_block);
                    let span_end = ((span + 1) * SPAN_BLOCKS).min(closed_end);
                    let Ok(row) = self.recording.table_span(recorded_run, span, table) else {
                        continue;
                    };
                    for rec_block in span_start..span_end {
                        let entry = &row.blocks[(rec_block % SPAN_BLOCKS) as usize].unit;
                        if !entry.parts.is_empty() || entry.first.is_some() || entry.restore.is_some() {
                            return true;
                        }
                    }
                }
            }
            for rec_block in first_block.max(closed_blocks)..end_block {
                if self.recording.table_entry(recorded_run, rec_block, table).is_ok_and(|entry| {
                    !entry.unit.parts.is_empty() || entry.unit.first.is_some() || entry.unit.restore.is_some()
                }) {
                    return true;
                }
            }
        }
        false
    }
}
