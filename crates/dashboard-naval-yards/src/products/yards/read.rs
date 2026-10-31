//! The yard views' publications ([`super::layout`]) as their readers take them: an owner's
//! streams at an hour, each rebuilt from its latest keyframe and the deltas after it, the
//! family's evidence then, the dating table's last read, and each token's naming operand.

use super::layout::{self, kind};
use dashboard_series::{
    Blob, Cell, Event, EventOrder, EventQuery, Evidence, Observation, PageRead, Selection,
    Snapshot, Span,
    codec::{Binary, Decode},
    family::Directory,
    query::EventView,
};
use std::{collections::BTreeMap, io};

/// Bytes one engine page answers at most.
const BUDGET: usize = 1 << 20;

pub fn malformed() -> io::Error {
    dashboard_core::invalid("A yard publication event is malformed")
}

fn selection(key: u64) -> Selection<'static> {
    Selection {
        key,
        membership: None,
    }
}

/// A cell as a reader holds it: lists decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Null,
    Integer(i128),
    Unsigned(u64),
    Boolean(bool),
    Text(String),
    List(Vec<Value>),
}

impl Value {
    /// The value as an integer the views name it by: Integer, Unsigned or Boolean; none for Null.
    pub fn number(&self) -> Option<i128> {
        match *self {
            Self::Integer(value) => Some(value),
            Self::Unsigned(value) => Some(i128::from(value)),
            Self::Boolean(value) => Some(i128::from(value)),
            _ => None,
        }
    }
}

/// Decodes `cells`' encoded cells, reading the bytes of every list and text they hold.
fn values<P: PageRead>(snapshot: &mut Snapshot<'_, P>, bytes: &[u8]) -> io::Result<Vec<Value>> {
    let mut values = Vec::with_capacity(bytes.len() / Cell::SIZE);
    for chunk in bytes.chunks_exact(Cell::SIZE) {
        values.push(value(snapshot, Cell::decode(&mut Decode::new(chunk))?)?);
    }
    Ok(values)
}

fn blob<P: PageRead>(snapshot: &mut Snapshot<'_, P>, blob: Blob) -> io::Result<Vec<u8>> {
    let mut bytes = vec![0; usize::try_from(blob.len).map_err(|_| malformed())?];
    snapshot.cell_bytes(blob, 0, &mut bytes)?;
    Ok(bytes)
}

fn value<P: PageRead>(snapshot: &mut Snapshot<'_, P>, cell: Cell) -> io::Result<Value> {
    Ok(match cell {
        Cell::Null => Value::Null,
        Cell::Integer(value) => Value::Integer(value),
        Cell::Unsigned(value) => Value::Unsigned(value),
        Cell::Boolean(value) => Value::Boolean(value),
        Cell::Text(text) => Value::Text(String::from_utf8_lossy(&blob(snapshot, text)?).into_owned()),
        Cell::List(list) => {
            let bytes = blob(snapshot, list)?;
            Value::List(values(snapshot, &bytes)?)
        }
        Cell::Number(_) => return Err(malformed()),
    })
}

/// An event's cells.
fn fields<P: PageRead>(snapshot: &mut Snapshot<'_, P>, event: &Event) -> io::Result<Vec<Value>> {
    let bytes = blob(snapshot, event.fields)?;
    values(snapshot, &bytes)
}

/// A stream's rows: every cell after its key and whether it is held, by key.
pub type Rows = BTreeMap<u64, Vec<Value>>;

/// One event of a stream of `width`-cell rows applied to `rows`: a keyframe replaces them all, a
/// delta's rows each replace or remove theirs.
fn apply(event_kind: u64, cells: &[Value], width: usize, rows: &mut Rows) -> io::Result<()> {
    let body = match event_kind {
        kind::KEYFRAME => cells,
        kind::DELTA => cells.get(1..).ok_or_else(malformed)?,
        _ => return Err(malformed()),
    };
    if width < 2 || body.len() % width != 0 {
        return Err(malformed());
    }
    if event_kind == kind::KEYFRAME {
        rows.clear();
    }
    for row in body.chunks_exact(width) {
        let (Value::Unsigned(key), Value::Boolean(held)) = (&row[0], &row[1]) else {
            return Err(malformed());
        };
        if *held {
            rows.insert(*key, row[2..].to_vec());
        } else if event_kind == kind::DELTA {
            rows.remove(key);
        } else {
            // A keyframe lists held rows only.
            return Err(malformed());
        }
    }
    Ok(())
}

