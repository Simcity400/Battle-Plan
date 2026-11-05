//! Wars replies from the family's publication ([`super::emit`]): the viewpoint's walk over the
//! prepared layers ([`super::layers`]), read in hour order and holding only what the hour read
//! holds, observing each hour where what it reads changes and extending between; each war's
//! casualties at the hours asked, from the counters of the relations it held then; and the groups
//! and contributions at an hour, from the layers there, the counters and the participation parts.
//! No recorded table is read.

use super::emit::{COUNTERS, GROUPS, Keys, viewpoint_name};
use super::inputs::{self, PARTICIPATION};
use super::layers::{self, Capitulations, Failure, Identity, Label, Members, Mode, Roster, State};
use super::*;
use dashboard_base::prepared::published::Opened;
use dashboard_series::{
    Cell, Cursor, EventQuery, KeyHandle, Observation, PageRead, Run, Selection, Snapshot, Span,
    family::Directory, query::EventView,
};
use replay_frame::frame::not_selected;
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};

/// The bytes a page of events or runs a walk holds at once may take.
const PAGE: usize = 1 << 16;

fn missing() -> io::Error {
    invalid("The wars publication is missing a key")
}

fn selection(key: KeyHandle) -> Selection<'static> {
    Selection {
        key,
        membership: None,
    }
}

fn earliest(a: Option<i32>, b: Option<i32>) -> Option<i32> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
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

/// One key's events over a span in native order, read a page at a time.
struct Events {
    key: KeyHandle,
    /// None once every page is read.
    span: Option<Span>,
    cursor: Option<Cursor>,
    page: VecDeque<EventView>,
}

impl Events {
    fn new(key: KeyHandle, span: Option<Span>) -> Self {
        Self {
            key,
            span,
            cursor: None,
            page: VecDeque::new(),
        }
    }
    /// The hour of the next event.
    fn peek<P: PageRead>(&mut self, snapshot: &mut Snapshot<'_, P>) -> io::Result<Option<i32>> {
        while self.page.is_empty() {
            let Some(span) = self.span else {
                return Ok(None);
            };
            let selections = [selection(self.key)];
            let page = &mut self.page;
            let read = snapshot.event_page(
                EventQuery {
                    selection: &selections,
                    native: span,
                    as_of: None,
                    order: dashboard_series::EventOrder::Native,
                },
                self.cursor,
                PAGE,
                |view| {
                    page.push_back(view);
                    Ok(())
                },
            )?;
            self.cursor = read.next;
            if self.cursor.is_none() {
                self.span = None;
            }
        }
        Ok(self.page.front().map(|view| view.native_hour()))
    }
    /// The next event: its hour and bytes.
    fn next<P: PageRead>(
        &mut self,
        snapshot: &mut Snapshot<'_, P>,
    ) -> io::Result<Option<(i32, Vec<u8>)>> {
        if self.peek(snapshot)?.is_none() {
            return Ok(None);
        }
        let view = self.page.pop_front().expect("a peeked event");
        Ok(Some((view.native_hour(), event_bytes(snapshot, view)?)))
    }
    /// Passes every event through `hour`, returning the last.
    fn pass<P: PageRead>(
        &mut self,
        snapshot: &mut Snapshot<'_, P>,
        hour: i32,
    ) -> io::Result<Option<EventView>> {
        let mut last = None;
        while self.peek(snapshot)?.is_some_and(|at| at <= hour) {
            last = self.page.pop_front();
        }
        Ok(last)
    }
    /// The bytes of the last event through `hour`, every event through it passed.
    fn through<P: PageRead>(
        &mut self,
        snapshot: &mut Snapshot<'_, P>,
        hour: i32,
    ) -> io::Result<Option<Vec<u8>>> {
        match self.pass(snapshot, hour)? {
            Some(view) => Ok(Some(event_bytes(snapshot, view)?)),
            None => Ok(None),
        }
    }
}

/// The last event of `key` through `hour`: its hour and bytes.
fn latest<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: KeyHandle,
    hour: i32,
) -> io::Result<Option<(i32, Vec<u8>)>> {
    match snapshot.latest_event(selection(key), hour, None)? {
        Some(view) => Ok(Some((view.native_hour(), event_bytes(snapshot, view)?))),
        None => Ok(None),
    }
}

