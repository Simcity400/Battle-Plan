use super::battles::battle_id;
use dashboard_base::Source;
use dashboard_query::{Query, read};
use replay_columnar::manifest::Status;
use replay_data::rows::{NavalResultObservationRow as Observation, Recorded, SunkShipEntry, TableId, tables};
use replay_lanes::{
    at::Seeker,
    lanes::{self, Cell, Keys},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub const SINKING_CAUSE_NOT_RECORDED: &str = "sinking cause was not recorded";
pub const SINKING_BATTLE_NOT_RECORDED: &str = "sinking battle was not recorded";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ShipSinkingAttribution {
    Cause {
        text: String,
        assisted: bool,
    },
    Ship {
        name: String,
        class_key: Option<String>,
        class_name: Option<String>,
        assisted: bool,
    },
    AirWing {
        carrier_name: Option<String>,
        carrier_class_key: Option<String>,
        carrier_class_name: Option<String>,
        assisted: bool,
    },
    NotRecorded {
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ShipSinkingBattle {
    Linked {
        id: String,
        live_id: Option<String>,
        result_id: Option<String>,
        first_hour: Option<i32>,
        result_hour: Option<i32>,
    },
    NotRecorded {
        reason: String,
    },
}

pub fn cause_not_recorded() -> ShipSinkingAttribution {
    ShipSinkingAttribution::NotRecorded {
        reason: SINKING_CAUSE_NOT_RECORDED.to_owned(),
    }
}

pub fn battle_not_recorded() -> ShipSinkingBattle {
    ShipSinkingBattle::NotRecorded {
        reason: SINKING_BATTLE_NOT_RECORDED.to_owned(),
    }
}

pub fn resolve_attribution(
    entry: &SunkShipEntry,
    attribution_text: Option<String>,
    attribution_class_key: Option<String>,
    attribution_class_name: Option<String>,
) -> ShipSinkingAttribution {
    let assisted = entry.assisted_kill != 0;
    if entry.cause_record != 0 {
        return match attribution_text {
            Some(text) => ShipSinkingAttribution::Cause { text, assisted },
            None => cause_not_recorded(),
        };
    }
    if entry.damage_source_reference != 0 {
        let has_class = attribution_text.is_some()
            && entry.attribution_class_present != 0
            && (attribution_class_key.is_some() || attribution_class_name.is_some());
        return ShipSinkingAttribution::AirWing {
            carrier_name: attribution_text,
            carrier_class_key: has_class.then_some(attribution_class_key).flatten(),
            carrier_class_name: has_class.then_some(attribution_class_name).flatten(),
            assisted,
        };
    }
    match attribution_text {
        Some(name) => {
            let has_class = entry.attribution_class_present != 0;
            ShipSinkingAttribution::Ship {
                name,
                class_key: has_class.then_some(attribution_class_key).flatten(),
                class_name: has_class.then_some(attribution_class_name).flatten(),
                assisted,
            }
        }
        None => cause_not_recorded(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LiveIncarnation {
    first_hour: i32,
    last_hour: i32,
    deletion_hour: Option<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ResultLink {
    result_id: u64,
    result_hour: i32,
    start_hour: i32,
    end_hour: i32,
}

#[derive(Default)]
pub(crate) struct BattleIndex {
    live: HashMap<u64, Vec<LiveIncarnation>>,
    results_by_combat: HashMap<u64, Vec<ResultLink>>,
    results: HashMap<u64, i32>,
}

fn complete_at(statuses: &[lanes::StatusRun], hour: i32) -> bool {
    statuses
        .iter()
        .any(|run| run.first <= hour && hour <= run.last && matches!(run.status, Status::Complete))
}

impl BattleIndex {
    pub(crate) fn build(
        source: &Source,
        query: &Query<'_>,
        requested_hour: i32,
        needed_combats: &BTreeSet<u64>,
        needed_results: &BTreeSet<u64>,
    ) -> io::Result<Self> {
        let mut index = Self::default();
        if needed_combats.is_empty() && needed_results.is_empty() {
            return Ok(index);
        }
        let recording = query.replay.recording();
        let mut seeker = Seeker::new(recording)?;
        let mut combats = needed_combats.clone();

        for stretch in &query.timeline.stretches {
            if stretch.first > requested_hour {
                continue;
            }
            let cap = stretch.last.min(requested_hour);
            let run = stretch.run;

            let mut res_by_read: BTreeMap<i32, Vec<u128>> = BTreeMap::new();
            let res_lives = dashboard_base::queries::lifetimes(source, run, TableId::NavalResults)?;
            for &(key, born, gone) in res_lives.iter() {
                if born > i64::from(cap) || gone < i64::from(stretch.first) {
                    continue;
                }
                let id = key as u64;
                if !needed_results.is_empty() && needed_combats.is_empty() && !needed_results.contains(&id) {
                    continue;
                }
                let at = (gone.min(i64::from(cap)) as i32).max(stretch.first);
                res_by_read.entry(at).or_default().push(key);
            }
            let mut stretch_results: HashMap<u64, i32> = HashMap::new();
            for (read_hour, mut keys) in res_by_read {
                keys.sort_unstable();
                keys.dedup();
                let frame = dashboard_query::cost::storage(|| {
                    seeker.frame(
                        run,
                        read_hour,
                        &[
                            read::Read { table: TableId::NavalResults, columns: None, keys: Keys::These(&keys) },
                            read::Read { table: TableId::NavalResultObservations, columns: None, keys: Keys::These(&keys) },
                        ],
                    )
                })?;
                let Ok(res_rows) = frame.complete::<tables::NavalResults>() else { continue };
                let obs_map: HashMap<u64, &Observation> = frame
                    .complete::<tables::NavalResultObservations>()
                    .map(|rows| rows.iter().map(|row| (row.result, row)).collect())
                    .unwrap_or_default();
                for row in res_rows {
                    if row.result_hour > requested_hour {
                        continue;
                    }
                    stretch_results.insert(row.id, row.result_hour);
                    index.results.entry(row.id).or_insert(row.result_hour);
                    if let Some(obs) = obs_map.get(&row.id).copied()
                        && obs.refusal == Observation::OBSERVED
                        && obs.combat != 0
                        && let Recorded::Value(first_hour) = obs.first_hour
                        && (combats.contains(&obs.combat) || needed_results.contains(&row.id))
                    {
                        combats.insert(obs.combat);
                        index.results_by_combat.entry(obs.combat).or_default().push(ResultLink {
                            result_id: row.id,
                            result_hour: row.result_hour,
                            start_hour: first_hour,
                            end_hour: row.result_hour,
                        });
                    }
                }
            }

            if !index.results.is_empty()
                && recording.predates(TableId::NavalResultSources.index()).is_none()
                && recording.withheld_reason(TableId::NavalResultSources.index()).is_none()
            {
                let mut rows_found: Vec<(i32, u64, u64, i32)> = Vec::new();
                dashboard_query::cost::storage(|| {
                    lanes::log_span(recording, run, TableId::NavalResultSources, &[0, 1, 2, 5], Keys::All, stretch.first, cap, &mut |_hour, _key, cells| {
                        let [Cell::Number(game_hour), Cell::Number(result), Cell::Number(combat), Cell::Number(created)] = cells else {
                            return;
                        };
                        let (game_hour, result, combat, created) = (*game_hour as i32, *result as u64, *combat as u64, *created as i32);
                        if combat != 0 && game_hour <= requested_hour {
                            rows_found.push((game_hour, result, combat, created));
                        }
                    })
                })?;
                for (game_hour, result, combat, created) in rows_found {
                    let Some(&result_hour) = stretch_results.get(&result).or_else(|| index.results.get(&result)) else {
                        continue;
                    };
                    if combats.contains(&combat) || needed_results.contains(&result) {
                        combats.insert(combat);
                        index.results_by_combat.entry(combat).or_default().push(ResultLink {
                            result_id: result,
                            result_hour,
                            start_hour: created,
                            end_hour: game_hour.min(result_hour),
                        });
                    }
                }
            }
        }

        if combats.is_empty() {
            return Ok(index);
        }

        let mut merged_live: HashMap<(u64, i32), LiveIncarnation> = HashMap::new();
        for stretch in &query.timeline.stretches {
            if stretch.first > requested_hour {
                continue;
            }
            let cap = stretch.last.min(requested_hour);
            let run = stretch.run;
            let (from, to) = (i64::from(stretch.first), i64::from(cap));
            let held: BTreeSet<u128> = dashboard_base::queries::lifetimes(source, run, TableId::NavalBattles)?
                .iter()
                .filter(|&&(key, born, gone)| born <= to && gone >= from && combats.contains(&(key as u64)))
                .map(|&(key, ..)| key)
                .collect();
            let (Some(&low), Some(&high)) = (held.first(), held.last()) else { continue };
            let presences = dashboard_query::cost::presences(recording, run, TableId::NavalBattles, (low, high), from, to)?;
            let mut asks: Vec<(i32, u128)> = Vec::new();
            let mut spans: HashMap<(u64, i32), i32> = HashMap::new();
            let mut needs_deletion_check = false;
            for (key, lived) in presences.iter().filter(|(key, _)| held.contains(key)) {
                for &(born, gone) in lived {
                    if born > to || gone < from {
                        continue;
                    }
                    let end = (gone.min(to) as i32).max(stretch.first);
                    if gone < to {
                        needs_deletion_check = true;
                    }
                    asks.push((end, *key));
                    spans.insert((*key as u64, end), gone as i32);
                }
            }
            if asks.is_empty() {
                continue;
            }
            let combat_statuses = if needs_deletion_check {
                lanes::statuses(recording, run, TableId::NavalCombats, stretch.first, cap)?
            } else {
                Vec::new()
            };
            let mut raw_incarnations: Vec<(u64, i32, i32, Option<i32>)> = Vec::new();
            lanes::at_each(recording, run, TableId::NavalBattles, Some(&[0]), &asks, &mut |hour, key, row| {
                let Some((view, slot)) = row else { return Ok(()) };
                let Some(first_hour) = view.number(0, slot).and_then(|value| i32::try_from(value).ok()) else {
                    return Ok(());
                };
                if first_hour > requested_hour {
                    return Ok(());
                }
                let combat = key as u64;
                let exact_gone = spans.get(&(combat, hour)).copied().unwrap_or(hour);
                let deletion_hour = (exact_gone < cap
                    && exact_gone.checked_add(1).is_some_and(|next| next <= requested_hour && complete_at(&combat_statuses, exact_gone) && complete_at(&combat_statuses, next)))
                .then_some(exact_gone + 1);
                raw_incarnations.push((combat, first_hour, hour, deletion_hour));
                Ok(())
            })?;
            let starts: BTreeSet<(u64, i32)> = raw_incarnations.iter().map(|&(combat, first_hour, ..)| (combat, first_hour)).collect();
            for (combat, first_hour, last_hour, deletion_hour) in raw_incarnations {
                let deletion_hour = deletion_hour.filter(|&del| !starts.contains(&(combat, del)));
                merged_live
                    .entry((combat, first_hour))
                    .and_modify(|existing| {
                        if last_hour >= existing.last_hour {
                            existing.last_hour = last_hour;
                            existing.deletion_hour = deletion_hour;
                        }
                    })
                    .or_insert(LiveIncarnation {
                        first_hour,
                        last_hour,
                        deletion_hour,
                    });
            }
        }

        for ((combat, _), inc) in merged_live {
            index.live.entry(combat).or_default().push(inc);
        }
        for list in index.live.values_mut() {
            list.sort_by_key(|inc| (inc.first_hour, inc.last_hour));
        }
        Ok(index)
    }

    pub(crate) fn result_hour(&self, result_id: u64) -> Option<i32> {
        self.results.get(&result_id).copied()
    }

    pub(crate) fn resolve_sinking(&self, combat: u64, sink_hour: i32) -> ShipSinkingBattle {
        if combat == 0 {
            return battle_not_recorded();
        }
        let live = if let Some(incarnations) = self.live.get(&combat) {
            let exact: Vec<LiveIncarnation> = incarnations
                .iter()
                .copied()
                .filter(|inc| inc.first_hour <= sink_hour && sink_hour <= inc.last_hour)
                .collect();
            if exact.len() > 1 {
                return battle_not_recorded();
            }
            if let Some(one) = exact.first().copied() {
                Some(one)
            } else {
                let del: Vec<LiveIncarnation> = incarnations
                    .iter()
                    .copied()
                    .filter(|inc| inc.deletion_hour == Some(sink_hour))
                    .collect();
                if del.len() > 1 {
                    return battle_not_recorded();
                }
                del.first().copied()
            }
        } else {
            None
        };

        let mut matched_results: BTreeMap<u64, i32> = BTreeMap::new();
        if let Some(links) = self.results_by_combat.get(&combat) {
            for link in links {
                if link.start_hour <= sink_hour && sink_hour <= link.end_hour {
                    matched_results.insert(link.result_id, link.result_hour);
                }
            }
        }
        if matched_results.len() > 1 {
            return battle_not_recorded();
        }
        let result = matched_results.into_iter().next();

        match (live, result) {
            (Some(inc), Some((result_id, result_hour))) => {
                if inc.first_hour > result_hour {
                    return battle_not_recorded();
                }
                let res_id = format!("result:{result_id}");
                ShipSinkingBattle::Linked {
                    id: res_id.clone(),
                    live_id: Some(battle_id(combat, inc.first_hour)),
                    result_id: Some(res_id),
                    first_hour: Some(inc.first_hour),
                    result_hour: Some(result_hour),
                }
            }
            (None, Some((result_id, result_hour))) => {
                let res_id = format!("result:{result_id}");
                ShipSinkingBattle::Linked {
                    id: res_id.clone(),
                    live_id: None,
                    result_id: Some(res_id),
                    first_hour: None,
                    result_hour: Some(result_hour),
                }
            }
            (Some(inc), None) => {
                let lid = battle_id(combat, inc.first_hour);
                ShipSinkingBattle::Linked {
                    id: lid.clone(),
                    live_id: Some(lid),
                    result_id: None,
                    first_hour: Some(inc.first_hour),
                    result_hour: None,
                }
            }
            (None, None) => battle_not_recorded(),
        }
    }

    pub(crate) fn matches_live(&self, combat: u64, first_hour: i32, entry: &SunkShipEntry) -> bool {
        if entry.battle != combat {
            return false;
        }
        let expected = battle_id(combat, first_hour);
        matches!(
            self.resolve_sinking(entry.battle, entry.sink_hour),
            ShipSinkingBattle::Linked { live_id: Some(live_id), .. } if live_id == expected
        )
    }

    pub(crate) fn matches_result(&self, result_id: u64, entry: &SunkShipEntry) -> bool {
        if entry.battle == 0 {
            return false;
        }
        let expected = format!("result:{result_id}");
        matches!(
            self.resolve_sinking(entry.battle, entry.sink_hour),
            ShipSinkingBattle::Linked { result_id: Some(linked), .. } if linked == expected
        )
    }
}
