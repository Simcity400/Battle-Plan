//! Demand-driven retained script state reduced from the existing event feed.
//! Baselines prove membership at attachment; they never create past fires.
use super::decisions::{ALL, CLEAR_ALL_CANDIDATES, Operation};
use super::*;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    hash::{BuildHasherDefault, Hash, Hasher},
    sync::Arc,
};
mod delayed;
#[cfg(test)]
mod plans;
pub use delayed::DelayedEvent;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct ScriptIdentity {
    pub native_event: EventFeedEventIdentity,
    pub token: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PendingEvent {
    pub instance: u32,
    pub definition: ScriptIdentity,
    pub originator: EventFeedCountry,
    pub due_hour: i32,
    /// Binding IDs are local to this graph, preserving shared/self links.
    pub scopes: Vec<EventFeedScope>,
}

/// The parts of the retained state the last observed row may have changed, so an observer
/// of changes visits those alone.
#[derive(Debug, Default)]
pub struct Touched {
    /// Any part may have changed: a new generation, a gap, a loss or a refusal.
    pub all: bool,
    pub pending: bool,
    pub fired: bool,
    /// Delayed-event owners, by [`ScriptEventState::delayed_owner`] key.
    pub delayed: std::collections::BTreeSet<(u32, u32)>,
    /// Decision owners, by [`ScriptEventState::decision_owner`] key.
    pub decisions: std::collections::BTreeSet<(u32, u32)>,
}

/// A part's version: what the part holds, but its countdowns and its availability, changes
/// only with it, and the state never gives one twice. 0 is a change not yet stamped.
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct Version(u64);
impl Version {
    pub(super) fn change(&mut self) {
        self.0 = 0;
    }
    fn stamp(&mut self, clock: &mut u64) {
        if self.0 == 0 {
            *clock += 1;
            self.0 = *clock;
        }
    }
    pub(super) fn get(self) -> Option<u64> {
        (self.0 != 0).then_some(self.0)
    }
}

/// What a reduction reads of a pack besides its changes, with what is shown of its coverage: its
/// world, read hour and coverage, and the sequences of its first and last occurrences.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct PackHeader {
    pub generation: u64,
    pub read_hour: i32,
    pub coverage: EventFeedCoverage,
    pub outstanding: u64,
    pub failure: Option<Vec<u8>>,
    pub script_refusals: u8,
    /// The sequences of its first and last occurrences.
    pub sequences: Option<(u64, u64)>,
}
impl PackHeader {
    pub fn of(row: &EventFeedHistoryRow) -> io::Result<Self> {
        Ok(Self {
            generation: row.generation,
            read_hour: row.read_hour,
            coverage: row.coverage,
            outstanding: row.outstanding,
            failure: row
                .failure
                .as_deref()
                .map(allocation::copy_vec)
                .transpose()?,
            script_refusals: row.script_refusals,
            sequences: sequences(row),
        })
    }
    pub fn refused_script_families(&self) -> impl Iterator<Item = &'static str> {
        super::refused_script_families(self.script_refusals)
    }
    fn reading(&self) -> Reading {
        Reading {
            generation: self.generation,
            read_hour: self.read_hour,
            coverage: self.coverage,
            failed: self.failure.is_some(),
            script_refusals: self.script_refusals,
            sequences: self.sequences,
        }
    }
}
fn sequences(row: &EventFeedHistoryRow) -> Option<(u64, u64)> {
    row.events
        .first()
        .zip(row.events.last())
        .map(|(first, last)| (first.sequence, last.sequence))
}

/// What a reduction reads of a pack besides its changes ([`PackHeader`]).
#[derive(Clone, Copy)]
struct Reading {
    generation: u64,
    read_hour: i32,
    coverage: EventFeedCoverage,
    failed: bool,
    script_refusals: u8,
    sequences: Option<(u64, u64)>,
}
impl Reading {
    fn of(row: &EventFeedHistoryRow) -> Self {
        Self {
            generation: row.generation,
            read_hour: row.read_hour,
            coverage: row.coverage,
            failed: row.failure.is_some(),
            script_refusals: row.script_refusals,
            sequences: sequences(row),
        }
    }
}

/// A retained-state change as a reduction reads it: its occurrence, the arenas its spans index,
/// and its sequence.
#[derive(Clone, Copy)]
pub struct Change<'a> {
    pub sequence: u64,
    pub event: &'a EventFeedEvent,
    tokens: &'a [u8],
    scopes: &'a [EventFeedScope],
}
impl<'a> Change<'a> {
    /// Occurrence `event` of `row`.
    pub fn of(row: &'a EventFeedHistoryRow, event: &'a EventFeedEvent) -> Self {
        Self {
            sequence: event.sequence,
            event,
            tokens: &row.script_tokens,
            scopes: &row.script_scopes,
        }
    }
    /// The token `script`'s span names.
    pub(super) fn token(&self, script: EventFeedScript) -> &'a [u8] {
        let start = script.token_start as usize;
        &self.tokens[start..start + script.token_len as usize]
    }
    /// The scopes `script`'s span names.
    pub(super) fn scopes(&self, script: EventFeedScript) -> &'a [EventFeedScope] {
        let start = script.scope_start as usize;
        &self.scopes[start..start + script.scope_len as usize]
    }
    pub(super) fn identity(&self, script: EventFeedScript) -> io::Result<ScriptIdentity> {
        Ok(ScriptIdentity {
            native_event: script
                .native_event
                .ok_or_else(|| invalid("script registry event identity absent"))?,
            token: allocation::copy_vec(self.token(script))?,
        })
    }
}

/// A retained-state change but for its sequence and hour, with its own token and scopes: what
/// recurs from read to read, held once by its walk ([`Payloads`]).
#[derive(Debug)]
pub struct Payload {
    event: EventFeedEvent,
    token: Token,
    scopes: Box<[EventFeedScope]>,
    /// Its place among its walk's payloads.
    id: u32,
}
impl PartialEq for Payload {
    fn eq(&self, other: &Self) -> bool {
        (&self.event, &self.token, &self.scopes) == (&other.event, &other.token, &other.scopes)
    }
}
impl Eq for Payload {}
impl Payload {
    fn change(&self, sequence: u64) -> Change<'_> {
        Change {
            sequence,
            event: &self.event,
            tokens: self.token.bytes(),
            scopes: &self.scopes,
        }
    }
    /// Whether it is the payload of `occurrence`, as [`Payloads::intern`] would make it.
    fn holds(&self, occurrence: Change<'_>) -> bool {
        let (held, event) = (&self.event, occurrence.event);
        held.source == event.source
            && held.actor == event.actor
            && held.target == event.target
            && held.cause == event.cause
            && match (held.script, event.script) {
                (Some(own), Some(script)) => {
                    own == EventFeedScript {
                        token_start: 0,
                        scope_start: 0,
                        ..script
                    } && self.token.bytes() == occurrence.token(script)
                        && *self.scopes == *occurrence.scopes(script)
                }
                (None, None) => true,
                _ => false,
            }
    }
}

/// The longest token a payload holds in place.
const SHORT: usize = 63;

/// A change's token as its payload holds it: in place when it fits, so finding the payload
/// reads no other allocation.
#[derive(Debug, PartialEq, Eq)]
enum Token {
    Short(u8, [u8; SHORT]),
    Long(Box<[u8]>),
}
impl Token {
    fn of(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() <= SHORT {
            let mut held = [0; SHORT];
            held[..bytes.len()].copy_from_slice(bytes);
            return Ok(Self::Short(bytes.len() as u8, held));
        }
        Ok(Self::Long(allocation::boxed_slice(allocation::copy_vec(
            bytes,
        )?)?))
    }
    fn bytes(&self) -> &[u8] {
        match self {
            Self::Short(len, held) => &held[..*len as usize],
            Self::Long(bytes) => bytes,
        }
    }
}
impl Hash for Payload {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Equal payloads hash alike: these words and the token are what tells roster changes apart.
        let event = &self.event;
        state.write_u64(u64::from(event.actor.id) << 32 | u64::from(event.target.reference));
        state.write_u64(u64::from(event.target.id) << 32 | event.source as u64);
        if let Some(script) = event.script {
            state.write_u64(
                u64::from(script.country_target.unwrap_or(u32::MAX)) << 32
                    | u64::from(script.instance.unwrap_or(u32::MAX)),
            );
            state.write_u64(script.days.map_or(u64::MAX, |days| days as u32 as u64));
            if let Some(EventFeedScriptChange::Decision(change)) = script.change {
                state.write_u64(
                    u64::from(change.ordinal) << 32
                        | u64::from(change.flags) << 16
                        | u64::from(change.container) << 8
                        | change.operation as u64,
                );
                // A rebuild's balance tells one owner's rebuilds apart.
                state.write_u64(
                    change
                        .political_power
                        .map_or(u64::MAX, |power| power as u64),
                );
            }
        }
        state.write(self.token.bytes());
    }
}