/// The runs of `key` over `span`.
fn runs<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: KeyHandle,
    span: Span,
) -> io::Result<Vec<Run>> {
    let mut held = Vec::new();
    let mut cursor = None;
    loop {
        let page = snapshot.run_page(selection(key), span, cursor, 1 << 20, |run| {
            held.push(run);
            Ok(())
        })?;
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    Ok(held)
}

/// Whether a coverage run's value marks an hour naming reads again ([`super::emit`]).
fn names_again(run: &Run) -> bool {
    run.value & 1 == 1
}

/// Whether the walk names its groups again at `hour`: a coverage run starts there whose tables
/// naming reads changed.
fn renamed<P: PageRead>(snapshot: &mut Snapshot<'_, P>, keys: Keys, hour: i32) -> io::Result<bool> {
    // Read over the hour before too: a run read at one hour alone starts again at each cut.
    let span = Span::new(hour.saturating_sub(1), hour)?;
    let mut renamed = false;
    snapshot.run_page(selection(keys.coverage), span, None, PAGE, |run| {
        renamed |= run.span.first == hour && names_again(&run);
        Ok(())
    })?;
    Ok(renamed)
}

/// The coverage runs a walk reads ahead of the hour it observes: the hours it reads its tables
/// again, or only those naming reads again, where `naming`. Read once forward as hours rise.
#[derive(Default)]
struct Starts {
    naming: bool,
    /// The hour and last hour read from: every run starting after it through the last is in
    /// `page` or past `cursor`.
    held: Option<(i32, i32)>,
    span: Option<Span>,
    cursor: Option<Cursor>,
    page: VecDeque<Run>,
}

impl Starts {
    fn naming(naming: bool) -> Self {
        Self {
            naming,
            ..Self::default()
        }
    }
    fn fetch<P: PageRead>(
        &mut self,
        snapshot: &mut Snapshot<'_, P>,
        key: KeyHandle,
        cursor: Option<Cursor>,
    ) -> io::Result<()> {
        let span = self
            .span
            .ok_or_else(|| invalid("Wars coverage read past its span"))?;
        let page = &mut self.page;
        let read = snapshot.run_page(selection(key), span, cursor, PAGE, |run| {
            page.push_back(run);
            Ok(())
        })?;
        self.cursor = read.next;
        Ok(())
    }
    /// Reads the runs over `hour` through `last`; the one holding `hour` is clipped to it.
    fn open<P: PageRead>(
        &mut self,
        snapshot: &mut Snapshot<'_, P>,
        key: KeyHandle,
        hour: i32,
        last: i32,
    ) -> io::Result<()> {
        self.span = Some(Span::new(hour, last)?);
        self.held = Some((hour, last));
        self.page.clear();
        self.fetch(snapshot, key, None)
    }
    /// The first hour after `hour`, through `last`, at which a coverage run starts (one whose
    /// tables naming reads changed, where naming).
    fn after<P: PageRead>(
        &mut self,
        snapshot: &mut Snapshot<'_, P>,
        key: KeyHandle,
        (hour, last): (i32, i32),
    ) -> io::Result<Option<i32>> {
        if hour >= last {
            return Ok(None);
        }
        if self
            .held
            .is_none_or(|(from, through)| through != last || hour < from)
        {
            self.open(snapshot, key, hour, last)?;
        }
        loop {
            while self
                .page
                .front()
                .is_some_and(|run| run.span.first <= hour || (self.naming && !names_again(run)))
            {
                self.page.pop_front();
            }
            if let Some(run) = self.page.front() {
                return Ok(Some(run.span.first));
            }
            let Some(cursor) = self.cursor else {
                return Ok(None);
            };
            self.fetch(snapshot, key, Some(cursor))?;
        }
    }
}

/// A roster and the countries it reads, or why it reads none.
struct Rostered {
    roster: Roster,
    countries: Result<Arc<Countries>, Failure>,
}

fn rostered(bytes: &[u8]) -> io::Result<Rostered> {
    let roster = Roster::decode(bytes)?;
    let countries = roster.resolve().map(Arc::new);
    Ok(Rostered { roster, countries })
}

/// The layers every viewpoint reads alike, as they hold at the hour read.
#[derive(Default)]
struct Shared {
    roster: Option<Rostered>,
    state: Option<State>,
    /// The active relations' bytes, decoded once a group reads them.
    relations: Option<Vec<u8>>,
    graph: Option<Arc<World>>,
    capitulations: Option<Capitulations>,
}

impl Shared {
    /// The layers at `hour` alone.
    fn at<P: PageRead>(snapshot: &mut Snapshot<'_, P>, keys: Keys, hour: i32) -> io::Result<Self> {
        let roster = match latest(snapshot, keys.roster, hour)? {
            Some((_, bytes)) => Some(rostered(&bytes)?),
            None => None,
        };
        let state = match latest(snapshot, keys.state, hour)? {
            Some((_, bytes)) => Some(State::decode(&bytes)?),
            None => None,
        };
        Ok(Self {
            roster,
            state,
            relations: latest(snapshot, keys.relations, hour)?.map(|(_, bytes)| bytes),
            graph: None,
            capitulations: None,
        })
    }
    /// The slot `mode` views and its identity, or why the hour cannot be read ([`layers::read`]).
    fn read(&self, mode: &Mode<'_>, counted: bool) -> Result<(u32, Identity), Failure> {
        let roster = self
            .roster
            .as_ref()
            .ok_or_else(|| Failure::plain("Not recorded: war countries"))?;
        let view = layers::read(
            mode,
            &roster.roster,
            &roster.countries,
            self.state.as_ref(),
            counted,
        )?;
        roster
            .countries
            .as_ref()
            .ok()
            .and_then(|countries| countries.identities.get(&view).copied())
            .map(|identity| (view, identity))
            .ok_or_else(|| Failure::plain("Not recorded: viewpoint identity"))
    }
    fn countries(&self) -> io::Result<Arc<Countries>> {
        self.roster
            .as_ref()
            .and_then(|roster| roster.countries.as_ref().ok())
            .cloned()
            .ok_or_else(|| invalid("The wars publication holds no countries for a read hour"))
    }
    /// The active relations.
    fn graph(&mut self) -> io::Result<Arc<World>> {
        if let Some(graph) = &self.graph {
            return Ok(Arc::clone(graph));
        }
        let bytes = self
            .relations
            .as_ref()
            .ok_or_else(|| invalid("The wars publication holds no relations for a group"))?;
        let graph = Arc::new(layers::graph(&layers::decode_relations(bytes)?));
        self.graph = Some(Arc::clone(&graph));
        Ok(graph)
    }
    fn capitulations(&self) -> Result<&[(u32, i32)], ()> {
        match &self.capitulations {
            Some(Ok(rows)) => Ok(rows.as_slice()),
            _ => Err(()),
        }
    }
}

/// The shared layers' changes over the hours a walk reads.
struct Changes {
    roster: Events,
    state: Events,
    relations: Events,
    capitulations: Events,
    dated: Events,
}

impl Changes {
    fn over(keys: Keys, span: Span) -> Self {
        let events = |key| Events::new(key, Some(span));
        Self {
            roster: events(keys.roster),
            state: events(keys.state),
            relations: events(keys.relations),
            capitulations: events(keys.capitulations),
            dated: events(keys.dated),
        }
    }
    /// Brings `shared` to `hour`.
    fn apply<P: PageRead>(
        &mut self,
        snapshot: &mut Snapshot<'_, P>,
        shared: &mut Shared,
        hour: i32,
    ) -> io::Result<()> {
        if let Some(bytes) = self.roster.through(snapshot, hour)? {
            shared.roster = Some(rostered(&bytes)?);
        }
        if let Some(bytes) = self.state.through(snapshot, hour)? {
            shared.state = Some(State::decode(&bytes)?);
        }
        if let Some(bytes) = self.relations.through(snapshot, hour)? {
            shared.relations = Some(bytes);
            shared.graph = None;
        }
        if let Some(bytes) = self.capitulations.through(snapshot, hour)? {
            shared.capitulations = Some(layers::decode_capitulations(&bytes)?);
        }
        self.dated.pass(snapshot, hour)?;
        Ok(())
    }
    /// The next hour a walk reads its tables differently at.
    fn next<P: PageRead>(&mut self, snapshot: &mut Snapshot<'_, P>) -> io::Result<Option<i32>> {
        let mut next = None;
        for events in [
            &mut self.roster,
            &mut self.state,
            &mut self.relations,
            &mut self.capitulations,
            &mut self.dated,
        ] {
            next = earliest(next, events.peek(snapshot)?);
        }
        Ok(next)
    }
}

/// One group as a list names it.
type Listed = (Arc<Members>, Arc<Label>);

fn record<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: KeyHandle,
    ordinal: u32,
) -> io::Result<Vec<u8>> {
    let view = snapshot
        .event_raw(selection(key), None, u64::from(ordinal))?
        .ok_or_else(|| invalid("A wars group record is missing"))?;
    event_bytes(snapshot, view)
}

