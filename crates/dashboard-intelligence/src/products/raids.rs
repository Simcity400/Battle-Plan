use dashboard_base::products::Country;
use dashboard_base::{Source, products::{LoadContext, Product}, queries::country_ids::Names};
use dashboard_base::catalogue::{self, CachedResult, Catalogue, View};
use dashboard_query::{Query, identity::{Holder, Holders}, read, series::{MissingSpan, ranked::GroupSelection}};
use dashboard_world::products::group_members::GroupsAt;
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
const TARGET_NOT_RECORDED: &str = "The raid's target is not recorded in full";
const RAIDERS_UNNAMED: &str = "Some raids' raiders are not named: their country holds no tag at the raid's hour.";
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
    #[doc = " The hour the icon was settled (the type spelled, the target read and its building spelled); none, and no icon, before it."]
    pub icon_hour: Option<i32>,
    #[doc = " The sprite key into the reply's artwork, or why there is none."]
    pub icon: Option<String>,
    pub icon_missing: Option<String>,
    #[doc = " The hour the target's place was first read; none, and no place, before it."]
    pub place_hour: Option<i32>,
    pub province: Option<u32>,
    #[doc = " The hour the target's state was first read; none, and no state, before it."]
    pub state_hour: Option<i32>,
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
    #[doc = " With countries asked, the hour a read first named one of them; the raid is not among theirs before it."]
    pub member_hour: Option<i32>,
}

struct Content {
    catalogue: Result<Arc<Catalogue>, String>,
    raids: Result<Arc<Catalog>, String>,
}

