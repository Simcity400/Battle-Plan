//! A keyed family on a native recording whose publication is the walk's scan itself, its runs
//! on the series engine. A segment's operator ([`segment::Operator`] of [`Native`]) keeps, at
//! each hour its observer observes, the views of the observer's tables as the world holds them
//! then, copied where the hour changed them, with the hour's metrics and change facts; every
//! other hour is quiet. An observer with a [`Keeper`] keeps its own operands of each such hour
//! instead, from what the hour changed. The branch's [`Emit`] gives those hours in order to its
//! one [`Scan`], whose observer observes them as the walk's does, and writes the runs the walk's
//! accumulator would hold.
//!
//! The publication ([`SCHEMA`]): every name is an actor, spelled as the name and numbered in
//! the scan's order, whose [`NAME`] key holds the name's runs; [`FAMILY`]'s [`READ`] and
//! [`KNOWN`] keys hold the scan's own; every key is bound to one domain, known at each hour
//! walked; scalar [`OBSERVER`] holds the observer as of the last cut, as JSON.

use crate::{
    Batch, FIRST, KNOWN, Observer, READ, Scan,
    block::Operands,
    segment::{self, Reader, Segmented, Touched},
};
use dashboard_base::prepared::{
    engine, frame_changes::FrameChanges, kept::kept_changes, published, series::Run,
};
use dashboard_series::{
    Cell, Evidence, KeyHandle, Knownness, PageRead, RunMode, Snapshot, Span, Writer,
    carry::Carry,
    family::{Actors, Directory, Place},
};
use replay::{columnar_frames::view_heap_bytes, container::TableFields};
use replay_data::{
    format::FormatVersion,
    rows::{RowCodec, RowsPairVisit, TableId},
};
use replay_frame::frame::{
    DecodedFrame, Metrics, SharedView, TableView, ViewIdentity, not_selected,
};
use serde::{Deserialize, Serialize};
use std::{io, marker::PhantomData, sync::atomic::AtomicBool};

#[cfg(test)]
mod tests;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;

/// The families' keys are spelled under it.
pub const SCHEMA: u64 = 1;
/// The actor of the family's own keys, [`READ`] and [`KNOWN`], by measure.
pub const FAMILY: u64 = u64::MAX;
/// A name's measure: its runs.
pub const NAME: u64 = 0;
/// The family scalar holding the observer.
pub const OBSERVER: u64 = 0;
/// Every key's evidence: each hour walked is known.
const DOMAIN: u64 = 0;

fn invalid(what: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what)
}

/// The keyed family of `O`, read a segment at a time.
pub struct Native<O>(PhantomData<fn() -> O>);

impl<O: Observer> Segmented for Native<O> {
    const TABLES: &'static [TableId] = O::TABLES;
    const READ: TableId = O::READ;
    type Read = Kept<O>;

    fn fields() -> io::Result<Vec<TableFields>> {
        O::fields()
    }

    fn changed(changes: &FrameChanges) -> bool {
        O::changed(changes)
    }

    fn reader(format: FormatVersion) -> Box<dyn Reader<Self>> {
        Box::new(Keep::<O> {
            format,
            copies: Vec::new(),
            keeper: O::native_keeper(),
        })
    }
}

/// A segment's keeper of an observer's own operands ([`Observer::native_keeper`]), given the
/// hours it observes in order from the segment's first.
pub trait Keeper<O: Observer>: Send {
    /// The operands of an hour the observer observes, from `frame`, its tables as the world
    /// lends them, and what the hour changed of each; and the bytes they hold that the hours
    /// kept before do not.
    fn keep(
        &mut self,
        format: FormatVersion,
        frame: &DecodedFrame,
        touched: &Touched,
    ) -> io::Result<(Box<dyn Operands<O>>, usize)>;
}

/// An hour the observer observes, as a segment's operator kept it, with its change facts.
pub struct Kept<O: Observer> {
    held: Held<O>,
    changes: Box<FrameChanges>,
}

enum Held<O: Observer> {
    /// Its tables' views in [`Observer::TABLES`] order, whether the hour gave each whole, and
    /// its metrics.
    Views {
        views: Box<[SharedView]>,
        restated: Box<[bool]>,
        metrics: Box<Metrics>,
    },
    /// What the observer's keeper kept.
    Operands(Box<dyn Operands<O>>),
}