/// The groups one viewpoint identity sees, from the hour a walk began reading them, with the
/// records of the list it holds alone.
struct Viewing {
    identity: Identity,
    events: Events,
    list: Vec<(u32, u32)>,
    groups: Vec<Listed>,
    /// Whether a label's refusal is worded with the hour naming reads it at.
    stamped: bool,
}

impl Viewing {
    /// `identity`'s groups at `hour`, with their changes through `last`.
    fn open<P: PageRead>(
        snapshot: &mut Snapshot<'_, P>,
        directory: &Directory,
        keys: Keys,
        identity: Identity,
        (hour, last): (i32, i32),
    ) -> io::Result<Self> {
        let mut viewing = Self {
            identity,
            events: Events::new(keys.members, None),
            list: Vec::new(),
            groups: Vec::new(),
            stamped: false,
        };
        let Some(key) = directory
            .actor(&viewpoint_name(identity))
            .and_then(|actor| directory.key(actor, GROUPS, None))
        else {
            return Ok(viewing);
        };
        if let Some((_, bytes)) = latest(snapshot, key, hour)? {
            viewing.hold(snapshot, keys, layers::decode_list(&bytes)?)?;
        }
        if hour < last {
            viewing.events = Events::new(key, Some(Span::new(hour + 1, last)?));
        }
        Ok(viewing)
    }
    /// Brings the groups to `hour`.
    fn apply<P: PageRead>(
        &mut self,
        snapshot: &mut Snapshot<'_, P>,
        keys: Keys,
        hour: i32,
    ) -> io::Result<()> {
        if let Some(bytes) = self.events.through(snapshot, hour)? {
            self.hold(snapshot, keys, layers::decode_list(&bytes)?)?;
        }
        Ok(())
    }
    /// Holds `list`, reading the records the list before did not hold.
    fn hold<P: PageRead>(
        &mut self,
        snapshot: &mut Snapshot<'_, P>,
        keys: Keys,
        list: Vec<(u32, u32)>,
    ) -> io::Result<()> {
        let mut members: HashMap<u32, Arc<Members>> = HashMap::new();
        let mut labels: HashMap<u32, Arc<Label>> = HashMap::new();
        for (&(held_members, held_label), (record_members, record_label)) in
            self.list.iter().zip(&self.groups)
        {
            members.insert(held_members, Arc::clone(record_members));
            labels.insert(held_label, Arc::clone(record_label));
        }
        let mut groups = Vec::with_capacity(list.len());
        for &(ordinal_members, ordinal_label) in &list {
            let group_members = match members.get(&ordinal_members) {
                Some(held) => Arc::clone(held),
                None => {
                    let held = Arc::new(Members::decode(&record(
                        snapshot,
                        keys.members,
                        ordinal_members,
                    )?)?);
                    members.insert(ordinal_members, Arc::clone(&held));
                    held
                }
            };
            let group_label = match labels.get(&ordinal_label) {
                Some(held) => Arc::clone(held),
                None => {
                    let held = Arc::new(Label::decode(&record(
                        snapshot,
                        keys.labels,
                        ordinal_label,
                    )?)?);
                    labels.insert(ordinal_label, Arc::clone(&held));
                    held
                }
            };
            groups.push((group_members, group_label));
        }
        self.stamped = groups.iter().any(|(_, label)| label.stamped());
        self.list = list;
        self.groups = groups;
        Ok(())
    }
}

