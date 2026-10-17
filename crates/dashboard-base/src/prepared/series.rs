//! Shared batches with uncompressed key directories and compact, compressed per-key blobs.
use crate::metadata::check_cancel;
use crate::prepared_chart::{
    ChartBucket, ChartRun, ChartSummary, MarkCluster, MarkSummary, decode_bucket, encode_bucket,
    merge, signed, take, var, zig,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io::{self, Read, Seek, SeekFrom},
    sync::atomic::AtomicBool,
};
use ts_rs::TS;
const LEVELS: [i64; 4] = [16, 256, 4096, 65536];
const HEADER: u64 = 16;
const DIRECTORY: u64 = 64;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub first_hour: i32,
    pub last_hour: i32,
    pub value: i128,
    pub break_before: bool,
}
/// References to delayed facts (read after their native date), ordered by
/// (native, read, ordinal, key); ordinary dated rows remain solely in Events.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct LateIndex {
    rows: Vec<(i32, i32, u64, u64)>,
    #[serde(skip)]
    ordinals: BTreeMap<u64, Vec<(u64, u64)>>,
}
impl<'de> Deserialize<'de> for LateIndex {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let rows = Vec::<(i32, i32, u64, u64)>::deserialize(deserializer)?;
        let mut ordinals = BTreeMap::<u64, Vec<(u64, u64)>>::new();
        for &(_, _, key, ordinal) in &rows {
            ordinals.entry(key).or_default().push((ordinal, ordinal));
        }
        for spans in ordinals.values_mut() {
            spans.sort_unstable();
            spans.dedup_by(|next, previous| {
                if next.0 <= previous.1.saturating_add(1) {
                    previous.1 = previous.1.max(next.1);
                    true
                } else {
                    false
                }
            });
        }
        Ok(Self { rows, ordinals })
    }
}
impl LateIndex {
    /// Records a late fact's events row; facts read at or before their date are skipped.
    pub fn insert(&mut self, native: i32, read: i32, key: u64, ordinal: u64) {
        if read <= native {
            return;
        }
        let at = self.rows.partition_point(|&(date, hour, actor, row)| {
            (date, hour, row, actor) < (native, read, ordinal, key)
        });
        if self.rows.get(at) != Some(&(native, read, key, ordinal)) {
            self.rows.insert(at, (native, read, key, ordinal));
            let spans = self.ordinals.entry(key).or_default();
            let at = spans.partition_point(|span| span.1 < ordinal);
            if spans.get(at).is_some_and(|span| span.0 <= ordinal) {
                return;
            }
            let previous = at > 0 && spans[at - 1].1.checked_add(1) == Some(ordinal);
            let next = spans
                .get(at)
                .is_some_and(|span| ordinal.checked_add(1) == Some(span.0));
            match (previous, next) {
                (true, true) => {
                    spans[at - 1].1 = spans.remove(at).1;
                }
                (true, false) => spans[at - 1].1 = ordinal,
                (false, true) => spans[at].0 = ordinal,
                (false, false) => spans.insert(at, (ordinal, ordinal)),
            }
        }
    }
    /// First ordinary ordinal at or after `ordinal`, in logarithmic time.
    /// None when the late span extends through u64::MAX.
    pub fn next_ordinary(&self, key: u64, ordinal: u64) -> Option<u64> {
        let Some(spans) = self.ordinals.get(&key) else {
            return Some(ordinal);
        };
        let at = spans.partition_point(|span| span.0 <= ordinal);
        match at.checked_sub(1).map(|at| spans[at]) {
            Some((_, last)) if ordinal <= last => last.checked_add(1),
            _ => Some(ordinal),
        }
    }
    /// Last ordinary ordinal at or before `ordinal`, in logarithmic time.
    /// None when the late span starts at zero.
    pub fn previous_ordinary(&self, key: u64, ordinal: u64) -> Option<u64> {
        let Some(spans) = self.ordinals.get(&key) else {
            return Some(ordinal);
        };
        let at = spans.partition_point(|span| span.0 <= ordinal);
        match at.checked_sub(1).map(|at| spans[at]) {
            Some((first, last)) if ordinal <= last => first.checked_sub(1),
            _ => Some(ordinal),
        }
    }
    /// Late facts dated `native_from..=native_to` and read by `read_max`, as
    /// (native, read, key, ordinal).
    pub fn between(
        &self,
        native_from: i32,
        native_to: i32,
        read_max: i32,
    ) -> impl DoubleEndedIterator<Item = &(i32, i32, u64, u64)> {
        let first = self.rows.partition_point(|row| row.0 < native_from);
        let last = self.rows.partition_point(|row| row.0 <= native_to);
        self.rows[first.min(last)..last]
            .iter()
            .filter(move |row| row.1 <= read_max)
    }
}
impl Run {
    pub fn new(first_hour: i32, last_hour: i32, value: i128, break_before: bool) -> Self {
        Self {
            first_hour,
            last_hour,
            value,
            break_before,
        }
    }
    fn continues(self, next: Self) -> bool {
        self.last_hour.checked_add(1) == Some(next.first_hour)
            && self.value == next.value
            && !next.break_before
    }
}
/// Only a canonical open run, counters and partial summaries survive a commit.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(try_from = "SavedKeyState", into = "SavedKeyState")]
pub struct KeyState {
    pub last: Option<Run>,
    pub runs: u64,
    pub events: u64,
    partial: [Option<Partial>; 4],
}
/// A level's open summary bucket. Most keys hold one value through a bucket's
/// whole open span, so such a bucket is kept as that span and made again, bit
/// for bit, when the next flush merges into it.
#[derive(Clone, Copy, Debug)]
enum Partial {
    Span {
        aligned: i64,
        first: i32,
        last: i32,
        value: i128,
        first_break: bool,
    },
    Bucket {
        aligned: i64,
        bucket: ChartBucket,
    },
}
impl Partial {
    fn aligned(self) -> i64 {
        match self {
            Self::Span { aligned, .. } | Self::Bucket { aligned, .. } => aligned,
        }
    }
    fn bucket(self, decimals: u8) -> ChartBucket {
        match self {
            Self::Span {
                first,
                last,
                value,
                first_break,
                ..
            } => constant_bucket(value, first, last, first_break, decimals),
            Self::Bucket { bucket, .. } => bucket,
        }
    }
    /// The span form when it makes `bucket` again exactly (a series keeps its
    /// decimals), else the bucket itself.
    fn of(aligned: i64, bucket: ChartBucket, run: Option<Run>, decimals: u8) -> Self {
        #[cfg(test)]
        if WHOLE_BUCKETS.with(std::cell::Cell::get) {
            return Self::Bucket { aligned, bucket };
        }
        if let Some(run) = run {
            let span = Self::Span {
                aligned,
                first: bucket.first_hour,
                last: bucket.last_hour,
                value: run.value,
                first_break: bucket.first_break,
            };
            if same_bucket(&span.bucket(decimals), &bucket) {
                return span;
            }
        }
        Self::Bucket { aligned, bucket }
    }
    /// Whether this is the span a level's open bucket covers when it ends the
    /// open run `last`.
    fn ends(self, last: Run) -> bool {
        match (self, tail_span(self.aligned(), last)) {
            (
                Self::Span {
                    first,
                    last: end,
                    value,
                    first_break,
                    ..
                },
                Self::Span {
                    first: tail_first,
                    last: tail_end,
                    value: tail_value,
                    first_break: tail_break,
                    ..
                },
            ) => (first, end, value, first_break) == (tail_first, tail_end, tail_value, tail_break),
            _ => false,
        }
    }
}
#[cfg(test)]
thread_local! {
    /// Keep every open bucket whole, as before spans: the reference in tests.
    pub(crate) static WHOLE_BUCKETS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
/// Buckets equal field for field, values bit for bit.
fn same_bucket(a: &ChartBucket, b: &ChartBucket) -> bool {
    let bits = |bucket: &ChartBucket| {
        [
            bucket.first_value,
            bucket.last_value,
            bucket.min_value,
            bucket.max_value,
        ]
        .map(f64::to_bits)
    };
    a == b && bits(a) == bits(b)
}
/// The span a level's open bucket covers when it ends the open run `last`.
fn tail_span(aligned: i64, last: Run) -> Partial {
    let first = aligned.max(i64::from(last.first_hour)) as i32;
    Partial::Span {
        aligned,
        first,
        last: last.last_hour,
        value: last.value,
        first_break: last.break_before && first == last.first_hour,
    }
}
/// A key's saved state: runs as tuples, and a span ending the open run as its
/// alignment alone.
#[derive(Serialize, Deserialize)]
struct SavedKeyState {
    #[serde(rename = "l")]
    last: Option<(i32, i32, i128, bool)>,
    #[serde(rename = "r")]
    runs: u64,
    #[serde(rename = "e")]
    events: u64,
    #[serde(rename = "p")]
    partial: [Option<SavedPartial>; 4],
}
#[derive(Serialize, Deserialize)]
struct SavedPartial {
    #[serde(rename = "a")]
    aligned: i64,
    #[serde(rename = "s", default, skip_serializing_if = "Option::is_none")]
    span: Option<(i32, i32, i128, bool)>,
    #[serde(rename = "b", default, skip_serializing_if = "Option::is_none")]
    bucket: Option<ChartBucket>,
}
impl From<KeyState> for SavedKeyState {
    fn from(state: KeyState) -> Self {
        let partial = state.partial.map(|partial| {
            partial.map(|partial| match partial {
                Partial::Span {
                    aligned,
                    first,
                    last,
                    value,
                    first_break,
                } => SavedPartial {
                    aligned,
                    span: state.last.is_none_or(|run| !partial.ends(run)).then_some((
                        first,
                        last,
                        value,
                        first_break,
                    )),
                    bucket: None,
                },
                Partial::Bucket { aligned, bucket } => SavedPartial {
                    aligned,
                    span: None,
                    bucket: Some(bucket),
                },
            })
        });
        Self {
            last: state
                .last
                .map(|run| (run.first_hour, run.last_hour, run.value, run.break_before)),
            runs: state.runs,
            events: state.events,
            partial,
        }
    }
}
impl TryFrom<SavedKeyState> for KeyState {
    type Error = String;
    fn try_from(saved: SavedKeyState) -> Result<Self, String> {
        let last = saved
            .last
            .map(|(first_hour, last_hour, value, break_before)| Run {
                first_hour,
                last_hour,
                value,
                break_before,
            });
        let mut partial = [None; 4];
        for (slot, saved) in partial.iter_mut().zip(saved.partial) {
            let Some(saved) = saved else {
                continue;
            };
            *slot = Some(match (saved.span, saved.bucket, last) {
                (Some((first, end, value, first_break)), None, _) => Partial::Span {
                    aligned: saved.aligned,
                    first,
                    last: end,
                    value,
                    first_break,
                },
                (None, Some(bucket), _) => Partial::Bucket {
                    aligned: saved.aligned,
                    bucket,
                },
                // An alignment alone is the span ending the open run.
                (None, None, Some(run)) => tail_span(saved.aligned, run),
                _ => return Err("saved series bucket names no span or bucket".into()),
            });
        }
        Ok(Self {
            last,
            runs: saved.runs,
            events: saved.events,
            partial,
        })
    }
}
#[derive(Default)]
struct Rows {
    runs: Vec<Run>,
    events: Vec<(i32, Vec<u8>)>,
    state: KeyState,
}
#[derive(Default)]
pub struct Batch {
    first: Option<i32>,
    last: i32,
    keys: BTreeMap<u64, Rows>,
    bytes: usize,
}
/// A product's open keys and the current publication batch.
#[derive(Default, Serialize, Deserialize)]
pub struct Accumulator {
    states: BTreeMap<u64, KeyState>,
    frontier: Option<i32>,
    previous: Option<i32>,
    active: BTreeSet<u64>,
    #[serde(skip)]
    batch: Batch,
    /// The series number this accumulator's calls are recorded under, while
    /// a block's extract records them ([`with_recording`]).
    #[serde(skip)]
    recorded: Option<u8>,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    pub key_visits: usize,
}
impl Accumulator {
    /// Record this accumulator's calls as series `series` while a recording
    /// is active on the calling thread.
    pub fn record_as(&mut self, series: u8) {
        self.recorded = Some(series);
    }
    fn record(&self, call: impl FnOnce(u8) -> Call) {
        if let Some(series) = self.recorded {
            RECORDING.with(|recording| {
                if let Some(recording) = recording.borrow_mut().as_mut() {
                    recording.push(call(series));
                }
            });
        }
    }
    /// [`Self::advance`] at every hour `first..=last` in order, in constant time.
    pub fn advance_span(&mut self, first: i32, last: i32) -> io::Result<()> {
        self.advance(first)?;
        if last > first {
            if let Some(series) = self.recorded {
                RECORDING.with(|recording| {
                    if let Some(recording) = recording.borrow_mut().as_mut() {
                        recording.advance_through(series, last);
                    }
                });
            }
            self.previous = Some(last - 1);
            self.frontier = Some(last);
            self.batch.through(last);
        }
        Ok(())
    }
    /// [`Self::event`] with its row already serialized.
    pub fn event_bytes(&mut self, key: u64, hour: i32, row: Vec<u8>) -> io::Result<()> {
        self.batch.through(hour);
        self.batch
            .event_bytes(key, self.states.entry(key).or_default(), hour, row)
    }
    pub fn state(&self, key: u64) -> Option<&KeyState> {
        self.states.get(&key)
    }
    /// Keys holding an authenticated value, in key order.
    pub fn active(&self) -> impl Iterator<Item = u64> + '_ {
        self.active.iter().copied()
    }
    /// This batch's runs for `key`, not yet readable through a `Reader`.
    pub fn pending_runs(&self, key: u64) -> &[Run] {
        self.batch
            .keys
            .get(&key)
            .map_or(&[], |rows| rows.runs.as_slice())
    }
    pub fn through(&mut self, hour: i32) {
        self.record(|series| Call::Through { series, hour });
        self.batch.through(hour);
    }
    /// Authenticate one accepted frame before applying its changed keys. Repeated
    /// hours are idempotent; a gap closes held values at the old frontier.
    pub fn advance(&mut self, hour: i32) -> io::Result<()> {
        if let Some(frontier) = self.frontier {
            if hour < frontier {
                return Err(invalid("Series authenticated hours reversed"));
            }
            if hour == frontier {
                return Ok(());
            }
            if frontier.checked_add(1) != Some(hour) {
                self.seal();
                self.active.clear();
            }
        }
        if let Some(series) = self.recorded {
            RECORDING.with(|recording| {
                if let Some(recording) = recording.borrow_mut().as_mut() {
                    recording.advance(series, hour);
                }
            });
        }
        self.previous = self.frontier;
        self.frontier = Some(hour);
        self.batch.through(hour);
        Ok(())
    }
    /// Materialize held values only through accepted hours. Explicit spans and
    /// events never become held values. Checkpoints flush before serializing.
    pub fn seal(&mut self) {
        let Some(frontier) = self.frontier else {
            return;
        };
        for &key in &self.active {
            #[cfg(any(test, feature = "test-support"))]
            {
                self.key_visits += 1;
            }
            let state = self.states.get_mut(&key).expect("active series key");
            Self::extend(&mut self.batch, key, state, frontier);
        }
        if self.batch.first.is_some() {
            self.batch.last = self.batch.last.max(frontier);
        }
    }
    fn extend(batch: &mut Batch, key: u64, state: &mut KeyState, through: i32) {
        if let Some(last) = state.last.filter(|run| run.last_hour < through) {
            batch.push(
                key,
                state,
                Run::new(last.last_hour + 1, through, last.value, false),
            );
        }
    }
    pub fn push(&mut self, key: u64, run: Run) {
        self.push_boundary(key, run, false);
    }
    /// A supplying read can start a new canonical run without an identity break.
    pub fn push_boundary(&mut self, key: u64, run: Run, boundary: bool) {
        self.record(|series| Call::Push {
            series,
            key,
            run,
            boundary,
        });
        self.batch.through(run.first_hour);
        self.batch.through(run.last_hour);
        let state = self.states.entry(key).or_default();
        if self.active.remove(&key)
            && let Some(previous) = self.previous
        {
            Self::extend(
                &mut self.batch,
                key,
                state,
                previous.min(run.first_hour.saturating_sub(1)),
            );
        }
        self.batch.push_boundary(key, state, run, boundary);
    }
    /// At an authenticated frame, only changed/new/unknown keys need observing;
    /// other active values carry to publication. Without advance, inputs remain
    /// exact point observations, including absence and supplying-hour breaks.
    pub fn observe(&mut self, key: u64, hour: i32, value: Option<i128>, starts: bool) {
        self.observe_boundary(key, hour, value, starts, false);
    }
    /// A supplying read can force a new canonical run without an identity break.
    /// Authenticated observations remain active through later accepted hours.
    pub fn observe_boundary(
        &mut self,
        key: u64,
        hour: i32,
        value: Option<i128>,
        starts: bool,
        boundary: bool,
    ) {
        self.record(|series| Call::Observe {
            series,
            key,
            hour,
            value,
            starts,
            boundary,
        });
        self.batch.through(hour);
        #[cfg(any(test, feature = "test-support"))]
        {
            self.key_visits += 1;
        }
        let state = self.states.entry(key).or_default();
        let authenticated = self.frontier == Some(hour);
        if authenticated
            && self.active.contains(&key)
            && let Some(previous) = self.previous
        {
            Self::extend(&mut self.batch, key, state, previous);
        }
        if !authenticated {
            self.active.remove(&key);
        }
        if let Some(value) = value {
            let starts = starts
                || state
                    .last
                    .is_none_or(|run| run.last_hour.checked_add(1) != Some(hour));
            self.batch
                .push_boundary(key, state, Run::new(hour, hour, value, starts), boundary);
            if authenticated {
                self.active.insert(key);
            }
        } else {
            self.active.remove(&key);
        }
    }
    pub fn event(&mut self, key: u64, hour: i32, row: &impl Serialize) -> io::Result<()> {
        self.event_raw(key, hour, serde_json::to_vec(row)?)
    }
    /// [`Self::event`] with a row in its family's own byte form, read back
    /// through [`Reader::event_raw`] rather than as JSON.
    pub fn event_raw(&mut self, key: u64, hour: i32, row: Vec<u8>) -> io::Result<()> {
        self.record(|series| Call::Event {
            series,
            key,
            hour,
            row: row.clone().into_boxed_slice(),
        });
        self.event_bytes(key, hour, row)
    }
    pub fn bytes(&self) -> usize {
        self.batch.bytes()
    }
    pub fn flush<W: super::pages::Pages + ?Sized>(
        &mut self,
        writer: &mut W,
        section: &str,
        decimals: u8,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.seal();
        if self.batch.first.is_none() {
            return Ok(());
        }
        for (&key, state) in &self.states {
            self.batch.register(key, state);
        }
        self.states =
            std::mem::take(&mut self.batch).write(writer, section, decimals, cancelled)?;
        Ok(())
    }
}