/// `key`'s events over `span`, in native order.
fn events<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    span: Span,
    mut sink: impl FnMut(&mut Snapshot<'_, P>, Event) -> io::Result<()>,
) -> io::Result<()> {
    let selection = [selection(key)];
    let query = EventQuery {
        selection: &selection,
        native: span,
        as_of: None,
        order: EventOrder::Native,
    };
    let mut found = Vec::new();
    let mut cursor = None;
    loop {
        let page = snapshot.event_page(query, cursor, BUDGET, |view| {
            if let EventView::Family(event) = view {
                found.push(event);
            }
            Ok(())
        })?;
        for event in found.drain(..) {
            sink(snapshot, event)?;
        }
        cursor = page.next;
        if cursor.is_none() {
            return Ok(());
        }
    }
}

/// `key`'s latest event at or before `hour`.
fn latest<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    hour: i32,
) -> io::Result<Option<Event>> {
    Ok(match snapshot.latest_event(selection(key), hour, None)? {
        Some(EventView::Family(event)) => Some(event),
        _ => None,
    })
}

/// A stream's rows at `hour` and the hour of the event they are from: its latest event by
/// then, applied over the keyframe a delta names. None before its first event.
pub fn rows_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<u64>,
    width: usize,
    hour: i32,
) -> io::Result<Option<(i32, Rows)>> {
    let Some(key) = key else {
        return Ok(None);
    };
    let Some(last) = latest(snapshot, key, hour)? else {
        return Ok(None);
    };
    let cells = fields(snapshot, &last)?;
    let mut rows = Rows::new();
    let first = match last.kind {
        kind::KEYFRAME => {
            apply(last.kind, &cells, width, &mut rows)?;
            return Ok(Some((last.native_hour, rows)));
        }
        kind::DELTA => match cells.first() {
            Some(Value::Integer(hour)) => i32::try_from(*hour).map_err(|_| malformed())?,
            _ => return Err(malformed()),
        },
        _ => return Err(malformed()),
    };
    let mut keyed = false;
    events(
        snapshot,
        key,
        Span::new(first, last.native_hour)?,
        |snapshot, event| {
            if event.sequence > last.sequence && event.native_hour == last.native_hour {
                return Ok(());
            }
            // The chain starts at the keyframe the delta names.
            keyed |= event.kind == kind::KEYFRAME && event.native_hour == first;
            if !keyed {
                return Err(malformed());
            }
            let cells = fields(snapshot, &event)?;
            apply(event.kind, &cells, width, &mut rows)
        },
    )?;
    Ok(Some((last.native_hour, rows)))
}

/// The evidence `key` holds at `hour`.
pub fn evidence_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<u64>,
    hour: i32,
) -> io::Result<Option<Evidence>> {
    let Some(key) = key else {
        return Ok(None);
    };
    let mut held = None;
    snapshot.evidence_page(key, Span::new(hour, hour)?, None, BUDGET, |row| {
        held = Some(row);
        Ok(())
    })?;
    Ok(held)
}

/// The dating table's last read by `hour`: within a READS run from `f` of period `p`, the latest
/// `f + k * p` by then; none outside a run.
pub fn last_read<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    hour: i32,
) -> io::Result<Option<i32>> {
    let Some(key) = directory.key(layout::FAMILY, layout::READS, None) else {
        return Ok(None);
    };
    let Observation::Value { run, .. } = snapshot.value_at(key, hour)? else {
        return Ok(None);
    };
    let period = i32::try_from(run.value)
        .ok()
        .filter(|period| *period > 0)
        .ok_or_else(malformed)?;
    let first = run.span.first;
    Ok((first <= hour).then(|| first + (run.span.last.min(hour) - first) / period * period))
}

/// Token `number`'s naming operand as the branch first numbered it: its TOKEN event's text.
pub fn token<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    number: u64,
) -> io::Result<Option<String>> {
    let Some(key) = directory.key(layout::FAMILY, layout::TOKEN, Some(number)) else {
        return Ok(None);
    };
    let Some(event) = latest(snapshot, key, snapshot.publication().accepted.last)? else {
        return Ok(None);
    };
    if event.kind != kind::TOKEN {
        return Err(malformed());
    }
    match fields(snapshot, &event)?.as_slice() {
        [Value::Text(text)] => Ok(Some(text.clone())),
        _ => Err(malformed()),
    }
}