/// A segment's reader: each table's view as the world lent it last, and its own copy of it; or
/// the observer's keeper.
struct Keep<O: Observer> {
    format: FormatVersion,
    copies: Vec<(ViewIdentity, SharedView)>,
    keeper: Option<Box<dyn Keeper<O>>>,
}

impl<O: Observer> Reader<Native<O>> for Keep<O> {
    fn read(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        touched: &Touched,
    ) -> io::Result<(Kept<O>, usize)> {
        if let Some(keeper) = &mut self.keeper {
            let (operands, bytes) = keeper.keep(self.format, frame, touched)?;
            return Ok((
                Kept {
                    held: Held::Operands(operands),
                    changes: Box::new(kept_changes(changes)),
                },
                bytes + size_of::<FrameChanges>(),
            ));
        }
        let mut bytes = size_of::<Metrics>()
            + size_of::<FrameChanges>()
            + O::TABLES.len() * size_of::<SharedView>();
        let mut views = Vec::with_capacity(O::TABLES.len());
        for (at, table) in O::TABLES.iter().enumerate() {
            let lent = &frame.tables[table.index()];
            // The world's views are never kept: an hour that changed one keeps a copy.
            let copy = match self.copies.get(at) {
                Some((identity, copy)) if identity.is(lent) => copy.clone(),
                _ => {
                    let copy = SharedView::from((**lent).clone());
                    bytes += view_heap_bytes(&copy);
                    let held = (lent.identity(), copy.clone());
                    match self.copies.get_mut(at) {
                        Some(slot) => *slot = held,
                        None => self.copies.push(held),
                    }
                    copy
                }
            };
            views.push(copy.with_sampled_at(lent.sampled_at()));
        }
        Ok((
            Kept {
                held: Held::Views {
                    views: views.into_boxed_slice(),
                    restated: O::TABLES
                        .iter()
                        .map(|&table| touched.of(table).restated)
                        .collect(),
                    metrics: Box::new(frame.metrics.clone()),
                },
                changes: Box::new(kept_changes(changes)),
            },
            bytes,
        ))
    }
}

/// The scan's runs of one hour, as its batch was given them: key, hour, value, new observation.
#[derive(Default, Serialize, Deserialize)]
pub struct Calls {
    #[serde(skip)]
    calls: Vec<(u64, i32, Option<i128>, bool)>,
}

impl Batch for Calls {
    fn advance(&mut self, _: i32) -> io::Result<()> {
        Ok(())
    }

    fn observe(&mut self, key: u64, hour: i32, value: Option<i128>, starts: bool) {
        self.calls.push((key, hour, value, starts));
    }

    fn through(&mut self, _: i32) {}
}

/// A keyed family's emit on one branch.
pub struct Emit<O: Observer> {
    format: FormatVersion,
    /// The fields the observer reads of each of its tables.
    fields: Vec<u64>,
    scan: Scan<O, Calls>,
    /// The frame the observer reads: the views of the last hour kept.
    frame: DecodedFrame,
    actors: Actors,
    /// Each scan key's series key: READ, KNOWN, then each name's.
    keys: Vec<KeyHandle>,
    carry: Carry,
    /// The first hour walked the domain has not appended, and the last hour walked.
    first: Option<i32>,
    last: Option<i32>,
    /// Whether the observer changed since the last cut.
    stale: bool,
}

impl<O: Observer> Emit<O> {
    /// The emit of `branch` of a recording in `format`.
    pub fn new(format: FormatVersion, branch: u64) -> io::Result<Self> {
        let named = O::fields()?;
        let fields = O::TABLES
            .iter()
            .map(|&table| {
                named
                    .iter()
                    .find(|fields| fields.table == table)
                    .map_or(table.full_mask(), |fields| fields.fields)
            })
            .collect();
        Ok(Self {
            format,
            fields,
            scan: Scan::default(),
            frame: DecodedFrame {
                branch,
                hour: 0,
                tables: std::array::from_fn(|_| not_selected()),
                metrics: Metrics::default(),
            },
            actors: Actors::new(SCHEMA),
            keys: Vec::new(),
            carry: Carry::default(),
            first: None,
            last: None,
            stale: true,
        })
    }

    /// Binds its observer to the recording's content ([`Observer::bind`]).
    pub fn bind(
        &mut self,
        source: &dashboard_base::Source,
        content: Option<&dashboard_core::config::ContentOptions>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        self.scan.observer.bind(source, content, check)
    }

