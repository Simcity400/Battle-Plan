//! Wars replies from the family's publication ([`super::emit`]): each viewpoint's walk over the
//! change inputs, kept on the source so a later query walks only the inputs published since; each
//! war's casualties at the hours asked, from the counters of the relations it held then; and the
//! groups and contributions at an hour, from the tables of the inputs covering it, the counters
//! and the participation parts there.

use super::emit::{COUNTERS, COVERAGE, DIFFS, FAMILY, INPUT, WHOLES};
use super::inputs::{self, INPUTS, Input, PARTICIPATION};
use super::*;
use dashboard_base::prepared::published::Opened;
use dashboard_series::{
    Cell, Cursor, EventQuery, KeyHandle, Observation, PageRead, Selection, Snapshot, Span,
    family::Directory, query::EventView,
};
use replay_frame::frame::not_selected;
use serde_json::Value;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

/// Viewpoint walks kept per source, the most recently used first.
const CACHED_WALKS: usize = 4;

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// Change inputs decoded on this thread.
    static DECODED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Change inputs decoded on this thread.
#[cfg(any(test, feature = "test-support"))]
pub fn decoded_inputs() -> usize {
    DECODED.with(std::cell::Cell::get)
}

fn missing() -> io::Error {
    invalid("The wars publication is missing a key")
}

/// The family's keys a reply reads.
#[derive(Clone, Copy)]
struct Keys {
    input: KeyHandle,
    coverage: KeyHandle,
    wholes: KeyHandle,
    diffs: KeyHandle,
}

impl Keys {
    fn of(directory: &Directory) -> io::Result<Self> {
        let key = |measure| directory.key(FAMILY, measure, None).ok_or_else(missing);
        Ok(Self {
            input: key(INPUT)?,
            coverage: key(COVERAGE)?,
            wholes: key(WHOLES)?,
            diffs: key(DIFFS)?,
        })
    }
}

fn selection(key: KeyHandle) -> Selection<'static> {
    Selection {
        key,
        membership: None,
    }
}

/// The bytes of an event's one text cell.
fn event_bytes<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    view: EventView,
) -> io::Result<Vec<u8>> {
    let EventView::Family(event) = view else {
        return Err(invalid("A wars event is not the family's"));
    };
    let Cell::Text(blob) = snapshot.field(event.fields, 0)? else {
        return Err(invalid("A wars event holds no bytes"));
    };
    let len = usize::try_from(blob.len).map_err(|_| invalid("A wars event is too large"))?;
    let mut bytes = vec![0; len];
    snapshot.cell_bytes(blob, 0, &mut bytes)?;
    Ok(bytes)
}

/// The change inputs as a walk reads them.
struct Inputs<'s, 'p, P: PageRead> {
    snapshot: &'s mut Snapshot<'p, P>,
    keys: Keys,
}

impl<P: PageRead> Inputs<'_, '_, P> {
    /// The change input at `ordinal`: its hour and its event's bytes.
    fn raw(&mut self, ordinal: u64) -> io::Result<(i32, Vec<u8>)> {
        let view = self
            .snapshot
            .event_raw(selection(self.keys.input), None, ordinal)?
            .ok_or_else(|| invalid("A wars change input is missing"))?;
        let hour = match view {
            EventView::Family(event) => event.native_hour,
            EventView::Evidence { .. } => return Err(invalid("A wars input is not an event")),
        };
        Ok((hour, event_bytes(self.snapshot, view)?))
    }

    /// The inputs published through `hour`.
    fn count_through(&mut self, hour: i32) -> io::Result<u64> {
        self.snapshot
            .event_count_through(self.keys.input, hour, None)
    }

    fn date(&mut self, ordinal: u64) -> io::Result<Option<i32>> {
        self.snapshot
            .event_date(selection(self.keys.input), None, ordinal)
    }

    /// The hours the input covering `hour` covers, and its ordinal; none at an hour not walked.
    fn coverage(&mut self, hour: i32) -> io::Result<Option<(Span, u64)>> {
        Ok(match self.snapshot.value_at(self.keys.coverage, hour)? {
            Observation::Value { run, .. } => Some((
                run.span,
                u64::try_from(run.value)
                    .map_err(|_| invalid("A wars coverage is not an ordinal"))?,
            )),
            _ => None,
        })
    }

    /// The coverage of the input at `ordinal`, published at `hour`.
    fn span(&mut self, ordinal: u64, hour: i32) -> io::Result<Span> {
        self.coverage(hour)?
            .filter(|&(span, held)| held == ordinal && span.first == hour)
            .map(|(span, _)| span)
            .ok_or_else(|| invalid("Wars change coverage is missing"))
    }
}

