use dashboard_base::products::Country;
use dashboard_base::{Source, products::{LoadContext, Product}, queries::country_ids::Names};
use dashboard_base::catalogue::{self, CachedResult, Catalogue, View};
use dashboard_query::{Query, read, series::MissingSpan};
use engine_native_air::craid_instance as native;
use content_reader_raids::raids::Catalog;
use replay_lanes::lanes::{self, Cell, Keys};
use replay_data::rows::{RaidRow, RaidScope, TableId};
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
const END_HOUR: usize = 18;
const COMPLETION_COUNTRY: usize = 19;
const COMPLETION_RECORDED: usize = 20;
const RAID_COLUMNS: [usize; 8] = [PARENT_SLOT, PHASE, OUTCOME, TRAVELLED, PREPARATION, END_HOUR, COMPLETION_COUNTRY, COMPLETION_RECORDED];
const TARGET_COLUMNS: [usize; 10] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
const RAID_TYPE: usize = 9;
const UNKNOWN_ICON: &str = "GFX_raid_type_icon_unknown";
const TYPE_NOT_RECORDED: &str = "The raid's type is not recorded";
const ENDED_VICTIMS: &str = "Some ended raids lacked a recorded completion-time country and fell back to the target controller or captor at their first ended read.";

fn lane_value(cell: Option<Cell>) -> Option<Option<i64>> {
    match cell? {
        Cell::Number(value) => Some(Some(value)),
        Cell::Missing => Some(None),
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TargetIdentity {
    province: u32,
    state: u32,
    leader_reference: u64,
    building_type: u32,
    building_location: u32,
    building_flags: u32,
    building_country: i32,
}

impl TargetIdentity {
    fn of(scope: &RaidScope) -> Self {
        Self {
            province: scope.province,
            state: scope.state,
            leader_reference: scope.leader_reference,
            building_type: scope.building_type,
            building_location: scope.building_location,
            building_flags: scope.building_flags,
            building_country: scope.building_country,
        }
    }
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unit {
    pub phase: i32,
    pub outcome: i32,
    pub raider: Option<u32>,
    pub launched: bool,
    pub preparation: Option<i32>,
    pub native_end: Option<i32>,
    pub completion: Option<Option<u32>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Piece {
    pub first: i32,
    pub last: i32,
    pub unit: Option<Unit>,
}

pub fn pieces(steps: &[lanes::Step], last: i32, splits: &[i32]) -> Vec<Piece> {
    let mut out = Vec::new();
    for (at, step) in steps.iter().enumerate() {
        if step.hour > last {
            break;
        }
        let through = steps.get(at + 1).map_or(last, |next| (next.hour - 1).min(last));
        let Some(cells) = step.cells.as_deref() else { continue };
        let cell = |column: usize| RAID_COLUMNS.iter().position(|&c| c == column).and_then(|at| cells.get(at).copied());
        let number = |column: usize| match cell(column) {
            Some(Cell::Number(value)) => Some(value),
            _ => None,
        };
        let unit = match (number(PHASE), number(OUTCOME)) {
            (Some(phase), Some(outcome)) => {
                let (phase, outcome) = (phase as i32, outcome as i32);
                let ended = phase == native::PHASE_ENDED;
                let recorded = number(COMPLETION_RECORDED).map_or(0, |flags| flags as u8);
                Some(Unit {
                    phase,
                    outcome,
                    raider: number(PARENT_SLOT).map(|slot| slot as u32),
                    launched: phase == native::PHASE_IN_PROGRESS || number(TRAVELLED).is_some_and(|distance| distance != 0 && finite(distance)),
                    preparation: number(PREPARATION).filter(|_| phase == native::PHASE_PREPARING).map(|hours| hours as i32),
                    native_end: number(END_HOUR).filter(|_| ended && recorded & RaidRow::END_HOUR_RECORDED != 0).map(|hour| hour as i32),
                    completion: if ended && recorded & RaidRow::COMPLETION_COUNTRY_RECORDED != 0 {
                        lane_value(cell(COMPLETION_COUNTRY)).map(|raw| raw.map(|raw| raw as u32))
                    } else {
                        None
                    },
                })
            }
            _ => None,
        };
        let mut first = step.hour;
        let start = splits.partition_point(|&hour| hour <= step.hour);
        for &split in splits[start..].iter().take_while(|&&hour| hour <= through) {
            out.push(Piece { first, last: split - 1, unit: unit.clone() });
            first = split;
        }
        out.push(Piece { first, last: through, unit });
    }
    out
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Identity {
    raider: Option<u32>,
    target: Option<TargetIdentity>,
    kind: Option<String>,
}

impl Identity {
    fn contradicts(&self, other: &Self) -> bool {
        fn differ<T: PartialEq>(a: &Option<T>, b: &Option<T>) -> bool {
            matches!((a, b), (Some(a), Some(b)) if a != b)
        }
        differ(&self.raider, &other.raider) || differ(&self.target, &other.target) || differ(&self.kind, &other.kind)
    }

    fn matches(&self, other: &Self) -> bool {
        self.raider.is_some() && self.target.is_some() && self.kind.is_some() && self == other
    }

    fn fill(&mut self, other: &Self) {
        self.raider = self.raider.or(other.raider);
        self.target = self.target.or(other.target);
        if self.kind.is_none() {
            self.kind.clone_from(&other.kind);
        }
    }
}

fn successful(outcome: i32) -> bool {
    matches!(outcome, native::OUTCOME_SUCCESS | native::OUTCOME_CRITICAL_SUCCESS)
}

fn prepare(runs: &mut Vec<RaidPreparationRun>, first: i32, last: i32, value: i32) {
    match runs.last_mut() {
        Some(run) if run.1 + 1 == first && (run.0 == run.1 || i64::from(run.2) + i64::from(first - run.0) * run.3 == i64::from(value)) => {
            if run.0 == run.1 {
                run.3 = i64::from(value) - i64::from(run.2);
            }
            run.1 = first;
        }
        _ => runs.push(RaidPreparationRun(first, first, value, 0)),
    }
    if last > first {
        match runs.last_mut() {
            Some(run) if run.0 == run.1 || run.3 == 0 => (run.1, run.3) = (last, 0),
            _ => runs.push(RaidPreparationRun(first + 1, last, value, 0)),
        }
    }
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
    CompletionCountry,
    FixedTarget,
    ControllerAtFirstEndedRead,
    ControllerAtObservedRead,
    CaptorAtFirstEndedRead,
    CaptorAtObservedRead,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum RaidEndedSource {
    NativeEndDate,
    ObservedEndedRead,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RaidVictimAt {
    pub hour: i32,
    pub tag: Option<String>,
    pub missing: Option<String>,
    pub source: Option<RaidVictimSource>,
}

pub fn victim_source(scope: &RaidScope, ended: bool) -> RaidVictimSource {
    if scope.building_type != 0 && scope.building_country > 0 {
        RaidVictimSource::FixedTarget
    } else if scope.building_type == 0 && scope.province == 0 && scope.state == 0 && scope.leader_country.is_some() {
        if ended { RaidVictimSource::CaptorAtFirstEndedRead } else { RaidVictimSource::CaptorAtObservedRead }
    } else if ended {
        RaidVictimSource::ControllerAtFirstEndedRead
    } else {
        RaidVictimSource::ControllerAtObservedRead
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

pub fn has_hour_in_range(spans: &[(i32, i32)], first: i32, last: i32) -> bool {
    if first > last {
        return false;
    }
    let at = spans.partition_point(|span| span.1 < first);
    spans.get(at).is_some_and(|span| span.0 <= last)
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StretchRun {
    pub stretch: usize,
    pub first: i32,
    pub last: i32,
    pub phase: i32,
    pub outcome: i32,
}

pub fn date_bounds(runs: &[StretchRun], raw_first: i32, complete_by_stretch: &[Vec<(i32, i32)>]) -> Vec<(bool, Option<i32>)> {
    runs.iter()
        .enumerate()
        .map(|(index, run)| {
            let complete = complete_by_stretch.get(run.stretch).map_or(&[][..], Vec::as_slice);
            if index == 0 {
                if run.first == raw_first {
                    let after = prev_in_spans(complete, run.first);
                    let exact = after == Some(run.first - 1) && in_spans(complete, run.first);
                    (exact, after)
                } else {
                    (false, None)
                }
            } else {
                let prev = runs[index - 1];
                let exact = prev.stretch == run.stretch
                    && prev.last + 1 == run.first
                    && in_spans(complete, run.first - 1)
                    && in_spans(complete, run.first);
                (exact, Some(prev.last))
            }
        })
        .collect()
}

pub fn exact_at(hour: i32, stretch: usize, raw_first: i32, first_exact: bool, runs: &[StretchRun], complete_by_stretch: &[Vec<(i32, i32)>]) -> bool {
    let complete = complete_by_stretch.get(stretch).map_or(&[][..], Vec::as_slice);
    if !in_spans(complete, hour) || !in_spans(complete, hour - 1) {
        return false;
    }
    if hour == raw_first {
        return first_exact;
    }
    runs.iter().any(|run| run.stretch == stretch && run.first <= hour - 1 && hour - 1 <= run.last)
}

pub fn disappearance(last: i32, last_stretch: usize, seen_again: Option<i32>, complete_by_stretch: &[Vec<(i32, i32)>]) -> Option<(i32, bool)> {
    complete_by_stretch
        .iter()
        .enumerate()
        .skip(last_stretch)
        .find_map(|(stretch, spans)| next_in_spans(spans, last).map(|hour| (hour, stretch == last_stretch && hour == last + 1)))
        .filter(|&(hour, _)| seen_again.is_none_or(|again| hour < again))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RaidEndAt {
    pub hour: i32,
    pub ended_hour: i32,
    pub exact: bool,
    pub source: RaidEndedSource,
    #[doc = " The raider slot's tag at the end."]
    pub raider: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RaidPhaseRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub first_exact: bool,
    pub after_hour: Option<i32>,
    pub raider: Option<String>,
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
    pub raider_slot: Option<u32>,
    pub launched_raider: Option<String>,
    pub victim: Option<String>,
    #[doc = " Why the victim is not named, when it is not."]
    pub victim_missing: Option<String>,
    pub victim_source: Option<RaidVictimSource>,
    pub victims: Vec<RaidVictimAt>,
    #[doc = " The hour the raid type's spelling was first read; none, and no type, before it."]
    pub raid_type_hour: Option<i32>,
    #[doc = " The raid type's token and its name in the reader's language."]
    pub raid_type: Option<String>,
    pub raid_type_name: Option<String>,
    #[doc = " The sprite key into the reply's artwork, or why there is none."]
    pub icon: Option<String>,
    pub icon_missing: Option<String>,
    #[doc = " The hour the target's place was first read; none, and no place, before it."]
    pub place_hour: Option<i32>,
    pub province: Option<u32>,
    pub state: Option<u32>,
    #[doc = " Its last phase, and its last outcome read."]
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
    #[doc = " What was known of its end, by the hour it was read: an observed end, then a native end date once read."]
    pub ends: Vec<RaidEndAt>,
    #[doc = " The first read that saw it ended and successful: what a later read reveals is never counted before it."]
    pub succeeded_hour: Option<i32>,
    pub disappeared_hour: Option<i32>,
    pub disappeared_exact: bool,
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
                    dashboard_base::products::artwork::resolve_sprite_type(cat, sprite, 0, 48)
                        .into_result()
                        .and_then(|url| url.ok_or_else(|| "artwork unavailable".to_owned()))
                        .map_err(|error| format!("raid sprite {key}: {error}"))
                })
                .map(|image| (key, image)),
        };
        drawn.lock().unwrap_or_else(|error| error.into_inner()).insert(held, made.clone());
        made
    }
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

fn step_hours_in(steps: Option<&Vec<lanes::Step>>, first: i32, last: i32, out: &mut BTreeSet<i32>) {
    let Some(steps) = steps else { return };
    let start = steps.partition_point(|step| step.hour <= first);
    for step in &steps[start..] {
        if step.hour > last {
            break;
        }
        if step.cells.is_some() {
            out.insert(step.hour);
        }
    }
}

fn push_victim(list: &mut Vec<RaidVictimAt>, item: RaidVictimAt) {
    if let Some(prev) = list.last()
        && prev.tag == item.tag
        && prev.missing == item.missing
        && prev.source == item.source
    {
        return;
    }
    if let Some(prev) = list.last_mut()
        && prev.hour == item.hour
    {
        *prev = item;
        return;
    }
    list.push(item);
}

struct End {
    hour: i32,
    stretch: usize,
    ended: i32,
    native: bool,
}

struct Accumulated {
    key: u128,
    raw_first: i32,
    first_hour: i32,
    last_hour: i32,
    last_stretch: usize,
    runs: Vec<StretchRun>,
    preparation: Vec<RaidPreparationRun>,
    identity: Identity,
    type_hour: Option<i32>,
    template: Option<Result<String, u32>>,
    place: Option<(i32, Option<u32>, Option<u32>)>,
    phase: Option<i32>,
    launched_hour: Option<(i32, usize)>,
    ends: Vec<End>,
    seen_again: Option<i32>,
    victims: Vec<RaidVictimAt>,
    completion_seen: bool,
}

fn episodes(
    source: &Source,
    query: &Query<'_>,
    (from, to): (i32, i32),
    chosen: &[Country],
    content: &Content,
    warnings: &mut BTreeSet<String>,
) -> io::Result<(Vec<RaidEpisode>, BTreeMap<String, RaidArtwork>)> {
    let recording = query.replay.recording();
    let countries = Names::new(&query.holders);
    let province_columns = [dashboard_declare::field(TableId::Provinces, "state")?, dashboard_declare::field(TableId::Provinces, "controller")?];
    let state_controller = dashboard_declare::field(TableId::States, "controller")?;
    let mut accumulated: Vec<Accumulated> = Vec::new();
    let mut active: HashMap<u128, usize> = HashMap::new();
    let mut complete_by_stretch: Vec<Vec<(i32, i32)>> = Vec::new();
    let mut complete_all: Vec<(i32, i32)> = Vec::new();
    let mut artwork = BTreeMap::new();
    let mut undrawn: BTreeMap<String, u64> = BTreeMap::new();
    let mut labels: HashMap<&'static str, Option<String>> = HashMap::new();
    let mut ended_victims = false;
    for (stretch_idx, stretch) in query.timeline.within(query.timeline.first, to).enumerate() {
        let run = stretch.run;
        let end = stretch.last.min(to);
        let stretch_complete = coalesce_spans(dashboard_query::cost::storage(|| lanes::complete_reads(recording, run, TableId::Raids, stretch.first, end))?);
        complete_all.extend_from_slice(&stretch_complete);
        complete_by_stretch.push(stretch_complete);
        let lives = dashboard_base::queries::lifetimes(source, run, TableId::Raids)?;
        let stretch_first = query.timeline.at(stretch.first).map_or(stretch.first, |held| held.first);
        let met: Vec<(u128, i32, i32)> = lives
            .iter()
            .filter_map(|&(key, raw_first, raw_last)| {
                let raw_first = i32::try_from(raw_first).ok()?.max(stretch_first);
                let last = i32::try_from(raw_last).ok()?.min(end);
                (raw_first <= last).then_some((key, raw_first, last))
            })
            .collect();
        if met.is_empty() {
            continue;
        }
        let links = dashboard_base::queries::links(source, run)?;
        let keys: Vec<u128> = met.iter().map(|(key, ..)| *key).collect::<BTreeSet<_>>().into_iter().collect();
        let start = met.iter().map(|(_, raw_first, _)| *raw_first).min().unwrap_or(stretch.first);
        let raids = dashboard_query::cost::storage(|| lanes::history(recording, run, TableId::Raids, &RAID_COLUMNS, Keys::These(&keys), start, end))?;
        let targets = dashboard_query::cost::storage(|| lanes::history(recording, run, TableId::RaidTargets, &TARGET_COLUMNS, Keys::These(&keys), start, end))?;
        let (mut provinces, mut states) = (BTreeSet::new(), BTreeSet::new());
        let mut tokens: BTreeSet<u128> = BTreeSet::new();
        for &key in &keys {
            for cells in steps_of(&targets, key).into_iter().flatten().filter_map(|step| step.cells.as_deref()) {
                if let Some(scope) = scope_of(|at| lane_value(cells.get(at).copied())) {
                    provinces.extend([scope.province, scope.leader_province, scope.building_location].into_iter().filter(|&id| id != 0).map(u128::from));
                    states.extend([scope.state, scope.building_location].into_iter().filter(|&id| id != 0).map(u128::from));
                    tokens.extend((scope.building_type != 0).then_some(u128::from(scope.building_type)));
                }
                if let Some(token) = lane_value(cells.get(RAID_TYPE).copied()).flatten().filter(|&t| t != 0) {
                    tokens.insert(token as u128);
                }
            }
        }
        let places = |table: TableId, columns: &[usize], ids: &BTreeSet<u128>| -> io::Result<lanes::History> {
            if ids.is_empty() {
                return Ok(lanes::History::default());
            }
            let ids: Vec<u128> = ids.iter().copied().collect();
            dashboard_query::cost::storage(|| lanes::history(recording, run, table, columns, Keys::These(&ids), start, end))
        };
        let province_history = places(TableId::Provinces, &province_columns, &provinces)?;
        let state_history = places(TableId::States, &[state_controller], &states)?;
        let lexical = places(TableId::LexicalTokens, &[0], &tokens)?;
        let spelled: Vec<i32> = lexical.keys.iter().flat_map(|(_, steps)| steps.iter().map(|step| step.hour)).collect();
        let literal_at = |token: u32, hour: i32| -> Option<String> {
            let cell = cell_at(&lexical, steps_of(&lexical, u128::from(token)), 0, hour)?;
            let literal = String::from_utf8_lossy(lexical.text(cell)?).into_owned();
            (!literal.is_empty()).then_some(literal)
        };
        let place = |history: &lanes::History, column: usize, id: u32, hour: i32| -> Result<u32, ()> {
            match cell_at(history, steps_of(history, u128::from(id)), column, hour) {
                Some(Cell::Number(value)) => Ok(value as u32),
                _ => Err(()),
            }
        };
        let tag = |raw: u32, hour: i32| countries.tag_at(&links, raw, hour).map(str::to_owned);
        let target_at = |key: u128, at: i32| -> (Option<RaidScope>, Option<u32>) {
            let steps = steps_of(&targets, key);
            let scope = scope_of(|column| lane_value(cell_at(&targets, steps, column, at)));
            let token = lane_value(cell_at(&targets, steps, RAID_TYPE, at)).flatten().filter(|&t| t != 0).map(|t| t as u32);
            (scope, token)
        };
        let victim_at = |scope: Option<&RaidScope>, at: i32, is_ended: bool| -> RaidVictimAt {
            let (victim_tag, victim_missing) = match scope {
                None => (None, Some("The raid's target is not recorded".to_owned())),
                Some(scope) => match scope.country_with(|id| place(&province_history, province_columns[1], id, at), |id| place(&state_history, state_controller, id, at)) {
                    Err(()) => (None, Some("The targeted province's or state's controller is not recorded".to_owned())),
                    Ok(None) => (None, Some("The raid's target names no country".to_owned())),
                    Ok(Some(raw)) => match tag(raw, at) {
                        Some(tag) => (Some(tag), None),
                        None => (None, Some("The victim's country holds no tag at the raid's hour".to_owned())),
                    },
                },
            };
            let source = victim_tag.as_ref().and_then(|_| scope.map(|s| victim_source(s, is_ended)));
            RaidVictimAt { hour: at, tag: victim_tag, missing: victim_missing, source }
        };
        let completion_victim = |at: i32, ended: i32, completion: Option<u32>| -> RaidVictimAt {
            match completion {
                Some(raw) => match tag(raw, ended) {
                    Some(t) => RaidVictimAt { hour: at, tag: Some(t), missing: None, source: Some(RaidVictimSource::CompletionCountry) },
                    None => RaidVictimAt { hour: at, tag: None, missing: Some("The completion-time victim country holds no tag at the raid's hour".to_owned()), source: None },
                },
                None => RaidVictimAt { hour: at, tag: None, missing: Some("The raid's target resolved to no country at completion".to_owned()), source: None },
            }
        };
        let absent = |after: i32, before: i32| has_hour_in_range(&complete_all, after.saturating_add(1), before.saturating_sub(1));
        for &(key, raw_first, last) in &met {
            let target_steps = steps_of(&targets, key);
            let mut splits: Vec<i32> = target_steps.into_iter().flatten().map(|step| step.hour).chain(spelled.iter().copied()).collect();
            splits.sort_unstable();
            splits.dedup();
            let seen = pieces(steps_of(&raids, key).map_or(&[][..], Vec::as_slice), last, &splits);
            let mut fresh = true;
            for piece in &seen {
                if let Some(&idx) = active.get(&key)
                    && absent(accumulated[idx].last_hour, piece.first)
                {
                    active.remove(&key);
                }
                let Some(unit) = &piece.unit else {
                    if let Some(&idx) = active.get(&key) {
                        accumulated[idx].seen_again.get_or_insert(piece.first);
                    }
                    fresh = false;
                    continue;
                };
                let (scope, token) = target_at(key, piece.first);
                let identity = Identity { raider: unit.raider, target: scope.as_ref().map(TargetIdentity::of), kind: token.and_then(|token| literal_at(token, piece.first)) };
                let admitted = active.get(&key).copied().filter(|&idx| {
                    let acc = &accumulated[idx];
                    let continuous = acc.last_stretch == stretch_idx && acc.last_hour + 1 == piece.first;
                    !acc.identity.contradicts(&identity) && !acc.phase.is_some_and(|phase| unit.phase < phase) && (continuous || acc.identity.matches(&identity))
                });
                if admitted.is_none()
                    && let Some(&idx) = active.get(&key)
                {
                    accumulated[idx].seen_again.get_or_insert(piece.first);
                }
                let idx = admitted.unwrap_or_else(|| {
                    accumulated.push(Accumulated {
                        key,
                        raw_first: if fresh { raw_first } else { piece.first },
                        first_hour: piece.first,
                        last_hour: piece.first,
                        last_stretch: stretch_idx,
                        runs: Vec::new(),
                        preparation: Vec::new(),
                        identity: Identity::default(),
                        type_hour: None,
                        template: None,
                        place: None,
                        phase: None,
                        launched_hour: None,
                        ends: Vec::new(),
                        seen_again: None,
                        victims: Vec::new(),
                        completion_seen: false,
                    });
                    active.insert(key, accumulated.len() - 1);
                    accumulated.len() - 1
                });
                fresh = false;
                let acc = &mut accumulated[idx];
                acc.last_hour = piece.last;
                acc.last_stretch = stretch_idx;
                acc.seen_again = None;
                acc.identity.fill(&identity);
                if acc.type_hour.is_none() && acc.identity.kind.is_some() {
                    acc.type_hour = Some(piece.first);
                }
                if let Some(scope) = &scope {
                    if acc.place.is_none() {
                        let state = scope.state_with(|id| place(&province_history, province_columns[0], id, piece.first)).ok().flatten();
                        acc.place = Some((piece.first, province_of(scope), state));
                    }
                    if scope.building_type != 0 && !matches!(acc.template, Some(Ok(_))) {
                        acc.template = Some(literal_at(scope.building_type, piece.first).ok_or(scope.building_type));
                    }
                }
                match acc.runs.last_mut() {
                    Some(prev) if prev.stretch == stretch_idx && prev.last + 1 == piece.first && (prev.phase, prev.outcome) == (unit.phase, unit.outcome) => prev.last = piece.last,
                    _ => acc.runs.push(StretchRun { stretch: stretch_idx, first: piece.first, last: piece.last, phase: unit.phase, outcome: unit.outcome }),
                }
                if let Some(hours) = unit.preparation {
                    prepare(&mut acc.preparation, piece.first, piece.last, hours);
                }
                acc.phase = Some(unit.phase);
                if acc.launched_hour.is_none() && unit.launched {
                    acc.launched_hour = Some((piece.first, stretch_idx));
                }
                if unit.phase == native::PHASE_ENDED {
                    let first_end = acc.ends.is_empty();
                    if first_end || (unit.native_end.is_some() && !acc.ends.iter().any(|end| end.native)) {
                        let (ended, native) = unit.native_end.map_or((piece.first, false), |ended| (ended, true));
                        acc.ends.push(End { hour: piece.first, stretch: stretch_idx, ended, native });
                    }
                    let ended = acc.ends.last().map_or(piece.first, |end| end.ended);
                    if first_end {
                        let victim = match unit.completion {
                            Some(completion) => completion_victim(piece.first, ended, completion),
                            None => victim_at(scope.as_ref(), piece.first, true),
                        };
                        push_victim(&mut acc.victims, victim);
                        acc.completion_seen = unit.completion.is_some();
                    } else if let Some(completion) = unit.completion.filter(|_| !acc.completion_seen) {
                        push_victim(&mut acc.victims, completion_victim(piece.first, ended, completion));
                        acc.completion_seen = true;
                    }
                } else {
                    let mut hours = BTreeSet::from([piece.first]);
                    step_hours_in(target_steps, piece.first, piece.last, &mut hours);
                    if let Some(scope) = &scope {
                        for id in [scope.province, scope.leader_province, scope.building_location].into_iter().filter(|&id| id != 0) {
                            step_hours_in(steps_of(&province_history, u128::from(id)), piece.first, piece.last, &mut hours);
                        }
                        for id in [scope.state, scope.building_location].into_iter().filter(|&id| id != 0) {
                            step_hours_in(steps_of(&state_history, u128::from(id)), piece.first, piece.last, &mut hours);
                        }
                    }
                    for hour in hours {
                        push_victim(&mut acc.victims, victim_at(target_at(key, hour).0.as_ref(), hour, false));
                    }
                }
            }
        }
    }
    let mut key_counts: HashMap<u128, usize> = HashMap::new();
    let mut keyed: Vec<(i32, u128, usize, RaidEpisode)> = Vec::with_capacity(accumulated.len());
    for (order, acc) in accumulated.into_iter().enumerate() {
        let count = key_counts.entry(acc.key).or_insert(0);
        *count += 1;
        let id = if *count == 1 { acc.key.to_string() } else { format!("{}#{}", acc.key, *count) };
        if acc.last_hour < from || acc.first_hour > to {
            continue;
        }
        let bounds = date_bounds(&acc.runs, acc.raw_first, &complete_by_stretch);
        let unclipped_first_exact = bounds.first().is_some_and(|&(exact, _)| exact);
        let raider_at = |hour: i32| acc.identity.raider.and_then(|slot| countries.slot_tag(slot, hour).map(str::to_owned));
        let mut label = |key: &'static str| labels.entry(key).or_insert_with(|| content.label(key)).clone();
        let mut runs: Vec<RaidPhaseRun> = Vec::new();
        for (&run, (first_exact, after_hour)) in acc.runs.iter().zip(bounds) {
            if run.last < from {
                continue;
            }
            let (first_hour, first_exact, after_hour) = if run.first < from { (from, false, None) } else { (run.first, first_exact, after_hour) };
            runs.push(RaidPhaseRun {
                first_hour,
                last_hour: run.last.min(to),
                first_exact,
                after_hour,
                raider: raider_at(first_hour),
                phase: run.phase,
                phase_name: phase_label(run.phase, &mut label),
                outcome: run.outcome,
                outcome_name: outcome_label(run.outcome, &mut label),
            });
        }
        if runs.is_empty() {
            continue;
        }
        let first_hour = acc.first_hour.max(from);
        let preparation: Vec<RaidPreparationRun> = acc
            .preparation
            .iter()
            .filter_map(|p| {
                if p.1 < from || p.0 > to {
                    return None;
                }
                let start = p.0.max(from);
                let val = (i64::from(p.2) + i64::from(start - p.0) * p.3) as i32;
                Some(RaidPreparationRun(start, p.1.min(to), val, p.3))
            })
            .collect();
        let (launched_hour, launched_exact, launched_raider) = match acc.launched_hour {
            Some((h, stretch)) if h >= from => (Some(h), exact_at(h, stretch, acc.raw_first, unclipped_first_exact, &acc.runs, &complete_by_stretch), raider_at(h)),
            _ => (None, false, None),
        };
        let mut ends: Vec<RaidEndAt> = Vec::new();
        for end in &acc.ends {
            let raider = raider_at(end.ended);
            let at = if end.native {
                RaidEndAt { hour: end.hour, ended_hour: end.ended, exact: true, source: RaidEndedSource::NativeEndDate, raider }
            } else {
                RaidEndAt { hour: end.hour, ended_hour: end.ended, exact: exact_at(end.ended, end.stretch, acc.raw_first, unclipped_first_exact, &acc.runs, &complete_by_stretch), source: RaidEndedSource::ObservedEndedRead, raider }
            };
            if at.hour <= from {
                ends.clear();
            }
            ends.push(at);
        }
        let disappeared = (acc.ends.is_empty()).then(|| disappearance(acc.last_hour, acc.last_stretch, acc.seen_again, &complete_by_stretch)).flatten();
        let (disappeared_hour, disappeared_exact) = match disappeared {
            Some((h, exact)) if h >= from && h <= to => (Some(h), exact),
            _ => (None, false),
        };
        let mut victims: Vec<RaidVictimAt> = Vec::new();
        let mut before_from: Option<RaidVictimAt> = None;
        for v in acc.victims {
            if v.hour <= from {
                before_from = Some(RaidVictimAt { hour: from, ..v });
            } else if v.hour <= to {
                if let Some(first_v) = before_from.take() {
                    push_victim(&mut victims, first_v);
                }
                push_victim(&mut victims, v);
            }
        }
        if let Some(first_v) = before_from {
            push_victim(&mut victims, first_v);
        }
        let raider = runs[0].raider.clone().or_else(|| raider_at(first_hour));
        if !chosen.is_empty() {
            let by_slot = acc.identity.raider.is_some_and(|slot| chosen.iter().any(|country| matches!(country, Country::Slot(chosen) if *chosen == slot)));
            let raider_tags = runs
                .iter()
                .filter_map(|run| run.raider.as_deref().map(|tag| (tag, run.first_hour)))
                .chain(raider.as_deref().map(|tag| (tag, first_hour)))
                .chain(launched_raider.as_deref().zip(launched_hour))
                .chain(ends.iter().filter_map(|end| end.raider.as_deref().map(|tag| (tag, end.ended_hour))));
            let victim_tags = victims.iter().filter_map(|v| v.tag.as_deref().map(|tag| (tag, v.hour)));
            if !by_slot && !raider_tags.chain(victim_tags).any(|(tag, at)| chosen.iter().any(|country| query.holders.goes_by(country, tag, at))) {
                continue;
            }
        }
        if victims.iter().any(|v| matches!(v.source, Some(RaidVictimSource::ControllerAtFirstEndedRead | RaidVictimSource::CaptorAtFirstEndedRead))) {
            ended_victims = true;
        }
        if raider.is_none() {
            warnings.insert("Some raids' raiders are not named: their country holds no tag at the raid's hour.".into());
        }
        let (icon, icon_missing) = match (&acc.identity.kind, &acc.template) {
            (None, _) => (None, Some(TYPE_NOT_RECORDED.to_owned())),
            (Some(_), Some(Err(token))) => (None, Some(format!("raid target building token {token} is not recorded"))),
            (Some(literal), template) => match content.icon(literal, template.as_ref().and_then(|template| template.as_deref().ok())) {
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
        let raid_type_name = acc.identity.kind.as_ref().and_then(|literal| content.label(&format!("raid_type_{literal}")));
        let (place_hour, province, state) = acc.place.map_or((None, None, None), |(hour, province, state)| (Some(hour.max(from)), province, state));
        let episode = RaidEpisode {
            id,
            first_hour,
            first_hour_exact: acc.first_hour >= from && unclipped_first_exact,
            last_hour: acc.last_hour.min(to),
            raider,
            raider_slot: acc.identity.raider,
            launched_raider,
            victim: None,
            victim_missing: None,
            victim_source: None,
            victims,
            raid_type_hour: acc.type_hour.map(|hour| hour.max(from)),
            raid_type: acc.identity.kind,
            raid_type_name,
            icon,
            icon_missing,
            place_hour,
            province,
            state,
            phase: None,
            phase_name: None,
            outcome: None,
            outcome_name: None,
            outcome_missing: None,
            succeeded_hour: acc.runs.iter().find(|run| run.phase == native::PHASE_ENDED && successful(run.outcome)).map(|run| run.first),
            runs,
            preparation,
            launched: acc.launched_hour.is_some(),
            launched_hour,
            launched_exact,
            ends,
            disappeared_hour,
            disappeared_exact,
        };
        if let Some(episode) = episode_at(&episode, to) {
            keyed.push((first_hour, acc.key, order, episode));
        }
    }
    if ended_victims {
        warnings.insert(ENDED_VICTIMS.into());
    }
    warnings.extend(undrawn.into_iter().map(|(reason, count)| format!("{reason} (count: {count})")));
    keyed.sort_by_key(|&(first_hour, key, order, _)| (first_hour, key, order));
    let out = keyed.into_iter().map(|(_, _, _, episode)| episode).collect();
    Ok((out, artwork))
}

pub fn episode_at(episode: &RaidEpisode, hour: i32) -> Option<RaidEpisode> {
    if episode.first_hour > hour {
        return None;
    }
    let mut out = episode.clone();
    out.runs.retain(|run| run.first_hour <= hour);
    for run in &mut out.runs {
        run.last_hour = run.last_hour.min(hour);
    }
    out.last_hour = out.runs.last()?.last_hour;
    out.preparation.retain(|run| run.0 <= hour);
    for run in &mut out.preparation {
        run.1 = run.1.min(hour);
        if run.0 == run.1 {
            run.3 = 0;
        }
    }
    out.victims.retain(|victim| victim.hour <= hour);
    let summary = out.victims.last().cloned().unwrap_or_else(|| RaidVictimAt { hour: out.first_hour, tag: None, missing: Some("The raid's target is not recorded".to_owned()), source: None });
    if out.victims.is_empty() {
        out.victims.push(summary.clone());
    }
    (out.victim, out.victim_missing, out.victim_source) = (summary.tag, summary.missing, summary.source);
    if out.raid_type_hour.is_none_or(|known| known > hour) {
        (out.raid_type_hour, out.raid_type, out.raid_type_name, out.icon) = (None, None, None, None);
        out.icon_missing = Some(TYPE_NOT_RECORDED.to_owned());
    }
    if out.place_hour.is_some_and(|known| known > hour) {
        (out.place_hour, out.province, out.state) = (None, None, None);
    }
    if out.launched_hour.is_some_and(|launched| launched > hour) {
        (out.launched, out.launched_hour, out.launched_exact, out.launched_raider) = (false, None, false, None);
    }
    out.ends.retain(|end| end.hour <= hour);
    out.succeeded_hour = out.succeeded_hour.filter(|&succeeded| succeeded <= hour);
    if out.disappeared_hour.is_some_and(|gone| gone > hour) {
        (out.disappeared_hour, out.disappeared_exact) = (None, false);
    }
    let latest = out.runs.last()?;
    let known = out.runs.iter().rev().find(|run| run.outcome != native::OUTCOME_NONE).unwrap_or(latest);
    (out.phase, out.phase_name) = (Some(latest.phase), latest.phase_name.clone());
    (out.outcome, out.outcome_name) = (Some(known.outcome), known.outcome_name.clone());
    out.outcome_missing = outcome_missing(!out.ends.is_empty(), out.disappeared_hour.is_some(), out.outcome);
    Some(out)
}

fn missing(query: &Query<'_>, from: i32, to: i32) -> io::Result<Vec<MissingSpan>> {
    let mut out: Vec<MissingSpan> = query
        .timeline
        .unheld_within(from, to)
        .into_iter()
        .map(|gap| MissingSpan { first_hour: gap.first.max(from), last_hour: gap.last.min(to), reason: gap.reason })
        .collect();
    let recording = query.replay.recording();
    for stretch in query.timeline.within(from, to) {
        let runs = read::statuses(recording, stretch.run, TableId::Raids, stretch.first.max(from), stretch.last.min(to))?;
        let reasons = read::status_reasons(recording, stretch.run, TableId::Raids, &runs)?;
        for run in runs {
            let held = match run.status {
                replay_columnar::manifest::Status::Unavailable { reason } => reasons.get(&reason).map(String::as_str),
                _ => None,
            };
            if let Some(reason) = dashboard_query::coverage::row_coverage(TableId::Raids, run.status, held, false).into_missing() {
                out.push(MissingSpan { first_hour: run.first, last_hour: run.last, reason });
            }
        }
    }
    out.sort_by_key(|span| span.first_hour);
    Ok(out)
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

fn recorded(source: &Source, query: &Query<'_>, to: i32) -> io::Result<bool> {
    for stretch in query.timeline.within(query.timeline.first, to) {
        let last = i64::from(stretch.last.min(to));
        let lives = dashboard_base::queries::lifetimes(source, stretch.run, TableId::Raids)?;
        if lives.iter().any(|&(_, first, end)| first <= last && end >= i64::from(stretch.first)) {
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
            RaidEndedSource::decl(),
            RaidVictimAt::decl(),
            RaidEndAt::decl(),
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
        let (episodes, artwork) = if params.catalogue { (Vec::new(), BTreeMap::new()) } else { episodes(source, &query, (from, to), &params.countries, &content, &mut warnings)? };
        let none = content.raids.as_ref().is_ok_and(|raids| raids.types.is_empty() && raids.uncertain.is_empty());
        let available = if none { Some(false) } else { recorded(source, &query, to)?.then_some(true) };
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
            missing: if params.catalogue { Vec::new() } else { missing(&query, from, to)? },
            warnings: warnings.into_iter().collect(),
        };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

pub mod tally;

#[cfg(test)]
mod tests;