    /// One hour, after the last one given.
    pub fn hour(
        &mut self,
        writer: &mut Writer<'_>,
        hour: segment::Hour<Native<O>>,
    ) -> io::Result<()> {
        let at = hour.hour;
        if let Some(last) = self.last
            && last.checked_add(1) != Some(at)
        {
            self.carry.close(writer, last)?;
            self.known(writer, last)?;
        }
        match hour.read {
            None => self.scan.prepare_quiet(at, hour.dated)?,
            Some(Kept { held, mut changes }) => {
                let operands = match held {
                    Held::Views {
                        views,
                        restated,
                        metrics,
                    } => {
                        for (at, (&table, view)) in
                            O::TABLES.iter().zip(views.into_vec()).enumerate()
                        {
                            let index = table.index();
                            let held = &self.frame.tables[index];
                            // A segment's first hour gives every table whole: one as it was is
                            // unchanged.
                            if restated[at] && same(&view, held, self.fields[at]) {
                                changes.rows_changed[index] = false;
                                self.frame.tables[index] =
                                    held.clone().with_sampled_at(view.sampled_at());
                            } else {
                                self.frame.tables[index] = view;
                            }
                        }
                        self.frame.hour = at;
                        self.frame.metrics = *metrics;
                        None
                    }
                    // A keeper's segment observes its first hour, as the walk observes a block's,
                    // but where its keeper finds a table restated as the observer holds it.
                    Held::Operands(operands) => {
                        for &table in O::TABLES {
                            let index = table.index();
                            if changes.rows_changed[index]
                                && operands.restates(&self.scan.observer, table)
                            {
                                changes.rows_changed[index] = false;
                            }
                        }
                        Some(operands)
                    }
                };
                if !O::changed(&changes) && self.scan.continues_to(at) {
                    // Nothing it reads moved: the hour is quiet, refused hours staying refused.
                    self.scan
                        .prepare_quiet(at, changes.slot_read[O::READ.index()])?;
                } else {
                    match operands {
                        Some(operands) => {
                            self.scan.prepare_kept(at, &changes, operands.as_ref())?
                        }
                        None => self
                            .scan
                            .prepare_changed(self.format, &self.frame, &changes)?,
                    }
                    self.stale = true;
                }
            }
        }
        self.first.get_or_insert(at);
        self.last = Some(at);
        self.runs(writer)
    }

    /// Writes the runs the scan's last hour gave its batch.
    fn runs(&mut self, writer: &mut Writer<'_>) -> io::Result<()> {
        let mut calls = std::mem::take(&mut self.scan.batch_mut().calls);
        for &(key, hour, value, starts) in &calls {
            let handle = self.key(writer, key, hour)?;
            // A run breaks where its observation starts or the hour before held none.
            let break_before = starts || self.carry.held(handle).is_none();
            self.carry.value(writer, handle, hour, value, break_before)?;
        }
        calls.clear();
        self.scan.batch_mut().calls = calls;
        Ok(())
    }

    /// The series key of scan key `key`, every key before it made first, in order.
    fn key(&mut self, writer: &mut Writer<'_>, key: u64, hour: i32) -> io::Result<KeyHandle> {
        let at = usize::try_from(key).map_err(|_| invalid("A keyed scan key is out of range"))?;
        while self.keys.len() <= at {
            let next = self.keys.len() as u64;
            let place = if next < FIRST {
                Place {
                    actor: FAMILY,
                    measure: next,
                    dimension: None,
                }
            } else {
                let name = self
                    .scan
                    .names()
                    .get((next - FIRST) as usize)
                    .ok_or_else(|| invalid("A keyed scan wrote a name it has not numbered"))?;
                Place {
                    actor: self.actors.actor(writer, name.as_bytes(), hour)?,
                    measure: NAME,
                    dimension: None,
                }
            };
            let handle = self
                .actors
                .key(writer, place, (RunMode::Exact, 0), Some(DOMAIN))?;
            self.keys.push(handle);
        }
        Ok(self.keys[at])
    }