/// A change input row's changed tables' indices and its decoded packet.
fn decode(hour: i32, branch: u64, row: &[u8]) -> io::Result<(Vec<usize>, DecodedFrame)> {
    let input = Input::decode(row)?;
    let packet = input.frame(branch, hour).map_err(|error| {
        io::Error::new(error.kind(), format!("Wars input at {hour}: {error}"))
    })?;
    Ok((
        input.changed.iter().map(|table| table.index()).collect(),
        packet,
    ))
}

struct Key {
    store: store::SourceKey,
    branch: u64,
    view: Option<String>,
    selected: Option<HashSet<(u32, u32)>>,
    /// Held, so its address names this catalogue while the walk is kept.
    catalogue: Option<Arc<Catalogue>>,
}

impl Key {
    fn same(&self, other: &Self) -> bool {
        self.store == other.store
            && self.branch == other.branch
            && self.view == other.view
            && self.selected == other.selected
            && match (&self.catalogue, &other.catalogue) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
    }
}

#[derive(Default)]
struct Walks(Vec<(Key, Arc<Mutex<Option<Resume>>>)>);

#[derive(Clone)]
struct Resume {
    /// The ordinal of the next change input; every earlier one is observed.
    next: u64,
    /// The hour of the last observed input. Its coverage, which still grows while it is the
    /// last published input, is extended into the walk just before the next input is observed.
    hour: Option<i32>,
    walk: Walk,
    decoded: Option<DecodedFrame>,
}

/// Inputs a walk's tail reads ahead of the one it takes: at least this many, and enough to keep
/// each of its workers busy.
const AHEAD: u64 = 16;
const AHEAD_PER_WORKER: u64 = 4;

/// A walk's work for its workers, by its input's ordinal.
enum Job {
    /// An input's row, published at its hour, to decode.
    Decode(u64, i32, Vec<u8>),
    /// The frame an input leaves the walk holding, whose hour's inputs to read.
    Read(u64, DecodedFrame),
    /// What the walk let go of (table views, a held hour), to free here rather than on the
    /// walking thread.
    Drop(Box<dyn Send>),
}

/// A worker's finished job, by its input's ordinal.
enum Done {
    Decoded(u64, i32, io::Result<(Vec<usize>, DecodedFrame)>),
    Read(u64, ReadyHour),
    /// The job panicked: the walk panics with it.
    Panicked(Box<dyn std::any::Any + Send>),
}

/// An input read ahead of the walk.
struct Slot {
    hour: Option<i32>,
    decoded: Option<io::Result<(Vec<usize>, DecodedFrame)>>,
    /// Its hour's inputs once asked for, None inside until they are read.
    read: Option<Option<ReadyHour>>,
}

/// The tail's inputs read in order here, decoded on other cores and put together as the walk
/// puts them together; each frame changing a table the walk compares has its hour's inputs read
/// on those cores too. Each input's decode failure is returned only when the walk takes that
/// input.
struct Ahead {
    /// The next input the walk takes, the first not yet put together, and the first not yet read.
    next: u64,
    composed: u64,
    read: u64,
    end: u64,
    window: u64,
    slots: VecDeque<Slot>,
    /// The frame the walk will hold once it takes the inputs before `composed`.
    frame: Option<DecodedFrame>,
    /// Whether each table is one the walk compares.
    compared: Vec<bool>,
    jobs: mpsc::Sender<Job>,
    done: mpsc::Receiver<Done>,
}