/// A walk's own hash of its payloads and targets, which are its keys.
#[derive(Default)]
struct Mixed(u64);
impl Hasher for Mixed {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut word = [0; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.write_u64(u64::from_le_bytes(word));
        }
    }
    fn write_u32(&mut self, word: u32) {
        self.write_u64(word.into());
    }
    fn write_u64(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

/// The payloads a walk's packs planned so far, each held once.
#[derive(Default)]
pub struct Payloads {
    held: HashSet<Arc<Payload>, BuildHasherDefault<Mixed>>,
    /// Every payload by id, with the id of the one that followed it last.
    all: Vec<(Arc<Payload>, u32)>,
    /// The id of the payload last interned.
    last: Option<u32>,
}
impl Payloads {
    /// The id of the walk's payload of `occurrence`, and that payload with the bytes it adds when
    /// the walk held none like it.
    fn intern(
        &mut self,
        occurrence: Change<'_>,
    ) -> io::Result<(u32, Option<(Arc<Payload>, usize)>)> {
        // Changes recur in the order they came: the last one's successor is tried first.
        if let Some(last) = self.last {
            let next = self.all[last as usize].1;
            if let Some((payload, _)) = self.all.get(next as usize)
                && payload.holds(occurrence)
            {
                self.last = Some(next);
                return Ok((next, None));
            }
        }
        let (id, added) = self.find(occurrence)?;
        if let Some(last) = self.last {
            self.all[last as usize].1 = id;
        }
        self.last = Some(id);
        Ok((id, added))
    }
    /// Forgets the payloads from id `count` on, which a pack that failed to plan introduced.
    fn truncate(&mut self, count: usize) {
        for (payload, _) in self.all.drain(count.min(self.all.len())..) {
            self.held.remove(&payload);
        }
        self.last = None;
    }
    fn find(&mut self, occurrence: Change<'_>) -> io::Result<(u32, Option<(Arc<Payload>, usize)>)> {
        let mut event = EventFeedEvent {
            sequence: 0,
            generation: 0,
            hour: 0,
            ..*occurrence.event
        };
        let (token, scopes) = match &mut event.script {
            Some(script) => {
                let spans = (occurrence.token(*script), occurrence.scopes(*script));
                script.token_start = 0;
                script.scope_start = 0;
                spans
            }
            None => (&[][..], &[][..]),
        };
        let payload = Payload {
            event,
            token: Token::of(token)?,
            scopes: allocation::boxed_slice(allocation::copy_vec(scopes)?)?,
            id: u32::try_from(self.all.len())
                .map_err(|_| invalid("A walk holds more payloads than it counts"))?,
        };
        if let Some(held) = self.held.get(&payload) {
            return Ok((held.id, None));
        }
        allocation::allocation_attempt()?;
        allocation::reserve_set(&mut self.held, 1)?;
        allocation::reserve(&mut self.all, 1)?;
        let added = 2 * size_of::<usize>()
            + size_of::<Payload>()
            + match &payload.token {
                Token::Short(..) => 0,
                Token::Long(bytes) => bytes.len(),
            }
            + payload.scopes.len() * size_of::<EventFeedScope>();
        let (id, payload) = (payload.id, Arc::new(payload));
        self.held.insert(Arc::clone(&payload));
        self.all.push((Arc::clone(&payload), u32::MAX));
        Ok((id, Some((payload, added))))
    }
}

/// The payloads of a walk's prepared packs by id, as the reduction of those packs holds them: a
/// pack names its changes' payloads by id ([`PreparedHistory`]), so its walk and its reduction
/// share no count of them, and each pack brings those it introduced.
#[derive(Default)]
pub struct HeldPayloads(Vec<Arc<Payload>>);
impl HeldPayloads {
    /// Takes in the payloads `prepared` introduced, which follow those of its walk's earlier
    /// packs; a walk's first pack begins anew. Every prepared pack of a walk is admitted, in
    /// order, whether or not it is reduced.
    pub fn admit(&mut self, prepared: &PreparedHistory) -> io::Result<()> {
        if prepared.first == 0 {
            self.0.clear();
        } else if prepared.first as usize != self.0.len() {
            return Err(invalid(
                "A prepared event pack does not follow its walk's last",
            ));
        }
        allocation::reserve(&mut self.0, prepared.introduced.len())?;
        self.0.extend(prepared.introduced.iter().cloned());
        Ok(())
    }
}

/// An event pack validated and planned once, for a reduction that need not validate it again:
/// its [`PackHeader`], and of its occurrences only its retained-state changes, each with the
/// work its reduction leaves out ([`Plan`]) and the id of its payload as its walk holds it.
pub struct PreparedHistory {
    header: PackHeader,
    steps: Vec<(Step, u64, u32)>,
    /// How many payloads its walk held before it, and those it was the first to hold, which
    /// follow them by id.
    first: u32,
    introduced: Vec<Arc<Payload>>,
    /// The bytes those add.
    added: usize,
}
impl PreparedHistory {
    pub fn new(row: &EventFeedHistoryRow) -> io::Result<Self> {
        row.validate()?;
        Self::planned(row, Plan::of(row, None), &mut Payloads::default())
    }
    /// [`Self::new`] of a walk's next pack, knowing what the walk's earlier packs wrote and the
    /// payloads it holds.
    pub fn following(
        row: &EventFeedHistoryRow,
        walk: &mut Written,
        payloads: &mut Payloads,
    ) -> io::Result<Self> {
        row.validate()?;
        Self::decoded(row, walk, payloads)
    }
    /// [`Self::following`] of a pack as its decoding left it ([`RowCodec::decode_fields`],
    /// [`Value::decode`]), which validates it.
    ///
    /// [`RowCodec::decode_fields`]: crate::rows::RowCodec::decode_fields
    pub fn decoded(
        row: &EventFeedHistoryRow,
        walk: &mut Written,
        payloads: &mut Payloads,
    ) -> io::Result<Self> {
        Self::planned(row, Plan::of(row, Some(walk)), payloads)
    }
    fn planned(row: &EventFeedHistoryRow, plan: Plan, payloads: &mut Payloads) -> io::Result<Self> {
        let first = payloads.all.len();
        let planned = (|| -> io::Result<Self> {
            let mut steps = allocation::vec(plan.steps.len())?;
            let (mut introduced, mut added) = (Vec::new(), 0);
            for step in plan.steps {
                let occurrence = Change::of(row, &row.events[step.event as usize]);
                let (id, new) = payloads.intern(occurrence)?;
                if let Some((payload, bytes)) = new {
                    allocation::reserve(&mut introduced, 1)?;
                    introduced.push(payload);
                    added += bytes;
                }
                steps.push((step, occurrence.sequence, id));
            }
            Ok(Self {
                header: PackHeader::of(row)?,
                steps,
                first: u32::try_from(first)
                    .map_err(|_| invalid("A walk holds more payloads than it counts"))?,
                introduced,
                added,
            })
        })();
        if planned.is_err() {
            payloads.truncate(first);
        }
        planned
    }
    pub fn header(&self) -> &PackHeader {
        &self.header
    }
    /// The bytes it owns on the heap: its steps', its refusal text's, and those of the payloads
    /// it was its walk's first to hold.
    pub fn heap_bytes(&self) -> usize {
        self.steps.capacity() * size_of::<(Step, u64, u32)>()
            + self.header.failure.as_ref().map_or(0, Vec::capacity)
            + self.introduced.capacity() * size_of::<Arc<Payload>>()
            + self.added
    }
}

/// A prepared pack with the payloads its ids name.
struct Admitted<'a> {
    prepared: &'a PreparedHistory,
    payloads: &'a [Arc<Payload>],
}
impl<'a> Admitted<'a> {
    /// `prepared`, the pack `held` admitted last.
    fn of(prepared: &'a PreparedHistory, held: &'a HeldPayloads) -> io::Result<Self> {
        if prepared.first as usize + prepared.introduced.len() != held.0.len() {
            return Err(invalid(
                "A prepared event pack is reduced apart from its walk's payloads",
            ));
        }
        Ok(Self {
            prepared,
            payloads: &held.0,
        })
    }
}

/// A pack's retained-state changes in plan order, as a reduction reads them.
trait Planned {
    fn len(&self) -> usize;
    /// The plan's `at`th step and its change.
    fn step(&self, at: usize) -> (Step, Change<'_>);
}
struct RowPlan<'a> {
    row: &'a EventFeedHistoryRow,
    plan: &'a Plan,
}
impl Planned for RowPlan<'_> {
    fn len(&self) -> usize {
        self.plan.steps.len()
    }
    fn step(&self, at: usize) -> (Step, Change<'_>) {
        let step = self.plan.steps[at];
        (
            step,
            Change::of(self.row, &self.row.events[step.event as usize]),
        )
    }
}
impl Planned for Admitted<'_> {
    fn len(&self) -> usize {
        self.prepared.steps.len()
    }
    fn step(&self, at: usize) -> (Step, Change<'_>) {
        let (step, sequence, id) = self.prepared.steps[at];
        (step, self.payloads[id as usize].change(sequence))
    }
}

/// The decision owners' target countries a walk's packs wrote since it began, each with the
/// pack that last wrote it. They hold while every one of those packs was reduced to its end.
#[derive(Clone, Default)]
pub struct Written {
    generation: Option<u64>,
    packs: u64,
    /// By owner, each target reference's country and the pack that wrote it.
    targets: Keyed<Keyed<(EventFeedCountry, u64)>>,
}
type Keyed<T> = HashMap<u32, T, BuildHasherDefault<Mixed>>;

/// A pack's retained-state changes in order, each with the work its reduction leaves out
/// because the pack, or the walk before it, already shows that work's effect.
#[derive(Default)]
struct Plan {
    steps: Vec<Step>,
}
#[derive(Clone, Copy)]
struct Step {
    event: u32,
    flags: u8,
    /// The event whose clear or baseline cancels this candidate change ([`CANCELLED`]).
    cancelled: u32,
}
/// The change writes a target country.
const TARGETED: u8 = 1;
/// An earlier change of the pack wrote the same target country for the same owner.
const REWRITE: u8 = 2;
/// An earlier pack of the walk wrote it.
const WRITTEN: u8 = 4;
/// A candidate change a later clear or baseline of the same owner in the pack cancels.
const CANCELLED: u8 = 8;
/// A candidate change whose owner country and touch an earlier change of the pack made.
const QUIET: u8 = 16;

