
use crate::{
    answer::{self, State, Table},
    invalid,
    query::{Query, Rows, SeriesMode, TagRow, Tags, tags_at},
    table_name, table_words,
};
use dashboard_query::{coverage::{Coverage, CoverageScope, row_coverage}, series::board::{measure_unrecorded, table_unrecorded}};
use playhead::lane::Lane;
use replay_columnar::{
    manifest::Status,
    state::Kind,
    store::{TableState, Value},
    summary::{Cells, Computed, Source},
};
use replay_data::{Hour, rows::{NavalLossTotalRow, TableId}};
use std::{cmp::Ordering, collections::{HashMap, HashSet}, io};

pub(crate) trait Find {
    fn slot(&self, key: u128) -> Option<u32>;
    fn ordered(&self) -> &[(u128, u32)];
}

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

pub(crate) fn coverage_of(table: usize, status: Status, reason: Option<&str>, witnessed: bool) -> Coverage {
    if let Some(&id) = TableId::ALL.get(table) {
        return row_coverage(id, status, reason, witnessed);
    }
    let name = table_words(table);
    match status {
        Status::Complete => Coverage::Recorded,
        Status::Truncated { .. } if witnessed => Coverage::Recorded,
        Status::NotRecorded => Coverage::NotRecorded(format!("{name} not recorded at this hour")),
        Status::Truncated { missing } => Coverage::NotRecorded(format!("{name} read only in part at this hour ({missing} rows not read)")),
        Status::Unavailable { .. } => Coverage::NotRecorded(match reason.filter(|reason| !reason.is_empty()) {
            Some(reason) => format!("{name} not available at this hour: {reason}"),
            None => format!("{name} not available at this hour"),
        }),
    }
}

pub(crate) fn unrecorded(table: usize, status: Status, reason: Option<&str>) -> Option<String> {
    if matches!(status, Status::Complete) {
        return None;
    }
    if let Some(&id) = TableId::ALL.get(table) {
        return table_unrecorded(id, status, reason);
    }
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

    pub(crate) fn row_coverage(&self, lane: &Lane, witnessed: bool) -> Coverage {
        if let Some(reason) = lane.withheld() {
            return Coverage::NotRecorded(reason.to_owned());
        }
        let status = lane.status().0;
        let text = match status {
            Status::Unavailable { reason } => self.0.get(&(lane.run(), reason)).map(|text| &**text),
            _ => None,
        };
        coverage_of(lane.table(), status, text, witnessed)
    }

    pub(crate) fn unrecorded(&self, lane: &Lane) -> Option<String> {
        if let Some(reason) = lane.withheld() {
            return Some(reason.to_owned());
        }
        let status = lane.status().0;
        let text = match status {
            Status::Unavailable { reason } => self.0.get(&(lane.run(), reason)).map(|text| &**text),
            _ => None,
        };
        unrecorded(lane.table(), status, text)
    }
}

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

pub(crate) fn holds(query: &Query, lane: &Lane) -> bool {
    lane.head().is_some() && query.declared().iter().all(|column| lane.requested().contains(column))
}

#[inline]
pub(crate) fn number(value: Value<'_>) -> (i64, State) {
    match value {
        Value::Number(bits) => (bits as i64, State::Value),
        Value::Missing => (0, State::Missing),
        Value::Unrecorded | Value::Text(_) | Value::List(_) => (0, State::Unrecorded),
    }
}

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

#[inline]
pub(crate) fn series_value(source: &Source, cells: &TableState, slot: u32) -> (i64, State, Option<&'static str>) {
    match source {
        Source::Cell(column) => {
            let (value, state) = number(cells.value(*column, slot));
            (value, state, None)
        }
        Source::Computed { value, .. } => match value(&Row(cells), slot) {
            Computed::Value(value) => (value, State::Value, None),
            Computed::NotApplicable(reason) => (0, State::Missing, Some(reason)),
            Computed::Unrecorded => (0, State::Unrecorded, None),
        },
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
        let Query::Series { series, countries: names, mode: SeriesMode::Top { n, least }, .. } = query else { return None };
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
            order.sort_by_cached_key(|&i| names[i].key());
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
            let (value, State::Value, _) = series_value(&source, cells, slot) else { continue };
            let ranked = (if *least { value } else { !value }, self.order[at as usize], key);
            self.sorted.insert(ranked);
            self.slots.insert(slot, ranked);
        }
        table.clear(1, false);
        let mut top: Vec<(i64, u32, u128)> = self.sorted.iter().take(*n).copied().collect();
        if series.absent_none {
            let held: HashSet<u128> = cells.rows().filter_map(|slot| cells.key(slot)).collect();
            let zero = if *least { 0 } else { !0 };
            let mut absent: Vec<(i64, u32, u128)> = tags.by_key.iter().filter(|(key, _)| !held.contains(key)).map(|(&key, &at)| (zero, self.order[at as usize], key)).collect();
            if absent.len() > *n && *n > 0 {
                absent.select_nth_unstable(*n - 1);
                absent.truncate(*n);
            }
            top.extend(absent);
            top.sort_unstable();
            top.truncate(*n);
        }
        for (value, _, key) in top {
            table.row(key);
            table.cell(if *least { value } else { !value }, State::Value);
            table.name(&names[tags.by_key[&key] as usize].key());
        }
        table.encode(out);
        Some(read)
    }
}

