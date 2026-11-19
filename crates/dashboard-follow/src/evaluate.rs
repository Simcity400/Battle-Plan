//! A query's answer from a lane's cells at the lane's head: the one product function, run by the
//! hook on the playhead's lanes and by [`answer_at`] on a lane sought to any hour.

use crate::{
    answer::{self, State, Table},
    invalid,
    query::{Query, Rows, TagRow, Tags, tags_at},
    table_name, table_words,
};
use playhead::lane::Lane;
use replay_columnar::{
    manifest::Status,
    state::Kind,
    store::{TableState, Value},
    summary::{Cells, Source},
};
use replay_data::Hour;
use std::{cmp::Ordering, collections::HashMap, io};

/// Where a query's rows are in a table's cells.
pub(crate) trait Find {
    /// The slot of the live row with `key`.
    fn slot(&self, key: u128) -> Option<u32>;
    /// Every live row, by key ascending.
    fn ordered(&self) -> &[(u128, u32)];
}

/// The rows of a table's cells, read once.
pub(crate) struct Scan {
    slots: HashMap<u128, u32>,
    ordered: Vec<(u128, u32)>,
}

impl Scan {
    pub(crate) fn new(cells: &TableState) -> Self {
        let mut ordered: Vec<(u128, u32)> = cells.rows().filter_map(|slot| Some((cells.key(slot)?, slot))).collect();
        ordered.sort_unstable();
        Self { slots: ordered.iter().copied().collect(), ordered }
    }
}

impl Find for Scan {
    fn slot(&self, key: u128) -> Option<u32> {
        self.slots.get(&key).copied()
    }

    fn ordered(&self) -> &[(u128, u32)] {
        &self.ordered
    }
}

/// Why a table of `status` answers nothing at this hour, in a user's words whatever the table's
/// number (no verb to agree): an unavailable table with its recorded reason's text where it is
/// known, never the reason's id.
pub(crate) fn unrecorded(table: usize, status: Status, reason: Option<&str>) -> Option<String> {
    let name = table_words(table);
    match status {
        Status::Complete => None,
        Status::NotRecorded => Some(format!("{name} not recorded at this hour")),
        Status::Truncated { missing } => Some(format!("{name} read only in part at this hour ({missing} rows not read), and which rows were read is not held at the head")),
        Status::Unavailable { .. } => Some(match reason.filter(|reason| !reason.is_empty()) {
            Some(reason) => format!("{name} not available at this hour: {reason}"),
            None => format!("{name} not available at this hour"),
        }),
    }
}

/// The text of each unavailable table's reason met, by run and id, kept for the steps whose head
/// sits between parts, where no part names it ([`Lane::reason`]). One entry a distinct reason.
#[derive(Default)]
pub(crate) struct Reasons(HashMap<(usize, u32), Box<str>>);

impl Reasons {
    pub(crate) fn learn(&mut self, lane: &Lane) {
        if let Status::Unavailable { reason } = lane.status().0
            && !self.0.contains_key(&(lane.run(), reason))
            && let Some(text) = lane.reason()
        {
            self.0.insert((lane.run(), reason), String::from_utf8_lossy(text).into());
        }
    }

    /// Why `lane`'s table answers nothing at its head, if it does not.
    pub(crate) fn unrecorded(&self, lane: &Lane) -> Option<String> {
        let status = lane.status().0;
        let text = match status {
            Status::Unavailable { reason } => self.0.get(&(lane.run(), reason)).map(|text| &**text),
            _ => None,
        };
        unrecorded(lane.table(), status, text)
    }
}

/// Why `query` cannot be answered from cells of these kinds, by name.
pub(crate) fn refusal(query: &Query, cells: &TableState) -> Option<String> {
    for column in query.declared() {
        let kind = cells.column(column).map(|held| held.kind())?;
        let what = match kind {
            Kind::Bytes => "a text",
            Kind::U32List | Kind::U64List | Kind::Opaque => "a list",
            _ => continue,
        };
        let name = replay_data::rows::TableId::ALL.get(query.table()).and_then(|id| id.field_name(column)).unwrap_or("?");
        return Some(format!("{}'s {name} is {what} column, which a followed query does not read", table_name(query.table())));
    }
    None
}

