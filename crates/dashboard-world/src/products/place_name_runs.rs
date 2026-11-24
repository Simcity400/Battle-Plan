use crate::products::place_names::{Content, Name, Places, read_hour};
use dashboard_base::products::{LoadContext, Product, at_hour};
use dashboard_base::{Source, catalogue::Refusals};
use dashboard_core::dto::RecordingGap;
use dashboard_query::read::{self, touched};
use dashboard_text::metadata::check_cancel;
use replay_data::rows::TableId;
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

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum Presence {
    Present,
    Absent,
    Unknown,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NameRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub name: Option<String>,
    pub missing: Option<String>,
    #[doc = " Whether the subject exists (for its owner) across the run: proven present, proven absent, or not known because what proves it is not recorded."]
    pub presence: Presence,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub name: Option<String>,
    pub missing: Option<String>,
    pub presence: Presence,
}

impl Answer {
    pub fn unknown(reason: String) -> Self {
        Self { name: None, missing: Some(reason), presence: Presence::Unknown }
    }
}

impl From<Name> for Answer {
    fn from(name: Name) -> Self {
        Self { name: name.name, missing: name.missing, presence: Presence::Present }
    }
}

pub fn extend_runs(runs: &mut Vec<NameRun>, first: i32, last: i32, answer: &Answer, unknown: &[RecordingGap]) {
    if let Some(held) = runs.last_mut()
        && held.name == answer.name
        && held.missing == answer.missing
        && held.presence == answer.presence
        && unknown_between(unknown, held.last_hour + 1, first - 1)
    {
        held.last_hour = held.last_hour.max(last);
        return;
    }
    runs.push(NameRun { first_hour: first, last_hour: last, name: answer.name.clone(), missing: answer.missing.clone(), presence: answer.presence });
}

pub struct RunBook<K> {
    held: HashMap<K, (Answer, i32)>,
    pub runs: BTreeMap<K, Vec<NameRun>>,
}

impl<K: Copy + Ord + std::hash::Hash> RunBook<K> {
    pub fn new(keys: impl IntoIterator<Item = K>) -> Self {
        Self { held: HashMap::new(), runs: keys.into_iter().map(|key| (key, Vec::new())).collect() }
    }

    pub fn settle(&mut self, key: K, answer: Answer, hour: i32, unknown: &[RecordingGap]) {
        match self.held.get_mut(&key) {
            Some((current, _)) if *current == answer => {}
            Some((current, since)) => {
                extend_runs(self.runs.entry(key).or_default(), *since, hour - 1, current, unknown);
                *current = answer;
                *since = hour;
            }
            None => {
                self.held.insert(key, (answer, hour));
            }
        }
    }

