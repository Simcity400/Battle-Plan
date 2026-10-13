//! Independent retained Rocket totals within the shared branch walk.

use super::{CostTotal, Identity, ProductionSpendingUnknown, TagHistory, Total};
use crate::metadata::{TaggedCountry, link_slot, slot_read, tagged_countries};
use replay::{
    frame::DecodedFrame,
    rows::{RocketProductionOutputRow, TableId, tables},
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
};

#[derive(Clone, Debug)]
struct Row {
    identity: Option<Identity>,
    units: Total,
    cost: CostTotal,
    present: bool,
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

#[derive(Clone, Debug, Default)]
pub(crate) struct RocketProduced {
    pub tags: BTreeMap<String, TagHistory>,
    pub reads: Vec<i32>,
    pub observations: Vec<i32>,
    pub unread: usize,
    pub unread_reason: Option<String>,
    pub unattributed: usize,
    pub inconsistent: usize,
    observation: u32,
    rows: HashMap<(u32, u64), Row>,
    owner_unknown: bool,
    delivery_owner_unknown: bool,
}

impl RocketProduced {
    fn end_reads(&mut self) {
        for track in self.tags.values_mut() {
            track.last = None;
        }
    }

    pub fn observe(
        &mut self,
        format: replay::format::FormatVersion,
        frame: &DecodedFrame,
    ) -> io::Result<()> {
        let hour = frame.hour;
        let read = slot_read(
            format,
            &frame.metrics,
            TableId::RocketProductionOutput,
            hour,
        );
        let (Ok(countries), Ok(links), Ok(output)) = (
            frame.complete::<tables::Countries>(),
            frame.complete::<tables::CountryLinks>(),
            frame.complete::<tables::RocketProductionOutput>(),
        ) else {
            if read {
                self.unread += 1;
            }
            self.unread_reason.get_or_insert_with(|| {
                "RocketProductionOutput or its country identities are unavailable".into()
            });
            self.end_reads();
            return Ok(());
        };
        let tagged: Vec<(String, Identity)> = tagged_countries(countries, links)
            .into_iter()
            .filter_map(|(tag, lookup)| {
                let TaggedCountry::Found(country, slot) = lookup else {
                    return None;
                };
                std::str::from_utf8(tag)
                    .ok()
                    .map(|tag| (tag.to_owned(), (slot, country.id, self.observation)))
            })
            .collect();
        if !read {
            let present: HashMap<&str, Identity> = tagged
                .iter()
                .map(|(tag, identity)| (tag.as_str(), *identity))
                .collect();
            for (tag, track) in &mut self.tags {
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
            }
            return Ok(());
        }
        // A complete empty array cannot authenticate old/unrecorded coverage.
        let Some(world) = output
            .iter()
            .find(|row| row.country == 0 && row.variant == 0)
        else {
            self.unread += 1;
            self.unread_reason
                .get_or_insert_with(|| "Rocket coverage singleton is absent".into());
            self.end_reads();
            return Ok(());
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
                self.end_reads();
            }
            Some(_) => {}
        }
        self.reads.push(hour);
        self.owner_unknown |= !world.exact;
        self.delivery_owner_unknown |= !world.units_exact;
        let observed: Vec<_> = tagged
            .into_iter()
            .map(|(tag, (slot, id, _))| (tag, (slot, id, self.observation)))
            .collect();
        let by_slot: HashMap<u32, Identity> = observed
            .iter()
            .map(|(_, identity)| (identity.0, *identity))
            .collect();
        let mut seen = BTreeSet::new();
        for row in output {
            let key = (row.country, row.variant);
            if !seen.insert(key) {
                return Err(io::Error::other(
                    "Rocket publication repeats a country/design key",
                ));
            }
            // Protocol zero is never the game's reserved country slot.
            let producer = (row.country != 0)
                .then(|| {
                    link_slot(links, row.country)
                        .and_then(|slot| by_slot.get(&slot))
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
            };
        }
        let mut unit_totals: HashMap<Identity, BTreeMap<(u64, bool), Total>> = HashMap::new();
        let mut cost_totals: HashMap<Identity, BTreeMap<(u64, bool), CostTotal>> = HashMap::new();
        let mut present_production = BTreeSet::new();
        for (&key, state) in &mut self.rows {
            if !seen.contains(&key) {
                state.units.inexact = 1;
                state.cost.reason = Some(ProductionSpendingUnknown::LostOutput);
            }
            let Some(identity) = state.identity else {
                continue;
            };
            if state.present {
                present_production.insert(identity);
            }
            let entry = (key.1, false);
            let units = unit_totals
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
            let cost = cost_totals
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
        let present: BTreeSet<&str> = observed.iter().map(|(tag, _)| tag.as_str()).collect();
        for (tag, track) in &mut self.tags {
            if !present.contains(tag.as_str()) {
                track.last = None;
            }
        }
        for (tag, identity) in observed {
            let track = self.tags.entry(tag).or_default();
            let starts = !track
                .last
                .is_some_and(|(seen, at)| seen == identity && at.checked_add(1) == Some(hour));
            let sample = u32::try_from(track.hours.len())
                .map_err(|_| io::Error::other("Rocket sample index exceeds its integer range"))?;
            let mut units = unit_totals.remove(&identity).unwrap_or_default();
            let mut costs = cost_totals.remove(&identity).unwrap_or_default();
            if self.delivery_owner_unknown {
                units.entry((0, false)).or_default().inexact = 1;
            }
            if self.owner_unknown {
                costs.entry((0, false)).or_default().reason = Some(
                    self.rows[&(0, 0)]
                        .cost
                        .reason
                        .unwrap_or(ProductionSpendingUnknown::RocketOwnerUnreadable),
                );
            }
            track.hours.push(hour);
            track.last_hours.push(hour);
            track.starts.push(starts);
            track.identities.push(identity);
            track
                .presence
                .push(if present_production.contains(&identity) {
                    Some(true)
                } else if self.owner_unknown || self.delivery_owner_unknown {
                    None
                } else {
                    Some(false)
                });
            track.last = Some((identity, hour));
            let keys: std::collections::BTreeSet<_> =
                track.current.keys().chain(units.keys()).copied().collect();
            for key in keys {
                let value = units.get(&key).copied().unwrap_or_default();
                if track.current.get(&key).copied().unwrap_or_default() != value {
                    track.changes.entry(key).or_default().push((sample, value));
                }
            }
            let keys: std::collections::BTreeSet<_> = track
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
}
