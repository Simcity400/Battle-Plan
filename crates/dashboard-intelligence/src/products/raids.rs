
use dashboard_base::products::Country;
use dashboard_base::{Source, products::{LoadContext, Product}, queries::country_ids::Names};
use dashboard_base::catalogue::{self, CachedResult, Catalogue, View};
use dashboard_query::{Query, log, read, series::MissingSpan};
use engine_native_air::craid_instance as native;
use content_reader_raids::raids::Catalog;
use replay_lanes::lanes::{self, Cell, Keys};
use replay_data::rows::{RaidEndRow, RaidScope, TableId, TagHolderRow};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, Mutex, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const PARENT_SLOT: usize = 0;
const PHASE: usize = 2;
const OUTCOME: usize = 3;
const TRAVELLED: usize = 16;
const PREPARATION: usize = 17;
const RAID_COLUMNS: [usize; 5] = [PARENT_SLOT, PHASE, OUTCOME, TRAVELLED, PREPARATION];
const TARGET_COLUMNS: [usize; 10] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
const RAID_TYPE: usize = 9;
const UNKNOWN_ICON: &str = "GFX_raid_type_icon_unknown";
const ENDED_VICTIMS: &str = "Ended victims use the target controller at each raid's first ended read. The saved completion-time country is not recorded, so control changes before that read are unknown.";

fn lane_value(cell: Option<Cell>) -> Option<Option<i64>> {
    match cell? {
        Cell::Number(value) => Some(Some(value)),
        Cell::Missing => Some(None),
        _ => None,
    }
}

fn log_value(cell: Option<&log::Cell>) -> Option<Option<i64>> {
    match cell? {
        log::Cell::Number(value) => Some(Some(*value)),
        log::Cell::Missing => Some(None),
        _ => None,
    }
}

fn scope_of(value: impl Fn(usize) -> Option<Option<i64>>) -> Option<RaidScope> {
    let number = |at: usize| value(at).flatten();
    Some(RaidScope {
        province: number(0)? as u32,
        state: number(1)? as u32,
        leader_reference: number(2)? as u64,
        leader_province: number(3)? as u32,
        building_type: number(4)? as u32,
        building_location: number(5)? as u32,
        building_flags: number(6)? as u32,
        building_country: number(7)? as i32,
        leader_country: value(8)?.map(|captor| captor as i32),
    })
}

pub fn province_of(scope: &RaidScope) -> Option<u32> {
    if scope.province != 0 {
        Some(scope.province)
    } else if scope.leader_province != 0 {
        Some(scope.leader_province)
    } else if scope.building_type != 0 && scope.building_flags & 1 != 0 {
        Some(scope.building_location)
    } else {
        None
    }
}