/// The accumulator calls a block's extract made after the block's first hour,
/// in call order, so its fold makes them again on the carried accumulators
/// ([`Recording::replay`]). Every recorded accumulator of one family shares
/// one recording, numbered by series; a family's own calls that need its
/// carried state are recorded as their index ([`record_family`]). Unchanged
/// hours cost nothing: consecutive advances of a series are one span.
#[derive(Default)]
pub struct Recording {
    calls: Vec<Call>,
    /// Each series' open advance span, extended while no other call of that
    /// series follows it.
    spans: BTreeMap<u8, usize>,
}

enum Call {
    Advance {
        series: u8,
        first: i32,
        last: i32,
    },
    Through {
        series: u8,
        hour: i32,
    },
    Observe {
        series: u8,
        key: u64,
        hour: i32,
        value: Option<i128>,
        starts: bool,
        boundary: bool,
    },
    Push {
        series: u8,
        key: u64,
        run: Run,
        boundary: bool,
    },
    Event {
        series: u8,
        key: u64,
        hour: i32,
        row: Box<[u8]>,
    },
    Family(usize),
}

impl Call {
    fn series(&self) -> Option<u8> {
        match *self {
            Self::Advance { series, .. }
            | Self::Through { series, .. }
            | Self::Observe { series, .. }
            | Self::Push { series, .. }
            | Self::Event { series, .. } => Some(series),
            Self::Family(_) => None,
        }
    }
}

/// The carried accumulators a [`Recording`] is made again on.
pub trait ReplayTarget {
    /// The carried accumulator recorded as `series`.
    fn series(&mut self, series: u8) -> &mut Accumulator;
    /// The carried key of the extract's `key` in `series`.
    fn key(&self, series: u8, key: u64) -> u64;
    /// The family's own recorded call `index`, made on the carried state.
    fn family(&mut self, index: usize) -> io::Result<()>;
}

impl Recording {
    /// What the recording holds, about: every call, and each recorded
    /// event's row at its length.
    pub fn bytes(&self) -> usize {
        self.calls.capacity() * size_of::<Call>()
            + self
                .calls
                .iter()
                .map(|call| match call {
                    Call::Event { row, .. } => row.len(),
                    _ => 0,
                })
                .sum::<usize>()
            + self.spans.len() * 3 * size_of::<usize>()
    }
    /// Recorded calls, advance spans included.
    pub fn len(&self) -> usize {
        self.calls.len()
    }
    pub fn is_empty(&self) -> bool {
        self.calls.is_empty()
    }
    /// Extends the advance span `series` just recorded through `last`.
    fn advance_through(&mut self, series: u8, last: i32) {
        if let Some(&at) = self.spans.get(&series)
            && let Some(Call::Advance { last: through, .. }) = self.calls.get_mut(at)
        {
            *through = last;
        }
    }
    fn advance(&mut self, series: u8, hour: i32) {
        if let Some(&at) = self.spans.get(&series)
            && let Some(Call::Advance { last, .. }) = self.calls.get_mut(at)
        {
            if *last == hour {
                return;
            }
            if last.checked_add(1) == Some(hour) {
                *last = hour;
                return;
            }
        }
        self.spans.insert(series, self.calls.len());
        self.calls.push(Call::Advance {
            series,
            first: hour,
            last: hour,
        });
    }
    fn push(&mut self, call: Call) {
        match call.series() {
            Some(series) => {
                self.spans.remove(&series);
            }
            // A family call may touch any series, each at its current hour.
            None => self.spans.clear(),
        }
        self.calls.push(call);
    }
    /// Make every recorded call again, in order, on `target`'s accumulators.
    pub fn replay(self, target: &mut impl ReplayTarget) -> io::Result<()> {
        for call in self.calls {
            match call {
                Call::Advance {
                    series,
                    first,
                    last,
                } => target.series(series).advance_span(first, last)?,
                Call::Through { series, hour } => target.series(series).through(hour),
                Call::Observe {
                    series,
                    key,
                    hour,
                    value,
                    starts,
                    boundary,
                } => {
                    let key = target.key(series, key);
                    target
                        .series(series)
                        .observe_boundary(key, hour, value, starts, boundary);
                }
                Call::Push {
                    series,
                    key,
                    run,
                    boundary,
                } => {
                    let key = target.key(series, key);
                    target.series(series).push_boundary(key, run, boundary);
                }
                Call::Event {
                    series,
                    key,
                    hour,
                    row,
                } => {
                    let key = target.key(series, key);
                    target
                        .series(series)
                        .event_bytes(key, hour, row.into_vec())?;
                }
                Call::Family(index) => target.family(index)?,
            }
        }
        Ok(())
    }
}

