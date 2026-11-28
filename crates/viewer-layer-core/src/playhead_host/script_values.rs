use super::{Entry, lock, views::{held_fields, prefix, view::{self, HeadFailure}}};
use playhead::{changes::Changes, lane::Lane, pages::TableWritten};
use replay_columnar::{manifest::Status, state::Kind, store::{ColumnState, Value}, unit::BLOCK};
use replay_data::{Hour, rows::{Field, TableId, TableRows}};
use replay_frame::frame::{KeyedNumber, KeyedNumbers, SharedView, TableView, cells_rows_of};
use std::{collections::BTreeSet, io, sync::{Arc, Mutex}};

#[cfg(test)]
mod tests;

pub(super) const TABLE: TableId = TableId::ScriptVariableValues;

enum Standing<T> {
    Recorded(T),
    NotRecorded,
    Withheld(String),
}

pub(super) struct ExactColumns {
    table: TableId,
    entry: Arc<Entry>,
    written: TableWritten,
    lane: Mutex<Option<Option<Lane>>>,
}

impl ExactColumns {
    pub(super) fn new(entry: &Arc<Entry>, table: TableId, written: TableWritten) -> Self {
        Self { table, entry: Arc::clone(entry), written, lane: Mutex::new(Some(None)) }
    }

    pub(super) fn release(&self) {
        *lock(&self.lane) = None;
    }

    pub(super) fn bytes(&self) -> usize {
        lock(&self.lane).as_ref().and_then(Option::as_ref).map_or(0, |lane| lane.held().total())
    }

    fn at<T>(&self, hour: Hour, columns: &[usize], keep: bool, read: impl FnOnce(&Lane) -> io::Result<T>) -> io::Result<Standing<T>> {
        match &self.written {
            TableWritten::Withheld(reason) => return Ok(Standing::Withheld(reason.clone())),
            TableWritten::Damaged(reason) => return Err(io::Error::other(reason.clone())),
            TableWritten::Unwritten => return Ok(Standing::NotRecorded),
            TableWritten::Written => {}
        }
        let pieces = self.entry.playhead().timeline().pieces();
        let Some(piece) = pieces.get(pieces.partition_point(|piece| piece.last < hour)).filter(|piece| piece.first <= hour) else { return Ok(Standing::NotRecorded) };
        let pages = self.entry.pages();
        let table = self.table.index();
        if let Some(reason) = pages.withheld_at(piece.run, table, hour) {
            return Ok(Standing::Withheld(reason.to_owned()));
        }
        let mut held = lock(&self.lane);
        let held = held.as_mut().ok_or_else(|| io::Error::other("the playback column was released"))?;
        let mut lane = held.take().filter(|lane| lane.run() == piece.run && lane.declared().0 == columns)
            .map_or_else(|| Lane::new(Arc::clone(pages), piece.run, table, columns, true), Ok)?;
        if lane.head() != Some(hour) {
            let first = pages.runs()[piece.run].first_hour;
            let block = |at| (i64::from(at) - i64::from(first)) / i64::from(BLOCK);
            if lane.head().is_some_and(|at| at < hour && block(at) == block(hour)) {
                let mut changes = Changes::default();
                lane.fetch_through(hour)?;
                if lane.step(hour, &mut changes)? != hour {
                    return Err(io::Error::other("the recorded columns did not reach the requested hour"));
                }
                lane.retire_into(&mut Vec::new());
                if keep {
                    changes.inserted().keys().unwrap_or_default().iter().try_for_each(|&key| self.table.check_key(key))?;
                }
            } else {
                lane.seek(hour)?;
                if keep {
                    lane.cells().rows().filter_map(|slot| lane.cells().key(slot)).try_for_each(|key| self.table.check_key(key))?;
                }
            }
        }
        let value = read(&lane);
        *held = keep.then_some(lane);
        value.map(Standing::Recorded)
    }

    pub(super) fn snapshot(&self, hour: Hour, fields: u64, keys: Option<&BTreeSet<u64>>, order: &mut Vec<(u128, u32)>) -> (SharedView, Option<HeadFailure>) {
        order.clear();
        let (rows, failure, sampled) = self.snapshot_with(hour, fields, |lane| {
            order_rows(lane, keys, order);
            cells_rows_of(self.table, &lane.view(), order, lane.format(), fields)
        });
        let width = self.entry.kinds()[self.table.index()].len();
        (held_fields(SharedView::from(rows).with_sampled_at(sampled), fields, width), failure)
    }

