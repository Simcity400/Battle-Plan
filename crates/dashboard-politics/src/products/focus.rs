
use dashboard_base::products::{Country, validate_country};
use dashboard_base::products::{CountryParams, LoadContext, Product, check_reply_budget, at_hour::{At, AtHour, Keys as AtKeys, Read as AtRead}};
use crate::products::focus_dto::{FocusEntry, FocusHistory, FocusOrder, FocusProgress, FocusSegment};
use dashboard_base::Source;
use dashboard_text::metadata::check_cancel;
use dashboard_query::{Query, cost, identity::Holder};
use replay_lanes::lanes::{self, Cell, History, Keys};
use replay_data::rows::{FocusRow, TableId, tables};
use replay_frame::frame::{DecodedFrame, TableView};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, PoisonError, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub const PAGE_ROWS: u64 = 128;
const TOKEN_BYTES: usize = 512;

pub(crate) const NOT_HELD: &str = "No country held this tag";
pub(crate) const UNRESOLVED: &str = "The country holding this tag could not be resolved";
pub(crate) const NO_ROW: &str = "No focus row was recorded for this country";
pub(crate) const UNRECORDED: &str = "The country's focus row was not recorded";
pub(crate) const INCOMPLETE: &str = "The country's completed focus list was not recorded whole";

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FocusMissing {
    pub first_hour: i32,
    pub last_hour: i32,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Segment {
    pub first: i32,
    pub last: i32,
    pub slot: u32,
    pub id: u32,
    pub order: FocusOrder,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Progress {
    pub first: i32,
    pub last: i32,
    pub raw: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Order {
    tree: Option<Vec<u8>>,
    completed: Vec<String>,
    completed_bytes: Vec<Vec<u8>>,
    current: Option<String>,
    continuous: Option<String>,
    paused: bool,
}

#[derive(Clone, Debug)]
struct CarryState {
    who: (u32, u32),
    last_hour: i32,
    dates: BTreeMap<Vec<u8>, i32>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Fold {
    pub segments: Vec<Segment>,
    pub progress: Vec<Progress>,
    pub missing: Vec<FocusMissing>,
    pub completions: Vec<FocusEntry>,
    last: Option<(i32, (u32, u32))>,
    held: Option<((u32, u32), Order)>,
    carry: Option<CarryState>,
}

struct Columns {
    tree: usize,
    current: usize,
    continuous: usize,
    progress: usize,
    paused: usize,
    count: usize,
    token: usize,
}

impl Columns {
    fn new() -> io::Result<Self> {
        let focus = |name| dashboard_declare::field(TableId::Focus, name);
        Ok(Self {
            tree: focus("tree")?,
            current: focus("current")?,
            continuous: focus("continuous")?,
            progress: focus("progress")?,
            paused: focus("paused")?,
            count: focus("completed_count")?,
            token: dashboard_declare::field(TableId::CompletedFocuses, "token")?,
        })
    }

    fn focus(&self) -> [usize; 6] {
        [self.tree, self.current, self.continuous, self.progress, self.paused, self.count]
    }
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn place(history: &History, column: usize) -> io::Result<usize> {
    history.columns.binary_search(&column).map_err(|_| dashboard_core::invalid("A focus history omitted a column"))
}

pub(crate) fn read(query: &Query<'_>, tag: &Country, cancelled: &AtomicBool) -> io::Result<Fold> {
    let columns = Columns::new()?;
    let mut fold = Fold::default();
    let runs = query.holders.runs(tag);
    for unheld in &query.timeline.unheld {
        fold.miss(unheld.first, unheld.last, &unheld.reason);
    }
    for stretch in &query.timeline.stretches {
        check_cancel(cancelled)?;
        fold.last = None;
        fold.carry = None;
        let mut covered = stretch.first;
        for run in runs.iter().filter(|run| run.last >= stretch.first && run.first <= stretch.last) {
            let (first, last) = (run.first.max(stretch.first), run.last.min(stretch.last));
            if covered < first {
                fold.unheld(query, covered, first - 1);
            }
            covered = last.saturating_add(1);
            match run.holder {
                Holder::Found { slot, id } => fold.country(query, &columns, stretch.run, (slot, id), first, last, cancelled)?,
                Holder::Unresolved => {
                    fold.miss(first, last, UNRESOLVED);
                    fold.last = None;
                    fold.carry = None;
                }
            }
        }
        if covered <= stretch.last {
            fold.unheld(query, covered, stretch.last);
        }
    }
    fold.missing.sort_by_key(|missing| missing.first_hour);
    fold.completions.sort_by_key(|entry| (entry.completed_by_hour, entry.ordinal));
    Ok(fold)
}

impl Fold {
    fn miss(&mut self, first: i32, last: i32, reason: &str) {
        if let Some(held) = self.missing.last_mut()
            && held.reason == reason
            && i64::from(held.last_hour) + 1 == i64::from(first)
        {
            held.last_hour = last;
            return;
        }
        self.missing.push(FocusMissing { first_hour: first, last_hour: last, reason: reason.to_owned() });
    }

    fn unheld(&mut self, query: &Query<'_>, first: i32, last: i32) {
        self.last = None;
        self.carry = None;
        let mut from = first;
        for (start, end, reason) in query.holders.unknown.iter().filter(|(start, end, _)| *end >= first && *start <= last) {
            let (start, end) = ((*start).max(first), (*end).min(last));
            if from < start {
                self.miss(from, start - 1, NOT_HELD);
            }
            self.miss(start, end, reason);
            from = end.saturating_add(1);
        }
        if from <= last {
            self.miss(from, last, NOT_HELD);
        }
    }

    fn country(&mut self, query: &Query<'_>, columns: &Columns, run: u32, who: (u32, u32), first: i32, last: i32, cancelled: &AtomicBool) -> io::Result<()> {
        let recording = query.replay.recording();
        let key = u128::from(who.1);
        let focus = cost::storage(|| lanes::history(recording, run, TableId::Focus, &columns.focus(), Keys::These(&[key]), first, last))?;
        let done = cost::storage(|| lanes::history(recording, run, TableId::CompletedFocuses, &[columns.token], Keys::Range(key << 32, key << 32 | 0xFFFF_FFFF), first, last))?;
        let done_reads = if recording.predates(TableId::CompletedFocuses.index()).is_some() {
            Vec::new()
        } else {
            cost::storage(|| lanes::complete_reads(recording, run, TableId::CompletedFocuses, first, last))?
        };
        check_cancel(cancelled)?;
        let [tree, current, continuous, progress, paused, count] = columns.focus().map(|column| place(&focus, column));
        let [tree, current, continuous, progress, paused, count] = [tree?, current?, continuous?, progress?, paused?, count?];
        let steps = focus.keys.iter().find(|(held, _)| *held == key).map(|(_, steps)| steps.as_slice()).unwrap_or_default();
        let written = written_rows(recording, run, key, steps, [tree, current, continuous])?;
        let mut rows: Vec<(i32, u32, Option<Cell>)> = done
            .keys
            .iter()
            .flat_map(|(key, steps)| steps.iter().map(move |step| (step.hour, *key as u32, step.cells.as_ref().map(|cells| cells[0]))))
            .collect();
        rows.sort_by_key(|(hour, ordinal, _)| (*hour, *ordinal));
        let mut hours: Vec<i32> = steps
            .iter()
            .map(|step| step.hour)
            .chain(rows.iter().map(|(hour, ..)| *hour))
            .chain(done_reads.iter().flat_map(|&(start, end)| [Some(start), (end < last).then_some(end + 1)]).flatten())
            .chain([first])
            .collect();
        hours.sort_unstable();
        hours.dedup();
        let (mut step, mut row) = (0, 0);
        let mut cells: Option<&[Cell]> = None;
        let mut listed: BTreeMap<u32, Cell> = BTreeMap::new();
        let mut known: Option<Order> = None;
        for (index, &hour) in hours.iter().enumerate() {
            let end = hours.get(index + 1).map_or(last, |next| next - 1);
            let mut changed = known.is_none();
            let mut listed_changed = false;
            while let Some(next) = steps.get(step).filter(|next| next.hour <= hour) {
                let before = cells;
                cells = next.cells.as_deref();
                changed |= before.zip(cells).is_none_or(|(a, b)| [tree, current, continuous, paused, count].iter().any(|&at| a[at] != b[at]));
                step += 1;
            }
            while let Some(&(_, ordinal, cell)) = rows.get(row).filter(|(when, ..)| *when <= hour) {
                match cell {
                    Some(cell) => listed.insert(ordinal, cell),
                    None => listed.remove(&ordinal),
                };
                changed = true;
                listed_changed = true;
                row += 1;
            }
            let done_complete = read_complete(&done_reads, hour, end);
            let order = if changed {
                order_of(&done, &written, cells, &listed, [tree, current, continuous, paused, count])
            } else {
                known.clone().ok_or(NO_ROW)
            };
            let raw = cells.map(|cells| cells[progress]);
            match (order, raw) {
                (Ok(order), Some(Cell::Number(raw))) => {
                    known = Some(order.clone());
                    self.observe(hour, end, who, order, raw, done_complete, listed_changed);
                }
                (Ok(_), _) => {
                    known = None;
                    self.miss(hour, end, UNRECORDED);
                    self.last = None;
                    self.advance_carry_without_segment(hour, end, who, done_complete, listed_changed, &done, &listed);
                }
                (Err(reason), _) => {
                    known = None;
                    self.miss(hour, end, reason);
                    self.last = None;
                    self.advance_carry_without_segment(hour, end, who, done_complete, listed_changed, &done, &listed);
                }
            }
        }
        Ok(())
    }

    fn advance_carry_without_segment(&mut self, hour: i32, end: i32, who: (u32, u32), done_complete: bool, listed_changed: bool, done: &History, listed: &BTreeMap<u32, Cell>) {
        if !done_complete {
            self.carry = None;
            return;
        }
        let Some(carry) = self.carry.as_mut().filter(|carry| carry.who == who && i64::from(carry.last_hour) + 1 == i64::from(hour)) else {
            self.carry = None;
            return;
        };
        if !listed_changed || carry.dates.is_empty() {
            carry.last_hour = end;
            return;
        }
        let mut counts: BTreeMap<&[u8], usize> = BTreeMap::new();
        for &cell in listed.values() {
            let Some(bytes) = done.text(cell).filter(|token| !token.is_empty() && token.len() <= TOKEN_BYTES) else {
                self.carry = None;
                return;
            };
            if std::str::from_utf8(bytes).is_ok() {
                *counts.entry(bytes).or_default() += 1;
            }
        }
        carry.dates.retain(|token, _| counts.get(token.as_slice()) == Some(&1));
        carry.last_hour = end;
    }

    fn observe(&mut self, hour: i32, end: i32, who: (u32, u32), order: Order, raw: i64, done_complete: bool, listed_changed: bool) {
        let continuing = self.last.is_some_and(|(end, held)| i64::from(end) + 1 == i64::from(hour) && held == who);
        let unchanged = continuing && self.held.as_ref().is_some_and(|(held, before)| *held == who && *before == order);
        if unchanged {
            if let Some(segment) = self.segments.last_mut() {
                segment.last = end;
            }
            if done_complete {
                if let Some(carry) = self.carry.as_mut().filter(|carry| carry.who == who && i64::from(carry.last_hour) + 1 == i64::from(hour)) {
                    carry.last_hour = end;
                } else {
                    self.carry = Some(CarryState { who, last_hour: end, dates: BTreeMap::new() });
                }
            } else {
                self.carry = None;
            }
        } else {
            let before = self.held.take().filter(|(held, _)| *held == who).map(|(_, order)| order);
            let extends = before.as_ref().is_some_and(|before| {
                before.tree == order.tree
                    && before.completed_bytes.len() <= order.completed_bytes.len()
                    && before.completed_bytes.iter().zip(&order.completed_bytes).all(|(a, b)| a == b)
            });
            let prior: &[FocusEntry] = match (&before, self.segments.last()) {
                (Some(_), Some(segment)) if extends => &segment.order.completed,
                _ => &[],
            };
            let prev_carry = done_complete
                .then(|| self.carry.as_ref().filter(|carry| carry.who == who && i64::from(carry.last_hour) + 1 == i64::from(hour)))
                .flatten();
            let held = if extends { before.as_ref().map_or(0, |before| before.completed_bytes.len()) } else { 0 };
            let keep_carry = !listed_changed && extends && held == order.completed_bytes.len() && prev_carry.is_some();
            let now_counts = if done_complete && !keep_carry {
                let mut counts: BTreeMap<&[u8], usize> = BTreeMap::new();
                for bytes in &order.completed_bytes {
                    if std::str::from_utf8(bytes).is_ok() {
                        *counts.entry(bytes.as_slice()).or_default() += 1;
                    }
                }
                Some(counts)
            } else {
                None
            };
            let carry_dates = if extends { None } else { prev_carry.map(|carry| &carry.dates) };
            let completed: Vec<FocusEntry> = order
                .completed
                .iter()
                .zip(&order.completed_bytes)
                .enumerate()
                .map(|(position, (token, bytes))| FocusEntry {
                    ordinal: position as u32,
                    token: token.clone(),
                    completed_by_hour: if extends && position < held {
                        prior.get(position).and_then(|entry| entry.completed_by_hour)
                    } else if extends {
                        continuing.then_some(hour)
                    } else if now_counts.as_ref().is_some_and(|counts| counts.get(bytes.as_slice()) == Some(&1)) {
                        carry_dates.and_then(|dates| dates.get(bytes.as_slice())).copied()
                    } else {
                        None
                    },
                })
                .collect();
            if extends {
                self.completions.extend(completed[held..].iter().filter(|entry| entry.completed_by_hour.is_some()).cloned());
            }
            self.carry = if !done_complete {
                None
            } else if keep_carry {
                let mut carry = prev_carry.expect("checked").clone();
                carry.last_hour = end;
                Some(carry)
            } else {
                let counts = now_counts.as_ref().expect("built when done_complete and !keep_carry");
                let mut next_dates = BTreeMap::new();
                for (position, bytes) in order.completed_bytes.iter().enumerate() {
                    if counts.get(bytes.as_slice()) != Some(&1) {
                        continue;
                    }
                    let eligible = if extends && position >= held {
                        continuing.then_some(hour)
                    } else {
                        prev_carry.and_then(|carry| carry.dates.get(bytes.as_slice())).copied()
                    };
                    if let Some(date) = eligible {
                        next_dates.insert(bytes.clone(), date);
                    }
                }
                Some(CarryState { who, last_hour: end, dates: next_dates })
            };
            self.segments.push(Segment {
                first: hour,
                last: end,
                slot: who.0,
                id: who.1,
                order: FocusOrder { completed, current: order.current.clone(), continuous: order.continuous.clone(), paused: order.paused },
            });
            self.held = Some((who, order));
        }
        match self.progress.last_mut() {
            Some(run) if continuing && run.raw == raw => run.last = end,
            _ => self.progress.push(Progress { first: hour, last: end, raw }),
        }
        self.last = Some((end, who));
    }

    pub fn history(&self) -> FocusHistory {
        FocusHistory {
            segments: self.segments.iter().map(|segment| FocusSegment { first_hour: segment.first, last_hour: segment.last, order: segment.order.clone() }).collect(),
            progress: self.progress.iter().map(|run| FocusProgress { first_hour: run.first, last_hour: run.last, progress_raw: run.raw.to_string() }).collect(),
        }
    }

    pub fn segment_at(&self, hour: i32) -> Option<(usize, &Segment)> {
        let at = self.segments.partition_point(|segment| segment.last < hour);
        self.segments.get(at).filter(|segment| segment.first <= hour).map(|segment| (at, segment))
    }

    fn progress_at(&self, hour: i32) -> Option<usize> {
        let at = self.progress.partition_point(|run| run.last < hour);
        self.progress.get(at).filter(|run| run.first <= hour).map(|_| at)
    }

    pub fn progress(&self, hour: i32) -> Option<(i64, Option<i64>)> {
        let at = self.progress_at(hour)?;
        let run = self.progress[at];
        let gain = at.checked_sub(1).map(|before| self.progress[before]).and_then(|before| {
            let (_, now) = self.segment_at(hour)?;
            let (_, then) = self.segment_at(before.last)?;
            let contiguous = i64::from(before.last) + 1 == i64::from(run.first);
            let same = (now.slot, now.id) == (then.slot, then.id) && now.order.current.is_some() && now.order.current == then.order.current;
            (contiguous && same).then(|| run.raw - before.raw).filter(|gain| *gain > 0)
        });
        Some((run.raw, gain))
    }
}

fn written_rows(recording: &replay_recording::columnar_frames::Recording, run: u32, key: u128, steps: &[lanes::Step], places: [usize; 3]) -> io::Result<BTreeMap<i32, FocusRow>> {
    let stored = recording.run(run)?;
    let mut asks: Vec<(i32, u128)> = steps
        .iter()
        .filter_map(|step| step.cells.as_deref())
        .flat_map(|cells| places.map(|at| cells[at]))
        .filter_map(|cell| match cell {
            Cell::Written(hour) if stored.ordinal(hour).is_some() => Some((hour, key)),
            _ => None,
        })
        .collect();
    asks.sort_unstable();
    asks.dedup();
    let mut rows = BTreeMap::new();
    if asks.is_empty() {
        return Ok(rows);
    }
    let format = recording.format();
    cost::storage(|| {
        lanes::at_each(recording, run, TableId::Focus, None, &asks, &mut |hour, key, row| {
            if let Some((view, slot)) = row
                && let Some(row) = replay_frame::frame::cells_rows(TableId::Focus, &view, &[(key, slot)], format)?.rows::<tables::Focus>().and_then(|rows| rows.first().cloned())
            {
                rows.insert(hour, row);
            }
            Ok(())
        })
    })?;
    Ok(rows)
}

fn read_complete(reads: &[(i32, i32)], hour: i32, end: i32) -> bool {
    let at = reads.partition_point(|&(_, last)| last < hour);
    reads.get(at).is_some_and(|&(first, last)| first <= hour && end <= last)
}

fn order_of(done: &History, written: &BTreeMap<i32, FocusRow>, cells: Option<&[Cell]>, listed: &BTreeMap<u32, Cell>, places: [usize; 5]) -> Result<Order, &'static str> {
    let [tree, current, continuous, paused, count] = places;
    let cells = cells.ok_or(NO_ROW)?;
    let text = |cell: Cell, field: fn(&FocusRow) -> &Option<Vec<u8>>| -> Result<Option<Vec<u8>>, &'static str> {
        match cell {
            Cell::Written(hour) => Ok(field(written.get(&hour).ok_or(UNRECORDED)?).clone().filter(|bytes| !bytes.is_empty())),
            Cell::Missing => Ok(None),
            _ => Err(UNRECORDED),
        }
    };
    let number = |cell: Cell| match cell {
        Cell::Number(value) => Ok(value),
        _ => Err(UNRECORDED),
    };
    let count = number(cells[count])?;
    if usize::try_from(count).ok() != Some(listed.len()) {
        return Err(INCOMPLETE);
    }
    let mut completed = Vec::with_capacity(listed.len());
    let mut completed_bytes = Vec::with_capacity(listed.len());
    for (position, (&ordinal, &cell)) in listed.iter().enumerate() {
        let token = done.text(cell).filter(|token| !token.is_empty() && token.len() <= TOKEN_BYTES);
        match token {
            Some(token) if ordinal as usize == position => {
                completed.push(lossy(token));
                completed_bytes.push(token.to_vec());
            }
            _ => return Err(INCOMPLETE),
        }
    }
    Ok(Order {
        tree: text(cells[tree], |row| &row.tree)?,
        completed,
        completed_bytes,
        current: text(cells[current], |row| &row.current)?.map(|bytes| lossy(&bytes)),
        continuous: text(cells[continuous], |row| &row.continuous)?.map(|bytes| lossy(&bytes)),
        paused: number(cells[paused])? != 0,
    })
}

#[derive(Default)]
struct Kept(Option<(u64, Country, Arc<Fold>)>);

pub(crate) fn fold(source: &Source, branch: u64, tag: &Country, cancelled: &AtomicBool) -> io::Result<Arc<Fold>> {
    let state = source.state::<Kept>();
    let held = state.lock().unwrap_or_else(PoisonError::into_inner).0.as_ref().filter(|(at, held, _)| *at == branch && *held == *tag).map(|(_, _, fold)| Arc::clone(fold));
    if let Some(fold) = held {
        return Ok(fold);
    }
    let query = dashboard_base::queries::query(source, branch)?;
    let fold = Arc::new(read(&query, tag, cancelled)?);
    state.lock().unwrap_or_else(PoisonError::into_inner).0 = Some((branch, tag.clone(), Arc::clone(&fold)));
    Ok(fold)
}

fn branch_of(id: &str) -> io::Result<u64> {
    id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FocusPages {
    #[doc = " The order's place among the country's orders."]
    #[serde(with = "decimal")]
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "string"))]
    pub first: u64,
    #[serde(with = "decimal")]
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "string"))]
    pub count: u64,
    #[serde(with = "decimal")]
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "string"))]
    pub rows: u64,
}