thread_local! {
    static RECORDING: std::cell::RefCell<Option<Recording>> =
        const { std::cell::RefCell::new(None) };
}

/// Run `run` with `recording` receiving the calls of every accumulator marked
/// [`Accumulator::record_as`] on this thread.
pub fn with_recording<T>(recording: &mut Recording, run: impl FnOnce() -> T) -> T {
    struct Restore<'a>(&'a mut Recording, Option<Recording>);
    impl Drop for Restore<'_> {
        fn drop(&mut self) {
            let outer = self.1.take();
            *self.0 = RECORDING
                .with(|recording| recording.replace(outer))
                .unwrap_or_default();
        }
    }
    let taken = std::mem::take(recording);
    let outer = RECORDING.with(|current| current.replace(Some(taken)));
    let _restore = Restore(recording, outer);
    run()
}

/// Run `run` without recording: a family's own call records itself
/// ([`record_family`]) and makes its accumulator calls on the carried state.
pub fn unrecorded<T>(run: impl FnOnce() -> T) -> T {
    struct Restore(Option<Recording>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let recording = self.0.take();
            RECORDING.with(|current| *current.borrow_mut() = recording);
        }
    }
    let _restore = Restore(RECORDING.with(|current| current.borrow_mut().take()));
    run()
}

/// Record a family's own call `index` when a recording is active here.
pub fn record_family(index: usize) -> bool {
    RECORDING.with(|recording| match recording.borrow_mut().as_mut() {
        Some(recording) => {
            recording.push(Call::Family(index));
            true
        }
        None => false,
    })
}

/// A first-seen key catalogue's names in key order (key `k` is the `k`th
/// name first seen).
pub fn catalogue_names(keys: &BTreeMap<String, u64>) -> io::Result<Vec<String>> {
    let mut names = vec![None; keys.len()];
    for (name, &key) in keys {
        let slot = usize::try_from(key)
            .ok()
            .and_then(|key| names.get_mut(key))
            .ok_or_else(|| invalid("Series key catalogue is not numbered first seen"))?;
        *slot = Some(name.clone());
    }
    names
        .into_iter()
        .map(|name| name.ok_or_else(|| invalid("Series key catalogue is not numbered first seen")))
        .collect()
}

/// Number `names`, in the order first seen, into a carried first-seen
/// catalogue: a known name keeps its key and a new one takes the next, as
/// the calls that first saw them would have.
pub fn number_names(keys: &mut BTreeMap<String, u64>, names: &[String]) -> Vec<u64> {
    names
        .iter()
        .map(|name| {
            let next = keys.len() as u64;
            *keys.entry(name.clone()).or_insert(next)
        })
        .collect()
}

/// Tag-to-key lookups and the metadata needed by a family's own reply builder.
pub type Catalogue<T> = (BTreeMap<String, u64>, T);

/// Scalar sections address each tag's separate measures with the same opaque key.
pub fn field_key(tag: &str, measure: &str) -> String {
    serde_json::to_string(&(tag, measure)).expect("string keys")
}

