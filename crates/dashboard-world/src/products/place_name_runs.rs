use crate::products::place_names::{Content, Name, PlaceHistory, ProvinceMode};
use dashboard_base::{
    Source,
    catalogue::Refusals,
    products::{LoadContext, Product},
};
use dashboard_core::dto::RecordingGap;
use dashboard_text::metadata::check_cancel;
use replay_lanes::lanes::Keys;
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

const PLACE_ABSENT: &str = "No such place at this hour";

fn answer_of(reading: Result<Option<Name>, String>) -> Answer {
    match reading {
        Ok(Some(name)) => Answer { name: name.name, missing: name.missing, presence: Presence::Present },
        Ok(None) => Answer { name: None, missing: Some(PLACE_ABSENT.into()), presence: Presence::Absent },
        Err(reason) => Answer::unknown(reason),
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
        let state_keys: Vec<u128> = states.iter().copied().map(u128::from).collect();
        let province_keys: Vec<u128> = provinces.iter().copied().map(u128::from).collect();
        let mode = ProvinceMode::from_land(params.land);
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
        for stretch in query.timeline.stretches.iter().filter(|stretch| stretch.first <= to && stretch.last >= from) {
            let (first, last) = (stretch.first.max(from), stretch.last.min(to));
            let history = PlaceHistory::load(
                recording,
                stretch.run,
                first,
                last,
                Keys::These(&province_keys),
                Keys::These(&state_keys),
                false,
                cancelled,
            )?;
            for &id in &states {
                check_cancel(cancelled)?;
                for hour in history.state_change_hours(id, first, last) {
                    state_book.settle(id, answer_of(history.state_reading(&content, id, hour)), hour, &unknown);
                }
            }
            for &id in &provinces {
                check_cancel(cancelled)?;
                for hour in history.province_change_hours(id, first, last) {
                    province_book.settle(id, answer_of(history.province_reading(&content, id, mode, hour)), hour, &unknown);
                }
            }
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
    use replay_data::rows::{ProvinceRow, StateRow, TableId, TableRows};
    use replay_frame::frame::TableView;

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

    fn history_for(frames: Vec<replay_frame::frame::DecodedFrame>, first: i32, last: i32, states: &[u32], provinces: &[u32]) -> PlaceHistory {
        let replay = replay::campaign::Replay::from_records(
            recorder_derive::writers(),
            dashboard_core::test_fixtures::CAMPAIGN.into(),
            &[dashboard_core::test_fixtures::records(1, frames)],
        )
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
        let source = Source::from_replay(
            dashboard_base::SourceKey { source_id: "places".into(), revision: "1".into() },
            "synthetic.battleplan".into(),
            std::sync::Arc::new(replay),
            &AtomicBool::new(false),
        )
        .unwrap();
        let query = dashboard_base::queries::query(&source, 1).unwrap();
        let run = query.timeline.stretches[0].run;
        let state_keys: Vec<u128> = states.iter().copied().map(u128::from).collect();
        let province_keys: Vec<u128> = provinces.iter().copied().map(u128::from).collect();
        PlaceHistory::load(
            query.replay.recording(),
            run,
            first,
            last,
            Keys::These(&province_keys),
            Keys::These(&state_keys),
            false,
            &AtomicBool::new(false),
        )
        .unwrap()
    }

    #[test]
    fn a_place_is_present_only_on_its_witnessed_row_and_absent_only_from_a_complete_read() {
        let make = |hour: i32, view: fn(TableRows) -> TableView| {
            let mut frame = dashboard_core::test_fixtures::source(1, hour, Some(1));
            *frame.tables[TableId::States.index()] = view(TableRows::States(vec![StateRow { id: 7, custom_name: b"Sample State".to_vec(), ..StateRow::default() }]));
            frame
        };
        let root = std::env::temp_dir().join(format!("place-runs-presence-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let (source, load) = dashboard_base::test_fixtures::source_with_content(
            vec![
                make(1_000, TableView::Complete),
                make(1_001, |rows| TableView::Truncated { rows, missing: 3 }),
                dashboard_core::test_fixtures::source(1, 1_002, Some(1)),
            ],
            &root,
        );
        let query = dashboard_base::queries::query(&source, 1).unwrap();
        let content = Content::read(&source, &load).unwrap();
        let history = PlaceHistory::load(
            query.replay.recording(),
            query.timeline.stretches[0].run,
            1_000,
            1_002,
            Keys::These(&[3]),
            Keys::These(&[7, 8]),
            false,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(answer_of(history.state_reading(&content, 7, 1_000)).presence, Presence::Present);
        assert_eq!(answer_of(history.state_reading(&content, 8, 1_000)).presence, Presence::Absent, "a complete read without the row proves the state does not exist");
        assert_eq!(answer_of(history.state_reading(&content, 7, 1_001)).presence, Presence::Present, "a witnessed row is present even in a truncated read");
        assert_eq!(answer_of(history.state_reading(&content, 8, 1_001)).presence, Presence::Unknown, "a truncated read without the row proves nothing");
        assert_eq!(answer_of(history.province_reading(&content, 3, ProvinceMode::Place, 1_002)).presence, Presence::Unknown);
        assert_eq!(history.state_change_hours(7, 1_000, 1_002), vec![1_000, 1_002]);
        assert_eq!(history.state_change_hours(8, 1_000, 1_002), vec![1_000, 1_001, 1_002]);
        drop(query);
        drop(source);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_state_that_drops_out_of_a_truncated_read_is_named_again_though_the_missing_count_stays_the_same() {
        let frame = |hour: i32| {
            let mut frame = dashboard_core::test_fixtures::source(1, hour, Some(1));
            let state = |id: u32| StateRow { id, custom_name: b"Sample State".to_vec(), ..StateRow::default() };
            *frame.tables[TableId::States.index()] = match hour {
                1_000 => TableView::Complete(TableRows::States(vec![state(7), state(8)])),
                1_001 => TableView::Truncated { rows: TableRows::States(vec![state(7)]), missing: 1 },
                _ => TableView::Truncated { rows: TableRows::States(vec![state(8)]), missing: 1 },
            };
            frame
        };
        let history = history_for((1_000..=1_003).map(frame).collect(), 1_000, 1_003, &[7, 8], &[]);
        assert_eq!(history.state_change_hours(7, 1_000, 1_003), vec![1_000, 1_002]);
        assert_eq!(history.state_change_hours(8, 1_000, 1_003), vec![1_000, 1_001, 1_002]);
    }

    #[test]
    fn a_province_follows_its_states_rename_through_the_state_cell_and_a_fresh_sample_alone_changes_nothing() {
        let frame = |hour: i32| {
            let mut frame = dashboard_core::test_fixtures::source(1, hour, Some(1));
            let name = if hour < 1_050 { b"Sample State".to_vec() } else { b"Renamed State".to_vec() };
            *frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![StateRow { id: 7, custom_name: name, ..StateRow::default() }]));
            *frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(vec![ProvinceRow { id: 3, state: 7, ..ProvinceRow::default() }]));
            frame
        };
        let history = history_for((1_000..=1_100).map(frame).collect(), 1_000, 1_100, &[7], &[3]);
        assert_eq!(history.state_change_hours(7, 1_000, 1_100), vec![1_000, 1_050]);
        assert_eq!(history.province_change_hours(3, 1_000, 1_100), vec![1_000, 1_050]);
    }
}
