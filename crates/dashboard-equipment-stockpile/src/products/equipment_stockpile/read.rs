//! The stockpile family's publication ([`super::emit`]) as its readers take it: a holder's
//! DAILY and LOGISTICS rows rebuilt from a keyframe and the deltas after it, their coverage,
//! the tables' read schedules, and each variant's names and class.

use super::emit as layout;
use dashboard_base::products::equipment::{RawVariant, StockVariantName, Texts, variant_names};
use dashboard_series::{
    Blob, Cell, Event, EventOrder, EventQuery, Evidence, Knownness, Observation, PageRead,
    Selection, Snapshot, Span,
    codec::{Binary, Decode},
    family::Directory,
    query::EventView,
};
use super::content::{Reading, publication};
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, atomic::AtomicBool},
};

/// Bytes one engine page answers at most.
pub(crate) const BUDGET: usize = 1 << 20;

pub(crate) fn malformed() -> io::Error {
    dashboard_core::invalid("A stockpile publication event is malformed")
}

fn selection(key: u64) -> Selection<'static> {
    Selection {
        key,
        membership: None,
    }
}

/// An event's cells, decoded from one read of its field table into buffers kept between events.
#[derive(Default)]
pub(crate) struct Cells {
    cells: Vec<Cell>,
    bytes: Vec<u8>,
}

impl Cells {
    pub(crate) fn read<P: PageRead>(
        &mut self,
        snapshot: &mut Snapshot<'_, P>,
        fields: Blob,
    ) -> io::Result<&[Cell]> {
        let len = usize::try_from(fields.len).map_err(|_| malformed())?;
        self.bytes.clear();
        self.bytes.resize(len, 0);
        snapshot.cell_bytes(fields, 0, &mut self.bytes)?;
        self.cells.clear();
        for chunk in self.bytes.chunks_exact(Cell::SIZE) {
            self.cells.push(Cell::decode(&mut Decode::new(chunk))?);
        }
        Ok(&self.cells)
    }
}

pub(crate) fn text<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    cell: Option<&Cell>,
) -> io::Result<Option<String>> {
    match cell.ok_or_else(malformed)? {
        Cell::Null => Ok(None),
        Cell::Text(blob) => {
            let mut bytes = vec![0; usize::try_from(blob.len).map_err(|_| malformed())?];
            snapshot.cell_bytes(*blob, 0, &mut bytes)?;
            Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
        }
        _ => Err(malformed()),
    }
}

fn amount(cell: &Cell) -> io::Result<Option<i64>> {
    match *cell {
        Cell::Null => Ok(None),
        Cell::Integer(value) => i64::try_from(value).map(Some).map_err(|_| malformed()),
        _ => Err(malformed()),
    }
}

fn flag(cell: &Cell) -> io::Result<bool> {
    match *cell {
        Cell::Boolean(value) => Ok(value),
        _ => Err(malformed()),
    }
}

pub(crate) fn unsigned(cell: Option<&Cell>) -> io::Result<u64> {
    match cell {
        Some(Cell::Unsigned(value)) => Ok(*value),
        _ => Err(malformed()),
    }
}

fn hour(cell: Option<&Cell>) -> io::Result<i32> {
    match cell {
        Some(Cell::Integer(value)) => i32::try_from(*value).map_err(|_| malformed()),
        _ => Err(malformed()),
    }
}

/// A variant's DAILY row at a read: its stock and its production lines. Only the halves
/// whose coverage holds at an hour say anything then.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Daily {
    pub stock: Option<i64>,
    pub stock_present: bool,
    pub lines_present: bool,
    pub units: Option<i64>,
    pub ic: Option<i64>,
    pub low: Option<i64>,
    pub high: Option<i64>,
}

/// An archetype token's LOGISTICS row: the balance, the stock the same update counted and
/// the available operand of that write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Logistics {
    pub balance: Option<i64>,
    pub stock: Option<i64>,
    pub available: Option<i64>,
}

/// A stream's row layout.
pub(crate) trait Stream {
    type Row: Copy + PartialEq;
    const CELLS: usize;
    /// A row's key and what it holds; none where the row is removed.
    fn row(cells: &[Cell]) -> io::Result<(u64, Option<Self::Row>)>;
}

pub(crate) struct DailyStream;
pub(crate) struct LogisticsStream;