fn finite(raw: i64) -> bool {
    (raw as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Course {
    pub runs: Vec<(i32, i32, i32, i32)>,
    pub preparation: Vec<RaidPreparationRun>,
    pub launched: Option<i32>,
    pub ended: Option<i32>,
    pub raider: Option<u32>,
    pub phase: Option<i32>,
    pub outcome: Option<i32>,
}

impl Course {
    pub fn read(steps: &[lanes::Step], last: i32) -> Self {
        let mut course = Self::default();
        for (at, step) in steps.iter().enumerate() {
            if step.hour > last {
                break;
            }
            let through = steps.get(at + 1).map_or(last, |next| (next.hour - 1).min(last));
            let Some(cells) = step.cells.as_deref() else { continue };
            let number = |column: usize| match cells.get(RAID_COLUMNS.iter().position(|&c| c == column)?) {
                Some(Cell::Number(value)) => Some(*value),
                _ => None,
            };
            let (Some(phase), Some(outcome)) = (number(PHASE), number(OUTCOME)) else { continue };
            let (phase, outcome) = (phase as i32, outcome as i32);
            course.raider = number(PARENT_SLOT).map(|slot| slot as u32).or(course.raider);
            course.phase = Some(phase);
            course.outcome = Some(outcome);
            if course.launched.is_none() && (phase == native::PHASE_IN_PROGRESS || number(TRAVELLED).is_some_and(|distance| distance != 0 && finite(distance))) {
                course.launched = Some(step.hour);
            }
            if course.ended.is_none() && phase == native::PHASE_ENDED {
                course.ended = Some(step.hour);
            }
            match course.runs.last_mut() {
                Some(run) if run.1 + 1 == step.hour && (run.2, run.3) == (phase, outcome) => run.1 = through,
                _ => course.runs.push((step.hour, through, phase, outcome)),
            }
            if phase == native::PHASE_PREPARING
                && let Some(hours) = number(PREPARATION)
            {
                prepare(&mut course.preparation, step.hour, through, hours as i32);
            }
        }
        course
    }

    pub fn succeeded(&self) -> bool {
        self.ended.is_some() && matches!(self.outcome, Some(native::OUTCOME_SUCCESS | native::OUTCOME_CRITICAL_SUCCESS))
    }
}

fn prepare(runs: &mut Vec<RaidPreparationRun>, first: i32, last: i32, value: i32) {
    if let Some(run) = runs.last_mut()
        && run.1 + 1 == first
    {
        let next = i64::from(run.2) + i64::from(first - run.0) * run.3;
        if run.0 == run.1 && first == last {
            run.3 = i64::from(value) - i64::from(run.2);
            run.1 = last;
            return;
        }
        if next == i64::from(value) && (first == last || run.3 == 0) {
            run.1 = last;
            return;
        }
        if run.0 == run.1 && value == run.2 {
            run.3 = 0;
            run.1 = last;
            return;
        }
    }
    runs.push(RaidPreparationRun(first, last, value, 0));
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RaidTypeInfo {
    pub token: String,
    pub name: Option<String>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RaidArtwork {
    pub key: String,
    pub image: Option<String>,
    pub missing: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum RaidVictimSource {
    FixedTarget,
    ControllerAtFirstEndedRead,
    LastObservedController,
}

pub fn victim_source(scope: &RaidScope, ended: bool) -> RaidVictimSource {
    let fixed = if scope.building_type != 0 {
        scope.building_country > 0
    } else if scope.province != 0 || scope.state != 0 {
        false
    } else {
        scope.leader_country.is_some()
    };
    if fixed {
        RaidVictimSource::FixedTarget
    } else if ended {
        RaidVictimSource::ControllerAtFirstEndedRead
    } else {
        RaidVictimSource::LastObservedController
    }
}

pub fn phase_label(phase: i32, label: &mut dyn FnMut(&'static str) -> Option<String>) -> Option<String> {
    match native::known_phase_key(phase) {
        Some(key) => label(key),
        None => Some(format!("Unsupported phase ({phase})")),
    }
}

pub fn outcome_label(outcome: i32, label: &mut dyn FnMut(&'static str) -> Option<String>) -> Option<String> {
    match native::known_outcome_key(outcome) {
        Some(key) => label(key),
        None => Some(format!("Unsupported outcome ({outcome})")),
    }
}

pub fn outcome_missing(ended: bool, disappeared: bool, outcome: Option<i32>) -> Option<String> {
    if ended {
        match outcome {
            Some(code) if (native::OUTCOME_FAILURE..=native::OUTCOME_CANCELED).contains(&code) => None,
            Some(code) if code != native::OUTCOME_NONE => Some(format!("Unsupported raid outcome ({code})")),
            _ => Some("raid outcome was not recorded".to_owned()),
        }
    } else if disappeared {
        Some("raid outcome was not recorded".to_owned())
    } else {
        None
    }
}

pub fn coalesce_spans(mut spans: Vec<(i32, i32)>) -> Vec<(i32, i32)> {
    spans.sort_unstable();
    let mut merged: Vec<(i32, i32)> = Vec::with_capacity(spans.len());
    for (first, last) in spans {
        if first > last {
            continue;
        }
        match merged.last_mut() {
            Some(prev) if first <= prev.1.saturating_add(1) => prev.1 = prev.1.max(last),
            _ => merged.push((first, last)),
        }
    }
    merged
}

pub fn in_spans(spans: &[(i32, i32)], hour: i32) -> bool {
    let at = spans.partition_point(|span| span.1 < hour);
    spans.get(at).is_some_and(|span| span.0 <= hour && hour <= span.1)
}

pub fn prev_in_spans(spans: &[(i32, i32)], before: i32) -> Option<i32> {
    let at = spans.partition_point(|span| span.0 < before);
    let span = spans.get(at.checked_sub(1)?)?;
    Some(span.1.min(before - 1))
}

pub fn next_in_spans(spans: &[(i32, i32)], after: i32) -> Option<i32> {
    let at = spans.partition_point(|span| span.1 <= after);
    spans.get(at).map(|span| span.0.max(after + 1))
}

pub fn date_bounds(runs: &[(i32, i32, i32, i32)], raw_first: i32, complete: &[(i32, i32)], available: &[(i32, i32)]) -> Vec<(bool, Option<i32>)> {
    runs.iter()
        .enumerate()
        .map(|(index, &(first, _, _, _))| {
            if index == 0 {
                if first == raw_first {
                    let after = prev_in_spans(complete, first);
                    let exact = after == Some(first - 1) && in_spans(available, first);
                    (exact, after)
                } else {
                    (false, None)
                }
            } else {
                let prev = runs[index - 1];
                let after = prev_in_spans(available, first);
                let exact = prev.1 + 1 == first && after == Some(first - 1) && in_spans(available, first);
                (exact, after)
            }
        })
        .collect()
}

pub fn exact_at(hour: i32, first: i32, first_exact: bool, runs: &[(i32, i32, i32, i32)], bounds: &[(bool, Option<i32>)], available: &[(i32, i32)]) -> bool {
    if let Some(index) = runs.iter().position(|run| run.0 == hour) {
        return bounds[index].0;
    }
    if hour == first {
        first_exact
    } else {
        in_spans(available, hour - 1) && in_spans(available, hour)
    }
}

pub fn disappearance(ended: bool, last: i32, deleted_hour: Option<i32>, complete: &[(i32, i32)], available: &[(i32, i32)]) -> (Option<i32>, bool) {
    if ended {
        return (None, false);
    }
    let next_complete = next_in_spans(complete, last);
    let at = match (deleted_hour, next_complete) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    let exact = at == Some(last + 1) && in_spans(available, last) && in_spans(complete, last + 1);
    (at, exact)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RaidPhaseRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub first_exact: bool,
    pub after_hour: Option<i32>,
    pub phase: i32,
    pub phase_name: Option<String>,
    pub outcome: i32,
    pub outcome_name: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
pub struct RaidPreparationRun(pub i32, pub i32, pub i32, #[cfg_attr(any(test, feature = "export-types"), ts(type = "number"))] pub i64);

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RaidEpisode {
    #[doc = " The raid's row key, as a decimal string."]
    pub id: String,
    #[doc = " The first and last hour its row lived within the span."]
    pub first_hour: i32,
    pub first_hour_exact: bool,
    pub last_hour: i32,
    pub raider: Option<String>,
    pub victim: Option<String>,
    #[doc = " Why the victim is not named, when it is not."]
    pub victim_missing: Option<String>,
    pub victim_source: Option<RaidVictimSource>,
    #[doc = " The raid type's token and its name in the reader's language."]
    pub raid_type: Option<String>,
    pub raid_type_name: Option<String>,
    #[doc = " The sprite key into the reply's artwork, or why there is none."]
    pub icon: Option<String>,
    pub icon_missing: Option<String>,
    pub province: Option<u32>,
    pub state: Option<u32>,
    #[doc = " Its last phase and outcome."]
    pub phase: Option<i32>,
    pub phase_name: Option<String>,
    pub outcome: Option<i32>,
    pub outcome_name: Option<String>,
    pub outcome_missing: Option<String>,
    #[doc = " Its phases over the span, in hour order."]
    pub runs: Vec<RaidPhaseRun>,
    pub preparation: Vec<RaidPreparationRun>,
    pub launched_hour: Option<i32>,
    pub launched_exact: bool,
    pub launched: bool,
    pub ended_hour: Option<i32>,
    pub ended_exact: bool,
    pub disappeared_hour: Option<i32>,
    pub disappeared_exact: bool,
    pub succeeded: bool,
}

struct Content {
    catalogue: Result<Arc<Catalogue>, String>,
    raids: Result<Arc<Catalog>, String>,
}

type Drawn = Mutex<HashMap<(String, Option<String>), Result<(String, String), String>>>;

impl Content {
    fn load(source: &Source, context: &LoadContext) -> Self {
        let catalogue = context.content().and_then(|options| catalogue::catalogue(source, options)).map_err(|error| error.to_string());
        let raids = catalogue.as_ref().map_err(Clone::clone).and_then(|cat| {
            cat.content.layers.derived::<CachedResult<Catalog>>().get_or_init(|| content_reader_raids::raids::read(&cat.content.layers)).map_err(|error| error.to_string())
        });
        Self { catalogue, raids }
    }

    fn label(&self, key: &str) -> Option<String> {
        let cat = self.catalogue.as_ref().ok()?;
        cat.localisation.get(key)?;
        cat.localised(key, View::Plain)
    }

    fn types(&self) -> Vec<RaidTypeInfo> {
        let Ok(raids) = &self.raids else { return Vec::new() };
        raids
            .types
            .iter()
            .map(|kind| RaidTypeInfo { token: kind.name.clone(), name: self.label(&format!("raid_type_{}", kind.name)), missing: None })
            .chain(raids.uncertain.iter().map(|kind| RaidTypeInfo { token: kind.name.clone(), name: None, missing: Some(kind.reason.clone()) }))
            .collect()
    }

    fn warnings(&self) -> Vec<String> {
        let mut out: Vec<String> = match &self.catalogue {
            Ok(cat) => cat.verification.iter().chain(&cat.warnings).cloned().collect(),
            Err(reason) => vec![format!("Raid names and icons are unavailable: {reason}")],
        };
        match &self.raids {
            Ok(raids) => out.extend(raids.misreads.iter().map(|(_, text)| text.clone())),
            Err(reason) if self.catalogue.is_ok() => out.push(format!("Raid types are unavailable: {reason}")),
            Err(_) => {}
        }
        out
    }

    fn icon(&self, token: &str, template: Option<&str>) -> Result<(String, String), String> {
        let cat = self.catalogue.as_ref().map_err(|reason| format!("raid icons are unavailable: {reason}"))?;
        let raids = self.raids.as_ref().map_err(|reason| format!("raid icons are unavailable: {reason}"))?;
        let kind = raids.get(token).ok_or_else(|| format!("raid type {token} is unavailable or refused"))?;
        let drawn = cat.content.layers.derived::<Drawn>();
        let held = (token.to_owned(), template.map(str::to_owned));
        if let Some(found) = drawn.lock().unwrap_or_else(|error| error.into_inner()).get(&held) {
            return found.clone();
        }
        let refused = std::cell::RefCell::new(None);
        let key = kind
            .icon(template, |key| match cat.sprites.find(key) {
                Ok(sprite) => sprite.is_some(),
                Err(reason) => {
                    *refused.borrow_mut() = Some(format!("raid sprite {key}: {reason}"));
                    false
                }
            })
            .unwrap_or_else(|| UNKNOWN_ICON.into());
        let made = match refused.into_inner() {
            Some(reason) => Err(reason),
            None => cat
                .sprites
                .find(&key)
                .and_then(|sprite| sprite.ok_or_else(|| format!("raid sprite {key} is unavailable")))
                .and_then(|sprite| {
                    let decode = cat.gui_decode.clone()?;
                    catalogue::sprite_frame_png(&cat.content.layers, sprite, decode, 0, 48).map_err(|error| format!("raid sprite {key}: {error}"))
                })
                .map(|image| (key, image)),
        };
        drawn.lock().unwrap_or_else(|error| error.into_inner()).insert(held, made.clone());
        made
    }
}

fn literals(query: &Query<'_>, run: u32, hour: i32, tokens: &BTreeSet<u32>) -> io::Result<HashMap<u32, String>> {
    if tokens.is_empty() {
        return Ok(HashMap::new());
    }
    let keys: Vec<u128> = tokens.iter().map(|&token| u128::from(token)).collect();
    let snapshots = read::at(query.replay.recording(), run, &[read::Read { table: TableId::LexicalTokens, columns: None, keys: read::Keys::These(&keys) }], hour)?;
    let Some(snapshot) = snapshots.first() else { return Ok(HashMap::new()) };
    let view = snapshot.view();
    Ok(snapshot
        .rows
        .iter()
        .filter_map(|&(key, slot)| Some((key as u32, String::from_utf8_lossy(read::text(&view, 0, slot)?).into_owned())))
        .filter(|(_, literal)| !literal.is_empty())
        .collect())
}

fn cell_at(history: &lanes::History, steps: Option<&Vec<lanes::Step>>, column: usize, hour: i32) -> Option<Cell> {
    let at = history.columns.binary_search(&column).ok()?;
    let steps = steps?;
    let step = &steps[steps.partition_point(|step| step.hour <= hour).checked_sub(1)?];
    step.cells.as_ref().map(|cells| cells[at])
}

fn steps_of(history: &lanes::History, key: u128) -> Option<&Vec<lanes::Step>> {
    history.keys.binary_search_by_key(&key, |(key, _)| *key).ok().map(|at| &history.keys[at].1)
}

struct Facts {
    scope: Option<RaidScope>,
    token: Option<u32>,
    state: Option<u32>,
    victim: (Option<String>, Option<String>),
}

struct Accumulated {
    key: u128,
    raw_first: i32,
    first_hour: i32,
    last_hour: i32,
    runs: Vec<(i32, i32, i32, i32)>,
    preparation: Vec<RaidPreparationRun>,
    raider: Option<String>,
    victim: Option<String>,
    victim_missing: Option<String>,
    victim_source: Option<RaidVictimSource>,
    victim_locked: bool,
    raid_type: Option<String>,
    raid_type_name: Option<String>,
    icon: Option<String>,
    icon_missing: Option<String>,
    province: Option<u32>,
    state: Option<u32>,
    phase: Option<i32>,
    outcome: Option<i32>,
    launched_hour: Option<i32>,
    ended_hour: Option<i32>,
    deleted_hour: Option<i32>,
}

fn min_opt(a: Option<i32>, b: Option<i32>) -> Option<i32> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (x, y) => x.or(y),
    }
}

fn merge_preparation(target: &mut Vec<RaidPreparationRun>, incoming: Vec<RaidPreparationRun>) {
    for run in incoming {
        if let Some(prev) = target.last_mut()
            && prev.1 + 1 == run.0
            && prev.3 == run.3
            && i64::from(prev.2) + i64::from(run.0 - prev.0) * prev.3 == i64::from(run.2)
        {
            prev.1 = run.1;
            continue;
        }
        target.push(run);
    }
}

fn episodes(source: &Source, query: &Query<'_>, from: i32, to: i32, content: &Content, warnings: &mut BTreeSet<String>) -> io::Result<(Vec<RaidEpisode>, BTreeMap<String, RaidArtwork>)> {
    let recording = query.replay.recording();
    let countries = Names::new(&query.holders);
    let end_log = dashboard_declare::log("raid-ends").ok_or_else(|| dashboard_core::invalid("No declared log raid-ends"))?;
    let reader = log::Reader::new(query, end_log)?;
    let ends = EndColumns::new()?;
    let province_columns = [dashboard_declare::field(TableId::Provinces, "state")?, dashboard_declare::field(TableId::Provinces, "controller")?];
    let state_controller = dashboard_declare::field(TableId::States, "controller")?;
    let mut accumulated: BTreeMap<u128, Accumulated> = BTreeMap::new();
    let mut complete_spans: Vec<(i32, i32)> = Vec::new();
    let mut available_spans: Vec<(i32, i32)> = Vec::new();
    let mut artwork = BTreeMap::new();
    let mut undrawn: BTreeMap<String, u64> = BTreeMap::new();
    let mut labels: HashMap<&'static str, Option<String>> = HashMap::new();
    let mut ended_victims = false;
    for stretch in query.timeline.within(from, to) {
        let run = stretch.run;
        let end = stretch.last.min(to);
        complete_spans.extend(dashboard_query::cost::storage(|| lanes::complete_reads(recording, run, TableId::Raids, stretch.first, end))?);
        available_spans.extend(dashboard_query::cost::storage(|| lanes::available_reads(recording, run, TableId::Raids, stretch.first, end))?);
        let lives = dashboard_base::queries::lifetimes(source, run, TableId::Raids)?;
        let met: Vec<(u128, i32, i32, i32)> = lives
            .iter()
            .filter_map(|&(key, raw_first, raw_last)| {
                let raw_first = i32::try_from(raw_first).ok()?.max(stretch.first);
                let first = raw_first.max(from);
                let last = i32::try_from(raw_last).ok()?.min(end);
                (first <= last).then_some((key, raw_first, first, last))
            })
            .collect();
        if met.is_empty() {
            continue;
        }
        let links = dashboard_base::queries::links(source, run)?;
        let keys: Vec<u128> = met.iter().map(|(key, ..)| *key).collect::<BTreeSet<_>>().into_iter().collect();
        let start = met.iter().map(|(_, _, first, _)| *first).min().unwrap_or(stretch.first);
        let raids = dashboard_query::cost::storage(|| lanes::history(recording, run, TableId::Raids, &RAID_COLUMNS, Keys::These(&keys), start, end))?;
        let mut ended: HashMap<u128, log::Row> = HashMap::new();
        for row in reader.inserted(run, &[(start, end)])? {
            ended.entry(row.key & u128::from(u64::MAX)).or_insert(row);
        }
        let open: Vec<u128> = keys.iter().copied().filter(|key| !ended.contains_key(key)).collect();
        let targets = if open.is_empty() {
            lanes::History::default()
        } else {
            dashboard_query::cost::storage(|| lanes::history(recording, run, TableId::RaidTargets, &TARGET_COLUMNS, Keys::These(&open), start, end))?
        };
        let open_at: HashMap<u128, i32> = met.iter().filter(|(key, ..)| !ended.contains_key(key)).map(|&(key, _, _, last)| (key, last)).collect();
        let open_scopes: HashMap<u128, Option<RaidScope>> =
            open_at.iter().map(|(&key, &at)| (key, scope_of(|column| lane_value(cell_at(&targets, steps_of(&targets, key), column, at))))).collect();
        let (mut provinces, mut states) = (BTreeSet::new(), BTreeSet::new());
        for scope in open_scopes.values().flatten() {
            for id in [scope.province, scope.leader_province, scope.building_location] {
                if id != 0 {
                    provinces.insert(u128::from(id));
                }
            }
            for id in [scope.state, scope.building_location] {
                if id != 0 {
                    states.insert(u128::from(id));
                }
            }
        }
        let (first_open, last_open) = (open_at.values().min().copied().unwrap_or(end), open_at.values().max().copied().unwrap_or(end));
        let places = |table: TableId, columns: &[usize], ids: &BTreeSet<u128>| -> io::Result<lanes::History> {
            if ids.is_empty() {
                return Ok(lanes::History::default());
            }
            let ids: Vec<u128> = ids.iter().copied().collect();
            dashboard_query::cost::storage(|| lanes::history(recording, run, table, columns, Keys::These(&ids), first_open, last_open))
        };
        let province_history = places(TableId::Provinces, &province_columns, &provinces)?;
        let state_history = places(TableId::States, &[state_controller], &states)?;
        let place = |history: &lanes::History, column: usize, id: u32, hour: i32| -> Result<u32, ()> {
            match cell_at(history, steps_of(history, u128::from(id)), column, hour) {
                Some(Cell::Number(value)) => Ok(value as u32),
                _ => Err(()),
            }
        };
        let tag = |raw: u32, hour: i32| countries.tag_at(&links, raw, hour).map(str::to_owned);
        let named = |tag: Option<String>| match tag {
            Some(tag) => (Some(tag), None),
            None => (None, Some("The victim's country holds no tag at the raid's hour".to_owned())),
        };
        let mut facts: HashMap<u128, Facts> = HashMap::new();
        for (&key, row) in &ended {
            let value = |column: usize| log_value(row.cells.get(column));
            let scope = scope_of(|at| value(ends.target[at]));
            let why = value(ends.victim_unknown).flatten().map(|why| why as u8);
            let victim = match (why, value(ends.victim)) {
                (Some(RaidEndRow::TARGET_UNKNOWN), _) => (None, Some("The raid's target is not recorded".to_owned())),
                (Some(RaidEndRow::LOCATION_UNKNOWN), _) => (None, Some("The targeted province's or state's controller is not recorded".to_owned())),
                (_, Some(None)) => (None, Some("The raid's target names no country".to_owned())),
                (_, Some(Some(raw))) => match value(ends.victim_slot).flatten() {
                    Some(slot) if slot as u32 != TagHolderRow::NONE => named(countries.slot_tag(slot as u32, row.hour).map(str::to_owned).or_else(|| tag(raw as u32, row.hour))),
                    _ => named(tag(raw as u32, row.hour)),
                },
                (_, None) => (None, Some("The raid's target country is not recorded".to_owned())),
            };
            let state = value(ends.target_state).flatten().map(|state| state as u32);
            let token = value(ends.target[RAID_TYPE]).flatten().filter(|&token| token != 0).map(|token| token as u32);
            facts.insert(key, Facts { scope, token, state, victim });
        }
        for (&key, scope) in &open_scopes {
            let at = open_at[&key];
            let token = lane_value(cell_at(&targets, steps_of(&targets, key), RAID_TYPE, at)).flatten().filter(|&token| token != 0).map(|token| token as u32);
            let state = scope.as_ref().and_then(|scope| scope.state_with(|id| place(&province_history, province_columns[0], id, at)).ok().flatten());
            let victim = match scope {
                None => (None, Some("The raid's target is not recorded".to_owned())),
                Some(scope) => match scope.country_with(|id| place(&province_history, province_columns[1], id, at), |id| place(&state_history, state_controller, id, at)) {
                    Err(()) => (None, Some("The targeted province's or state's controller is not recorded".to_owned())),
                    Ok(None) => (None, Some("The raid's target names no country".to_owned())),
                    Ok(Some(raw)) => named(tag(raw, at)),
                },
            };
            facts.insert(key, Facts { scope: scope.clone(), token, state, victim });
        }
        let tokens: BTreeSet<u32> = facts.values().flat_map(|facts| [facts.token, facts.scope.as_ref().map(|scope| scope.building_type).filter(|&token| token != 0)]).flatten().collect();
        let literals = literals(query, run, end, &tokens)?;
        for &(key, raw_first, first, last) in &met {
            let course = Course::read(steps_of(&raids, key).map_or(&[][..], Vec::as_slice), last);
            let row = ended.get(&key);
            let end_number = |column: usize| log_value(row.and_then(|row| row.cells.get(column))).flatten();
            let end_kind = end_number(ends.kind).map(|kind| kind as u8);
            let raider_slot = end_number(ends.raider).map(|slot| slot as u32).or(course.raider);
            let raider_hour = row.map_or(last, |row| row.hour);
            let raider = raider_slot.and_then(|slot| countries.slot_tag(slot, raider_hour).map(str::to_owned));
            let phase = end_number(ends.phase).map(|phase| phase as i32).or(course.phase);
            let outcome = end_number(ends.outcome).map(|outcome| outcome as i32).or(course.outcome);
            let launched_hour = match end_number(ends.launched_hour) {
                Some(hour) => Some(course.launched.map_or(hour as i32, |launched| launched.min(hour as i32))),
                None => course.launched,
            };
            let ended_hour = match row {
                Some(row) if end_kind == Some(RaidEndRow::ENDED) => Some(course.ended.map_or(row.hour, |hour| hour.min(row.hour))),
                _ => course.ended,
            };
            let deleted_hour = row.filter(|_| end_kind == Some(RaidEndRow::DELETED)).map(|row| row.hour);
            let held = facts.get(&key);
            let literal = held.and_then(|facts| facts.token).and_then(|token| literals.get(&token).cloned());
            let template = held.and_then(|facts| facts.scope.as_ref()).filter(|scope| scope.building_type != 0).map(|scope| literals.get(&scope.building_type).cloned().ok_or(scope.building_type));
            let (icon, icon_missing) = match (&literal, template) {
                (None, _) => (None, Some("The raid's type is not recorded".to_owned())),
                (Some(_), Some(Err(token))) => (None, Some(format!("raid target building token {token} is not recorded"))),
                (Some(literal), template) => match content.icon(literal, template.and_then(Result::ok).as_deref()) {
                    Ok((key, image)) => {
                        artwork.entry(key.clone()).or_insert_with(|| RaidArtwork { key: key.clone(), image: Some(image), missing: None });
                        (Some(key), None)
                    }
                    Err(reason) => {
                        *undrawn.entry(reason.clone()).or_default() += 1;
                        (None, Some(reason))
                    }
                },
            };
            let stretch_victim = held.and_then(|facts| facts.victim.0.clone());
            let stretch_victim_missing = held.map_or_else(|| Some("The raid's target is not recorded".to_owned()), |facts| facts.victim.1.clone());
            let stretch_victim_source = stretch_victim.as_ref().and_then(|_| held.and_then(|facts| facts.scope.as_ref()).map(|scope| victim_source(scope, ended_hour.is_some())));
            let stretch_raid_type_name = literal.as_ref().and_then(|literal| content.label(&format!("raid_type_{literal}")));
            let stretch_province = held.and_then(|facts| facts.scope.as_ref().and_then(province_of));
            let stretch_state = held.and_then(|facts| facts.state);
            let entry = accumulated.entry(key).or_insert_with(|| Accumulated {
                key,
                raw_first,
                first_hour: first,
                last_hour: last,
                runs: Vec::new(),
                preparation: Vec::new(),
                raider: None,
                victim: None,
                victim_missing: Some("The raid's target is not recorded".to_owned()),
                victim_source: None,
                victim_locked: false,
                raid_type: None,
                raid_type_name: None,
                icon: None,
                icon_missing: None,
                province: None,
                state: None,
                phase: None,
                outcome: None,
                launched_hour: None,
                ended_hour: None,
                deleted_hour: None,
            });
            entry.raw_first = entry.raw_first.min(raw_first);
            entry.first_hour = entry.first_hour.min(first);
            entry.last_hour = entry.last_hour.max(last);
            for (rf, rl, p, o) in course.runs {
                match entry.runs.last_mut() {
                    Some(prev) if prev.1 + 1 == rf && (prev.2, prev.3) == (p, o) => prev.1 = prev.1.max(rl),
                    _ => entry.runs.push((rf, rl, p, o)),
                }
            }
            merge_preparation(&mut entry.preparation, course.preparation);
            if entry.raider.is_none() {
                entry.raider = raider;
            }
            entry.launched_hour = min_opt(entry.launched_hour, launched_hour);
            entry.ended_hour = min_opt(entry.ended_hour, ended_hour);
            entry.deleted_hour = min_opt(entry.deleted_hour, deleted_hour);
            if !entry.victim_locked {
                entry.phase = phase.or(entry.phase);
                entry.outcome = outcome.or(entry.outcome);
                entry.victim = stretch_victim;
                entry.victim_missing = stretch_victim_missing;
                entry.victim_source = stretch_victim_source;
                entry.raid_type = literal.or_else(|| entry.raid_type.take());
                entry.raid_type_name = stretch_raid_type_name.or_else(|| entry.raid_type_name.take());
                entry.icon = icon.or_else(|| entry.icon.take());
                entry.icon_missing = if entry.icon.is_some() { None } else { icon_missing.or_else(|| entry.icon_missing.take()) };
                entry.province = stretch_province.or(entry.province);
                entry.state = stretch_state.or(entry.state);
                if ended_hour.is_some() {
                    entry.victim_locked = true;
                }
            }
        }
    }
    let complete = coalesce_spans(complete_spans);
    let available = coalesce_spans(available_spans);
    let mut out = Vec::with_capacity(accumulated.len());
    let mut keyed: Vec<(i32, u128, RaidEpisode)> = Vec::with_capacity(accumulated.len());
    for acc in accumulated.into_values() {
        if acc.raider.is_none() {
            warnings.insert("Some raids' raiders are not named: their country holds no tag at the raid's hour.".into());
        }
        if acc.victim_source == Some(RaidVictimSource::ControllerAtFirstEndedRead) {
            ended_victims = true;
        }
        let bounds = date_bounds(&acc.runs, acc.raw_first, &complete, &available);
        let first_hour_exact = bounds.first().map_or_else(
            || acc.first_hour == acc.raw_first && prev_in_spans(&complete, acc.first_hour) == Some(acc.first_hour - 1) && in_spans(&available, acc.first_hour),
            |&(exact, _)| exact,
        );
        let launched_exact = acc.launched_hour.is_some_and(|hour| exact_at(hour, acc.first_hour, first_hour_exact, &acc.runs, &bounds, &available));
        let ended_exact = acc.ended_hour.is_some_and(|hour| exact_at(hour, acc.first_hour, first_hour_exact, &acc.runs, &bounds, &available));
        let (disappeared_hour, disappeared_exact) = disappearance(acc.ended_hour.is_some(), acc.last_hour, acc.deleted_hour, &complete, &available);
        let outcome_missing = outcome_missing(acc.ended_hour.is_some(), disappeared_hour.is_some(), acc.outcome);
        let succeeded = acc.ended_hour.is_some() && matches!(acc.outcome, Some(native::OUTCOME_SUCCESS | native::OUTCOME_CRITICAL_SUCCESS));
        let mut label = |key: &'static str| labels.entry(key).or_insert_with(|| content.label(key)).clone();
        let runs = acc
            .runs
            .iter()
            .zip(bounds)
            .map(|(&(first_hour, last_hour, phase, outcome), (first_exact, after_hour))| RaidPhaseRun {
                first_hour,
                last_hour,
                first_exact,
                after_hour,
                phase,
                phase_name: phase_label(phase, &mut label),
                outcome,
                outcome_name: outcome_label(outcome, &mut label),
            })
            .collect();
        let episode = RaidEpisode {
            id: acc.key.to_string(),
            first_hour: acc.first_hour,
            first_hour_exact,
            last_hour: acc.last_hour,
            raider: acc.raider,
            victim: acc.victim,
            victim_missing: acc.victim_missing,
            victim_source: acc.victim_source,
            raid_type: acc.raid_type,
            raid_type_name: acc.raid_type_name,
            icon: acc.icon,
            icon_missing: acc.icon_missing,
            province: acc.province,
            state: acc.state,
            phase_name: acc.phase.and_then(|phase| phase_label(phase, &mut label)),
            outcome_name: acc.outcome.and_then(|outcome| outcome_label(outcome, &mut label)),
            outcome_missing,
            phase: acc.phase,
            outcome: acc.outcome,
            runs,
            preparation: acc.preparation,
            launched: acc.launched_hour.is_some(),
            launched_hour: acc.launched_hour,
            launched_exact,
            ended_hour: acc.ended_hour,
            ended_exact,
            disappeared_hour,
            disappeared_exact,
            succeeded,
        };
        keyed.push((acc.first_hour, acc.key, episode));
    }
    if ended_victims {
        warnings.insert(ENDED_VICTIMS.into());
    }
    warnings.extend(undrawn.into_iter().map(|(reason, count)| format!("{reason} (count: {count})")));
    keyed.sort_by_key(|&(first_hour, key, _)| (first_hour, key));
    out.extend(keyed.into_iter().map(|(_, _, episode)| episode));
    Ok((out, artwork))
}

struct EndColumns {
    kind: usize,
    launched_hour: usize,
    raider: usize,
    phase: usize,
    outcome: usize,
    target: [usize; 10],
    target_state: usize,
    victim: usize,
    victim_slot: usize,
    victim_unknown: usize,
}

impl EndColumns {
    fn new() -> io::Result<Self> {
        let field = |name: &str| dashboard_declare::field(TableId::RaidEnds, name);
        let mut target = [0; 10];
        for (at, name) in
            ["province", "state", "leader_reference", "leader_province", "building_type", "building_location", "building_flags", "building_country", "leader_country", "raid_type"].into_iter().enumerate()
        {
            target[at] = field(name)?;
        }
        Ok(Self {
            kind: field("kind")?,
            launched_hour: field("launched_hour")?,
            raider: field("raider")?,
            phase: field("phase")?,
            outcome: field("outcome")?,
            target,
            target_state: field("target_state")?,
            victim: field("victim")?,
            victim_slot: field("victim_slot")?,
            victim_unknown: field("victim_unknown")?,
        })
    }
}

fn unheld(query: &Query<'_>, from: i32, to: i32) -> Vec<MissingSpan> {
    query
        .timeline
        .unheld_within(from, to)
        .into_iter()
        .map(|gap| MissingSpan { first_hour: gap.first.max(from), last_hour: gap.last.min(to), reason: gap.reason })
        .collect()
}

fn span(query: &Query<'_>, from: Option<i32>, to: Option<i32>) -> io::Result<(i32, i32)> {
    let (from, to) = (from.unwrap_or(query.timeline.first), to.unwrap_or(query.timeline.last));
    if from > to {
        return Err(dashboard_core::invalid("Invalid span"));
    }
    Ok((from, to))
}

fn validate(branch_id: &str, countries: &[Country]) -> Result<(), &'static str> {
    crate::products::intelligence::branch(branch_id)?;
    countries.iter().try_for_each(dashboard_base::products::validate_country)
}

pub struct RaidLog;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RaidLogParams {
    pub branch_id: String,
    #[doc = " The timeline's first and last hour when none."]
    #[serde(default)]
    pub from: Option<i32>,
    #[serde(default)]
    pub to: Option<i32>,
    #[doc = " Raids by or against these countries; every raid when empty."]
    #[serde(default)]
    pub countries: Vec<Country>,
    #[doc = " Only the raid types and whether raids are available: no raid is read."]
    #[serde(default)]
    pub catalogue: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RaidLogReply {
    #[doc = " False only when the content proves no raid type exists; true once a raid was recorded; none"]
    #[doc = " when neither is known."]
    pub available: Option<bool>,
    pub unavailable: Option<String>,
    #[doc = " The game's phase and outcome numbers a reader compares with."]
    pub phase_preparing: i32,
    pub phase_ended: i32,
    pub outcome_none: i32,
    pub outcome_failure: i32,
    pub outcome_limited_success: i32,
    pub outcome_success: i32,
    pub outcome_critical_success: i32,
    pub outcome_canceled: i32,
    #[doc = " Every raid type the content declares, then each it holds undecided with why."]
    pub types: Vec<RaidTypeInfo>,
    #[doc = " Each icon the episodes name, once."]
    pub artwork: Vec<RaidArtwork>,
    #[doc = " By first hour."]
    pub episodes: Vec<RaidEpisode>,
    pub missing: Vec<MissingSpan>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(RaidLogReply);

fn recorded(source: &Source, query: &Query<'_>) -> io::Result<bool> {
    for run in query.timeline.stretches.iter().map(|stretch| stretch.run).collect::<BTreeSet<_>>() {
        if !dashboard_base::queries::lifetimes(source, run, TableId::Raids)?.is_empty() {
            return Ok(true);
        }
    }
    Ok(false)
}

impl Product for RaidLog {
    const KEY: &'static str = "raid-log";
    const CONTENT_DEPENDENT: bool = true;
    type Params = RaidLogParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, from: Start, to: End, countries, catalogue, "countries[]"];
    type Reply = RaidLogReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            RaidLogParams::decl(),
            RaidTypeInfo::decl(),
            RaidArtwork::decl(),
            RaidVictimSource::decl(),
            RaidPhaseRun::decl(),
            RaidPreparationRun::decl(),
            RaidEpisode::decl(),
            RaidLogReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        validate(&params.branch_id, &params.countries)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let query = dashboard_base::queries::query(source, params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?)?;
        let (from, to) = span(&query, params.from, params.to)?;
        let content = Content::load(source, context);
        let types = content.types();
        let mut warnings: BTreeSet<String> = content.warnings().into_iter().collect();
        let (mut episodes, artwork) = if params.catalogue { (Vec::new(), BTreeMap::new()) } else { episodes(source, &query, from, to, &content, &mut warnings)? };
        if !params.countries.is_empty() {
            episodes.retain(|episode| {
                [&episode.raider, &episode.victim].into_iter().flatten().any(|tag| params.countries.iter().any(|country| query.holders.goes_by(country, tag, episode.first_hour)))
            });
        }
        let none = content.raids.as_ref().is_ok_and(|raids| raids.types.is_empty() && raids.uncertain.is_empty());
        let available = if none { Some(false) } else { recorded(source, &query)?.then_some(true) };
        let reply = RaidLogReply {
            available,
            unavailable: none.then(|| "The content declares no raid type.".to_owned()),
            phase_preparing: native::PHASE_PREPARING,
            phase_ended: native::PHASE_ENDED,
            outcome_none: native::OUTCOME_NONE,
            outcome_failure: native::OUTCOME_FAILURE,
            outcome_limited_success: native::OUTCOME_LIMITED_SUCCESS,
            outcome_success: native::OUTCOME_SUCCESS,
            outcome_critical_success: native::OUTCOME_CRITICAL_SUCCESS,
            outcome_canceled: native::OUTCOME_CANCELED,
            types,
            artwork: artwork.into_values().collect(),
            episodes,
            missing: unheld(&query, from, to),
            warnings: warnings.into_iter().collect(),
        };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

#[cfg(test)]
mod tests;
