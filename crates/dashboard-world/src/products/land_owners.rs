
use dashboard_base::products::{LoadContext, Product};
use super::styles;
use dashboard_base::Source;
use dashboard_core::invalid;
use dashboard_core::dto::CountryMetadata;
use dashboard_query::{coverage::row_coverage, identity::Holder, read};
use serde::{Deserialize, Serialize};
use replay_recording::columnar_frames::Recording;
use replay_lanes::lanes;
use replay_columnar::{manifest::Status, summary::{self, Source as Summary}};
use replay_data::rows::{TableId, TerritoryTotalRow};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct LandOwnersProduct;

#[derive(Clone, Debug, PartialEq, Default, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(deny_unknown_fields)]
pub struct LandOwnersParams {}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LandOwners {
    pub countries: Vec<CountryMetadata>,
    pub countries_missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(LandOwners);

fn witnesses(source: &Source, cancelled: &AtomicBool, missing: &mut Option<String>) -> io::Result<BTreeMap<(String, u32), (u64, i32)>> {
    let replay = source.replay_ref();
    let timelines = replay.timelines().ok_or_else(|| invalid("The recording holds no history"))?;
    let mut lives: HashMap<u32, Vec<(u32, i64, i64)>> = HashMap::new();
    let mut found: BTreeMap<(String, u32), (u64, i32)> = BTreeMap::new();
    for timeline in &timelines.list {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let query = dashboard_base::queries::query(source, timeline.id)?;
        if let Some((_, _, reason)) = query.holders.unknown.first() {
            missing.get_or_insert_with(|| reason.clone());
        }
        let mut holders: HashMap<u32, Vec<(&str, i32, i32)>> = HashMap::new();
        for (tag, runs) in &query.holders.tags {
            for run in runs {
                if let Holder::Found { slot, .. } = run.holder {
                    holders.entry(slot).or_default().push((tag.as_str(), run.first, run.last));
                }
            }
        }
        for stretch in &query.timeline.stretches {
            if missing.is_none() {
                let (recording, table) = (replay.recording(), TableId::TerritoryTotals);
                let statuses = read::statuses(recording, stretch.run, table, stretch.first, stretch.last)?;
                if let Some(status) = statuses.iter().find(|status| status.status != Status::Complete) {
                    let reasons = read::status_reasons(recording, stretch.run, table, std::slice::from_ref(status))?;
                    let reason = match status.status {
                        Status::Unavailable { reason } => reasons.get(&reason).map(String::as_str),
                        _ => None,
                    };
                    *missing = row_coverage(table, status.status, reason, false).into_missing();
                }
            }
            if let std::collections::hash_map::Entry::Vacant(vacant) = lives.entry(stretch.run) {
                let read = crate::products::land_owners::cached_territory(source, stretch.run)?;
                if let Some(hour) = read.unvalued {
                    missing.get_or_insert_with(|| format!("Territory totals did not record a country's land holding at hour {hour}"));
                }
                vacant.insert(read.spans.iter().map(|&(key, first, last)| (key as u32, first, last)).collect());
            }
            for &(slot, first, last) in &lives[&stretch.run] {
                let (first, last) = (first.max(i64::from(stretch.first)), last.min(i64::from(stretch.last)));
                if first > last {
                    continue;
                }
                for &(tag, held_first, held_last) in holders.get(&slot).into_iter().flatten() {
                    let (from, to) = (first.max(i64::from(held_first)), last.min(i64::from(held_last)));
                    if from > to {
                        continue;
                    }
                    let seen = (timeline.id, from as i32);
                    found.entry((tag.to_owned(), slot)).and_modify(|held| *held = (*held).min(seen)).or_insert(seen);
                }
            }
        }
    }
    Ok(found)
}

impl Product for LandOwnersProduct {
    const KEY: &'static str = "land-owners";
    const CONTENT_DEPENDENT: bool = true;
    type Params = LandOwnersParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![];
    type Reply = LandOwners;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![LandOwnersParams::decl(), LandOwners::decl()]
    }