fn catalogue_section(family: &str, branch: u64) -> String {
    format!("{family}-{branch}-catalogue")
}
fn parse_catalogue<T: DeserializeOwned>(text: Option<Vec<u8>>) -> io::Result<Option<Catalogue<T>>> {
    text.map(|text| serde_json::from_slice(&text).map_err(io::Error::from))
        .transpose()
}
/// All families publish the same catalogue shape, the JSON text of
/// `(keys, metadata)`. A flush writes only what changed in that text since
/// the last flush (a base range, then patches: `series/catalogue.rs`).
pub fn write_catalogue<W: super::pages::Pages + ?Sized>(
    writer: &mut W,
    family: &str,
    branch: u64,
    keys: &BTreeMap<String, u64>,
    metadata: &impl Serialize,
) -> io::Result<()> {
    catalogue::write(
        writer,
        &catalogue_section(family, branch),
        &serde_json::to_vec(&(keys, metadata))?,
    )
}
/// The prepared prefix of `section` a query waiting for `coverage` reads.
/// When the section holds batches committed beyond a coverage that is not
/// complete (a family prepared on its own ahead of another its replies read
/// with), the coverage ends at the last batch committed by its last hour, or
/// is None when none was; the query reads the section only through that hour
/// ([`Reader::through_hour`]) and the hours after it as an unprepared tail.
/// Otherwise the coverage is unchanged.
pub fn committed_coverage(
    store: &store::Store,
    key: &store::SourceKey,
    section: String,
    coverage: Option<super::coverage::Coverage>,
    cancelled: &AtomicBool,
) -> io::Result<Option<super::coverage::Coverage>> {
    let Some(partial) = coverage.as_ref().filter(|coverage| !coverage.complete) else {
        return Ok(coverage);
    };
    let mut reader = Reader::new(store, key, section, cancelled)?;
    let all = reader.count;
    let reached = reader.through_hour(partial.last_hour)?;
    if reader.count == all {
        return Ok(coverage);
    }
    Ok(reached.map(|last_hour| super::coverage::Coverage {
        last_hour,
        ..partial.clone()
    }))
}
pub fn read_catalogue<T: DeserializeOwned>(
    store: &store::Store,
    key: &store::SourceKey,
    family: &str,
    branch: u64,
) -> io::Result<Option<Catalogue<T>>> {
    let Some(session) = store.session(key)? else {
        return Ok(None);
    };
    let section = catalogue_section(family, branch);
    parse_catalogue(catalogue::read(|range| session.read(&section, range))?)
}
/// The catalogue as `saved` published it when that is an earlier key whose
/// prefix this preparation continues, else as `key` published it.
pub fn read_saved_catalogue<T: DeserializeOwned>(
    store: &store::Store,
    key: &store::SourceKey,
    saved: Option<&store::SourceKey>,
    family: &str,
    branch: u64,
) -> io::Result<Option<Catalogue<T>>> {
    match saved {
        Some(previous) if previous != key => {
            let section = catalogue_section(family, branch);
            parse_catalogue(catalogue::read(|range| {
                store.read_prefix(previous, key, &section, range)
            })?)
        }
        _ => read_catalogue(store, key, family, branch),
    }
}
/// The identities of the ranges a published catalogue reads, which change
/// whenever its text does; None when it is not published.
pub fn catalogue_ranges(
    store: &store::Store,
    key: &store::SourceKey,
    family: &str,
    branch: u64,
) -> io::Result<Option<Vec<store::RangeId>>> {
    let Some(session) = store.session(key)? else {
        return Ok(None);
    };
    let section = catalogue_section(family, branch);
    catalogue::stamps(|range| session.read(&section, range))
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, TS)]
pub enum MeasureKey {
    ArrowLeft,
    ArrowRight,
    Home,
    End,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Navigation {
    pub from: Option<f64>,
    pub first_hour: f64,
    pub last_hour: f64,
    pub key: MeasureKey,
}
impl Navigation {
    pub fn origin(&self) -> io::Result<f64> {
        range(Some(self.first_hour), Some(self.last_hour))?;
        if self.from.is_some_and(|hour| !hour.is_finite()) {
            return Err(invalid("invalid navigation range"));
        }
        Ok(match self.key {
            MeasureKey::Home => self.first_hour - 1.0,
            MeasureKey::End => self.last_hour + 1.0,
            MeasureKey::ArrowRight => self.from.unwrap_or(self.first_hour - 1.0),
            MeasureKey::ArrowLeft => self.from.unwrap_or(self.last_hour + 1.0),
        })
    }
}
/// A key and its optional membership spans; an empty slice excludes the key.
pub type NavigationKey<'a> = (u64, Option<&'a [crate::products::country_groups::HourSpan]>);
/// One keyboard range walk, including clipped left-edge stops and the reached-hour fallback.
pub fn navigate(
    readers: &mut [(&mut Reader<'_>, Vec<NavigationKey<'_>>)],
    navigation: &Navigation,
) -> io::Result<Option<f64>> {
    let origin = navigation.origin()?;
    let (first, last) = (navigation.first_hour, navigation.last_hour);
    let forward = matches!(navigation.key, MeasureKey::Home | MeasureKey::ArrowRight);
    let mut result: Option<f64> = None;
    for (reader, keys) in readers {
        for &(key, within) in keys.iter() {
            let spans = within.map_or_else(
                || vec![(first, last)],
                |spans| {
                    spans
                        .iter()
                        .map(|span| {
                            (
                                f64::from(span.first_hour).max(first),
                                f64::from(span.last_hour).min(last),
                            )
                        })
                        .collect()
                },
            );
            for (first, last) in spans.into_iter().filter(|(first, last)| first <= last) {
                let at = if forward {
                    origin.max(first - 1.0)
                } else {
                    origin.min(last + 1.0)
                };
                let candidate = reader
                    .next_change(key, at, forward)?
                    .map(f64::from)
                    .filter(|hour| *hour >= first && *hour <= last);
                let carried = reader.next_change(key, first, false)?.is_some();
                let edge = (carried && (forward && origin < first || !forward && origin > first))
                    .then_some(first);
                for hour in [candidate, edge].into_iter().flatten() {
                    result = Some(result.map_or(hour, |previous| {
                        if forward {
                            previous.min(hour)
                        } else {
                            previous.max(hour)
                        }
                    }));
                }
            }
        }
    }
    Ok(result.or_else(|| navigation.from.map(|hour| hour.clamp(first, last))))
}
impl Batch {
    pub fn through(&mut self, hour: i32) {
        self.first.get_or_insert(hour);
        self.last = hour;
    }
    pub fn bytes(&self) -> usize {
        self.bytes
    }
    /// Register every known key at publication, including keys absent from this batch.
    pub fn register(&mut self, key: u64, state: &KeyState) {
        self.keys.entry(key).or_default().state = state.clone();
    }
    pub fn push(&mut self, key: u64, state: &mut KeyState, run: Run) {
        self.push_boundary(key, state, run, false);
    }
    /// Keep supplying-read boundaries even when their amounts are equal.
    pub fn push_boundary(&mut self, key: u64, state: &mut KeyState, run: Run, boundary: bool) {
        let canonical = if let Some(previous) = state
            .last
            .filter(|previous| !boundary && previous.continues(run))
        {
            Run {
                last_hour: run.last_hour,
                ..previous
            }
        } else {
            state.runs += 1;
            run
        };
        let rows = self.keys.entry(key).or_insert_with(|| Rows {
            state: state.clone(),
            ..Default::default()
        });
        if let Some(last) = rows
            .runs
            .last_mut()
            .filter(|last| !boundary && last.continues(run))
        {
            last.last_hour = run.last_hour;
        } else {
            rows.runs.push(run);
            self.bytes += std::mem::size_of::<Run>() * 2;
        }
        state.last = Some(canonical);
        rows.state.last = state.last;
        rows.state.runs = state.runs;
    }
    pub fn event(
        &mut self,
        key: u64,
        state: &mut KeyState,
        hour: i32,
        row: &impl Serialize,
    ) -> io::Result<()> {
        self.event_bytes(key, state, hour, serde_json::to_vec(row)?)
    }
    /// [`Self::event`] with its row already serialized.
    pub fn event_bytes(
        &mut self,
        key: u64,
        state: &mut KeyState,
        hour: i32,
        bytes: Vec<u8>,
    ) -> io::Result<()> {
        self.bytes += bytes.len() * 2 + 32;
        state.events += 1;
        let rows = self.keys.entry(key).or_insert_with(|| Rows {
            state: state.clone(),
            ..Default::default()
        });
        if rows
            .events
            .last()
            .is_some_and(|(previous, _)| *previous > hour)
        {
            return Err(invalid("event hours reversed"));
        }
        rows.events.push((hour, bytes));
        rows.state.events = state.events;
        Ok(())
    }
    pub fn write<W: super::pages::Pages + ?Sized>(
        self,
        writer: &mut W,
        section: &str,
        decimals: u8,
        cancelled: &AtomicBool,
    ) -> io::Result<BTreeMap<u64, KeyState>> {
        let Some(first) = self.first else {
            return Ok(BTreeMap::new());
        };
        let mut bytes = vec![0; (HEADER + DIRECTORY * self.keys.len() as u64) as usize];
        put(&mut bytes, 0, &first.to_le_bytes());
        put(&mut bytes, 4, &self.last.to_le_bytes());
        put(&mut bytes, 8, &(self.keys.len() as u64).to_le_bytes());
        let mut states = BTreeMap::new();
        let mut blobs = HashMap::new();
        for (index, (key, mut rows)) in self.keys.into_iter().enumerate() {
            check_cancel(cancelled)?;
            let at = HEADER as usize + index * DIRECTORY as usize;
            put(&mut bytes, at, &key.to_le_bytes());
            put(&mut bytes, at + 24, &(rows.runs.len() as u64).to_le_bytes());
            put(&mut bytes, at + 32, &rows.state.runs.to_le_bytes());
            put(
                &mut bytes,
                at + 40,
                &(rows.state.events - rows.events.len() as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                at + 48,
                &(rows.events.len() as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                at + 56,
                &rows
                    .state
                    .last
                    .map_or(first, |run| run.last_hour)
                    .to_le_bytes(),
            );
            let mut raw = Vec::new();
            let (mut hour, mut value) = (0_i128, 0_i128);
            for run in &rows.runs {
                var(&mut raw, zig(i128::from(run.first_hour) - hour));
                var(
                    &mut raw,
                    (i64::from(run.last_hour) - i64::from(run.first_hour)) as u128,
                );
                var(&mut raw, zig(run.value.wrapping_sub(value)));
                var(&mut raw, u128::from(run.break_before));
                hour = i128::from(run.last_hour);
                value = run.value;
            }
            hour = 0;
            for (date, row) in &rows.events {
                var(&mut raw, zig(i128::from(*date) - hour));
                var(&mut raw, row.len() as u128);
                raw.extend_from_slice(row);
                hour = i128::from(*date);
            }
            for (level, size) in LEVELS.into_iter().enumerate() {
                let mut buckets = BTreeMap::new();
                for &run in &rows.runs {
                    let mut hour = i64::from(run.first_hour);
                    while hour <= i64::from(run.last_hour) {
                        check_cancel(cancelled)?;
                        let aligned = hour.div_euclid(size) * size;
                        let end = (aligned + size - 1).min(i64::from(run.last_hour));
                        let bucket = run_bucket(run, hour as i32, end as i32, decimals);
                        let previous = buckets.remove(&aligned).flatten();
                        buckets.insert(
                            aligned,
                            Some(previous.map_or(bucket, |previous| {
                                merge(Some(previous), Some(bucket)).unwrap()
                            })),
                        );
                        hour = end + 1;
                    }
                }
                if let Some(partial) = rows.state.partial[level] {
                    buckets.entry(partial.aligned()).or_insert(None);
                }
                var(&mut raw, buckets.len() as u128);
                let mut pending = rows.state.partial[level]
                    .map(|partial| (partial.aligned(), partial.bucket(decimals)));
                let mut previous = 0_i64;
                for (aligned, mut bucket) in buckets {
                    if let Some(bucket) = &mut bucket {
                        bucket.has_break |= i64::from(bucket.first_hour)
                            > aligned.max(i64::from(first))
                            || i64::from(bucket.last_hour)
                                < (aligned + size - 1).min(i64::from(self.last));
                    }
                    var(&mut raw, zig(i128::from(aligned - previous)));
                    previous = aligned;
                    encode_bucket(&mut raw, bucket, aligned);
                    let mut prefix = pending
                        .filter(|(previous, _)| *previous == aligned)
                        .map_or_else(
                            || bucket.unwrap(),
                            |(_, value)| merge(Some(value), bucket).unwrap(),
                        );
                    prefix.has_break |= i64::from(prefix.last_hour)
                        < (aligned + size - 1).min(i64::from(self.last));
                    encode_bucket(&mut raw, Some(prefix), aligned);
                    pending = Some((aligned, prefix));
                }
                let tail = i64::from(self.last).div_euclid(size) * size;
                let last = rows.state.last;
                rows.state.partial[level] = pending
                    .filter(|(aligned, _)| {
                        *aligned == tail && i64::from(self.last) < tail + size - 1
                    })
                    .map(|(aligned, bucket)| Partial::of(aligned, bucket, last, decimals));
                let mut counts = BTreeMap::<i64, u64>::new();
                for (hour, _) in &rows.events {
                    *counts
                        .entry(i64::from(*hour).div_euclid(size) * size)
                        .or_default() += 1;
                }
                var(&mut raw, counts.len() as u128);
                previous = 0;
                for (aligned, count) in counts {
                    var(&mut raw, zig(i128::from(aligned - previous)));
                    var(&mut raw, u128::from(count));
                    previous = aligned;
                }
            }
            blob(&mut bytes, at + 8, raw, &mut blobs)?;
            states.insert(key, rows.state);
        }
        check_cancel(cancelled)?;
        writer.write(section, bytes.as_slice())?;
        Ok(states)
    }
}
fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
    bytes[at..at + value.len()].copy_from_slice(value);
}
/// Compresses one prepared payload as a complete zstd frame. Each thread keeps
/// one context per level, and the known input size sizes its tables, so many
/// small payloads (one per key) don't each allocate a full streaming window.
pub fn compress(raw: &[u8], level: i32) -> io::Result<Vec<u8>> {
    thread_local! {
        static COMPRESSORS: std::cell::RefCell<BTreeMap<i32, zstd::bulk::Compressor<'static>>> =
            const { std::cell::RefCell::new(BTreeMap::new()) };
    }
    COMPRESSORS.with(|compressors| {
        let mut compressors = compressors.borrow_mut();
        let compressor = match compressors.entry(level) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(zstd::bulk::Compressor::new(level)?)
            }
        };
        compressor.compress(raw)
    })
}
/// Decompresses one prepared payload. A single frame that records its content
/// size reuses this thread's context and allocates its output once; anything
/// else (several frames, an unrecorded size) takes the streaming decoder.
pub fn decompress(bytes: &[u8]) -> io::Result<Vec<u8>> {
    thread_local! {
        static DECOMPRESSOR: std::cell::RefCell<Option<zstd::bulk::Decompressor<'static>>> =
            const { std::cell::RefCell::new(None) };
    }
    let single = zstd::zstd_safe::find_frame_compressed_size(bytes).ok() == Some(bytes.len());
    let size = zstd::zstd_safe::get_frame_content_size(bytes)
        .ok()
        .flatten();
    let Some(size) = size
        .filter(|_| single)
        .and_then(|size| usize::try_from(size).ok())
    else {
        return zstd::stream::decode_all(bytes);
    };
    DECOMPRESSOR.with(|decompressor| {
        let mut decompressor = decompressor.borrow_mut();
        if decompressor.is_none() {
            *decompressor = Some(zstd::bulk::Decompressor::new()?);
        }
        decompressor
            .as_mut()
            .expect("decompressor created above")
            .decompress(bytes, size)
    })
}
/// Equal per-key payloads share compression work, while each keeps its own
/// bytes and offset in the batch. The full collision-resistant BLAKE3 digest
/// and raw length identify a payload, following production endpoint identities,
/// so reuse needs no retained raw bytes for a second equality check. The cache
/// refers to encoded bytes already in the batch and retains no raw buffers.
fn blob(
    bytes: &mut Vec<u8>,
    at: usize,
    raw: Vec<u8>,
    blobs: &mut HashMap<([u8; 32], usize), (usize, usize)>,
) -> io::Result<()> {
    let offset = bytes.len();
    let identity = (*blake3::hash(&raw).as_bytes(), raw.len());
    let length = match blobs.entry(identity) {
        std::collections::hash_map::Entry::Occupied(entry) => {
            let &(start, length) = entry.get();
            bytes.extend_from_within(start..start + length);
            #[cfg(test)]
            BLOB_WORK.with(|work| work.borrow_mut().1 += raw.len());
            length
        }
        std::collections::hash_map::Entry::Vacant(entry) => {
            let encoded = if raw.is_empty() {
                Vec::new()
            } else {
                compress(&raw, 3)?
            };
            let length = encoded.len();
            bytes.extend(encoded);
            #[cfg(test)]
            BLOB_WORK.with(|work| work.borrow_mut().0 += raw.len());
            entry.insert((offset, length));
            length
        }
    };
    put(bytes, at, &(offset as u64).to_le_bytes());
    put(bytes, at + 8, &(length as u64).to_le_bytes());
    Ok(())
}
#[cfg(test)]
thread_local! {
    /// Raw bytes compressed and reused on this thread.
    static BLOB_WORK: std::cell::RefCell<(usize, usize)> = const { std::cell::RefCell::new((0, 0)) };
}
fn run_bucket(run: Run, first: i32, last: i32, decimals: u8) -> ChartBucket {
    constant_bucket(
        run.value,
        first,
        last,
        run.break_before && first == run.first_hour,
        decimals,
    )
}
/// The bucket of one raw value held from `first` through `last`.
fn constant_bucket(
    raw: i128,
    first: i32,
    last: i32,
    first_break: bool,
    decimals: u8,
) -> ChartBucket {
    let value = raw as f64 / 10_f64.powi(i32::from(decimals));
    ChartBucket {
        absolute_total: raw.unsigned_abs() * (i64::from(last) - i64::from(first) + 1) as u128,
        first_hour: first,
        last_hour: last,
        first_value: value,
        last_value: value,
        last_value_hour: first,
        min_hour: first,
        min_value: value,
        max_hour: first,
        max_value: value,
        first_break,
        has_break: false,
    }
}
/// Return the shared chart grid and whether its bins are exact hours.
pub fn summary_bins(
    bounds: (i32, i32),
    view: (f64, f64, u32),
) -> io::Result<(bool, Vec<(i32, i32)>)> {
    let (first, last) = bounds;
    let (from, to, width) = view;
    viewport(Some(from), Some(to), Some(width))?;
    let from_hour = from.floor().max(f64::from(first)) as i32;
    let to_hour = to.floor().min(f64::from(last)) as i32;
    if from_hour > to_hour {
        return Ok((true, Vec::new()));
    }
    // Fine views retain today's power-of-two geometry exactly; wide views fit the screen.
    let count = i64::from(to_hour) - i64::from(from_hour) + 1;
    let per_pixel = (count / i64::from(width)).max(1);
    let fine = per_pixel <= LEVELS[0];
    let size = if fine {
        1_i64 << (63 - (per_pixel as u64).leading_zeros())
    } else {
        count.div_euclid(i64::from(width)) + i64::from(count % i64::from(width) != 0)
    };
    let mut bins = Vec::new();
    let mut start = i64::from(from_hour);
    while start <= i64::from(to_hour) {
        let boundary = if fine {
            i64::from(first) + ((start - i64::from(first)) / size + 1) * size - 1
        } else {
            start + size - 1
        };
        let end = boundary.min(i64::from(to_hour));
        bins.push((start as i32, end as i32));
        start = end + 1;
    }
    Ok((size == 1, bins))
}
fn validate_membership(
    within: Option<&[crate::products::country_groups::HourSpan]>,
) -> io::Result<()> {
    if let Some(spans) = within
        && (spans.iter().any(|span| span.first_hour > span.last_hour)
            || spans
                .windows(2)
                .any(|pair| pair[0].last_hour >= pair[1].first_hour))
    {
        return Err(invalid("Invalid series membership"));
    }
    Ok(())
}
fn coverage_break(bucket: &mut Option<ChartBucket>, bounds: (i32, i32)) {
    if let Some(bucket) = bucket {
        bucket.has_break |= bucket.first_hour > bounds.0 || bucket.last_hour < bounds.1;
    }
}
/// Merge a non-overlapping unpublished continuation on the summary's existing
/// grid. Callers retain the distinction between exact and sampled spans.
pub fn merge_tail(
    summary: &mut ChartSummary,
    bounds: (i32, i32),
    tail: &[Run],
    preceding: Option<Run>,
    decimals: u8,
    within: Option<&[crate::products::country_groups::HourSpan]>,
) -> io::Result<()> {
    let (exact, bins) = summary_bins(bounds, (summary.from, summary.to, summary.width))?;
    summary.buckets.resize(bins.len(), None);
    if !exact {
        summary.runs = None;
    }
    for &run in tail {
        let whole = crate::products::country_groups::HourSpan {
            first_hour: run.first_hour,
            last_hour: run.last_hour,
        };
        let spans = within.unwrap_or_else(|| std::slice::from_ref(&whole));
        let start = spans.partition_point(|span| span.last_hour < run.first_hour);
        for span in spans[start..]
            .iter()
            .take_while(|span| span.first_hour <= run.last_hour)
        {
            let first = run.first_hour.max(span.first_hour);
            let last = run.last_hour.min(span.last_hour);
            let start = bins.partition_point(|(_, end)| *end < first);
            for (index, &(from, to)) in bins.iter().enumerate().skip(start) {
                if from > last {
                    break;
                }
                let bucket = run_bucket(run, first.max(from), last.min(to), decimals);
                summary.buckets[index] = if summary.buckets[index].is_some() {
                    merge(summary.buckets[index], Some(bucket))
                } else {
                    Some(bucket)
                };
            }
        }
    }
    if within.is_none() {
        for (bucket, &bounds) in summary.buckets.iter_mut().zip(&bins) {
            coverage_break(bucket, bounds);
        }
    }
    if let Some(runs) = &mut summary.runs {
        let mut canonical = Vec::<Run>::new();
        for &run in tail {
            if let Some(previous) = canonical.last_mut()
                && previous.continues(run)
            {
                previous.last_hour = run.last_hour;
            } else {
                canonical.push(run);
            }
        }
        if let Some(head) = canonical.first_mut()
            && let Some(mut previous) = preceding
        {
            previous.last_hour = head.first_hour - 1;
            if previous.continues(*head) {
                head.first_hour = previous.first_hour;
                head.break_before = previous.break_before;
            }
        }
        for run in canonical {
            if let Some(previous) = runs
                .last_mut()
                .filter(|previous| previous.first_hour == run.first_hour)
            {
                previous.last_hour = run.last_hour;
            } else {
                runs.push(ChartRun {
                    first_hour: run.first_hour,
                    last_hour: run.last_hour,
                    value: run.value as f64 / 10_f64.powi(i32::from(decimals)),
                    break_before: run.break_before,
                });
            }
        }
        let first = summary.from.floor().max(f64::from(bounds.0));
        let last = summary.to.floor().min(f64::from(bounds.1));
        runs.retain(|run| {
            first <= last && f64::from(run.last_hour) >= first && f64::from(run.first_hour) <= last
        });
        if let Some(spans) = within {
            *runs = runs_within(runs, spans);
        }
    }
    Ok(())
}
fn runs_within(
    runs: &[ChartRun],
    spans: &[crate::products::country_groups::HourSpan],
) -> Vec<ChartRun> {
    runs.iter()
        .flat_map(|run| {
            let start = spans.partition_point(|span| span.last_hour < run.first_hour);
            spans[start..]
                .iter()
                .take_while(|span| span.first_hour <= run.last_hour)
                .filter_map(move |span| {
                    let first = run.first_hour.max(span.first_hour);
                    let last = run.last_hour.min(span.last_hour);
                    (first <= last).then_some(ChartRun {
                        first_hour: first,
                        last_hour: last,
                        break_before: run.break_before || first != run.first_hour,
                        ..run.clone()
                    })
                })
        })
        .collect()
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn i32_at(bytes: &[u8], at: usize) -> i32 {
    i32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}
fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}
fn read<const N: usize>(file: &mut (impl Read + Seek), at: u64) -> io::Result<[u8; N]> {
    #[cfg(test)]
    directory_tests::SMALL_READS.set(directory_tests::SMALL_READS.get() + 1);
    file.seek(SeekFrom::Start(at))?;
    let mut bytes = [0; N];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}