impl Ahead {
    /// Decodes inputs `next..end` ahead of a walk that holds `frame`, on threads of `scope` that
    /// end once this is dropped or `stopped` is set, an hour's inputs as `read` reads them.
    fn start<'s>(
        scope: &'s std::thread::Scope<'s, '_>,
        (next, end): (u64, u64),
        branch: u64,
        frame: Option<DecodedFrame>,
        stopped: &'s AtomicBool,
        read: &'s (dyn Fn(&DecodedFrame) -> ReadyHour + Sync),
    ) -> Self {
        let (jobs, work) = mpsc::channel::<Job>();
        let (finished, done) = mpsc::channel();
        let work = Arc::new(Mutex::new(work));
        let workers = std::thread::available_parallelism()
            .map_or(1, |cores| cores.get().saturating_sub(1))
            .min((end - next).div_ceil(AHEAD) as usize)
            .max(1);
        for _ in 0..workers {
            let (work, finished) = (Arc::clone(&work), finished.clone());
            scope.spawn(move || {
                loop {
                    let job = work
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .recv_timeout(Duration::from_millis(50));
                    let job = match job {
                        Ok(job) => job,
                        Err(mpsc::RecvTimeoutError::Timeout)
                            if !stopped.load(Ordering::Acquire) =>
                        {
                            continue;
                        }
                        Err(_) => return,
                    };
                    let job = match job {
                        Job::Drop(views) => {
                            drop(views);
                            continue;
                        }
                        job => job,
                    };
                    let outcome =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match job {
                            Job::Decode(ordinal, hour, row) => {
                                Done::Decoded(ordinal, hour, decode(hour, branch, &row))
                            }
                            Job::Read(ordinal, frame) => Done::Read(ordinal, read(&frame)),
                            Job::Drop(_) => unreachable!("views are dropped above"),
                        }));
                    let panicked = outcome.is_err();
                    if finished
                        .send(outcome.unwrap_or_else(Done::Panicked))
                        .is_err()
                        || panicked
                    {
                        return;
                    }
                }
            });
        }
        let mut compared = vec![false; TableId::ALL.len()];
        for table in compared_tables() {
            compared[table.index()] = true;
        }
        Self {
            next,
            composed: next,
            read: next,
            end,
            window: AHEAD.max(AHEAD_PER_WORKER * workers as u64),
            slots: VecDeque::new(),
            frame,
            compared,
            jobs,
            done,
        }
    }

    /// The input at `self.next` and its hour's inputs if they were read, after reading the rows
    /// up to the window beyond it with `raw`.
    fn take(
        &mut self,
        raw: &mut dyn FnMut(u64) -> io::Result<(i32, Vec<u8>)>,
    ) -> io::Result<(i32, Vec<usize>, DecodedFrame, Option<ReadyHour>)> {
        while self.read < self.end && self.read <= self.next + self.window {
            let (hour, row) = raw(self.read)?;
            self.jobs
                .send(Job::Decode(self.read, hour, row))
                .map_err(|_| io::Error::other("Wars input decoding stopped"))?;
            self.slots.push_back(Slot {
                hour: None,
                decoded: None,
                read: None,
            });
            self.read += 1;
        }
        loop {
            self.compose();
            let slot = self
                .slots
                .front()
                .ok_or_else(|| invalid("Wars inputs ran out ahead of the walk"))?;
            if slot.decoded.is_some() && !matches!(slot.read, Some(None)) {
                break;
            }
            match self.done.recv() {
                Ok(Done::Decoded(ordinal, hour, decoded)) => {
                    let slot = &mut self.slots[(ordinal - self.next) as usize];
                    slot.hour = Some(hour);
                    slot.decoded = Some(decoded);
                }
                Ok(Done::Read(ordinal, read)) => {
                    self.slots[(ordinal - self.next) as usize].read = Some(Some(read));
                }
                Ok(Done::Panicked(panic)) => std::panic::resume_unwind(panic),
                Err(_) => return Err(io::Error::other("Wars input decoding stopped")),
            }
        }
        let slot = self.slots.pop_front().expect("the input was read");
        self.next += 1;
        let hour = slot.hour.expect("the input's row was read");
        let (changed, packet) = slot.decoded.expect("the input was decoded")?;
        Ok((hour, changed, packet, slot.read.flatten()))
    }

    /// Puts the decoded inputs together in order, as [`Resume::step`] does, and asks for the
    /// hour's inputs of each frame changing a compared table, or of the walk's first frame.
    fn compose(&mut self) {
        while let Some(slot) = self.slots.get_mut((self.composed - self.next) as usize) {
            let Some(Ok((changed, packet))) = &slot.decoded else {
                return;
            };
            let mut read = self.frame.is_none();
            let mut frame = self.frame.take().unwrap_or_else(|| packet.clone());
            frame.branch = packet.branch;
            frame.hour = packet.hour;
            for &index in changed {
                frame.tables[index] = packet.tables[index].clone();
                read |= self.compared[index];
            }
            if read {
                let job = Job::Read(self.composed, frame.clone());
                if self.jobs.send(job).is_ok() {
                    slot.read = Some(None);
                }
            }
            self.frame = Some(frame);
            self.composed += 1;
        }
    }
}

