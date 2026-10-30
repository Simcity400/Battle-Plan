//! Feed-only witnessed facts. Topic changes remain in their owning families.
use super::*;
use crate::prepared::series::Accumulator;
use dashboard_base::prepared::entries::{self, Entries};
use dashboard_base::prepared::kept::Reduced;
use replay_data::{
    rows::CountryCounterInputRow, rows::CountryExperienceGainRow, rows::RelationTupleRow,
    rows::SliceReadRow, rows::TableId, rows::TableRows, rows::tables,
};
use replay_frame::frame::DecodedFrame;

pub(crate) const FACT_KEYS: [u64; 56] = [
    1,
    2,
    3,
    4,
    5,
    6,
    7,
    8,
    11,
    15,
    22,
    23,
    24,
    25,
    26,
    27,
    28,
    29,
    30,
    31,
    32,
    33,
    34,
    35,
    36,
    37,
    38,
    39,
    (1 << 63) | 1,
    (1 << 63) | 2,
    (1 << 63) | 3,
    (1 << 63) | 4,
    (1 << 63) | 5,
    (1 << 63) | 6,
    (1 << 63) | 7,
    (1 << 63) | 8,
    (1 << 63) | 11,
    (1 << 63) | 15,
    (1 << 63) | 22,
    (1 << 63) | 23,
    (1 << 63) | 24,
    (1 << 63) | 25,
    (1 << 63) | 26,
    (1 << 63) | 27,
    (1 << 63) | 28,
    (1 << 63) | 29,
    (1 << 63) | 30,
    (1 << 63) | 31,
    (1 << 63) | 32,
    (1 << 63) | 33,
    (1 << 63) | 34,
    (1 << 63) | 35,
    (1 << 63) | 36,
    (1 << 63) | 37,
    (1 << 63) | 38,
    (1 << 63) | 39,
];

#[derive(Serialize, Deserialize)]
pub(crate) struct Fact {
    pub row: EventFeedRow,
    pub nuke_type: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script: Option<script_effects::Inputs>,
    /// A native occurrence's typed operands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<Occurrence>,
    /// The item `row.item` names, kept with the fact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<EventFeedItem>,
}

/// A prepared fact's address: its stream key and ordinal.
pub(crate) type FactRef = (u64, u64);

/// A fact's stream key: its kind, and whether its read came after its date.
fn fact_key(kind: EventFeedKind, hour: i32, read_hour: i32) -> u64 {
    (kind as u64) | (u64::from(read_hour > hour) << 63)
}

/// Where an encoded occurrence names the fact it continues, written null until publication.
const OPENER: &[u8] = b",\"opener\":";

/// A fact encoded before its publication: the bytes before its occurrence's opener and those
/// after, which publication joins with the opener it links.
pub(super) struct Draft {
    hour: i32,
    read_hour: i32,
    kind: EventFeedKind,
    occurrence: Option<Occurrence>,
    head: Vec<u8>,
    /// A roster rebuild's script, written into `head` at its offset once its reduction is known.
    roster: Option<(usize, Box<script_effects::Inputs>)>,
    tail: Vec<u8>,
    /// What another thread writes of it ([`Self::write`]).
    later: Option<Later>,
}

/// What a draft leaves to be written when its fact's bytes are: a roster rebuild's reduction,
/// or a whole retained observation.
pub(super) enum Later {
    Roster(Option<Box<decision_roster::Captured>>),
    Retained(Box<native::retained::Observed>),
}

