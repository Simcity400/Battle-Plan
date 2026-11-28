use crate::products::place_names::{Content, Name, PlaceHistory, ProvinceMode, RowHistory};
use dashboard_base::{
    Source,
    catalogue::Refusals,
    products::{
        LoadContext, Product,
        at_hour::ReadDates,
        scoped_text::{Scope, ScopedText, Validity, key_needs_scope},
    },
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PlacePoint {
    State(u32, i32),
    Province(u32, i32),
}

impl PlacePoint {
    fn hour(self) -> i32 {
        match self { Self::State(_, hour) | Self::Province(_, hour) => hour }
    }
}

const BROKER_WINDOW_HOURS: i64 = 720;

pub fn names_at(source: &Source, query: &dashboard_query::Query<'_>, load: &LoadContext, points: &BTreeSet<PlacePoint>, cancelled: &AtomicBool) -> io::Result<BTreeMap<PlacePoint, Name>> {
    names_at_with(source, query, load, points, cancelled, None)
}

#[derive(Default)]
pub struct RowNames {
    held: BTreeMap<PlacePoint, Name>,
    history: Option<RowHistoryWindows>,
}

#[derive(Default)]
struct RowHistoryWindows {
    recent: [RowHistory; 2],
    hints: Vec<(i64, PlacePoint)>,
}

impl RowNames {
    pub fn following() -> Self {
        Self { history: Some(RowHistoryWindows::default()), ..Self::default() }
    }

    pub fn hints(&mut self, points: impl IntoIterator<Item = PlacePoint>) {
        let Some(history) = &mut self.history else { return };
        let limit = dashboard_base::products::MAX_PRODUCT_REPLY_BYTES / std::mem::size_of::<(i64, PlacePoint)>();
        history.hints = points.into_iter().take(limit).map(|point| {
            let bucket = i64::from(point.hour()).div_euclid(BROKER_WINDOW_HOURS);
            let id = match point { PlacePoint::State(id, _) => PlacePoint::State(id, 0), PlacePoint::Province(id, _) => PlacePoint::Province(id, 0) };
            (bucket, id)
        }).collect();
        history.hints.sort_unstable();
        history.hints.dedup();
        history.hints.shrink_to_fit();
    }

    pub fn update(&mut self, source: &Source, query: &dashboard_query::Query<'_>, load: &LoadContext, points: BTreeSet<PlacePoint>, cancelled: &AtomicBool) -> io::Result<()> {
        self.held.retain(|point, _| points.contains(point));
        let needed = points.into_iter().filter(|point| !self.held.contains_key(point)).collect();
        self.held.extend(names_at_with(source, query, load, &needed, cancelled, self.history.as_mut())?);
        Ok(())
    }

    pub fn get(&self, point: &PlacePoint) -> Option<&Name> {
        self.held.get(point)
    }

    pub fn trim(&mut self) {
        let bytes = self.held.values().map(|name| name.name.as_ref().map_or(0, String::capacity) + name.missing.as_ref().map_or(0, String::capacity)).sum::<usize>();
        if bytes > dashboard_base::products::MAX_PRODUCT_REPLY_BYTES { self.held.clear(); }
    }
}

fn names_at_with(source: &Source, query: &dashboard_query::Query<'_>, load: &LoadContext, points: &BTreeSet<PlacePoint>, cancelled: &AtomicBool, mut cache: Option<&mut RowHistoryWindows>) -> io::Result<BTreeMap<PlacePoint, Name>> {
    let mut names = BTreeMap::new();
    let missing = |reason: String| Name { name: None, missing: Some(reason) };
    let mut windows = BTreeMap::<_, Vec<_>>::new();
    for &point in points {
        if (query.timeline.first..=query.timeline.last).contains(&point.hour()) {
            let first = i64::from(point.hour()).div_euclid(BROKER_WINDOW_HOURS) * BROKER_WINDOW_HOURS;
            windows.entry(first.max(i64::from(query.timeline.first)) as i32).or_default().push(point);
        } else {
            names.insert(point, missing("Place name not answered".into()));
        }
    }
    for (from, asked) in windows {
        let mut params = PlaceNameRunsParams {
            branch_id: query.timeline.id.to_string(), land: false, from,
            to: asked.iter().map(|point| point.hour()).max().expect("asked window"), states: Vec::new(), provinces: Vec::new(),
        };
        for &point in &asked {
            match point {
                PlacePoint::State(id, _) => params.states.push(id),
                PlacePoint::Province(id, _) => params.provinces.push(id),
            }
        }
        let runs = PlaceNameRuns::read(source, query, &params, load, cancelled, cache.as_deref_mut());
        for point in asked {
            check_cancel(cancelled)?;
            let hour = point.hour();
            let name = match &runs {
                Ok(reply) => {
                    if let Some(gap) = reply.unknown.iter().find(|gap| gap.first_hour <= hour && hour <= gap.last_hour) {
                        names.insert(point, missing(gap.reason.clone()));
                        continue;
                    }
                    let (id, places) = match point {
                        PlacePoint::State(id, _) => (id, &reply.states),
                        PlacePoint::Province(id, _) => (id, &reply.provinces),
                    };
                    places.binary_search_by_key(&id, |place| place.id).ok()
                        .and_then(|at| places[at].runs.iter().find(|run| run.first_hour <= hour && hour <= run.last_hour))
                        .map_or_else(|| missing("Place name not answered".into()), |run| Name { name: run.name.clone(), missing: run.missing.clone() })
                }
                Err(why) => missing(why.to_string()),
            };
            names.insert(point, name);
        }
    }
    Ok(names)
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
        Self::read(source, &query, params, load, cancelled, None)
    }
}