fn lower(count: u64, mut before: impl FnMut(u64) -> io::Result<bool>) -> io::Result<u64> {
    let (mut first, mut last) = (0, count);
    while first < last {
        let middle = first + (last - first) / 2;
        if before(middle)? {
            first = middle + 1;
        } else {
            last = middle;
        }
    }
    Ok(first)
}
#[derive(Default, Clone, Copy)]
struct Directory {
    blob: (u64, u64),
    run_rows: u64,
    run_count: u64,
    event_base: u64,
    event_rows: u64,
    last_end: i32,
}
impl Directory {
    fn first_run(self) -> u64 {
        self.run_count.saturating_sub(self.run_rows) + 1
    }
}
fn directory(file: &mut (impl Read + Seek), key: u64) -> io::Result<Directory> {
    let count = u64_at(&read::<16>(file, 0)?, 8);
    // A damaged or foreign file can claim any count; its entries then lie past the end.
    let entry = |index: u64| {
        index
            .checked_mul(DIRECTORY)
            .and_then(|offset| offset.checked_add(HEADER))
            .ok_or_else(|| invalid("prepared series directory is out of range"))
    };
    let index = lower(count, |index| {
        Ok(u64_at(&read::<8>(file, entry(index)?)?, 0) < key)
    })?;
    if index == count {
        return Ok(Directory::default());
    }
    let bytes = read::<64>(file, entry(index)?)?;
    if u64_at(&bytes, 0) != key {
        return Ok(Directory::default());
    }
    Ok(Directory {
        blob: (u64_at(&bytes, 8), u64_at(&bytes, 16)),
        run_rows: u64_at(&bytes, 24),
        run_count: u64_at(&bytes, 32),
        event_base: u64_at(&bytes, 40),
        event_rows: u64_at(&bytes, 48),
        last_end: i32_at(&bytes, 56),
    })
}
/// One batch's whole directory, read at once: its entries in key order.
struct Table(Vec<u8>);
impl Table {
    /// The batch's directory in one read, or None when the count its header
    /// claims does not fit the batch (a damaged file), which single-key
    /// searches then answer exactly as before.
    fn read(file: &mut store::Range) -> io::Result<Option<Self>> {
        let count = u64_at(&read::<16>(file, 0)?, 8);
        let Some(bytes) = count
            .checked_mul(DIRECTORY)
            .filter(|bytes| {
                bytes
                    .checked_add(HEADER)
                    .is_some_and(|end| end <= file.len())
            })
            .and_then(|bytes| usize::try_from(bytes).ok())
        else {
            return Ok(None);
        };
        #[cfg(test)]
        directory_tests::SMALL_READS.set(directory_tests::SMALL_READS.get() + 1);
        let mut table = vec![0; bytes];
        file.seek(SeekFrom::Start(HEADER))?;
        file.read_exact(&mut table)?;
        Ok(Some(Self(table)))
    }
    /// The entry [`directory`] finds for `key`, searched the same way in memory.
    fn entry(&self, key: u64) -> io::Result<Directory> {
        let at = |index: u64| (index * DIRECTORY) as usize;
        let count = self.0.len() as u64 / DIRECTORY;
        let index = lower(count, |index| Ok(u64_at(&self.0, at(index)) < key))?;
        if index == count {
            return Ok(Directory::default());
        }
        let bytes = &self.0[at(index)..at(index + 1)];
        if u64_at(bytes, 0) != key {
            return Ok(Directory::default());
        }
        Ok(Directory {
            blob: (u64_at(bytes, 8), u64_at(bytes, 16)),
            run_rows: u64_at(bytes, 24),
            run_count: u64_at(bytes, 32),
            event_base: u64_at(bytes, 40),
            event_rows: u64_at(bytes, 48),
            last_end: i32_at(bytes, 56),
        })
    }
}
/// Single-key searches in one batch before a reader considers reading its
/// whole directory instead.
const DIRECTORY_SEARCHES: u64 = 16;
/// A batch's whole directory is read once it has had one single-key search
/// per this many entries (and at least [`DIRECTORY_SEARCHES`]). A search is
/// about log2(entries) separate small reads; the whole directory is one read
/// of 64 bytes per entry, so by then the searches have cost about as much as
/// the read, and a reader never spends more than about twice the cheaper way.
const ENTRIES_PER_SEARCH: u64 = 256;
/// Whole-directory bytes a reader keeps before it drops them and counts its
/// searches again.
const DIRECTORY_TABLE_BYTES: usize = 64 << 20;

#[cfg(test)]
pub(crate) mod directory_tests;

struct Cached {
    runs: Vec<Run>,
    buckets: [Vec<(i64, ChartBucket)>; 4],
    events: Vec<(i32, Vec<u8>)>,
}
fn decode(raw: &[u8], dir: Directory, cancelled: &AtomicBool) -> io::Result<Cached> {
    let mut bytes = raw;
    let mut cached = Cached {
        runs: Vec::new(),
        buckets: Default::default(),
        events: Vec::new(),
    };
    let (mut hour, mut value) = (0_i128, 0_i128);
    for _ in 0..dir.run_rows {
        check_cancel(cancelled)?;
        hour += signed(&mut bytes)?;
        let first = i32::try_from(hour).map_err(|_| invalid("run hour overflow"))?;
        hour += i128::try_from(take(&mut bytes)?).map_err(|_| invalid("run duration overflow"))?;
        let last = i32::try_from(hour).map_err(|_| invalid("run hour overflow"))?;
        value = value.wrapping_add(signed(&mut bytes)?);
        let flags = take(&mut bytes)?;
        if flags > 1 {
            return Err(invalid("invalid run flags"));
        }
        cached.runs.push(Run::new(first, last, value, flags != 0));
    }
    hour = 0;
    for _ in 0..dir.event_rows {
        check_cancel(cancelled)?;
        hour += signed(&mut bytes)?;
        let date = i32::try_from(hour).map_err(|_| invalid("event hour overflow"))?;
        let length =
            usize::try_from(take(&mut bytes)?).map_err(|_| invalid("event length overflow"))?;
        let row = bytes
            .get(..length)
            .ok_or_else(|| invalid("truncated event row"))?;
        cached.events.push((date, row.to_vec()));
        bytes = &bytes[length..];
    }
    if !bytes.is_empty() {
        for rows in &mut cached.buckets {
            let count = take(&mut bytes)?;
            let mut aligned = 0_i64;
            for _ in 0..count {
                check_cancel(cancelled)?;
                aligned += i64::try_from(signed(&mut bytes)?)
                    .map_err(|_| invalid("bucket alignment overflow"))?;
                decode_bucket(&mut bytes, aligned)?;
                let bucket = decode_bucket(&mut bytes, aligned)?
                    .ok_or_else(|| invalid("missing summary snapshot"))?;
                rows.push((aligned, bucket));
            }
            let count = take(&mut bytes)?;
            for _ in 0..count {
                signed(&mut bytes)?;
                take(&mut bytes)?;
            }
        }
    }
    if !bytes.is_empty() {
        return Err(invalid("trailing series bytes"));
    }
    Ok(cached)
}
/// Decoded blobs a reader keeps before it starts again.
const DECODED_BLOBS: usize = 256;