impl Stream for DailyStream {
    type Row = Daily;
    const CELLS: usize = layout::DAILY_CELLS;
    fn row(cells: &[Cell]) -> io::Result<(u64, Option<Daily>)> {
        let row = Daily {
            stock: amount(&cells[1])?,
            stock_present: flag(&cells[2])?,
            lines_present: flag(&cells[3])?,
            units: amount(&cells[4])?,
            ic: amount(&cells[5])?,
            low: amount(&cells[6])?,
            high: amount(&cells[7])?,
        };
        Ok((
            unsigned(cells.first())?,
            (row.stock_present || row.lines_present).then_some(row),
        ))
    }
}

impl Stream for LogisticsStream {
    type Row = Logistics;
    const CELLS: usize = layout::LOGISTICS_CELLS;
    fn row(cells: &[Cell]) -> io::Result<(u64, Option<Logistics>)> {
        let row = Logistics {
            balance: amount(&cells[1])?,
            stock: amount(&cells[2])?,
            available: amount(&cells[3])?,
        };
        Ok((unsigned(cells.first())?, flag(&cells[4])?.then_some(row)))
    }
}

/// One event applied to a stream's rows, each row it changes handed to `changed` with what it
/// holds now (none where removed). A keyframe replaces every row.
pub(crate) fn apply<S: Stream>(
    kind: u64,
    cells: &[Cell],
    rows: &mut BTreeMap<u64, S::Row>,
    mut changed: impl FnMut(u64, Option<S::Row>),
) -> io::Result<()> {
    let header = match kind {
        layout::kind::KEYFRAME => 2,
        layout::kind::DELTA => 1,
        _ => return Err(malformed()),
    };
    let body = cells.get(header..).ok_or_else(malformed)?;
    if body.len() % S::CELLS != 0 {
        return Err(malformed());
    }
    if kind == layout::kind::DELTA {
        for cells in body.chunks_exact(S::CELLS) {
            let (key, row) = S::row(cells)?;
            match row {
                Some(row) => rows.insert(key, row),
                None => rows.remove(&key),
            };
            changed(key, row);
        }
        return Ok(());
    }
    let mut next = BTreeMap::new();
    for cells in body.chunks_exact(S::CELLS) {
        let (key, row) = S::row(cells)?;
        // A keyframe lists present rows only, each once.
        if next.insert(key, row.ok_or_else(malformed)?).is_some() {
            return Err(malformed());
        }
    }
    for &key in rows.keys() {
        if !next.contains_key(&key) {
            changed(key, None);
        }
    }
    for (&key, row) in &next {
        if rows.get(&key) != Some(row) {
            changed(key, Some(*row));
        }
    }
    *rows = next;
    Ok(())
}

/// `key`'s events over `span`, in native order.
pub(crate) fn events<P: PageRead>(
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
pub(crate) fn latest<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<u64>,
    hour: i32,
) -> io::Result<Option<Event>> {
    let Some(key) = key else {
        return Ok(None);
    };
    Ok(match snapshot.latest_event(selection(key), hour, None)? {
        Some(EventView::Family(event)) => Some(event),
        _ => None,
    })
}

/// A stream's rows at `hour` and the hour of the event they are from: its latest event by
/// then, applied over the keyframe a delta names. None before its first event.
pub(crate) fn rows_at<S: Stream, P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<u64>,
    hour: i32,
    cells: &mut Cells,
) -> io::Result<Option<(i32, BTreeMap<u64, S::Row>)>> {
    let Some(latest) = latest(snapshot, key, hour)? else {
        return Ok(None);
    };
    let mut rows = BTreeMap::new();
    let fields = cells.read(snapshot, latest.fields)?;
    let first = match latest.kind {
        layout::kind::KEYFRAME => {
            apply::<S>(latest.kind, fields, &mut rows, |_, _| ())?;
            return Ok(Some((latest.native_hour, rows)));
        }
        layout::kind::DELTA => self::hour(fields.first())?,
        _ => return Err(malformed()),
    };
    let mut keyed = false;
    events(
        snapshot,
        latest.key,
        Span::new(first, latest.native_hour)?,
        |snapshot, event| {
            if event.sequence > latest.sequence && event.native_hour == latest.native_hour {
                return Ok(());
            }
            // The chain starts at the keyframe the delta names.
            keyed |= event.kind == layout::kind::KEYFRAME && event.native_hour == first;
            if !keyed {
                return Err(malformed());
            }
            let fields = cells.read(snapshot, event.fields)?;
            apply::<S>(event.kind, fields, &mut rows, |_, _| ())
        },
    )?;
    Ok(Some((latest.native_hour, rows)))
}