/// What a walk reads its inputs with, and how.
struct Walker<'r, 's, 'p, P: PageRead> {
    inputs: &'r mut Inputs<'s, 'p, P>,
    branch: u64,
    view: Option<&'r str>,
    selected: Option<&'r HashSet<(u32, u32)>>,
    cat: Option<&'r Catalogue>,
    cancelled: &'r AtomicBool,
    /// Inputs decoded ahead of the walk while it steps its tail.
    ahead: Option<Ahead>,
}

impl<P: PageRead> Walker<'_, '_, '_, P> {
    fn input(
        &mut self,
        ordinal: u64,
    ) -> io::Result<(i32, Vec<usize>, DecodedFrame, Option<ReadyHour>)> {
        let inputs = &mut *self.inputs;
        if let Some(ahead) = self.ahead.as_mut().filter(|ahead| ahead.next == ordinal) {
            return ahead.take(&mut |ordinal| inputs.raw(ordinal));
        }
        let (hour, row) = inputs.raw(ordinal)?;
        let (changed, packet) = decode(hour, self.branch, &row)?;
        Ok((hour, changed, packet, None))
    }

    /// Frees `released` on a worker when the walk reads ahead, else here.
    fn discard(&self, released: impl Send + 'static) {
        if let Some(ahead) = &self.ahead {
            let _ = ahead.jobs.send(Job::Drop(Box::new(released)));
        }
    }
}

impl Resume {
    fn start(branch: u64, first: i32) -> Self {
        Self {
            next: 0,
            hour: None,
            walk: Walk {
                losses: false,
                ..Walk::new(branch, first)
            },
            decoded: None,
        }
    }

    /// Observes the next change input and returns its coverage. `known` is the last observed
    /// input's coverage when this query already read it.
    fn step<P: PageRead>(
        &mut self,
        walker: &mut Walker<'_, '_, '_, P>,
        known: Option<Span>,
    ) -> io::Result<Span> {
        dashboard_text::metadata::check_cancel(walker.cancelled)?;
        let ordinal = self.next;
        let (hour, changed, packet, read) = walker.input(ordinal)?;
        #[cfg(any(test, feature = "test-support"))]
        DECODED.with(|count| count.set(count.get() + 1));
        let before = match (self.hour, known) {
            (Some(_), Some(known)) => Some(known),
            (Some(previous), None) => Some(walker.inputs.span(ordinal - 1, previous)?),
            (None, _) => None,
        };
        let span = walker.inputs.span(ordinal, hour)?;
        // Every read succeeded: the input is applied whole or not at all.
        if let Some(before) = before {
            self.walk.extend(before.last);
        }
        let mut current = self.decoded.take().unwrap_or_else(|| packet.clone());
        current.branch = packet.branch;
        current.hour = packet.hour;
        let mut replaced = Vec::with_capacity(changed.len());
        for index in changed {
            replaced.push(std::mem::replace(
                &mut current.tables[index],
                packet.tables[index].clone(),
            ));
        }
        let frame = self.decoded.insert(current);
        let released = self
            .walk
            .observe(frame, walker.view, walker.selected, walker.cat, read);
        walker.discard((replaced, released));
        self.next += 1;
        self.hour = Some(hour);
        Ok(span)
    }

    /// Whether the publication still holds every input this walk observed: one only appends,
    /// and a rebuilt one that disagrees starts the walk again.
    fn holds<P: PageRead>(&self, inputs: &mut Inputs<'_, '_, P>, end: u64) -> io::Result<bool> {
        if self.next > end {
            return Ok(false);
        }
        match self.hour {
            Some(hour) => Ok(inputs.date(self.next - 1)? == Some(hour)),
            None => Ok(true),
        }
    }
}

fn cached(source: &Source, key: Key) -> Arc<Mutex<Option<Resume>>> {
    let state = source.state::<Walks>();
    let mut walks = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(at) = walks.0.iter().position(|(held, _)| held.same(&key)) {
        walks.0[..=at].rotate_right(1);
    } else {
        walks.0.insert(0, (key, Arc::new(Mutex::new(None))));
        walks.0.truncate(CACHED_WALKS);
    }
    Arc::clone(&walks.0[0].1)
}

/// What a walk of one branch reads: its source and publication, and its viewpoint.
pub(crate) struct Walked<'a> {
    pub source: &'a Source,
    pub store: &'a store::SourceKey,
    pub branch: u64,
    pub bounds: (i32, i32),
    pub view: Option<&'a str>,
    pub selected: Option<&'a HashSet<(u32, u32)>>,
    pub cat: Option<&'a Arc<Catalogue>>,
    pub cancelled: &'a AtomicBool,
}