    fn snapshot_with(&self, hour: Hour, fields: u64, decode: impl FnOnce(&Lane) -> io::Result<TableRows>) -> (TableView, Option<HeadFailure>, Option<Hour>) {
        let width = self.entry.kinds()[self.table.index()].len();
        let columns: Vec<_> = (0..width).filter(|&column| column >= 64 || fields & (1 << column) != 0).collect();
        let capture = self.entry.capture.get().copied();
        let (view, failure, sampled) = match self.at(hour, &columns, false, |lane| {
            let (status, sample) = lane.status();
            let (view, failure) = view::make_rows(self.table, status, capture, |reason| reason_at(lane, reason), || decode(lane));
            Ok((view, failure, sample))
        }) {
            Ok(Standing::Recorded(answer)) => answer,
            Ok(Standing::Withheld(reason)) => (TableView::Unavailable { reason }, None, None),
            Ok(Standing::NotRecorded) => (view::make_rows(self.table, Status::NotRecorded, capture, |_| unreachable!(), || unreachable!()).0, None, None),
            Err(error) => {
                let (view, failure) = view::failed_rows(self.table, error);
                (view, failure, None)
            }
        };
        let sampled = sampled.filter(|_| matches!(view, TableView::Complete(_)));
        (view, failure, sampled)
    }

    pub(super) fn decoded(&self, hour: Hour, fields: u64, keys: Option<&BTreeSet<u64>>, order: &mut Vec<(u128, u32)>) -> (TableView, view::Made) {
        order.clear();
        let mut untold = 0;
        let (rows, failure, _) = self.snapshot_with(hour, fields, |lane| {
            order_rows(lane, keys, order);
            let keys: Vec<_> = order.iter().map(|&(key, _)| key).collect();
            let columns = lane.view();
            self.table.rows_from_wire(&keys, &mut |at, column| {
                let Some(&kind) = columns.kinds().get(column) else { return Ok(Field::NotRecorded) };
                if column < 64 && fields & (1 << column) == 0 {
                    return Ok(view::zero(kind));
                }
                let value = columns.value(column, order[at].1);
                match value {
                    Value::Missing if !matches!(kind, Kind::OptionalI32 | Kind::OptionalI64 | Kind::OptionalU32 | Kind::OptionalU64) => Ok(Field::NotRecorded),
                    Value::Text(id) => Ok(columns.string(id).map_or_else(|| {
                        untold += 1;
                        Field::NotRecorded
                    }, Field::Bytes)),
                    value => replay_frame::frame::value_field(kind, value, |id| columns.string(id), lane.format()),
                }
            })
        });
        (rows, view::Made { rows: order.len(), failure: failure.map(|failure| failure.kind), untold })
    }
}

fn order_rows(lane: &Lane, keys: Option<&BTreeSet<u64>>, order: &mut Vec<(u128, u32)>) {
    let add = |slot| {
        if let Some(key) = lane.cells().key(slot).filter(|&key| keys.is_none_or(|keys| keys.contains(&prefix(key)))) {
            order.push((key, slot));
        }
    };
    match lane.read_rows() {
        Some(slots) => slots.iter().copied().for_each(add),
        None => lane.cells().rows().for_each(add),
    }
    order.sort_unstable();
}

pub(super) struct ScriptValues {
    pub(super) columns: ExactColumns,
}

impl ScriptValues {
    pub(super) fn new(entry: &Arc<Entry>, written: TableWritten) -> Self {
        Self { columns: ExactColumns::new(entry, TABLE, written) }
    }
}

impl KeyedNumbers for ScriptValues {
    fn read(&self, hour: Hour, key: Option<u128>) -> KeyedNumber {
        match self.columns.at(hour, &[0], true, |lane| match lane.status().0 {
            Status::NotRecorded => Ok(KeyedNumber::NotRecorded),
            Status::Unavailable { reason } => Err(io::Error::other(reason_at(lane, reason))),
            Status::Truncated { missing } => Ok(KeyedNumber::Unavailable(format!("hour {hour} {TABLE:?} is truncated ({missing} missing rows)"))),
            Status::Complete => number(lane, key),
        }) {
            Ok(Standing::Recorded(answer)) => answer,
            Ok(Standing::NotRecorded) => KeyedNumber::NotRecorded,
            Ok(Standing::Withheld(reason)) => unavailable(hour, &reason),
            Err(error) => unavailable(hour, &error),
        }
    }
}

fn unavailable(hour: Hour, reason: &dyn std::fmt::Display) -> KeyedNumber {
    KeyedNumber::Unavailable(format!("hour {hour} {TABLE:?} is unavailable: {reason}"))
}

fn reason_at(lane: &Lane, reason: u32) -> String {
    lane.reason().map_or_else(|| format!("reason {reason}, whose text the recording did not name here"), |text| String::from_utf8_lossy(text).into_owned())
}

fn number(lane: &Lane, key: Option<u128>) -> io::Result<KeyedNumber> {
    let cells = lane.cells();
    let not_recorded = || io::Error::other("value_raw was not recorded");
    let Some(ColumnState::Number { unrecorded, .. }) = cells.column(0).filter(|_| lane.reads() == [0]) else {
        return Err(not_recorded());
    };
    if unrecorded.as_ref().is_some_and(|bits| bits.ones().any(|slot| cells.live().get(slot))) {
        return Err(not_recorded());
    }
    let Some(slot) = key.and_then(|key| cells.find(key)) else { return Ok(KeyedNumber::Value(None)) };
    let Value::Number(value) = cells.value(0, slot) else { return Err(not_recorded()) };
    Ok(KeyedNumber::Value(Some(value as i64)))
}