impl Draft {
    /// `fact` as serde writes it, but for the opener and, when `roster`, the script.
    pub(super) fn new(fact: Fact, roster: bool) -> io::Result<Self> {
        let Fact {
            row,
            nuke_type,
            script,
            occurrence,
            item,
        } = fact;
        let mut head = b"{\"row\":".to_vec();
        serde_json::to_writer(&mut head, &row)?;
        head.extend_from_slice(b",\"nuke_type\":");
        serde_json::to_writer(&mut head, &nuke_type)?;
        let mut held = None;
        if let Some(script) = script {
            head.extend_from_slice(b",\"script\":");
            if roster {
                held = Some((head.len(), Box::new(script)));
            } else {
                serde_json::to_writer(&mut head, &script)?;
            }
        }
        let mut tail = Vec::new();
        if let Some(occurrence) = &occurrence {
            head.extend_from_slice(b",\"occurrence\":");
            let start = head.len();
            serde_json::to_writer(&mut head, occurrence)?;
            let opener = head[start..]
                .windows(OPENER.len() + 4)
                .rposition(|bytes| bytes.starts_with(OPENER) && bytes.ends_with(b"null"))
                .ok_or_else(|| crate::invalid("An encoded occurrence has no opener"))?
                + start
                + OPENER.len();
            tail = head.split_off(opener + 4);
            head.truncate(opener);
        }
        let end = if occurrence.is_some() {
            &mut tail
        } else {
            &mut head
        };
        if let Some(item) = &item {
            end.extend_from_slice(b",\"item\":");
            serde_json::to_writer(&mut *end, item)?;
        }
        end.push(b'}');
        Ok(Self {
            hour: row.hour,
            read_hour: row.read_hour,
            kind: row.kind,
            occurrence,
            head,
            roster: held,
            tail,
            later: None,
        })
    }

    /// A retained observation's fact, written whole by [`Self::write`].
    pub(super) fn observed(observed: native::retained::Observed) -> Self {
        let read_hour = observed.read_hour();
        Self {
            hour: read_hour,
            read_hour,
            kind: EventFeedKind::ScriptStateObserved,
            occurrence: None,
            head: Vec::new(),
            roster: None,
            tail: Vec::new(),
            later: Some(Later::Retained(Box::new(observed))),
        }
    }

    /// What orders the fact among others of the hour but for its tags and identity.
    pub(super) fn order(&self) -> (i32, i32, EventFeedKind) {
        (self.hour, self.read_hour, self.kind)
    }

    pub(super) fn read_hour(&self) -> i32 {
        self.read_hour
    }

    /// The bytes it owns on the heap: its encoded parts, and what is left to write.
    pub(super) fn heap(&self) -> usize {
        let later = match &self.later {
            Some(Later::Roster(Some(captured))) => captured.heap(),
            Some(Later::Retained(observed)) => observed.heap(),
            Some(Later::Roster(None)) | None => 0,
        };
        let occurrence = self.occurrence.as_ref().map_or(0, |occurrence| {
            occurrence.token.as_ref().map_or(0, String::capacity)
                + occurrence.costs.as_ref().map_or(0, |costs| {
                    costs.capacity() * size_of::<ScriptCost>()
                        + costs.iter().map(|cost| cost.amount.capacity()).sum::<usize>()
                })
        });
        self.head.capacity()
            + self.tail.capacity()
            + occurrence
            + self
                .roster
                .as_ref()
                .map_or(0, |(_, inputs)| inputs.heap())
            + later
    }

    /// The occurrence and hour of a roster rebuild whose script waits for its reduction.
    pub(super) fn held_roster(&self) -> Option<(u64, i32)> {
        self.roster.as_ref()?;
        let sequence = self
            .occurrence
            .as_ref()
            .map_or(0, |occurrence| occurrence.sequence);
        Some((sequence, self.hour))
    }

    /// Leaves a held roster rebuild's script to [`Self::write`], with what its reduction
    /// `captured`, its target countries' slots resolved.
    pub(super) fn hold(&mut self, captured: Option<decision_roster::Captured>) {
        self.later = Some(Later::Roster(captured.map(Box::new)));
    }

    /// The fact's bytes, with what it left to be written, on whichever thread writes them.
    pub(super) fn write(mut self) -> io::Result<Vec<u8>> {
        match self.later.take() {
            Some(Later::Retained(observed)) => Self::new(
                Fact {
                    row: observed.row(true)?,
                    nuke_type: None,
                    script: None,
                    occurrence: None,
                    item: None,
                },
                false,
            )?
            .finish(),
            Some(Later::Roster(captured)) => {
                self.complete(|script, _, _| {
                    script.roster = captured.map(|captured| captured.inputs());
                })?;
                self.finish()
            }
            None => self.finish(),
        }
    }