/// The viewpoint's whole-branch walk, continued from its kept state through every published
/// change input.
fn walk<P: PageRead>(walked: &Walked<'_>, inputs: &mut Inputs<'_, '_, P>) -> io::Result<WarsReply> {
    let (first, last) = walked.bounds;
    if first > last {
        return Err(invalid("Wars change inputs: invalid branch"));
    }
    let end = inputs.count_through(last)?;
    let shared = cached(
        walked.source,
        Key {
            store: walked.store.clone(),
            branch: walked.branch,
            view: walked.view.map(str::to_owned),
            selected: walked.selected.cloned(),
            catalogue: walked.cat.cloned(),
        },
    );
    let mut guard = match shared.lock() {
        Ok(guard) => guard,
        // A walk that panicked part way is started again.
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            *guard = None;
            shared.clear_poison();
            guard
        }
    };
    let holds = match guard.as_ref() {
        Some(entry) => entry.holds(inputs, end)?,
        None => false,
    };
    if !holds {
        *guard = Some(Resume::start(walked.branch, first));
    }
    let entry = guard.as_mut().expect("kept walk");
    let (view, selected) = (walked.view, walked.selected);
    let catalogue = walked.cat.map(|cat| &**cat);
    let read = move |frame: &DecodedFrame| read_ahead(frame, view, selected, catalogue, false);
    let mut walker = Walker {
        inputs,
        branch: walked.branch,
        view,
        selected,
        cat: catalogue,
        cancelled: walked.cancelled,
        ahead: None,
    };
    let stopped = AtomicBool::new(false);
    let mut known = None;
    std::thread::scope(|scope| {
        // Ends the workers before the scope joins them, however the walk ends.
        struct Stop<'s>(&'s AtomicBool);
        impl Drop for Stop<'_> {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let _stop = Stop(&stopped);
        walker.ahead = (entry.next < end).then(|| {
            let frame = entry.decoded.clone();
            Ahead::start(scope, (entry.next, end), walked.branch, frame, &stopped, &read)
        });
        let walking = (|| {
            while entry.next < end {
                known = Some(entry.step(&mut walker, known)?);
            }
            io::Result::Ok(())
        })();
        walker.ahead = None;
        walking
    })?;
    let mut walk = entry.walk.clone();
    if let Some(hour) = entry.hour {
        let span = match known {
            Some(span) => span,
            None => walker.inputs.span(entry.next - 1, hour)?,
        };
        walk.extend(span.last);
    }
    drop(guard);
    walk.finish(last, catalogue)
}

/// Adds one counter's run to a war's hour: a known counter to its side's sum, an unknown one to
/// its country's maximum, as the walk's losses count them.
fn accumulate(
    sums: &mut (i32, i32, [i128; 4]),
    unknown: &mut BTreeMap<(usize, u32), i128>,
    (output, country): (usize, u32),
    run: dashboard_series::Run,
) {
    let (first, last, values) = sums;
    if output % 2 == 0 {
        values[output] += run.value;
    } else {
        let held = unknown.entry((output, country)).or_default();
        *held = (*held).max(run.value);
    }
    *first = (*first).max(run.span.first);
    *last = (*last).min(run.span.last);
}