pub(crate) fn known(evidence: Option<&Evidence>) -> bool {
    matches!(
        evidence,
        Some(Evidence {
            known: Knownness::Known,
            ..
        })
    )
}

/// Every evidence row of `key` over `span`, clipped to it: none over hours not walked.
pub(crate) fn evidence<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<u64>,
    span: Span,
) -> io::Result<Vec<Evidence>> {
    let mut rows = Vec::new();
    let Some(key) = key else {
        return Ok(rows);
    };
    let mut cursor = None;
    loop {
        let page = snapshot.evidence_page(key, span, cursor, BUDGET, |row| {
            rows.push(row);
            Ok(())
        })?;
        cursor = page.next;
        if cursor.is_none() {
            return Ok(rows);
        }
    }
}

/// The evidence `key` holds at `hour`.
pub(crate) fn evidence_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<u64>,
    hour: i32,
) -> io::Result<Option<Evidence>> {
    Ok(evidence(snapshot, key, Span::new(hour, hour)?)?.pop())
}

/// The last read by `hour` of the schedule run holding `at`: a run reads at its first hour and
/// every period after (a period of 0, a table read without a cadence, reads at its first hour
/// alone); none where no run holds `at`.
fn read_by<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    at: i32,
    hour: i32,
) -> io::Result<Option<i32>> {
    let Observation::Value { run, .. } = snapshot.value_at(key, at)? else {
        return Ok(None);
    };
    let period = i32::try_from(run.value)
        .ok()
        .filter(|period| *period >= 0)
        .ok_or_else(malformed)?;
    let first = run.span.first;
    Ok((first <= hour).then(|| match period {
        0 => first,
        period => first + (run.span.last.min(hour) - first) / period * period,
    }))
}

/// A table's latest read by `hour` from its schedule `key`, at least `anchor` (its stream's
/// latest event, itself a read): the run holding the hour, else the last run before it.
pub(crate) fn latest_read<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<u64>,
    anchor: i32,
    hour: i32,
) -> io::Result<i32> {
    let Some(key) = key else {
        return Ok(anchor);
    };
    if let Some(read) = read_by(snapshot, key, hour, hour)? {
        return Ok(read.max(anchor));
    }
    // Past an expected read that did not happen: the run before ends ahead of the hour.
    let mut latest = anchor;
    let accepted = snapshot.publication().accepted;
    if let Some(stop) = snapshot.next_change(selection(key), hour, false, accepted)? {
        for at in [stop.saturating_sub(1), stop] {
            if let Some(read) = read_by(snapshot, key, at, hour)? {
                latest = latest.max(read);
            }
        }
    }
    Ok(latest)
}

/// A variant's final equipment token: its latest CLASS in the publication, whose archetype
/// content resolves where the reply is read.
pub(crate) fn token<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    variant: u64,
    cells: &mut Cells,
) -> io::Result<Option<String>> {
    let key = directory.key(layout::FAMILY, layout::CLASSES, Some(variant));
    let Some(event) = latest(snapshot, key, i32::MAX)? else {
        return Ok(None);
    };
    let fields = cells.read(snapshot, event.fields)?;
    Ok(Some(text(snapshot, fields.first())?.ok_or_else(malformed)?))
}

/// Every logistics token's spelling, as first spelled, kept per publication.
pub(crate) fn tokens<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    branch: u64,
    reading: &Reading<'_>,
    cancelled: &AtomicBool,
) -> io::Result<Arc<BTreeMap<u64, String>>> {
    let published = publication(snapshot);
    reading.keep(
        "stockpile:tokens",
        branch,
        &(),
        &published,
        cancelled,
        || spellings(snapshot, directory),
    )
}

fn spellings<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
) -> io::Result<BTreeMap<u64, String>> {
    let mut cells = Cells::default();
    let cells = &mut cells;
    let mut tokens = BTreeMap::new();
    let Some(key) = directory.key(layout::FAMILY, layout::TOKENS, None) else {
        return Ok(tokens);
    };
    events(
        snapshot,
        key,
        Span::new(i32::MIN, i32::MAX)?,
        |snapshot, event| {
            let fields = cells.read(snapshot, event.fields)?;
            let id = unsigned(fields.first())?;
            let spelling = text(snapshot, fields.get(1))?.ok_or_else(malformed)?;
            tokens.entry(id).or_insert(spelling);
            Ok(())
        },
    )?;
    Ok(tokens)
}