    /// The hours walked since the domain's last row, through `last`, as known.
    fn known(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        if let Some(first) = self.first.take() {
            writer.domain(
                DOMAIN,
                Evidence {
                    span: Span::new(first, last)?,
                    known: Knownness::Known,
                    supplying_read: None,
                    holder: None,
                    identity_first: None,
                },
            )?;
        }
        Ok(())
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next, and the observer where it changed.
    pub fn cut(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        self.carry.cut(writer, last)?;
        self.known(writer, last)?;
        if self.stale {
            let json = serde_json::to_vec(&self.scan.observer)?;
            let text = writer.blob(&mut &json[..], json.len() as u64)?;
            writer.catalogue(OBSERVER, Cell::Text(text))?;
            self.stale = false;
        }
        Ok(())
    }

    /// What it goes on with after a cut: the scan, then the open runs.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let saved = serde_json::to_vec(&(&self.scan, self.last))?;
        out.extend_from_slice(&(saved.len() as u64).to_le_bytes());
        out.extend_from_slice(&saved);
        self.carry.save(out)
    }

    /// The emit [`Self::save`] wrote, going on from the publication `directory` reads.
    pub fn load(
        format: FormatVersion,
        branch: u64,
        directory: &Directory,
        bytes: &[u8],
    ) -> io::Result<Self> {
        let (length, rest) = bytes
            .split_first_chunk::<8>()
            .ok_or_else(|| invalid("A saved keyed emit is truncated"))?;
        let length = usize::try_from(u64::from_le_bytes(*length))
            .ok()
            .filter(|&length| length <= rest.len())
            .ok_or_else(|| invalid("A saved keyed emit is truncated"))?;
        let (saved, carry) = rest.split_at(length);
        let (scan, last): (Scan<O, Calls>, Option<i32>) = serde_json::from_slice(saved)?;
        let mut emit = Self::new(format, branch)?;
        emit.keys = keys(directory, &scan)?;
        emit.actors = Actors::resume(SCHEMA, directory)?;
        emit.carry = Carry::load(carry)?;
        emit.scan = scan;
        emit.last = last;
        emit.stale = false;
        Ok(emit)
    }
}

/// Whether `view` holds what `held` holds in `fields`: the same status, and complete rows of
/// the same keys whose `fields` agree.
fn same(view: &TableView, held: &TableView, fields: u64) -> bool {
    match (view, held) {
        (TableView::Complete(rows), TableView::Complete(old)) => {
            rows.visit_pair(old, Agree(fields)).unwrap_or(false)
        }
        _ => view == held && !view.is_not_selected(),
    }
}

/// Whether two tables' rows agree in some fields, row for row.
struct Agree(u64);

impl RowsPairVisit for Agree {
    type Output = bool;

    fn rows<R: RowCodec>(self, rows: &[R], held: &[R]) -> bool {
        rows.len() == held.len()
            && rows
                .iter()
                .zip(held)
                .all(|(row, held)| row.key() == held.key() && row.differing_fields(held) & self.0 == 0)
    }
}

/// Each key `scan` numbered, as `directory` holds it, READ and KNOWN first: as far as it holds
/// them, for a scan whose last cut wrote none of the later ones.
fn keys<O: Observer, B: Batch>(
    directory: &Directory,
    scan: &Scan<O, B>,
) -> io::Result<Vec<KeyHandle>> {
    let mut keys = Vec::new();
    for measure in [READ, KNOWN] {
        match directory.key(FAMILY, measure, None) {
            Some(key) => keys.push(key),
            None => return Ok(keys),
        }
    }
    for name in scan.names() {
        let Some(key) = directory
            .actor(name.as_bytes())
            .and_then(|actor| directory.key(actor, NAME, None))
        else {
            break;
        };
        keys.push(key);
    }
    Ok(keys)
}

/// A keyed family's publication on a branch, as a reply reads it.
pub(crate) struct Opened {
    opened: published::Opened,
    /// Each scan key's series key, by scan key.
    keys: Vec<KeyHandle>,
}

