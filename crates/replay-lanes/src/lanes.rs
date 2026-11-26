
use replay_recording::{cells::TableReader, columnar_frames::Recording};
use replay_data::rows::TableId;
use replay_columnar::{
    envelope::{Extent, SPAN_BLOCKS, UNIT_ORDINALS},
    manifest::Status,
    state::Kind,
    store::{Strings, TableState, TableView},
    summary::{self, ColumnFrame, FANOUT, Fold, Piece, Shape},
    unit,
};
use rayon::prelude::*;
use std::{collections::HashMap, io};

pub use crate::points::Cell;

#[cfg(test)]
mod tests;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

#[derive(Clone, Copy, Debug)]
pub enum Keys<'a> {
    All,
    These(&'a [u128]),
    Range(u128, u128),
    Ranges(&'a [(u128, u128)]),
}

impl Keys<'_> {
    fn asks(&self, sorted: &[u128], key: u128) -> bool {
        match *self {
            Keys::All => true,
            Keys::These(_) => sorted.binary_search(&key).is_ok(),
            Keys::Range(low, high) => low <= key && key <= high,
            Keys::Ranges(ranges) => ranges.get(ranges.partition_point(|range| range.1 < key)).is_some_and(|range| range.0 <= key),
        }
    }
}

pub struct Snapshot {
    pub table: TableState,
    pub strings: Strings,
    pub kinds: Vec<Kind>,
    pub status: Status,
    pub sampled_at: Option<i32>,
    pub was_read: bool,
    pub rows: Vec<(u128, u32)>,
}

impl Snapshot {
    pub fn view(&self) -> TableView<'_> {
        TableView::new(&self.table, &self.kinds, &self.strings)
    }
}

fn ordinal_of(recording: &Recording, run: u32, hour: i32) -> io::Result<u32> {
    recording.run(run)?.ordinal(hour).ok_or_else(|| invalid("The run holds no such hour"))
}

pub fn at(recording: &Recording, run: u32, table: TableId, hour: i32, columns: Option<&[usize]>) -> io::Result<Snapshot> {
    at_keys(recording, run, table, hour, columns, Keys::All)
}

pub fn at_keys(recording: &Recording, run: u32, table: TableId, hour: i32, columns: Option<&[usize]>, keys: Keys<'_>) -> io::Result<Snapshot> {
    check_withheld_run(recording, run, table, hour, hour)?;
    let ordinal = ordinal_of(recording, run, hour)?;
    let mut reader = TableReader::new(recording, run, table, columns)?;
    reader.seek(recording, ordinal)?;
    let rows = asked_rows(&reader, keys)?;
    let (status, sampled_at, was_read) = (reader.status(), reader.sampled_at(), reader.was_read());
    let (table, strings, kinds) = reader.into_parts();
    Ok(Snapshot { table, strings, kinds, status, sampled_at, was_read, rows })
}

pub(crate) fn asked_rows(reader: &TableReader, keys: Keys<'_>) -> io::Result<Vec<(u128, u32)>> {
    let table = reader.table();
    let mut sorted = Vec::new();
    if let Keys::These(asked) = keys {
        sorted.extend_from_slice(asked);
        sorted.sort_unstable();
    }
    let view = reader.view();
    let mut rows: Vec<(u128, u32)> = Vec::new();
    let mut keep = |slot: u32| -> io::Result<()> {
        let key = view.key(slot).ok_or_else(|| invalid(format!("{table:?} slot {slot} holds no row")))?;
        if keys.asks(&sorted, key) {
            rows.push((key, slot));
        }
        Ok(())
    };
    match reader.status() {
        Status::Complete => view.rows().try_for_each(&mut keep)?,
        Status::Truncated { .. } => reader.read_rows().unwrap_or_default().iter().try_for_each(|&slot| keep(slot))?,
        _ => {}
    }
    rows.sort_unstable();
    Ok(rows)
}