/// One query's view of a section's batches. It verifies the source key and
/// reads the manifest once (through a [`store::Session`]); keeps every batch's
/// range handle and hour bounds once read, so batch searches run in memory; and
/// keeps directory entries and decoded blobs it has read. A batch it searches
/// for many keys has its whole directory read once and searched in memory.
pub struct Reader<'a> {
    store: &'a store::Store,
    key: &'a store::SourceKey,
    section: String,
    count: u64,
    cancelled: &'a AtomicBool,
    session: Option<store::Session>,
    files: HashMap<u64, store::Range>,
    hours: HashMap<u64, (i32, i32)>,
    directories: HashMap<(u64, u64), Directory>,
    cached: HashMap<(u64, u64), Cached>,
    /// Each batch's single-key directory searches so far, and the entry count
    /// its header claims once read.
    searches: HashMap<u64, (u64, Option<u64>)>,
    /// Whole batch directories read so far; None for a batch whose header
    /// claims more entries than it holds.
    tables: HashMap<u64, Option<Table>>,
    table_bytes: usize,
}
impl<'a> Reader<'a> {
    pub fn from_first_file(
        store: &'a store::Store,
        key: &'a store::SourceKey,
        section: String,
        count: u64,
        first: Option<store::Range>,
        cancelled: &'a AtomicBool,
    ) -> Self {
        Self {
            store,
            key,
            section,
            count,
            cancelled,
            session: None,
            files: first.map(|file| (0, file)).into_iter().collect(),
            hours: HashMap::new(),
            directories: HashMap::new(),
            cached: HashMap::new(),
            searches: HashMap::new(),
            tables: HashMap::new(),
            table_bytes: 0,
        }
    }