/// The hours every variant's names stopped being known, ascending.
pub(crate) fn forgets<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    last: i32,
) -> io::Result<Vec<i32>> {
    let mut hours = Vec::new();
    if let Some(key) = directory.key(layout::FAMILY, layout::FORGETS, None) {
        events(snapshot, key, Span::new(i32::MIN, last)?, |_, event| {
            hours.push(event.native_hour);
            Ok(())
        })?;
    }
    Ok(hours)
}

/// The latest hour every variant's names stopped being known, by `hour`.
pub(crate) fn forget_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    hour: i32,
) -> io::Result<Option<i32>> {
    let key = directory.key(layout::FAMILY, layout::FORGETS, None);
    Ok(latest(snapshot, key, hour)?.map(|event| event.native_hour))
}

fn raw<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    event: &Event,
    cells: &mut Cells,
) -> io::Result<RawVariant> {
    let fields = cells.read(snapshot, event.fields)?;
    let json = text(snapshot, fields.first())?.ok_or_else(malformed)?;
    serde_json::from_str(&json).map_err(|_| malformed())
}

/// A naming entry: its operands and how content names them, or unknown from a FORGET.
pub(crate) fn name(
    first_hour: i32,
    raw: Option<&RawVariant>,
    texts: Option<&dyn Texts>,
) -> StockVariantName {
    match raw {
        Some(raw) => StockVariantName {
            first_hour,
            name: variant_names(raw, texts).0,
            known: true,
            design: raw.design.clone(),
        },
        None => StockVariantName {
            first_hour,
            name: None,
            known: false,
            design: None,
        },
    }
}

/// A variant's naming at `hour`: its latest NAME by then, unless every name was forgotten
/// after it (`forget`); none before its first NAME.
pub(crate) fn naming_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    variant: u64,
    hour: i32,
    forget: Option<i32>,
    cells: &mut Cells,
) -> io::Result<Option<(i32, Option<RawVariant>)>> {
    let key = directory.key(layout::FAMILY, layout::NAMES, Some(variant));
    let Some(event) = latest(snapshot, key, hour)? else {
        return Ok(None);
    };
    // A FORGET never shares an hour with a NAME; were it to, it would come first.
    if let Some(forget) = forget.filter(|forget| *forget > event.native_hour) {
        return Ok(Some((forget, None)));
    }
    Ok(Some((event.native_hour, Some(raw(snapshot, &event, cells)?))))
}

/// A variant's whole naming history, with every FORGET after its first NAME.
pub(crate) fn naming<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    variant: u64,
    forgets: &[i32],
    last: i32,
    cells: &mut Cells,
) -> io::Result<Vec<(i32, Option<RawVariant>)>> {
    let mut named = Vec::new();
    if let Some(key) = directory.key(layout::FAMILY, layout::NAMES, Some(variant)) {
        events(snapshot, key, Span::new(i32::MIN, last)?, |snapshot, event| {
            named.push((event.native_hour, Some(raw(snapshot, &event, cells)?)));
            Ok(())
        })?;
    }
    let Some(&(first, _)) = named.first() else {
        return Ok(named);
    };
    let mut history = Vec::with_capacity(named.len());
    let mut forgets = forgets[forgets.partition_point(|hour| *hour <= first)..].iter().peekable();
    for entry in named {
        while let Some(&&forget) = forgets.peek() {
            if forget > entry.0 {
                break;
            }
            history.push((forget, None));
            forgets.next();
        }
        history.push(entry);
    }
    history.extend(forgets.map(|&forget| (forget, None)));
    Ok(history)
}

/// The holder's read counts at its latest read by `hour` ([`super::emit`]'s order).
pub(crate) fn counts_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<u64>,
    hour: i32,
    cells: &mut Cells,
) -> io::Result<[u64; 11]> {
    let Some(event) = latest(snapshot, key, hour)? else {
        return Ok([0; 11]);
    };
    counts(snapshot, &event, cells)
}

pub(crate) fn counts<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    event: &Event,
    cells: &mut Cells,
) -> io::Result<[u64; 11]> {
    let fields = cells.read(snapshot, event.fields)?;
    if fields.len() != 11 {
        return Err(malformed());
    }
    let mut counts = [0; 11];
    for (count, cell) in counts.iter_mut().zip(fields) {
        *count = unsigned(Some(cell))?;
    }
    Ok(counts)
}