type Drawn = Mutex<HashMap<(String, Option<String>), (Result<(String, String), String>, catalogue::OwnRefusals)>>;

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
        if let Some((found, noted)) = drawn.lock().unwrap_or_else(|error| error.into_inner()).get(&held) {
            catalogue::credit(noted);
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
        let (made, noted) = catalogue::capture(|| match refused.into_inner() {
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
        });
        catalogue::credit(&noted);
        drawn.lock().unwrap_or_else(|error| error.into_inner()).insert(held, (made.clone(), noted));
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

fn spelled_at(recording: &replay_recording::columnar_frames::Recording, run: u32, asks: &[(i32, u128)]) -> io::Result<HashMap<u128, String>> {
    let mut found = HashMap::new();
    dashboard_query::cost::storage(|| {
        lanes::at_each(recording, run, TableId::LexicalTokens, Some(&[0][..]), asks, &mut |_, key, row| {
            if let Some((view, slot)) = row
                && let Some(text) = read::text(&view, 0, slot).filter(|text| !text.is_empty())
            {
                found.insert(key, String::from_utf8_lossy(text).into_owned());
            }
            Ok(())
        })
    })?;
    Ok(found)
}

struct Readable {
    spans: Vec<(i32, i32)>,
    before: Vec<i64>,
}

impl Readable {
    fn hours(&self) -> i64 {
        self.spans.last().map_or(0, |&(first, last)| self.before[self.spans.len() - 1] + i64::from(last - first) + 1)
    }

    fn hour(&self, at: i64) -> i32 {
        let span = self.before.partition_point(|&before| before <= at) - 1;
        self.spans[span].0 + (at - self.before[span]) as i32
    }

    fn index(&self, hour: i32) -> i64 {
        let span = self.spans.partition_point(|&(first, _)| first <= hour) - 1;
        self.before[span] + i64::from(hour - self.spans[span].0)
    }
}

fn first_reading(
    recording: &replay_recording::columnar_frames::Recording,
    run: u32,
    readable: &Readable,
    mut open: Vec<(u128, i64, i64, Option<String>)>,
    readings: &mut HashMap<(u128, i32), String>,
) -> io::Result<HashMap<u128, i32>> {
    let mut found = HashMap::new();
    while !open.is_empty() {
        let mut asks: Vec<(i32, u128)> = open.iter().filter(|&&(_, low, high, _)| low < high).map(|&(token, low, high, _)| (readable.hour(low + (high - low) / 2), token)).collect();
        asks.sort_unstable();
        let read = if asks.is_empty() { HashMap::new() } else { spelled_at(recording, run, &asks)? };
        for (token, low, high, wanted) in &mut open {
            if *low >= *high {
                continue;
            }
            let middle = *low + (*high - *low) / 2;
            let literal = read.get(token);
            if let Some(literal) = literal {
                readings.insert((*token, readable.hour(middle)), literal.clone());
            }
            if literal.is_some_and(|literal| wanted.as_ref().is_none_or(|wanted| wanted == literal)) {
                *high = middle;
            } else {
                *low = middle + 1;
            }
        }
        open.retain(|&(token, low, high, _)| {
            if low < high {
                return true;
            }
            found.insert(token, readable.hour(high));
            false
        });
    }
    Ok(found)
}

fn dated_spellings(
    recording: &replay_recording::columnar_frames::Recording,
    run: u32,
    tokens: &BTreeSet<u128>,
    from: i32,
    to: i32,
) -> io::Result<HashMap<u128, Vec<(i32, String)>>> {
    let statuses = dashboard_query::cost::storage(|| lanes::statuses(recording, run, TableId::LexicalTokens, from, to))?;
    let mut spans: Vec<(i32, i32)> = Vec::new();
    for status in statuses.iter().filter(|status| replay_recording::columnar_frames::complete(status.status)) {
        match spans.last_mut() {
            Some(span) if span.1 + 1 >= status.first => span.1 = span.1.max(status.last),
            _ => spans.push((status.first, status.last)),
        }
    }
    let mut before = Vec::with_capacity(spans.len());
    let mut total = 0i64;
    for &(first, last) in &spans {
        before.push(total);
        total += i64::from(last - first) + 1;
    }
    let readable = Readable { spans, before };
    let Some(anchor) = readable.spans.last().map(|&(_, last)| last) else { return Ok(HashMap::new()) };
    let end = readable.hours() - 1;
    let last: HashMap<u128, String> = spelled_at(recording, run, &tokens.iter().map(|&token| (anchor, token)).collect::<Vec<_>>())?;
    let mut readings: HashMap<(u128, i32), String> = last.iter().map(|(&token, literal)| ((token, anchor), literal.clone())).collect();
    let first = first_reading(recording, run, &readable, last.keys().map(|&token| (token, 0, end, None)).collect(), &mut readings)?;
    let mut later = Vec::new();
    let mut out: HashMap<u128, Vec<(i32, String)>> = HashMap::new();
    for (&token, literal) in &last {
        let Some(&hour) = first.get(&token) else { continue };
        let earliest = readings.get(&(token, hour)).cloned().unwrap_or_else(|| literal.clone());
        if earliest != *literal {
            later.push((token, readable.index(hour) + 1, end, Some(literal.clone())));
        }
        out.insert(token, vec![(hour, earliest)]);
    }
    for (token, hour) in first_reading(recording, run, &readable, later, &mut readings)? {
        if let (Some(steps), Some(literal)) = (out.get_mut(&token), last.get(&token)) {
            steps.push((hour, literal.clone()));
        }
    }
    Ok(out)
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
    scope: Option<RaidScope>,
    icon: Option<(i32, Option<String>)>,
    place: Option<(i32, Option<u32>)>,
    state: Option<(i32, Option<u32>)>,
    phase: Option<i32>,
    launched_hour: Option<(i32, usize)>,
    ends: Vec<End>,
    seen_again: Option<i32>,
    victims: Vec<RaidVictimAt>,
    completion: Option<Option<u32>>,
    completion_end: Option<i32>,
    end_witnessed: bool,
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
    let mut images: HashMap<String, String> = HashMap::new();
    let mut labels: HashMap<&'static str, Option<String>> = HashMap::new();
    let tag_when = |raw: u32, hour: i32| -> io::Result<Option<String>> {
        let Some(held) = query.timeline.at(hour) else { return Ok(None) };
        let links = dashboard_base::queries::links(source, held.run)?;
        Ok(countries.tag_at(&links, raw, hour).map(str::to_owned))
    };
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
        let lexicon = dated_spellings(recording, run, &tokens, start, end)?;
        let spelled: Vec<i32> = lexicon.values().flatten().map(|&(hour, _)| hour).collect();
        let literal_at = |token: u32, hour: i32| -> Option<String> {
            let steps = lexicon.get(&u128::from(token))?;
            steps[..steps.partition_point(|&(first, _)| first <= hour)].last().map(|(_, literal)| literal.clone())
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
        let completion_victim = |at: i32, ended: i32, completion: Option<u32>| -> io::Result<RaidVictimAt> {
            Ok(match completion {
                Some(raw) => match tag_when(raw, ended)? {
                    Some(t) => RaidVictimAt { hour: at, tag: Some(t), missing: None, source: Some(RaidVictimSource::CompletionCountry) },
                    None => RaidVictimAt { hour: at, tag: None, missing: Some("The completion-time victim country holds no tag at the raid's hour".to_owned()), source: None },
                },
                None => RaidVictimAt { hour: at, tag: None, missing: Some("The raid's target resolved to no country at completion".to_owned()), source: None },
            })
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
                        scope: None,
                        icon: None,
                        place: None,
                        state: None,
                        phase: None,
                        launched_hour: None,
                        ends: Vec::new(),
                        seen_again: None,
                        victims: Vec::new(),
                        completion: None,
                        completion_end: None,
                        end_witnessed: false,
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
                if acc.scope.is_none() {
                    acc.scope.clone_from(&scope);
                }
                if let Some(known) = &acc.scope {
                    if acc.place.is_none() {
                        acc.place = Some((piece.first, province_of(known)));
                    }
                    if acc.state.is_none() {
                        let mut hours = BTreeSet::from([piece.first]);
                        for id in [known.province, known.leader_province, known.building_location].into_iter().filter(|&id| id != 0) {
                            step_hours_in(steps_of(&province_history, u128::from(id)), piece.first, piece.last, &mut hours);
                        }
                        acc.state = hours.into_iter().find_map(|hour| known.state_with(|id| place(&province_history, province_columns[0], id, hour)).ok().map(|state| (hour, state)));
                    }
                    if acc.icon.is_none() && acc.identity.kind.is_some() {
                        let template = if known.building_type == 0 { Some(None) } else { literal_at(known.building_type, piece.first).map(Some) };
                        acc.icon = template.map(|template| (piece.first, template));
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
                    if acc.completion.is_none() {
                        acc.completion = unit.completion;
                    }
                    if first_end && acc.completion.is_none() {
                        let victim = victim_at(scope.as_ref(), piece.first, true);
                        acc.end_witnessed = victim.tag.is_some();
                        push_victim(&mut acc.victims, victim);
                    }
                    if let Some(completion) = acc.completion
                        && acc.completion_end != Some(ended)
                    {
                        acc.completion_end = Some(ended);
                        let victim = completion_victim(piece.first, ended, completion)?;
                        let refused = completion.is_some() && victim.tag.is_none();
                        if !(refused && acc.end_witnessed) {
                            acc.end_witnessed |= victim.tag.is_some();
                            push_victim(&mut acc.victims, victim);
                        }
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
        if acc.last_hour < from {
            continue;
        }
        let bounds = date_bounds(&acc.runs, acc.raw_first, &complete_by_stretch);
        let first_exact = bounds.first().is_some_and(|&(exact, _)| exact);
        let raider_at = |hour: i32| acc.identity.raider.and_then(|slot| countries.slot_tag(slot, hour).map(str::to_owned));
        let mut label = |key: &'static str| labels.entry(key).or_insert_with(|| content.label(key)).clone();
        let runs: Vec<RaidPhaseRun> = acc
            .runs
            .iter()
            .zip(bounds)
            .map(|(run, (first_exact, after_hour))| RaidPhaseRun {
                first_hour: run.first,
                last_hour: run.last,
                first_exact,
                after_hour,
                raider: raider_at(run.first),
                phase: run.phase,
                phase_name: phase_label(run.phase, &mut label),
                outcome: run.outcome,
                outcome_name: outcome_label(run.outcome, &mut label),
            })
            .collect();
        let (launched_hour, launched_exact, launched_raider) = match acc.launched_hour {
            Some((h, stretch)) => (Some(h), exact_at(h, stretch, acc.raw_first, first_exact, &acc.runs, &complete_by_stretch), raider_at(h)),
            None => (None, false, None),
        };
        let ends: Vec<RaidEndAt> = acc
            .ends
            .iter()
            .map(|end| RaidEndAt {
                hour: end.hour,
                ended_hour: end.ended,
                exact: end.native || exact_at(end.ended, end.stretch, acc.raw_first, first_exact, &acc.runs, &complete_by_stretch),
                source: if end.native { RaidEndedSource::NativeEndDate } else { RaidEndedSource::ObservedEndedRead },
                raider: raider_at(end.ended),
            })
            .collect();
        let disappeared = acc.ends.is_empty().then(|| disappearance(acc.last_hour, acc.last_stretch, acc.seen_again, &complete_by_stretch)).flatten().filter(|&(hour, _)| hour <= to);
        let (icon, icon_missing) = match (&acc.identity.kind, &acc.icon) {
            (None, _) => (None, Some(TYPE_NOT_RECORDED.to_owned())),
            (Some(_), None) => (None, Some(TARGET_NOT_RECORDED.to_owned())),
            (Some(literal), Some((_, template))) => match content.icon(literal, template.as_deref()) {
                Ok((key, image)) => {
                    images.entry(key.clone()).or_insert(image);
                    (Some(key), None)
                }
                Err(reason) => (None, Some(reason)),
            },
        };
        let raid_type_name = acc.identity.kind.as_ref().and_then(|literal| content.label(&format!("raid_type_{literal}")));
        let (state_hour, state) = match acc.state {
            Some((hour, Some(state))) => (Some(hour), Some(state)),
            _ => (None, None),
        };
        let full = RaidEpisode {
            id,
            first_hour: acc.first_hour,
            first_hour_exact: first_exact,
            last_hour: acc.last_hour,
            raider: runs.first().and_then(|run| run.raider.clone()),
            raider_slot: acc.identity.raider,
            launched_raider,
            victim: None,
            victim_missing: None,
            victim_source: None,
            victims: acc.victims,
            raid_type_hour: acc.type_hour,
            raid_type: acc.identity.kind,
            raid_type_name,
            icon_hour: acc.icon.as_ref().map(|(hour, _)| *hour),
            icon,
            icon_missing,
            place_hour: acc.place.map(|(hour, _)| hour),
            province: acc.place.and_then(|(_, province)| province),
            state_hour,
            state,
            phase: None,
            phase_name: None,
            outcome: None,
            outcome_name: None,
            outcome_missing: None,
            succeeded_hour: acc.runs.iter().find(|run| run.phase == native::PHASE_ENDED && successful(run.outcome)).map(|run| run.first),
            runs,
            preparation: acc.preparation,
            launched: acc.launched_hour.is_some(),
            launched_hour,
            launched_exact,
            ends,
            disappeared_hour: disappeared.map(|(hour, _)| hour),
            disappeared_exact: disappeared.is_some_and(|(_, exact)| exact),
            member_hour: None,
        };
        let Some(mut clipped) = episode_from(&full, from) else { continue };
        if !chosen.is_empty() {
            let Some(hour) = member_hour(&clipped, chosen, &query.holders) else { continue };
            clipped.member_hour = Some(hour);
        }
        let Some(episode) = episode_at(&clipped, to) else { continue };
        if let Some(key) = &episode.icon
            && let Some(image) = images.get(key)
        {
            artwork.entry(key.clone()).or_insert_with(|| RaidArtwork { key: key.clone(), image: Some(image.clone()), missing: None });
        }
        keyed.push((episode.first_hour, acc.key, order, episode));
    }
    keyed.sort_by_key(|&(first_hour, key, order, _)| (first_hour, key, order));
    let out: Vec<RaidEpisode> = keyed.into_iter().map(|(_, _, _, episode)| episode).collect();
    warnings.extend(episode_warnings(&out));
    Ok((out, artwork))
}

pub fn episode_warnings(episodes: &[RaidEpisode]) -> Vec<String> {
    let mut out = Vec::new();
    if episodes.iter().any(|episode| episode.raider.is_none()) {
        out.push(RAIDERS_UNNAMED.to_owned());
    }
    if episodes.iter().flat_map(|episode| &episode.victims).any(|victim| matches!(victim.source, Some(RaidVictimSource::ControllerAtFirstEndedRead | RaidVictimSource::CaptorAtFirstEndedRead))) {
        out.push(ENDED_VICTIMS.to_owned());
    }
    let mut undrawn: BTreeMap<&str, u64> = BTreeMap::new();
    for reason in episodes.iter().filter_map(|episode| episode.icon_missing.as_deref()).filter(|&reason| reason != TYPE_NOT_RECORDED && reason != TARGET_NOT_RECORDED) {
        *undrawn.entry(reason).or_default() += 1;
    }
    out.extend(undrawn.into_iter().map(|(reason, count)| format!("{reason} (count: {count})")));
    out
}

fn member_hour(episode: &RaidEpisode, chosen: &[Country], holders: &Holders) -> Option<i32> {
    if episode.raider_slot.is_some_and(|slot| chosen.iter().any(|country| matches!(country, Country::Slot(chosen) if *chosen == slot))) {
        return Some(episode.first_hour);
    }
    let named = |tag: Option<&str>, at: i32| tag.is_some_and(|tag| chosen.iter().any(|country| holders.goes_by(country, tag, at)));
    let runs = episode.runs.iter().filter(|run| named(run.raider.as_deref(), run.first_hour)).map(|run| run.first_hour);
    let launched = episode.launched_hour.filter(|&hour| named(episode.launched_raider.as_deref(), hour));
    let ends = episode.ends.iter().filter(|end| named(end.raider.as_deref(), end.ended_hour)).map(|end| end.hour);
    let victims = episode.victims.iter().filter(|victim| named(victim.tag.as_deref(), victim.hour)).map(|victim| victim.hour);
    runs.chain(launched).chain(ends).chain(victims).min()
}

pub fn episode_from(episode: &RaidEpisode, from: i32) -> Option<RaidEpisode> {
    if episode.last_hour < from {
        return None;
    }
    let mut out = episode.clone();
    out.runs.retain(|run| run.last_hour >= from);
    let first = out.runs.first_mut()?;
    if first.first_hour < from {
        (first.first_hour, first.first_exact, first.after_hour) = (from, false, None);
    }
    out.raider = first.raider.clone();
    out.first_hour_exact = episode.first_hour >= from && episode.first_hour_exact;
    out.first_hour = episode.first_hour.max(from);
    out.preparation = episode
        .preparation
        .iter()
        .filter(|run| run.1 >= from)
        .map(|run| {
            let start = run.0.max(from);
            let value = (i64::from(run.2) + i64::from(start - run.0) * run.3) as i32;
            RaidPreparationRun(start, run.1, value, if start == run.1 { 0 } else { run.3 })
        })
        .collect();
    if out.launched_hour.is_some_and(|launched| launched < from) {
        (out.launched_hour, out.launched_exact, out.launched_raider) = (None, false, None);
    }
    let edge = out.ends.iter().rposition(|end| end.hour <= from).unwrap_or(0);
    out.ends.drain(..edge);
    out.victims.clear();
    let mut before: Option<RaidVictimAt> = None;
    for victim in &episode.victims {
        if victim.hour <= from {
            before = Some(RaidVictimAt { hour: from, ..victim.clone() });
        } else {
            if let Some(edge) = before.take() {
                push_victim(&mut out.victims, edge);
            }
            push_victim(&mut out.victims, victim.clone());
        }
    }
    if let Some(edge) = before {
        push_victim(&mut out.victims, edge);
    }
    for hour in [&mut out.raid_type_hour, &mut out.icon_hour, &mut out.place_hour, &mut out.state_hour].into_iter().flatten() {
        *hour = (*hour).max(from);
    }
    if out.disappeared_hour.is_some_and(|gone| gone < from) {
        (out.disappeared_hour, out.disappeared_exact) = (None, false);
    }
    Some(out)
}

pub fn episode_at(episode: &RaidEpisode, hour: i32) -> Option<RaidEpisode> {
    if episode.first_hour > hour || episode.member_hour.is_some_and(|member| member > hour) {
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
        (out.raid_type_hour, out.raid_type, out.raid_type_name, out.icon_hour, out.icon) = (None, None, None, None, None);
        out.icon_missing = Some(TYPE_NOT_RECORDED.to_owned());
    } else if out.icon_hour.is_none_or(|known| known > hour) {
        (out.icon_hour, out.icon) = (None, None);
        out.icon_missing = Some(TARGET_NOT_RECORDED.to_owned());
    }
    if out.place_hour.is_some_and(|known| known > hour) {
        (out.place_hour, out.province) = (None, None);
    }
    if out.state_hour.is_none_or(|known| known > hour) {
        (out.state_hour, out.state) = (None, None);
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

pub struct RaidTallyAt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum RaidTallyMetric {
    Both,
    Launched,
    Succeeded,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RaidTallyAtParams {
    pub branch_id: String,
    pub hour: i32,
    #[serde(default)]
    pub countries: Vec<Country>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub group: Option<GroupSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub top: Option<u32>,
    pub metric: RaidTallyMetric,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RaidTallyAtRow {
    pub country: Country,
    pub tag: String,
    pub launched: u32,
    pub succeeded: u32,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RaidTallyAtReply {
    pub branch_id: String,
    pub hour: i32,
    pub first_hour: i32,
    pub last_hour: i32,
    pub metric: RaidTallyMetric,
    pub available: Option<bool>,
    pub unavailable: Option<String>,
    pub rows: Vec<RaidTallyAtRow>,
    pub unattributed: u32,
    pub unattributed_launched: u32,
    pub unattributed_succeeded: u32,
    pub missing: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub partial: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(RaidTallyAtReply);

#[derive(Clone)]
struct TallyEvent {
    hour: i32,
    succeeded: bool,
    tag: String,
    slot: Option<u32>,
}

#[derive(Clone)]
struct CachedTallyBranch {
    first_hour: i32,
    last_hour: i32,
    available: Option<bool>,
    unavailable: Option<String>,
    events: Vec<TallyEvent>,
    unattributed_launched: Vec<i32>,
    unattributed_succeeded: Vec<i32>,
    unattributed_both: Vec<i32>,
    event_hours: Vec<i32>,
    missing: Vec<MissingSpan>,
    warnings: Vec<String>,
}

#[derive(Default)]
struct CachedRaidTallies(HashMap<(u64, bool), Arc<CachedTallyBranch>>);

fn load_tally_branch(source: &Source, query: &Query<'_>, branch_id: u64, context: &LoadContext) -> io::Result<Arc<CachedTallyBranch>> {
    let key = (branch_id, context.content.is_some());
    {
        let held = source.state::<CachedRaidTallies>();
        let guard = held.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(found) = guard.0.get(&key) {
            return Ok(Arc::clone(found));
        }
    }
    let from = query.timeline.first;
    let to = query.timeline.last;
    let content = Content::load(source, context);
    let mut warnings: BTreeSet<String> = content.warnings().into_iter().collect();
    let (episodes, _) = episodes(source, query, (from, to), &[], &content, &mut warnings)?;
    let missing_spans = missing(query, from, to)?;
    let none = content.raids.as_ref().is_ok_and(|raids| raids.types.is_empty() && raids.uncertain.is_empty());
    let available = if none { Some(false) } else { recorded(source, query, to)?.then_some(true) };
    let unavailable = none.then(|| "The content declares no raid type.".to_owned());

    let mut events = Vec::new();
    let mut unattributed_launched = Vec::new();
    let mut unattributed_succeeded = Vec::new();
    let mut unattributed_both = Vec::new();
    for episode in episodes {
        let mut unattributed_first: Option<i32> = None;
        if let Some(h) = episode.launched_hour {
            let tag = episode.launched_raider.as_ref().or(episode.raider.as_ref());
            if episode.raider_slot.is_some() || tag.is_some() {
                events.push(TallyEvent { hour: h, succeeded: false, tag: tag.cloned().unwrap_or_default(), slot: episode.raider_slot });
            } else {
                unattributed_launched.push(h);
                unattributed_first = Some(h);
            }
        }
        if let Some(h) = episode.succeeded_hour {
            let tag = episode.raider.as_ref().or(episode.launched_raider.as_ref());
            if episode.raider_slot.is_some() || tag.is_some() {
                events.push(TallyEvent { hour: h, succeeded: true, tag: tag.cloned().unwrap_or_default(), slot: episode.raider_slot });
            } else {
                unattributed_succeeded.push(h);
                unattributed_first = Some(unattributed_first.map_or(h, |prev| prev.min(h)));
            }
        }
        if let Some(h) = unattributed_first {
            unattributed_both.push(h);
        }
    }
    events.sort_by_key(|event| event.hour);
    unattributed_launched.sort_unstable();
    unattributed_succeeded.sort_unstable();
    unattributed_both.sort_unstable();
    let mut event_hours: Vec<i32> = events
        .iter()
        .map(|event| event.hour)
        .chain(unattributed_launched.iter().copied())
        .chain(unattributed_succeeded.iter().copied())
        .collect();
    event_hours.sort_unstable();
    event_hours.dedup();
    let built = Arc::new(CachedTallyBranch {
        first_hour: from,
        last_hour: to,
        available,
        unavailable,
        events,
        unattributed_launched,
        unattributed_succeeded,
        unattributed_both,
        event_hours,
        missing: missing_spans,
        warnings: warnings.into_iter().collect(),
    });
    let held = source.state::<CachedRaidTallies>();
    let mut guard = held.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.0.insert(key, Arc::clone(&built));
    Ok(built)
}

fn tally_interval(branch: &CachedTallyBranch, hour: i32) -> (i32, i32) {
    let mut first = branch.first_hour;
    let mut last = branch.last_hour;
    for gap in &branch.missing {
        if gap.last_hour < hour {
            first = first.max(gap.last_hour.saturating_add(1));
        } else if gap.first_hour > hour {
            last = last.min(gap.first_hour.saturating_sub(1));
        }
    }
    let idx = branch.event_hours.partition_point(|&h| h <= hour);
    if idx > 0 {
        first = first.max(branch.event_hours[idx - 1]);
    }
    if idx < branch.event_hours.len() {
        last = last.min(branch.event_hours[idx].saturating_sub(1));
    }
    (first.min(hour), last.max(hour))
}

pub(super) fn metric_unattributed(metric: RaidTallyMetric, launched: u32, succeeded: u32, both: u32) -> u32 {
    match metric {
        RaidTallyMetric::Launched => launched,
        RaidTallyMetric::Succeeded => succeeded,
        RaidTallyMetric::Both => both,
    }
}

impl Product for RaidTallyAt {
    const KEY: &'static str = "raid-tally-at";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = RaidTallyAtParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, hour: Read, countries, group, top, metric, "countries[]"];
    type Reply = RaidTallyAtReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![RaidTallyMetric::decl(), RaidTallyAtParams::decl(), RaidTallyAtRow::decl(), RaidTallyAtReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        validate(&params.branch_id, &params.countries)?;
        let selectors = usize::from(!params.countries.is_empty()) + usize::from(params.group.is_some()) + usize::from(params.top.is_some());
        if selectors > 1 {
            return Err("Chosen countries, a group or a top N, not more than one");
        }
        if params.top == Some(0) {
            return Err("top must be at least 1");
        }
        Ok(())
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let branch_id: u64 = params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch_id)?;
        let cached = load_tally_branch(source, &query, branch_id, context)?;
        let hour = params.hour;
        let unattributed_launched = cached.unattributed_launched.partition_point(|&h| h <= hour) as u32;
        let unattributed_succeeded = cached.unattributed_succeeded.partition_point(|&h| h <= hour) as u32;
        let unattributed_both = cached.unattributed_both.partition_point(|&h| h <= hour) as u32;
        let unattributed = metric_unattributed(params.metric, unattributed_launched, unattributed_succeeded, unattributed_both);
        if cached.available == Some(false) {
            return Ok(RaidTallyAtReply {
                branch_id: params.branch_id.clone(),
                hour,
                first_hour: cached.first_hour,
                last_hour: cached.last_hour,
                metric: params.metric,
                available: cached.available,
                unavailable: cached.unavailable.clone(),
                rows: Vec::new(),
                unattributed,
                unattributed_launched,
                unattributed_succeeded,
                missing: None,
                partial: None,
                warnings: cached.warnings.clone(),
            });
        }
        if let Some(gap) = cached.missing.iter().find(|gap| gap.first_hour <= hour && hour <= gap.last_hour) {
            return Ok(RaidTallyAtReply {
                branch_id: params.branch_id.clone(),
                hour,
                first_hour: gap.first_hour,
                last_hour: gap.last_hour,
                metric: params.metric,
                available: cached.available,
                unavailable: cached.unavailable.clone(),
                rows: Vec::new(),
                unattributed,
                unattributed_launched,
                unattributed_succeeded,
                missing: Some(gap.reason.clone()),
                partial: None,
                warnings: cached.warnings.clone(),
            });
        }
        let (mut first_hour, mut last_hour) = tally_interval(&cached, hour);
        let explicit_countries: Option<Vec<Country>> = if let Some(group) = &params.group {
            let groups = GroupsAt::load(source, context, &params.branch_id, Some(group))?;
            let members = groups.slots_at(group, &query.holders, hour)?;
            match &members.slots {
                Err(refused) => {
                    return Ok(RaidTallyAtReply {
                        branch_id: params.branch_id.clone(),
                        hour,
                        first_hour: first_hour.max(members.from).min(hour),
                        last_hour: last_hour.min(members.until).max(hour),
                        metric: params.metric,
                        available: cached.available,
                        unavailable: cached.unavailable.clone(),
                        rows: Vec::new(),
                        unattributed,
                        unattributed_launched,
                        unattributed_succeeded,
                        missing: Some(refused.clone()),
                        partial: None,
                        warnings: cached.warnings.clone(),
                    });
                }
                Ok(slots) => {
                    first_hour = first_hour.max(members.from);
                    last_hour = last_hour.min(members.until);
                    Some(slots.iter().copied().map(Country::Slot).collect())
                }
            }
        } else if !params.countries.is_empty() {
            Some(params.countries.clone())
        } else {
            None
        };
        if explicit_countries.is_some() {
            let (held_from, held_through) = query.holders.interval(hour);
            first_hour = first_hour.max(held_from);
            last_hour = last_hour.min(held_through);
        }

        if explicit_countries.is_none() && params.top.is_some() {
            if let Some(gap) = cached.missing.iter().find(|gap| gap.first_hour <= hour) {
                return Ok(RaidTallyAtReply {
                    branch_id: params.branch_id.clone(),
                    hour,
                    first_hour: first_hour.min(hour),
                    last_hour: last_hour.max(hour),
                    metric: params.metric,
                    available: cached.available,
                    unavailable: cached.unavailable.clone(),
                    rows: Vec::new(),
                    unattributed,
                    unattributed_launched,
                    unattributed_succeeded,
                    missing: Some(format!("Raid history is incomplete up to this date: {}", gap.reason)),
                    partial: None,
                    warnings: cached.warnings.clone(),
                });
            }
            if unattributed > 0 {
                return Ok(RaidTallyAtReply {
                    branch_id: params.branch_id.clone(),
                    hour,
                    first_hour: first_hour.min(hour),
                    last_hour: last_hour.max(hour),
                    metric: params.metric,
                    available: cached.available,
                    unavailable: cached.unavailable.clone(),
                    rows: Vec::new(),
                    unattributed,
                    unattributed_launched,
                    unattributed_succeeded,
                    missing: Some(format!("{unattributed} raid(s) up to this date have an unrecorded raider, so the top ranking is withheld")),
                    partial: None,
                    warnings: cached.warnings.clone(),
                });
            }
        }

        let rank_value = |launched: u32, succeeded: u32| match params.metric {
            RaidTallyMetric::Succeeded => succeeded,
            RaidTallyMetric::Both | RaidTallyMetric::Launched => launched,
        };
        let mut rows_out: Vec<RaidTallyAtRow> = if let Some(asked) = explicit_countries {
            let resolved: Vec<(Country, Option<u32>, String, Option<String>)> = asked
                .into_iter()
                .map(|country| {
                    let tag = query.holders.name(&country, hour);
                    let missing = query.holders.not_known(&country, hour);
                    let slot = match query.holders.at(&country, hour).map(|run| run.holder) {
                        Some(Holder::Found { slot, .. }) => Some(slot),
                        _ => match country {
                            Country::Slot(slot) => Some(slot),
                            Country::Tag(_) => None,
                        },
                    };
                    let out_country = slot.map_or(country, Country::Slot);
                    (out_country, slot, tag, missing)
                })
                .collect();
            let mut counts: Vec<(u32, u32)> = vec![(0, 0); resolved.len()];
            for event in cached.events.iter().take_while(|event| event.hour <= hour) {
                for (index, (country, slot, _, _)) in resolved.iter().enumerate() {
                    let matches = match (slot, event.slot) {
                        (Some(asked_slot), Some(event_slot)) => *asked_slot == event_slot,
                        (_, None) => query.holders.goes_by(country, &event.tag, event.hour),
                        _ => false,
                    };
                    if matches {
                        if event.succeeded {
                            counts[index].1 = counts[index].1.saturating_add(1);
                        } else {
                            counts[index].0 = counts[index].0.saturating_add(1);
                        }
                    }
                }
            }
            resolved
                .into_iter()
                .zip(counts)
                .map(|((country, _, tag, missing), (launched, succeeded))| RaidTallyAtRow {
                    country,
                    tag,
                    launched,
                    succeeded,
                    missing,
                })
                .collect()
        } else {
            #[derive(Hash, PartialEq, Eq)]
            enum RaidSubject {
                Slot(u32),
                Tag(String),
            }
            let mut by_subject: HashMap<RaidSubject, (u32, u32, String)> = HashMap::new();
            for event in cached.events.iter().take_while(|event| event.hour <= hour) {
                let key = match event.slot {
                    Some(slot) => RaidSubject::Slot(slot),
                    None => RaidSubject::Tag(event.tag.clone()),
                };
                let entry = by_subject.entry(key).or_insert_with(|| (0, 0, event.tag.clone()));
                if event.succeeded {
                    entry.1 = entry.1.saturating_add(1);
                } else {
                    entry.0 = entry.0.saturating_add(1);
                }
            }
            by_subject
                .into_iter()
                .filter(|&(_, (launched, succeeded, _))| {
                    if params.top.is_none() {
                        true
                    } else {
                        rank_value(launched, succeeded) > 0 || (params.metric == RaidTallyMetric::Both && succeeded > 0)
                    }
                })
                .map(|(subject, (launched, succeeded, fallback_tag))| {
                    let (country, tag) = match subject {
                        RaidSubject::Slot(slot) => {
                            let c = Country::Slot(slot);
                            let name = query.holders.name(&c, hour);
                            (c, if name.is_empty() { fallback_tag } else { name })
                        }
                        RaidSubject::Tag(tag) => (Country::from(tag.clone()), tag),
                    };
                    RaidTallyAtRow { country, tag, launched, succeeded, missing: None }
                })
                .collect()
        };
        rows_out.sort_unstable_by(|a, b| {
            rank_value(b.launched, b.succeeded)
                .cmp(&rank_value(a.launched, a.succeeded))
                .then_with(|| match params.metric {
                    RaidTallyMetric::Both => b.succeeded.cmp(&a.succeeded),
                    RaidTallyMetric::Launched | RaidTallyMetric::Succeeded => std::cmp::Ordering::Equal,
                })
                .then_with(|| a.country.cmp(&b.country))
                .then_with(|| a.tag.cmp(&b.tag))
        });
        if let Some(limit) = params.top {
            rows_out.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
        }
        let partial = if let Some(gap) = cached.missing.iter().find(|gap| gap.first_hour <= hour) {
            Some(format!("Raid history is incomplete up to this date: {}", gap.reason))
        } else if unattributed > 0 {
            Some(format!("{unattributed} raid(s) up to this date have an unrecorded raider"))
        } else {
            None
        };
        let reply = RaidTallyAtReply {
            branch_id: params.branch_id.clone(),
            hour,
            first_hour: first_hour.min(hour),
            last_hour: last_hour.max(hour),
            metric: params.metric,
            available: cached.available,
            unavailable: cached.unavailable.clone(),
            rows: rows_out,
            unattributed,
            unattributed_launched,
            unattributed_succeeded,
            missing: None,
            partial,
            warnings: cached.warnings.clone(),
        };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

pub mod tail;

#[cfg(test)]
mod tests;
