//! Each requested viewpoint's walk over the prepared change inputs, kept so a later query walks
//! only the inputs published since.
use super::*;
use dashboard_base::prepared::series::{self, Reader};
use replay_data::codec::Decoder;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

/// Viewpoint walks kept per source, the most recently used first.
pub(super) const CACHED_WALKS: usize = 4;
/// Nominal bytes of one held row, run, name or key in the estimate.
#[cfg(test)]
const NOMINAL_ROW_BYTES: usize = 64;

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// Change inputs decoded on this thread.
    pub(super) static DECODED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Hours whose inputs were read on this thread.
    pub(super) static READ: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// One change input row: the number of changed tables and their numbers as
/// little-endian u16s, then the compressed checkpoint packet.
pub(super) fn encode_input(changes: &[u16], packet: &[u8]) -> io::Result<Vec<u8>> {
    let count =
        u16::try_from(changes.len()).map_err(|_| invalid("Too many changed wars input tables"))?;
    let mut row = Vec::with_capacity(2 + 2 * changes.len() + packet.len());
    row.extend_from_slice(&count.to_le_bytes());
    for table in changes {
        row.extend_from_slice(&table.to_le_bytes());
    }
    row.extend_from_slice(packet);
    Ok(row)
}
pub(super) fn decode_input(row: &[u8]) -> io::Result<(Vec<u16>, &[u8])> {
    let malformed = || invalid("Malformed wars change input");
    let (count, rest) = row.split_first_chunk::<2>().ok_or_else(malformed)?;
    let count = usize::from(u16::from_le_bytes(*count));
    let (tables, packet) = rest.split_at_checked(2 * count).ok_or_else(malformed)?;
    Ok((
        tables
            .chunks_exact(2)
            .map(|table| u16::from_le_bytes([table[0], table[1]]))
            .collect(),
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
struct Walks(Vec<(Key, Arc<Mutex<Option<Entry>>>)>);

#[derive(Clone)]
struct Resume {
    /// The ordinal of the next change input; every earlier one is observed.
    next: u64,
    /// The hour of the last observed input. Its coverage, which still grows
    /// while it is the last published input, is extended into the walk just
    /// before the next input is observed.
    hour: Option<i32>,
    walk: Walk,
    decoded: Option<DecodedFrame>,
}

struct Entry {
    base: u64,
    /// After every input walked so far.
    tail: Resume,
}

/// Opens, on the thread reading a walk's change input rows, its own reader of them in a scope
/// of its own.
type Rows<'a> = dyn Fn() -> io::Result<(store::Scope, Reader<'a>)> + Sync + 'a;

struct Walker<'r, 'a> {
    topology: &'r mut Reader<'a>,
    base: u64,
    view: Option<&'r str>,
    selected: Option<&'r HashSet<(u32, u32)>>,
    cat: Option<&'r Catalogue>,
    cancelled: &'r AtomicBool,
    /// Inputs decoded ahead of the walk while it steps its tail.
    ahead: Option<Ahead>,
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
    /// An input's hour and its decoded row.
    Decoded(u64, i32, io::Result<(Vec<usize>, DecodedFrame)>),
    /// Why an input's row could not be read: the walk stops at it.
    Unread(u64, io::Error),
    Read(u64, ReadyHour),
    /// The job panicked: the walk panics with it.
    Panicked(Box<dyn std::any::Any + Send>),
}

/// An input read ahead of the walk.
struct Slot {
    /// Its hour, or why its row could not be read, once its row is read.
    hour: Option<io::Result<i32>>,
    decoded: Option<io::Result<(Vec<usize>, DecodedFrame)>>,
    /// Its hour's inputs once asked for, None inside until they are read.
    read: Option<Option<ReadyHour>>,
}

/// The tail's inputs read in order on a thread of their own, decoded on other cores and put
/// together here as the walk puts them together; each frame changing a table the walk compares
/// has its hour's inputs read on those cores too. Each input's read or decode failure is
/// returned only when the walk takes that input.
struct Ahead {
    /// The next input the walk takes, the first not yet put together, and the first not yet read.
    next: u64,
    composed: u64,
    read: u64,
    end: u64,
    /// Inputs `read` ahead of `next`, at most this many.
    window: u64,
    /// The inputs from `next` not yet taken.
    slots: VecDeque<Slot>,
    /// The frame the walk will hold once it takes the inputs before `composed`.
    frame: Option<DecodedFrame>,
    /// Whether each table is one the walk compares.
    compared: Vec<bool>,
    /// The inputs whose rows to read, in order.
    rows: mpsc::Sender<u64>,
    jobs: mpsc::Sender<Job>,
    done: mpsc::Receiver<Done>,
}

impl Ahead {
    /// Reads inputs `next..end` ahead of a walk that holds `frame`, on threads of `scope` that
    /// end once this is dropped or `stopped` is set: their rows through `rows`, an hour's inputs
    /// as `read` reads them.
    fn start<'s, 'a: 's>(
        scope: &'s std::thread::Scope<'s, '_>,
        (next, end): (u64, u64),
        frame: Option<DecodedFrame>,
        stopped: &'s AtomicBool,
        read: &'s (dyn Fn(&DecodedFrame) -> ReadyHour + Sync),
        rows: &'s Rows<'a>,
    ) -> Self {
        let (jobs, work) = mpsc::channel::<Job>();
        let (finished, done) = mpsc::channel();
        let (wanted, ordinals) = mpsc::channel::<u64>();
        {
            let (jobs, finished) = (jobs.clone(), finished.clone());
            scope.spawn(move || {
                // A panic reading rows reaches the walk, as a worker's does.
                let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut reader = rows();
                    loop {
                        let ordinal = match ordinals.recv_timeout(Duration::from_millis(50)) {
                            Ok(ordinal) => ordinal,
                            Err(mpsc::RecvTimeoutError::Timeout)
                                if !stopped.load(Ordering::Acquire) =>
                            {
                                continue;
                            }
                            Err(_) => return,
                        };
                        let row = match &mut reader {
                            Ok((_, reader)) => raw(reader, ordinal),
                            Err(error) => Err(io::Error::new(error.kind(), error.to_string())),
                        };
                        let sent = match row {
                            Ok((hour, row)) => jobs.send(Job::Decode(ordinal, hour, row)).is_ok(),
                            Err(error) => finished.send(Done::Unread(ordinal, error)).is_ok(),
                        };
                        if !sent {
                            return;
                        }
                    }
                }));
                if let Err(panic) = read {
                    let _ = finished.send(Done::Panicked(panic));
                }
            });
        }
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
                                Done::Decoded(ordinal, hour, decode(hour, &row))
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
            rows: wanted,
            jobs,
            done,
        }
    }

    /// The input at `self.next` and its hour's inputs if they were read, after asking for the
    /// rows up to the window beyond it.
    fn take(&mut self) -> io::Result<(i32, Vec<usize>, DecodedFrame, Option<ReadyHour>)> {
        while self.read < self.end && self.read <= self.next + self.window {
            let hour = self
                .rows
                .send(self.read)
                .is_err()
                .then(|| Err(io::Error::other("Wars input reading stopped")));
            self.slots.push_back(Slot {
                hour,
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
            if matches!(slot.hour, Some(Err(_)))
                || (slot.decoded.is_some() && !matches!(slot.read, Some(None)))
            {
                break;
            }
            match self.done.recv() {
                Ok(Done::Decoded(ordinal, hour, decoded)) => {
                    let slot = &mut self.slots[(ordinal - self.next) as usize];
                    slot.hour = Some(Ok(hour));
                    slot.decoded = Some(decoded);
                }
                Ok(Done::Unread(ordinal, error)) => {
                    // The walk stops at this input: nothing after it is read.
                    self.end = self.end.min(ordinal + 1);
                    self.slots[(ordinal - self.next) as usize].hour = Some(Err(error));
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
        let hour = slot.hour.expect("the input's row was read")?;
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

/// The change input row at `ordinal` and the hour it was published.
fn raw(topology: &mut Reader<'_>, ordinal: u64) -> io::Result<(i32, Vec<u8>)> {
    topology
        .event_raw(0, ordinal)
        .map_err(|error| io::Error::new(error.kind(), format!("Wars change inputs: {error}")))
}

/// A change input row's changed tables' indices and its decoded packet.
fn decode(hour: i32, row: &[u8]) -> io::Result<(Vec<usize>, DecodedFrame)> {
    let (changes, packet) = decode_input(row)?;
    let bytes = series::decompress(packet)
        .map_err(|error| io::Error::new(error.kind(), format!("Wars input at {hour}: {error}")))?;
    let mut input = Decoder::new(&bytes);
    let packet = DecodedFrame::decode_checkpoint(&mut input)?;
    input.finish()?;
    let changed = changes
        .iter()
        .map(|&table| {
            TableId::ALL
                .iter()
                .position(|id| *id as u16 == table)
                .ok_or_else(|| invalid("Unknown wars input table"))
        })
        .collect::<io::Result<Vec<_>>>()?;
    Ok((changed, packet))
}
impl Walker<'_, '_> {
    /// The coverage of the input at `ordinal`, published at `hour`.
    fn span(&mut self, ordinal: u64, hour: i32) -> io::Result<series::Run> {
        self.topology
            .value_at(0, hour)
            .map_err(|error| {
                io::Error::new(error.kind(), format!("Wars coverage at {hour}: {error}"))
            })?
            .filter(|span| span.value == i128::from(ordinal - self.base) && span.first_hour == hour)
            .ok_or_else(|| invalid("Wars change coverage is missing"))
    }
    /// The change input at `ordinal`: its hour, its changed tables' indices, its decoded packet
    /// and, when they were read ahead, the inputs of the hour it leaves the walk holding.
    fn input(
        &mut self,
        ordinal: u64,
    ) -> io::Result<(i32, Vec<usize>, DecodedFrame, Option<ReadyHour>)> {
        if let Some(ahead) = self.ahead.as_mut().filter(|ahead| ahead.next == ordinal) {
            return ahead.take();
        }
        let (hour, row) = raw(self.topology, ordinal)?;
        let (changed, packet) = decode(hour, &row)?;
        Ok((hour, changed, packet, None))
    }
    /// Frees `released` on a worker when the walk reads ahead, else here.
    fn discard(&self, released: impl Send + 'static) {
        if let Some(ahead) = &self.ahead {
            let _ = ahead.jobs.send(Job::Drop(Box::new(released)));
        }
    }
}

/// Estimated bytes held by `walk`.
#[cfg(test)]
fn walk_bytes(walk: &Walk) -> usize {
    let reply = &walk.reply;
    let mut count = reply.viewpoints.len()
        + reply.unobserved.len()
        + reply.capitulations_unobserved.len()
        + reply.wars.len()
        + walk
            .open
            .iter()
            .map(|war| 1 + war.relations.len() + war.loss_pairs.len())
            .sum::<usize>()
        + walk.seen_caps.len()
        + walk.dated_caps.values().map(Vec::len).sum::<usize>()
        + walk.identity_since.len()
        + walk.warnings.len()
        + walk.country_labels.entries();
    for war in &reply.wars {
        count += war.observations.len()
            + war.casualties.len()
            + war.participants.len()
            + war.capitulations.len()
            + war.relations.len()
            + war.open_participants.len()
            + war.open_relations.len();
    }
    if let Some(Ok((inputs, observed))) = walk.held.as_ref().map(|held| &held.observed) {
        let world = &inputs.world;
        count += world.relations.len()
            + world.owned.values().map(Vec::len).sum::<usize>()
            + world.adjacent.values().map(Vec::len).sum::<usize>()
            + world.factions.len()
            + world.scores.len()
            + inputs.relation_order.len()
            + inputs.tags.len()
            + inputs.identities.len()
            + inputs.raw_to_slot.len()
            + inputs.goal_owned.len();
        for observation in observed {
            count += 1
                + observation.relations.len()
                + observation.loss_pairs.len()
                + observation.data.viewpoint_side.len()
                + observation.data.opposing_side.len();
        }
    }
    count * NOMINAL_ROW_BYTES
}

impl Resume {
    fn start(branch: u64, first: i32, base: u64) -> Self {
        Self {
            next: base,
            hour: None,
            walk: Walk {
                losses: false,
                ..Walk::new(branch, first)
            },
            decoded: None,
        }
    }
    /// Observe the next change input and return its coverage. `known` is the
    /// last observed input's coverage when this query already read it.
    fn step(
        &mut self,
        walker: &mut Walker<'_, '_>,
        known: Option<series::Run>,
    ) -> io::Result<series::Run> {
        dashboard_text::metadata::check_cancel(walker.cancelled)?;
        let ordinal = self.next;
        let (hour, changed, packet, read) = walker.input(ordinal)?;
        #[cfg(any(test, feature = "test-support"))]
        DECODED.with(|count| count.set(count.get() + 1));
        let before = match (self.hour, known) {
            (Some(_), Some(known)) => Some(known),
            (Some(previous), None) => Some(walker.span(ordinal - 1, previous)?),
            (None, _) => None,
        };
        let span = walker.span(ordinal, hour)?;
        // Every read succeeded: the input is applied whole or not at all.
        if let Some(before) = before {
            self.walk.extend(before.last_hour);
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
    /// Estimated bytes held by this copy's walk.
    #[cfg(test)]
    fn walk_bytes(&self) -> usize {
        walk_bytes(&self.walk)
    }
    /// Estimated bytes of this copy's table views. Views already named in
    /// `tables` are shared with another copy and not counted again.
    #[cfg(test)]
    fn table_bytes(&self, tables: &mut HashSet<usize>) -> usize {
        let mut count = 0;
        for view in self.decoded.iter().flat_map(|frame| &frame.tables) {
            let view: &TableView = view;
            if tables.insert(std::ptr::from_ref(view).addr()) {
                count += 1 + match view {
                    TableView::Complete(table) | TableView::Truncated { rows: table, .. } => {
                        table.len()
                    }
                    _ => 0,
                };
            }
        }
        count * NOMINAL_ROW_BYTES
    }
}

impl Entry {
    fn new(branch: u64, first: i32, base: u64) -> Self {
        Self {
            base,
            tail: Resume::start(branch, first, base),
        }
    }
    /// Whether the store still holds every input this walk observed. A store
    /// only appends; a rebuilt one that disagrees starts the walk again.
    fn holds(&self, walker: &mut Walker<'_, '_>, base: u64, end: u64) -> io::Result<bool> {
        if self.base != base || self.tail.next > end {
            return Ok(false);
        }
        match self.tail.hour {
            Some(hour) => Ok(walker.topology.event_date(0, self.tail.next - 1)? == hour),
            None => Ok(true),
        }
    }
}

fn cached(source: &Source, key: Key) -> Arc<Mutex<Option<Entry>>> {
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

/// The viewpoint's whole-branch walk, continued from its kept state through every published
/// change input.
#[allow(clippy::too_many_arguments)]
pub(super) fn walk(
    source: &Source,
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    (first, last): (i32, i32),
    params: &WarsParams,
    selected: Option<&HashSet<(u32, u32)>>,
    cat: Option<&Arc<Catalogue>>,
    cancelled: &AtomicBool,
) -> io::Result<WarsReply> {
    if first > last {
        return Err(invalid("Wars change inputs: invalid event page"));
    }
    // The recorded viewpoint's walk is published while preparing.
    if selected.is_none()
        && let Some(reply) = published(
            store,
            key,
            branch,
            (first, last),
            params.viewpoint_tag.as_deref(),
            cat.map(|cat| &**cat),
            cancelled,
        )?
    {
        return Ok(reply);
    }
    let mut topology = Reader::new(store, key, format!("wars-series-{branch}"), cancelled)?;
    let base = if first == i32::MIN {
        0
    } else {
        topology.event_count_through(0, first - 1)?
    };
    let end = topology.event_count_through(0, last)?;
    let shared = cached(
        source,
        Key {
            store: key.clone(),
            branch,
            view: params.viewpoint_tag.clone(),
            selected: selected.cloned(),
            catalogue: cat.cloned(),
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
    let mut walker = Walker {
        topology: &mut topology,
        base,
        view: params.viewpoint_tag.as_deref(),
        selected,
        cat: cat.map(|cat| &**cat),
        cancelled,
        ahead: None,
    };
    let holds = match guard.as_ref() {
        Some(entry) => entry.holds(&mut walker, base, end)?,
        None => false,
    };
    if !holds {
        *guard = Some(Entry::new(branch, first, base));
    }
    let entry = guard.as_mut().expect("kept walk");
    let mut known = None;
    let stopped = AtomicBool::new(false);
    let (view, catalogue) = (walker.view, walker.cat);
    let read = move |frame: &DecodedFrame| read_ahead(frame, view, selected, catalogue, false);
    // The rows are read on a thread of their own, through its own scope and reader.
    let rows = || {
        let scope = store.scope(key)?;
        let reader = Reader::new(store, key, format!("wars-series-{branch}"), cancelled)?;
        io::Result::Ok((scope, reader))
    };
    std::thread::scope(|scope| {
        // Ends the workers before the scope joins them, however the walk ends.
        struct Stop<'s>(&'s AtomicBool);
        impl Drop for Stop<'_> {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let _stop = Stop(&stopped);
        walker.ahead = (entry.tail.next < end).then(|| {
            let frame = entry.tail.decoded.clone();
            Ahead::start(scope, (entry.tail.next, end), frame, &stopped, &read, &rows)
        });
        let walked = (|| {
            while entry.tail.next < end {
                known = Some(entry.tail.step(&mut walker, known)?);
            }
            io::Result::Ok(())
        })();
        walker.ahead = None;
        walked
    })?;
    let mut walk = entry.tail.walk.clone();
    if let Some(hour) = entry.tail.hour {
        let span = match known {
            Some(span) => span,
            None => walker.span(entry.tail.next - 1, hour)?,
        };
        walk.extend(span.last_hour);
    }
    drop(guard);
    walk.finish(last, cat.map(|cat| &**cat))
}

/// The recorded viewpoint's walk as preparation published it
/// ([`super::published`]): the tail, with its closed wars back in place,
/// finishes the reply. It is
/// also `view`'s walk when it refused no hour and every hour it read had `view`
/// as its recorded viewpoint. None when no walk is published through the store's last input
/// (prepared hour by hour, or the branch's first hour moved) or it is not
/// `view`'s, so the query walks.
#[allow(clippy::too_many_arguments)]
fn published(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    (first, last): (i32, i32),
    view: Option<&str>,
    cat: Option<&Catalogue>,
    cancelled: &AtomicBool,
) -> io::Result<Option<WarsReply>> {
    use super::published::Published;
    let Some(mut tail) = Published::read(store, key, branch, cat)? else {
        return Ok(None);
    };
    // An hour refused, or recorded from another viewpoint, may read differently from `view`.
    if let Some(view) = view
        && (tail.walk.warnings.iter().any(|warning| warning != super::CAPITULATIONS_UNOBSERVED)
            || tail.walk.reply.viewpoints.iter().any(|run| run.tag != view))
    {
        return Ok(None);
    }
    let mut topology = Reader::new(store, key, format!("wars-series-{branch}"), cancelled)?;
    let base = if first == i32::MIN {
        0
    } else {
        topology.event_count_through(0, first - 1)?
    };
    let end = topology.event_count_through(0, last)?;
    if tail.first != first || base != 0 || tail.next != end {
        return Ok(None);
    }
    tail.close(store, key, branch)?;
    let mut walk = tail.walk;
    if let Some(hour) = tail.hour {
        let mut walker = Walker {
            topology: &mut topology,
            base,
            view: None,
            selected: None,
            cat,
            cancelled,
            ahead: None,
        };
        let span = walker.span(end - 1, hour)?;
        walk.extend(span.last_hour);
    }
    walk.finish(last, cat).map(Some)
}

/// Change inputs decoded on this thread.
#[cfg(any(test, feature = "test-support"))]
pub fn decoded_inputs() -> usize {
    DECODED.with(std::cell::Cell::get)
}

/// One kept walk's bounds, for tests.
#[cfg(test)]
#[derive(Debug)]
pub(super) struct Held {
    pub inputs: u64,
    pub tail_bytes: usize,
}
/// The source's kept walks, the most recently used first.
#[cfg(test)]
pub(super) fn held(source: &Source) -> Vec<Held> {
    let state = source.state::<Walks>();
    let walks = state.lock().unwrap();
    walks
        .0
        .iter()
        .filter_map(|(_, entry)| {
            let entry = entry.lock().unwrap();
            entry.as_ref().map(|entry| Held {
                inputs: entry.tail.next - entry.base,
                tail_bytes: entry.tail.walk_bytes() + entry.tail.table_bytes(&mut HashSet::new()),
            })
        })
        .collect()
}