/// `O`'s publication on `branch` and the scan it catalogues; None before its first.
pub(crate) fn open<O: Observer>(
    source: &dashboard_base::Source,
    content: Option<&dashboard_core::config::ContentOptions>,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Option<(Scan<O>, Opened)>> {
    let Some(opened) = published::open(source, content, O::NAME, SCHEMA, branch)? else {
        return Ok(None);
    };
    let (scan, keys) = opened.read(cancelled, |_, directory| catalogued(directory))?;
    Ok(Some((scan, Opened { opened, keys })))
}

/// The scan `directory` catalogues, its observer and names alone, and each of its keys.
fn catalogued<O: Observer>(directory: &Directory) -> io::Result<(Scan<O>, Vec<KeyHandle>)> {
    let observer = match directory.scalar(OBSERVER).and_then(|scalar| scalar.text()) {
        Some(json) => serde_json::from_slice(json)?,
        None => O::default(),
    };
    let names = directory
        .actors
        .iter()
        .map(|actor| {
            String::from_utf8(actor.name.to_vec()).map_err(|_| invalid("A keyed name is not text"))
        })
        .collect::<io::Result<_>>()?;
    let scan = Scan::<O>::catalogued(observer, names);
    let keys = keys(directory, &scan)?;
    Ok((scan, keys))
}

/// Each of scan keys `keys`' runs over `from..=to`, clipped to it, as the walk's pages hold
/// them; `handles` are the series keys of the scan's keys.
fn runs<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    handles: &[KeyHandle],
    keys: &[u64],
    from: i32,
    to: i32,
) -> io::Result<Vec<Vec<Run>>> {
    keys.iter()
        .map(
            |&key| match usize::try_from(key).ok().and_then(|at| handles.get(at)) {
                Some(&handle) => {
                    let mut runs: Vec<Run> = engine::runs(snapshot, handle, from, to)?
                        .into_iter()
                        .map(|run| Run {
                            first_hour: run.span.first,
                            last_hour: run.span.last,
                            value: run.value,
                            break_before: run.break_before,
                        })
                        .collect();
                    // A run the window clips starts no segment at the window's first hour.
                    if let Some(head) = runs.first_mut()
                        && head.break_before
                        && head.first_hour == from
                        && engine::run_at(snapshot, handle, from)?
                            .is_some_and(|(run, _)| run.span.first < from)
                    {
                        head.break_before = false;
                    }
                    Ok(runs)
                }
                None => Ok(Vec::new()),
            },
        )
        .collect()
}

/// Each of scan keys `keys`' changes over the ascending `hours`: the index of each hour whose
/// value differs from the hour before's (the first hour's when it holds one) and its value;
/// `handles` are the series keys of the scan's keys. Each key's runs over the hours are read once.
fn changes<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    handles: &[KeyHandle],
    keys: &[u64],
    hours: &[i32],
) -> io::Result<Vec<Vec<(usize, Option<i128>)>>> {
    if hours.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(invalid("requested hours must be distinct and ascending"));
    }
    let (Some(&first), Some(&last)) = (hours.first(), hours.last()) else {
        return Ok(vec![Vec::new(); keys.len()]);
    };
    keys.iter()
        .map(|&key| {
            let mut changes = Vec::new();
            let Some(&handle) = usize::try_from(key).ok().and_then(|at| handles.get(at)) else {
                return Ok(changes);
            };
            let runs = engine::runs(snapshot, handle, first, last)?;
            let mut runs = runs.iter().peekable();
            let mut held = None;
            for (at, &hour) in hours.iter().enumerate() {
                while runs.next_if(|run| run.span.last < hour).is_some() {}
                let now = runs
                    .peek()
                    .filter(|run| run.span.first <= hour)
                    .map(|run| run.value);
                if now != held {
                    changes.push((at, now));
                }
                held = now;
            }
            Ok(changes)
        })
        .collect()
}

impl Opened {
    /// Each of `keys`' runs over `from..=to`, clipped to it, as the walk's pages hold them.
    pub(crate) fn runs(
        &self,
        keys: &[u64],
        from: i32,
        to: i32,
        cancelled: &AtomicBool,
    ) -> io::Result<Vec<Vec<Run>>> {
        self.opened.read(cancelled, |snapshot, _| {
            runs(snapshot, &self.keys, keys, from, to)
        })
    }

    /// The values of `keys` at `hour`, None where a key holds none then.
    pub(crate) fn values_at(
        &self,
        keys: &[u64],
        hour: i32,
        cancelled: &AtomicBool,
    ) -> io::Result<Vec<Option<i128>>> {
        Ok(self
            .changes_at(keys, &[hour], cancelled)?
            .into_iter()
            .map(|changes| changes.first().and_then(|&(_, value)| value))
            .collect())
    }

    /// [`Self::values_at`] of `keys` at each of the ascending `hours`, as each key's changes:
    /// the index of each hour whose value differs from the hour before's (the first hour's when
    /// it holds one) and its value. Each key's runs over the hours are read once.
    pub(crate) fn changes_at(
        &self,
        keys: &[u64],
        hours: &[i32],
        cancelled: &AtomicBool,
    ) -> io::Result<Vec<Vec<(usize, Option<i128>)>>> {
        self.opened
            .read(cancelled, |snapshot, _| changes(snapshot, &self.keys, keys, hours))
    }
}