    /// Writes a held roster rebuild's script, once `fill` gives it its roster and names from
    /// the fact's hour and occurrence.
    pub(super) fn complete(
        &mut self,
        fill: impl FnOnce(&mut script_effects::Inputs, i32, u64),
    ) -> io::Result<()> {
        let Some((at, mut script)) = self.roster.take() else {
            return Ok(());
        };
        let sequence = self
            .occurrence
            .as_ref()
            .map_or(0, |occurrence| occurrence.sequence);
        fill(&mut *script, self.hour, sequence);
        let rest = self.head.split_off(at);
        serde_json::to_writer(&mut self.head, &*script)?;
        self.head.extend_from_slice(&rest);
        Ok(())
    }

    /// The fact's bytes, with the opener its occurrence joined.
    fn finish(self) -> io::Result<Vec<u8>> {
        if self.roster.is_some() || self.later.is_some() {
            return Err(crate::invalid(
                "A roster rebuild's script was never written",
            ));
        }
        let mut bytes = self.head;
        if let Some(occurrence) = &self.occurrence {
            serde_json::to_writer(&mut bytes, &occurrence.opener)?;
            bytes.extend_from_slice(&self.tail);
        }
        Ok(bytes)
    }
}

impl Builder {
    /// The hour's rows as facts in row order, encoded but for their openers, with their script
    /// inputs named by `slot_at`; a charge is kept by the one execution of the read it is
    /// proven for. A roster rebuild's script is held for its reduction unless `rosters`, all
    /// but its roster named.
    pub(super) fn drafts(
        &mut self,
        charges: &mut BTreeMap<ChargeKey, Vec<ScriptCost>>,
        mut slot_at: impl FnMut(i32, replay_data::event_feed_rows::EventFeedCountry) -> Option<u32>,
        rosters: bool,
    ) -> io::Result<Vec<Draft>> {
        self.feed.rows.sort_by(row_order);
        // An execution shares its charge key with no other of this read, or
        // the charge stays unattributed.
        let mut executions = BTreeMap::<ChargeKey, u32>::new();
        for row in &self.feed.rows {
            if let Some(key) = self
                .occurrences
                .get(&row.id)
                .and_then(|occurrence| charge_key(row, occurrence))
            {
                *executions.entry(key).or_default() += 1;
            }
        }
        let rows: Vec<EventFeedRow> = self.feed.rows.drain(..).collect();
        let mut drafts = Vec::with_capacity(rows.len());
        for row in rows {
            let mut occurrence = self.occurrences.remove(&row.id);
            if let Some(occurrence) = &mut occurrence
                && let Some(charge) = charge_key(&row, occurrence)
                && executions.get(&charge) == Some(&1)
            {
                occurrence.costs = charges.remove(&charge);
            }
            let held = !rosters && row.kind == EventFeedKind::DecisionRosterRebuilt;
            let script = self.script_inputs.remove(&row.id).map(|mut inputs| {
                inputs.resolve(|country| slot_at(row.hour, country));
                inputs
            });
            let fact = Fact {
                item: item_of(&self.feed.items, &row)?,
                occurrence,
                script,
                row,
                nuke_type: None,
            };
            drafts.push(Draft::new(fact, held)?);
        }
        Ok(drafts)
    }
}

/// What a continuing occurrence is matched by. A choice continues the
/// dispatch of the same world, native event, instance and receiving country
/// (the dispatch's target, the choosing command's actor);
/// a decision edge continues the open selection or activation of the same
/// world, country, decision and target. Nothing else joins them.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
enum Opening {
    Event {
        generation: u64,
        event: (u32, u32),
        instance: u32,
        country: u32,
    },
    Decision {
        generation: u64,
        country: u32,
        token: String,
        target: Option<u32>,
        numeric: Option<i32>,
    },
}

/// The item `row` names among the hour's `items`.
fn item_of(items: &[EventFeedItem], row: &EventFeedRow) -> io::Result<Option<EventFeedItem>> {
    row.item
        .map(|index| {
            items
                .get(index as usize)
                .cloned()
                .ok_or_else(|| crate::invalid("Event feed fact item is not the hour's"))
        })
        .transpose()
}

/// The section of a branch's facts catalogue entries ([`entries`]).
fn entries_section(branch: u64) -> String {
    format!("event-feed-entries-{branch}")
}