/// The groups `view` sees at `hour`, as a walk observes them, named as naming at `named` words
/// them.
fn observations(
    countries: &Countries,
    graph: &World,
    view: u32,
    groups: &[Listed],
    losses: Option<&Losses<'_>>,
    (hour, named): (i32, i32),
) -> Vec<Observed> {
    groups
        .iter()
        .map(|(members, label)| {
            observed(
                graph,
                (&countries.tags, &countries.identities),
                view,
                Blob {
                    first: members.first.clone(),
                    second: members.second.clone(),
                },
                label.at(named),
                losses,
                hour,
            )
        })
        .collect()
}

/// What a walk of one branch reads: its viewpoint, and the content naming its wars.
pub(crate) struct Walked<'a> {
    pub branch: u64,
    pub bounds: (i32, i32),
    pub view: Option<&'a str>,
    pub selected: Option<&'a HashSet<(u32, u32)>>,
    pub cat: Option<&'a Arc<Catalogue>>,
    pub cancelled: &'a AtomicBool,
}

impl Walked<'_> {
    fn mode(&self) -> Mode<'_> {
        match (self.selected, self.view) {
            (Some(selected), _) => Mode::Selected(selected),
            (None, Some(tag)) => Mode::Tag(tag),
            (None, None) => Mode::Recorded,
        }
    }
}