    fn validate(_: &Self::Params) -> Result<(), &'static str> {
        Ok(())
    }

    fn load(source: &Source, _: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let mut countries_missing = None;
        let found = witnesses(source, cancelled, &mut countries_missing)?;
        let mut warnings = Vec::new();
        let localisation = styles::localisation(source, context.content.as_ref(), &mut warnings)?;
        let mut names: HashMap<(String, u64, i32), String> = HashMap::new();
        let mut failures: Vec<String> = Vec::new();
        if let Some(localisation) = &localisation {
            let mut looks: HashMap<u32, std::sync::Arc<styles::RunLooks>> = HashMap::new();
            let mut evaluator: Option<styles::ScriptedEvaluator> = None;
            for ((tag, _), &(timeline, hour)) in &found {
                dashboard_text::metadata::check_cancel(cancelled)?;
                let query = dashboard_base::queries::query(source, timeline)?;
                let Some(stretch) = query.timeline.at(hour).copied() else { continue };
                if let std::collections::hash_map::Entry::Vacant(slot) = looks.entry(stretch.run) {
                    slot.insert(crate::products::styles::cached_looks(source, stretch.run)?);
                }
                let run = &looks[&stretch.run];
                let look = run.spans.iter().find(|&&(number, _, first, last)| {
                    first <= hour && hour <= last && run.tags.get(&number).is_some_and(|held| **held == **tag)
                });
                if let Some(look) = look.and_then(|&(_, look, ..)| run.looks.get(&look)) {
                    let (style, problem) = styles::style_with(tag, look, Some(localisation), |key, text, slot| {
                        let eval = evaluator.get_or_insert_with(|| styles::ScriptedEvaluator::for_source(source, context.content.as_ref()));
                        eval.expand(stretch.run, hour, slot, key, text)
                    });
                    if let Some(name) = style.name {
                        names.insert((tag.clone(), timeline, hour), name);
                    } else if let Some(reason) = problem {
                        failures.push(format!("{tag}: {reason}"));
                    }
                }
            }
        }
        let mut unnamed = 0;
        let mut countries: Vec<(String, u64, u32, CountryMetadata)> = found
            .into_iter()
            .map(|((tag, slot), (timeline, hour))| {
                let name = names.remove(&(tag.clone(), timeline, hour)).unwrap_or_else(|| {
                    unnamed += 1;
                    tag.clone()
                });
                let entry = CountryMetadata { id: slot.to_string(), tag: tag.clone(), name, observed_branch_id: timeline.to_string(), observed_hour: hour };
                (tag, timeline, slot, entry)
            })
            .collect();
        countries.sort_unstable_by(|a, b| (&a.0, a.1, a.2).cmp(&(&b.0, b.1, b.2)));
        if localisation.is_some() && unnamed > 0 {
            warnings.push(format!("{unnamed} country names could not be resolved from the recorded content and show their tag."));
            failures.sort();
            failures.dedup();
            warnings.extend(failures);
        }
        Ok(LandOwners { countries: countries.into_iter().map(|(.., entry)| entry).collect(), countries_missing, warnings })
    }
}

#[derive(Default)]
pub struct Holdings {
    spans: Vec<(u128, i64, i64)>,
    unvalued: Option<i32>,
}

pub fn holding(recording: &Recording, run: u32) -> io::Result<Holdings> {
    let series = dashboard_declare::series("territory").ok_or_else(|| invalid("Territory is not declared"))?;
    let measure = series.measure("holding").ok_or_else(|| invalid("Territory declares no holding"))?;
    let source = dashboard_declare::source(series.table, measure)?;
    let held = recording.run(run)?;
    let (first, last) = (held.first_hour, held.first_hour + held.ordinals as i32 - 1);
    if held.ordinals == 0 {
        return Ok(Holdings::default());
    }
    let hours = i64::from(last) - i64::from(first) + 1;
    let mut level = 0u8;
    while level < 2 && summary::bucket_hours(level + 1) * 16 <= hours {
        level += 1;
    }
    let mut spans: BTreeMap<u128, Vec<(i64, i64)>> = BTreeMap::new();
    let unvalued = refine(recording, run, source, None, first, last, level, &mut spans)?;
    let mut out = Vec::new();
    for (key, mut held) in spans {
        held.sort_unstable();
        let mut joined: Vec<(i64, i64)> = Vec::with_capacity(held.len());
        for (a, b) in held {
            match joined.last_mut() {
                Some(span) if span.1 + 1 >= a => span.1 = span.1.max(b),
                _ => joined.push((a, b)),
            }
        }
        out.extend(joined.into_iter().map(|(a, b)| (key, a, b)));
    }
    Ok(Holdings { spans: out, unvalued })
}