/// The lane is sought and holds every column `query` reads.
pub(crate) fn holds(query: &Query, lane: &Lane) -> bool {
    lane.head().is_some() && query.declared().iter().all(|column| lane.columns().contains(column))
}

#[inline]
pub(crate) fn number(value: Value<'_>) -> (i64, State) {
    match value {
        Value::Number(bits) => (bits as i64, State::Value),
        Value::Missing => (0, State::Missing),
        Value::Unrecorded | Value::Text(_) | Value::List(_) => (0, State::Unrecorded),
    }
}

/// A lane's cells as a computed column reads a row.
struct Row<'a>(&'a TableState);

impl Cells for Row<'_> {
    fn number(&self, column: usize, slot: u32) -> Option<i64> {
        match self.0.value(column, slot) {
            Value::Number(bits) => Some(bits as i64),
            _ => None,
        }
    }

    fn key(&self, slot: u32) -> Option<u128> {
        self.0.key(slot)
    }

    fn slot(&self, key: u128) -> Option<u32> {
        self.0.find(key)
    }

    fn each_row(&self, each: &mut dyn FnMut(u32)) {
        self.0.rows().for_each(each);
    }
}

/// A series' value at `slot`: its cell, or its computed value from the row's cells (not recorded
/// when an input is not).
#[inline]
pub(crate) fn series_value(source: &Source, cells: &TableState, slot: u32) -> (i64, State) {
    match source {
        Source::Cell(column) => number(cells.value(*column, slot)),
        Source::Computed { value, .. } => value(&Row(cells), slot).map_or((0, State::Unrecorded), |value| (value, State::Value)),
    }
}

#[derive(Default)]
pub(crate) struct Ranking {
    ready: bool,
    sorted: std::collections::BTreeSet<(i64, u32, u128)>,
    slots: HashMap<u32, (i64, u32, u128)>,
    order: Vec<u32>,
    dirty: Vec<u32>,
}

impl Ranking {
    pub(crate) fn clear(&mut self) { self.ready = false; }

    pub(crate) fn bytes(&self) -> usize {
        self.sorted.len() * (32 + 16) + self.slots.capacity() * 48 + (self.order.capacity() + self.dirty.capacity()) * 4
    }

    pub(crate) fn write(&mut self, query: &Query, lane: &Lane, changes: Option<&playhead::changes::Changes>, tags: &Tags, reasons: &Reasons, table: &mut Table, out: &mut Vec<u8>) -> Option<u64> {
        let Query::Series { tags: names, top: Some(n), least, parts: false, .. } = query else { return None };
        let source = query.source()?;
        let cells = lane.cells();
        out.clear();
        if let Some(reason) = reasons.unrecorded(lane) {
            self.clear();
            answer::not_recorded(&reason, out);
            return Some(0);
        }
        self.dirty.clear();
        if self.ready && let Some(c) = changes.filter(|c| !c.reset() && c.renumber().is_none()) {
            for slot in c.deleted().slots() {
                if let Some(old) = self.slots.remove(slot) { self.sorted.remove(&old); }
            }
            self.dirty.extend(c.inserted().slots());
            let columns = query.declared();
            self.dirty.extend(c.columns().iter().filter(|w| columns.contains(&w.column())).flat_map(|w| w.slots()));
            self.dirty.sort_unstable();
            self.dirty.dedup();
        } else {
            self.slots.clear();
            self.sorted.clear();
            let mut order: Vec<_> = (0..names.len()).collect();
            order.sort_unstable_by_key(|&i| &names[i]);
            self.order.resize(order.len(), 0);
            for (rank, i) in order.into_iter().enumerate() { self.order[i] = rank as u32; }
            self.dirty.extend(cells.rows());
            self.ready = true;
        }
        let mut read = 0;
        for &slot in &self.dirty {
            if let Some(old) = self.slots.remove(&slot) { self.sorted.remove(&old); }
            let Some(key) = cells.key(slot) else { continue };
            let Some(&at) = tags.by_key.get(&key) else { continue };
            read += 1;
            let (value, State::Value) = series_value(&source, cells, slot) else { continue };
            let ranked = (if *least { value } else { !value }, self.order[at as usize], key);
            self.sorted.insert(ranked);
            self.slots.insert(slot, ranked);
        }
        table.clear(1, false);
        for &(value, _, key) in self.sorted.iter().take(*n) {
            table.row(key);
            table.cell(if *least { value } else { !value }, State::Value);
            table.name(&names[tags.by_key[&key] as usize]);
        }
        table.encode(out);
        Some(read)
    }
}