pub fn at_each_with_status(
    recording: &Recording,
    run: u32,
    table: TableId,
    columns: Option<&[usize]>,
    asks: &[(i32, u128)],
    each: &mut dyn FnMut(i32, u128, Status, Option<&str>, Option<(TableView<'_>, u32)>) -> io::Result<()>,
) -> io::Result<()> {
    let mut asks = asks.to_vec();
    asks.sort_unstable();
    asks.dedup();
    if let (Some(first), Some(last)) = (asks.first(), asks.last()) {
        if let Some(valid) = check_withheld_run(recording, run, table, first.0, last.0)?
            && first.0 < valid
        {
            check_withheld_run(recording, run, table, first.0, first.0)?;
        }
    }
    let mut reader = TableReader::new(recording, run, table, columns)?;
    reader.index_keys();
    for hour_asks in asks.chunk_by(|a, b| a.0 == b.0) {
        let hour = hour_asks[0].0;
        reader.go_to(recording, ordinal_of(recording, run, hour)?)?;
        let (status, view, read, available) = (reader.status(), reader.view(), reader.read_rows(), reader.available());
        let unavailable_reason = match status {
            Status::Unavailable { reason } => view.string(reason).and_then(|text| std::str::from_utf8(text).ok()),
            _ => None,
        };
        for &(_, key) in hour_asks {
            let row = view.slot(key).filter(|&slot| available && read.is_none_or(|read| read.binary_search(&slot).is_ok()));
            each(hour, key, status, unavailable_reason, row.map(|slot| (view, slot)))?;
        }
    }
    Ok(())
}

pub fn at_each(
    recording: &Recording,
    run: u32,
    table: TableId,
    columns: Option<&[usize]>,
    asks: &[(i32, u128)],
    each: &mut dyn FnMut(i32, u128, Option<(TableView<'_>, u32)>) -> io::Result<()>,
) -> io::Result<()> {
    at_each_with_status(recording, run, table, columns, asks, &mut |hour, key, _, _, row| each(hour, key, row))
}

pub type Runs = Vec<(i32, i32, Option<i64>)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoveredValue {
    Value(i64),
    Unvalued,
    Uncovered,
}

pub type CoveredRuns = Vec<(i32, i32, CoveredValue)>;

pub fn runs(recording: &Recording, run: u32, table: TableId, column: usize, from: i32, to: i32) -> io::Result<Vec<(u128, Runs)>> {
    let held = keyed_runs(recording, run, table, summary::Source::Cell(column), None, from, to)?;
    let mut out: Vec<(u128, Runs)> = held.into_iter().collect();
    out.sort_unstable_by_key(|(key, _)| *key);
    Ok(out)
}

pub fn keyed_runs(
    recording: &Recording,
    run: u32,
    table: TableId,
    source: summary::Source,
    keys: Option<&[u128]>,
    from: i32,
    to: i32,
) -> io::Result<HashMap<u128, Runs, foldhash::fast::RandomState>> {
    collect_keyed_runs(recording, run, table, source, keys, from, to, crate::points::points)
}

pub fn keyed_witnessed_runs(
    recording: &Recording,
    run: u32,
    table: TableId,
    source: summary::Source,
    keys: Option<&[u128]>,
    from: i32,
    to: i32,
) -> io::Result<HashMap<u128, Runs, foldhash::fast::RandomState>> {
    collect_keyed_runs(recording, run, table, source, keys, from, to, crate::points::witnessed_points)
}

type PointWalker = fn(&Recording, u32, TableId, &[usize], Option<summary::Source>, Keys<'_>, i32, i32, &mut dyn FnMut(crate::points::Point) -> bool) -> io::Result<()>;

fn collect_keyed_runs(
    recording: &Recording,
    run: u32,
    table: TableId,
    source: summary::Source,
    keys: Option<&[u128]>,
    from: i32,
    to: i32,
    walk: PointWalker,
) -> io::Result<HashMap<u128, Runs, foldhash::fast::RandomState>> {
    let mut held: HashMap<u128, Runs, foldhash::fast::RandomState> = HashMap::default();
    walk(recording, run, table, &inputs(source), Some(source), keys.map_or(Keys::All, Keys::These), from, to, &mut |point| {
        let runs = held.entry(point.key).or_default();
        let value = if point.covered { point.value } else { None };
        match runs.last_mut() {
            Some((_, _, now)) if *now == value => {}
            Some((_, last, _)) => {
                *last = point.hour - 1;
                runs.push((point.hour, point.hour, value));
            }
            None => runs.push((point.hour, point.hour, value)),
        }
        true
    })?;
    for runs in held.values_mut() {
        if let Some((_, last, _)) = runs.last_mut() {
            *last = to;
        }
    }
    Ok(held)
}

pub fn keyed_covered_runs(
    recording: &Recording,
    run: u32,
    table: TableId,
    source: summary::Source,
    keys: Option<&[u128]>,
    from: i32,
    to: i32,
) -> io::Result<(HashMap<u128, CoveredRuns, foldhash::fast::RandomState>, Vec<(i32, i32)>)> {
    let mut held: HashMap<u128, CoveredRuns, foldhash::fast::RandomState> = HashMap::default();
    let complete = crate::points::points_with_complete(recording, run, table, &inputs(source), Some(source), keys.map_or(Keys::All, Keys::These), from, to, &mut |point| {
        let runs = held.entry(point.key).or_default();
        let value = match (point.covered, point.value) {
            (true, Some(value)) => CoveredValue::Value(value),
            (true, None) => CoveredValue::Unvalued,
            (false, _) => CoveredValue::Uncovered,
        };
        match runs.last_mut() {
            Some((_, _, now)) if *now == value => {}
            Some((_, last, _)) => {
                *last = point.hour - 1;
                runs.push((point.hour, point.hour, value));
            }
            None => runs.push((point.hour, point.hour, value)),
        }
        true
    })?;
    for runs in held.values_mut() {
        if let Some((_, last, _)) = runs.last_mut() {
            *last = to;
        }
    }
    Ok((held, complete))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub hour: i32,
    pub cells: Option<Box<[Cell]>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct History {
    pub columns: Vec<usize>,
    pub to: i32,
    pub texts: Vec<Box<[u8]>>,
    pub keys: Vec<(u128, Vec<Step>)>,
    pub statuses: Vec<StatusRun>,
    pub reasons: HashMap<u32, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reading<'a> {
    Cells(&'a [Cell]),
    Absent,
    Unread { status: Status, reason: Option<&'a str> },
}

fn push_reading<'a>(out: &mut Vec<(i32, i32, Reading<'a>)>, first: i32, last: i32, reading: Reading<'a>) {
    if first > last {
        return;
    }
    match out.last_mut() {
        Some(prev) if prev.2 == reading && prev.1 + 1 == first => prev.1 = last,
        _ => out.push((first, last, reading)),
    }
}

impl History {
    fn steps(&self, key: u128) -> &[Step] {
        self.keys.binary_search_by_key(&key, |(held, _)| *held).map_or(&[], |at| &self.keys[at].1)
    }

    fn without_cells(&self, status: Option<Status>) -> Reading<'_> {
        match status {
            Some(Status::Complete) => Reading::Absent,
            Some(status @ Status::Unavailable { reason }) => Reading::Unread { status, reason: self.reasons.get(&reason).map(String::as_str) },
            Some(status) => Reading::Unread { status, reason: None },
            None => Reading::Unread { status: Status::NotRecorded, reason: None },
        }
    }

    pub fn reading(&self, key: u128, hour: i32) -> Reading<'_> {
        let at = self.statuses.partition_point(|held| held.last < hour);
        let status = self.statuses.get(at).filter(|held| held.first <= hour).map(|held| held.status);
        if status.is_none() {
            return self.without_cells(None);
        }
        let steps = self.steps(key);
        match steps.partition_point(|step| step.hour <= hour).checked_sub(1).and_then(|at| steps[at].cells.as_deref()) {
            Some(cells) => Reading::Cells(cells),
            None => self.without_cells(status),
        }
    }

    pub fn readings(&self, key: u128) -> Vec<(i32, i32, Reading<'_>)> {
        let (Some(start), Some(end)) = (self.statuses.first().map(|held| held.first), self.statuses.last().map(|held| held.last.min(self.to))) else { return Vec::new() };
        let steps = self.steps(key);
        let mut out = Vec::new();
        let mut held = steps.first().map_or(end + 1, |step| step.hour);
        let mut stretches = vec![(start, held - 1, None)];
        for (index, step) in steps.iter().enumerate() {
            held = steps.get(index + 1).map_or(end + 1, |next| next.hour);
            stretches.push((step.hour.max(start), held.min(end + 1) - 1, step.cells.as_deref()));
        }
        for (first, last, cells) in stretches {
            if let Some(cells) = cells {
                push_reading(&mut out, first, last, Reading::Cells(cells));
                continue;
            }
            let (mut hour, mut at) = (first, self.statuses.partition_point(|run| run.last < first));
            while hour <= last {
                let (until, status) = match self.statuses.get(at) {
                    Some(run) if run.first <= hour => (run.last.min(last), Some(run.status)),
                    Some(run) => ((run.first - 1).min(last), None),
                    None => (last, None),
                };
                push_reading(&mut out, hour, until, self.without_cells(status));
                if status.is_some() {
                    at += 1;
                }
                hour = until + 1;
            }
        }
        out
    }

    pub fn text(&self, cell: Cell) -> Option<&[u8]> {
        match cell {
            Cell::Text(index) => self.texts.get(index as usize).map(|text| &**text),
            _ => None,
        }
    }

    pub fn runs(&self, column: usize) -> Option<Vec<(u128, Vec<(i32, i32, Option<Cell>)>)>> {
        let at = self.columns.binary_search(&column).ok()?;
        Some(
            self.keys
                .iter()
                .map(|(key, steps)| {
                    let mut runs: Vec<(i32, i32, Option<Cell>)> = Vec::new();
                    for (index, step) in steps.iter().enumerate() {
                        let cell = step.cells.as_ref().map(|cells| cells[at]);
                        let last = steps.get(index + 1).map_or(self.to, |next| next.hour - 1);
                        match runs.last_mut() {
                            Some(run) if run.2 == cell => run.1 = last,
                            _ => runs.push((step.hour, last, cell)),
                        }
                    }
                    (*key, runs)
                })
                .collect(),
        )
    }
}

pub fn table_statuses(recording: &Recording, run: u32, table: TableId) -> io::Result<Vec<StatusRun>> {
    crate::points::table_statuses(recording, run, table)
}

pub fn history(recording: &Recording, run: u32, table: TableId, columns: &[usize], keys: Keys<'_>, from: i32, to: i32) -> io::Result<History> {
    let mut held: HashMap<u128, Vec<Step>, foldhash::fast::RandomState> = HashMap::default();
    let mut statuses = Vec::new();
    let (columns, texts) = crate::points::cells(recording, run, table, columns, keys, from, to, Some(&mut statuses), &mut |hour, key, cells| {
        held.entry(key).or_default().push(Step { hour, cells: cells.map(Box::from) });
        true
    })?;
    let reasons = status_reasons(recording, run, table, &statuses)?;
    let mut keys: Vec<(u128, Vec<Step>)> = held.into_iter().collect();
    keys.sort_unstable_by_key(|(key, _)| *key);
    Ok(History { columns, to, texts, keys, statuses, reasons })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogGap {
    pub first: i32,
    pub last: i32,
    pub status: Status,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogSpan {
    pub columns: Vec<usize>,
    pub texts: Vec<Box<[u8]>>,
    pub gaps: Vec<LogGap>,
}

impl LogSpan {
    pub fn text(&self, cell: Cell) -> Option<&[u8]> {
        match cell {
            Cell::Text(index) => self.texts.get(index as usize).map(|text| &**text),
            _ => None,
        }
    }
}

fn cell_number(kind: Kind, bits: u64) -> i64 {
    match kind {
        Kind::U8 => i64::from(bits as u8),
        Kind::I8 => i64::from(bits as i8),
        Kind::U16 => i64::from(bits as u16),
        Kind::I16 => i64::from(bits as i16),
        Kind::U32 | Kind::OptionalU32 => i64::from(bits as u32),
        Kind::I32 | Kind::OptionalI32 => i64::from(bits as i32),
        Kind::Bool => (bits & 1) as i64,
        _ => bits as i64,
    }
}

fn resolve_log_reasons(
    recording: &Recording,
    held: &replay_recording::columnar_frames::Run,
    pages: &[unit::BlockPage],
    kinds: &[Kind],
    table: usize,
    to: i32,
    needed: &[(usize, Vec<u32>)],
) -> io::Result<HashMap<u32, String>> {
    let mut reasons: HashMap<u32, String> = HashMap::new();
    let source = recording.extents();
    for (page_idx, ids) in needed {
        let missing: Vec<u32> = ids.iter().copied().filter(|id| !reasons.contains_key(id)).collect();
        if missing.is_empty() {
            continue;
        }
        let page = &pages[*page_idx];
        let block_first = held.block_first_hour(page.block);
        let entry = &page.tables[table];
        if let Some(last) = entry.parts.iter().rposition(|part| block_first + i32::from(part.first) <= to) {
            let mut prefix = None;
            for part_idx in 0..=last {
                let part_entry = &entry.parts[part_idx];
                let part_last = entry.parts.get(part_idx + 1).map_or(page.hours - 1, |following| following.first - 1);
                let fetched = unit::reader::fetch(source, part_entry, part_last, &[], prefix.as_ref())?;
                for (id, bytes) in fetched.strings() {
                    if missing.contains(id) {
                        reasons.entry(*id).or_insert_with(|| String::from_utf8_lossy(bytes).into_owned());
                    }
                }
                if missing.iter().all(|id| reasons.contains_key(id)) {
                    break;
                }
                prefix = Some(fetched.prefix());
            }
        }
        if !missing.iter().all(|id| reasons.contains_key(id)) {
            let prior = if *page_idx == 0 { entry.first.as_ref() } else { pages[*page_idx - 1].tables[table].restore.as_ref() };
            for restore in [prior, entry.restore.as_ref()].into_iter().flatten() {
                let (_, meta) = unit::reader::restore(source, restore, kinds, &[])?;
                for (id, bytes) in meta.strings {
                    if missing.contains(&id) {
                        reasons.entry(id).or_insert_with(|| String::from_utf8_lossy(&bytes).into_owned());
                    }
                }
                if missing.iter().all(|id| reasons.contains_key(id)) {
                    break;
                }
            }
        }
    }
    Ok(reasons)
}

pub fn log_span(
    recording: &Recording,
    run: u32,
    table: TableId,
    columns: &[usize],
    keys: Keys<'_>,
    from: i32,
    to: i32,
    each: &mut dyn FnMut(i32, u128, &[Cell]),
) -> io::Result<LogSpan> {
    let index = table.index();
    let kinds = replay_recording::schema::schema()?.swap_remove(index);
    if let Some(&column) = columns.iter().find(|&&column| column >= kinds.len()) {
        return Err(invalid(format!("{table:?} has no column {column}")));
    }
    if recording.predates(index).is_some() {
        return Ok(LogSpan { columns: columns.to_vec(), texts: Vec::new(), gaps: Vec::new() });
    }
    let valid = check_withheld_run(recording, run, table, from, to)?;
    let held = recording.run(run)?;
    if held.ordinals == 0 {
        return Ok(LogSpan { columns: columns.to_vec(), texts: Vec::new(), gaps: Vec::new() });
    }
    let (from, to) = (from.max(held.first_hour).max(valid.unwrap_or(i32::MIN)), to.min(held.last_hour()));
    if from > to {
        return Ok(LogSpan { columns: columns.to_vec(), texts: Vec::new(), gaps: Vec::new() });
    }
    let (Some(first), Some(last)) = (held.ordinal(from), held.ordinal(to)) else {
        return Ok(LogSpan { columns: columns.to_vec(), texts: Vec::new(), gaps: Vec::new() });
    };
    let recorded = recording.columns()[index].min(kinds.len());
    let mut read: Vec<usize> = columns.iter().copied().filter(|&column| column < recorded).collect();
    read.sort_unstable();
    read.dedup();
    let first_block = held.place(first).0;
    let last_block = held.place(last).0;
    let pages = recording.table_pages(run, index, first_block.saturating_sub(1), last_block + 1)?;
    if !unit::reader::class_of(&pages, index)? {
        return Err(invalid(format!("{table:?} holds state: it has no rows by hour to read as a log")));
    }
    let mut status_runs: Vec<StatusRun> = Vec::new();
    let mut needed_reasons: Vec<(usize, Vec<u32>)> = Vec::new();
    for (page_idx, page) in pages.iter().enumerate() {
        if page.block < first_block {
            continue;
        }
        let mut page_reasons = Vec::new();
        unit::reader::statuses(&page.tables[index], page.hours, held.block_first_hour(page.block), &mut |start, end, status| {
            let (a, b) = (start.max(from), end.min(to));
            if a > b {
                return;
            }
            if let Status::Unavailable { reason } = status
                && !page_reasons.contains(&reason)
            {
                page_reasons.push(reason);
            }
            match status_runs.last_mut() {
                Some(held) if held.status == status && held.last + 1 == a => held.last = b,
                _ => status_runs.push(StatusRun { first: a, last: b, status }),
            }
        })?;
        if !page_reasons.is_empty() {
            needed_reasons.push((page_idx, page_reasons));
        }
    }
    let reasons = if needed_reasons.is_empty() {
        HashMap::new()
    } else {
        resolve_log_reasons(recording, held, &pages, &kinds[..recorded], index, to, &needed_reasons)?
    };
    let mut gaps: Vec<LogGap> = Vec::new();
    let mut available_runs: Vec<(i32, i32)> = Vec::new();
    for run in &status_runs {
        if replay_recording::columnar_frames::available(run.status) {
            match available_runs.last_mut() {
                Some((_, last)) if *last + 1 == run.first => *last = run.last,
                _ => available_runs.push((run.first, run.last)),
            }
        }
        if run.status != Status::Complete {
            let reason = match run.status {
                Status::Unavailable { reason } => reasons.get(&reason).cloned(),
                _ => None,
            };
            match gaps.last_mut() {
                Some(held) if held.status == run.status && held.reason == reason && held.last + 1 == run.first => held.last = run.last,
                _ => gaps.push(LogGap { first: run.first, last: run.last, status: run.status, reason }),
            }
        }
    }
    if available_runs.is_empty() {
        return Ok(LogSpan { columns: columns.to_vec(), texts: Vec::new(), gaps });
    }
    let mut sorted = Vec::new();
    if let Keys::These(asked) = keys {
        sorted.extend_from_slice(asked);
        sorted.sort_unstable();
        sorted.dedup();
    }
    struct LogSink<'a> {
        table: TableId,
        columns: &'a [usize],
        kinds: &'a [Kind],
        recorded: usize,
        named: HashMap<u32, Box<[u8]>, foldhash::fast::RandomState>,
        texts: Vec<Box<[u8]>>,
        by_text: HashMap<Box<[u8]>, u32, foldhash::fast::RandomState>,
        by_id: HashMap<u32, u32, foldhash::fast::RandomState>,
        out: Vec<Cell>,
        failed: Option<io::Error>,
    }

    impl LogSink<'_> {
        fn name(&mut self, id: u32, text: &[u8]) {
            self.named.entry(id).or_insert_with(|| text.into());
        }

        fn fail(&mut self, error: io::Error) {
            if self.failed.is_none() {
                self.failed = Some(error);
            }
        }

        fn emit(&mut self, hour: i32, key: u128, rows: &TableState, at: u32, each: &mut dyn FnMut(i32, u128, &[Cell])) {
            if self.failed.is_some() {
                return;
            }
            self.out.clear();
            for &column in self.columns {
                let cell = if column >= self.recorded {
                    Cell::Unrecorded
                } else {
                    match rows.value(column, at) {
                        replay_columnar::store::Value::Number(bits) => Cell::Number(cell_number(self.kinds[column], bits)),
                        replay_columnar::store::Value::Missing => Cell::Missing,
                        replay_columnar::store::Value::Unrecorded => Cell::Unrecorded,
                        replay_columnar::store::Value::Text(id) => match self.by_id.get(&id) {
                            Some(&text_index) => Cell::Text(text_index),
                            None => {
                                let Some(bytes) = self.named.get(&id) else {
                                    self.fail(invalid(format!("{:?} names string {id}, which was not supplied by its log", self.table)));
                                    return;
                                };
                                let text_index = match self.by_text.get(bytes) {
                                    Some(&index) => index,
                                    None => {
                                        let index = self.texts.len() as u32;
                                        self.texts.push(bytes.clone());
                                        self.by_text.insert(bytes.clone(), index);
                                        index
                                    }
                                };
                                self.by_id.insert(id, text_index);
                                Cell::Text(text_index)
                            }
                        },
                        replay_columnar::store::Value::List(_) => Cell::Written(hour),
                    }
                };
                self.out.push(cell);
            }
            each(hour, key, &self.out);
        }
    }

    let mut sink = LogSink {
        table,
        columns,
        kinds: &kinds,
        recorded,
        named: HashMap::default(),
        texts: Vec::new(),
        by_text: HashMap::default(),
        by_id: HashMap::default(),
        out: Vec::with_capacity(columns.len()),
        failed: None,
    };
    let mut avail_at = 0usize;
    let mut is_available = |hour: i32| -> bool {
        while avail_at < available_runs.len() && available_runs[avail_at].1 < hour {
            avail_at += 1;
        }
        available_runs.get(avail_at).is_some_and(|&(a, b)| a <= hour && hour <= b)
    };
    if matches!(keys, Keys::All) && !read.is_empty() {
        unit::reader::log_span(recording.extents(), &pages, &kinds[..recorded], index, &read, held.first_hour, from..=to, &mut |hour, event| match event {
            unit::reader::LogEvent::Named { id, text } => sink.name(id, text),
            unit::reader::LogEvent::Appended { key, rows, at, .. } => {
                if is_available(hour) {
                    sink.emit(hour, key, rows, at, each);
                }
            }
            unit::reader::LogEvent::Forgotten { .. } => {}
        })?;
    } else {
        let mut ordinals: Vec<u64> = Vec::new();
        let mut matched_hours: Vec<i32> = Vec::new();
        unit::reader::log_span(recording.extents(), &pages, &kinds[..recorded], index, &[], held.first_hour, from..=to, &mut |hour, event| {
            let unit::reader::LogEvent::Appended { ordinal, key, rows, at, .. } = event else { return };
            if is_available(hour) && keys.asks(&sorted, key) {
                if read.is_empty() {
                    sink.emit(hour, key, rows, at, each);
                } else {
                    ordinals.push(ordinal);
                    matched_hours.push(hour);
                }
            }
        })?;
        if !ordinals.is_empty() {
            let mut matched_idx = 0usize;
            unit::reader::log_rows_at(recording.extents(), &pages, &kinds[..recorded], index, &read, &ordinals, &mut |event| match event {
                unit::reader::LogEvent::Named { id, text } => sink.name(id, text),
                unit::reader::LogEvent::Appended { ordinal, key, rows, at, .. } => {
                    if sink.failed.is_some() {
                        return;
                    }
                    if ordinals.get(matched_idx) != Some(&ordinal) {
                        sink.fail(invalid(format!("{table:?} returned log ordinal {ordinal} out of order")));
                        return;
                    }
                    let hour = matched_hours[matched_idx];
                    matched_idx += 1;
                    sink.emit(hour, key, rows, at, each);
                }
                unit::reader::LogEvent::Forgotten { .. } => {}
            })?;
            if sink.failed.is_none() && matched_idx != ordinals.len() {
                return Err(invalid(format!("{table:?} returned {matched_idx} of {} selected log rows", ordinals.len())));
            }
        }
    }
    if let Some(error) = sink.failed {
        return Err(error);
    }
    Ok(LogSpan { columns: columns.to_vec(), texts: sink.texts, gaps })
}

fn check_withheld_run(recording: &Recording, run: u32, table: TableId, _from: i32, to: i32) -> io::Result<Option<i32>> {
    let index = table.index();
    if recording.predates(index).is_some() {
        return Ok(None);
    }
    match recording.table_coverage(index, run) {
        replay_recording::columnar_frames::TableCoverage::Valid => Ok(None),
        replay_recording::columnar_frames::TableCoverage::WithheldAll(reason) => Err(replay_recording::columnar_frames::Damage::wrap(
            replay_recording::columnar_frames::DamageScope::Withheld { table: index as u32 },
            invalid(reason.to_owned()),
        )),
        replay_recording::columnar_frames::TableCoverage::WithheldBefore { ordinal, reason } => {
            let valid = recording.run(run)?.first_hour + ordinal as i32;
            if to < valid {
                return Err(replay_recording::columnar_frames::Damage::wrap(
                    replay_recording::columnar_frames::DamageScope::Withheld { table: index as u32 },
                    invalid(reason.to_owned()),
                ));
            }
            Ok(Some(valid))
        }
    }
}

pub fn reads(recording: &Recording, run: u32, table: TableId, from: i32, to: i32) -> io::Result<Vec<(i32, i32)>> {
    let valid = check_withheld_run(recording, run, table, from, to)?;
    let held = recording.run(run)?;
    let (from, to) = (from.max(held.first_hour).max(valid.unwrap_or(i32::MIN)), to.min(held.last_hour()));
    let (Some(first), Some(last)) = (held.ordinal(from), held.ordinal(to)) else { return Ok(Vec::new()) };
    if first > last {
        return Ok(Vec::new());
    }
    let blocks = held.place(first).0..held.place(last).0 + 1;
    let found = blocks
        .into_par_iter()
        .map(|block| -> io::Result<Vec<(i32, i32)>> {
            let entry = recording.table_entry(run, block, table.index())?.unit;
            let mut runs = Vec::new();
            unit::reader::reads(&entry, held.block(block as usize).1 as u16, held.block_first_hour(block), &mut |start, end| {
                let (start, end) = (start.max(from), end.min(to));
                if start <= end {
                    runs.push((start, end));
                }
            })?;
            Ok(runs)
        })
        .collect::<io::Result<Vec<_>>>()?;
    let mut out: Vec<(i32, i32)> = Vec::new();
    for (start, end) in found.into_iter().flatten() {
        match out.last_mut() {
            Some((_, last)) if *last + 1 >= start => *last = (*last).max(end),
            _ => out.push((start, end)),
        }
    }
    Ok(out)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatusRun {
    pub first: i32,
    pub last: i32,
    pub status: Status,
}

pub fn available_reads(recording: &Recording, run: u32, table: TableId, from: i32, to: i32) -> io::Result<Vec<(i32, i32)>> {
    reads_where(recording, run, table, from, to, replay_recording::columnar_frames::available)
}

pub fn complete_reads(recording: &Recording, run: u32, table: TableId, from: i32, to: i32) -> io::Result<Vec<(i32, i32)>> {
    reads_where(recording, run, table, from, to, replay_recording::columnar_frames::complete)
}

fn reads_where(recording: &Recording, run: u32, table: TableId, from: i32, to: i32, kept: fn(Status) -> bool) -> io::Result<Vec<(i32, i32)>> {
    let reads = reads(recording, run, table, from, to)?;
    let available: Vec<(i32, i32)> = statuses(recording, run, table, from, to)?
        .into_iter()
        .filter(|held| kept(held.status))
        .map(|held| (held.first, held.last))
        .collect();
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < reads.len() && j < available.len() {
        let (first, last) = (reads[i].0.max(available[j].0), reads[i].1.min(available[j].1));
        if first <= last {
            out.push((first, last));
        }
        if reads[i].1 < available[j].1 { i += 1 } else { j += 1 }
    }
    Ok(out)
}

pub fn statuses(recording: &Recording, run: u32, table: TableId, from: i32, to: i32) -> io::Result<Vec<StatusRun>> {
    let valid = check_withheld_run(recording, run, table, from, to)?;
    let held = recording.run(run)?;
    let (from, to) = (from.max(held.first_hour).max(valid.unwrap_or(i32::MIN)), to.min(held.last_hour()));
    let (Some(first), Some(last)) = (held.ordinal(from), held.ordinal(to)) else { return Ok(Vec::new()) };
    if first > last {
        return Ok(Vec::new());
    }
    let blocks = held.place(first).0..held.place(last).0 + 1;
    let read_block = |block: u32| -> io::Result<Vec<StatusRun>> {
        let entry = recording.table_entry(run, block, table.index())?.unit;
        let mut runs = Vec::new();
        unit::reader::statuses(&entry, held.block(block as usize).1 as u16, held.block_first_hour(block), &mut |start, end, status| {
            let (first, last) = (start.max(from), end.min(to));
            if first <= last {
                runs.push(StatusRun { first, last, status });
            }
        })?;
        Ok(runs)
    };
    let found = if unit::reader::counting_decodes() {
        blocks.map(read_block).collect::<io::Result<Vec<_>>>()?
    } else {
        blocks.into_par_iter().map(read_block).collect::<io::Result<Vec<_>>>()?
    };
    let mut out: Vec<StatusRun> = Vec::new();
    for run in found.into_iter().flatten() {
        match out.last_mut() {
            Some(held) if held.status == run.status && held.last + 1 == run.first => held.last = run.last,
            _ => out.push(run),
        }
    }
    Ok(out)
}

pub fn status_reasons(recording: &Recording, run: u32, table: TableId, runs: &[StatusRun]) -> io::Result<HashMap<u32, String>> {
    let mut unavailable: Vec<(i32, i32, u32)> = Vec::new();
    for item in runs {
        if let Status::Unavailable { reason } = item.status {
            unavailable.push((item.first, item.last, reason));
        }
    }
    if unavailable.is_empty() {
        return Ok(HashMap::new());
    }
    let held = recording.run(run)?;
    let index = table.index();
    let kinds = replay_recording::schema::schema()?.swap_remove(index);
    let recorded = recording.columns()[index].min(kinds.len());
    let mut min_block = u32::MAX;
    let mut max_block = 0u32;
    let mut max_to = i32::MIN;
    let mut block_reasons: Vec<(u32, u32, u32)> = Vec::new();
    for &(first, last, reason) in &unavailable {
        let (Some(a), Some(b)) = (held.ordinal(first), held.ordinal(last)) else { continue };
        let (ba, bb) = (held.place(a).0, held.place(b).0);
        min_block = min_block.min(ba);
        max_block = max_block.max(bb);
        max_to = max_to.max(last);
        block_reasons.push((ba, bb, reason));
    }
    if block_reasons.is_empty() {
        return Ok(HashMap::new());
    }
    let pages = recording.table_pages(run, index, min_block.saturating_sub(1), max_block + 1)?;
    let mut needed: Vec<(usize, Vec<u32>)> = Vec::new();
    for (page_idx, page) in pages.iter().enumerate() {
        if page.block < min_block {
            continue;
        }
        let mut ids = Vec::new();
        for &(ba, bb, reason) in &block_reasons {
            if ba <= page.block && page.block <= bb && !ids.contains(&reason) {
                ids.push(reason);
            }
        }
        if !ids.is_empty() {
            needed.push((page_idx, ids));
        }
    }
    let mut reasons = resolve_log_reasons(recording, held, &pages, &kinds[..recorded], index, max_to, &needed)?;
    let mut unresolved: Vec<(i32, u32)> = unavailable
        .iter()
        .filter(|&&(_, _, reason)| !reasons.contains_key(&reason))
        .map(|&(first, _, reason)| (first, reason))
        .collect();
    if !unresolved.is_empty() {
        unresolved.sort_unstable_by_key(|&(first, _)| first);
        let mut reader = TableReader::new(recording, run, table, Some(&[]))?;
        for (first, reason) in unresolved {
            if reasons.contains_key(&reason) {
                continue;
            }
            let Some(ordinal) = held.ordinal(first) else { continue };
            reader.go_to(recording, ordinal)?;
            if let Some(bytes) = reader.view().string(reason) {
                reasons.insert(reason, String::from_utf8_lossy(bytes).into_owned());
            }
        }
    }
    Ok(reasons)
}

pub fn logged_hours(recording: &Recording, run: u32, table: TableId, from: i32, to: i32) -> io::Result<Vec<(i32, i32)>> {
    let held = recording.run(run)?;
    let (from, to) = (from.max(held.first_hour), to.min(held.last_hour()));
    if from > to {
        return Ok(Vec::new());
    }
    let first = ordinal_of(recording, run, from)? / UNIT_ORDINALS;
    let last = ordinal_of(recording, run, to)? / UNIT_ORDINALS;
    let extents = pieces(recording, run, table.index(), first..last + 1, 0)?;
    let found = extents
        .par_iter()
        .map(|extent| -> io::Result<Vec<(i64, i64)>> {
            let bytes = recording.stored_bytes(extent)?;
            let piece = Piece::read(&bytes)?;
            let frame = piece.column(0)?;
            let size = summary::bucket_hours(piece.directory.level);
            let mut held = vec![false; piece.directory.buckets as usize];
            for at in 0..piece.keys.len() {
                for (bucket, count) in frame.counts(&piece, at)?.into_iter().enumerate() {
                    if count > 0
                        && let Some(held) = held.get_mut(bucket)
                    {
                        *held = true;
                    }
                }
            }
            Ok(held
                .iter()
                .enumerate()
                .filter(|(_, held)| **held)
                .map(|(bucket, _)| {
                    let start = (piece.directory.first_bucket + bucket as i64) * size;
                    (start, start + size - 1)
                })
                .collect())
        })
        .collect::<io::Result<Vec<_>>>()?;
    let mut buckets: Vec<(i64, i64)> = found.into_iter().flatten().collect();
    buckets.sort_unstable();
    buckets.dedup();
    let mut out: Vec<(i32, i32)> = Vec::new();
    for (start, end) in buckets {
        let (start, end) = (start.max(i64::from(from)), end.min(i64::from(to)));
        if start > end {
            continue;
        }
        let (start, end) = (start as i32, end as i32);
        match out.last_mut() {
            Some((_, last)) if *last + 1 >= start => *last = (*last).max(end),
            _ => out.push((start, end)),
        }
    }
    Ok(out)
}

pub(crate) fn pieces(recording: &Recording, run: u32, table: usize, blocks: std::ops::Range<u32>, level: u8) -> io::Result<Vec<Extent>> {
    let held = recording.run(run)?;
    let closed = held.directory().spans.len() as u32 * SPAN_BLOCKS;
    let mut out = Vec::new();
    cover(recording, run, table, blocks.start..blocks.end.min(closed), level, closed, &mut out)?;
    if blocks.end > closed {
        let tail = held.directory().tail.iter().find(|(held, _)| *held as usize == table);
        match tail {
            Some((_, extent)) if level > 0 => out.push(*extent),
            _ => cover(recording, run, table, blocks.start.max(closed)..blocks.end, 0, closed, &mut out)?,
        }
    }
    Ok(out)
}

fn cover(
    recording: &Recording,
    run: u32,
    table: usize,
    blocks: std::ops::Range<u32>,
    level: u8,
    closed: u32,
    out: &mut Vec<Extent>,
) -> io::Result<()> {
    if blocks.is_empty() {
        return Ok(());
    }
    if level == 0 {
        let found = blocks
            .into_par_iter()
            .map(|block| recording.block_of(run, block as usize, table))
            .collect::<io::Result<Vec<_>>>()?;
        out.extend(found.into_iter().flatten());
        return Ok(());
    }
    let group = u64::from(SPAN_BLOCKS) * (FANOUT as u64).saturating_pow(u32::from(level) - 1);
    if group > u64::from(closed) {
        return cover(recording, run, table, blocks, level - 1, closed, out);
    }
    let group = group as u32;
    let groups: Vec<u32> = (blocks.start - blocks.start % group..blocks.end).step_by(group as usize).collect();
    let found = groups
        .par_iter()
        .map(|&start| -> io::Result<Option<Extent>> {
            let end = start + group;
            if end > closed {
                return Ok(None);
            }
            Ok(recording
                .span_summaries(run, end / SPAN_BLOCKS - 1, table)?
                .into_iter()
                .find(|(held, _)| *held == level)
                .map(|(_, extent)| extent))
        })
        .collect::<io::Result<Vec<_>>>()?;
    for (&start, piece) in groups.iter().zip(found) {
        let part = start.max(blocks.start)..(start + group).min(blocks.end);
        match piece {
            Some(extent) => out.push(extent),
            None => cover(recording, run, table, part, level - 1, closed, out)?,
        }
    }
    Ok(())
}

fn uncovered(recording: &Recording, run: u32, blocks: std::ops::Range<u32>, covered: &[(i64, i64)], first: u32, last: u32) -> io::Result<Vec<(u32, u32)>> {
    let held = recording.run(run)?;
    let first_hour = i64::from(held.first_hour);
    let mut out: Vec<(u32, u32)> = Vec::new();
    let mut remain: Vec<(i64, i64)> = Vec::new();
    let mut next: Vec<(i64, i64)> = Vec::new();
    for block in blocks {
        let (start, hours) = held.block(block as usize);
        let (a, b) = (start.max(first), (start + hours - 1).min(last));
        if a > b {
            continue;
        }
        remain.clear();
        remain.push((first_hour + i64::from(a), first_hour + i64::from(b) + 1));
        for &(begin, end) in covered {
            if remain.is_empty() {
                break;
            }
            next.clear();
            for &(lo, hi) in &remain {
                if end <= lo || begin >= hi {
                    next.push((lo, hi));
                } else {
                    if lo < begin {
                        next.push((lo, begin));
                    }
                    if end < hi {
                        next.push((end, hi));
                    }
                }
            }
            std::mem::swap(&mut remain, &mut next);
        }
        for &(lo, hi) in &remain {
            let (a, b) = ((lo - first_hour) as u32, (hi - 1 - first_hour) as u32);
            match out.last_mut() {
                Some((_, end)) if *end + 1 == a => *end = b,
                _ => out.push((a, b)),
            }
        }
    }
    Ok(out)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bin {
    pub hours: u32,
    pub first: i64,
    pub last: i64,
    pub min: i64,
    pub max: i64,
    pub first_bucket: i32,
    pub last_bucket: i32,
    pub min_bucket: i32,
    pub max_bucket: i32,
}

impl Bin {
    fn add(&mut self, fold: &Fold) {
        let bucket = |bucket: i64| bucket as i32;
        if self.hours == 0 {
            *self = Self {
                hours: fold.hours,
                first: fold.first,
                last: fold.last,
                min: fold.min,
                max: fold.max,
                first_bucket: bucket(fold.first_bucket),
                last_bucket: bucket(fold.last_bucket),
                min_bucket: bucket(fold.min_bucket),
                max_bucket: bucket(fold.max_bucket),
            };
            return;
        }
        let first_bucket = bucket(fold.first_bucket);
        let last_bucket = bucket(fold.last_bucket);
        let min_bucket = bucket(fold.min_bucket);
        let max_bucket = bucket(fold.max_bucket);
        if first_bucket < self.first_bucket {
            (self.first, self.first_bucket) = (fold.first, first_bucket);
        }
        if fold.min < self.min || (fold.min == self.min && min_bucket < self.min_bucket) {
            (self.min, self.min_bucket) = (fold.min, min_bucket);
        }
        if fold.max > self.max || (fold.max == self.max && max_bucket < self.max_bucket) {
            (self.max, self.max_bucket) = (fold.max, max_bucket);
        }
        if last_bucket >= self.last_bucket {
            (self.last, self.last_bucket) = (fold.last, last_bucket);
        }
        self.hours += fold.hours;
    }

    fn merge_later(&mut self, later: &Self) {
        if self.hours == 0 {
            *self = *later;
            return;
        }
        if later.hours == 0 {
            return;
        }
        if later.min < self.min {
            (self.min, self.min_bucket) = (later.min, later.min_bucket);
        }
        if later.max > self.max {
            (self.max, self.max_bucket) = (later.max, later.max_bucket);
        }
        (self.last, self.last_bucket) = (later.last, later.last_bucket);
        self.hours += later.hours;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    pub sum: i128,
    pub abs: u128,
    pub falls: u64,
    pub changes: u64,
}

impl Totals {
    fn add(&mut self, fold: &Fold) {
        self.sum += fold.sum;
        self.abs += fold.abs;
        self.falls += fold.falls;
        self.changes += fold.changes;
    }
}

#[derive(Debug)]
pub struct Chart {
    pub level: u8,
    pub bin_hours: i64,
    pub first_bin_hour: i64,
    pub bins: usize,
    pub keys: Vec<u128>,
    values: Vec<Bin>,
    totals: Vec<Totals>,
    aggregates: u8,
    life: Vec<u32>,
    pub pieces: usize,
    pub walked: usize,
}

impl Chart {
    pub fn values(&self, at: usize) -> &[Bin] {
        &self.values[at * self.bins..(at + 1) * self.bins]
    }

    pub fn totals(&self, at: usize) -> Option<&[Totals]> {
        (self.aggregates != 0).then(|| &self.totals[at * self.bins..(at + 1) * self.bins])
    }

    pub fn life(&self, at: usize) -> &[u32] {
        &self.life[at * self.bins..(at + 1) * self.bins]
    }

    fn row(&mut self, key: u128, rows: &mut HashMap<u128, usize, foldhash::fast::RandomState>) -> usize {
        let bins = self.bins;
        let declares = self.aggregates != 0;
        *rows.entry(key).or_insert_with(|| {
            self.keys.push(key);
            self.values.resize(self.values.len() + bins, Bin::default());
            self.life.resize(self.life.len() + bins, 0);
            if declares {
                self.totals.resize(self.totals.len() + bins, Totals::default());
            }
            self.keys.len() - 1
        })
    }

    fn merge_walk(&mut self, walk: Walk<'_>, covered: &[(i64, i64)], rows: &mut HashMap<u128, usize, foldhash::fast::RandomState>) {
        let pieced_after = |hour: i64| {
            let bucket_end = (summary::bucket_of(hour, 0) + 1) * summary::bucket_hours(0);
            covered.iter().any(|&(lo, _)| hour < lo && lo < bucket_end)
        };
        let mut walked: Vec<(u128, Row)> = walk.rows.into_iter().collect();
        walked.sort_unstable_by_key(|(key, _)| *key);
        for (key, folded) in walked {
            let row = self.row(key, rows);
            for (bin, walked_bin) in folded.bins {
                self.life[row * self.bins + bin] += walked_bin.life;
                if let Some(fold) = walked_bin.fold() {
                    self.add_beside(row, bin, &fold, pieced_after(walked_bin.first_hour), pieced_after(walked_bin.last_hour));
                }
            }
        }
    }

    fn add_beside(&mut self, at: usize, bin: usize, fold: &Fold, first_earlier: bool, last_earlier: bool) {
        let index = at * self.bins + bin;
        let held = self.values[index];
        self.values[index].add(fold);
        if held.hours > 0 {
            let value = &mut self.values[index];
            if first_earlier && fold.first_bucket as i32 == held.first_bucket {
                value.first = fold.first;
            }
            if last_earlier && fold.last_bucket as i32 == held.last_bucket {
                value.last = held.last;
            }
        }
        if let Some(totals) = self.totals.get_mut(index) {
            totals.add(fold);
        }
    }

    fn declared(mut self) -> Self {
        let declared = |bit: u8| self.aggregates & bit != 0;
        let (sum, abs, falls, changes) = (declared(summary::aggregate::SUM), declared(summary::aggregate::ABS), declared(summary::aggregate::FALLS), declared(summary::aggregate::CHANGES));
        for totals in &mut self.totals {
            *totals = Totals { sum: if sum { totals.sum } else { 0 }, abs: if abs { totals.abs } else { 0 }, falls: if falls { totals.falls } else { 0 }, changes: if changes { totals.changes } else { 0 } };
        }
        self
    }
}

pub fn chart(
    recording: &Recording,
    run: u32,
    table: TableId,
    source: u32,
    keys: Option<&[u128]>,
    from: i32,
    to: i32,
    width: u32,
) -> io::Result<Chart> {
    let per_pixel = ((i64::from(to) - i64::from(from) + 1) / i64::from(width.max(1))).max(1);
    if per_pixel < summary::bucket_hours(0) {
        return chart_at(recording, run, table, source, keys, from, to, WALKED, per_pixel);
    }
    let mut level = 0u8;
    while summary::bucket_hours(level + 1) <= per_pixel {
        level += 1;
    }
    let bin_hours = (per_pixel / summary::bucket_hours(level)) * summary::bucket_hours(level);
    chart_at(recording, run, table, source, keys, from, to, level, bin_hours)
}

pub const WALKED: u8 = u8::MAX;

const fn ceil_multiple(hour: i64, size: i64) -> i64 {
    (hour + size - 1).div_euclid(size) * size
}

pub fn chart_at(
    recording: &Recording,
    run: u32,
    table: TableId,
    source: u32,
    keys: Option<&[u128]>,
    from: i32,
    to: i32,
    level: u8,
    bin_hours: i64,
) -> io::Result<Chart> {
    let valid_hour = check_withheld_run(recording, run, table, from, to)?;
    let first_bin_hour = i64::from(from).div_euclid(bin_hours) * bin_hours;
    let bins = ((i64::from(to) - first_bin_hour) / bin_hours + 1) as usize;
    let (read_from, min_bucket_hour) = match valid_hour {
        Some(valid) if from < valid => {
            let aligned = ceil_multiple(i64::from(valid), bin_hours);
            if aligned > i64::from(to) {
                return Ok(Chart {
                    level,
                    bin_hours,
                    first_bin_hour,
                    bins,
                    keys: Vec::new(),
                    values: Vec::new(),
                    totals: Vec::new(),
                    aggregates: 0,
                    life: Vec::new(),
                    pieces: 0,
                    walked: 0,
                });
            }
            (aligned as i32, aligned)
        }
        Some(valid) => (from, i64::from(valid)),
        None => (from, i64::MIN),
    };
    let (first, last) = (ordinal_of(recording, run, read_from)?, ordinal_of(recording, run, to)?);
    let blocks = first / UNIT_ORDINALS..last / UNIT_ORDINALS + 1;
    let extents = match level {
        WALKED => Vec::new(),
        _ => pieces(recording, run, table.index(), blocks.clone(), level)?,
    };
    let registry = replay_recording::summaries::registry()?;
    let Some((_, Shape::Values(columns))) = registry.iter().find(|(held, _)| *held == table.index()) else {
        return Err(invalid("The table has no summarized columns"));
    };
    let column = *columns
        .iter()
        .find(|column| column.source.id() == source)
        .ok_or_else(|| invalid("The summary holds no such column"))?;
    let mut chart = Chart {
        level,
        bin_hours,
        first_bin_hour,
        bins,
        keys: Vec::new(),
        values: Vec::new(),
        totals: Vec::new(),
        aggregates: column.aggregates,
        life: Vec::new(),
        pieces: extents.len(),
        walked: 0,
    };
    let mut rows: HashMap<u128, usize, foldhash::fast::RandomState> = HashMap::default();
    let read = extents.par_iter().map(|extent| recording.stored_bytes(extent)).collect::<io::Result<Vec<_>>>()?;
    let decoded = read
        .par_iter()
        .map(|bytes| -> io::Result<Option<(Piece<'_>, ColumnFrame)>> {
            let piece = Piece::read(bytes)?;
            let Some(column) = piece.column_of(source) else { return Ok(None) };
            let frame = piece.column(column)?;
            Ok(Some((piece, frame)))
        })
        .collect::<io::Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let covered: Vec<(i64, i64)> = decoded
        .iter()
        .filter_map(|(piece, _)| {
            let size = summary::bucket_hours(piece.directory.level);
            let floor = if min_bucket_hour > i64::MIN {
                ceil_multiple(min_bucket_hour, size)
            } else {
                i64::MIN
            };
            let lo = piece.directory.first_hour.max(floor);
            let hi = piece.directory.first_hour + i64::from(piece.directory.hours);
            (lo < hi).then_some((lo, hi))
        })
        .collect();
    let mut asked: Vec<u128> = match keys {
        Some(keys) => keys.iter().copied().filter(|&key| decoded.iter().any(|(piece, _)| piece.key_at(key).is_some())).collect(),
        None => decoded.iter().flat_map(|(piece, _)| piece.keys.iter().copied()).collect(),
    };
    asked.sort_unstable();
    asked.dedup();
    let grid = Grid {
        from: i64::from(read_from),
        to: i64::from(to),
        first_bin_hour,
        bin_hours,
        bins,
        min_bucket_hour,
    };
    chart.values = vec![Bin::default(); asked.len() * bins];
    chart.life = vec![0; asked.len() * bins];
    if chart.aggregates != 0 {
        chart.totals = vec![Totals::default(); asked.len() * bins];
        chart
            .values
            .par_chunks_mut(bins.max(1))
            .zip(chart.life.par_chunks_mut(bins.max(1)))
            .zip(chart.totals.par_chunks_mut(bins.max(1)))
            .zip(asked.par_iter())
            .try_for_each(|(((values, life), totals), &key)| grid.fold_key(&decoded, key, values, life, Some(totals), None))?;
    } else {
        chart
            .values
            .par_chunks_mut(bins.max(1))
            .zip(chart.life.par_chunks_mut(bins.max(1)))
            .zip(asked.par_iter())
            .try_for_each(|((values, life), &key)| grid.fold_key(&decoded, key, values, life, None, None))?;
    }
    for (at, &key) in asked.iter().enumerate() {
        rows.insert(key, at);
    }
    chart.keys = asked;
    let ranges = uncovered(recording, run, blocks, &covered, first, last)?;
    if !ranges.is_empty() {
        let mut walk = Walk::new(Some(column.source), keys, first_bin_hour, bin_hours, bins);
        chart.walked = walk_ranges(recording, run, table, &inputs(column.source), &ranges, &mut walk)?;
        chart.merge_walk(walk, &covered, &mut rows);
    }
    Ok(chart.declared())
}

pub fn lifetimes(recording: &Recording, run: u32, table: TableId) -> io::Result<Vec<(u128, i64, i64)>> {
    Ok(lives(recording, run, table, None)?
        .into_iter()
        .filter_map(|(key, spans)| Some((key, spans.first()?.0, spans.last()?.1)))
        .collect())
}

pub fn presences(recording: &Recording, run: u32, table: TableId, keys: (u128, u128), from: i64, to: i64) -> io::Result<Vec<(u128, Vec<(i64, i64)>)>> {
    lives(recording, run, table, Some((keys, from, to)))
}

fn lives(recording: &Recording, run: u32, table: TableId, exact: Option<((u128, u128), i64, i64)>) -> io::Result<Vec<(u128, Vec<(i64, i64)>)>> {
    let held = recording.run(run)?;
    let valid_hour = match recording.predates(table.index()) {
        Some(_) => None,
        None => match recording.table_coverage(table.index(), run) {
            replay_recording::columnar_frames::TableCoverage::Valid => None,
            replay_recording::columnar_frames::TableCoverage::WithheldAll(reason) => {
                return Err(replay_recording::columnar_frames::Damage::wrap(
                    replay_recording::columnar_frames::DamageScope::Withheld { table: table.index() as u32 },
                    invalid(reason.to_owned()),
                ));
            }
            replay_recording::columnar_frames::TableCoverage::WithheldBefore { ordinal, .. } => {
                Some(i64::from(held.first_hour) + i64::from(ordinal))
            }
        },
    };
    let blocks = held.pages() as u32;
    let run_first = i64::from(held.first_hour);
    let floor = valid_hour.map_or(0, |hour| hour - run_first);
    let (low, high) = match exact {
        Some((_, from, to)) => ((from - run_first).max(floor).max(0), (to - run_first).min(i64::from(held.ordinals) - 1)),
        None => (floor.max(0), i64::from(held.ordinals) - 1),
    };
    if low > high || blocks == 0 {
        return Ok(Vec::new());
    }
    let pages = (low / i64::from(UNIT_ORDINALS)) as u32..((high / i64::from(UNIT_ORDINALS)) as u32 + 1).min(blocks);
    let extents = pieces(recording, run, table.index(), pages.clone(), 8)?;
    let mut spans: HashMap<u128, Vec<(i64, i64)>, foldhash::fast::RandomState> = HashMap::default();
    let mut covered = Vec::new();
    let mut gapped: Vec<(u128, i64, i64)> = Vec::new();
    for extent in &extents {
        let bytes = recording.stored_bytes(extent)?;
        let piece = Piece::read(&bytes)?;
        let size = summary::bucket_hours(piece.directory.level);
        covered.push((piece.directory.first_hour, piece.directory.first_hour + i64::from(piece.directory.hours)));
        for (at, &key) in piece.keys.iter().enumerate() {
            if exact.is_some_and(|((lo, hi), ..)| key < lo || key > hi) {
                continue;
            }
            for (bucket, life) in piece.life(at)?.iter().enumerate() {
                if life.hours == 0 {
                    continue;
                }
                let start = (piece.directory.first_bucket + bucket as i64) * size;
                let (first, last) = (start + i64::from(life.first), start + i64::from(life.end) - 1);
                if last < run_first + low || first > run_first + high {
                    continue;
                }
                let gap = i64::from(life.hours) < i64::from(life.end) - i64::from(life.first);
                let refined = match (gap, exact) {
                    (false, _) => None,
                    (true, Some(_)) => Some((first.max(run_first + low), last.min(run_first + high))).filter(|(first, last)| first <= last),
                    (true, None) => valid_hour.filter(|&valid| first < valid && valid <= last).map(|valid| (valid, last)),
                };
                if let Some((first, last)) = refined {
                    gapped.push((key, first, last));
                } else {
                    spans.entry(key).or_default().push((first, last));
                }
            }
        }
    }
    if !gapped.is_empty() {
        let mut keys: Vec<u128> = gapped.iter().map(|(key, ..)| *key).collect();
        keys.sort_unstable();
        keys.dedup();
        let mut ranges: Vec<(u32, u32)> = gapped.iter().map(|&(_, first, last)| ((first - run_first) as u32, (last - run_first) as u32)).collect();
        ranges.sort_unstable();
        let mut merged: Vec<(u32, u32)> = Vec::with_capacity(ranges.len());
        for (first, last) in ranges {
            match merged.last_mut() {
                Some(range) if first <= range.1.saturating_add(1) => range.1 = range.1.max(last),
                _ => merged.push((first, last)),
            }
        }
        let mut walk = Walk::new(None, Some(keys.as_slice()), run_first, i64::from(held.ordinals), 1);
        walk_ranges(recording, run, table, &[], &merged, &mut walk)?;
        for (key, row) in walk.rows {
            let asked: Vec<(i64, i64)> = gapped.iter().filter(|(gap, ..)| *gap == key).map(|&(_, first, last)| (first, last)).collect();
            for (first, last) in row.lived {
                for &(from, to) in &asked {
                    let (first, last) = (first.max(from), last.min(to));
                    if first <= last {
                        spans.entry(key).or_default().push((first, last));
                    }
                }
            }
        }
    }
    let values = matches!(
        replay_recording::summaries::registry()?.iter().find(|(held, _)| *held == table.index()),
        Some((_, Shape::Values(_)))
    );
    if values {
        let ranges = uncovered(recording, run, pages, &covered, low as u32, high as u32)?;
        let keyed: Option<(u128, u128)> = exact.map(|(keys, ..)| keys);
        if !ranges.is_empty() {
            let mut walk = Walk::new(None, None, i64::from(held.first_hour), i64::from(held.ordinals), 1);
            walk.span = keyed;
            walk_ranges(recording, run, table, &[], &ranges, &mut walk)?;
            for (key, row) in walk.rows {
                if keyed.is_none_or(|(lo, hi)| lo <= key && key <= hi) {
                    spans.entry(key).or_default().extend(row.lived);
                }
            }
        }
    }
    let mut out: Vec<(u128, Vec<(i64, i64)>)> = spans
        .into_iter()
        .filter_map(|(key, mut held)| {
            held.sort_unstable();
            let mut merged: Vec<(i64, i64)> = Vec::with_capacity(held.len());
            for (first, last) in held {
                let first = valid_hour.map_or(first, |floor| first.max(floor));
                if first > last {
                    continue;
                }
                match merged.last_mut() {
                    Some(span) if first <= span.1 + 1 => span.1 = span.1.max(last),
                    _ => merged.push((first, last)),
                }
            }
            (!merged.is_empty()).then_some((key, merged))
        })
        .collect();
    out.sort_unstable_by_key(|(key, _)| *key);
    Ok(out)
}

fn inputs(source: summary::Source) -> Vec<usize> {
    match source {
        summary::Source::Cell(column) => vec![column],
        summary::Source::Computed { inputs, .. } => (0..64).filter(|bit| inputs >> bit & 1 != 0).collect(),
    }
}

fn walk_ranges(recording: &Recording, run: u32, table: TableId, columns: &[usize], ranges: &[(u32, u32)], walk: &mut Walk<'_>) -> io::Result<usize> {
    let first_hour = recording.run(run)?.first_hour;
    let mut blocks = 0;
    for &(first, last) in ranges {
        blocks += (last / UNIT_ORDINALS - first / UNIT_ORDINALS + 1) as usize;
        let start = i64::from(first_hour) + i64::from(first);
        walk.tracks.clear();
        let before = first.checked_sub(1);
        let from = first_hour + before.unwrap_or(0) as i32;
        let to = first_hour + last as i32;
        let keys = match (walk.keys, walk.span) {
            (Some(keys), _) => Keys::These(keys),
            (None, Some((lo, hi))) => Keys::Range(lo, hi),
            (None, None) => Keys::All,
        };
        crate::points::points(recording, run, table, columns, walk.source, keys, from, to, &mut |point| {
            if before.is_some() && point.hour == from {
                walk.begin(point.key, point.covered, point.value, start);
            } else {
                walk.point(point.key, point.covered, point.value, i64::from(point.hour));
            }
            true
        })?;
        walk.end(start + i64::from(last - first) + 1);
    }
    Ok(blocks)
}

#[derive(Clone, Copy, Default)]
struct WalkBin {
    value: Option<Fold>,
    life: u32,
    changes: u64,
    falls: u64,
    first_hour: i64,
    last_hour: i64,
}

impl WalkBin {
    fn fold(&self) -> Option<Fold> {
        let mut fold = self.value?;
        fold.changes += self.changes;
        fold.falls += self.falls;
        Some(fold)
    }
}

#[derive(Default)]
struct Row {
    bins: Vec<(usize, WalkBin)>,
    lived: Vec<(i64, i64)>,
}

impl Row {
    fn bin(&mut self, bin: usize) -> &mut WalkBin {
        if let Some(&(last, _)) = self.bins.last() {
            if last == bin {
                return &mut self.bins.last_mut().expect("last").1;
            }
            if bin > last {
                self.bins.push((bin, WalkBin::default()));
                return &mut self.bins.last_mut().expect("pushed").1;
            }
        }
        match self.bins.binary_search_by_key(&bin, |&(at, _)| at) {
            Ok(at) => &mut self.bins[at].1,
            Err(at) => {
                self.bins.insert(at, (bin, WalkBin::default()));
                &mut self.bins[at].1
            }
        }
    }
}

struct Track {
    covered: bool,
    since: i64,
    value: Option<i64>,
    value_since: i64,
    before: Option<i64>,
}

struct Walk<'a> {
    source: Option<summary::Source>,
    keys: Option<&'a [u128]>,
    span: Option<(u128, u128)>,
    first_bin: i64,
    bin_hours: i64,
    bins: usize,
    tracks: HashMap<u128, Track, foldhash::fast::RandomState>,
    rows: HashMap<u128, Row, foldhash::fast::RandomState>,
}

impl<'a> Walk<'a> {
    fn new(source: Option<summary::Source>, keys: Option<&'a [u128]>, first_bin: i64, bin_hours: i64, bins: usize) -> Self {
        Self {
            source,
            keys,
            span: None,
            first_bin,
            bin_hours,
            bins,
            tracks: HashMap::default(),
            rows: HashMap::default(),
        }
    }

    fn wanted(&self, key: u128) -> bool {
        self.keys.is_none_or(|keys| keys.contains(&key))
    }

    fn row(&mut self, key: u128) -> &mut Row {
        self.rows.entry(key).or_default()
    }

    fn parts(&self, from: i64, to: i64) -> Vec<(usize, i64, i64)> {
        let mut out = Vec::new();
        let mut at = from.max(self.first_bin);
        let last = self.first_bin + self.bins as i64 * self.bin_hours;
        while at < to.min(last) {
            let bin = (at - self.first_bin) / self.bin_hours;
            let end = (self.first_bin + (bin + 1) * self.bin_hours).min(to);
            out.push((bin as usize, at, end));
            at = end;
        }
        out
    }

    fn add_life(&mut self, key: u128, from: i64, to: i64) {
        if to <= from {
            return;
        }
        let parts = self.parts(from, to);
        let row = self.row(key);
        for (bin, start, end) in parts {
            row.bin(bin).life += (end - start) as u32;
        }
        match row.lived.last_mut() {
            Some(lived) if from <= lived.1 + 1 && to - 1 >= lived.0 => {
                lived.0 = lived.0.min(from);
                lived.1 = lived.1.max(to - 1);
            }
            _ => {
                row.lived.push((from, to - 1));
                row.lived.sort_unstable();
            }
        }
    }

    fn add_value(&mut self, key: u128, value: i64, from: i64, to: i64) {
        let parts = self.parts(from, to);
        if parts.is_empty() {
            return;
        }
        let row = self.row(key);
        for (bin, start, end) in parts {
            let mut part = Fold::constant(value, (end - start) as u32, summary::bucket_of(start, 0));
            part.last_bucket = summary::bucket_of(end - 1, 0);
            let slot = row.bin(bin);
            match &mut slot.value {
                Some(held) => {
                    held.merge(&part);
                    slot.first_hour = slot.first_hour.min(start);
                    slot.last_hour = slot.last_hour.max(end - 1);
                }
                empty @ None => {
                    *empty = Some(part);
                    slot.first_hour = start;
                    slot.last_hour = end - 1;
                }
            }
        }
    }

    fn cover(&mut self, key: u128, covered: bool, hour: i64) {
        let track = self.tracks.get_mut(&key).expect("a track");
        if track.covered == covered {
            return;
        }
        let (was, since) = (track.covered, track.since);
        track.covered = covered;
        track.since = hour;
        if was {
            self.add_life(key, since, hour);
        }
    }

    fn set(&mut self, key: u128, value: Option<i64>, hour: i64) {
        let track = self.tracks.get_mut(&key).expect("a track");
        if track.value == value && track.value_since <= hour {
            return;
        }
        let before = if track.value_since < hour { track.value } else { track.before };
        let (held, since) = (track.value, track.value_since);
        track.before = before;
        track.value = value;
        track.value_since = hour;
        if let Some(held) = held {
            self.add_value(key, held, since, hour);
        }
        if let (Some(before), Some(now)) = (before, value)
            && before != now
            && let Some(&(bin, _, _)) = self.parts(hour, hour + 1).first()
        {
            let slot = self.row(key).bin(bin);
            slot.changes += 1;
            slot.falls += u64::from(now < before);
        }
    }

    fn track(&mut self, key: u128, hour: i64) {
        self.tracks.entry(key).or_insert(Track {
            covered: false,
            since: hour,
            value: None,
            value_since: hour,
            before: None,
        });
    }

    fn begin(&mut self, key: u128, covered: bool, value: Option<i64>, hour: i64) {
        if !covered || !self.wanted(key) {
            return;
        }
        let value = self.source.and(value);
        self.tracks.insert(key, Track { covered: true, since: hour, value, value_since: hour, before: value });
    }

    fn point(&mut self, key: u128, covered: bool, value: Option<i64>, hour: i64) {
        if !self.wanted(key) {
            return;
        }
        self.track(key, hour);
        self.cover(key, covered, hour);
        if self.source.is_some() {
            self.set(key, if covered { value } else { None }, hour);
        }
    }

    fn end(&mut self, hour: i64) {
        let keys: Vec<u128> = self.tracks.keys().copied().collect();
        for key in keys {
            self.cover(key, false, hour);
            self.set(key, None, hour);
        }
        self.tracks.clear();
    }
}

pub fn log(recording: &Recording, run: u32, table: TableId, from: i32, to: i32, bin_hours: i64) -> io::Result<Vec<(u128, Vec<u64>)>> {
    log_of(recording, run, table, (from, to), bin_hours, &|_| true)
}

pub fn log_of(recording: &Recording, run: u32, table: TableId, (from, to): (i32, i32), bin_hours: i64, keep: &dyn Fn(u128) -> bool) -> io::Result<Vec<(u128, Vec<u64>)>> {
    let bin_hours = bin_hours.max(summary::bucket_hours(0));
    let mut level = 0u8;
    while summary::bucket_hours(level + 1) <= bin_hours && bin_hours % summary::bucket_hours(level + 1) == 0 {
        level += 1;
    }
    let first_bin = i64::from(from).div_euclid(bin_hours) * bin_hours;
    let bins = ((i64::from(to) - first_bin) / bin_hours + 1) as usize;
    let first = ordinal_of(recording, run, from)? / UNIT_ORDINALS;
    let last = ordinal_of(recording, run, to)? / UNIT_ORDINALS;
    let extents = pieces(recording, run, table.index(), first..last + 1, level)?;
    let mut counts: HashMap<u128, Vec<u64>, foldhash::fast::RandomState> = HashMap::default();
    for extent in &extents {
        let bytes = recording.stored_bytes(extent)?;
        let piece = Piece::read(&bytes)?;
        let frame = piece.column(0)?;
        let size = summary::bucket_hours(piece.directory.level);
        for (at, &key) in piece.keys.iter().enumerate() {
            if !keep(key) {
                continue;
            }
            for (bucket, count) in frame.counts(&piece, at)?.into_iter().enumerate() {
                let start = (piece.directory.first_bucket + bucket as i64) * size;
                if count == 0 || start + size <= i64::from(from) || start > i64::from(to) {
                    continue;
                }
                let bin = ((start - first_bin).div_euclid(bin_hours)) as usize;
                if bin < bins {
                    counts.entry(key).or_insert_with(|| vec![0; bins])[bin] += count;
                }
            }
        }
    }
    let mut out: Vec<(u128, Vec<u64>)> = counts.into_iter().collect();
    out.sort_unstable_by_key(|(key, _)| *key);
    Ok(out)
}

pub fn next(recording: &Recording, run: u32, table: TableId, column: usize, key: u128, from: i32) -> io::Result<Option<i32>> {
    let held = recording.run(run)?;
    let last_hour = held.first_hour + (held.ordinals - 1) as i32;
    if from >= last_hour {
        return Ok(None);
    }
    let start = at_keys(recording, run, table, from, Some(&[column]), Keys::These(&[key]))?;
    let now = {
        let available = matches!(start.status, Status::Complete | Status::Truncated { .. });
        let view = start.view();
        available.then(|| view.slot(key).and_then(|slot| view.number(column, slot))).flatten()
    };
    let mut cursor = i64::from(from) + 1;
    let first_block = ordinal_of(recording, run, from + 1)? / UNIT_ORDINALS;
    for extent in pieces(recording, run, table.index(), first_block..held.pages() as u32, 1)? {
        let bytes = recording.stored_bytes(&extent)?;
        let piece = Piece::read(&bytes)?;
        let directory = &piece.directory;
        if directory.first_hour > cursor {
            break;
        }
        let size = summary::bucket_hours(directory.level);
        let Some(column_at) = piece.column_of(column as u32) else { break };
        let frame = piece.column(column_at)?;
        let mut tokens = piece.key_at(key).map(|at| frame.tokens(at));
        for bucket in 0..directory.buckets {
            let index = directory.first_bucket + i64::from(bucket);
            let covered = directory.covered(bucket);
            let fold = match &mut tokens {
                Some(tokens) => tokens.next(index, covered)?,
                None => None,
            };
            let begin = index * size;
            let end = (begin + size).min(directory.first_hour + i64::from(directory.hours));
            if end <= cursor {
                continue;
            }
            let steady = match (fold, now) {
                (None, None) => true,
                (Some(fold), Some(now)) => fold.first == now && fold.last == now && fold.min == now && fold.max == now && fold.hours == covered,
                _ => false,
            };
            if !steady {
                return walk_to_change(recording, run, table, column, key, (begin.max(cursor) - 1) as i32, last_hour);
            }
            cursor = end;
        }
    }
    walk_to_change(recording, run, table, column, key, (cursor - 1) as i32, last_hour)
}

fn walk_to_change(recording: &Recording, run: u32, table: TableId, column: usize, key: u128, after: i32, last_hour: i32) -> io::Result<Option<i32>> {
    if after >= last_hour {
        return Ok(None);
    }
    let (mut held, mut found) = (None, None);
    crate::points::points(recording, run, table, &[column], Some(summary::Source::Cell(column)), Keys::These(&[key]), after, last_hour, &mut |point| {
        let now = if point.covered { point.value } else { None };
        if point.hour == after {
            held = now;
        } else if now != held {
            found = Some(point.hour);
            return false;
        }
        true
    })?;
    Ok(found)
}

struct Grid {
    from: i64,
    to: i64,
    first_bin_hour: i64,
    bin_hours: i64,
    bins: usize,
    min_bucket_hour: i64,
}

impl Grid {
    fn bin(&self, directory: &summary::Directory, bucket: u32) -> Option<usize> {
        let size = summary::bucket_hours(directory.level);
        let start = (directory.first_bucket + i64::from(bucket)) * size;
        if start < self.min_bucket_hour || start + size <= self.from || start > self.to {
            return None;
        }
        usize::try_from((start - self.first_bin_hour).div_euclid(self.bin_hours))
            .ok()
            .filter(|&bin| bin < self.bins)
    }

    fn fold_key(
        &self,
        pieces: &[(Piece<'_>, ColumnFrame)],
        key: u128,
        values: &mut [Bin],
        life: &mut [u32],
        mut totals: Option<&mut [Totals]>,
        mut marks: Option<&mut [u64]>,
    ) -> io::Result<()> {
        let mut merge = |bin: usize, mut fold: Fold, level: u8, at: u64| {
            if let Some(marks) = marks.as_deref_mut() {
                marks[bin] = at;
            }
            let ratio = summary::bucket_hours(level) / summary::bucket_hours(0);
            fold.first_bucket *= ratio;
            fold.min_bucket *= ratio;
            fold.max_bucket *= ratio;
            fold.last_bucket = (fold.last_bucket + 1) * ratio - 1;
            values[bin].add(&fold);
            if let Some(totals) = totals.as_deref_mut() {
                totals[bin].add(&fold);
            }
        };
        for (piece, frame) in pieces {
            let Some(at) = piece.key_at(key) else { continue };
            let directory = &piece.directory;
            let buckets = directory.buckets;
            let run = |count: u64, at: u32| -> io::Result<u32> {
                u32::try_from(count)
                    .ok()
                    .filter(|&count| count <= buckets - at)
                    .ok_or_else(|| invalid("A key's tokens pass its piece's buckets"))
            };
            let mut tokens = frame.tokens(at);
            let mut bucket = 0;
            while bucket < buckets {
                let index = directory.first_bucket + i64::from(bucket);
                match tokens.span(index, directory.covered(bucket))? {
                    summary::Span::None(count) => bucket += run(count, bucket)?,
                    summary::Span::One(fold) => {
                        if let Some(bin) = self.bin(directory, bucket) {
                            merge(bin, fold, directory.level, hour_mark(bucket_last_hour(directory, bucket)));
                        }
                        bucket += 1;
                    }
                    summary::Span::Same(value, count) => {
                        let end = bucket + run(count, bucket)?;
                        while bucket < end {
                            let bin = self.bin(directory, bucket);
                            let (start, mut hours) = (bucket, directory.covered(bucket));
                            bucket += 1;
                            while bucket < end && self.bin(directory, bucket) == bin {
                                hours += directory.covered(bucket);
                                bucket += 1;
                            }
                            if let Some(bin) = bin {
                                let mut fold = Fold::constant(value, hours, directory.first_bucket + i64::from(start));
                                fold.last_bucket = directory.first_bucket + i64::from(bucket - 1);
                                merge(bin, fold, directory.level, hour_mark(bucket_last_hour(directory, bucket - 1)));
                            }
                        }
                    }
                }
            }
            let mut lives = piece.lives(at);
            let mut bucket = 0;
            while bucket < buckets {
                match lives.span()? {
                    summary::LifeSpan::Empty(count) => bucket += run(count, bucket)?,
                    summary::LifeSpan::Full(count) => {
                        let end = bucket + run(count, bucket)?;
                        for full in bucket..end {
                            if let Some(bin) = self.bin(directory, full) {
                                life[bin] += directory.covered(full);
                            }
                        }
                        bucket = end;
                    }
                    summary::LifeSpan::Part(lived) => {
                        if let Some(bin) = self.bin(directory, bucket) {
                            life[bin] += lived.hours;
                        }
                        bucket += 1;
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum By {
    Last,
    Max,
    Min,
}

impl By {
    fn of(self, bin: &Bin) -> i64 {
        match self {
            By::Last => bin.last,
            By::Max => bin.max,
            By::Min => bin.min,
        }
    }
}

fn hour_mark(hour: i64) -> u64 {
    (hour - i64::from(i32::MIN) + 1) as u64
}

fn bucket_last_hour(directory: &summary::Directory, bucket: u32) -> i64 {
    let size = summary::bucket_hours(directory.level);
    let bucket_end = (directory.first_bucket + i64::from(bucket) + 1) * size;
    let piece_end = directory.first_hour + i64::from(directory.hours);
    bucket_end.min(piece_end) - 1
}

pub fn top(
    recording: &Recording,
    run: u32,
    table: TableId,
    source: u32,
    from: i32,
    to: i32,
    width: u32,
    n: usize,
    by: By,
) -> io::Result<Vec<Vec<(u128, Bin)>>> {
    top_with(recording, run, table, source, from, to, width, n, by, true)
}

fn top_with(
    recording: &Recording,
    run: u32,
    table: TableId,
    source: u32,
    from: i32,
    to: i32,
    width: u32,
    n: usize,
    by: By,
    ranked: bool,
) -> io::Result<Vec<Vec<(u128, Bin)>>> {
    let valid_hour = check_withheld_run(recording, run, table, from, to)?;
    let per_pixel = ((i64::from(to) - i64::from(from) + 1) / i64::from(width.max(1))).max(1);
    if per_pixel < summary::bucket_hours(0) {
        return Err(invalid("A top-N view's bins are six hours or wider"));
    }
    let mut level = 0u8;
    while summary::bucket_hours(level + 1) <= per_pixel {
        level += 1;
    }
    let bin_hours = (per_pixel / summary::bucket_hours(level)) * summary::bucket_hours(level);
    let first_bin_hour = i64::from(from).div_euclid(bin_hours) * bin_hours;
    let bins = ((i64::from(to) - first_bin_hour) / bin_hours + 1) as usize;
    if n == 0 {
        return Ok(vec![Vec::new(); bins]);
    }
    let (read_from, min_bucket_hour) = match valid_hour {
        Some(valid) if from < valid => {
            let aligned = ceil_multiple(i64::from(valid), bin_hours);
            if aligned > i64::from(to) {
                return Ok(vec![Vec::new(); bins]);
            }
            (aligned as i32, aligned)
        }
        Some(valid) => (from, i64::from(valid)),
        None => (from, i64::MIN),
    };
    let (first, last) = (ordinal_of(recording, run, read_from)?, ordinal_of(recording, run, to)?);
    let blocks = first / UNIT_ORDINALS..last / UNIT_ORDINALS + 1;
    let extents = pieces(recording, run, table.index(), blocks.clone(), level)?;
    let read = extents.par_iter().map(|extent| recording.stored_bytes(extent)).collect::<io::Result<Vec<_>>>()?;
    let decoded = read
        .par_iter()
        .map(|bytes| -> io::Result<(i64, Option<(Piece<'_>, ColumnFrame, usize)>)> {
            let piece = Piece::read(bytes)?;
            let end = piece.directory.first_hour + i64::from(piece.directory.hours) - 1;
            let Some(column) = piece.column_of(source) else { return Ok((end, None)) };
            let frame = piece.column(column)?;
            Ok((end, Some((piece, frame, column))))
        })
        .collect::<io::Result<Vec<_>>>()?;
    let mut walked_ends: Vec<i64> = decoded.iter().map(|(end, _)| *end).collect();
    let decoded = decoded.into_iter().filter_map(|(_, decoded)| decoded).collect::<Vec<_>>();
    let covered: Vec<(i64, i64)> = decoded
        .iter()
        .filter_map(|(piece, _, _)| {
            let size = summary::bucket_hours(piece.directory.level);
            let floor = if min_bucket_hour > i64::MIN {
                ceil_multiple(min_bucket_hour, size)
            } else {
                i64::MIN
            };
            let lo = piece.directory.first_hour.max(floor);
            let hi = piece.directory.first_hour + i64::from(piece.directory.hours);
            (lo < hi).then_some((lo, hi))
        })
        .collect();
    let ranges = uncovered(recording, run, blocks, &covered, first, last)?;
    let run_first = i64::from(recording.run(run)?.first_hour);
    walked_ends.extend(ranges.iter().map(|&(_, last)| run_first + i64::from(last)));
    walked_ends.sort_unstable();
    walked_ends.dedup();
    let walked: HashMap<u128, Row, foldhash::fast::RandomState> = if ranges.is_empty() {
        HashMap::default()
    } else {
        let registry = replay_recording::summaries::registry()?;
        let Some((_, Shape::Values(columns))) = registry.iter().find(|(held, _)| *held == table.index()) else {
            return Err(invalid("The table has no summarized columns"));
        };
        let column = *columns
            .iter()
            .find(|column| column.source.id() == source)
            .ok_or_else(|| invalid("The summary holds no such column"))?;
        let mut walk = Walk::new(Some(column.source), None, first_bin_hour, bin_hours, bins);
        walk_ranges(recording, run, table, &inputs(column.source), &ranges, &mut walk)?;
        walk.rows
    };
    let from_ranks =
        ranges.is_empty() && ranked && by == By::Last && !decoded.is_empty() && decoded.iter().all(|(piece, _, column)| usize::from(piece.rank(*column)) >= n);
    let ranks = match from_ranks {
        true => decoded
            .par_iter()
            .map(|(piece, _, column)| piece.ranks(*column)?.ok_or_else(|| invalid("A ranked column's piece holds no ranks")))
            .collect::<io::Result<Vec<_>>>()?,
        false => Vec::new(),
    };
    let decoded: Vec<(Piece<'_>, ColumnFrame)> = decoded.into_iter().map(|(piece, frame, _)| (piece, frame)).collect();
    let grid = Grid {
        from: i64::from(read_from),
        to: i64::from(to),
        first_bin_hour,
        bin_hours,
        bins,
        min_bucket_hour,
    };
    match from_ranks {
        true => grid.top_ranked(&decoded, &ranks, n),
        false => grid.top_folded(&decoded, &walked, (&walked_ends, summary::bucket_hours(level)), n, by),
    }
}

fn better(by: By) -> impl Fn(&(u128, Bin), &(u128, Bin)) -> std::cmp::Ordering + Copy + Send + Sync {
    move |a, b| by.of(&b.1).cmp(&by.of(&a.1)).then(a.0.cmp(&b.0))
}

impl Grid {
    fn top_ranked(&self, pieces: &[(Piece<'_>, ColumnFrame)], ranks: &[summary::Ranks], n: usize) -> io::Result<Vec<Vec<(u128, Bin)>>> {
        let mut chosen: Vec<Vec<u128>> = vec![Vec::new(); self.bins];
        for ((piece, _), ranks) in pieces.iter().zip(ranks) {
            for bucket in 0..piece.directory.buckets {
                let list = ranks.bucket(bucket);
                if list.is_empty() {
                    continue;
                }
                if let Some(bin) = self.bin(&piece.directory, bucket) {
                    chosen[bin] = list.iter().take(n).map(|&at| piece.keys[at as usize]).collect();
                }
            }
        }
        let mut keys: Vec<u128> = chosen.iter().flatten().copied().collect();
        keys.sort_unstable();
        keys.dedup();
        let folded = keys
            .par_iter()
            .map(|&key| -> io::Result<Vec<Bin>> {
                let (mut values, mut life) = (vec![Bin::default(); self.bins], vec![0u32; self.bins]);
                self.fold_key(pieces, key, &mut values, &mut life, None, None)?;
                Ok(values)
            })
            .collect::<io::Result<Vec<_>>>()?;
        let better = better(By::Last);
        Ok(chosen
            .into_iter()
            .enumerate()
            .map(|(bin, held)| {
                let mut list: Vec<(u128, Bin)> = held.into_iter().map(|key| (key, folded[keys.binary_search(&key).expect("a chosen key")][bin])).collect();
                list.sort_unstable_by(better);
                list
            })
            .collect())
    }

    fn top_folded(
        &self,
        pieces: &[(Piece<'_>, ColumnFrame)],
        walked: &HashMap<u128, Row, foldhash::fast::RandomState>,
        (walked_ends, bucket_hours): (&[i64], i64),
        n: usize,
        by: By,
    ) -> io::Result<Vec<Vec<(u128, Bin)>>> {
        let walked_mark = |hour: i64| {
            let bucket_last = (hour.div_euclid(bucket_hours) + 1) * bucket_hours - 1;
            let range_last = walked_ends[walked_ends.partition_point(|&end| end < hour).min(walked_ends.len() - 1)];
            hour_mark(bucket_last.min(range_last))
        };
        use std::cmp::Ordering;
        let mut keys: Vec<u128> = pieces.iter().flat_map(|(piece, _)| piece.keys.iter().copied()).chain(walked.keys().copied()).collect();
        keys.sort_unstable();
        keys.dedup();
        let bins = self.bins;
        let better = better(by);
        let empty = || (vec![Bin::default(); bins], vec![0u32; bins], vec![0u64; bins], vec![(0u64, Vec::<(u128, Bin)>::new()); bins]);
        let (_, _, _, best) = keys
            .par_iter()
            .try_fold(empty, |(mut values, mut life, mut marks, mut best), &key| -> io::Result<_> {
                values.fill(Bin::default());
                marks.fill(0);
                self.fold_key(pieces, key, &mut values, &mut life, None, Some(&mut marks))?;
                if let Some(row) = walked.get(&key) {
                    for &(bin, walked_bin) in &row.bins {
                        if let Some(fold) = walked_bin.fold() {
                            let mark = walked_mark(walked_bin.last_hour);
                            if values[bin].hours == 0 {
                                values[bin].add(&fold);
                                marks[bin] = mark;
                            } else if mark < marks[bin] {
                                let mut combined = Bin::default();
                                combined.add(&fold);
                                combined.merge_later(&values[bin]);
                                values[bin] = combined;
                            } else {
                                values[bin].add(&fold);
                                marks[bin] = mark;
                            }
                        }
                    }
                }
                for (bin, held) in values.iter().enumerate() {
                    if held.hours == 0 {
                        continue;
                    }
                    let (latest, list) = &mut best[bin];
                    if by == By::Last {
                        match marks[bin].cmp(latest) {
                            Ordering::Less => continue,
                            Ordering::Greater => {
                                *latest = marks[bin];
                                list.clear();
                            }
                            Ordering::Equal => {}
                        }
                    }
                    let entry = (key, *held);
                    if list.len() == n && better(&entry, list.last().expect("full")) != Ordering::Less {
                        continue;
                    }
                    let at = list.partition_point(|other| better(other, &entry) == Ordering::Less);
                    list.insert(at, entry);
                    list.truncate(n);
                }
                Ok((values, life, marks, best))
            })
            .try_reduce(empty, |(values, life, marks, mut ours), (_, _, _, theirs)| {
                for ((latest, list), (their_latest, their_list)) in ours.iter_mut().zip(theirs) {
                    match their_latest.cmp(latest) {
                        Ordering::Greater => (*latest, *list) = (their_latest, their_list),
                        Ordering::Equal => {
                            list.extend(their_list);
                            list.sort_unstable_by(better);
                            list.truncate(n);
                        }
                        Ordering::Less => {}
                    }
                }
                Ok((values, life, marks, ours))
            })?;
        Ok(best.into_iter().map(|(_, list)| list).collect())
    }
}

#[cfg(test)]
pub(crate) fn top_folding(
    recording: &Recording,
    run: u32,
    table: TableId,
    source: u32,
    view: (i32, i32, u32),
    n: usize,
    by: By,
) -> io::Result<Vec<Vec<(u128, Bin)>>> {
    top_with(recording, run, table, source, view.0, view.1, view.2, n, by, false)
}