    pub fn new(
        store: &'a store::Store,
        key: &'a store::SourceKey,
        section: String,
        cancelled: &'a AtomicBool,
    ) -> io::Result<Self> {
        let session = store.session(key)?;
        let count = session
            .as_ref()
            .and_then(|session| session.range_count(&section))
            .unwrap_or(0);
        Ok(Self {
            store,
            key,
            section,
            count,
            cancelled,
            session,
            files: HashMap::new(),
            hours: HashMap::new(),
            directories: HashMap::new(),
            cached: HashMap::new(),
            searches: HashMap::new(),
            tables: HashMap::new(),
            table_bytes: 0,
        })
    }
    fn file(&mut self, batch: u64) -> io::Result<&mut store::Range> {
        check_cancel(self.cancelled)?;
        if !self.files.contains_key(&batch) {
            if self.session.is_none() {
                self.session = self.store.session(self.key)?;
            }
            let file = self
                .session
                .as_ref()
                .map(|session| session.read(&self.section, batch))
                .transpose()?
                .flatten()
                .ok_or_else(|| invalid("series batch is missing"))?;
            self.files.insert(batch, file);
        }
        Ok(self.files.get_mut(&batch).unwrap())
    }
    /// A batch's first and last hour, read from its header once.
    fn hours(&mut self, batch: u64) -> io::Result<(i32, i32)> {
        if let Some(&hours) = self.hours.get(&batch) {
            return Ok(hours);
        }
        let header = read::<16>(self.file(batch)?, 0)?;
        let hours = (i32_at(&header, 0), i32_at(&header, 4));
        self.hours.insert(batch, hours);
        Ok(hours)
    }
    fn info(&mut self, batch: u64, key: u64) -> io::Result<Directory> {
        check_cancel(self.cancelled)?;
        if let Some(&dir) = self.directories.get(&(batch, key)) {
            return Ok(dir);
        }
        if let Some(table) = self.table(batch)? {
            return table.entry(key);
        }
        let dir = directory(self.file(batch)?, key)?;
        self.directories.insert((batch, key), dir);
        Ok(dir)
    }
    /// The batch's whole directory, once its single-key searches have cost
    /// about as much as reading it; None until then. Counts this search.
    fn table(&mut self, batch: u64) -> io::Result<Option<&Table>> {
        if self.tables.contains_key(&batch) {
            return Ok(self.tables[&batch].as_ref());
        }
        let (searches, count) = *self.searches.entry(batch).or_default();
        let searches = searches + 1;
        let count = match count {
            Some(count) => Some(count),
            None if searches >= DIRECTORY_SEARCHES => {
                Some(u64_at(&read::<16>(self.file(batch)?, 0)?, 8))
            }
            None => None,
        };
        self.searches.insert(batch, (searches, count));
        if count.is_none_or(|count| searches < DIRECTORY_SEARCHES.max(count / ENTRIES_PER_SEARCH)) {
            return Ok(None);
        }
        let table = Table::read(self.file(batch)?)?;
        let bytes = table.as_ref().map_or(0, |table| table.0.len());
        if bytes > DIRECTORY_TABLE_BYTES {
            // Too large to keep: this batch keeps single-key searches.
            self.tables.insert(batch, None);
            return Ok(None);
        }
        if self.table_bytes + bytes > DIRECTORY_TABLE_BYTES {
            self.tables.clear();
            self.searches.clear();
            self.table_bytes = 0;
        }
        self.table_bytes += bytes;
        self.tables.insert(batch, table);
        Ok(self.tables[&batch].as_ref())
    }
    fn data(&mut self, batch: u64, key: u64) -> io::Result<&Cached> {
        if !self.cached.contains_key(&(batch, key)) {
            let dir = self.info(batch, key)?;
            let raw = if dir.blob.1 == 0 {
                Vec::new()
            } else {
                let file = self.file(batch)?;
                file.seek(SeekFrom::Start(dir.blob.0))?;
                let mut compressed = Vec::new();
                file.take(dir.blob.1).read_to_end(&mut compressed)?;
                decompress(&compressed)?
            };
            let decoded = decode(&raw, dir, self.cancelled)?;
            if self.cached.len() >= DECODED_BLOBS {
                self.cached.clear();
            }
            self.cached.insert((batch, key), decoded);
        }
        Ok(&self.cached[&(batch, key)])
    }
    /// Read only the batches committed by `hour` (those ending by then): the
    /// section as a preparation that had accepted no later hour published it,
    /// for a reader whose accepted coverage ends before pages a family
    /// prepared on its own committed ahead of it. The last hour that prefix
    /// reaches; None when no batch ends by then.
    pub fn through_hour(&mut self, hour: i32) -> io::Result<Option<i32>> {
        self.count = lower(self.count, |batch| Ok(self.hours(batch)?.1 <= hour))?;
        match self.count.checked_sub(1) {
            Some(last) => Ok(Some(self.hours(last)?.1)),
            None => Ok(None),
        }
    }
    fn batches(&mut self, from: i32, to: i32) -> io::Result<std::ops::Range<u64>> {
        let first = lower(self.count, |batch| Ok(self.hours(batch)?.1 < from))?;
        let last = lower(self.count, |batch| Ok(self.hours(batch)?.0 <= to))?;
        Ok(first..last)
    }
    /// Read amounts from the hour's batch once, without reconstructing run bounds.
    pub fn values_at(&mut self, keys: &[u64], hour: i32) -> io::Result<Vec<Option<i128>>> {
        check_cancel(self.cancelled)?;
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let Some(batch) = self.batches(hour, hour)?.next() else {
            return Ok(vec![None; keys.len()]);
        };
        keys.iter()
            .map(|&key| {
                let rows = &self.data(batch, key)?.runs;
                let index = rows.partition_point(|run| run.first_hour <= hour);
                Ok(index
                    .checked_sub(1)
                    .map(|index| rows[index])
                    .filter(|run| run.last_hour >= hour)
                    .map(|run| run.value))
            })
            .collect()
    }
    pub fn value_at(&mut self, key: u64, hour: i32) -> io::Result<Option<Run>> {
        let Some(batch) = self.batches(hour, hour)?.next() else {
            return Ok(None);
        };
        let dir = self.info(batch, key)?;
        let rows = &self.data(batch, key)?.runs;
        let index = rows.partition_point(|run| run.first_hour <= hour);
        let Some(mut value) = index
            .checked_sub(1)
            .map(|index| rows[index])
            .filter(|run| run.last_hour >= hour)
        else {
            return Ok(None);
        };
        let ordinal = dir.first_run() + index as u64 - 1;
        let first = lower(self.count, |batch| {
            Ok(self.info(batch, key)?.run_count < ordinal)
        })?;
        let start = self.info(first, key)?.first_run();
        let origin = self.data(first, key)?.runs[(ordinal - start) as usize];
        value.first_hour = origin.first_hour;
        value.break_before = origin.break_before;
        if ordinal == dir.run_count {
            let end = lower(self.count, |batch| {
                Ok(self.info(batch, key)?.run_count <= ordinal)
            })?;
            value.last_hour = value
                .last_hour
                .max(self.info(end.saturating_sub(1), key)?.last_end);
            if end < self.count && self.info(end, key)?.first_run() == ordinal {
                value.last_hour = value.last_hour.max(self.data(end, key)?.runs[0].last_hour);
            }
        }
        Ok(Some(value))
    }
    /// Exact runs intersecting one requested window, joining continued batch tails.
    pub fn runs(&mut self, key: u64, from: i32, to: i32) -> io::Result<Vec<Run>> {
        let mut result: Vec<Run> = Vec::new();
        for batch in self.batches(from, to)? {
            for &run in &self.data(batch, key)?.runs {
                if run.last_hour < from || run.first_hour > to {
                    continue;
                }
                let run = Run {
                    first_hour: run.first_hour.max(from),
                    last_hour: run.last_hour.min(to),
                    ..run
                };
                if let Some(last) = result.last_mut().filter(|last| last.continues(run)) {
                    last.last_hour = run.last_hour;
                } else {
                    result.push(run);
                }
            }
        }
        Ok(result)
    }
    pub fn next_change(&mut self, key: u64, hour: f64, forward: bool) -> io::Result<Option<i32>> {
        if !hour.is_finite() {
            return Err(invalid("invalid navigation hour"));
        }
        if forward {
            if let Some(value) = self.value_at(key, hour.floor() as i32)? {
                return Ok(value.last_hour.checked_add(1));
            }
            let batch = lower(self.count, |batch| {
                Ok(f64::from(self.hours(batch)?.1) <= hour)
            })?;
            if batch == self.count {
                return Ok(None);
            }
            let dir = self.info(batch, key)?;
            let rows = &self.data(batch, key)?.runs;
            if let Some(value) =
                rows.get(rows.partition_point(|run| f64::from(run.first_hour) <= hour))
            {
                return Ok(Some(value.first_hour));
            }
            let next = lower(self.count, |batch| {
                Ok(self.info(batch, key)?.run_count <= dir.run_count)
            })?;
            if next == self.count {
                return Ok(None);
            }
            let first = self.info(next, key)?.first_run();
            Ok(Some(
                self.data(next, key)?.runs[(dir.run_count + 1 - first) as usize].first_hour,
            ))
        } else {
            let end = lower(self.count, |batch| {
                Ok(f64::from(self.hours(batch)?.0) < hour)
            })?;
            if end == 0 {
                return Ok(None);
            }
            let rows = &self.data(end - 1, key)?.runs;
            let index = rows.partition_point(|run| f64::from(run.first_hour) < hour);
            let last = if index > 0 {
                rows[index - 1].last_hour
            } else if end > 1 {
                let previous = self.info(end - 2, key)?;
                if previous.run_count == 0 {
                    return Ok(None);
                }
                previous.last_end
            } else {
                return Ok(None);
            };
            let run = self
                .value_at(key, last)?
                .ok_or_else(|| invalid("missing preceding run"))?;
            Ok(Some(
                run.last_hour
                    .checked_add(1)
                    .filter(|last| f64::from(*last) < hour)
                    .unwrap_or(run.first_hour),
            ))
        }
    }
    pub fn summary(
        &mut self,
        key: u64,
        bounds: (i32, i32),
        view: (f64, f64, u32),
        decimals: u8,
    ) -> io::Result<ChartSummary> {
        let (from, to, width) = view;
        let (exact, bins) = summary_bins(bounds, view)?;
        let mut result = ChartSummary {
            from,
            to,
            width,
            buckets: Vec::new(),
            runs: exact.then(Vec::new),
            approximate: Vec::new(),
        };
        for (start, end) in bins {
            let bucket = if let Some(runs) = &mut result.runs {
                let run = self.value_at(key, start)?;
                if let Some(run) = run
                    && runs
                        .last()
                        .is_none_or(|previous| previous.first_hour != run.first_hour)
                {
                    runs.push(ChartRun {
                        first_hour: run.first_hour,
                        last_hour: run.last_hour,
                        value: run.value as f64 / 10_f64.powi(i32::from(decimals)),
                        break_before: run.break_before,
                    });
                }
                run.map(|run| run_bucket(run, start, end, decimals))
            } else {
                self.range_bucket(key, start, end, decimals)?
            };
            result.buckets.push(bucket);
        }
        Ok(result)
    }
    /// Dated keys share the existing summary grid and membership masking. A
    /// known alias boundary can continue a lane; excluded hours remain unknown.
    pub fn summary_spans(
        &mut self,
        spans: &[(u64, i32, i32, Option<bool>)],
        bounds: (i32, i32),
        view: (f64, f64, u32),
        decimals: u8,
    ) -> io::Result<ChartSummary> {
        viewport(Some(view.0), Some(view.1), Some(view.2))?;
        if spans.iter().any(|span| span.1 > span.2)
            || spans.windows(2).any(|pair| pair[0].2 >= pair[1].1)
        {
            return Err(invalid("Invalid dated series spans"));
        }
        let mut result: Option<ChartSummary> = None;
        for &(key, first, last, boundary) in spans {
            if f64::from(last) < view.0.floor() || f64::from(first) > view.1.floor() {
                continue;
            }
            let within = [crate::products::country_groups::HourSpan {
                first_hour: first,
                last_hour: last,
            }];
            let mut part = self.summary_within(key, bounds, view, decimals, Some(&within))?;
            if let Some(starts) = boundary {
                for bucket in part.buckets.iter_mut().flatten() {
                    if bucket.first_hour == first {
                        bucket.first_break = starts;
                    }
                }
                for run in part.runs.iter_mut().flatten() {
                    if run.first_hour == first {
                        run.break_before = starts;
                    }
                }
            }
            if let Some(result) = &mut result {
                for (bucket, next) in result.buckets.iter_mut().zip(part.buckets) {
                    *bucket = match (*bucket, next) {
                        (Some(_), Some(_)) => merge(*bucket, next),
                        (previous, next) => previous.or(next),
                    };
                }
                if let (Some(runs), Some(next)) = (&mut result.runs, part.runs) {
                    runs.extend(next);
                }
            } else {
                result = Some(part);
            }
        }
        Ok(result.unwrap_or(ChartSummary {
            from: view.0,
            to: view.1,
            width: view.2,
            buckets: Vec::new(),
            runs: Some(Vec::new()),
            approximate: Vec::new(),
        }))
    }
    pub fn summary_within(
        &mut self,
        key: u64,
        bounds: (i32, i32),
        view: (f64, f64, u32),
        decimals: u8,
        within: Option<&[crate::products::country_groups::HourSpan]>,
    ) -> io::Result<ChartSummary> {
        let mut summary = self.summary(key, bounds, view, decimals)?;
        validate_membership(within)?;
        if let Some(spans) = within {
            if let Some(runs) = &mut summary.runs {
                *runs = runs_within(runs, spans);
            }
            for bucket in &mut summary.buckets {
                *bucket = self.bucket_within(key, *bucket, decimals, spans)?;
            }
        }
        Ok(summary)
    }
    fn bucket_within(
        &mut self,
        key: u64,
        original: Option<ChartBucket>,
        decimals: u8,
        spans: &[crate::products::country_groups::HourSpan],
    ) -> io::Result<Option<ChartBucket>> {
        let mut bucket = None;
        if let Some(original) = original {
            let first = spans.partition_point(|span| span.last_hour < original.first_hour);
            for span in spans[first..]
                .iter()
                .take_while(|span| span.first_hour <= original.last_hour)
            {
                let value = self.range_bucket(
                    key,
                    original.first_hour.max(span.first_hour),
                    original.last_hour.min(span.last_hour),
                    decimals,
                )?;
                bucket = if bucket.is_none() {
                    value
                } else {
                    merge(bucket, value)
                };
            }
        }
        Ok(bucket)
    }
    /// Merge a derived continuation without treating its unpublished hours as gaps.
    pub fn summary_with_tail(
        &mut self,
        key: u64,
        bounds: (i32, i32),
        view: (f64, f64, u32),
        decimals: u8,
        tail: &[Run],
        within: Option<&[crate::products::country_groups::HourSpan]>,
    ) -> io::Result<ChartSummary> {
        let Some(head) = tail.first() else {
            return self.summary_within(key, bounds, view, decimals, within);
        };
        validate_membership(within)?;
        let (exact, bins) = summary_bins(bounds, view)?;
        let mut summary = ChartSummary {
            approximate: Vec::new(),
            from: view.0,
            to: view.1,
            width: view.2,
            buckets: Vec::with_capacity(bins.len()),
            runs: exact.then(Vec::new),
        };
        // Sparse tails can begin after settled publication, so an aligned
        // prefix bucket must not ask for unpublished hours beyond its last file.
        let published_last = if self.count == 0 {
            None
        } else {
            Some(self.hours(self.count - 1)?.1)
        };
        for (first, last) in bins {
            let end = published_last.map_or(i64::from(first) - 1, |published| {
                i64::from(last)
                    .min(i64::from(head.first_hour) - 1)
                    .min(i64::from(published))
            });
            let mut bucket = None;
            if i64::from(first) <= end {
                if let Some(runs) = &mut summary.runs {
                    if let Some(run) = self.value_at(key, first)? {
                        let run = Run {
                            last_hour: run.last_hour.min(head.first_hour - 1),
                            ..run
                        };
                        if runs
                            .last()
                            .is_none_or(|previous| previous.first_hour != run.first_hour)
                        {
                            runs.push(ChartRun {
                                first_hour: run.first_hour,
                                last_hour: run.last_hour,
                                value: run.value as f64 / 10_f64.powi(i32::from(decimals)),
                                break_before: run.break_before,
                            });
                        }
                        bucket = Some(run_bucket(run, first, end as i32, decimals));
                    }
                } else {
                    bucket = self.range_bucket(key, first, end as i32, decimals)?;
                }
                if let Some(spans) = within {
                    bucket = self.bucket_within(key, bucket, decimals, spans)?;
                }
            }
            summary.buckets.push(bucket);
        }
        let preceding = if exact {
            match head.first_hour.checked_sub(1) {
                Some(hour) => self.value_at(key, hour)?,
                None => None,
            }
        } else {
            None
        };
        merge_tail(&mut summary, bounds, tail, preceding, decimals, within)?;
        Ok(summary)
    }
    fn range_bucket(
        &mut self,
        key: u64,
        from: i32,
        to: i32,
        decimals: u8,
    ) -> io::Result<Option<ChartBucket>> {
        let mut result = None;
        let mut initialized = false;
        let mut hour = i64::from(from);
        while hour <= i64::from(to) {
            let bucket = if let Some((level, &size)) =
                LEVELS.iter().enumerate().rev().find(|(_, size)| {
                    hour.rem_euclid(**size) == 0 && hour + **size - 1 <= i64::from(to)
                }) {
                let end = (hour + size - 1) as i32;
                let batch = lower(self.count, |batch| Ok(self.hours(batch)?.1 < end))?;
                let mut value = None;
                if batch < self.count {
                    let values = &self.data(batch, key)?.buckets[level];
                    let index = values.partition_point(|(aligned, _)| *aligned < hour);
                    if let Some((_, bucket)) =
                        values.get(index).filter(|(aligned, _)| *aligned == hour)
                    {
                        value = Some(*bucket);
                    }
                }
                hour += size;
                value
            } else {
                let end = i64::from(to).min((hour.div_euclid(LEVELS[0]) + 1) * LEVELS[0] - 1);
                let mut value = None;
                let mut has_value = false;
                while hour <= end {
                    let next = self.value_at(key, hour as i32)?;
                    let bucket = next.map(|run| {
                        run_bucket(run, hour as i32, run.last_hour.min(end as i32), decimals)
                    });
                    value = if has_value {
                        merge(value, bucket)
                    } else {
                        bucket
                    };
                    has_value = true;
                    hour =
                        next.map_or(hour + 1, |run| i64::from(run.last_hour.min(end as i32)) + 1);
                }
                value
            };
            result = if initialized {
                merge(result, bucket)
            } else {
                bucket
            };
            initialized = true;
        }
        coverage_break(&mut result, (from, to));
        Ok(result)
    }
    pub fn event_count_through(&mut self, key: u64, hour: i32) -> io::Result<u64> {
        let end = lower(self.count, |batch| Ok(self.hours(batch)?.0 <= hour))?;
        if end == 0 {
            return Ok(0);
        }
        let dir = self.info(end - 1, key)?;
        Ok(dir.event_base
            + self
                .data(end - 1, key)?
                .events
                .partition_point(|(date, _)| *date <= hour) as u64)
    }
    fn event_at(&mut self, key: u64, ordinal: u64) -> io::Result<DatedRow> {
        let batch = lower(self.count, |batch| {
            let dir = self.info(batch, key)?;
            Ok(dir.event_base + dir.event_rows <= ordinal)
        })?;
        let (hour, row) = self.event_row(batch, key, ordinal)?;
        Ok(DatedRow {
            key,
            hour: *hour,
            ordinal,
            row: serde_json::from_slice(row)?,
        })
    }
    pub fn latest_event(&mut self, key: u64, hour: i32) -> io::Result<Option<(i32, Value)>> {
        let count = self.event_count_through(key, hour)?;
        if count == 0 {
            return Ok(None);
        }
        let row = self.event_at(key, count - 1)?;
        Ok(Some((row.hour, row.row)))
    }
    /// [`Self::latest_event`] of each key at one hour, each row still
    /// serialized. The hour's batch is found once. A key's row comes from that
    /// batch when the key has events there, else from the batch holding it; a
    /// key without events in the hour's batch is not decoded there.
    pub fn latest_event_rows(
        &mut self,
        keys: &[u64],
        hour: i32,
    ) -> io::Result<Vec<Option<(i32, Vec<u8>)>>> {
        let end = lower(self.count, |batch| Ok(self.hours(batch)?.0 <= hour))?;
        let mut rows = Vec::with_capacity(keys.len());
        for &key in keys {
            check_cancel(self.cancelled)?;
            let Some(last) = end.checked_sub(1) else {
                rows.push(None);
                continue;
            };
            let dir = self.info(last, key)?;
            let within = if dir.event_rows == 0 {
                0
            } else {
                self.data(last, key)?
                    .events
                    .partition_point(|(date, _)| *date <= hour) as u64
            };
            let Some(ordinal) = (dir.event_base + within).checked_sub(1) else {
                rows.push(None);
                continue;
            };
            // Event ordinals only grow from batch to batch, so a row at or
            // after this batch's base is in this batch.
            let batch = if ordinal >= dir.event_base {
                last
            } else {
                lower(self.count, |batch| {
                    let dir = self.info(batch, key)?;
                    Ok(dir.event_base + dir.event_rows <= ordinal)
                })?
            };
            let (date, row) = self.event_row(batch, key, ordinal)?;
            rows.push(Some((*date, row.clone())));
        }
        Ok(rows)
    }
    fn event_count(&mut self, keys: &[u64], hour: i32) -> io::Result<u64> {
        keys.iter().try_fold(0_u64, |total, &key| {
            total
                .checked_add(self.event_count_through(key, hour)?)
                .ok_or_else(|| invalid("event count overflow"))
        })
    }
    /// One decimal ordinal addresses the merged hour/key/duplicate order for any key selection.
    pub fn events(
        &mut self,
        keys: &[u64],
        from: i32,
        to: i32,
        cursor: u64,
        page_size: u32,
    ) -> io::Result<EventsPage> {
        if from > to || page_size == 0 {
            return Err(invalid("invalid event page"));
        }
        let keys: Vec<_> = keys
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        let base = if from == i32::MIN {
            0
        } else {
            self.event_count(&keys, from - 1)?
        };
        let last = self.event_count(&keys, to)?;
        let first = cursor.max(base);
        let end = first.saturating_add(u64::from(page_size)).min(last);
        let mut rows = Vec::new();
        if first < end {
            // One key's ordinal already identifies its hour. Multiple keys rank the same cursor by date.
            let hour = if keys.len() == 1 {
                self.event_date(keys[0], first)?
            } else {
                let count = (i64::from(to) - i64::from(from) + 1) as u64;
                let offset = lower(count, |offset| {
                    Ok(self.event_count(&keys, (i64::from(from) + offset as i64) as i32)? <= first)
                })?;
                (i64::from(from) + offset as i64) as i32
            };
            let before = if hour == i32::MIN {
                0
            } else {
                self.event_count(&keys, hour - 1)?
            };
            let mut skipped = first - before;
            let mut pending = BTreeMap::new();
            for &key in &keys {
                let begin = if hour == i32::MIN {
                    0
                } else {
                    self.event_count_through(key, hour - 1)?
                };
                let count = self.event_count_through(key, hour)? - begin;
                let skip = skipped.min(count);
                skipped -= skip;
                let ordinal = begin + skip;
                if ordinal < self.event_count_through(key, to)? {
                    let row = self.event_at(key, ordinal)?;
                    pending.insert((row.hour, key, ordinal), row);
                }
            }
            while (rows.len() as u64) < end - first {
                let Some(((_, key, ordinal), row)) = pending.pop_first() else {
                    break;
                };
                rows.push(row);
                if ordinal + 1 < self.event_count_through(key, to)? {
                    let row = self.event_at(key, ordinal + 1)?;
                    pending.insert((row.hour, key, ordinal + 1), row);
                }
            }
            if (rows.len() as u64) < end - first {
                return Err(invalid("event page rows do not match its ordinal interval"));
            }
        }
        Ok(EventsPage {
            first,
            total: last - base,
            rows,
            cursor: (end < last).then_some(end),
        })
    }
    pub fn event_clusters(
        &mut self,
        key: u64,
        from: f64,
        to: f64,
        width: u32,
    ) -> io::Result<MarkSummary> {
        self.clusters(key, from, to, width, None)
    }
    pub fn event_clusters_within(
        &mut self,
        key: u64,
        from: f64,
        to: f64,
        width: u32,
        spans: &[crate::products::country_groups::HourSpan],
    ) -> io::Result<MarkSummary> {
        self.clusters(key, from, to, width, Some(spans))
    }
    fn clusters(
        &mut self,
        key: u64,
        from: f64,
        to: f64,
        width: u32,
        spans: Option<&[crate::products::country_groups::HourSpan]>,
    ) -> io::Result<MarkSummary> {
        viewport(Some(from), Some(to), Some(width))?;
        let mut ranges = spans.map_or_else(
            || vec![(i32::MIN, i32::MAX)],
            |spans| {
                spans
                    .iter()
                    .map(|span| (span.first_hour, span.last_hour))
                    .collect()
            },
        );
        if ranges.iter().any(|(first, last)| first > last) {
            return Err(invalid("invalid event membership span"));
        }
        ranges.sort_unstable();
        let mut merged: Vec<(i32, i32)> = Vec::new();
        for (first, last) in ranges {
            if let Some(previous) = merged
                .last_mut()
                .filter(|previous| first <= previous.1.saturating_add(1))
            {
                previous.1 = previous.1.max(last);
            } else {
                merged.push((first, last));
            }
        }
        let cells = width.div_ceil(18);
        let mut clusters = Vec::new();
        for cell in 0..cells {
            let start = from + (to - from) * f64::from(cell) / f64::from(cells);
            let end = from + (to - from) * f64::from(cell + 1) / f64::from(cells);
            let first_hour = start.ceil() as i32;
            let last_hour = if cell + 1 == cells {
                end.floor()
            } else {
                end.ceil() - 1.0
            } as i32;
            if first_hour > last_hour {
                continue;
            }
            let mut first = None;
            let mut last = 0;
            let mut count = 0;
            for &(start, end) in merged
                .iter()
                .skip(merged.partition_point(|span| span.1 < first_hour))
                .take_while(|span| span.0 <= last_hour)
            {
                let start = start.max(first_hour);
                let end = end.min(last_hour);
                let begin = if start == i32::MIN {
                    0
                } else {
                    self.event_count_through(key, start - 1)?
                };
                let finish = self.event_count_through(key, end)?;
                if begin < finish {
                    first.get_or_insert(begin);
                    last = finish;
                    count += finish - begin;
                }
            }
            if let Some(first) = first {
                let a = self.event_date(key, first)?;
                let b = self.event_date(key, last - 1)?;
                clusters.push(MarkCluster {
                    first_hour: a,
                    last_hour: b,
                    first_ordinal: first.to_string(),
                    count: count.to_string(),
                });
            }
        }
        Ok(MarkSummary {
            from,
            to,
            width,
            clusters,
        })
    }
    /// The hour of a key's event at a native ordinal.
    pub fn event_date(&mut self, key: u64, ordinal: u64) -> io::Result<i32> {
        let batch = lower(self.count, |batch| {
            let dir = self.info(batch, key)?;
            Ok(dir.event_base + dir.event_rows <= ordinal)
        })?;
        Ok(self.event_row(batch, key, ordinal)?.0)
    }
    /// A key's event at a native ordinal with its row as stored, for rows a
    /// family writes in its own byte form ([`Accumulator::event_raw`]).
    pub fn event_raw(&mut self, key: u64, ordinal: u64) -> io::Result<(i32, Vec<u8>)> {
        let batch = lower(self.count, |batch| {
            let dir = self.info(batch, key)?;
            Ok(dir.event_base + dir.event_rows <= ordinal)
        })?;
        let (hour, row) = self.event_row(batch, key, ordinal)?;
        Ok((*hour, row.clone()))
    }
    /// A key's event at a native ordinal, within the batch that should hold it.
    fn event_row(&mut self, batch: u64, key: u64, ordinal: u64) -> io::Result<&(i32, Vec<u8>)> {
        let dir = self.info(batch, key)?;
        let index = ordinal
            .checked_sub(dir.event_base)
            .and_then(|index| usize::try_from(index).ok());
        let data = self.data(batch, key)?;
        index
            .and_then(|index| data.events.get(index))
            .ok_or_else(|| invalid("prepared event ordinal is outside its batch"))
    }
}
#[derive(Debug)]
pub struct DatedRow {
    pub key: u64,
    pub hour: i32,
    pub ordinal: u64,
    pub row: Value,
}
#[derive(Debug)]
pub struct EventsPage {
    pub first: u64,
    pub total: u64,
    pub rows: Vec<DatedRow>,
    pub cursor: Option<u64>,
}