impl Plan {
    fn of(row: &EventFeedHistoryRow, walk: Option<&mut Written>) -> Self {
        let mut alone = Written::default();
        let walk = walk.unwrap_or(&mut alone);
        if walk.generation != Some(row.generation) {
            walk.generation = Some(row.generation);
            walk.targets.clear();
        }
        walk.packs += 1;
        let pack = walk.packs;
        let mut steps = Vec::with_capacity(row.events.len());
        // Each decision owner so far: its last change's actor, and whether it is touched.
        let mut owners: Keyed<(EventFeedCountry, bool)> = Keyed::default();
        for (index, event) in row.events.iter().enumerate() {
            let Some(script) = event.script else {
                continue;
            };
            let Some(change) = script.change else {
                continue;
            };
            let mut flags = 0;
            if let EventFeedScriptChange::Decision(decision) = change {
                let id = event.actor.id;
                let owner = owners.entry(id);
                let touches = decision.operation != Operation::Rebuild;
                if decision.operation == Operation::Candidate
                    && !script.token_unrecorded
                    && matches!(&owner, std::collections::hash_map::Entry::Occupied(held)
                        if *held.get() == (event.actor, true))
                {
                    flags |= QUIET;
                }
                let owner = owner.or_insert((event.actor, false));
                *owner = (event.actor, owner.1 | touches);
                if !script.token_unrecorded && event.target.reference as i32 > 0 {
                    flags |= TARGETED;
                    let targets = walk.targets.entry(id).or_default();
                    match targets.entry(event.target.reference) {
                        std::collections::hash_map::Entry::Occupied(mut entry) => {
                            let (held, by) = entry.get_mut();
                            if *held == event.target {
                                flags |= if *by == pack { REWRITE } else { WRITTEN };
                            }
                            *held = event.target;
                            *by = pack;
                        }
                        std::collections::hash_map::Entry::Vacant(entry) => {
                            entry.insert((event.target, pack));
                        }
                    }
                }
                // A baseline makes its owner anew, after its own target write.
                if decision.operation == Operation::Baseline
                    && !script.token_unrecorded
                    && let Some(targets) = walk.targets.get_mut(&id)
                {
                    targets.clear();
                }
            }
            steps.push(Step {
                event: index as u32,
                flags,
                cancelled: 0,
            });
        }
        // Walked back, each owner's nearest later clear of every kind and of each kind.
        let mut clears: Keyed<(Option<u32>, [Option<u32>; 4])> = Keyed::default();
        for step in steps.iter_mut().rev() {
            let event = &row.events[step.event as usize];
            let Some(script) = event.script.filter(|script| !script.token_unrecorded) else {
                continue;
            };
            let Some(EventFeedScriptChange::Decision(decision)) = script.change else {
                continue;
            };
            if !matches!(
                decision.operation,
                Operation::Baseline | Operation::Candidate
            ) {
                continue;
            }
            let (every, kinds) = clears.entry(event.actor.id).or_default();
            if decision.operation == Operation::Baseline
                || (decision.flags & ALL != 0 && decision.flags & CLEAR_ALL_CANDIDATES != 0)
            {
                *every = Some(step.event);
                continue;
            }
            // Validation keeps candidate kinds below four.
            let kind = decision.container as usize;
            if decision.flags & ALL != 0 {
                kinds[kind] = Some(step.event);
            } else if let Some(cancelled) = [*every, kinds[kind]].into_iter().flatten().min() {
                step.flags |= CANCELLED;
                step.cancelled = cancelled;
            }
        }
        Self { steps }
    }
}