/// The viewpoint's whole-branch walk over the prepared layers, read in hour order: each walked
/// run observed at its first hour, wherever a layer it reads changes, and, while a refusal is
/// worded with the hour of a read, at each hour that read is made again; extended between.
fn walk<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    keys: Keys,
    walked: &Walked<'_>,
) -> io::Result<WarsReply> {
    let (first, last) = walked.bounds;
    if first > last {
        return Err(invalid("Wars: invalid branch"));
    }
    let cat = walked.cat.map(|cat| &**cat);
    let mut walk = Walk {
        losses: false,
        ..Walk::new(walked.branch, first)
    };
    let accepted = snapshot.publication().accepted;
    let Some(span) = Span::new(first, last)?.intersect(accepted) else {
        return walk.finish(last, cat);
    };
    let mut available = Events::new(keys.available, Some(span));
    while let Some((_, bytes)) = available.next(snapshot)? {
        walk.reply.available |= bytes.as_slice() == [1];
    }
    let mut names = Events::new(keys.names, Some(span));
    while let Some((_, bytes)) = names.next(snapshot)? {
        for (raw, name) in layers::decode_names(&bytes)? {
            walk.country_labels.add(raw, name);
        }
    }
    let mode = walked.mode();
    let mut changes = Changes::over(keys, span);
    let mut shared = Shared::default();
    let mut starts = Starts::naming(false);
    let mut naming_starts = Starts::naming(true);
    let mut viewing: Option<Viewing> = None;
    // The hour the walk last read the tables its groups are named from; none after an hour that
    // did not read.
    let mut named: Option<i32> = None;
    for run in runs(snapshot, keys.walked, span)? {
        let (a, b) = (run.span.first, run.span.last);
        let mut hour = a;
        loop {
            dashboard_text::metadata::check_cancel(walked.cancelled)?;
            changes.apply(snapshot, &mut shared, hour)?;
            let mut next = changes.next(snapshot)?;
            let read = match shared.read(&mode, false) {
                Err(failure) => {
                    viewing = None;
                    named = None;
                    if failure.stamped() {
                        let again = starts.after(snapshot, keys.coverage, (hour, b))?;
                        next = earliest(next, again);
                    }
                    Err(failure.at(hour))
                }
                Ok((view, identity)) => {
                    if viewing
                        .as_ref()
                        .is_none_or(|held| held.identity != identity)
                    {
                        viewing = Some(Viewing::open(
                            snapshot,
                            directory,
                            keys,
                            identity,
                            (hour, span.last),
                        )?);
                    }
                    let held = viewing.as_mut().expect("the viewpoint's groups are read");
                    held.apply(snapshot, keys, hour)?;
                    next = earliest(next, held.events.peek(snapshot)?);
                    if held.stamped {
                        if named.is_none() || renamed(snapshot, keys, hour)? {
                            named = Some(hour);
                        }
                        let again = naming_starts.after(snapshot, keys.coverage, (hour, b))?;
                        next = earliest(next, again);
                    }
                    let countries = shared.countries()?;
                    let observed = if held.groups.is_empty() {
                        Vec::new()
                    } else {
                        let graph = shared.graph()?;
                        let at = (hour, named.unwrap_or(hour));
                        observations(&countries, &graph, view, &held.groups, None, at)
                    };
                    Ok((Viewed { countries, view }, observed))
                }
            };
            walk.observe(Seen {
                hour,
                read,
                capitulations: shared.capitulations(),
                losses: None,
            });
            match next.filter(|&next| next <= b) {
                Some(next) => {
                    walk.extend(next - 1);
                    hour = next;
                }
                None => {
                    walk.extend(b);
                    break;
                }
            }
        }
    }
    walk.finish(last, cat)
}

