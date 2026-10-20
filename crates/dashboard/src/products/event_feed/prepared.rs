//! Feed-only witnessed facts. Topic changes remain in their owning families.
use super::*;
use crate::prepared::series::Accumulator;
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
}

/// A prepared fact's address: its stream key and ordinal.
pub(crate) type FactRef = (u64, u64);

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
        country: (u32, u32),
    },
    Decision {
        generation: u64,
        country: (u32, u32),
        token: String,
        target: Option<u32>,
        numeric: Option<i32>,
    },
}

/// Each opener's continuing facts, in stream order.
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Edges(
    #[serde(with = "crate::prepared_production::map_pairs")] pub BTreeMap<FactRef, Vec<FactRef>>,
);

/// Opened occurrences and the edges continuing them.
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Links {
    #[serde(with = "crate::prepared_production::map_pairs")]
    open: BTreeMap<Opening, FactRef>,
    /// Each opener's continuing facts, in stream order.
    pub edges: Edges,
    generation: Option<u64>,
}

impl Links {
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
                country: receiver,
            });
        let decision = occurrence.token.clone().map(|token| Opening::Decision {
            generation: occurrence.generation,
            country: occurrence.actor,
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
                    self.edges.0.entry(opener).or_default().push(at);
                }
            }
            DecisionSelected => {
                if let Some(opening) = decision {
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
                let Some(opening) = decision else {
                    return;
                };
                match self.open.get(&opening).copied() {
                    Some(opener) => {
                        occurrence.opener = Some(opener);
                        self.edges.0.entry(opener).or_default().push(at);
                        if matches!(
                            kind,
                            DecisionCompleted | DecisionCancelled | DecisionTimedOut | DecisionRemoved
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

/// What attributes a power mutation to a decision execution: its native
/// hour, the country's id and instance, the decision's token and whether it
/// is the timeout's charge rather than the selection's.
type ChargeKey = (i32, (u32, u32), String, bool);

fn charge_key(row: &EventFeedRow, occurrence: &Occurrence) -> Option<ChargeKey> {
    let timeout = match row.kind {
        EventFeedKind::DecisionSelected => false,
        EventFeedKind::DecisionTimeoutEffect => true,
        _ => return None,
    };
    Some((row.hour, occurrence.actor, occurrence.token.clone()?, timeout))
}

/// The power charges the hour's recorded mutations prove for each decision
/// execution: mutations the writer attributed to a decision's execution or
/// timeout, for a country whose political and command power were both
/// captured over the mutation's hour, summed per currency as realized.
/// Anything else proves nothing, so its execution keeps no cost; a currency
/// with no attributed change is not listed.
pub(crate) fn charges(frame: &DecodedFrame) -> BTreeMap<ChargeKey, Vec<ScriptCost>> {
    use replay_data::rows::{PoliticsCaptureCoverageRow, PowerAction, PowerCause, PowerCurrency};
    let (Ok(changes), Ok(coverage)) = (
        frame.complete::<tables::CountryPowerChanges>(),
        frame.complete::<tables::PoliticsCaptureCoverage>(),
    ) else {
        return BTreeMap::new();
    };
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
                && cover.instance == row.instance
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
            .entry((row.hour, (row.country, row.instance), token.to_owned(), timeout))
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

#[derive(Serialize, Deserialize)]
pub(crate) struct Observer {
    native: native::Projection,
    diplomacy: diplomacy::Observer,
    events: Accumulator,
    late: crate::prepared::series::LateIndex,
    diagnostics: Builder,
    observed: BTreeSet<String>,
    #[serde(default)]
    links: Links,
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
        let read = match pack {
            Ok([pack]) if pack.read_hour == frame.hour => {
                self.native
                    .append_named(&mut self.diagnostics, pack, tag_at, cancelled)
            }
            Ok(_) => Err(crate::invalid(
                "Event history has no unique pack for its supplying hour.",
            )),
            Err(reason) => Err(reason),
        };
        if let Err(reason) = read {
            check_cancel(cancelled)?;
            self.diagnostics.sampled_unavailable(
                "nativeEventCoverage",
                "Native event coverage",
                reason.to_string(),
                1,
            );
        }
        self.diagnostics.feed.rows.sort_by(row_order);
        let mut charges = charges(frame);
        // An execution shares its charge key with no other of this read, or
        // the charge stays unattributed.
        let mut executions = BTreeMap::<ChargeKey, u32>::new();
        for row in &self.diagnostics.feed.rows {
            if let Some(key) = self
                .diagnostics
                .occurrences
                .get(&row.id)
                .and_then(|occurrence| charge_key(row, occurrence))
            {
                *executions.entry(key).or_default() += 1;
            }
        }
        // Facts publish in row order; their joins follow the native order,
        // so an edge continues the occurrence open when it ran.
        let rows: Vec<EventFeedRow> = self.diagnostics.feed.rows.drain(..).collect();
        let mut next = BTreeMap::<u64, u64>::new();
        let at: Vec<FactRef> = rows
            .iter()
            .map(|row| {
                let key = (row.kind as u64) | (u64::from(row.read_hour > row.hour) << 63);
                let ordinal = next
                    .entry(key)
                    .or_insert_with(|| self.events.state(key).map_or(0, |state| state.events));
                let at = (key, *ordinal);
                *ordinal += 1;
                at
            })
            .collect();
        let mut occurrences: Vec<Option<Occurrence>> = rows
            .iter()
            .map(|row| self.diagnostics.occurrences.remove(&row.id))
            .collect();
        let mut native: Vec<usize> = (0..rows.len())
            .filter(|&index| occurrences[index].is_some())
            .collect();
        native.sort_by_key(|&index| {
            occurrences[index]
                .as_ref()
                .map(|occurrence| (occurrence.generation, occurrence.sequence))
        });
        for index in native {
            let row = &rows[index];
            let Some(occurrence) = occurrences[index].as_mut() else {
                continue;
            };
            if let Some(charge) = charge_key(row, occurrence)
                && executions.get(&charge) == Some(&1)
            {
                occurrence.costs = charges.remove(&charge);
            }
            self.links.link(row.kind, occurrence, at[index]);
        }
        for ((row, occurrence), (key, ordinal)) in rows.into_iter().zip(occurrences).zip(at) {
            self.late.insert(row.hour, row.read_hour, key, ordinal);
            self.events.event(
                key,
                row.read_hour,
                &Fact {
                    occurrence,
                    script: self
                        .diagnostics
                        .script_inputs
                        .remove(&row.id)
                        .map(|mut inputs| {
                            inputs
                                .resolve(|country| identities.native_scope_slot(row.hour, country));
                            inputs
                        }),
                    row,
                    nuke_type: None,
                },
            )?;
        }
        self.diagnostics.war_ends.clear();
        let mut changes = country_groups::changes::Scan {
            moments: Vec::new(),
            observed: BTreeSet::new(),
            unavailable: Vec::new(),
        };
        self.diplomacy.observe(frame, replay.format, &mut changes)?;
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
        for mut row in self.diagnostics.feed.rows.drain(..) {
            row.id.clear(); // The shared reader's ordinal owns non-native row identities.
            let key = (row.kind as u64) | (u64::from(row.read_hour > row.hour) << 63);
            let ordinal = self.events.state(key).map_or(0, |state| state.events);
            self.late.insert(row.hour, row.read_hour, key, ordinal);
            self.events.event(
                key,
                row.read_hour,
                &Fact {
                    row,
                    nuke_type: None,
                    script: None,
                    occurrence: None,
                },
            )?;
        }
        let mut strikes = Vec::new();
        let type_operands =
            country_nukes::native_type_info(&country_nukes::ScannedNukes::default(), |_| None)
                .into_iter()
                .map(|info| (info.ordinal, info.ordinal))
                .collect();
        for strike in nuclear::read(frame, &mut self.diagnostics, cancelled)? {
            // The type is an operand, not an item identity; its family names it at query time.
            nuclear::project(
                frame.hour,
                &strike,
                tag_at,
                &mut self.diagnostics,
                &type_operands,
            );
            for mut row in self.diagnostics.feed.rows.drain(..) {
                row.item = None;
                strikes.push(Fact {
                    row,
                    nuke_type: Some(strike.nuke_type),
                    script: None,
                    occurrence: None,
                });
            }
        }
        strikes.sort_by(|a, b| row_order(&a.row, &b.row));
        for strike in strikes {
            let key = (strike.row.kind as u64)
                | (u64::from(strike.row.read_hour > strike.row.hour) << 63);
            let ordinal = self.events.state(key).map_or(0, |state| state.events);
            self.late
                .insert(strike.row.hour, strike.row.read_hour, key, ordinal);
            self.events.event(key, strike.row.read_hour, &strike)?;
        }
        Ok(())
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
        crate::prepared::series::write_catalogue(
            writer,
            "event-feed-facts",
            branch,
            &BTreeMap::new(),
            &(&self.diagnostics, &self.observed, &self.late, &self.links.edges),
        )
    }
}
