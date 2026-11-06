//! Independent retained Rocket totals within the shared branch walk.

use super::{
    CostTotal, CountryHolders, Entry, Identity, ProductionSpendingUnknown, Purpose, Rows,
    TagHistory, Total,
};
use crate::metadata::slot_read;
use replay_data::{rows::RocketProductionOutputRow, rows::TableId, rows::tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
};

/// One hour of the Rocket output as the totals read it.
pub(crate) struct RocketRead<'a> {
    pub hour: i32,
    /// Whether the output's read is dated at the hour.
    pub read: bool,
    /// Whether Countries, CountryLinks and the output hold every row.
    pub complete: bool,
    pub rows: Rows<'a, RocketProductionOutputRow>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Row {
    identity: Option<Identity>,
    units: Total,
    cost: CostTotal,
    present: bool,
    #[serde(with = "previous_row")]
    previous: Option<RocketProductionOutputRow>,
}

// Amounts remain signed; exact changes need an observed event.
fn consistent(
    previous: &RocketProductionOutputRow,
    row: &RocketProductionOutputRow,
) -> (bool, bool) {
    let spending = row.ticks >= previous.ticks
        && row.inexact_ticks >= previous.inexact_ticks
        && row.unknown_design_ticks >= previous.unknown_design_ticks
        && row.unknown_country_ticks >= previous.unknown_country_ticks
        && (!row.exact
            || previous.exact
                && (row.ticks != previous.ticks
                    || (row.spent_raw == previous.spent_raw
                        && row.instant_raw == previous.instant_raw)));
    let units = row.deliveries >= previous.deliveries
        && row.inexact_deliveries >= previous.inexact_deliveries
        && (!row.units_exact
            || previous.units_exact
                && (row.deliveries != previous.deliveries || row.units == previous.units));
    (spending, units)
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(crate) struct RocketProduced {
    pub tags: BTreeMap<String, TagHistory>,
    pub(crate) active: BTreeSet<String>,
    pub reads: Vec<i32>,
    pub observations: Vec<i32>,
    pub unread: usize,
    pub unread_reason: Option<String>,
    pub unattributed: usize,
    pub inconsistent: usize,
    observation: u32,
    #[serde(with = "crate::prepared_production::map_pairs")]
    rows: HashMap<(u32, u64), Row>,
    owner_unknown: bool,
    delivery_owner_unknown: bool,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    pub work: crate::prepared::observer_tests::Work,
}

/// What a read of the Rocket output gives each holder.
#[derive(Default)]
pub(crate) struct RocketTotals {
    units: HashMap<Identity, BTreeMap<Entry, Total>>,
    costs: HashMap<Identity, BTreeMap<Entry, CostTotal>>,
    /// The producers with a tick or a delivery.
    present: BTreeSet<Identity>,
    /// Some delivery's owner could not be told: every holder's units are inexact.
    delivery_owner_unknown: bool,
    /// Why every holder's spending is unknown, where some tick's owner could not be told.
    owner_reason: Option<ProductionSpendingUnknown>,
    /// The read began another observation: no holder's read goes on into it.
    pub restarted: bool,
}

impl RocketTotals {
    /// Units and spending as a holder takes them.
    fn held(
        &self,
        units: Option<&BTreeMap<Entry, Total>>,
        costs: Option<&BTreeMap<Entry, CostTotal>>,
    ) -> (BTreeMap<Entry, Total>, BTreeMap<Entry, CostTotal>) {
        let mut units = units.cloned().unwrap_or_default();
        let mut costs = costs.cloned().unwrap_or_default();
        if self.delivery_owner_unknown {
            units.entry((0, Purpose::New)).or_default().inexact = 1;
        }
        if let Some(reason) = self.owner_reason {
            costs.entry((0, Purpose::New)).or_default().reason = Some(reason);
        }
        (units, costs)
    }

    /// `identity`'s units and spending for the holder that takes them first, which leaves none
    /// for a later holder of it.
    fn take(&mut self, identity: Identity) -> (BTreeMap<Entry, Total>, BTreeMap<Entry, CostTotal>) {
        let units = self.units.remove(&identity);
        let costs = self.costs.remove(&identity);
        self.held(units.as_ref(), costs.as_ref())
    }

    /// `identity`'s units and spending for its first holder, and none for a later one.
    pub(crate) fn of(
        &self,
        identity: Identity,
        first: bool,
    ) -> (BTreeMap<Entry, Total>, BTreeMap<Entry, CostTotal>) {
        if first {
            self.held(self.units.get(&identity), self.costs.get(&identity))
        } else {
            self.held(None, None)
        }
    }

    /// Whether `identity` produced, as the read tells it.
    pub(crate) fn presence(&self, identity: Identity) -> Option<bool> {
        if self.present.contains(&identity) {
            Some(true)
        } else if self.owner_reason.is_some() || self.delivery_owner_unknown {
            None
        } else {
            Some(false)
        }
    }
}

/// What one hour of the Rocket output's rows gives the holders.
pub(crate) enum RocketFold {
    /// No read can be taken: every holder's read ends.
    Ended,
    /// Not a read: each holder's read goes on while its identity holds.
    Carried,
    Read(RocketTotals),
}

impl RocketProduced {
    pub(crate) fn end_reads(&mut self) {
        for tag in &self.active {
            self.tags.get_mut(tag).expect("active Rocket holder").last = None;
        }
        self.active.clear();
    }

    /// The observation reads are counted in.
    pub(crate) fn observation(&self) -> u32 {
        self.observation
    }

    pub fn observe(
        &mut self,
        format: replay_data::format::FormatVersion,
        frame: &DecodedFrame,
    ) -> io::Result<()> {
        // The Rocket totals never count unnamed tags.
        let reads = frame.complete::<tables::Countries>().ok().map(|rows| {
            CountryHolders::read(
                &crate::products::territory::CountryReads::new(frame, rows),
                0,
            )
        });
        let output = frame.complete::<tables::RocketProductionOutput>();
        let read = RocketRead {
            hour: frame.hour,
            read: slot_read(
                format,
                &frame.metrics,
                TableId::RocketProductionOutput,
                frame.hour,
            ),
            complete: frame.complete::<tables::Countries>().is_ok()
                && frame.complete::<tables::CountryLinks>().is_ok()
                && output.is_ok(),
            rows: Rows::Slice(output.unwrap_or(&[])),
        };
        self.observe_resolved(&read, reads.as_ref())
    }

    /// One hour's read of the output, its countries resolved as `reads`, present while
    /// Countries is complete.
    pub(crate) fn observe_resolved(
        &mut self,
        input: &RocketRead<'_>,
        reads: Option<&CountryHolders>,
    ) -> io::Result<()> {
        let hour = input.hour;
        let mut totals = match self.fold(input, reads)? {
            RocketFold::Ended => {
                self.end_reads();
                return Ok(());
            }
            RocketFold::Carried => {
                let reads = reads.expect("resolved Rocket countries");
                let observation = self.observation;
                let present: HashMap<&str, Identity> = reads
                    .holders
                    .iter()
                    .filter_map(|(tag, holder)| {
                        holder.map(|(slot, id)| (tag.as_str(), (slot, id, observation)))
                    })
                    .collect();
                self.active.retain(|tag| {
                    let track = self.tags.get_mut(tag).expect("active Rocket holder");
                    let identity = present.get(tag.as_str()).copied().filter(|identity| {
                        track.last.is_some_and(|(previous, at)| {
                            previous == *identity && at.checked_add(1) == Some(hour)
                        })
                    });
                    track.last = identity.map(|identity| (identity, hour));
                    if identity.is_some()
                        && let Some(last) = track.last_hours.last_mut()
                    {
                        *last = hour;
                    }
                    identity.is_some()
                });
                return Ok(());
            }
            RocketFold::Read(totals) => totals,
        };
        if totals.restarted {
            self.end_reads();
        }
        let reads = reads.expect("resolved Rocket countries");
        let observation = self.observation;
        let observed: Vec<(String, Identity)> = reads
            .holders
            .iter()
            .filter_map(|(tag, holder)| {
                holder.map(|(slot, id)| (tag.clone(), (slot, id, observation)))
            })
            .collect();
        let present: BTreeSet<&str> = observed.iter().map(|(tag, _)| tag.as_str()).collect();
        for tag in &self.active {
            if !present.contains(tag.as_str()) {
                self.tags.get_mut(tag).expect("active Rocket holder").last = None;
            }
        }
        self.active = observed.iter().map(|(tag, _)| tag.clone()).collect();
        for (tag, identity) in observed {
            let track = self.tags.entry(tag).or_default();
            let starts = !track
                .last
                .is_some_and(|(seen, at)| seen == identity && at.checked_add(1) == Some(hour));
            let sample = u32::try_from(track.hours.len())
                .map_err(|_| io::Error::other("Rocket sample index exceeds its integer range"))?;
            let (units, costs) = totals.take(identity);
            track.hours.push(hour);
            track.last_hours.push(hour);
            track.starts.push(starts);
            track.identities.push(identity);
            track.presence.push(totals.presence(identity));
            track.last = Some((identity, hour));
            let keys: BTreeSet<_> = track.current.keys().chain(units.keys()).copied().collect();
            for key in keys {
                let value = units.get(&key).copied().unwrap_or_default();
                if track.current.get(&key).copied().unwrap_or_default() != value {
                    track.changes.entry(key).or_default().push((sample, value));
                }
            }
            let keys: BTreeSet<_> = track
                .cost_current
                .keys()
                .chain(costs.keys())
                .copied()
                .collect();
            for key in keys {
                let value = costs.get(&key).copied().unwrap_or_default();
                if track.cost_current.get(&key).copied().unwrap_or_default() != value {
                    track
                        .cost_changes
                        .entry(key)
                        .or_default()
                        .push((sample, value));
                }
            }
            track.current = units;
            track.cost_current = costs;
        }
        Ok(())
    }

    /// One hour of the output's rows, its countries resolved as `reads`: what each holder's
    /// read takes of it, the holders' own tracks left to the caller.
    pub(crate) fn fold(
        &mut self,
        input: &RocketRead<'_>,
        reads: Option<&CountryHolders>,
    ) -> io::Result<RocketFold> {
        let hour = input.hour;
        let read = input.read;
        let output = input.rows;
        if !input.complete {
            if read {
                self.unread += 1;
            }
            self.unread_reason.get_or_insert_with(|| {
                "RocketProductionOutput or its country identities are unavailable".into()
            });
            return Ok(RocketFold::Ended);
        }
        let reads = reads.expect("resolved Rocket countries");
        if !read {
            return Ok(RocketFold::Carried);
        }
        // A complete empty array cannot authenticate old/unrecorded coverage.
        let Some(world) = output
            .iter()
            .find(|row| row.country == 0 && row.variant == 0)
        else {
            self.unread += 1;
            self.unread_reason
                .get_or_insert_with(|| "Rocket coverage singleton is absent".into());
            return Ok(RocketFold::Ended);
        };
        if output
            .iter()
            .filter(|row| row.country == 0 && row.variant == 0)
            .count()
            != 1
            || output.iter().any(|row| {
                row.variant == 0 && (row.spent_raw != 0 || row.instant_raw != 0 || row.units != 0)
            })
        {
            return Err(io::Error::other(
                "Rocket coverage rows are duplicated or contain numeric amounts",
            ));
        }
        if output.iter().any(|row| row.since_hour != world.since_hour) {
            return Err(io::Error::other(
                "Rocket publication contains different observations",
            ));
        }
        let mut restarted = false;
        match self.observations.last() {
            None => self.observations.push(world.since_hour),
            Some(since) if *since != world.since_hour => {
                self.observations.push(world.since_hour);
                self.observation = self
                    .observation
                    .checked_add(1)
                    .ok_or_else(|| io::Error::other("Rocket observation index overflows"))?;
                self.rows.clear();
                self.owner_unknown = false;
                self.delivery_owner_unknown = false;
                restarted = true;
            }
            Some(_) => {}
        }
        self.reads.push(hour);
        self.owner_unknown |= !world.exact;
        self.delivery_owner_unknown |= !world.units_exact;
        // A slot two tags name is the later tag's, as the tags are met in order.
        let observation = self.observation;
        let by_slot: HashMap<u32, Identity> = reads
            .holders
            .values()
            .filter_map(|holder| holder.map(|(slot, id)| (slot, (slot, id, observation))))
            .collect();
        let mut seen = BTreeSet::new();
        for row in output.iter() {
            #[cfg(any(test, feature = "test-support"))]
            {
                self.work.row_visits += 1;
            }
            let key = (row.country, row.variant);
            if !seen.insert(key) {
                return Err(io::Error::other(
                    "Rocket publication repeats a country/design key",
                ));
            }
            // Protocol zero is never the game's reserved country slot.
            let producer = (row.country != 0)
                .then(|| {
                    reads
                        .slots
                        .get(&row.country)
                        .and_then(|slot| by_slot.get(slot))
                        .copied()
                })
                .flatten();
            let world_scope = key == (0, 0);
            if !world_scope && !self.rows.contains_key(&key) {
                self.unattributed += usize::from(producer.is_none());
            }
            let state = self.rows.entry(key).or_insert(Row {
                identity: producer,
                units: Total::default(),
                cost: CostTotal::default(),
                present: false,
                previous: None,
            });
            let (spending_valid, units_valid) = state
                .previous
                .as_ref()
                .map_or((true, true), |previous| consistent(previous, row));
            state.previous = Some(row.clone());
            self.inconsistent += usize::from(!spending_valid || !units_valid);
            state.units.inexact = state.units.inexact.max(u32::from(!units_valid));
            state.cost.reason = state
                .cost
                .reason
                .or((!spending_valid).then_some(ProductionSpendingUnknown::RocketInputInexact));
            if world_scope {
                self.owner_unknown |= !spending_valid;
                self.delivery_owner_unknown |= !units_valid;
                continue;
            }
            state.present |= row.ticks != 0 || row.deliveries != 0;
            // An unresolved row with no events may be identified later;
            // recorded events never migrate to a later holder of its raw link.
            if state.identity.is_none() && row.ticks == 0 && row.deliveries == 0 {
                state.identity = producer;
            }
            let Some(_) = state.identity else {
                if row.ticks != 0 || row.spent_raw != 0 || !row.exact {
                    self.owner_unknown = true;
                }
                if row.deliveries != 0 || row.units != 0 || !row.units_exact {
                    self.delivery_owner_unknown = true;
                }
                continue;
            };
            // Variant zero is a scope marker only: its amounts are never added.
            let scope = row.variant == 0;
            state.units = Total {
                units: if scope { 0 } else { i128::from(row.units) },
                inexact: state.units.inexact.max(u32::from(!row.units_exact)),
            };
            state.cost = CostTotal {
                raw: if scope { 0 } else { i128::from(row.spent_raw) },
                reason: state.cost.reason.or((!row.exact).then_some(
                    if scope && row.unknown_design_ticks != 0 {
                        ProductionSpendingUnknown::RocketDesignUnreadable
                    } else {
                        ProductionSpendingUnknown::RocketInputInexact
                    },
                )),
                unproven_raw: 0,
            };
        }
        let mut totals = RocketTotals {
            restarted,
            ..RocketTotals::default()
        };
        for (&key, state) in &mut self.rows {
            #[cfg(any(test, feature = "test-support"))]
            {
                self.work.history_visits += 1;
            }
            if !seen.contains(&key) {
                state.units.inexact = 1;
                state.cost.reason = Some(ProductionSpendingUnknown::LostOutput);
            }
            let Some(identity) = state.identity else {
                continue;
            };
            if state.present {
                totals.present.insert(identity);
            }
            let entry = (key.1, Purpose::New);
            let units = totals
                .units
                .entry(identity)
                .or_default()
                .entry(entry)
                .or_default();
            units.units = units
                .units
                .checked_add(state.units.units)
                .ok_or_else(|| io::Error::other("Rocket units exceed their exact integer range"))?;
            units.inexact = units
                .inexact
                .checked_add(state.units.inexact)
                .ok_or_else(|| io::Error::other("Rocket inexact row count overflows"))?;
            let cost = totals
                .costs
                .entry(identity)
                .or_default()
                .entry(entry)
                .or_default();
            cost.raw = cost
                .raw
                .checked_add(state.cost.raw)
                .ok_or_else(|| io::Error::other("Rocket IC exceeds its exact integer range"))?;
            cost.reason = cost.reason.or(state.cost.reason);
        }
        totals.delivery_owner_unknown = self.delivery_owner_unknown;
        if self.owner_unknown {
            totals.owner_reason = Some(
                self.rows[&(0, 0)]
                    .cost
                    .reason
                    .unwrap_or(ProductionSpendingUnknown::RocketOwnerUnreadable),
            );
        }
        Ok(RocketFold::Read(totals))
    }
}

// Persist only the recorded Rocket operands needed to continue consistency checks.
mod previous_row {
    use super::*;
    #[derive(Deserialize, Serialize)]
    #[serde(remote = "RocketProductionOutputRow")]
    struct RecordedRow {
        country: u32,
        variant: u64,
        spent_raw: i64,
        instant_raw: i64,
        since_hour: i32,
        exact: bool,
        ticks: u64,
        inexact_ticks: u64,
        unknown_design_ticks: u64,
        unknown_country_ticks: u64,
        units: i64,
        units_exact: bool,
        deliveries: u64,
        inexact_deliveries: u64,
    }
    pub fn serialize<S: serde::Serializer>(
        value: &Option<RocketProductionOutputRow>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Present<'a>(#[serde(with = "RecordedRow")] &'a RocketProductionOutputRow);
        value.as_ref().map(Present).serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<RocketProductionOutputRow>, D::Error> {
        #[derive(Deserialize)]
        struct Present(#[serde(with = "RecordedRow")] RocketProductionOutputRow);
        Ok(Option::<Present>::deserialize(deserializer)?.map(|value| value.0))
    }
}