/// Each war's casualties at `hours`, keyed by its place in `wars`: those of the wars observed at
/// an hour, summed over the relations they held then ([`super::WarRelationMembership`]), each
/// run bounded by the change input covering the hour and by every counter's run.
fn casualties_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    keys: Keys,
    wars: &[WarTimeline],
    hours: &BTreeSet<i32>,
) -> io::Result<HashMap<usize, Vec<WarCasualtyRun>>> {
    let _cpu = replay_frame::cpu::scope_phase("wars:query_losses");
    let accepted = snapshot.publication().accepted;
    let mut counters: HashMap<WarRelationKey, [KeyHandle; 4]> = HashMap::new();
    let mut casualties: HashMap<usize, Vec<WarCasualtyRun>> = HashMap::new();
    for &hour in hours {
        if !accepted.contains(hour) {
            continue;
        }
        let Observation::Value { run: span, .. } = snapshot.value_at(keys.coverage, hour)? else {
            continue;
        };
        for (id, war) in wars.iter().enumerate() {
            let at = war.observations.partition_point(|run| run.last_hour < hour);
            if war
                .observations
                .get(at)
                .is_none_or(|run| run.first_hour > hour)
            {
                continue;
            }
            let mut sums = (span.span.first, span.span.last, [0_i128; 4]);
            let mut unknown = BTreeMap::new();
            for membership in war
                .relations
                .iter()
                .filter(|run| run.first_hour <= hour && hour <= run.last_hour)
            {
                let held = match counters.get(&membership.key) {
                    Some(&held) => held,
                    None => {
                        let actor = directory
                            .actor(&inputs::relation_name(&membership.key))
                            .ok_or_else(|| invalid("Missing prepared war relation lifetime"))?;
                        let mut held = [0; 4];
                        for (field, key) in held.iter_mut().enumerate() {
                            *key = directory
                                .key(actor, COUNTERS + field as u64, None)
                                .ok_or_else(missing)?;
                        }
                        counters.insert(membership.key.clone(), held);
                        held
                    }
                };
                let order = if membership.oriented {
                    [0, 1, 2, 3]
                } else {
                    [2, 3, 0, 1]
                };
                let countries = [membership.key.first.0, membership.key.second.0];
                for (output, field) in order.into_iter().enumerate() {
                    let Observation::Value { run, .. } = snapshot.value_at(held[field], hour)? else {
                        return Err(invalid("Missing prepared war casualty value"));
                    };
                    accumulate(
                        &mut sums,
                        &mut unknown,
                        (output, countries[field / 2]),
                        run,
                    );
                }
            }
            for ((output, _), value) in unknown {
                sums.2[output] += value;
            }
            let (first, last, values) = sums;
            let [a, b, c, d] = values.map(|value| value.to_string());
            casualties
                .entry(id)
                .or_default()
                .push(WarCasualtyRun(first, last, a, b, c, d));
        }
    }
    Ok(casualties)
}

/// Each war's casualty runs put in hour order, adjacent equal runs merged.
fn place_casualties(reply: &mut WarsReply, mut casualties: HashMap<usize, Vec<WarCasualtyRun>>) {
    for (index, war) in reply.wars.iter_mut().enumerate() {
        war.casualties.clear();
        let mut runs = casualties.remove(&index).unwrap_or_default();
        runs.sort_by_key(|run| run.0);
        for run in runs {
            if let Some(last) = war.casualties.last_mut()
                && i64::from(run.0) <= i64::from(last.1) + 1
                && (&run.2, &run.3, &run.4, &run.5) == (&last.2, &last.3, &last.4, &last.5)
            {
                last.1 = last.1.max(run.1);
            } else {
                war.casualties.push(run);
            }
        }
    }
}

/// The whole-branch projection `walked` reads from `opened`, with each war's casualties at
/// `hours`; an empty set returns structure. None before the family publishes the branch.
pub(crate) fn projection(
    walked: &Walked<'_>,
    opened: Option<&Opened>,
    hours: &BTreeSet<i32>,
) -> io::Result<Option<WarsReply>> {
    let Some(opened) = opened else {
        return Ok(None);
    };
    opened.read(walked.cancelled, |snapshot, directory| {
        let keys = Keys::of(directory)?;
        let cpu = replay_frame::cpu::scope_phase("wars:query_walk");
        let mut reply = walk(
            walked,
            &mut Inputs {
                snapshot: &mut *snapshot,
                keys,
            },
        )?;
        drop(cpu);
        if !hours.is_empty() {
            let casualties = casualties_at(snapshot, directory, keys, &reply.wars, hours)?;
            place_casualties(&mut reply, casualties);
        }
        check_reply(&reply)?;
        Ok(Some(reply))
    })
}