mod decimal {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        String::deserialize(deserializer)?.parse().map_err(serde::de::Error::custom)
    }
}

fn pages(first: usize, rows: usize) -> FocusPages {
    FocusPages { first: first as u64, count: (rows as u64).div_ceil(PAGE_ROWS), rows: rows as u64 }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FocusHourParams {
    pub branch_id: String,
    pub country: Country,
    pub hour: i32,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FocusHour {
    pub slot: u32,
    pub country_id: u32,
    pub hour: i32,
    pub last_hour: i32,
    pub order_first_hour: Option<i32>,
    pub current: Option<String>,
    pub continuous: Option<String>,
    pub paused: bool,
    pub completed: FocusPages,
    pub at: i32,
    pub progress_raw: String,
    pub daily_gain_raw: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FocusHourReply {
    pub focus: Option<FocusHour>,
    pub missing: Option<FocusMissing>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FocusOrderPageParams {
    pub branch_id: String,
    pub country: Country,
    pub hour: i32,
    #[doc = " The pages `focus-hour` gave for the hour."]
    pub pages: FocusPages,
    #[serde(with = "decimal")]
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "string"))]
    pub page: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FocusOrderPage {
    pub entries: Option<Vec<FocusEntry>>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FocusCompletions {
    pub completions: Vec<FocusEntry>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(FocusHourReply, FocusOrderPage, FocusCompletions, FocusUnavailable);

pub(crate) fn hour_reply(fold: &Fold, hour: i32) -> FocusHourReply {
    let focus = fold.segment_at(hour).and_then(|(index, segment)| {
        let (raw, gain) = fold.progress(hour)?;
        Some(FocusHour {
            slot: segment.slot,
            country_id: segment.id,
            hour: segment.first,
            last_hour: segment.last,
            order_first_hour: Some(segment.first),
            current: segment.order.current.clone(),
            continuous: segment.order.continuous.clone(),
            paused: segment.order.paused,
            completed: pages(index, segment.order.completed.len()),
            at: hour,
            progress_raw: raw.to_string(),
            daily_gain_raw: gain.map(|gain| gain.to_string()),
        })
    });
    let missing = if focus.is_some() {
        None
    } else {
        let at = fold.missing.partition_point(|missing| missing.last_hour < hour);
        fold.missing.get(at).filter(|missing| missing.first_hour <= hour).cloned()
    };
    FocusHourReply { focus, missing, warnings: Vec::new() }
}

pub(crate) fn order_page(fold: &Fold, hour: i32, asked: &FocusPages, page: u64) -> io::Result<FocusOrderPage> {
    let (index, segment) = fold.segment_at(hour).ok_or_else(|| dashboard_core::refused("The country's focus was not recorded at this hour"))?;
    let completed = &segment.order.completed;
    if pages(index, completed.len()) != *asked {
        return Err(dashboard_core::invalid("The focus order at this hour changed. Refresh this view."));
    }
    let first = page.saturating_mul(PAGE_ROWS);
    let entries = (page < asked.count).then(|| completed.iter().skip(first as usize).take(PAGE_ROWS as usize).cloned().collect());
    Ok(FocusOrderPage { entries, warnings: Vec::new() })
}

pub struct FocusHourProduct;

impl Product for FocusHourProduct {
    const KEY: &'static str = "focus-hour";
    const CONTENT_DEPENDENT: bool = false;
    type Params = FocusHourParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, hour: Read];
    type Reply = FocusHourReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![FocusHourParams::decl(), FocusHourReply::decl(), FocusHour::decl(), FocusPages::decl(), FocusMissing::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        validate_country(&params.country)
    }

    fn load(source: &Source, params: &Self::Params, _: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let fold = fold(source, branch_of(&params.branch_id)?, &params.country, cancelled)?;
        Ok(hour_reply(&fold, params.hour))
    }
}

pub struct FocusOrderPageProduct;

impl Product for FocusOrderPageProduct {
    const KEY: &'static str = "focus-order-page";
    const CONTENT_DEPENDENT: bool = false;
    type Params = FocusOrderPageParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, hour: Read, pages, page];
    type Reply = FocusOrderPage;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![FocusOrderPageParams::decl(), FocusOrderPage::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        validate_country(&params.country)
    }

    fn load(source: &Source, params: &Self::Params, _: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let fold = fold(source, branch_of(&params.branch_id)?, &params.country, cancelled)?;
        order_page(&fold, params.hour, &params.pages, params.page)
    }
}

pub struct FocusCompletionsProduct;

impl Product for FocusCompletionsProduct {
    const KEY: &'static str = "focus-completions";
    const CONTENT_DEPENDENT: bool = false;
    type Params = CountryParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country];
    type Reply = FocusCompletions;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![FocusCompletions::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }

    fn load(source: &Source, params: &Self::Params, _: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let fold = fold(source, params.branch(), &params.country, cancelled)?;
        let reply = FocusCompletions { completions: fold.completions.clone(), warnings: Vec::new() };
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}

const FOCUS_TABLES: [TableId; 4] = [TableId::Countries, TableId::CountryLinks, TableId::Focus, TableId::CompletedFocuses];

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FocusUnavailableParams {
    pub branch_id: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FocusTableRefusal {
    pub table: String,
    pub reason: Option<String>,
    pub missing_rows: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FocusUnavailable {
    pub hour: i32,
    pub reasons: Vec<FocusTableRefusal>,
    pub warnings: Vec<String>,
}

pub(crate) fn refusals(frame: &DecodedFrame) -> Vec<FocusTableRefusal> {
    FOCUS_TABLES
        .iter()
        .filter_map(|table| {
            let name = format!("{table:?}");
            match &*frame.tables[table.index()] {
                TableView::Complete(_) => None,
                TableView::Truncated { missing, .. } => Some(FocusTableRefusal { table: name, reason: None, missing_rows: Some(missing.to_string()) }),
                TableView::Unavailable { reason } => Some(FocusTableRefusal { table: name, reason: Some(reason.clone()), missing_rows: None }),
                TableView::NotRecorded { .. } => Some(FocusTableRefusal { table: name, reason: Some("Not recorded in this capture mode".into()), missing_rows: None }),
            }
        })
        .collect()
}

pub struct FocusUnavailableAt;

impl AtHour for FocusUnavailableAt {
    const KEY: &'static str = "focus-unavailable";
    const TITLE: &'static str = "Focus";
    const CONTENT_DEPENDENT: bool = false;
    type Params = FocusUnavailableParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId];
    type Reply = FocusUnavailable;
    type Context = ();

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![FocusUnavailableParams::decl(), FocusUnavailable::decl(), FocusTableRefusal::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid branch")
    }

    fn tables(_: &Self::Params) -> Vec<TableId> {
        FOCUS_TABLES.to_vec()
    }

    fn reads(_: &Self::Params, _: Option<dashboard_base::products::at_hour::Owner>, step: usize, _: &DecodedFrame) -> Vec<AtRead> {
        if step > 0 {
            return Vec::new();
        }
        FOCUS_TABLES.iter().map(|&table| AtRead { table, keys: AtKeys::These(Vec::new()) }).collect()
    }

    fn context(_: &Source, _: &LoadContext) -> io::Result<Self::Context> {
        Ok(())
    }

    fn reply(frame: &DecodedFrame, _: &Self::Params, _: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        Ok(FocusUnavailable { hour: frame.hour, reasons: refusals(frame), warnings: Vec::new() })
    }
}

#[cfg(test)]
mod tests;