    pub fn close(&mut self, last: i32, unknown: &[RecordingGap]) {
        for (key, (answer, since)) in self.held.drain() {
            extend_runs(self.runs.entry(key).or_default(), since, last, &answer, unknown);
        }
    }
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
const PROVINCE_STATE: usize = 0;

pub const PLACE_TABLES: [(TableId, Option<&[usize]>); 7] = [
    (TableId::States, Some(&[0, 3, 4])),
    (TableId::Provinces, Some(&[PROVINCE_STATE])),
    (TableId::ProvinceNames, None),
    (TableId::CountryLinks, None),
    (TableId::Countries, Some(&[0, 1])),
    (TableId::CountryAppearance, None),
    (TableId::ReservedCountryNaming, None),
];

#[derive(Default)]
pub struct PlaceInputs {
    province_state: HashMap<u32, u32>,
}

impl PlaceInputs {
    pub fn dirty(&mut self, readers: &[TableReader], opening: bool, states: &[u32], provinces: &[u32]) -> (Vec<u32>, Vec<u32>) {
        let countries_moved = opening || readers[COUNTRY_TABLES].iter().any(|reader| reader.changes().rows_moved());
        let dirty_states: Vec<u32> = states.iter().copied().filter(|&id| opening || touched(&readers[STATES], u128::from(id))).collect();
        let dirty_provinces: Vec<u32> = provinces
            .iter()
            .copied()
            .filter(|&id| {
                let key = u128::from(id);
                let moved = countries_moved || touched(&readers[PROVINCES], key) || touched(&readers[PROVINCE_NAMES], key);
                moved || self.province_state.get(&id).is_some_and(|&state| state != 0 && touched(&readers[STATES], u128::from(state)))
            })
            .collect();
        for &id in &dirty_provinces {
            match read::number(&readers[PROVINCES], u128::from(id), PROVINCE_STATE).and_then(|state| u32::try_from(state).ok()) {
                Some(state) => self.province_state.insert(id, state),
                None => self.province_state.remove(&id),
            };
        }
        (dirty_states, dirty_provinces)
    }
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
        vec![PlaceNameRunsParams::decl(), Presence::decl(), NameRun::decl(), PlaceRuns::decl(), PlaceNameRunsReply::decl()]
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
        let mut state_book = RunBook::new(states.iter().copied());
        let mut province_book = RunBook::new(provinces.iter().copied());
        let recording = query.replay.recording();
        let mut frames = at_hour::Frames::new(&query)?;
        for stretch in query.timeline.stretches.iter().filter(|stretch| stretch.first <= to && stretch.last >= from) {
            let (first, last) = (stretch.first.max(from), stretch.last.min(to));
            let mut inputs = PlaceInputs::default();
            read::walk(recording, stretch.run, &PLACE_TABLES, first, last, |hour, readers: &[TableReader]| {
                check_cancel(cancelled)?;
                let (dirty_states, dirty_provinces) = inputs.dirty(readers, hour == first, &states, &provinces);
                if dirty_states.is_empty() && dirty_provinces.is_empty() {
                    return Ok(());
                }
                let named = read_hour(&query, &mut frames, hour, &dirty_provinces, &dirty_states);
                let places = named.as_ref().map(Places::new);
                let answer = |name: &dyn Fn(&Places) -> Name| match &places {
                    Ok(places) => Answer::from(name(places)),
                    Err(reason) => Answer::unknown(String::clone(reason)),
                };
                for &id in &dirty_states {
                    state_book.settle(id, answer(&|places| places.state(&content, id)), hour, &unknown);
                }
                for &id in &dirty_provinces {
                    province_book.settle(id, answer(&|places| places.province(&content, id, params.land)), hour, &unknown);
                }
                Ok(())
            })?;
            state_book.close(last, &unknown);
            province_book.close(last, &unknown);
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
        let states = collect(state_book.runs, "state");
        let provinces = collect(province_book.runs, "province");
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

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_query::read;

    fn name(text: &str) -> Answer {
        Answer { name: Some(text.to_owned()), missing: None, presence: Presence::Present }
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
        extend_runs(&mut missing, 0, 9, &Answer::unknown("Not in this recording: state 7 at this hour".into()), &[]);
        extend_runs(&mut missing, 10, 19, &name("Sample State"), &[]);
        assert_eq!(missing.len(), 2, "an absent stretch is its own run with its reason");
    }

    fn walked_dirty(frames: Vec<replay_frame::frame::DecodedFrame>, first: i32, last: i32) -> Vec<(i32, Vec<u32>, Vec<u32>)> {
        let replay = replay::campaign::Replay::from_records(recorder_derive::writers(), dashboard_core::test_fixtures::CAMPAIGN.into(), &[dashboard_core::test_fixtures::records(1, frames)])
            .unwrap()
            .into_timelines(&Default::default())
            .unwrap();
        let source = Source::from_replay(dashboard_base::SourceKey { source_id: "places".into(), revision: "1".into() }, "synthetic.battleplan".into(), std::sync::Arc::new(replay), &AtomicBool::new(false)).unwrap();
        let query = dashboard_base::queries::query(&source, 1).unwrap();
        let run = query.timeline.stretches[0].run;
        let mut inputs = PlaceInputs::default();
        let mut seen = Vec::new();
        read::walk(query.replay.recording(), run, &PLACE_TABLES, first, last, |hour, readers| {
            let (states, provinces) = inputs.dirty(readers, hour == first, &[7], &[3]);
            if !states.is_empty() || !provinces.is_empty() {
                seen.push((hour, states, provinces));
            }
            Ok(())
        })
        .unwrap();
        seen
    }

    #[test]
    fn a_province_follows_its_states_rename_through_the_state_cell_and_a_fresh_sample_alone_changes_nothing() {
        use replay_data::rows::{ProvinceRow, StateRow, TableRows};
        use replay_frame::frame::TableView;
        let frame = |hour: i32| {
            let mut frame = dashboard_core::test_fixtures::source(1, hour, Some(1));
            let name = if hour < 1_050 { b"Sample State".to_vec() } else { b"Renamed State".to_vec() };
            *frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![StateRow { id: 7, custom_name: name, ..StateRow::default() }]));
            *frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(vec![ProvinceRow { id: 3, state: 7, ..ProvinceRow::default() }]));
            frame
        };
        let seen = walked_dirty((1_000..=1_100).map(frame).collect(), 1_000, 1_100);
        assert_eq!(seen, vec![(1_000, vec![7], vec![3]), (1_050, vec![7], vec![3])], "every table is sampled afresh each hour, yet only the opening hour and the state's rename name anything again");
    }
}
