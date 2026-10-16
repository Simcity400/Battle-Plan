//! Each requested viewpoint's walk over the prepared change inputs, kept with
//! resume points so later queries walk only the inputs their reply needs.
use super::*;
use dashboard_base::prepared::series::{self, Reader};
use replay::codec::Decoder;
use std::sync::{Arc, Mutex};

/// Viewpoint walks kept per source, the most recently used first.
pub(super) const CACHED_WALKS: usize = 4;
/// Resume points kept per walk: their interval doubles to stay within this.
/// Over 50 years of near-hourly inputs a point every few hundred inputs keeps
/// a mid-branch read short; the byte budget bounds their memory. Tests keep
/// fewer, so their short fixtures double the interval.
#[cfg(not(test))]
pub(super) const WALK_POINTS: usize = 1024;
#[cfg(test)]
pub(super) const WALK_POINTS: usize = 64;
/// Estimated bytes of one walk's resume points, beyond which their interval
/// doubles.
pub(super) const WALK_POINT_BYTES: usize = 64 << 20;
/// Nominal bytes of one held row, run, name or key in the estimate.
const NOMINAL_ROW_BYTES: usize = 64;

#[cfg(test)]
thread_local! {
    /// Change inputs decoded on this thread.
    pub(super) static DECODED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// A smaller resume point budget for this thread's walks.
    pub(super) static POINT_BYTES: std::cell::Cell<Option<usize>> =
        const { std::cell::Cell::new(None) };
}
fn point_budget() -> usize {
    #[cfg(test)]
    if let Some(bytes) = POINT_BYTES.with(std::cell::Cell::get) {
        return bytes;
    }
    WALK_POINT_BYTES
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
    /// Resumable copies every `interval` inputs from `base`, the branch start
    /// first.
    points: Vec<Point>,
    interval: u64,
}

/// A resumable copy and its estimated bytes, apart from its table views.
struct Point {
    resume: Resume,
    bytes: usize,
}
impl Point {
    fn new(resume: Resume) -> Self {
        Self {
            bytes: resume.walk_bytes(),
            resume,
        }
    }
}

struct Walker<'r, 'a> {
    topology: &'r mut Reader<'a>,
    base: u64,
    view: Option<&'r str>,
    selected: Option<&'r HashSet<(u32, u32)>>,
    cat: Option<&'r Catalogue>,
    cancelled: &'r AtomicBool,
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
}