pub fn range(from: Option<f64>, to: Option<f64>) -> io::Result<(f64, f64)> {
    let (from, to) = (
        from.ok_or_else(|| invalid("missing series range"))?,
        to.ok_or_else(|| invalid("missing series range"))?,
    );
    if !from.is_finite() || !to.is_finite() || from > to {
        return Err(invalid("invalid series range"));
    }
    Ok((from, to))
}
pub fn viewport(
    from: Option<f64>,
    to: Option<f64>,
    width: Option<u32>,
) -> io::Result<(f64, f64, u32)> {
    let (from, to) = range(from, to)?;
    let width = width
        .filter(|width| *width > 0)
        .ok_or_else(|| invalid("invalid series width"))?;
    Ok((from, to, width))
}
pub fn summary_budget(from: f64, to: f64, width: u32, keys: usize) -> io::Result<()> {
    let hours = (to.floor() - from.floor() + 1.0).max(0.0) as usize;
    let buckets = hours.min((width as usize).saturating_mul(2).saturating_add(2));
    if buckets
        .saturating_mul(keys)
        .saturating_mul(std::mem::size_of::<ChartBucket>() + std::mem::size_of::<ChartRun>())
        > crate::products::MAX_PRODUCT_REPLY_BYTES
    {
        return Err(invalid(
            "Series summary exceeds the transport reply budget. Request a smaller viewport.",
        ));
    }
    Ok(())
}
pub fn event_window(
    from: Option<f64>,
    to: Option<f64>,
    cursor: Option<&str>,
    page_size: Option<u32>,
) -> io::Result<(i32, i32, u64, u32)> {
    let (from, to) = range(
        Some(from.unwrap_or(f64::from(i32::MIN))),
        Some(to.unwrap_or(f64::from(i32::MAX))),
    )?;
    let cursor = cursor
        .unwrap_or("0")
        .parse()
        .map_err(|_| invalid("invalid event cursor"))?;
    let page_size = page_size
        .filter(|size| *size > 0)
        .ok_or_else(|| invalid("invalid event page size"))?;
    Ok((from.ceil() as i32, to.floor() as i32, cursor, page_size))
}
mod catalogue;
#[cfg(any(test, feature = "test-support"))]
pub mod tests;

/// Whole product replies have no series directory or event wrapper.
pub fn write_reply<W: super::pages::Pages + ?Sized>(
    writer: &mut W,
    section: &str,
    ordinal: u64,
    value: &impl Serialize,
) -> io::Result<()> {
    let json = serde_json::to_vec(value)?;
    let compressed = compress(json.as_slice(), 3)?;
    writer.write_at(section, ordinal, compressed.as_slice())
}
pub fn read_reply<T: serde::de::DeserializeOwned>(
    store: &store::Store,
    key: &store::SourceKey,
    section: &str,
    ordinal: u64,
) -> io::Result<Option<T>> {
    let Some(mut file) = store.read(key, section, ordinal)? else {
        return Ok(None);
    };
    let mut compressed = Vec::new();
    file.read_to_end(&mut compressed)?;
    let json = decompress(&compressed)?;
    Ok(Some(serde_json::from_slice(&json)?))
}