/// Where `opener`'s continuing facts are published, in the order they joined it.
pub(crate) fn edges_prefix((key, ordinal): FactRef) -> Vec<u8> {
    [
        b"edge".as_slice(),
        &key.to_be_bytes(),
        &ordinal.to_be_bytes(),
    ]
    .concat()
}

/// The edges joined since the last flush, in the order they joined, and how
/// many joined before them.
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Edges {
    pending: Vec<(FactRef, FactRef)>,
    published: u64,
}

impl Edges {
    fn push(&mut self, opener: FactRef, next: FactRef) {
        self.pending.push((opener, next));
    }

    /// Each pending edge under its opener, ordered by when it joined.
    fn publish(&mut self, catalogue: &mut Entries) -> io::Result<()> {
        for (opener, next) in std::mem::take(&mut self.pending) {
            let key = [
                edges_prefix(opener).as_slice(),
                &self.published.to_be_bytes(),
            ]
            .concat();
            catalogue.put(key, &next)?;
            self.published += 1;
        }
        Ok(())
    }
}

/// Opened occurrences and the edges continuing them.
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Links {
    #[serde(with = "crate::prepared_production::map_pairs")]
    open: BTreeMap<Opening, FactRef>,
    /// Each opener's continuing facts, in stream order.
    pub edges: Edges,
    generation: Option<u64>,
    read: Option<i32>,
    /// False if the interval may hide a cancellation and reselection of the same decision.
    #[serde(skip)]
    covered: bool,
}

impl Links {
    /// Decisions lack execution identity, so incomplete intervals cannot safely join them.
    fn observe_read(&mut self, hour: i32, complete: bool) {
        if !complete || self.read.and_then(|read| read.checked_add(1)) != Some(hour) {
            self.open
                .retain(|opening, _| !matches!(opening, Opening::Decision { .. }));
        }
        self.read = Some(hour);
        self.covered = complete;
    }

    /// Joins `occurrence`, about to be published at `at`, to the occurrence it
    /// continues, and opens it for later edges. A new world closes every
    /// opening of the old one.
    fn link(&mut self, kind: EventFeedKind, occurrence: &mut Occurrence, at: FactRef) {
        use EventFeedKind::*;
        if self.generation != Some(occurrence.generation) {
            self.open.clear();
            self.generation = Some(occurrence.generation);
        }
        // A dispatch names its receiving country as its target; the option
        // command names the country choosing as its actor.
        let receiver = match kind {
            ScriptEventFired | NewsEventFired => occurrence.target,
            _ => occurrence.actor,
        };
        let event = occurrence
            .event
            .zip(occurrence.instance)
            .map(|(event, instance)| Opening::Event {
                generation: occurrence.generation,
                event,
                instance,
                country: receiver.0,
            });
        let decision = occurrence.token.clone().map(|token| Opening::Decision {
            generation: occurrence.generation,
            country: occurrence.actor.0,
            token,
            target: occurrence.country_target,
            numeric: occurrence.numeric_target,
        });
        match kind {
            ScriptEventFired | NewsEventFired => {
                if let Some(opening) = event {
                    self.open.insert(opening, at);
                }
            }
            EventOptionSelected => {
                // A negative ordinal is the event's immediate effects, run on
                // dispatch: it joins the occurrence but leaves it open for the choice.
                let immediate = occurrence.option.is_some_and(|ordinal| ordinal < 0);
                let opener = event.and_then(|opening| match immediate {
                    true => self.open.get(&opening).copied(),
                    false => self.open.remove(&opening),
                });
                if let Some(opener) = opener {
                    occurrence.opener = Some(opener);
                    self.edges.push(opener, at);
                }
            }
            DecisionSelected => {
                if let Some(opening) = decision.filter(|_| self.covered) {
                    self.open.insert(opening, at);
                }
            }
            DecisionActivated
            | DecisionDurationChanged
            | DecisionRemovalEffect
            | DecisionCancellationEffect
            | DecisionCompletionEffect
            | DecisionTimeoutEffect
            | DecisionCompleted
            | DecisionCancelled
            | DecisionTimedOut
            | DecisionRemoved => {
                let Some(opening) = decision.filter(|_| self.covered) else {
                    return;
                };
                match self.open.get(&opening).copied() {
                    Some(opener) => {
                        occurrence.opener = Some(opener);
                        self.edges.push(opener, at);
                        if matches!(
                            kind,
                            DecisionCompleted
                                | DecisionCancelled
                                | DecisionTimedOut
                                | DecisionRemoved
                        ) {
                            self.open.remove(&opening);
                        }
                    }
                    // An activation with no open selection starts its own
                    // episode: a mission, or a selection before the recording.
                    None if kind == DecisionActivated => {
                        self.open.insert(opening, at);
                    }
                    None => {}
                }
            }
            _ => {}
        }
    }
}