/// The tables at `hour` of the input covering it, `ordinal`: each input table's view from the
/// last input publishing it, the relations' counters, and the participation parts there.
fn frame_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    keys: Keys,
    branch: u64,
    (hour, ordinal): (i32, u64),
) -> io::Result<DecodedFrame> {
    let mut reader = Inputs { snapshot, keys };
    let (published, row) = reader.raw(ordinal)?;
    let covering = Input::decode(&row)?;
    let mut frame = DecodedFrame {
        branch,
        hour,
        metrics: Default::default(),
        tables: std::array::from_fn(|_| not_selected()),
    };
    let mut decoded: HashMap<u32, DecodedFrame> = HashMap::new();
    decoded.insert(ordinal as u32, covering.frame(branch, published)?);
    for (&table, &held) in INPUTS.iter().zip(&covering.refs) {
        if held == u32::MAX {
            continue;
        }
        if let std::collections::hash_map::Entry::Vacant(entry) = decoded.entry(held) {
            let (at, row) = reader.raw(u64::from(held))?;
            entry.insert(Input::decode(&row)?.frame(branch, at)?);
        }
        frame.tables[table.index()] = decoded[&held].tables[table.index()].clone();
    }
    // The relations' counters at the hour.
    if let Some(lifetimes) = inputs::lifetimes(&frame) {
        let relations = &mut frame.tables[TableId::WarRelations.index()];
        let mut restored = Vec::with_capacity(lifetimes.len());
        let mut counted = true;
        for lifetime in &lifetimes {
            let Some((key, _)) = lifetime else {
                restored.push(None);
                continue;
            };
            let Some(actor) = directory.actor(&inputs::relation_name(key)) else {
                counted = false;
                break;
            };
            let mut held = [0_i64; 4];
            for (field, value) in held.iter_mut().enumerate() {
                let key = directory
                    .key(actor, COUNTERS + field as u64, None)
                    .ok_or_else(missing)?;
                let Observation::Value { run, .. } = reader.snapshot.value_at(key, hour)? else {
                    counted = false;
                    break;
                };
                *value = i64::try_from(run.value)
                    .map_err(|_| invalid("A wars casualty counter is out of range"))?;
            }
            if !counted {
                break;
            }
            restored.push(Some(held));
        }
        if counted {
            if let TableView::Complete(rows::TableRows::WarRelations(rows)) = &mut **relations {
                for ((row, lifetime), held) in rows.iter_mut().zip(&lifetimes).zip(restored) {
                    if let (Some((_, reversed)), Some(held)) = (lifetime, held) {
                        inputs::restore_counters(row, held, *reversed);
                    }
                }
            }
        } else {
            // Never zero for counters the walk did not hold.
            *relations = TableView::Unavailable {
                reason: "Not recorded: war casualty counters at this hour".into(),
            }
            .into();
        }
    }
    // The participation parts: the last whole ones, then each hour's diff since.
    let snapshot = reader.snapshot;
    if let Some(whole) = snapshot.latest_event(selection(keys.wholes), hour, None)? {
        let EventView::Family(event) = whole else {
            return Err(invalid("A wars event is not the family's"));
        };
        let mut held = [None, None];
        inputs::fold(&mut held, inputs::unpack_parts(&event_bytes(snapshot, whole)?)?)?;
        if event.native_hour < hour {
            let mut diffs = Vec::new();
            let selections = [selection(keys.diffs)];
            let mut cursor: Option<Cursor> = None;
            loop {
                let page = snapshot.event_page(
                    EventQuery {
                        selection: &selections,
                        native: Span::new(event.native_hour + 1, hour)?,
                        as_of: None,
                        order: dashboard_series::EventOrder::Native,
                    },
                    cursor,
                    1 << 20,
                    |view| {
                        diffs.push(view);
                        Ok(())
                    },
                )?;
                cursor = page.next;
                if cursor.is_none() {
                    break;
                }
            }
            for view in diffs {
                inputs::fold(&mut held, inputs::unpack_parts(&event_bytes(snapshot, view)?)?)?;
            }
        }
        for (table, folded) in PARTICIPATION.iter().zip(&held) {
            if let Some(view) = inputs::folded_view(folded) {
                frame.tables[table.index()] = view;
            }
        }
    }
    Ok(frame)
}

/// The groups and contributions at each requested hour within what the publication accepted,
/// each from the tables there ([`super::at_hour`]); an hour outside the branch or not walked is
/// unavailable, and one past what was accepted is left out, still being prepared.
fn current<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    walked: &Walked<'_>,
    hours: &BTreeSet<i32>,
) -> io::Result<Vec<WarsAtHour>> {
    let keys = Keys::of(directory)?;
    let accepted = snapshot.publication().accepted;
    let (first, last) = walked.bounds;
    let mut current = Vec::with_capacity(hours.len());
    for &hour in hours {
        dashboard_text::metadata::check_cancel(walked.cancelled)?;
        if hour > accepted.last {
            continue;
        }
        let covering = if (first..=last).contains(&hour) {
            Inputs {
                snapshot: &mut *snapshot,
                keys,
            }
            .coverage(hour)?
        } else {
            None
        };
        current.push(match covering {
            Some((_, ordinal)) => {
                let frame = frame_at(snapshot, directory, keys, walked.branch, (hour, ordinal))?;
                super::at_hour(
                    walked.branch,
                    &frame,
                    walked.view,
                    walked.cat.map(|cat| &**cat),
                )?
            }
            None => WarsAtHour {
                hour,
                contributions: Vec::new(),
                available: false,
                wars: Vec::new(),
                warnings: Vec::new(),
            },
        });
    }
    Ok(current)
}