fn refine(recording: &Recording, run: u32, source: Summary, keys: Option<&[u128]>, from: i32, to: i32, level: u8, spans: &mut BTreeMap<u128, Vec<(i64, i64)>>) -> io::Result<Option<i32>> {
    let world = u128::from(TerritoryTotalRow::WORLD);
    let mut unvalued = None;
    if level == lanes::WALKED {
        let (covered, _) = lanes::keyed_covered_runs(recording, run, TableId::TerritoryTotals, source, keys, from, to)?;
        for (key, runs) in covered {
            if key == world {
                continue;
            }
            for (a, b, value) in runs {
                match value {
                    lanes::CoveredValue::Value(value) if value > 0 => spans.entry(key).or_default().push((i64::from(a.max(from)), i64::from(b.min(to)))),
                    lanes::CoveredValue::Unvalued => unvalued = Some(unvalued.map_or(a, |old: i32| old.min(a))),
                    _ => {}
                }
            }
        }
        return Ok(unvalued);
    }
    let bin = summary::bucket_hours(level);
    let chart = dashboard_query::cost::storage(|| lanes::chart_at(recording, run, TableId::TerritoryTotals, source.id(), keys, from, to, level, bin))?;
    let mut mixed: BTreeMap<(i32, i32), Vec<u128>> = BTreeMap::new();
    for (at, &key) in chart.keys.iter().enumerate() {
        if key == world {
            continue;
        }
        for (index, value) in chart.values(at).iter().enumerate() {
            let start = chart.first_bin_hour + index as i64 * bin;
            let (lo, hi) = (start.max(i64::from(from)), (start + bin - 1).min(i64::from(to)));
            if lo > hi || (value.max <= 0 && chart.life(at)[index] <= value.hours) {
                continue;
            }
            if value.min > 0 && i64::from(value.hours) == bin && hi - lo + 1 == bin {
                spans.entry(key).or_default().push((lo, hi));
            } else {
                mixed.entry((lo as i32, hi as i32)).or_default().push(key);
            }
        }
    }
    let finer = if level == 0 { lanes::WALKED } else { level - 1 };
    for ((lo, hi), mut keys) in mixed {
        keys.sort_unstable();
        if let Some(hour) = refine(recording, run, source, Some(&keys), lo, hi, finer, spans)? {
            unvalued = Some(unvalued.map_or(hour, |old| old.min(hour)));
        }
    }
    Ok(unvalued)
}

enum RunKey {}

type Runs = dashboard_base::queries::RunCache<RunKey, Holdings>;

pub(crate) fn forget(source: &dashboard_base::Source) {
    Runs::forget(source);
}

pub fn cached_territory(source: &dashboard_base::Source, run: u32) -> io::Result<std::sync::Arc<Holdings>> {
    Runs::held(&source.state::<Runs>(), run, || holding(source.replay_ref().recording(), run))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_base::test_fixtures::{replay, source};
    use replay_data::rows::{StateRow, TableRows};
    use replay_frame::frame::TableView;
    use std::sync::Arc;

    #[test]
    fn owners_distinguish_read_empty_from_unread_territory_and_keep_the_known_owner() {
        let empty = TableView::Complete(TableRows::Provinces(vec![]));
        let unread = TableView::Unavailable { reason: "fixture provinces failed".into() };
        let state = |owner, controller| StateRow { id: 17, owner, controller, ..Default::default() };
        let ask = |views: Vec<(TableView, Option<StateRow>)>| {
            let frames = views.into_iter().enumerate().map(|(offset, (view, state))| {
                let mut frame = source(1, 100 + offset as i32, Some(1));
                *frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(state.into_iter().collect()));
                *frame.tables[TableId::Provinces.index()] = view;
                frame
            }).collect();
            let replay = replay(frames).into_timelines(&Default::default()).unwrap();
            let source = Source::from_replay(dashboard_base::SourceKey { source_id: "territory-choices".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &false.into()).unwrap();
            LandOwnersProduct::load(&source, &LandOwnersParams {}, &LoadContext { content: None }, &false.into()).unwrap()
        };
        let none = ask(vec![(empty.clone(), None)]);
        assert!(none.countries.is_empty());
        assert_eq!(serde_json::to_value(&none).unwrap().get("countriesMissing"), Some(&serde_json::Value::Null));
        let missing = ask(vec![(unread.clone(), None)]);
        assert!(missing.countries.is_empty());
        assert!(serde_json::to_value(&missing).unwrap()["countriesMissing"].as_str().is_some_and(|reason| reason.contains("fixture provinces failed")));
        let partial = ask(vec![(empty.clone(), Some(state(9001, 9001))), (unread, None)]);
        assert_eq!(partial.countries.len(), 1);
        assert!(serde_json::to_value(&partial).unwrap()["countriesMissing"].is_string());
        let unknown = ask(vec![(empty, Some(state(8052, 0)))]);
        assert!(unknown.countries.is_empty());
        assert!(unknown.countries_missing.as_deref().is_some_and(|reason| reason.contains("land holding")));
    }
}