/// Reusable buffers of the answers made on one thread.
#[derive(Default)]
pub(crate) struct Scratch {
    pub(crate) table: Table,
    ranked: Vec<(i64, u128, u32)>,
}

fn by_rank(a: &(i64, u128, u32), b: &(i64, u128, u32)) -> Ordering {
    b.0.cmp(&a.0).then(a.1.cmp(&b.1))
}

/// `query`'s answer at `lane`'s head into `out`, a series' tags as `tags` (its [`tag_rows`] at the
/// head); `query` reads no appended rows.
pub(crate) fn write(query: &Query, lane: &Lane, find: &dyn Find, tags: &Tags, reasons: &Reasons, scratch: &mut Scratch, out: &mut Vec<u8>) {
    out.clear();
    if let Some(reason) = reasons.unrecorded(lane) {
        answer::not_recorded(&reason, out);
        return;
    }
    let cells = lane.cells();
    let Scratch { table, ranked } = scratch;
    match query {
        Query::Head(_) => {
            answer::not_recorded("A head value is not read from the playhead's tables", out);
            return;
        }
        Query::Cells { columns, rows, .. } => {
            table.clear(columns.len(), false);
            let row = |table: &mut Table, key: u128, slot: u32| {
                table.row(key);
                for &column in columns {
                    let (value, state) = number(cells.value(column, slot));
                    table.cell(value, state);
                }
            };
            match rows {
                Rows::Keys(keys) | Rows::Live(keys) => {
                    for &key in keys {
                        match find.slot(key) {
                            Some(slot) => row(table, key, slot),
                            None => {
                                table.row(key);
                                table.absent(crate::live::NO_ROW);
                            }
                        }
                    }
                }
                Rows::All => find.ordered().iter().for_each(|&(key, slot)| row(table, key, slot)),
                Rows::Top { column, n } => {
                    ranked.clear();
                    ranked.extend(cells.rows().filter_map(|slot| match number(cells.value(*column, slot)) {
                        (value, State::Value) => Some((value, cells.key(slot)?, slot)),
                        _ => None,
                    }));
                    if ranked.len() > *n {
                        ranked.select_nth_unstable_by(*n - 1, by_rank);
                        ranked.truncate(*n);
                    }
                    ranked.sort_unstable_by(by_rank);
                    ranked.iter().for_each(|&(_, key, slot)| row(table, key, slot));
                }
                Rows::Appended | Rows::Listed => {}
            }
        }
        Query::Series { series, tags: names, top, parts: true, .. } => {
            let source = query.source().expect("a checked series");
            let owned = series.parts.expect("a checked sub-keyed series");
            table.clear(1, false);
            for (at, tag) in tags.rows.iter().enumerate() {
                let owner = match tag {
                    TagRow::Absent(reason) => {
                        table.row(0);
                        table.name(&names[at]);
                        table.absent(reason);
                        continue;
                    }
                    TagRow::Key(key) => *key,
                };
                let (lo, hi) = owned.range(owner);
                let ordered = find.ordered();
                let rows = &ordered[ordered.partition_point(|(key, _)| *key < lo)..];
                let rows = rows.iter().take_while(|(key, _)| *key <= hi);
                let before = table.rows();
                match top {
                    None => {
                        for &(key, slot) in rows {
                            let (value, state) = series_value(&source, cells, slot);
                            table.row(key);
                            table.cell(value, state);
                            table.name(&names[at]);
                        }
                    }
                    Some(n) => {
                        ranked.clear();
                        ranked.extend(rows.filter_map(|&(key, slot)| match series_value(&source, cells, slot) {
                            (value, State::Value) => Some((value, key, slot)),
                            _ => None,
                        }));
                        if ranked.len() > *n {
                            ranked.select_nth_unstable_by(*n - 1, by_rank);
                            ranked.truncate(*n);
                        }
                        ranked.sort_unstable_by(by_rank);
                        for &(value, key, _) in ranked.iter() {
                            table.row(key);
                            table.cell(value, State::Value);
                            table.name(&names[at]);
                        }
                    }
                }
                if table.rows() == before {
                    table.row(0);
                    table.name(&names[at]);
                    table.absent(&format!("{} holds no row of {} at this hour", names[at], series.kind));
                }
            }
        }
        Query::Series { tags: names, top: Some(n), least, .. } => {
            let source = query.source().expect("a checked series");
            table.clear(1, false);
            ranked.clear();
            ranked.extend(cells.rows().filter_map(|slot| match series_value(&source, cells, slot) {
                (value, State::Value) => {
                    let key = cells.key(slot)?;
                    Some((value, key, *tags.by_key.get(&key)?))
                }
                _ => None,
            }));
            // As the UI ranks (`compareTags`): greatest first, a tie by tag in byte order.
            let by_tag = |a: &(i64, u128, u32), b: &(i64, u128, u32)| {
                let (a0, b0) = if *least { (b.0, a.0) } else { (a.0, b.0) };
                b0.cmp(&a0).then_with(|| names[a.2 as usize].cmp(&names[b.2 as usize]))
            };
            if ranked.len() > *n {
                ranked.select_nth_unstable_by(*n - 1, by_tag);
                ranked.truncate(*n);
            }
            ranked.sort_unstable_by(by_tag);
            for &(value, key, at) in ranked.iter() {
                table.row(key);
                table.cell(value, State::Value);
                table.name(&names[at as usize]);
            }
        }
        Query::Series { measure, tags: names, top: None, .. } => {
            let source = query.source().expect("a checked series");
            table.clear(1, false);
            let unread = |at: usize| format!("{}'s {} was not recorded at this hour", names[at], measure.name);
            for (at, tag) in tags.rows.iter().enumerate() {
                let key = match tag {
                    TagRow::Absent(reason) => {
                        table.row(0);
                        table.name(&names[at]);
                        table.absent(reason);
                        continue;
                    }
                    TagRow::Key(key) => *key,
                };
                table.row(key);
                table.name(&names[at]);
                match find.slot(key) {
                    None => table.absent(&unread(at)),
                    Some(slot) => {
                        let (value, state) = series_value(&source, cells, slot);
                        table.cell(value, state);
                        if state != State::Value {
                            table.explain(&unread(at));
                        }
                    }
                }
            }
        }
    }
    table.encode(out);
}