/// The decision owner a run of a pack's changes reduces.
struct Run<'a> {
    key: (u32, u32),
    /// The run made the owner, and has not reduced a change of it yet.
    made: bool,
    /// The run touched it.
    touched: bool,
    owner: &'a mut super::decisions::DecisionState,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct ScriptEventState {
    #[serde(with = "owner_pairs")]
    decisions: BTreeMap<(u32, u32), super::decisions::DecisionState>,
    #[serde(with = "owner_pairs")]
    delayed: BTreeMap<(u32, u32), delayed::Owner>,
    generation: Option<u64>,
    hour: Option<i32>,
    sequence: Option<u64>,
    pending: Vec<PendingEvent>,
    fired: Vec<ScriptIdentity>,
    pending_ready: bool,
    fired_ready: bool,
    pending_fault: bool,
    fired_fault: bool,
    pending_building: bool,
    fired_building: bool,
    #[serde(skip)]
    touched: Touched,
    #[serde(skip)]
    pending_version: Version,
    #[serde(skip)]
    fired_version: Version,
    /// The last version given.
    #[serde(skip)]
    clock: u64,
}
impl ScriptEventState {
    /// What the last observed row may have changed.
    pub fn touched(&self) -> &Touched {
        &self.touched
    }
    /// The pending instances' version: they hold the same, but their availability, while it
    /// stays the same.
    pub fn pending_version(&self) -> Option<u64> {
        self.pending_version.get()
    }
    /// The fired set's version, as [`Self::pending_version`].
    pub fn fired_version(&self) -> Option<u64> {
        self.fired_version.get()
    }
    /// Each decision owner's key, in order.
    pub fn decision_keys(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        self.decisions.keys().copied()
    }
    pub fn decision_owner(&self, key: (u32, u32)) -> Option<&super::decisions::DecisionState> {
        self.decisions.get(&key)
    }
    /// Each delayed-event owner's key, in order.
    pub fn delayed_keys(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        self.delayed.keys().copied()
    }
    /// The owner, its version (as [`Self::pending_version`], countdowns aside) and its timers.
    pub fn delayed_owner(
        &self,
        key: (u32, u32),
    ) -> Option<(
        EventFeedCountry,
        Option<u64>,
        Result<&[DelayedEvent], &'static str>,
    )> {
        self.delayed
            .get(&key)
            .map(|owner| (owner.country, owner.version.get(), owner.rows()))
    }
    pub fn decisions(
        &self,
        country: EventFeedCountry,
    ) -> Result<&super::decisions::DecisionState, &'static str> {
        self.decisions
            .range((country.id, 0)..=(country.id, u32::MAX))
            .next()
            .map(|(_, owner)| owner)
            .filter(|state| state.complete())
            .ok_or("decision owner baseline/writer certificate unavailable")
    }
    pub fn decision_owners(&self) -> impl Iterator<Item = &super::decisions::DecisionState> {
        self.decisions.values()
    }

    pub fn delayed(&self, country: EventFeedCountry) -> Result<&[DelayedEvent], &'static str> {
        self.delayed
            .range((country.id, 0)..=(country.id, u32::MAX))
            .next()
            .map(|(_, owner)| owner)
            .ok_or("country delayed-event baseline was not observed")?
            .rows()
    }
    pub fn delayed_owners(
        &self,
    ) -> impl Iterator<Item = (EventFeedCountry, Result<&[DelayedEvent], &'static str>)> {
        self.delayed
            .values()
            .map(|owner| (owner.country, owner.rows()))
    }
    pub fn pending(&self) -> Result<&[PendingEvent], &'static str> {
        self.pending_ready
            .then_some(self.pending.as_slice())
            .ok_or("pending event registry has no complete baseline and writer coverage")
    }
    /// Native membership retains duplicates; callers may sort for set display
    /// without inventing an occurrence time for pre-attachment members.
    pub fn fired(&self) -> Result<&[ScriptIdentity], &'static str> {
        self.fired_ready
            .then_some(self.fired.as_slice())
            .ok_or("fired_event_names has no complete baseline and writer coverage")
    }
    pub fn hour(&self) -> Option<i32> {
        self.hour
    }
    pub fn observe(&mut self, row: &EventFeedHistoryRow) -> io::Result<()> {
        self.observe_decisions(row, |_, _| Ok(()))
    }
    /// Visit each decision boundary after reducing its preceding changes; a candidate change
    /// that leaves its owner as the change before it left it is no boundary.
    pub fn observe_decisions(
        &mut self,
        row: &EventFeedHistoryRow,
        mut visit: impl FnMut(&super::decisions::DecisionState, Change<'_>) -> io::Result<()>,
    ) -> io::Result<()> {
        row.validate()?;
        let plan = Plan::of(row, None);
        self.reduce(
            Reading::of(row),
            &RowPlan { row, plan: &plan },
            true,
            &mut visit,
        )
    }
    /// [`Self::observe_decisions`] of a pack validated when it was prepared, the pack `held`
    /// admitted last. `sure`: every earlier pack of its walk was reduced to its end, so what they
    /// wrote holds.
    pub fn observe_prepared(
        &mut self,
        prepared: &PreparedHistory,
        held: &HeldPayloads,
        sure: bool,
        mut visit: impl FnMut(&super::decisions::DecisionState, Change<'_>) -> io::Result<()>,
    ) -> io::Result<()> {
        let admitted = Admitted::of(prepared, held)?;
        self.reduce(prepared.header.reading(), &admitted, sure, &mut visit)
    }
    fn reduce(
        &mut self,
        row: Reading,
        plan: &impl Planned,
        sure: bool,
        visit: &mut impl FnMut(&super::decisions::DecisionState, Change<'_>) -> io::Result<()>,
    ) -> io::Result<()> {
        self.touched = Touched::default();
        if self.generation != Some(row.generation) {
            *self = Self {
                generation: Some(row.generation),
                clock: self.clock,
                ..Self::default()
            };
            self.touched.all = true;
        }
        if self
            .hour
            .is_some_and(|hour| hour.checked_add(1) != Some(row.read_hour))
        {
            self.touched.all = true;
            for owner in self.decisions.values_mut() {
                owner.gap();
            }
            self.pending_ready = false;
            self.fired_ready = false;
            for owner in self.delayed.values_mut() {
                owner.gap();
            }
        }
        // A loss known on this row already makes the retained queues stale;
        // its deltas must not meet them before the row's own baselines.
        if row.coverage != EventFeedCoverage::Complete || row.failed {
            self.touched.all = true;
            for owner in self.delayed.values_mut() {
                owner.gap();
            }
        }
        // Validation keeps a pack's occurrences in order: only its first can repeat one seen.
        if let Some((first, _)) = row.sequences
            && self.sequence.is_some_and(|last| first <= last)
        {
            return Err(invalid("script state occurrence order differs"));
        }
        let mut baselines = (false, false);
        if let Err((at, sequence, error)) =
            self.steps(row.read_hour, plan, sure, visit, &mut baselines)
        {
            self.sequence = Some(sequence);
            self.restore_cancelled(plan, at);
            return Err(error);
        }
        if let Some((_, last)) = row.sequences {
            self.sequence = Some(last);
        }
        let (pending_baseline, fired_baseline) = baselines;
        if row.coverage == EventFeedCoverage::Unavailable
            || row.failed
            || row.script_refusals != 0
            || row.coverage == EventFeedCoverage::Partial
        {
            self.touched.all = true;
        }
        if row.coverage == EventFeedCoverage::Unavailable || row.failed {
            for owner in self.decisions.values_mut() {
                owner.gap();
            }
            self.pending_ready = false;
            self.fired_ready = false;
            for owner in self.delayed.values_mut() {
                owner.gap();
            }
        }
        if row.script_refusals != 0 {
            for owner in self.decisions.values_mut() {
                owner.gap();
            }
        }
        if row.coverage == EventFeedCoverage::Partial {
            for owner in self.decisions.values_mut() {
                owner.partial(row.read_hour);
            }
            self.pending_ready &= pending_baseline;
            self.fired_ready &= fired_baseline;
            for owner in self.delayed.values_mut() {
                owner.partial(row.read_hour);
            }
        }
        self.hour = Some(row.read_hour);
        self.stamp();
        Ok(())
    }
    /// The plan's steps in order, changes of the read at `read_hour`, or the event a failing one
    /// stopped at and its sequence.
    fn steps(
        &mut self,
        read_hour: i32,
        plan: &impl Planned,
        sure: bool,
        visit: &mut impl FnMut(&super::decisions::DecisionState, Change<'_>) -> io::Result<()>,
        (pending_baseline, fired_baseline): &mut (bool, bool),
    ) -> Result<(), (usize, u64, io::Error)> {
        let mut decided: Option<Run<'_>> = None;
        let mut timed: Option<((u32, u32), &mut delayed::Owner)> = None;
        for index in 0..plan.len() {
            let (step, occurrence) = plan.step(index);
            let at = step.event as usize;
            let fail = |error| (at, occurrence.sequence, error);
            let event = occurrence.event;
            let Some(script) = event.script else {
                continue;
            };
            let Some(change) = script.change else {
                continue;
            };
            use EventFeedScriptChange::*;
            match change {
                PendingBaseline | PendingStored { .. } | PendingReady | PendingRemoved => {
                    self.touched.pending = true;
                }
                FiredBaseline | FiredMember | FiredRemoved | FiredReady => {
                    self.touched.fired = true;
                }
                _ => {}
            }
            match change {
                Decision(change) => {
                    let write = step.flags & TARGETED != 0
                        && step.flags & REWRITE == 0
                        && !(sure && step.flags & WRITTEN != 0);
                    if step.flags & (QUIET | CANCELLED) == QUIET | CANCELLED && !write {
                        continue;
                    }
                    if decided
                        .as_ref()
                        .is_none_or(|run| run.key.0 != event.actor.id)
                    {
                        let key = owner_key(&self.decisions, event.actor);
                        let made = !self.decisions.contains_key(&key);
                        decided = Some(Run {
                            key,
                            made,
                            touched: false,
                            owner: self.decisions.entry(key).or_default(),
                        });
                    }
                    let run = decided.as_mut().expect("a reduced decision owner");
                    let before = (!run.made).then(|| (run.owner.country, run.owner.complete()));
                    run.made = false;
                    let cancelled = step.flags & CANCELLED != 0;
                    run.owner
                        .apply(read_hour, occurrence, script, change, write, cancelled)
                        .map_err(fail)?;
                    // A rebuild's roster is its own occurrence: it changes the retained state
                    // only when it makes, renames or loses its owner.
                    if !run.touched
                        && (change.operation != Operation::Rebuild
                            || before != Some((run.owner.country, run.owner.complete())))
                    {
                        self.touched.decisions.insert(run.key);
                        run.touched = true;
                    }
                    visit(&*run.owner, occurrence).map_err(fail)?;
                }
                PendingBaseline => {
                    self.pending.clear();
                    self.pending_version.change();
                    self.pending_ready = false;
                    self.pending_fault = false;
                    self.pending_building = true;
                }
                PendingStored { due_hour } => {
                    if script.token_unrecorded || script.scope_unrecorded || script.scope_len == 0 {
                        self.pending_fault = true;
                        self.pending_ready = false;
                        continue;
                    }
                    let scopes = occurrence.scopes(script);
                    allocation::reserve_exact(&mut self.pending, 1).map_err(fail)?;
                    self.pending.push(PendingEvent {
                        instance: script
                            .instance
                            .ok_or_else(|| fail(invalid("pending event has no instance")))?,
                        definition: occurrence.identity(script).map_err(fail)?,
                        originator: event.actor,
                        due_hour,
                        scopes: allocation::copy_vec(scopes).map_err(fail)?,
                    });
                    self.pending_version.change();
                }
                PendingReady => {
                    if !self.pending_building {
                        return Err(fail(invalid("pending registry completion has no baseline")));
                    }
                    self.pending_building = false;
                    self.pending_ready = !self.pending_fault;
                    *pending_baseline = true;
                }
                PendingRemoved => {
                    let instance = script
                        .instance
                        .ok_or_else(|| fail(invalid("pending removal has no instance")))?;
                    if let Some(index) =
                        self.pending.iter().position(|row| row.instance == instance)
                    {
                        // Both native removal writers replace with the last entry.
                        self.pending.swap_remove(index);
                        self.pending_version.change();
                    } else if self.pending_ready {
                        self.pending_fault = true;
                        self.pending_ready = false;
                    }
                }
                FiredBaseline => {
                    self.fired.clear();
                    self.fired_version.change();
                    self.fired_ready = false;
                    self.fired_fault = false;
                    self.fired_building = true;
                }
                FiredMember => {
                    if script.token_unrecorded {
                        self.fired_fault = true;
                        self.fired_ready = false;
                        continue;
                    }
                    allocation::reserve_exact(&mut self.fired, 1).map_err(fail)?;
                    self.fired.push(occurrence.identity(script).map_err(fail)?);
                    self.fired_version.change();
                }
                FiredRemoved => {
                    if script.token_unrecorded {
                        self.fired_fault = true;
                        self.fired_ready = false;
                        continue;
                    }
                    let removed = occurrence.identity(script).map_err(fail)?;
                    if let Some(index) = self.fired.iter().position(|member| *member == removed) {
                        // Native replacement erases one matched definition, not every
                        // equal identity. Initial duplicates retain multiplicity.
                        self.fired.remove(index);
                        self.fired_version.change();
                    } else if self.fired_ready {
                        self.fired_fault = true;
                        self.fired_ready = false;
                    }
                }
                FiredReady => {
                    if !self.fired_building {
                        return Err(fail(invalid("fired registry completion has no baseline")));
                    }
                    self.fired_building = false;
                    self.fired_ready = !self.fired_fault;
                    *fired_baseline = true;
                }
                DelayedBaseline
                | DelayedReady
                | DelayedEmpty
                | DelayedStored { .. }
                | DelayedAdvance { .. }
                | DelayedRemoved { .. }
                | DelayedRefused => {
                    if timed
                        .as_ref()
                        .is_none_or(|(key, _)| key.0 != event.target.id)
                    {
                        let key = owner_key(&self.delayed, event.target);
                        self.touched.delayed.insert(key);
                        timed = Some((key, self.delayed.entry(key).or_default()));
                    }
                    let (_, owner) = timed.as_mut().expect("a reduced delayed-event owner");
                    owner
                        .apply(read_hour, occurrence, script, change)
                        .map_err(fail)?;
                }
            }
        }
        Ok(())
    }
    /// The candidate changes the plan left to a clear or baseline of the pack that the
    /// reduction, stopping at `at`, never reached.
    fn restore_cancelled(&mut self, plan: &impl Planned, at: usize) {
        for index in 0..plan.len() {
            let (step, occurrence) = plan.step(index);
            if step.event as usize >= at {
                break;
            }
            if step.flags & CANCELLED == 0 || step.cancelled as usize <= at {
                continue;
            }
            let event = occurrence.event;
            let Some(script) = event.script else {
                continue;
            };
            let Some(EventFeedScriptChange::Decision(change)) = script.change else {
                continue;
            };
            let token = occurrence.token(script);
            let key = owner_key(&self.decisions, event.actor);
            if let Some(owner) = self.decisions.get_mut(&key) {
                owner.candidate(token, event, script, change);
            }
        }
    }
    /// Gives each part the row may have changed a version of its own.
    fn stamp(&mut self) {
        let clock = &mut self.clock;
        self.pending_version.stamp(clock);
        self.fired_version.stamp(clock);
        if self.touched.all {
            for owner in self.decisions.values_mut() {
                owner.version.stamp(clock);
            }
            for owner in self.delayed.values_mut() {
                owner.version.stamp(clock);
            }
            return;
        }
        for key in &self.touched.decisions {
            if let Some(owner) = self.decisions.get_mut(key) {
                owner.version.stamp(clock);
            }
        }
        for key in &self.touched.delayed {
            if let Some(owner) = self.delayed.get_mut(key) {
                owner.version.stamp(clock);
            }
        }
    }
}
// Continuations retain the first raw witness; only the published link identifies its owner.
fn owner_key<T>(owners: &BTreeMap<(u32, u32), T>, country: EventFeedCountry) -> (u32, u32) {
    owners
        .range((country.id, 0)..=(country.id, u32::MAX))
        .next()
        .map_or((country.id, country.instance), |(key, _)| *key)
}
#[cfg(test)]
fn identity(row: &EventFeedHistoryRow, script: EventFeedScript) -> io::Result<ScriptIdentity> {
    let start = script.token_start as usize;
    let end = start + script.token_len as usize;
    Ok(ScriptIdentity {
        native_event: script
            .native_event
            .ok_or_else(|| invalid("script registry event identity absent"))?,
        token: allocation::copy_vec(&row.script_tokens[start..end])?,
    })
}

#[cfg(test)]
mod tests {
    use super::super::decisions::{DecisionChange, Operation};
    use super::*;
    use EventFeedScriptChange::*;
    /// Owner refusals carry this reason after the shared definition token.
    const REASON: &[u8] = b"synthetic owner refusal";
    fn refusal(change: &EventFeedScriptChange) -> bool {
        matches!(
            change,
            DelayedRefused
                | Decision(DecisionChange {
                    operation: Operation::Refused,
                    ..
                })
        )
    }
    fn row(
        hour: i32,
        first: u64,
        changes: &[(EventFeedScriptChange, Option<u32>, Option<u32>)],
    ) -> EventFeedHistoryRow {
        EventFeedHistoryRow {
            generation: 7,
            read_hour: hour,
            coverage: EventFeedCoverage::Complete,
            counts: Some(EventFeedCounts {
                acquired: 1000,
                settled: 1000,
                ..Default::default()
            }),
            script_tokens: [b"synthetic.1".as_slice(), REASON].concat(),
            script_scopes: vec![EventFeedScope::default()],
            events: changes
                .iter()
                .enumerate()
                .map(|(n, (change, instance, id))| EventFeedEvent {
                    sequence: first + n as u64,
                    generation: 7,
                    hour,
                    source: EventFeedSource::ScriptState,
                    actor: if matches!(change, PendingStored { .. } | DelayedStored { .. }) {
                        EventFeedCountry {
                            reference: 3,
                            id: 2,
                            instance: 9,
                        }
                    } else {
                        Default::default()
                    },
                    target: if matches!(
                        change,
                        DelayedBaseline
                            | DelayedReady
                            | DelayedEmpty
                            | DelayedStored { .. }
                            | DelayedAdvance { .. }
                            | DelayedRemoved { .. }
                            | DelayedRefused
                    ) {
                        EventFeedCountry {
                            reference: 6,
                            id: 5,
                            instance: 7,
                        }
                    } else {
                        Default::default()
                    },
                    cause: Recorded::Value(Default::default()),
                    script: Some(EventFeedScript {
                        token_start: if refusal(change) { 11 } else { 0 },
                        token_len: if refusal(change) {
                            REASON.len() as u32
                        } else if id.is_some() {
                            11
                        } else {
                            0
                        },
                        scope_len: u32::from(matches!(
                            change,
                            PendingStored { .. } | DelayedStored { .. }
                        )),
                        native_event: id.map(|id| EventFeedEventIdentity { kind: 99, id }),
                        instance: *instance,
                        change: Some(*change),
                        ..Default::default()
                    }),
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn surrender_recipient_changes_keep_one_delayed_owner_and_recover_after_a_gap() {
        let mut state = ScriptEventState::default();
        state
            .observe(&row(
                10,
                0,
                &[
                    (DelayedBaseline, None, None),
                    (DelayedStored { remaining: 9 }, Some(1), Some(91)),
                    (DelayedReady, None, None),
                ],
            ))
            .unwrap();
        let mut next = row(
            11,
            3,
            &[(DelayedStored { remaining: 8 }, Some(2), Some(92))],
        );
        next.events[0].target.instance = 1;
        let owner = next.events[0].target;
        state.observe(&next).unwrap();
        assert_eq!(state.delayed(owner).unwrap().len(), 2);
        assert_eq!(state.delayed_owners().count(), 1);
        let reused = EventFeedCountry {
            id: 1501,
            reference: 1501,
            ..owner
        };
        assert!(state.delayed(reused).is_err());
        state.observe(&row(13, 4, &[])).unwrap();
        assert!(state.delayed(owner).is_err());
        let mut recovered = row(14, 4, &[(DelayedEmpty, None, None)]);
        recovered.events[0].target = owner;
        state.observe(&recovered).unwrap();
        assert!(state.delayed(owner).unwrap().is_empty());
        assert_eq!(state.delayed_owners().count(), 1);
    }
    #[test]
    fn surrender_recipient_changes_keep_one_decision_owner() {
        let mut state = ScriptEventState::default();
        let change = |operation| {
            (
                Decision(DecisionChange {
                    operation,
                    container: 0,
                    flags: 0,
                    ordinal: 0,
                    political_power: None,
                }),
                None,
                None,
            )
        };
        let mut baseline = row(
            10,
            0,
            &[change(Operation::Baseline), change(Operation::Ready)],
        );
        let owner = EventFeedCountry {
            reference: 24,
            id: 24,
            instance: 0,
        };
        for event in &mut baseline.events {
            event.actor = owner;
        }
        state.observe(&baseline).unwrap();
        assert!(
            state
                .decisions(EventFeedCountry {
                    instance: 1,
                    ..owner
                })
                .is_ok()
        );
        assert_eq!(state.decision_owners().count(), 1);
        assert!(
            state
                .decisions(EventFeedCountry {
                    reference: 1501,
                    id: 1501,
                    ..owner
                })
                .is_err()
        );
    }
    #[test]
    fn refused_decision_families_do_not_poison_independent_registry_certificates() {
        let mut state = ScriptEventState::default();
        let mut baseline = row(
            10,
            0,
            &[
                (PendingBaseline, None, None),
                (PendingReady, None, None),
                (FiredBaseline, None, None),
                (FiredReady, None, None),
            ],
        );
        baseline.script_refusals = DECISION_FAMILY_REFUSALS;
        state.observe(&baseline).unwrap();
        assert!(state.pending().unwrap().is_empty());
        assert!(state.fired().unwrap().is_empty());
        let mut next = row(11, 4, &[]);
        next.script_refusals = DECISION_FAMILY_REFUSALS;
        state.observe(&next).unwrap();
        assert_eq!(next.refused_script_families().count(), 5);
        assert!(state.pending().unwrap().is_empty());
        assert!(state.fired().unwrap().is_empty());
    }
    #[test]
    fn sparse_pass_cancellation_behind_its_cursor_closes_without_decrementing_a_swapped_tail() {
        let owner = EventFeedCountry {
            reference: 6,
            id: 5,
            instance: 7,
        };
        let mut state = ScriptEventState::default();
        state
            .observe(&row(
                1,
                0,
                &[
                    (DelayedBaseline, None, None),
                    (DelayedStored { remaining: 9 }, Some(1), Some(91)),
                    (DelayedStored { remaining: 9 }, Some(2), Some(92)),
                    (DelayedStored { remaining: 9 }, Some(3), Some(93)),
                    (DelayedReady, None, None),
                ],
            ))
            .unwrap();
        state
            .observe(&row(
                2,
                5,
                &[
                    (
                        DelayedAdvance {
                            count: 2,
                            finished: false,
                        },
                        None,
                        None,
                    ),
                    (
                        DelayedRemoved {
                            iteration: false,
                            visited: false,
                        },
                        Some(1),
                        None,
                    ),
                    (
                        DelayedRemoved {
                            iteration: false,
                            visited: false,
                        },
                        Some(2),
                        None,
                    ),
                    (
                        DelayedAdvance {
                            count: 0,
                            finished: true,
                        },
                        None,
                        None,
                    ),
                ],
            ))
            .unwrap();
        let timers = state.delayed(owner).unwrap();
        assert_eq!(timers.len(), 1);
        assert_eq!((timers[0].instance, timers[0].remaining), (3, 9));
    }
    #[test]
    fn fired_replacement_erases_one_member_without_reset_and_survives_codec_round_trip() {
        let mut state = ScriptEventState::default();
        state
            .observe(&row(
                10,
                0,
                &[
                    (FiredBaseline, None, None),
                    (FiredMember, None, Some(91)),
                    (FiredMember, None, Some(91)),
                    (FiredMember, None, Some(92)),
                    (FiredReady, None, None),
                ],
            ))
            .unwrap();
        let delta = row(
            11,
            5,
            &[
                (FiredRemoved, None, Some(91)),
                (FiredMember, None, Some(93)),
            ],
        );
        let mut encoded = Vec::new();
        delta.encode(&mut encoded).unwrap();
        state
            .observe(&EventFeedHistoryRow::decode(&mut Decoder::new(&encoded)).unwrap())
            .unwrap();
        assert_eq!(
            state
                .fired()
                .unwrap()
                .iter()
                .map(|member| member.native_event.id)
                .collect::<Vec<_>>(),
            [91, 92, 93]
        );
        state
            .observe(&row(12, 7, &[(FiredRemoved, None, Some(91))]))
            .unwrap();
        assert_eq!(
            state
                .fired()
                .unwrap()
                .iter()
                .map(|member| member.native_event.id)
                .collect::<Vec<_>>(),
            [92, 93]
        );
        state
            .observe(&row(13, 8, &[(FiredRemoved, None, Some(99))]))
            .unwrap();
        assert!(
            state.fired().is_err(),
            "an erased unknown member invalidates coverage"
        );
    }
    #[test]
    fn registry_baseline_and_deltas_retain_duplicate_instances_and_fire_once_members_without_inventing_fires()
     {
        let mut state = ScriptEventState::default();
        let mut baseline = row(
            10,
            0,
            &[
                (PendingBaseline, None, None),
                (PendingStored { due_hour: -25 }, Some(3), Some(90)),
                (PendingStored { due_hour: 50 }, Some(4), Some(91)),
                (PendingStored { due_hour: 60 }, Some(3), Some(92)),
                (PendingReady, None, None),
                (FiredBaseline, None, None),
                (FiredMember, None, Some(90)),
                (FiredMember, None, Some(90)),
                (FiredReady, None, None),
            ],
        );
        baseline.coverage = EventFeedCoverage::Partial;
        let mut bytes = Vec::new();
        baseline.encode(&mut bytes).unwrap();
        let decoded = EventFeedHistoryRow::decode(&mut Decoder::new(&bytes)).unwrap();
        assert_eq!(decoded, baseline);
        state.observe(&decoded).unwrap();
        assert_eq!(
            state
                .pending()
                .unwrap()
                .iter()
                .map(|row| row.definition.native_event.id)
                .collect::<Vec<_>>(),
            [90, 91, 92]
        );
        assert_eq!(state.pending().unwrap()[0].originator.instance, 9);
        assert_eq!(state.pending().unwrap()[0].due_hour, -25);
        assert_eq!(state.fired().unwrap().len(), 2);
        let delta = row(
            11,
            9,
            &[
                (PendingRemoved, Some(3), None),
                (FiredMember, None, Some(91)),
            ],
        );
        state.observe(&delta).unwrap();
        assert_eq!(
            state
                .pending()
                .unwrap()
                .iter()
                .map(|row| row.definition.native_event.id)
                .collect::<Vec<_>>(),
            [92, 91]
        );
        assert_eq!(state.fired().unwrap().len(), 3);
        state.observe(&row(12, 12, &[])).unwrap();
        assert_eq!(state.pending().unwrap().len(), 2);
        for end in 0..bytes.len() {
            assert!(EventFeedHistoryRow::decode(&mut Decoder::new(&bytes[..end])).is_err());
        }
    }
    #[test]
    fn registry_gaps_unknown_operands_and_world_replacement_require_a_new_baseline() {
        let mut state = ScriptEventState::default();
        state
            .observe(&row(
                1,
                0,
                &[
                    (PendingBaseline, None, None),
                    (PendingReady, None, None),
                    (FiredBaseline, None, None),
                    (FiredReady, None, None),
                ],
            ))
            .unwrap();
        assert!(state.pending().unwrap().is_empty());
        assert!(state.fired().unwrap().is_empty());
        state.observe(&row(3, 4, &[])).unwrap();
        assert!(state.pending().is_err());
        assert!(state.fired().is_err());
        let mut partial = row(
            4,
            4,
            &[
                (PendingBaseline, None, None),
                (PendingStored { due_hour: 5 }, Some(1), Some(90)),
                (PendingReady, None, None),
            ],
        );
        let script = partial.events[1].script.as_mut().unwrap();
        script.scope_len = 0;
        script.scope_unrecorded = true;
        state.observe(&partial).unwrap();
        assert!(state.pending().is_err());
        let mut next = row(
            5,
            0,
            &[(PendingBaseline, None, None), (PendingReady, None, None)],
        );
        next.generation = 8;
        for event in &mut next.events {
            event.generation = 8;
        }
        state.observe(&next).unwrap();
        assert!(state.pending().unwrap().is_empty());
        assert!(state.fired().is_err());
        assert!(
            state
                .observe(&row(6, 6, &[(PendingReady, None, None)]))
                .is_err()
        );
    }
    #[test]
    fn registry_delayed_prefixes_replay_in_pass_schedule_cancel_swap_and_signed_countdowns() {
        let owner = EventFeedCountry {
            reference: 6,
            id: 5,
            instance: 7,
        };
        let mut state = ScriptEventState::default();
        state
            .observe(&row(
                1,
                0,
                &[
                    (DelayedBaseline, None, None),
                    (DelayedStored { remaining: 1 }, Some(1), Some(90)),
                    (DelayedStored { remaining: 5 }, Some(2), Some(91)),
                    (DelayedReady, None, None),
                ],
            ))
            .unwrap();
        // Fire the first timer; its immediate effects append another timer.
        // Swap-removal visits the newly appended timer in this same pass.
        state
            .observe(&row(
                2,
                4,
                &[
                    (
                        DelayedAdvance {
                            count: 1,
                            finished: false,
                        },
                        None,
                        None,
                    ),
                    (DelayedStored { remaining: 9 }, Some(3), Some(92)),
                    (
                        DelayedRemoved {
                            iteration: true,
                            visited: true,
                        },
                        Some(1),
                        None,
                    ),
                    (
                        DelayedAdvance {
                            count: 2,
                            finished: true,
                        },
                        None,
                        None,
                    ),
                ],
            ))
            .unwrap();
        let remaining = |state: &ScriptEventState| {
            state
                .delayed(owner)
                .unwrap()
                .iter()
                .map(|timer| (timer.instance, timer.remaining))
                .collect::<Vec<_>>()
        };
        assert_eq!(remaining(&state), [(3, 8), (2, 4)]);
        state
            .observe(&row(
                3,
                8,
                &[(
                    DelayedAdvance {
                        count: 2,
                        finished: true,
                    },
                    None,
                    None,
                )],
            ))
            .unwrap();
        assert_eq!(remaining(&state), [(3, 7), (2, 3)]);
        state
            .observe(&row(
                4,
                9,
                &[
                    (
                        DelayedAdvance {
                            count: 1,
                            finished: false,
                        },
                        None,
                        None,
                    ),
                    (
                        DelayedRemoved {
                            iteration: false,
                            visited: false,
                        },
                        Some(2),
                        None,
                    ),
                    (DelayedStored { remaining: 7 }, Some(4), Some(93)),
                    (
                        DelayedAdvance {
                            count: 1,
                            finished: true,
                        },
                        None,
                        None,
                    ),
                ],
            ))
            .unwrap();
        assert_eq!(remaining(&state), [(3, 6), (4, 6)]);
        state
            .observe(&row(
                5,
                13,
                &[
                    (
                        DelayedAdvance {
                            count: 1,
                            finished: false,
                        },
                        None,
                        None,
                    ),
                    (
                        DelayedRemoved {
                            iteration: true,
                            visited: false,
                        },
                        Some(4),
                        None,
                    ),
                    (
                        DelayedAdvance {
                            count: 0,
                            finished: true,
                        },
                        None,
                        None,
                    ),
                ],
            ))
            .unwrap();
        assert_eq!(remaining(&state), [(3, 5)]);
        let mut wrapped = row(
            6,
            16,
            &[
                (DelayedEmpty, None, None),
                (
                    DelayedStored {
                        remaining: i32::MIN,
                    },
                    Some(5),
                    Some(94),
                ),
                (
                    DelayedAdvance {
                        count: 1,
                        finished: true,
                    },
                    None,
                    None,
                ),
            ],
        );
        let mut bytes = Vec::new();
        wrapped.encode(&mut bytes).unwrap();
        wrapped = EventFeedHistoryRow::decode(&mut Decoder::new(&bytes)).unwrap();
        state.observe(&wrapped).unwrap();
        assert_eq!(remaining(&state), [(5, i32::MAX)]);
        state
            .observe(&row(
                7,
                19,
                &[(
                    DelayedAdvance {
                        count: 1,
                        finished: false,
                    },
                    None,
                    None,
                )],
            ))
            .unwrap();
        assert!(
            state.delayed(owner).is_err(),
            "an unclosed native pass cannot certify the boundary"
        );
    }
    const DELAYED_OWNER: EventFeedCountry = EventFeedCountry {
        reference: 6,
        id: 5,
        instance: 7,
    };
    type Change = (EventFeedScriptChange, Option<u32>, Option<u32>);
    fn lost(mut row: EventFeedHistoryRow, coverage: EventFeedCoverage) -> EventFeedHistoryRow {
        row.coverage = coverage;
        if coverage == EventFeedCoverage::Unavailable {
            row.failure = Some(b"synthetic observation overlap".to_vec());
        }
        row
    }
    fn advance(count: u32, finished: bool) -> Change {
        (DelayedAdvance { count, finished }, None, None)
    }
    fn retire(instance: u32, iteration: bool, visited: bool) -> Change {
        (DelayedRemoved { iteration, visited }, Some(instance), None)
    }
    fn store(remaining: i32, instance: u32, definition: u32) -> Change {
        (
            DelayedStored { remaining },
            Some(instance),
            Some(definition),
        )
    }
    /// Every row also passes through the value codec, the table field codec
    /// and a table patch against its predecessor; all four reductions keep
    /// the same state and readiness.
    #[derive(Default)]
    struct Reductions([ScriptEventState; 4], Option<EventFeedHistoryRow>);
    impl Reductions {
        fn observe(&mut self, row: &EventFeedHistoryRow) -> io::Result<()> {
            use crate::rows::RowCodec;
            let mut value = Vec::new();
            row.encode(&mut value)?;
            let mut fields = Vec::new();
            row.encode_fields(&mut fields)?;
            let patch = row
                .delta(self.1.as_ref())?
                .ok_or_else(|| invalid("synthetic rows always differ"))?;
            let rows = [
                row.clone(),
                EventFeedHistoryRow::decode(&mut Decoder::new(&value))?,
                EventFeedHistoryRow::decode_fields(0, &mut Decoder::new(&fields))?,
                EventFeedHistoryRow::apply_delta(&patch, self.1.take())?,
            ];
            for (state, row) in self.0.iter_mut().zip(&rows) {
                state.observe(row)?;
            }
            let expected = format!("{:?}", self.0[0]);
            for state in &self.0[1..] {
                assert_eq!(format!("{state:?}"), expected);
            }
            self.1 = Some(row.clone());
            Ok(())
        }
        fn timers(&self) -> Result<Vec<(u32, u32, i32)>, &'static str> {
            self.timers_of(DELAYED_OWNER)
        }
        fn timers_of(&self, owner: EventFeedCountry) -> Result<Vec<(u32, u32, i32)>, &'static str> {
            self.0[0].delayed(owner).map(|timers| {
                timers
                    .iter()
                    .map(|timer| {
                        (
                            timer.instance,
                            timer.definition.native_event.id,
                            timer.remaining,
                        )
                    })
                    .collect()
            })
        }
    }
    #[test]
    fn delayed_writer_deltas_without_a_baseline_keep_their_owner_unavailable() {
        let mut state = Reductions::default();
        // Partial startup: a store, then an orphan pass and retirements,
        // before the owner's first census.
        state
            .observe(&lost(
                row(1, 0, &[store(360, 1, 31)]),
                EventFeedCoverage::Partial,
            ))
            .unwrap();
        assert!(state.timers().is_err());
        state
            .observe(&row(
                2,
                1,
                &[advance(2, false), retire(1, true, true), advance(0, true)],
            ))
            .unwrap();
        state
            .observe(&row(3, 4, &[retire(4, false, false)]))
            .unwrap();
        assert!(state.timers().is_err());
        // A loss between rows, then a different timer under a reused label
        // in a Complete interval: no uncertified timer is kept to collide.
        state.observe(&row(5, 5, &[store(1, 1, 20)])).unwrap();
        // The same on a row that itself records the loss.
        state
            .observe(&lost(
                row(6, 6, &[store(5, 1, 21)]),
                EventFeedCoverage::Unavailable,
            ))
            .unwrap();
        // Complete transport alone restores nothing.
        for hour in 7..9 {
            state.observe(&row(hour, 7 + hour as u64, &[])).unwrap();
            assert!(state.timers().is_err());
        }
        // A native queue reset opens a baseline; a loss interrupts it, so its
        // timers must not meet later stores either.
        state
            .observe(&row(
                9,
                20,
                &[(DelayedBaseline, None, None), store(9, 2, 22)],
            ))
            .unwrap();
        assert!(state.timers().is_err());
        state
            .observe(&lost(
                row(10, 22, &[store(8, 2, 23)]),
                EventFeedCoverage::Unavailable,
            ))
            .unwrap();
        state.observe(&row(11, 23, &[store(8, 2, 23)])).unwrap();
        assert!(state.timers().is_err());
        // The explicit census baseline restores exact ordered state.
        state
            .observe(&row(
                12,
                24,
                &[
                    (DelayedBaseline, None, None),
                    store(330, 3, 31),
                    store(8, 2, 23),
                    (DelayedReady, None, None),
                ],
            ))
            .unwrap();
        assert_eq!(state.timers().unwrap(), [(3, 31, 330), (2, 23, 8)]);
        state.observe(&row(13, 28, &[advance(2, true)])).unwrap();
        assert_eq!(state.timers().unwrap(), [(3, 31, 329), (2, 23, 7)]);
    }
    #[test]
    fn delayed_loss_on_the_current_row_retires_a_certified_queue_before_its_deltas() {
        let mut state = Reductions::default();
        let baseline = |hour, first| {
            row(
                hour,
                first,
                &[
                    (DelayedBaseline, None, None),
                    store(10, 1, 40),
                    (DelayedReady, None, None),
                ],
            )
        };
        state.observe(&baseline(1, 0)).unwrap();
        assert_eq!(state.timers().unwrap(), [(1, 40, 10)]);
        // A loss-qualified row neither certifies its deltas nor checks them
        // against the queue it has already made stale.
        for (hour, coverage) in [
            (2, EventFeedCoverage::Partial),
            (3, EventFeedCoverage::Unavailable),
        ] {
            state
                .observe(&lost(
                    row(
                        hour,
                        hour as u64 * 10,
                        &[store(3, 1, 41), retire(1, true, false)],
                    ),
                    coverage,
                ))
                .unwrap();
            assert!(state.timers().is_err());
        }
        state.observe(&baseline(4, 40)).unwrap();
        let mut failed = lost(row(5, 50, &[store(3, 1, 41)]), EventFeedCoverage::Partial);
        failed.failure = Some(b"synthetic writer refusal".to_vec());
        state.observe(&failed).unwrap();
        assert!(state.timers().is_err());
        // A baseline inside an Unavailable interval cannot certify it.
        state
            .observe(&lost(baseline(6, 60), EventFeedCoverage::Unavailable))
            .unwrap();
        assert!(state.timers().is_err());
        // A Partial interval's own baseline does, as before.
        state
            .observe(&lost(baseline(7, 70), EventFeedCoverage::Partial))
            .unwrap();
        assert_eq!(state.timers().unwrap(), [(1, 40, 10)]);
        state
            .observe(&lost(row(8, 80, &[]), EventFeedCoverage::Unavailable))
            .unwrap();
        // So does an empty certificate, after which writer deltas apply.
        state
            .observe(&row(9, 90, &[(DelayedEmpty, None, None), store(2, 1, 42)]))
            .unwrap();
        assert_eq!(state.timers().unwrap(), [(1, 42, 2)]);
    }
    #[test]
    fn delayed_certified_queue_keeps_equal_definitions_and_refuses_a_repeated_label() {
        let mut state = Reductions::default();
        let mut baseline = row(
            1,
            0,
            &[
                (DelayedBaseline, None, None),
                store(i32::MIN, 1, 90),
                store(1, 2, 90),
                store(7, 3, 90),
                (DelayedReady, None, None),
            ],
        );
        baseline.script_scopes = (0..3)
            .map(|n| EventFeedScope {
                random: [n, n + 10],
                ..Default::default()
            })
            .collect();
        for (n, event) in baseline.events[1..4].iter_mut().enumerate() {
            event.script.as_mut().unwrap().scope_start = n as u32;
        }
        state.observe(&baseline).unwrap();
        // The middle timer fires: the swapped tail is revisited in the same
        // pass, and the first timer's countdown wraps.
        state
            .observe(&row(
                2,
                5,
                &[advance(2, false), retire(2, true, true), advance(1, true)],
            ))
            .unwrap();
        assert_eq!(state.timers().unwrap(), [(1, 90, i32::MAX), (3, 90, 6)]);
        let scopes = state.0[0]
            .delayed(DELAYED_OWNER)
            .unwrap()
            .iter()
            .map(|timer| timer.scopes[0].random)
            .collect::<Vec<_>>();
        assert_eq!(scopes, [[0, 10], [2, 12]]);
        // Within one certified queue a repeated label stays corrupt, whether
        // it reaches a carried queue or one baseline.
        assert!(state.0[0].observe(&row(3, 8, &[store(4, 3, 90)])).is_err());
        assert!(
            ScriptEventState::default()
                .observe(&row(
                    1,
                    0,
                    &[
                        (DelayedBaseline, None, None),
                        store(4, 1, 90),
                        store(5, 1, 91),
                    ],
                ))
                .is_err()
        );
    }
    #[test]
    fn registry_quiet_delayed_pass_encodes_one_compact_count_for_every_timer() {
        let owner = EventFeedCountry {
            reference: 6,
            id: 5,
            instance: 7,
        };
        let mut changes = vec![(DelayedBaseline, None, None)];
        for id in 1..=50 {
            changes.push((DelayedStored { remaining: 20 }, Some(id), Some(id)));
        }
        changes.push((DelayedReady, None, None));
        let baseline = row(1, 0, &changes);
        let mut state = ScriptEventState::default();
        state.observe(&baseline).unwrap();
        let quiet = row(
            2,
            52,
            &[(
                DelayedAdvance {
                    count: 50,
                    finished: true,
                },
                None,
                None,
            )],
        );
        assert_eq!(quiet.events.len(), 1);
        assert!(quiet.events[0].encoded_bytes().unwrap() <= 20);
        state.observe(&quiet).unwrap();
        assert_eq!(state.delayed(owner).unwrap().len(), 50);
        assert!(
            state
                .delayed(owner)
                .unwrap()
                .iter()
                .all(|timer| timer.remaining == 19)
        );
    }
    fn owner(id: u32) -> EventFeedCountry {
        EventFeedCountry {
            reference: id + 1,
            id,
            instance: 7,
        }
    }
    fn decision(operation: Operation) -> Change {
        (Decision(DecisionChange::new(operation, 0)), None, None)
    }
    /// `row` with each occurrence's owner chosen: a delayed change's target,
    /// a decision change's actor. Registry changes ignore it.
    fn owned(hour: i32, first: u64, changes: &[(u32, Change)]) -> EventFeedHistoryRow {
        let plain: Vec<_> = changes.iter().map(|(_, change)| *change).collect();
        let mut row = row(hour, first, &plain);
        for (event, (id, _)) in row.events.iter_mut().zip(changes) {
            if matches!(
                event.script.and_then(|script| script.change),
                Some(Decision(_))
            ) {
                event.actor = owner(*id);
            } else if event.target != EventFeedCountry::default() {
                event.target = owner(*id);
            }
        }
        row
    }
    #[test]
    fn delayed_owner_refusal_retires_only_its_queue_until_its_next_baseline() {
        let (a, b, c) = (1, 2, 3);
        let mut state = Reductions::default();
        state
            .observe(&owned(
                1,
                0,
                &[
                    (a, (DelayedBaseline, None, None)),
                    (a, store(10, 1, 40)),
                    (a, (DelayedReady, None, None)),
                    (b, (DelayedBaseline, None, None)),
                    (b, store(20, 2, 41)),
                    (b, (DelayedReady, None, None)),
                    (c, (DelayedEmpty, None, None)),
                    (0, (PendingBaseline, None, None)),
                    (0, (PendingStored { due_hour: 50 }, Some(3), Some(90))),
                    (0, (PendingReady, None, None)),
                    (0, (FiredBaseline, None, None)),
                    (0, (FiredMember, None, Some(91))),
                    (0, (FiredReady, None, None)),
                ],
            ))
            .unwrap();
        assert_eq!(state.timers_of(owner(a)).unwrap(), [(1, 40, 10)]);
        let others = |state: &Reductions, b_timers: &[(u32, u32, i32)]| {
            assert_eq!(state.timers_of(owner(b)).unwrap(), b_timers);
            assert!(state.timers_of(owner(c)).unwrap().is_empty());
            let pending = state.0[0].pending().unwrap();
            assert_eq!(
                pending
                    .iter()
                    .map(|row| (row.instance, row.definition.native_event.id, row.due_hour))
                    .collect::<Vec<_>>(),
                [(3, 90, 50)]
            );
            assert_eq!(
                state.0[0]
                    .fired()
                    .unwrap()
                    .iter()
                    .map(|member| member.native_event.id)
                    .collect::<Vec<_>>(),
                [91]
            );
        };
        others(&state, &[(2, 41, 20)]);
        // A Complete row refuses A alone. A's later writer is not checked
        // against the retired queue; B's pass still applies.
        let refused = owned(
            2,
            13,
            &[
                (a, (DelayedRefused, None, None)),
                (a, store(5, 9, 42)),
                (b, advance(1, true)),
            ],
        );
        assert_eq!(refused.coverage, EventFeedCoverage::Complete);
        state.observe(&refused).unwrap();
        assert!(state.timers_of(owner(a)).is_err());
        others(&state, &[(2, 41, 19)]);
        // A's next baseline restores it.
        state
            .observe(&owned(
                3,
                16,
                &[
                    (a, (DelayedBaseline, None, None)),
                    (a, store(9, 1, 40)),
                    (a, (DelayedReady, None, None)),
                ],
            ))
            .unwrap();
        assert_eq!(state.timers_of(owner(a)).unwrap(), [(1, 40, 9)]);
        others(&state, &[(2, 41, 19)]);
        // A refusal also closes an open baseline.
        assert!(
            state.0[0]
                .observe(&owned(
                    4,
                    19,
                    &[
                        (a, (DelayedBaseline, None, None)),
                        (a, (DelayedRefused, None, None)),
                        (a, (DelayedReady, None, None)),
                    ],
                ))
                .is_err()
        );
    }
    #[test]
    fn decision_owner_refusal_retires_only_its_owner_and_closes_an_open_baseline() {
        let (a, b, c) = (1, 2, 3);
        let mut state = Reductions::default();
        state
            .observe(&owned(
                1,
                0,
                &[
                    (a, decision(Operation::Baseline)),
                    (a, decision(Operation::Ready)),
                    (b, decision(Operation::Baseline)),
                    (b, decision(Operation::Ready)),
                    // C's baseline stays open.
                    (c, decision(Operation::Baseline)),
                    (b, (DelayedEmpty, None, None)),
                    (0, (PendingBaseline, None, None)),
                    (0, (PendingReady, None, None)),
                ],
            ))
            .unwrap();
        assert!(state.0[0].decisions(owner(a)).is_ok());
        assert!(state.0[0].decisions(owner(c)).is_err());
        state
            .observe(&owned(
                2,
                8,
                &[
                    (a, decision(Operation::Refused)),
                    (c, decision(Operation::Refused)),
                ],
            ))
            .unwrap();
        assert!(state.0[0].decisions(owner(a)).is_err());
        assert!(state.0[0].decisions(owner(b)).is_ok());
        assert!(state.0[0].decisions(owner(c)).is_err());
        assert!(state.timers_of(owner(b)).unwrap().is_empty());
        assert!(state.0[0].pending().unwrap().is_empty());
        state
            .observe(&owned(
                3,
                10,
                &[
                    (a, decision(Operation::Baseline)),
                    (a, decision(Operation::Ready)),
                ],
            ))
            .unwrap();
        assert!(state.0[0].decisions(owner(a)).is_ok());
        assert!(state.0[0].decisions(owner(b)).is_ok());
        // C's refused baseline has nothing left to complete.
        assert!(
            state.0[0]
                .observe(&owned(4, 12, &[(c, decision(Operation::Ready))]))
                .is_err()
        );
    }
    #[test]
    fn one_owner_refusal_touches_only_its_owner_among_thousands() {
        for owners in [1_500_u32, 5_000] {
            let mut changes = Vec::new();
            for id in 1..=owners {
                changes.push((id, (DelayedEmpty, None, None)));
                changes.push((id, decision(Operation::Baseline)));
                changes.push((id, decision(Operation::Ready)));
            }
            let mut baseline = owned(1, 0, &changes);
            let total = 4 * u64::from(owners);
            baseline.counts = Some(EventFeedCounts {
                acquired: total,
                settled: total,
                ..Default::default()
            });
            let mut state = ScriptEventState::default();
            state.observe(&baseline).unwrap();
            let refused = owners / 2;
            state
                .observe(&owned(
                    2,
                    3 * u64::from(owners),
                    &[
                        (refused, (DelayedRefused, None, None)),
                        (refused + 1, decision(Operation::Refused)),
                    ],
                ))
                .unwrap();
            assert_eq!(state.delayed_owners().count(), owners as usize);
            assert_eq!(state.decision_owners().count(), owners as usize);
            assert_eq!(
                state
                    .delayed_owners()
                    .filter(|(_, rows)| rows.is_err())
                    .map(|(country, _)| country)
                    .collect::<Vec<_>>(),
                [owner(refused)]
            );
            assert_eq!(
                state
                    .decision_owners()
                    .filter(|state| !state.complete())
                    .map(|state| state.country)
                    .collect::<Vec<_>>(),
                [owner(refused + 1)]
            );
        }
    }
    /// Each row names the parts it may have changed: the delayed owner whose timers it
    /// passed and no other, and every part after a gap or a new generation.
    #[test]
    fn rows_name_the_parts_they_may_have_changed() {
        let mut state = ScriptEventState::default();
        state
            .observe(&row(
                10,
                0,
                &[(DelayedBaseline, None, None), (DelayedReady, None, None)],
            ))
            .unwrap();
        assert!(state.touched().all);
        state
            .observe(&row(
                11,
                2,
                &[(
                    DelayedAdvance {
                        count: 0,
                        finished: true,
                    },
                    None,
                    None,
                )],
            ))
            .unwrap();
        let touched = state.touched();
        assert!(!touched.all && !touched.pending && !touched.fired);
        assert_eq!(
            touched.delayed.iter().copied().collect::<Vec<_>>(),
            [(5, 7)]
        );
        assert!(touched.decisions.is_empty());
        assert!(
            state
                .delayed_owner((5, 7))
                .is_some_and(|(_, _, rows)| rows.is_ok())
        );
        state
            .observe(&row(12, 3, &[(FiredBaseline, None, None)]))
            .unwrap();
        let touched = state.touched();
        assert!(touched.fired && touched.delayed.is_empty());
        state.observe(&row(14, 4, &[])).unwrap();
        assert!(state.touched().all);
    }
    /// A rebuild touches its owner only when it makes, renames or loses it.
    #[test]
    fn rebuilds_touch_only_the_owners_they_make_rename_or_lose() {
        let mut state = ScriptEventState::default();
        let change = |operation| {
            (
                Decision(DecisionChange {
                    operation,
                    container: 3,
                    flags: 0,
                    ordinal: 0,
                    political_power: None,
                }),
                None,
                None,
            )
        };
        state
            .observe(&row(
                10,
                0,
                &[change(Operation::Baseline), change(Operation::Ready)],
            ))
            .unwrap();
        state
            .observe(&row(11, 2, &[change(Operation::Rebuild)]))
            .unwrap();
        assert!(state.touched().decisions.is_empty());
        let mut made = row(12, 3, &[change(Operation::Rebuild)]);
        made.events[0].actor = EventFeedCountry {
            reference: 24,
            id: 24,
            instance: 0,
        };
        state.observe(&made).unwrap();
        assert_eq!(
            state
                .touched()
                .decisions
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            [(24, 0)]
        );
    }
    /// A part's version moves with what it holds, never with its countdowns alone, and no
    /// part of a later generation takes an earlier part's version.
    #[test]
    fn versions_move_with_what_a_part_holds_and_never_repeat() {
        let version = |state: &ScriptEventState| {
            state
                .delayed_owner((5, 7))
                .and_then(|(_, version, _)| version)
                .unwrap()
        };
        let census = |hour, first| {
            row(
                hour,
                first,
                &[
                    (DelayedBaseline, None, None),
                    store(9, 1, 90),
                    (DelayedReady, None, None),
                ],
            )
        };
        let mut state = ScriptEventState::default();
        state.observe(&census(1, 0)).unwrap();
        let first = version(&state);
        state.observe(&row(2, 3, &[advance(1, true)])).unwrap();
        assert_eq!(version(&state), first);
        state.observe(&row(3, 4, &[store(5, 2, 91)])).unwrap();
        let stored = version(&state);
        assert_ne!(stored, first);
        // A gap retires the timers; a later loss finds nothing left to change.
        state.observe(&row(5, 5, &[])).unwrap();
        let gapped = version(&state);
        assert_ne!(gapped, stored);
        state
            .observe(&lost(row(6, 5, &[]), EventFeedCoverage::Unavailable))
            .unwrap();
        assert_eq!(version(&state), gapped);
        let mut next = census(7, 0);
        next.generation = 8;
        for event in &mut next.events {
            event.generation = 8;
        }
        state.observe(&next).unwrap();
        assert!(version(&state) > gapped);
    }
}

// JSON continuations encode tuple-keyed owner maps as ordered pairs.
mod owner_pairs {
    use super::*;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    pub fn serialize<S: Serializer, T: Serialize>(
        map: &BTreeMap<(u32, u32), T>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(map.iter())
    }
    pub fn deserialize<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<(u32, u32), T>, D::Error> {
        Ok(Vec::<((u32, u32), T)>::deserialize(deserializer)?
            .into_iter()
            .collect())
    }
}