impl PlaceNameRuns {
    fn read(source: &Source, query: &dashboard_query::Query<'_>, params: &PlaceNameRunsParams, load: &LoadContext, cancelled: &AtomicBool, mut cache: Option<&mut RowHistoryWindows>) -> io::Result<PlaceNameRunsReply> {
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
        let scoped = ScopedText::new(query.replay, &content.cat);
        for stretch in query.timeline.stretches.iter().filter(|stretch| stretch.first <= to && stretch.last >= from) {
            let (first, last) = (stretch.first.max(from), stretch.last.min(to));
            let keys = (Keys::These(&province_keys), Keys::These(&state_keys));
            let history = if let Some(cache) = cache.as_deref_mut() {
                let bucket = i64::from(to).div_euclid(BROKER_WINDOW_HOURS);
                let end = ((bucket + 1) * BROKER_WINDOW_HOURS - 1).min(i64::from(stretch.last)) as i32;
                let window = (stretch.run, first, end);
                if cache.recent[0].bucket != Some(bucket) {
                    cache.recent.swap(0, 1);
                }
                let history = &mut cache.recent[0];
                history.select(bucket);
                if !history.holds(window) {
                    let start = cache.hints.partition_point(|(at, _)| *at < bucket);
                    let mut provinces = Vec::new();
                    let mut states = Vec::new();
                    for (_, point) in cache.hints[start..].iter().take_while(|(at, _)| *at == bucket) {
                        match *point {
                            PlacePoint::State(id, _) => states.push(u128::from(id)),
                            PlacePoint::Province(id, _) => provinces.push(u128::from(id)),
                        }
                    }
                    if !provinces.is_empty() || !states.is_empty() {
                        let keys = (Keys::These(&provinces), Keys::These(&states));
                        let _ = PlaceHistory::load_with(recording, window, keys, (false, true), cancelled,
                            |table, fields, keys| history.read(recording, window, end, (table, fields, keys)));
                    }
                }
                PlaceHistory::load_with(recording, (stretch.run, first, last), keys, (false, false), cancelled,
                    |table, fields, keys| history.read(recording, window, last, (table, fields, keys)))?
            } else {
                PlaceHistory::load(recording, stretch.run, first, last, keys.0, keys.1, false, cancelled)?
            };
            for (ids, book, is_state) in [(&states, &mut state_book, true), (&provinces, &mut province_book, false)] {
                for &id in ids {
                    let changes = if is_state { history.state_change_hours(id, first, last) } else { history.province_change_hours(id, first, last) };
                    let ends = changes.iter().skip(1).map(|hour| hour - 1).chain([last]);
                    for (mut hour, end) in changes.iter().copied().zip(ends) {
                        loop {
                            check_cancel(cancelled)?;
                            let mut validity = Validity::default();
                            let localise = |state, key: &str| {
                                key_needs_scope(&content.cat.localisation, key, &[]).then(|| {
                                    scoped.localise(stretch.run, hour, Scope::State(state), key, &[], Some(&mut validity)).and_then(|name| {
                                        if name.trim().is_empty() { Err(format!("localisation {key}: the state name is empty")) } else { Ok(name) }
                                    })
                                })
                            };
                            let reading = if is_state { history.state_reading(&content, id, hour, localise) } else { history.province_reading(&content, id, mode, hour, localise) };
                            book.settle(id, answer_of(reading), hour, &unknown);
                            if hour == end || validity.is_empty() { break; }
                            let dates = source.state::<ReadDates>();
                            let mut dates = dates.lock().unwrap_or_else(|error| error.into_inner());
                            let through = validity.through(&mut dates, query.replay, stretch.run, first, hour, last, cancelled)?;
                            if through >= end { break; }
                            hour = through + 1;
                        }
                    }
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
    fn state_names_follow_scoped_inputs_and_refuse_unrecorded_operands() {
        use replay_data::rows::{CountryAppearanceRow, CountryRow, PoliticsRow};
        let (_catalogue, folder) = dashboard_base::catalogue::tests::folder_catalogue(&[
            ("common/defines/synth.lua", "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=7},NBuildings={MAX_BUILDING_LEVELS=13}}"),
            ("map/default.map", "definitions=definition.csv provinces=provinces.bmp terrain=terrain.bmp rivers=rivers.bmp heightmap=heightmap.bmp tree_definition=trees.bmp"),
            ("map/definition.csv", concat!(
                "0;0;0;0;land;false;unknown;0\n",
                "1;10;0;0;land;false;plains;1\n2;20;0;0;land;false;plains;1\n",
                "3;30;0;0;land;false;plains;1\n4;40;0;0;land;false;plains;1\n",
            )),
            ("history/states/7.txt", "state={id=7 name=SCOPED_REACH provinces={1}}"),
            ("history/states/8.txt", "state={id=8 name=UNREAD_REACH provinces={2}}"),
            ("history/states/9.txt", "state={id=9 name=PLAIN_REACH provinces={3}}"),
            ("history/states/10.txt", "state={id=10 name=UNREAD_REACH provinces={4}}"),
            ("localisation/english/synth_l_english.yml", concat!(
                "\u{feff}l_english:\n AAA_ADJ:0 \"Alphan\"\n BBB_ADJ:0 \"Betan\"\n",
                " SCOPED_REACH:0 \"[ROOT.owner.GetAdjective] Reach\"\n",
                " UNREAD_REACH:0 \"[?temp_synth] Reach\"\n PLAIN_REACH:0 \"Plain Reach\"\n",
            )),
        ]);
        let mut bitmap = [0_u8; 58];
        bitmap[..2].copy_from_slice(b"BM");
        for (offset, value) in [(2, 58_u32), (10, 54), (14, 40), (18, 1), (22, 1)] {
            bitmap[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        bitmap[26..28].copy_from_slice(&1_u16.to_le_bytes());
        bitmap[28..30].copy_from_slice(&24_u16.to_le_bytes());
        bitmap[56] = 10;
        std::fs::write(folder.0.join("map/provinces.bmp"), bitmap).unwrap();
        let frames = (100..=104).map(|hour| {
            let mut frame = dashboard_core::test_fixtures::source(1, hour, Some(1));
            for rows in [
                TableRows::Countries(vec![CountryRow { id: 9001, canonical_slot: 7, tag: if hour < 103 { b"AAA" } else { b"BBB" }.to_vec(), ..Default::default() }]),
                TableRows::CountryAppearance(vec![CountryAppearanceRow { country: 9001, ..Default::default() }]),
                TableRows::Politics(vec![PoliticsRow { country: 9001, ..Default::default() }]),
                TableRows::States((7..=10).map(|id| StateRow {
                    id, owner: 9001, controller: 9001,
                    custom_name: if id == 10 { b"Recorded Reach".to_vec() } else { Vec::new() },
                    ..Default::default()
                }).collect()),
                TableRows::Provinces(vec![ProvinceRow { id: 1, state: 7, ..Default::default() }]),
                TableRows::ProvinceNames(vec![]),
            ] {
                let table = rows.table();
                frame.tables[table.index()] = TableView::Complete(rows).into();
            }
            frame
        }).collect();
        let (source, load) = dashboard_base::test_fixtures::source_with_content(frames, &folder.0);
        let params = PlaceNameRunsParams { branch_id: "1".into(), land: false, from: 100, to: 104, states: vec![7, 8, 9, 10, 11], provinces: vec![1] };
        let executable = dashboard_text::catalogue::tests::script_game::executable().build();
        let reply = engine_executable::executable_tables::scope(Some(executable), || PlaceNameRuns::load(&source, &params, &load, &AtomicBool::new(false))).unwrap();
        let runs = |i: usize| reply.states[i].runs.iter().map(|run| (run.first_hour, run.last_hour, run.name.as_deref())).collect::<Vec<_>>();
        assert_eq!(runs(0), [(100, 102, Some("Alphan Reach")), (103, 104, Some("Betan Reach"))], "{:?}", reply.states[0].runs);
        let unread = &reply.states[1].runs[0];
        assert!(unread.name.is_none());
        assert!(unread.missing.as_deref().is_some_and(|why| why.contains("UNREAD_REACH") && why.contains("temp_synth")), "{:?}", unread.missing);
        assert_eq!(runs(2), [(100, 104, Some("Plain Reach"))]);
        assert_eq!(runs(3), [(100, 104, Some("Recorded Reach"))]);
        assert_eq!(reply.provinces[0].runs.iter().map(|run| (run.first_hour, run.last_hour, run.name.as_deref())).collect::<Vec<_>>(), runs(0));
        let query = dashboard_base::queries::query(&source, 1).unwrap();
        let points = BTreeSet::from([
            PlacePoint::State(7, 100), PlacePoint::State(7, 103), PlacePoint::State(8, 102),
            PlacePoint::State(9, 103), PlacePoint::State(11, 100), PlacePoint::Province(1, 103),
        ]);
        let cancelled = AtomicBool::new(false);
        let actual = engine_executable::executable_tables::scope(Some(executable), || names_at(&source, &query, &load, &points, &cancelled)).unwrap();
        assert_eq!(actual.len(), points.len(), "resolve only the places and hours carried by the rows");
        for point in &points {
            let (id, hour, places) = match *point {
                PlacePoint::State(id, hour) => (id, hour, &reply.states),
                PlacePoint::Province(id, hour) => (id, hour, &reply.provinces),
            };
            let expected = places.iter().find(|place| place.id == id).unwrap().runs.iter()
                .find(|run| run.first_hour <= hour && hour <= run.last_hour).unwrap();
            assert_eq!(actual[point], Name { name: expected.name.clone(), missing: expected.missing.clone() }, "{point:?}");
        }
        let mut held = RowNames::following();
        let mut timeline = (*query.timeline).clone();
        let recorded = timeline.stretches[0];
        timeline.stretches = [(100, 100), (101, 102), (103, 104)].map(|(first, last)| {
            dashboard_query::timeline::Stretch { first, last, ..recorded }
        }).into();
        let sliced = dashboard_query::Query { timeline: std::sync::Arc::new(timeline), holders: query.holders.clone(), replay: query.replay };
        let first = BTreeSet::from([PlacePoint::State(7, 103), PlacePoint::Province(1, 103)]);
        let later = BTreeSet::from([PlacePoint::State(7, 104), PlacePoint::Province(1, 104), PlacePoint::State(9, 104)]);
        held.hints(first.iter().chain(&later).copied());
        engine_executable::executable_tables::scope(Some(executable), || {
            held.update(&source, &sliced, &load, first, &cancelled).unwrap();
            let reads = held.history.as_ref().unwrap().recent[0].reads;
            assert!(reads > 0);
            held.update(&source, &sliced, &load, later.clone(), &cancelled).unwrap();
            assert_eq!(held.history.as_ref().unwrap().recent[0].reads, reads, "a newly shown place reuses all three stretches of the window");
            let expected = names_at(&source, &sliced, &load, &later, &cancelled).unwrap();
            for point in later { assert_eq!(held.get(&point), expected.get(&point)); }
        });
        let no_content = LoadContext { content: None };
        let reason = PlaceNameRuns::load(&source, &params, &no_content, &cancelled).unwrap_err().to_string();
        let withheld = names_at(&source, &query, &no_content, &points, &cancelled).unwrap();
        assert!(withheld.values().all(|name| name.name.is_none() && name.missing.as_deref() == Some(reason.as_str())));

        let unrecorded = (100..=104).map(|hour| {
            let mut frame = dashboard_core::test_fixtures::source(1, hour, Some(1));
            *frame.tables[TableId::States.index()] = TableView::NotRecorded { mode: replay_data::scope::CaptureMode::Full };
            *frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(vec![ProvinceRow { id: 1, state: 7, ..Default::default() }]));
            *frame.tables[TableId::ProvinceNames.index()] = TableView::Complete(TableRows::ProvinceNames(vec![]));
            frame
        }).collect();
        let (source, load) = dashboard_base::test_fixtures::source_with_content(unrecorded, &folder.0);
        let query = dashboard_base::queries::query(&source, 1).unwrap();
        let params = PlaceNameRunsParams { states: Vec::new(), ..params };
        let reply = PlaceNameRuns::load(&source, &params, &load, &cancelled).unwrap();
        let wanted = PlacePoint::Province(1, 103);
        let points = names_at(&source, &query, &load, &BTreeSet::from([wanted]), &cancelled).unwrap();
        let missing = &reply.provinces[0].runs[0].missing;
        assert!(missing.as_deref().is_some_and(|reason| reason.contains("hour 100")), "{missing:?}");
        assert_eq!(&points[&wanted].missing, missing, "an interior point keeps the run's exact refusal reason");
        let mut followed = RowNames::following();
        followed.hints([wanted]);
        followed.update(&source, &query, &load, BTreeSet::from([wanted]), &cancelled).unwrap();
        assert_eq!(followed.get(&wanted), Some(&points[&wanted]));
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
        let context_free = |_, _: &str| None;
        assert_eq!(answer_of(history.state_reading(&content, 7, 1_000, context_free)).presence, Presence::Present);
        assert_eq!(answer_of(history.state_reading(&content, 8, 1_000, context_free)).presence, Presence::Absent, "a complete read without the row proves the state does not exist");
        assert_eq!(answer_of(history.state_reading(&content, 7, 1_001, context_free)).presence, Presence::Present, "a witnessed row is present even in a truncated read");
        assert_eq!(answer_of(history.state_reading(&content, 8, 1_001, context_free)).presence, Presence::Unknown, "a truncated read without the row proves nothing");
        assert_eq!(answer_of(history.province_reading(&content, 3, ProvinceMode::Place, 1_002, context_free)).presence, Presence::Unknown);
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