impl Resume {
    fn start(branch: u64, first: i32, base: u64) -> Self {
        Self {
            next: base,
            hour: None,
            walk: Walk::new(branch, first),
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
        let (hour, row) = walker.topology.event_raw(0, ordinal).map_err(|error| {
            io::Error::new(error.kind(), format!("Wars change inputs: {error}"))
        })?;
        let (changes, packet) = decode_input(&row)?;
        let bytes = series::decompress(packet).map_err(|error| {
            io::Error::new(error.kind(), format!("Wars input at {hour}: {error}"))
        })?;
        let mut input = Decoder::new(&bytes);
        let packet = DecodedFrame::decode_checkpoint(&mut input)?;
        input.finish()?;
        #[cfg(test)]
        DECODED.with(|count| count.set(count.get() + 1));
        let changed = changes
            .iter()
            .map(|&table| {
                TableId::ALL
                    .iter()
                    .position(|id| *id as u16 == table)
                    .ok_or_else(|| invalid("Unknown wars input table"))
            })
            .collect::<io::Result<Vec<_>>>()?;
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
        for index in changed {
            current.tables[index] = packet.tables[index].clone();
        }
        let frame = self.decoded.insert(current);
        self.walk
            .observe(frame, walker.view, walker.selected, walker.cat);
        self.next += 1;
        self.hour = Some(hour);
        Ok(span)
    }
    /// Estimated bytes held by this copy's walk.
    fn walk_bytes(&self) -> usize {
        let walk = &self.walk;
        let reply = &walk.reply;
        let mut count = reply.viewpoints.len()
            + reply.unobserved.len()
            + reply.capitulations_unobserved.len()
            + reply.wars.len()
            + walk
                .open
                .iter()
                .map(|(_, relations)| 1 + relations.len())
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
    /// Estimated bytes of this copy's table views. Views already named in
    /// `tables` are shared with another copy and not counted again.
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
        let start = Resume::start(branch, first, base);
        Self {
            base,
            points: vec![Point::new(start.clone())],
            tail: start,
            interval: 1,
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
    /// Keep a resumable copy of the tail at each interval, doubling the
    /// interval while the copies exceed their count or byte bound.
    fn mark(&mut self) {
        if !(self.tail.next - self.base).is_multiple_of(self.interval) {
            return;
        }
        self.points.push(Point::new(Resume {
            next: self.tail.next,
            hour: self.tail.hour,
            walk: self.tail.walk.resumable(),
            decoded: self.tail.decoded.clone(),
        }));
        while self.points.len() > 1
            && (self.points.len() > WALK_POINTS || self.point_bytes() > point_budget())
        {
            self.interval *= 2;
            let (base, interval) = (self.base, self.interval);
            self.points
                .retain(|point| (point.resume.next - base).is_multiple_of(interval));
        }
    }
    fn point_bytes(&self) -> usize {
        let mut tables = HashSet::new();
        self.points
            .iter()
            .map(|point| point.bytes + point.resume.table_bytes(&mut tables))
            .sum()
    }
    /// The nearest resume point at or before the state after input `ordinal`.
    /// Every multiple of the interval up to the tail is kept, so it is fewer
    /// than `interval` inputs before.
    fn point(&self, ordinal: u64) -> &Resume {
        let at = self
            .points
            .partition_point(|point| point.resume.next <= ordinal + 1);
        &self.points[at.max(1) - 1].resume
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

/// The viewpoint's whole-branch walk, continued from its kept state through
/// every published change input. `hydrate` receives the walk just after each
/// input covering a requested hour, with that input's coverage, in input
/// order: inputs already walked resume from their nearest point.
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
    hours: &BTreeSet<i32>,
    cancelled: &AtomicBool,
    mut hydrate: impl FnMut(&Walk, series::Run) -> io::Result<()>,
) -> io::Result<WarsReply> {
    if first > last {
        return Err(invalid("Wars change inputs: invalid event page"));
    }
    let mut topology = Reader::new(store, key, format!("wars-series-{branch}"), cancelled)?;
    let base = if first == i32::MIN {
        0
    } else {
        topology.event_count_through(0, first - 1)?
    };
    let end = topology.event_count_through(0, last)?;
    // The last input at or before each requested hour; hydration checks that
    // its coverage reaches the hour.
    let mut wanted = BTreeSet::new();
    for &hour in hours {
        let through = topology.event_count_through(0, hour)?.min(end);
        if let Some(ordinal) = through.checked_sub(1).filter(|ordinal| *ordinal >= base) {
            wanted.insert(ordinal);
        }
    }
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
    };
    let holds = match guard.as_ref() {
        Some(entry) => entry.holds(&mut walker, base, end)?,
        None => false,
    };
    if !holds {
        *guard = Some(Entry::new(branch, first, base));
    }
    let entry = guard.as_mut().expect("kept walk");
    let published = entry.tail.next;
    let mut work: Option<Resume> = None;
    for &ordinal in wanted.range(..published) {
        if ordinal + 1 == published {
            let hour = entry.tail.hour.expect("observed input");
            let span = walker.span(ordinal, hour)?;
            hydrate(&entry.tail.walk, span)?;
            continue;
        }
        let point = entry.point(ordinal);
        if !work
            .as_ref()
            .is_some_and(|work| point.next <= work.next && work.next <= ordinal + 1)
        {
            work = Some(point.clone());
        }
        let resume = work.as_mut().expect("resume point");
        let mut known = None;
        while resume.next <= ordinal {
            known = Some(resume.step(&mut walker, known)?);
        }
        let span = match known {
            Some(span) => span,
            None => walker.span(ordinal, resume.hour.expect("observed input"))?,
        };
        hydrate(&resume.walk, span)?;
    }
    let mut known = None;
    while entry.tail.next < end {
        let span = entry.tail.step(&mut walker, known)?;
        known = Some(span);
        entry.mark();
        if wanted.contains(&(entry.tail.next - 1)) {
            hydrate(&entry.tail.walk, span)?;
        }
    }
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

/// One kept walk's bounds, for tests.
#[cfg(test)]
#[derive(Debug)]
pub(super) struct Held {
    pub inputs: u64,
    pub points: usize,
    pub interval: u64,
    pub point_bytes: usize,
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
                points: entry.points.len(),
                interval: entry.interval,
                point_bytes: entry.point_bytes(),
                tail_bytes: entry.tail.walk_bytes() + entry.tail.table_bytes(&mut HashSet::new()),
            })
        })
        .collect()
}