/// Charges join by hour, published country link, token and selection/timeout.
pub(super) type ChargeKey = (i32, u32, String, bool);

fn charge_key(row: &EventFeedRow, occurrence: &Occurrence) -> Option<ChargeKey> {
    let timeout = match row.kind {
        EventFeedKind::DecisionSelected => false,
        EventFeedKind::DecisionTimeoutEffect => true,
        _ => return None,
    };
    Some((
        row.hour,
        occurrence.actor.0,
        occurrence.token.clone()?,
        timeout,
    ))
}

/// The power charges the hour's recorded mutations prove for each decision
/// execution: mutations the writer attributed to a decision's execution or
/// timeout, for a country whose political and command power were both
/// captured over the mutation's hour, summed per currency as realized.
/// Anything else proves nothing, so its execution keeps no cost; a currency
/// with no attributed change is not listed.
pub(crate) fn charges(frame: &DecodedFrame) -> BTreeMap<ChargeKey, Vec<ScriptCost>> {
    let (Ok(changes), Ok(coverage)) = (
        frame.complete::<tables::CountryPowerChanges>(),
        frame.complete::<tables::PoliticsCaptureCoverage>(),
    ) else {
        return BTreeMap::new();
    };
    charges_of(changes, coverage)
}

/// [`charges`] of a read's complete power changes and capture coverage.
pub(crate) fn charges_of(
    changes: &[replay_data::rows::CountryPowerChangeRow],
    coverage: &[replay_data::rows::PoliticsCaptureCoverageRow],
) -> BTreeMap<ChargeKey, Vec<ScriptCost>> {
    use replay_data::rows::{PoliticsCaptureCoverageRow, PowerAction, PowerCause, PowerCurrency};
    let streams = PoliticsCaptureCoverageRow::PP | PoliticsCaptureCoverageRow::CP;
    let mut sums = BTreeMap::<ChargeKey, BTreeMap<ScriptCurrency, i128>>::new();
    for row in changes {
        let timeout = match (row.cause, row.identity.action) {
            (PowerCause::Decision | PowerCause::TargetedDecision, PowerAction::Execute) => false,
            (PowerCause::DecisionTimeout, PowerAction::Timeout) => true,
            _ => continue,
        };
        let covered = coverage.iter().any(|cover| {
            cover.country == row.country
                && cover.since_hour <= row.hour
                && row.hour <= cover.through_hour
                && cover.streams & streams == streams
        });
        let token = std::str::from_utf8(installed_content_core::parser::c_string(&row.identity.item_key));
        let Ok(token) = token else {
            continue;
        };
        if !covered || token.is_empty() {
            continue;
        }
        let currency = match row.currency {
            PowerCurrency::Political => ScriptCurrency::PoliticalPower,
            PowerCurrency::Command => ScriptCurrency::CommandPower,
        };
        *sums
            .entry((row.hour, row.country, token.to_owned(), timeout))
            .or_default()
            .entry(currency)
            .or_default() += row.realized_delta_raw();
    }
    sums.into_iter()
        .map(|(key, currencies)| {
            (
                key,
                currencies
                    .into_iter()
                    .filter(|(_, amount)| *amount != 0)
                    .map(|(currency, amount)| ScriptCost {
                        currency,
                        amount: amount.to_string(),
                    })
                    .collect(),
            )
        })
        .collect()
}

pub(crate) mod query;
pub mod script;
pub(crate) use query::query;

#[cfg(test)]
pub(crate) mod tests;