/// Adds one counter's run to a war's hour: a known counter to its side's sum, an unknown one to
/// its country's maximum, as the walk's losses count them.
fn accumulate(
    sums: &mut (i32, i32, [i128; 4]),
    unknown: &mut BTreeMap<(usize, u32), i128>,
    (output, country): (usize, u32),
    run: Run,
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

/// The counters' keys of the relation lifetime `key`.
fn counter_keys(directory: &Directory, key: &WarRelationKey) -> io::Result<[KeyHandle; 4]> {
    let actor = directory
        .actor(&inputs::relation_name(key))
        .ok_or_else(|| invalid("Missing prepared war relation lifetime"))?;
    let mut held = [0; 4];
    for (field, handle) in held.iter_mut().enumerate() {
        *handle = directory
            .key(actor, COUNTERS + field as u64, None)
            .ok_or_else(missing)?;
    }
    Ok(held)
}

/// Each war's casualties at `hours`, keyed by its place in `wars`: those of the wars observed at
/// an hour, summed over the relations they held then ([`super::WarRelationMembership`]), each
/// run bounded by the coverage run holding the hour and by every counter's run.
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
                        let held = counter_keys(directory, &membership.key)?;
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
        let mut reply = walk(snapshot, directory, keys, walked)?;
        drop(cpu);
        if !hours.is_empty() {
            let casualties = casualties_at(snapshot, directory, keys, &reply.wars, hours)?;
            place_casualties(&mut reply, casualties);
        }
        check_reply(&reply)?;
        Ok(Some(reply))
    })
}

