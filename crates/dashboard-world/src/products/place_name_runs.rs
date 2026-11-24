use crate::products::place_names::{Content, Name, Places, read_hour};
use dashboard_base::products::{LoadContext, Product, at_hour};
use dashboard_base::{Source, catalogue::Refusals};
use dashboard_core::dto::RecordingGap;
use dashboard_query::read::{self, row, touched};
use dashboard_text::metadata::check_cancel;
use replay_data::rows::{TableId, tables};
use replay_recording::cells::TableReader;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlaceNameRunsParams {
    pub branch_id: String,
    #[doc = " Land's ladder: a province in no state stays unnamed rather than taking its region's name."]
    #[serde(default)]
    pub land: bool,
    pub from: i32,
    pub to: i32,
    #[serde(default)]
    pub states: Vec<u32>,
    #[serde(default)]
    pub provinces: Vec<u32>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NameRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub name: Option<String>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PlaceRuns {
    pub id: u32,
    #[doc = " One run per constant answer, in hour order; equal answers either side of an `unknown` span are"]
    #[doc = " one run, and that span's hours are still unknown."]
    pub runs: Vec<NameRun>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PlaceNameRunsReply {
    pub from: i32,
    pub to: i32,
    #[doc = " The hours inside `[from, to]` the recording does not hold, for every place alike."]
    pub unknown: Vec<RecordingGap>,
    pub states: Vec<PlaceRuns>,
    pub provinces: Vec<PlaceRuns>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(PlaceNameRunsReply);

pub(crate) fn extend_runs(runs: &mut Vec<NameRun>, first: i32, last: i32, name: &Name, unknown: &[RecordingGap]) {
    if let Some(held) = runs.last_mut()
        && held.name == name.name
        && held.missing == name.missing
        && unknown_between(unknown, held.last_hour + 1, first - 1)
    {
        held.last_hour = held.last_hour.max(last);
        return;
    }
    runs.push(NameRun { first_hour: first, last_hour: last, name: name.name.clone(), missing: name.missing.clone() });
}

fn unknown_between(unknown: &[RecordingGap], first: i32, last: i32) -> bool {
    let mut cursor = first;
    for span in unknown {
        if cursor > last {
            break;
        }
        if span.last_hour < cursor {
            continue;
        }
        if span.first_hour > cursor {
            return false;
        }
        cursor = span.last_hour + 1;
    }
    cursor > last
}

const STATES: usize = 0;
const PROVINCES: usize = 1;
const PROVINCE_NAMES: usize = 2;
const COUNTRY_TABLES: std::ops::Range<usize> = 3..7;

struct Held {
    name: Name,
    since: i32,
}

pub struct PlaceNameRuns;

impl Product for PlaceNameRuns {
    const KEY: &'static str = "place-name-runs";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = PlaceNameRunsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, land, from: Start, to: End, "states[]", "provinces[]"];
    type Reply = PlaceNameRunsReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid place name runs branch");
        }
        if params.from > params.to {
            return Err("Invalid place name runs span");
        }
        if params.states.is_empty() && params.provinces.is_empty() {
            return Err("Place name runs name no place");
        }
        Ok(())
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![PlaceNameRunsParams::decl(), NameRun::decl(), PlaceRuns::decl(), PlaceNameRunsReply::decl()]
    }

    fn load(source: &Source, params: &Self::Params, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid place name runs branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let content = Content::read(source, load)?;
        let (from, to) = (params.from.max(query.timeline.first), params.to.min(query.timeline.last));
        if from > to {
            return Err(dashboard_core::invalid("Place name runs ask outside the branch"));
        }
        let states: Vec<u32> = params.states.iter().copied().collect::<BTreeSet<_>>().into_iter().collect();
        let provinces: Vec<u32> = params.provinces.iter().copied().collect::<BTreeSet<_>>().into_iter().collect();
        let unknown: Vec<RecordingGap> = query
            .timeline
            .unheld
            .iter()
            .filter(|gap| gap.first <= to && gap.last >= from)
            .map(|gap| RecordingGap { first_hour: gap.first.max(from), last_hour: gap.last.min(to), reason: format!("Not in this recording: {}", gap.reason) })
            .collect();
        let mut state_runs: BTreeMap<u32, Vec<NameRun>> = states.iter().map(|&id| (id, Vec::new())).collect();
        let mut province_runs: BTreeMap<u32, Vec<NameRun>> = provinces.iter().map(|&id| (id, Vec::new())).collect();
        let recording = query.replay.recording();
        let format = recording.format();
        let mut frames = at_hour::Frames::new(&query)?;
        let tables: [(TableId, Option<&[usize]>); 7] = [
            (TableId::States, Some(&[0, 3, 4])),
            (TableId::Provinces, Some(&[0])),
            (TableId::ProvinceNames, None),
            (TableId::CountryLinks, None),
            (TableId::Countries, Some(&[0, 1])),
            (TableId::CountryAppearance, None),
            (TableId::ReservedCountryNaming, None),
        ];
        for stretch in query.timeline.stretches.iter().filter(|stretch| stretch.first <= to && stretch.last >= from) {
            let (first, last) = (stretch.first.max(from), stretch.last.min(to));
            let mut held_states: HashMap<u32, Held> = HashMap::new();
            let mut held_provinces: HashMap<u32, Held> = HashMap::new();
            let mut province_state: HashMap<u32, u32> = HashMap::new();
            read::walk(recording, stretch.run, &tables, first, last, |hour, readers: &[TableReader]| {
                check_cancel(cancelled)?;
                let opening = hour == first;
                let countries_moved = opening || readers[COUNTRY_TABLES].iter().any(|reader| !reader.changes().quiet());
                let dirty_states: Vec<u32> = states.iter().copied().filter(|&id| opening || touched(&readers[STATES], u128::from(id))).collect();
                let dirty_provinces: Vec<u32> = provinces
                    .iter()
                    .copied()
                    .filter(|&id| {
                        let key = u128::from(id);
                        let moved = opening || countries_moved || touched(&readers[PROVINCES], key) || touched(&readers[PROVINCE_NAMES], key);
                        moved || province_state.get(&id).is_some_and(|&state| state != 0 && touched(&readers[STATES], u128::from(state)))
                    })
                    .collect();
                if dirty_states.is_empty() && dirty_provinces.is_empty() {
                    return Ok(());
                }
                for &id in &dirty_provinces {
                    match row::<tables::Provinces>(&readers[PROVINCES], u128::from(id), format) {
                        Some(province) => province_state.insert(id, province.state),
                        None => province_state.remove(&id),
                    };
                }
                let named = read_hour(&query, &mut frames, hour, &dirty_provinces, &dirty_states);
                let places = named.as_ref().ok().map(Places::new);
                let answer = |name: Option<Name>| name.unwrap_or_else(|| Name { name: None, missing: named.as_ref().err().cloned() });
                for &id in &dirty_states {
                    let name = answer(places.as_ref().map(|places| places.state(&content, id)));
                    settle(&mut held_states, &mut state_runs, id, name, hour, &unknown);
                }
                for &id in &dirty_provinces {
                    let name = answer(places.as_ref().map(|places| places.province(&content, id, params.land)));
                    settle(&mut held_provinces, &mut province_runs, id, name, hour, &unknown);
                }
                Ok(())
            })?;
            for (id, held) in held_states {
                extend_runs(state_runs.entry(id).or_default(), held.since, last, &held.name, &unknown);
            }
            for (id, held) in held_provinces {
                extend_runs(province_runs.entry(id).or_default(), held.since, last, &held.name, &unknown);
            }
        }
        let mut refused = Refusals::default();
        let mut collect = |runs: BTreeMap<u32, Vec<NameRun>>, what: &str| -> Vec<PlaceRuns> {
            runs.into_iter()
                .map(|(id, runs)| {
                    for run in &runs {
                        if let Some(reason) = &run.missing {
                            refused.note(reason, format!("{what} {id} from {}", run.first_hour));
                        }
                    }
                    PlaceRuns { id, runs }
                })
                .collect()
        };
        let states = collect(state_runs, "state");
        let provinces = collect(province_runs, "province");
        let mut warnings = content.warnings();
        warnings.extend(content.cat.refusal_lines());
        warnings.extend(refused.lines("place names"));
        warnings.sort();
        warnings.dedup();
        let reply = PlaceNameRunsReply { from, to, unknown, states, provinces, warnings };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

fn settle(held: &mut HashMap<u32, Held>, runs: &mut BTreeMap<u32, Vec<NameRun>>, id: u32, name: Name, hour: i32, unknown: &[RecordingGap]) {
    match held.get_mut(&id) {
        Some(current) if current.name == name => {}
        Some(current) => {
            extend_runs(runs.entry(id).or_default(), current.since, hour - 1, &current.name, unknown);
            *current = Held { name, since: hour };
        }
        None => {
            held.insert(id, Held { name, since: hour });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(text: &str) -> Name {
        Name { name: Some(text.to_owned()), missing: None }
    }

    #[test]
    fn runs_do_not_grow_with_sparse_stretches_and_never_join_across_an_observed_change() {
        let mut unknown = Vec::new();
        let mut runs = Vec::new();
        for i in 0..200 {
            let hour = i * 10;
            if i > 0 {
                unknown.push(RecordingGap { first_hour: hour - 9, last_hour: hour - 1, reason: "Not in this recording: gap".into() });
            }
        }
        for i in 0..200 {
            let hour = i * 10;
            extend_runs(&mut runs, hour, hour, &name(if i < 120 { "Sample Province" } else { "Renamed Province" }), &unknown);
        }
        assert_eq!(runs.len(), 2, "a constant name across 200 sparse stretches is one run, a rename starts the next");
        assert_eq!((runs[0].first_hour, runs[0].last_hour), (0, 1190));
        assert_eq!((runs[1].first_hour, runs[1].last_hour), (1200, 1990));

        let mut contiguous = Vec::new();
        extend_runs(&mut contiguous, 0, 9, &name("Sample State"), &[]);
        extend_runs(&mut contiguous, 10, 19, &name("Sample State"), &[]);
        extend_runs(&mut contiguous, 25, 30, &name("Sample State"), &[]);
        assert_eq!(contiguous.len(), 2, "hours between that are neither recorded runs nor unknown never join");

        let mut missing = Vec::new();
        extend_runs(&mut missing, 0, 9, &Name { name: None, missing: Some("Not in this recording: state 7 at this hour".into()) }, &[]);
        extend_runs(&mut missing, 10, 19, &name("Sample State"), &[]);
        assert_eq!(missing.len(), 2, "an absent stretch is its own run with its reason");
    }
}