#[derive(Default)]
pub(crate) struct Scratch {
    pub(crate) table: Table,
    ranked: Vec<(i64, u128, u32)>,
}

fn by_rank(a: &(i64, u128, u32), b: &(i64, u128, u32)) -> Ordering {
    b.0.cmp(&a.0).then(a.1.cmp(&b.1))
}

pub(crate) fn write(query: &Query, lane: &Lane, find: &dyn Find, tags: &Tags, reasons: &Reasons, scratch: &mut Scratch, out: &mut Vec<u8>) {
    out.clear();
    let keyed = query.witness_sensitive();
    if keyed {
        if let Coverage::NotRecorded(reason) = reasons.row_coverage(lane, true) {
            answer::not_recorded(&reason, out);
            return;
        }
    } else if let Some(reason) = reasons.unrecorded(lane) {
        answer::not_recorded(&reason, out);
        return;
    }
    let read_rows = keyed.then(|| lane.read_rows()).flatten();
    let slot_of = |key: u128| find.slot(key).filter(|slot| read_rows.is_none_or(|read| read.binary_search(slot).is_ok()));
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
                        let slot = slot_of(key);
                        match reasons.row_coverage(lane, slot.is_some()) {
                            Coverage::NotRecorded(reason) => {
                                table.row(key);
                                table.absent(&reason);
                            }
                            Coverage::Recorded => match slot {
                                Some(slot) => row(table, key, slot),
                                None => {
                                    table.row(key);
                                    table.absent(crate::live::NO_ROW);
                                }
                            },
                        }
                    }
                }
                Rows::LiveTable(keys) => {
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
        Query::Series { countries: names, mode: SeriesMode::NavalLosses { attribution, kinds, top }, .. } => {
            let hour = head(lane);
            for &kind in *kinds {
                let Some(slot) = find.slot(NavalLossTotalRow::run_key(kind)) else {
                    answer::not_recorded("Naval losses are not known at this hour", out);
                    return;
                };
                let known = match cells.value(3, slot) {
                    Value::Number(bits) => bits as i64 != 0,
                    _ => false,
                };
                let counted_from = match cells.value(2, slot) {
                    Value::Number(bits) => i32::try_from(bits as i64).ok(),
                    _ => None,
                };
                if !known || counted_from.is_none_or(|from| from > hour) {
                    answer::not_recorded("Naval losses are not known at this hour", out);
                    return;
                }
            }
            table.clear(1, false);
            let sum_for = |owner: u128| -> i64 {
                kinds.iter().fold(0i64, |sum, &kind| {
                    let count = find
                        .slot(NavalLossTotalRow::holder_key(owner, *attribution, kind))
                        .and_then(|slot| match cells.value(0, slot) {
                            Value::Number(bits) => Some(bits as i64),
                            _ => None,
                        })
                        .unwrap_or(0);
                    sum.saturating_add(count)
                })
            };
            match top {
                Some(n) => {
                    ranked.clear();
                    for (at, tag) in tags.rows.iter().enumerate() {
                        let TagRow::Key(owner) = tag else { continue };
                        let total = sum_for(*owner);
                        if total > 0 {
                            ranked.push((total, *owner, at as u32));
                        }
                    }
                    let by_tag = |a: &(i64, u128, u32), b: &(i64, u128, u32)| {
                        b.0.cmp(&a.0).then_with(|| names[a.2 as usize].key().cmp(&names[b.2 as usize].key()))
                    };
                    if ranked.len() > *n {
                        ranked.select_nth_unstable_by(*n - 1, by_tag);
                        ranked.truncate(*n);
                    }
                    ranked.sort_unstable_by(by_tag);
                    for &(value, owner, at) in ranked.iter() {
                        table.row(owner);
                        table.cell(value, State::Value);
                        table.name(&names[at as usize].key());
                    }
                }
                None => {
                    for (at, tag) in tags.rows.iter().enumerate() {
                        match tag {
                            TagRow::Absent(reason) => {
                                table.row(0);
                                table.name(&names[at].key());
                                table.absent(reason);
                            }
                            TagRow::Key(owner) => {
                                table.row(*owner);
                                table.cell(sum_for(*owner), State::Value);
                                table.name(&names[at].key());
                            }
                        }
                    }
                }
            }
        }
        Query::Series { series, countries: names, mode: SeriesMode::Parts { top }, .. } => {
            let source = query.source().expect("a checked series");
            let owned = series.parts.expect("a checked sub-keyed series");
            table.clear(1, false);
            for (at, tag) in tags.rows.iter().enumerate() {
                let owner = match tag {
                    TagRow::Absent(reason) => {
                        table.row(0);
                        table.name(&names[at].key());
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
                            let (value, state, reason) = series_value(&source, cells, slot);
                            table.row(key);
                            table.cell(value, state);
                            table.name(&names[at].key());
                            if let Some(reason) = reason {
                                table.explain(reason);
                            }
                        }
                    }
                    Some(n) => {
                        ranked.clear();
                        ranked.extend(rows.filter_map(|&(key, slot)| match series_value(&source, cells, slot) {
                            (value, State::Value, _) => Some((value, key, slot)),
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
                            table.name(&names[at].key());
                        }
                    }
                }
                if table.rows() == before {
                    table.row(0);
                    table.name(&names[at].key());
                    table.absent(&format!("{} holds no row of {} at this hour", names[at], series.kind));
                }
            }
        }
        Query::Series { series, countries: names, mode: SeriesMode::Top { n, least }, .. } => {
            let source = query.source().expect("a checked series");
            table.clear(1, false);
            ranked.clear();
            ranked.extend(cells.rows().filter_map(|slot| match series_value(&source, cells, slot) {
                (value, State::Value, _) => {
                    let key = cells.key(slot)?;
                    Some((value, key, *tags.by_key.get(&key)?))
                }
                _ => None,
            }));
            if series.absent_none {
                let held: HashSet<u128> = cells.rows().filter_map(|slot| cells.key(slot)).collect();
                ranked.extend(tags.by_key.iter().filter(|(key, _)| !held.contains(key)).map(|(&key, &at)| (0, key, at)));
            }
            let by_tag = |a: &(i64, u128, u32), b: &(i64, u128, u32)| {
                let (a0, b0) = if *least { (b.0, a.0) } else { (a.0, b.0) };
                b0.cmp(&a0).then_with(|| names[a.2 as usize].key().cmp(&names[b.2 as usize].key()))
            };
            if ranked.len() > *n {
                ranked.select_nth_unstable_by(*n - 1, by_tag);
                ranked.truncate(*n);
            }
            ranked.sort_unstable_by(by_tag);
            for &(value, key, at) in ranked.iter() {
                table.row(key);
                table.cell(value, State::Value);
                table.name(&names[at as usize].key());
            }
        }
        Query::Series { series, measure, countries: names, mode: SeriesMode::Keyed(scope), identity } => {
            let source = query.source().expect("a checked series");
            let hour = head(lane);
            table.clear(1, false);
            let unread = |at: usize| measure_unrecorded(&identity.holders.name(&names[at], hour), measure.name);
            for (at, tag) in tags.rows.iter().enumerate() {
                let key = match tag {
                    TagRow::Absent(reason) => {
                        table.row(0);
                        table.name(&names[at].key());
                        table.absent(reason);
                        continue;
                    }
                    TagRow::Key(key) => *key,
                };
                table.row(key);
                table.name(&names[at].key());
                let slot = slot_of(key);
                let covered = match scope {
                    CoverageScope::Table => Coverage::Recorded,
                    CoverageScope::Row => reasons.row_coverage(lane, slot.is_some()),
                };
                match covered {
                    Coverage::NotRecorded(reason) => table.absent(&reason),
                    Coverage::Recorded => match slot {
                        None if series.absent_none => table.cell(0, State::Value),
                        None => table.absent(&unread(at)),
                        Some(slot) => {
                            let (value, state, reason) = series_value(&source, cells, slot);
                            table.cell(value, state);
                            if state != State::Value {
                                match reason {
                                    Some(reason) => table.explain(reason),
                                    None => table.explain(&unread(at)),
                                }
                            }
                        }
                    },
                }
            }
        }
    }
    table.encode(out);
}

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

pub(crate) fn head(lane: &Lane) -> Hour {
    lane.head().expect("a sought lane")
}