/// The participation tables at `hour`: the last whole parts, then each hour's diff since.
fn participation_tables<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    keys: Keys,
    branch: u64,
    hour: i32,
) -> io::Result<DecodedFrame> {
    let mut frame = DecodedFrame {
        branch,
        hour,
        metrics: Default::default(),
        tables: std::array::from_fn(|_| not_selected()),
    };
    if let Some(whole) = snapshot.latest_event(selection(keys.wholes), hour, None)? {
        let EventView::Family(event) = whole else {
            return Err(invalid("A wars event is not the family's"));
        };
        let mut held = [None, None];
        inputs::fold(&mut held, inputs::unpack_parts(&event_bytes(snapshot, whole)?)?)?;
        if event.native_hour < hour {
            let since = Span::new(event.native_hour + 1, hour)?;
            let mut diffs = Events::new(keys.diffs, Some(since));
            while let Some((_, bytes)) = diffs.next(snapshot)? {
                inputs::fold(&mut held, inputs::unpack_parts(&bytes)?)?;
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

/// The groups and contributions the viewpoint sees at `hour`, a walked hour, as one hour's walk
/// reads them ([`WarsAtHour`]): its relations carrying the counters prepared for that hour.
fn at_hour<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    keys: Keys,
    walked: &Walked<'_>,
    hour: i32,
) -> io::Result<WarsAtHour> {
    let cat = walked.cat.map(|cat| &**cat);
    let mut shared = Shared::at(snapshot, keys, hour)?;
    let mut walk = Walk::new(walked.branch, hour);
    walk.labels = false;
    walk.reply.available = latest(snapshot, keys.available, hour)?
        .is_some_and(|(_, bytes)| bytes.as_slice() == [1]);
    let mut contributions = Vec::new();
    match shared.read(&walked.mode(), true) {
        Err(failure) => walk.observe(Seen {
            hour,
            read: Err(failure.at(hour)),
            capitulations: Err(()),
            losses: None,
        }),
        Ok((view, identity)) => {
            let countries = shared.countries()?;
            let groups = Viewing::open(snapshot, directory, keys, identity, (hour, hour))?.groups;
            let mut counted = World {
                owned: Default::default(),
                relations: Default::default(),
                adjacent: Default::default(),
                factions: Default::default(),
                scores: Default::default(),
            };
            let mut blobs = Vec::with_capacity(groups.len());
            let mut graph = None;
            if !groups.is_empty() {
                let held = shared.graph()?;
                for (members, _) in &groups {
                    let blob = Blob {
                        first: members.first.clone(),
                        second: members.second.clone(),
                    };
                    for p in held.inside(&blob) {
                        if counted.relations.contains_key(&p) {
                            continue;
                        }
                        let mut row = held.relations[&p].clone();
                        let key = WarRelationKey::new(
                            countries.identities[&p.0],
                            countries.identities[&p.1],
                            row.start_hour,
                        );
                        let reversed = countries.identities[&row.first_country] != key.first;
                        let mut values = [0_i64; 4];
                        for (value, handle) in values.iter_mut().zip(counter_keys(directory, &key)?)
                        {
                            let Observation::Value { run, .. } = snapshot.value_at(handle, hour)?
                            else {
                                return Err(invalid("Missing prepared war casualty value"));
                            };
                            *value = i64::try_from(run.value)
                                .map_err(|_| invalid("A wars casualty counter is out of range"))?;
                        }
                        inputs::restore_counters(&mut row, values, reversed);
                        counted.relations.insert(p, row);
                    }
                    blobs.push(blob);
                }
                graph = Some(held);
            }
            let losses = |pairs: &[(Pair, bool)]| losses_of(&counted, pairs);
            let observed = match &graph {
                Some(graph) => observations(
                    &countries,
                    graph,
                    view,
                    &groups,
                    Some(&losses as &Losses<'_>),
                    (hour, hour),
                ),
                None => Vec::new(),
            };
            let tables = participation_tables(snapshot, keys, walked.branch, hour)?;
            contributions = participation::contributions(
                &blobs,
                (&countries.tags, &countries.raw_to_slot),
                &|a, b| graph.as_ref().is_some_and(|graph| graph.active(a, b)),
                &tables,
                participation::factors(cat),
            );
            walk.observe(Seen {
                hour,
                read: Ok((Viewed { countries, view }, observed)),
                capitulations: Err(()),
                losses: Some(&losses as &Losses<'_>),
            });
        }
    }
    let reply = walk.finish(hour, cat)?;
    Ok(at_hour_reply(hour, reply, contributions))
}

/// The groups and contributions at each requested hour within what the publication accepted
/// ([`at_hour`]); an hour outside the branch or not walked is unavailable, and one past what was
/// accepted is left out, still being prepared.
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
        let covered = (first..=last).contains(&hour)
            && matches!(
                snapshot.value_at(keys.coverage, hour)?,
                Observation::Value { .. }
            );
        current.push(if covered {
            at_hour(snapshot, directory, keys, walked, hour)?
        } else {
            WarsAtHour {
                hour,
                contributions: Vec::new(),
                available: false,
                wars: Vec::new(),
                warnings: Vec::new(),
            }
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
    _store: &store::SourceKey,
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
    _store: &store::SourceKey,
    cat: Option<&Arc<Catalogue>>,
    cancelled: &AtomicBool,
) -> io::Result<Option<WarsReply>> {
    let (branch, bounds) = bounds(source, params)?;
    projection(
        &Walked {
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

/// The replies a walk of `snapshot`, a publication of the family, gives `walked`, `reads` and
/// `asked`, for tests holding it to the row walk.
#[cfg(test)]
pub(crate) fn walk_snapshot<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    walked: &Walked<'_>,
    reads: &BTreeSet<i32>,
    asked: &BTreeSet<i32>,
) -> io::Result<(WarsReply, Vec<WarsAtHour>)> {
    let keys = Keys::of(directory)?;
    let mut reply = walk(snapshot, directory, keys, walked)?;
    if !reads.is_empty() {
        let casualties = casualties_at(snapshot, directory, keys, &reply.wars, reads)?;
        place_casualties(&mut reply, casualties);
    }
    let current = current(snapshot, directory, walked, asked)?;
    Ok((reply, current))
}