/// `query`'s answer from `lane`'s cells at the lane's head, as the hook pushes it: the product at
/// whatever hour the lane was sought or stepped to. A log has no answer at an hour (its answer is
/// the rows appended since its last push), and a lane that does not hold the query's columns is
/// refused.
pub fn answer_at(query: &Query, lane: &Lane) -> io::Result<Vec<u8>> {
    query.check()?;
    if query.appended() {
        return Err(invalid("A log has no answer at an hour: it is the rows appended since its last push"));
    }
    if query.listed() {
        return Err(invalid("A log's rows listed at an hour are read by ordinal through the playhead, not from a lane's cells"));
    }
    if lane.table() != query.table() || !holds(query, lane) {
        return Err(invalid(format!("The lane of {} is not sought or does not hold the query's columns", table_name(lane.table()))));
    }
    if let Some(refused) = refusal(query, lane.cells()) {
        return Err(invalid(refused));
    }
    let mut out = Vec::new();
    let mut reasons = Reasons::default();
    reasons.learn(lane);
    write(query, lane, &Scan::new(lane.cells()), &tags_at(query, head(lane)), &reasons, &mut Scratch::default(), &mut out);
    Ok(out)
}

/// The game hour of `lane`'s head.
pub(crate) fn head(lane: &Lane) -> Hour {
    lane.head().expect("a sought lane")
}