/// Frames as the feed and the identity history it names countries through
/// read them, for a block's hours kept to be observed later. The countries
/// keep their keys, the owned-state counters their country and count, the
/// experience gains their country and instance, the relations the rows the
/// diplomacy observer reads at any hour and the slice reads those it reads
/// at the frame's hour; every other field of these tables moves hourly and
/// is read by neither. The links, the event packs and the strikes are kept
/// as read: the links are told apart by allocation, the packs and strikes
/// are this hour's. Kept hours whose reduced rows are equal share one view.
pub(crate) struct KeptFrames {
    countries: Reduced<replay_data::rows::CountryRow>,
    counters: Reduced<CountryCounterInputRow>,
    gains: Reduced<CountryExperienceGainRow>,
    relations: Reduced<RelationTupleRow>,
    slices: Reduced<SliceReadRow>,
}

impl Default for KeptFrames {
    fn default() -> Self {
        Self {
            countries: Reduced::new(TableId::Countries),
            counters: Reduced::new(TableId::CountryCounterInputs),
            gains: Reduced::new(TableId::CountryExperienceGains),
            relations: Reduced::new(TableId::RelationTuples),
            slices: Reduced::new(TableId::SliceReads),
        }
    }
}

impl KeptFrames {
    /// `frame` as the feed and the identity history read it.
    pub(crate) fn kept(&mut self, frame: &DecodedFrame) -> DecodedFrame {
        let mut kept = frame.clone();
        self.countries.countries(&mut kept);
        let counters = frame
            .complete::<tables::CountryCounterInputs>()
            .ok()
            .map(|rows| {
                rows.iter()
                    .map(|row| CountryCounterInputRow {
                        country: row.country,
                        owned_state_count: row.owned_state_count,
                        ..Default::default()
                    })
                    .collect()
            });
        self.counters
            .apply(&mut kept, counters, TableRows::CountryCounterInputs);
        let gains = frame
            .complete::<tables::CountryExperienceGains>()
            .ok()
            .map(|rows| {
                rows.iter()
                    .map(|row| CountryExperienceGainRow {
                        country: row.country,
                        instance: row.instance,
                        ..Default::default()
                    })
                    .collect()
            });
        self.gains
            .apply(&mut kept, gains, TableRows::CountryExperienceGains);
        let relations = frame.complete::<tables::RelationTuples>().ok().map(|rows| {
            rows.iter()
                .filter(|row| super::diplomacy::reads_relation(row))
                .cloned()
                .collect()
        });
        self.relations
            .apply(&mut kept, relations, TableRows::RelationTuples);
        let slices = frame.complete::<tables::SliceReads>().ok().map(|rows| {
            rows.iter()
                .filter(|read| super::diplomacy::reads_slice(read, frame.hour))
                .cloned()
                .collect()
        });
        self.slices.apply(&mut kept, slices, TableRows::SliceReads);
        kept
    }
}

/// The strike types' native ordinals, each its own operand: the type is not an item identity,
/// and its family names it at query time.
pub(super) fn strike_types() -> BTreeMap<u32, u32> {
    country_nukes::native_type_info(&country_nukes::ScannedNukes::default(), |_| None)
        .into_iter()
        .map(|info| (info.ordinal, info.ordinal))
        .collect()
}