#[derive(Deserialize)]
struct Reached {
    hour: i32,
}

#[derive(Deserialize)]
struct Requested {
    /// Hours whose casualties the walk hydrates.
    #[serde(default)]
    reads: Vec<Reached>,
    /// Hours whose groups and contributions are read ([`current`]).
    #[serde(default)]
    current: Vec<Reached>,
}

/// The branch and its bounds a reply of `params` reads.
fn bounds(source: &Source, params: &WarsParams) -> io::Result<(u64, (i32, i32))> {
    Wars::validate(params).map_err(invalid)?;
    let branch = params.branch_id.parse().expect("validated branch");
    let start = source
        .replay_ref()
        .branches
        .iter()
        .find(|start| start.id == branch)
        .ok_or_else(|| invalid("Wars branch is not in this recording"))?;
    Ok((branch, branch_bounds(source.replay_ref(), start)))
}

/// The wars reply to `value` from `opened`, the family's publication on its branch (none before
/// its first): the viewpoint's wars, their casualties at the `reads` hours and, when asked, the
/// groups at the `current` hours.
pub fn query(
    source: &Source,
    value: &Value,
    opened: Option<&Opened>,
    store: &store::SourceKey,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let params: WarsParams = serde_json::from_value(
        serde_json::json!({"branchId":value["branchId"],"viewpointTag":value["viewpointTag"]}),
    )?;
    let (branch, bounds) = bounds(source, &params)?;
    let cat = context
        .content
        .as_ref()
        .map(|options| catalogue::catalogue(source, options))
        .transpose()?;
    let requested: Requested = serde_json::from_value(value.clone())?;
    let reads: BTreeSet<i32> = requested.reads.iter().map(|read| read.hour).collect();
    let asked: BTreeSet<i32> = requested.current.iter().map(|read| read.hour).collect();
    let walked = Walked {
        source,
        store,
        branch,
        bounds,
        view: params.viewpoint_tag.as_deref(),
        selected: None,
        cat: cat.as_ref(),
        cancelled,
    };
    let reply = projection(&walked, opened, &reads)?.unwrap_or_else(|| WarsReply {
        branch_id: params.branch_id.clone(),
        available: false,
        viewpoints: Vec::new(),
        country_names: Vec::new(),
        wars: Vec::new(),
        unobserved: Vec::new(),
        capitulations_unobserved: Vec::new(),
        warnings: Vec::new(),
    });
    let mut data = serde_json::to_value(reply)?;
    if let Some(opened) = opened
        && !asked.is_empty()
    {
        let current = opened.read(cancelled, |snapshot, directory| {
            current(snapshot, directory, &walked, &asked)
        })?;
        if !current.is_empty()
            && let Some(reply) = data.as_object_mut()
        {
            reply.insert("current".into(), serde_json::to_value(current)?);
        }
    }
    Ok(data)
}

/// The structure the casualty breakdown reads: the wars of the native holder among `selected`,
/// read as `tag`'s, from the family's publication on the branch; None before its first.
pub fn selected_structure(
    source: &Source,
    params: &WarsParams,
    selected: &HashSet<(u32, u32)>,
    opened: Option<&Opened>,
    store: &store::SourceKey,
    cat: Option<&Arc<Catalogue>>,
    cancelled: &AtomicBool,
) -> io::Result<Option<WarsReply>> {
    let (branch, bounds) = bounds(source, params)?;
    projection(
        &Walked {
            source,
            store,
            branch,
            bounds,
            view: params.viewpoint_tag.as_deref(),
            selected: Some(selected),
            cat,
            cancelled,
        },
        opened,
        &BTreeSet::new(),
    )
}

/// The replies a walk of `snapshot`, a publication of the family, gives `params` and `reads`,
/// for tests holding it to the row walk.
#[cfg(test)]
pub(crate) fn walk_snapshot<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    walked: &Walked<'_>,
    reads: &BTreeSet<i32>,
    asked: &BTreeSet<i32>,
) -> io::Result<(WarsReply, Vec<WarsAtHour>)> {
    let keys = Keys::of(directory)?;
    let mut reply = walk(
        walked,
        &mut Inputs {
            snapshot: &mut *snapshot,
            keys,
        },
    )?;
    if !reads.is_empty() {
        let casualties = casualties_at(snapshot, directory, keys, &reply.wars, reads)?;
        place_casualties(&mut reply, casualties);
    }
    let current = current(snapshot, directory, walked, asked)?;
    Ok((reply, current))
}