/// The facts of a read's `strikes`, named by `tag_at`, in row order.
pub(super) fn strike_facts(
    read_hour: i32,
    strikes: &[replay_data::rows::NuclearStrikeRow],
    mut tag_at: impl FnMut(i32, replay_data::event_feed_rows::EventFeedCountry) -> Option<String>,
    builder: &mut Builder,
    types: &BTreeMap<u32, u32>,
) -> Vec<Fact> {
    let mut facts = Vec::new();
    for strike in strikes {
        nuclear::project(read_hour, strike, &mut tag_at, builder, types);
        for mut row in builder.feed.rows.drain(..) {
            row.item = None;
            facts.push(Fact {
                row,
                nuke_type: Some(strike.nuke_type),
                script: None,
                occurrence: None,
                item: None,
            });
        }
    }
    facts.sort_by(|a, b| row_order(&a.row, &b.row));
    facts
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Observer {
    pub(super) native: native::Projection,
    pub(super) diplomacy: diplomacy::Observer,
    pub(super) events: Accumulator,
    late: crate::prepared::series::LateIndex,
    pub(super) diagnostics: Builder,
    observed: BTreeSet<String>,
    #[serde(default)]
    links: Links,
    /// Facts published but not yet written ([`Draft::write`]), by key.
    #[serde(skip)]
    held: BTreeMap<u64, u64>,
}
impl Default for Observer {
    fn default() -> Self {
        Self {
            native: Default::default(),
            diplomacy: Default::default(),
            events: Default::default(),
            late: Default::default(),
            diagnostics: Builder::new(0, 0),
            observed: BTreeSet::new(),
            links: Links::default(),
            held: BTreeMap::new(),
        }
    }
}
impl Observer {
    pub(crate) fn observe(
        &mut self,
        replay: &replay::container::Replay,
        frame: &DecodedFrame,
        identities: &crate::products::country_history::HistoryScan,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        check_cancel(cancelled)?;
        let start = replay
            .branches
            .iter()
            .find(|start| start.id == frame.branch)
            .ok_or_else(|| crate::invalid("Event feed branch is not recorded"))?;
        let (first, last) = branch_bounds(replay, start);
        self.diagnostics.feed.first = first;
        self.diagnostics.feed.last = last;
        self.native.losses(replay, frame.branch);
        self.events.through(frame.hour);
        let tag_at = |hour, country| identities.native_tag(hour, country);
        let pack = frame.complete::<tables::EventFeedHistory>();
        let mut complete = false;
        let read = match pack {
            Ok([pack]) if pack.read_hour == frame.hour => {
                complete = pack.coverage
                    == replay_data::event_feed_rows::EventFeedCoverage::Complete
                    && pack.script_refusals == 0;
                self.native
                    .append_named(&mut self.diagnostics, pack, tag_at, cancelled)
            }
            Ok(_) => Err(crate::invalid(
                "Event history has no unique pack for its supplying hour.",
            )),
            Err(reason) => Err(reason),
        };
        let complete = complete && read.is_ok();
        if let Err(reason) = read {
            check_cancel(cancelled)?;
            self.diagnostics.sampled_unavailable(
                "nativeEventCoverage",
                "Native event coverage",
                reason.to_string(),
                1,
            );
        }
        let drafts = self.diagnostics.drafts(
            &mut charges(frame),
            |hour, country| identities.native_scope_slot(hour, country),
            true,
        )?;
        if !self.publish(drafts, frame.hour, complete)?.is_empty() {
            return Err(crate::invalid(
                "The row path left a fact to be written later",
            ));
        }
        let mut changes = country_groups::changes::Scan {
            moments: Vec::new(),
            observed: BTreeSet::new(),
            unavailable: Vec::new(),
        };
        self.diplomacy.observe(frame, replay.format, &mut changes)?;
        self.publish_diplomacy(changes)?;
        let strikes = nuclear::read(frame, &mut self.diagnostics, cancelled)?;
        let types = strike_types();
        for strike in strike_facts(frame.hour, &strikes, tag_at, &mut self.diagnostics, &types) {
            self.publish_one(Draft::new(strike, false)?)?;
        }
        self.end_hour();
        Ok(())
    }
    /// The hour's native facts, published in row order once each is joined, in native order,
    /// to the occurrence it continues. Those with bytes still to be written, and every later
    /// one of their keys, are held for [`Self::written`], in order, with their keys.
    pub(super) fn publish(
        &mut self,
        mut drafts: Vec<Draft>,
        hour: i32,
        complete: bool,
    ) -> io::Result<Vec<(u64, Draft)>> {
        // Facts publish in row order; their joins follow the native order,
        // so an edge continues the occurrence open when it ran.
        let mut next = BTreeMap::<u64, u64>::new();
        let at: Vec<FactRef> = drafts
            .iter()
            .map(|draft| {
                let key = fact_key(draft.kind, draft.hour, draft.read_hour);
                let ordinal = next.entry(key).or_insert_with(|| self.ordinal(key));
                let at = (key, *ordinal);
                *ordinal += 1;
                at
            })
            .collect();
        let mut native: Vec<usize> = (0..drafts.len())
            .filter(|&index| drafts[index].occurrence.is_some())
            .collect();
        native.sort_by_key(|&index| {
            drafts[index]
                .occurrence
                .as_ref()
                .map(|occurrence| (occurrence.generation, occurrence.sequence))
        });
        self.links.observe_read(hour, complete);
        for index in native {
            let draft = &mut drafts[index];
            if let Some(occurrence) = &mut draft.occurrence {
                self.links.link(draft.kind, occurrence, at[index]);
            }
        }
        let mut later = Vec::new();
        for (draft, (key, ordinal)) in drafts.into_iter().zip(at) {
            self.late.insert(draft.hour, draft.read_hour, key, ordinal);
            if draft.later.is_some() || self.held.contains_key(&key) {
                *self.held.entry(key).or_default() += 1;
                later.push((key, draft));
                continue;
            }
            let read_hour = draft.read_hour;
            self.events.event_raw(key, read_hour, draft.finish()?)?;
        }
        self.diagnostics.war_ends.clear();
        Ok(later)
    }
    /// The next ordinal of `key`, after the facts it holds.
    fn ordinal(&self, key: u64) -> u64 {
        self.events.state(key).map_or(0, |state| state.events)
            + self.held.get(&key).copied().unwrap_or(0)
    }
    /// The bytes of the oldest fact `key` holds, written ([`Self::publish`]).
    pub(super) fn written(&mut self, key: u64, read_hour: i32, bytes: Vec<u8>) -> io::Result<()> {
        match self.held.get_mut(&key) {
            Some(held) if *held > 1 => *held -= 1,
            Some(_) => {
                self.held.remove(&key);
            }
            None => {
                return Err(crate::invalid(
                    "An event-feed fact was written but never held",
                ));
            }
        }
        self.events.event_raw(key, read_hour, bytes)
    }
    /// A fact after the hour's native ones, at its key's next ordinal.
    pub(super) fn publish_one(&mut self, draft: Draft) -> io::Result<()> {
        let key = fact_key(draft.kind, draft.hour, draft.read_hour);
        if self.held.contains_key(&key) {
            return Err(crate::invalid(
                "An event-feed fact would pass one held before it",
            ));
        }
        let ordinal = self.ordinal(key);
        self.late.insert(draft.hour, draft.read_hour, key, ordinal);
        let read_hour = draft.read_hour;
        self.events.event_raw(key, read_hour, draft.finish()?)
    }
    /// The hour's diplomacy observation: the tables it read, its refusals and its moments.
    pub(super) fn publish_diplomacy(
        &mut self,
        changes: country_groups::changes::Scan,
    ) -> io::Result<()> {
        self.observed.extend(
            changes
                .observed
                .into_iter()
                .map(|table| format!("{table:?}")),
        );
        for refusal in changes.unavailable {
            self.diagnostics.unavailable(
                &refusal.key,
                &refusal.label,
                refusal.reason,
                Some(
                    u32::try_from(refusal.count)
                        .map_err(|_| crate::invalid("Diplomacy refusal count exceeds u32"))?,
                ),
            );
        }
        for change in changes.moments {
            group_row(&mut self.diagnostics, &change, None)?;
        }
        self.diagnostics.feed.rows.sort_by(row_order);
        let rows: Vec<EventFeedRow> = self.diagnostics.feed.rows.drain(..).collect();
        for mut row in rows {
            row.id.clear(); // The shared reader's ordinal owns non-native row identities.
            let item = item_of(&self.diagnostics.feed.items, &row)?;
            let fact = Fact {
                nuke_type: None,
                script: None,
                occurrence: None,
                item,
                row,
            };
            self.publish_one(Draft::new(fact, false)?)?;
        }
        Ok(())
    }
    /// Every row of the hour keeps its item: none is named again.
    pub(super) fn end_hour(&mut self) {
        self.diagnostics.feed.items.clear();
        self.diagnostics.item_keys.clear();
    }
    pub(crate) fn bytes(&self) -> usize {
        self.events.bytes()
    }
    pub(crate) fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.events
            .flush(writer, &format!("event-feed-facts-{branch}"), 0, cancelled)?;
        let mut catalogue = Entries::default();
        self.links.edges.publish(&mut catalogue)?;
        catalogue.flush(writer, &entries_section(branch))?;
        crate::prepared::series::write_catalogue(
            writer,
            "event-feed-facts",
            branch,
            &BTreeMap::new(),
            &(&self.diagnostics, &self.observed, &self.late),
        )
    }
}
