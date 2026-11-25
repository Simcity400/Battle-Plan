
use dashboard_base::products::{Country, regions::{ActiveRegions, NoRegions, automatic, no_regions}, validate_country};
use dashboard_base::products::{LoadContext, Product, at_hour};
use dashboard_world::products::{place_names, sea_regions::{self, SeaContent}, styles::LookResolver};
#[cfg(any(test, feature = "export-types"))]
use dashboard_world::products::sea_regions::Choice;
use dashboard_base::{Source, catalogue::Refusals, span::HourSpan};
use dashboard_query::{Query, coverage, identity::Holder};
use replay_lanes::{lanes, points::Cell};
use replay_data::rows::{AirCountryRelationRow, NavalRegionControlRow, TableId, tables};
use replay_derive::naval_region_control;
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[cfg(test)]
mod tests;

const DAY: i32 = 24;
const SCALE: i32 = 100_000;

const COLUMNS: [usize; 7] = [FRIENDLY, ENEMY, RATIO_FRIENDLY, RATIO_ENEMY, OWN, CONTROL_REFUSAL, SHARE_REFUSAL];
const FRIENDLY: usize = 0;
const ENEMY: usize = 1;
const RATIO_FRIENDLY: usize = 2;
const RATIO_ENEMY: usize = 3;
const OWN: usize = 4;
const CONTROL_REFUSAL: usize = 5;
const SHARE_REFUSAL: usize = 6;

pub struct NavalSupremacy;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NavalSupremacyParams {
    pub branch_id: String,
    pub country: Country,
    #[doc = " The window of days asked ([`super::windows`])."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub window: Option<u32>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub first_hour: Option<i32>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub last_hour: Option<i32>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub regions: Option<Vec<u32>>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub pinned_regions: Option<Vec<u32>>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub hidden_regions: Option<Vec<u32>>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub top: Option<usize>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SupremacyReadout {
    Control {
        control: i8,
        share: Option<String>,
        friendly: Option<String>,
        enemy: Option<String>,
        own: Option<String>,
        as_of_hour: i32,
    },
    NotRecorded {
        reason: String,
    },
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MissionShips {
    Counted { ships: i32 },
    NotRecorded { reason: String },
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SupremacyLegendItem {
    pub id: String,
    pub label: String,
    pub control: i8,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SupremacyRun {
    pub first_day: u32,
    pub last_day: u32,
    pub control: Option<i8>,
    pub friendly: Option<String>,
    pub enemy: Option<String>,
    pub share: Option<String>,
    pub own: Option<String>,
    #[doc = " Why the control is not recorded on these days, when the read did not reach the region."]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SupremacyRegion {
    pub id: String,
    pub name: String,
    pub pinned: bool,
    pub own_ships: MissionShips,
    pub enemy_ships: MissionShips,
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub readout: Option<SupremacyReadout>,
    pub runs: Vec<SupremacyRun>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct UnknownDays {
    pub first_day: u32,
    pub last_day: u32,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NavalSupremacyReply {
    pub first_hour: i32,
    pub last_hour: i32,
    #[doc = " The last hour through which nothing this reply shows changes: read beside the reply, never among its rows."]
    pub valid_through: i32,
    #[doc = " Day 0 starts at this hour (a multiple of `dayHours`); day `n` covers the `n`th day after."]
    pub first_day_hour: i32,
    pub day_hours: i32,
    pub days: u32,
    pub unknown: Vec<UnknownDays>,
    pub gaps: Vec<HourSpan>,
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub viewer_name: Option<String>,
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub viewer_name_missing: Option<String>,
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub viewer_color: Option<[u8; 3]>,
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub viewer_color_missing: Option<String>,
    pub legend: Vec<SupremacyLegendItem>,
    pub regions: Vec<SupremacyRegion>,
    #[doc = " Why the most active regions are not a proven rank at the asked hour: its day, or a region's score on it, is not recorded. A top then lists only the pinned regions."]
    pub rank_missing: Option<String>,
    #[doc = " Why no region is listed in the ranked view; set only when `regions` is empty."]
    pub empty: Option<NoRegions>,
    pub active_unlisted: ActiveRegions,
    pub scale: i32,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(NavalSupremacyReply);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Values {
    control: Option<i8>,
    friendly: Option<i64>,
    enemy: Option<i64>,
    share: Option<i64>,
    own: Option<i64>,
    missing: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Operands {
    friendly: Option<i64>,
    enemy: Option<i64>,
    ratio_friendly: Option<i64>,
    ratio_enemy: Option<i64>,
    own: Option<i64>,
    control_refusal: Option<String>,
    share_refusal: Option<String>,
}

impl Operands {
    const ZERO: Operands = Operands { friendly: Some(0), enemy: Some(0), ratio_friendly: Some(0), ratio_enemy: Some(0), own: Some(0), control_refusal: None, share_refusal: None };

    fn read(history: &lanes::History, cells: &[Cell]) -> Self {
        let at = |column: usize| history.columns.binary_search(&column).ok().map(|at| cells[at]);
        let number = |column: usize| match at(column) {
            Some(Cell::Number(value)) => Some(value),
            _ => None,
        };
        let text = |column: usize| at(column).and_then(|cell| history.text(cell)).filter(|text| !text.is_empty()).map(|text| String::from_utf8_lossy(text).into_owned());
        Self {
            friendly: number(FRIENDLY),
            enemy: number(ENEMY),
            ratio_friendly: number(RATIO_FRIENDLY),
            ratio_enemy: number(RATIO_ENEMY),
            own: number(OWN),
            control_refusal: text(CONTROL_REFUSAL),
            share_refusal: text(SHARE_REFUSAL),
        }
    }
}

fn region_values(operands: &Operands, minimum: &Result<i64, String>, ratio: &Result<i64, String>, notes: &mut Refusals, at: &str) -> Values {
    let mut values = Values { friendly: operands.friendly, enemy: operands.enemy, own: operands.own, ..Values::default() };
    let unrecorded = || "Naval region control's operands were not recorded".to_owned();
    let control = match &operands.control_refusal {
        Some(why) => Err(why.clone()),
        None => operands
            .friendly
            .zip(operands.enemy)
            .ok_or_else(unrecorded)
            .and_then(|(friendly, enemy)| minimum.clone().and_then(|minimum| ratio.clone().and_then(|ratio| naval_region_control::control(friendly, enemy, minimum, ratio)))),
    };
    match control {
        Ok(control) => {
            values.control = Some(match control {
                1 => 1,
                2 => -1,
                _ => 0,
            });
        }
        Err(why) => notes.note(why, at.to_owned()),
    }
    let share = match &operands.share_refusal {
        Some(why) => Err(why.clone()),
        None => operands
            .ratio_friendly
            .zip(operands.ratio_enemy)
            .ok_or_else(unrecorded)
            .and_then(|(friendly, enemy)| minimum.clone().and_then(|minimum| naval_region_control::share(friendly, enemy.max(minimum)))),
    };
    match share {
        Ok(share) => values.share = Some(share),
        Err(why) => notes.note(why, at.to_owned()),
    }
    if operands.own.is_none() {
        notes.note("The viewer's own current was not recorded".to_owned(), at.to_owned());
    }
    values
}

fn step_at(steps: &[lanes::Step], at: &mut usize, hour: i32) -> Option<usize> {
    while *at < steps.len() && steps[*at].hour <= hour {
        *at += 1;
    }
    at.checked_sub(1)
}

fn unread_at(history: &lanes::History, viewer: u32, region: u32, hour: i32) -> Option<String> {
    [viewer, NavalRegionControlRow::EVERY_VIEWER].into_iter().find_map(|holder| match coverage::reading_witness(TableId::NavalRegionControl, history.reading(u128::from(holder) << 32 | u128::from(region), hour)) {
        coverage::Witness::NotRecorded(reason) => Some(reason),
        _ => None,
    })
}

type Made = ((Option<Operands>, Option<String>), Values);

#[expect(clippy::too_many_arguments, reason = "one viewer's read: its days, the regions it covers, the sea content, the notes and each region's last state")]
fn region_runs(history: &lanes::History, viewer: u32, days: &[(u32, i32)], regions: &BTreeSet<u32>, sea: &SeaContent, notes: &mut Refusals, made: &mut HashMap<u32, Made>, out: &mut BTreeMap<u32, Vec<(u32, u32, Values)>>) -> BTreeSet<u32> {
    let none: &[lanes::Step] = &[];
    let mut own: HashMap<u32, &[lanes::Step]> = HashMap::new();
    let mut every: HashMap<u32, &[lanes::Step]> = HashMap::new();
    for (key, steps) in &history.keys {
        let (holder, region) = ((key >> 32) as u32, *key as u32);
        if holder == viewer {
            own.insert(region, steps);
        } else if holder == NavalRegionControlRow::EVERY_VIEWER {
            every.insert(region, steps);
        }
    }
    let mut all = regions.clone();
    all.extend(own.keys().chain(every.keys()).copied());
    for &region in &all {
        let minimum = sea.minimum(region);
        let (own, every) = (own.get(&region).copied().unwrap_or(none), every.get(&region).copied().unwrap_or(none));
        let (mut at_own, mut at_every) = (0, 0);
        let runs = out.entry(region).or_default();
        for &(day, hour) in days {
            let (at_own_step, at_every_step) = (step_at(own, &mut at_own, hour), step_at(every, &mut at_every, hour));
            let row = at_own_step.and_then(|at| own[at].cells.as_deref()).or_else(|| at_every_step.and_then(|at| every[at].cells.as_deref()));
            let unread = if row.is_some() { None } else { unread_at(history, viewer, region, hour) };
            let shown = (row.map(|cells| Operands::read(history, cells)), unread);
            let values = match made.get(&region) {
                Some((held, values)) if *held == shown => values.clone(),
                _ => {
                    let values = match &shown {
                        (_, Some(reason)) => Values { missing: Some(reason.clone()), ..Values::default() },
                        (operands, None) => region_values(operands.as_ref().unwrap_or(&Operands::ZERO), &minimum, &sea.ratio, notes, &format!("{hour}:{region}")),
                    };
                    made.insert(region, (shown, values.clone()));
                    values
                }
            };
            match runs.last_mut() {
                Some((_, last, held)) if *last + 1 == day && *held == values => *last = day,
                _ => runs.push((day, day, values)),
            }
        }
    }
    all
}

fn runs(pieces: BTreeMap<u32, Vec<(u32, u32, Values)>>) -> BTreeMap<u32, Vec<SupremacyRun>> {
    pieces
        .into_iter()
        .map(|(region, mut pieces)| {
            pieces.sort_unstable_by_key(|piece| piece.0);
            let mut joined: Vec<(u32, u32, Values)> = Vec::with_capacity(pieces.len());
            for piece in pieces {
                match joined.last_mut() {
                    Some((_, last, values)) if *last + 1 == piece.0 && *values == piece.2 => *last = piece.1,
                    _ => joined.push(piece),
                }
            }
            let text = |value: Option<i64>| value.map(|value| value.to_string());
            let runs = joined
                .into_iter()
                .map(|(first_day, last_day, values)| SupremacyRun { first_day, last_day, control: values.control, friendly: text(values.friendly), enemy: text(values.enemy), share: text(values.share), own: text(values.own), missing: values.missing })
                .collect();
            (region, runs)
        })
        .collect()
}

fn day_hours(timeline: &dashboard_query::timeline::Timeline, first: i32, last: i32) -> BTreeMap<u32, (u32, i32)> {
    let first_day = first.div_euclid(DAY);
    let mut out = BTreeMap::new();
    for stretch in timeline.within(first, last) {
        let mut start = stretch.first;
        while start <= stretch.last {
            let day = start.div_euclid(DAY);
            let hour = (day * DAY + DAY - 1).min(stretch.last);
            out.insert((day - first_day) as u32, (stretch.run, hour));
            start = hour + 1;
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MissionRegisteredShips {
    pub(crate) own: Result<BTreeMap<u32, i32>, String>,
    pub(crate) enemy: Result<BTreeMap<u32, i32>, String>,
}

impl MissionRegisteredShips {
    fn refused(reason: String) -> Self {
        Self { own: Err(reason.clone()), enemy: Err(reason) }
    }

    fn of(side: &Result<BTreeMap<u32, i32>, String>, region: u32) -> MissionShips {
        match side {
            Ok(counts) => MissionShips::Counted { ships: counts.get(&region).copied().unwrap_or(0) },
            Err(reason) => MissionShips::NotRecorded { reason: reason.clone() },
        }
    }
}

pub(crate) fn mission_registered_ships(frame: &DecodedFrame, viewer_slot: u32) -> MissionRegisteredShips {
    let shared = (|| -> Result<_, String> {
        let ships = frame.complete::<tables::Ships>().map_err(|e| format!("Ships table unavailable: {e}"))?;
        let ledgers = frame.complete::<tables::NavalMissionLedgers>().map_err(|e| format!("Naval mission ledgers unavailable: {e}"))?;
        let countries = frame.complete::<tables::Countries>().map_err(|e| format!("Countries table unavailable: {e}"))?;
        Ok((ships, ledgers, countries))
    })();
    let (ships, ledgers, countries) = match shared {
        Ok(tables) => tables,
        Err(reason) => return MissionRegisteredShips::refused(reason),
    };
    let mut tf_ships: HashMap<u64, i32> = HashMap::new();
    for ship in ships {
        *tf_ships.entry(ship.task_force).or_insert(0) += 1;
    }
    let mut by_owner_region: HashMap<(u32, u32), i32> = HashMap::new();
    let mut regions = BTreeSet::new();
    for row in ledgers {
        regions.insert(row.region);
        let count = row
            .task_forces
            .iter()
            .filter(|&&tf| tf != 0)
            .fold(0_i32, |sum, tf| sum.wrapping_add(tf_ships.get(tf).copied().unwrap_or(0)));
        by_owner_region.insert((row.owner, row.region), count);
    }
    let slot_to_raw: HashMap<u32, u32> = countries.iter().map(|c| (c.canonical_slot, c.id)).collect();
    let own_raw = slot_to_raw.get(&viewer_slot).copied();
    let own = regions.iter().map(|&region| (region, own_raw.and_then(|id| by_owner_region.get(&(id, region)).copied()).unwrap_or(0))).collect();
    let enemy = (|| -> Result<BTreeMap<u32, i32>, String> {
        let links_rows = frame.complete::<tables::CountryLinks>().map_err(|e| format!("Country links unavailable: {e}"))?;
        let enemy_raws = naval_region_control::raw_roster(frame, viewer_slot, AirCountryRelationRow::ENEMY)?;
        let raw_to_slot: HashMap<u32, u32> = links_rows.iter().map(|l| (l.id, l.canonical_slot)).collect();
        let mut enemy_raw_ids: Vec<u32> = Vec::with_capacity(enemy_raws.len());
        for raw in enemy_raws {
            let slot = if raw == 0 { 0 } else { *raw_to_slot.get(&raw).ok_or_else(|| format!("Enemy {raw} has no CountryLinks row"))? };
            if let Some(&id) = slot_to_raw.get(&slot) {
                enemy_raw_ids.push(id);
            }
        }
        Ok(regions.iter().map(|&region| (region, enemy_raw_ids.iter().fold(0_i32, |sum, &id| sum.wrapping_add(by_owner_region.get(&(id, region)).copied().unwrap_or(0))))).collect())
    })();
    MissionRegisteredShips { own: Ok(own), enemy }
}

fn supremacy_legend(regions: &[SupremacyRegion], viewer_label: &str) -> Vec<SupremacyLegendItem> {
    let mut seen = BTreeSet::new();
    for region in regions {
        for run in &region.runs {
            if let Some(control) = run.control {
                seen.insert(control);
            }
        }
    }
    let mut out = Vec::new();
    if seen.contains(&1) {
        out.push(SupremacyLegendItem { id: "friendly".into(), label: format!("{viewer_label} and friends"), control: 1 });
    }
    if seen.contains(&-1) {
        out.push(SupremacyLegendItem { id: "enemy".into(), label: format!("Enemies of {viewer_label}"), control: -1 });
    }
    if seen.contains(&0) {
        out.push(SupremacyLegendItem { id: "neither".into(), label: "Contested".into(), control: 0 });
    }
    out
}

struct Control {
    runs: BTreeMap<u32, Vec<SupremacyRun>>,
    unknown: Vec<UnknownDays>,
    hours: BTreeMap<u32, (u32, i32)>,
    notes: Refusals,
}

#[derive(Clone)]
struct Segment {
    run: u32,
    viewer: u32,
    from: i32,
    to: i32,
    shown: Vec<(u32, i32)>,
    regions: BTreeSet<u32>,
}

#[derive(Clone, Default)]
struct NavalHistory {
    last: i32,
    pieces: BTreeMap<u32, Vec<(u32, u32, Values)>>,
    known: BTreeSet<u32>,
    refused: BTreeMap<u32, String>,
    hours: BTreeMap<u32, (u32, i32)>,
    segments: Vec<Segment>,
    open: bool,
    made: HashMap<u32, Made>,
    notes: Refusals,
    used: u64,
}

impl NavalHistory {
    fn read(&mut self, query: &Query<'_>, sea: &SeaContent, country: &Country, first_day: i32, (first, last): (i32, i32), cancelled: &AtomicBool) -> io::Result<()> {
        let recording = query.replay.recording();
        let regions: BTreeSet<u32> = sea.sea.clone().unwrap_or_default();
        let every = u128::from(NavalRegionControlRow::EVERY_VIEWER) << 32;
        let ranges = |viewer: u32, only: Option<&BTreeSet<u32>>| -> Vec<(u128, u128)> {
            let own = u128::from(viewer) << 32;
            match only {
                None => vec![(own, own | u128::from(u32::MAX)), (every, every | u128::from(u32::MAX))],
                Some(only) => only.iter().flat_map(|&region| [(own | u128::from(region), own | u128::from(region)), (every | u128::from(region), every | u128::from(region))]).collect(),
            }
        };
        let mut segment: Vec<(u32, i32)> = Vec::new();
        let mut held: Option<(u32, u32)> = self.segments.last().filter(|_| self.open).map(|segment| (segment.run, segment.viewer));
        let hours = day_hours(&query.timeline, first, last);
        let first_held_day = (first.div_euclid(DAY) - first_day) as u32;
        let flush = |history: &mut Self, held: Option<(u32, u32)>, segment: &mut Vec<(u32, i32)>| -> io::Result<()> {
            let (Some((run, viewer)), Some(&(_, from)), Some(&(_, to))) = (held, segment.first(), segment.last()) else { return Ok(()) };
            dashboard_text::metadata::check_cancel(cancelled)?;
            let read = dashboard_query::cost::storage(|| lanes::history(recording, run, TableId::NavalRegionControl, &COLUMNS, lanes::Keys::Ranges(&ranges(viewer, None)), from, to))?;
            let available = |hour: i32| read.statuses.iter().any(|status| status.first <= hour && hour <= status.last && replay_recording::columnar_frames::available(status.status));
            let (shown, unavailable): (Vec<(u32, i32)>, Vec<(u32, i32)>) = segment.drain(..).partition(|&(_, hour)| available(hour));
            history.refused.extend(unavailable.into_iter().map(|(day, _)| (day, "Naval region control was not available on this day".to_owned())));
            history.known.extend(shown.iter().map(|(day, _)| *day));
            let continues = history.open && history.segments.last().is_some_and(|held| (held.run, held.viewer) == (run, viewer));
            if !continues {
                history.made.clear();
                history.segments.push(Segment { run, viewer, from, to, shown: Vec::new(), regions: regions.clone() });
            }
            let mut covered = history.segments.last().map(|segment| segment.regions.clone()).unwrap_or_default();
            let fresh: BTreeSet<u32> = read.keys.iter().map(|(key, _)| *key as u32).filter(|region| !covered.contains(region)).collect();
            if continues && !fresh.is_empty()
                && let Some(earlier) = history.segments.last().cloned()
            {
                let back = dashboard_query::cost::storage(|| lanes::history(recording, run, TableId::NavalRegionControl, &COLUMNS, lanes::Keys::Ranges(&ranges(viewer, Some(&fresh))), earlier.from, earlier.to))?;
                let mut made = HashMap::new();
                region_runs(&back, viewer, &earlier.shown, &fresh, sea, &mut history.notes, &mut made, &mut history.pieces);
                history.made.extend(made);
            }
            covered.extend(region_runs(&read, viewer, &shown, &covered, sea, &mut history.notes, &mut history.made, &mut history.pieces));
            if let Some(held) = history.segments.last_mut() {
                held.to = to;
                held.shown.extend(shown);
                held.regions = covered;
            }
            history.open = true;
            Ok(())
        };
        for (day, (run, hour)) in &hours {
            let day = first_held_day + day;
            self.hours.insert(day, (*run, *hour));
            match query.holders.at(country, *hour).map(|run| run.holder) {
                Some(Holder::Found { slot, .. }) => {
                    if held != Some((*run, slot)) {
                        flush(self, held, &mut segment)?;
                        if held.is_some() {
                            self.open = false;
                        }
                        held = Some((*run, slot));
                    }
                    segment.push((day, *hour));
                }
                _ => {
                    self.refused.insert(day, "The viewer's tag is held by no country on this day".to_owned());
                }
            }
        }
        flush(self, held, &mut segment)?;
        self.last = last;
        Ok(())
    }

    fn control(&self, query: &Query<'_>, first_day: i32, count: u32) -> Control {
        let mut unknown: Vec<UnknownDays> = Vec::new();
        let gap = |day: u32, reason: String, unknown: &mut Vec<UnknownDays>| match unknown.last_mut() {
            Some(span) if span.last_day + 1 == day && span.reason == reason => span.last_day = day,
            _ => unknown.push(UnknownDays { first_day: day, last_day: day, reason }),
        };
        for day in 0..count {
            if self.known.contains(&day) {
                continue;
            }
            match self.refused.get(&day) {
                Some(reason) => gap(day, reason.clone(), &mut unknown),
                None => {
                    let hour = (first_day + day as i32) * DAY;
                    let reason = query.timeline.unheld.iter().find(|unheld| unheld.first <= hour + DAY - 1 && hour <= unheld.last).map_or_else(|| "The day was not recorded".to_owned(), |unheld| unheld.reason.clone());
                    gap(day, reason, &mut unknown);
                }
            }
        }
        Control { runs: runs(self.pieces.clone()), unknown, hours: self.hours.clone(), notes: self.notes.clone() }
    }
}

#[derive(Default)]
struct HeldHistories {
    histories: HashMap<String, NavalHistory>,
    uses: u64,
}

const HELD_HISTORIES: usize = 16;

#[expect(clippy::too_many_arguments, reason = "the history's key: the viewer, its first day and the content, then the span asked")]
fn control(source: &Source, load: &LoadContext, query: &Query<'_>, sea: &SeaContent, country: &Country, (first, last): (i32, i32), count: u32, cancelled: &AtomicBool) -> io::Result<Control> {
    let first_day = first.div_euclid(DAY);
    if count == 0 {
        return Ok(NavalHistory::default().control(query, first_day, 0));
    }
    let content = load.content.as_ref().map(|options| (dashboard_base::content::content_key(Some(options)), options.presentation.identity().clone()));
    let key = format!("{}|{country:?}|{first}|{content:?}", query.timeline.id);
    let state = source.state::<HeldHistories>();
    let held = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).histories.get(&key).filter(|held| held.last == last || (held.last < last && (held.last + 1).rem_euclid(DAY) == 0)).cloned();
    let history = match held {
        Some(held) if held.last == last => held,
        Some(mut held) => {
            held.read(query, sea, country, first_day, (held.last + 1, last), cancelled)?;
            held
        }
        None => {
            let mut history = NavalHistory::default();
            history.read(query, sea, country, first_day, (first, last), cancelled)?;
            history
        }
    };
    let control = history.control(query, first_day, count);
    let mut state = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    state.uses += 1;
    let used = state.uses;
    if state.histories.len() >= HELD_HISTORIES
        && !state.histories.contains_key(&key)
        && let Some(oldest) = state.histories.iter().min_by_key(|(_, held)| held.used).map(|(key, _)| key.clone())
    {
        state.histories.remove(&oldest);
    }
    state.histories.insert(key, NavalHistory { used, ..history });
    Ok(control)
}

fn ranked(at_hour: impl Iterator<Item = (u32, Option<i128>)>) -> Result<Vec<u32>, String> {
    let mut order: Vec<(u32, i128)> = Vec::new();
    for (id, score) in at_hour {
        match score {
            None => return Err("A region's naval scores are not recorded on this day".to_owned()),
            Some(score) if score > 0 => order.push((id, score)),
            Some(_) => {}
        }
    }
    order.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    Ok(order.into_iter().map(|(id, _)| id).collect())
}

fn activity_on(runs: &[SupremacyRun], day: u32) -> Option<i128> {
    let Some(run) = runs.iter().find(|run| run.first_day <= day && day <= run.last_day) else { return Some(0) };
    let score = |value: &Option<String>| value.as_deref().and_then(|s| s.parse::<i128>().ok()).map(|score| score.max(0));
    Some(score(&run.friendly)? + score(&run.enemy)?)
}

impl Product for NavalSupremacy {
    const KEY: &'static str = "naval-supremacy";
    const CONTENT_DEPENDENT: bool = true;
    type Params = NavalSupremacyParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, window, firstHour: Start, lastHour: End, regions, pinnedRegions, hiddenRegions, top, "regions[]", "pinnedRegions[]", "hiddenRegions[]"];
    type Reply = NavalSupremacyReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        match (params.first_hour, params.last_hour, params.window) {
            (Some(a), Some(b), _) => {
                if a > b {
                    return Err("Invalid naval supremacy hours");
                }
            }
            (None, None, Some(window)) => {
                if dashboard_world::products::windows::hours(window).is_none() {
                    return Err("Invalid naval supremacy window");
                }
            }
            _ => return Err("Naval supremacy requires firstHour/lastHour or window"),
        }
        validate_country(&params.country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            NavalSupremacyParams::decl(),
            SupremacyRun::decl(),
            SupremacyRegion::decl(),
            SupremacyReadout::decl(),
            MissionShips::decl(),
            SupremacyLegendItem::decl(),
            UnknownDays::decl(),
            Choice::decl(),
            NoRegions::decl(),
            ActiveRegions::decl(),
            HourSpan::decl(),
            NavalSupremacyReply::decl(),
        ]
    }

    fn load(source: &Source, params: &Self::Params, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let places = place_names::Content::read(source, load)?;
        let sea = SeaContent::read(source, load)?;
        let (first, last, outside) = match (params.first_hour, params.last_hour, params.window) {
            (Some(a), Some(b), _) => {
                let f = a.max(query.timeline.first);
                let l = b.min(query.timeline.last);
                if f > l { (a, b, true) } else { (f, l, false) }
            }
            (_, _, Some(window)) => {
                let (f, l) = dashboard_world::products::windows::within(&query, window)?;
                let out = f > l;
                let (f, l) = if out { dashboard_world::products::windows::hours(window).unwrap_or((f, l)) } else { (f, l) };
                (f, l, out)
            }
            _ => return Err(dashboard_core::invalid("Naval supremacy requires firstHour/lastHour or window")),
        };
        let first_day = first.div_euclid(DAY);
        let count = if outside { 0 } else { (last.div_euclid(DAY) - first_day + 1) as u32 };
        let Control { mut runs, unknown, hours, notes } = control(source, load, &query, &sea, &params.country, (first, last), count, cancelled)?;
        let mut warnings = sea.notes.clone();
        warnings.extend(places.warnings());
        let mut looks = LookResolver::new(source, &query.timeline, load, &mut warnings)?;
        let look_hour = count.checked_sub(1).and_then(|idx| hours.get(&idx).map(|&(_, hour)| hour)).unwrap_or(last);
        let viewer_tag = query.holders.name(&params.country, look_hour);
        let viewer_look = looks.at(&viewer_tag, look_hour)?;
        let viewer_label = viewer_look.name.clone().unwrap_or_else(|| viewer_tag.clone());
        let mut names = Refusals::default();
        let all_choices = sea_regions::choices(&places, &sea, runs.keys().copied().chain(params.regions.iter().flatten().copied()).chain(params.pinned_regions.iter().flatten().copied()), &mut names);
        let name_map: HashMap<u32, String> = all_choices.iter().filter_map(|c| Some((c.id.parse::<u32>().ok()?, c.name.clone()))).collect();
        let last_day_index = count.checked_sub(1);
        let last_day_hour = last_day_index.and_then(|idx| hours.get(&idx).map(|&(_, h)| h));
        let last_day_unknown = last_day_index.and_then(|idx| unknown.iter().find(|u| u.first_day <= idx && idx <= u.last_day).map(|u| u.reason.clone()));
        let ships_at_end = match (outside, last_day_hour) {
            (true, _) => MissionRegisteredShips::refused("Outside the recording".into()),
            (false, None) => MissionRegisteredShips::refused(last_day_unknown.clone().unwrap_or_else(|| "The day was not recorded".into())),
            (false, Some(ship_hour)) => match query.holders.at(&params.country, ship_hour).map(|r| r.holder) {
                Some(Holder::Found { slot, .. }) => {
                    let reads = [
                        at_hour::Read::all(TableId::Ships),
                        at_hour::Read::all(TableId::NavalMissionLedgers),
                        at_hour::Read::all(TableId::Countries),
                        at_hour::Read::all(TableId::CountryLinks),
                        at_hour::Read::all(TableId::AirCountryRelations),
                    ];
                    match at_hour::frame(&query, ship_hour, &reads, "Naval mission ships") {
                        Ok(frame) => mission_registered_ships(&frame, slot),
                        Err(error) => MissionRegisteredShips::refused(error.to_string()),
                    }
                }
                _ => MissionRegisteredShips::refused("The viewer's tag is held by no country at this hour".into()),
            },
        };
        let make_region = |id: u32, pinned: bool, runs_vec: Vec<SupremacyRun>| -> SupremacyRegion {
            let name = name_map.get(&id).cloned().unwrap_or_else(|| format!("Region {id} (name unavailable)"));
            let own_ships = MissionRegisteredShips::of(&ships_at_end.own, id);
            let enemy_ships = MissionRegisteredShips::of(&ships_at_end.enemy, id);
            let readout = if let Some(reason) = &last_day_unknown {
                Some(SupremacyReadout::NotRecorded { reason: reason.clone() })
            } else if let (Some(idx), Some(as_of_hour)) = (last_day_index, last_day_hour) {
                match runs_vec.iter().find(|r| r.first_day <= idx && idx <= r.last_day) {
                    Some(run) => match run.control {
                        Some(control) => Some(SupremacyReadout::Control {
                            control,
                            share: run.share.clone(),
                            friendly: run.friendly.clone(),
                            enemy: run.enemy.clone(),
                            own: run.own.clone(),
                            as_of_hour,
                        }),
                        None => Some(SupremacyReadout::NotRecorded { reason: run.missing.clone().unwrap_or_else(|| "Naval region control was not recorded".into()) }),
                    },
                    None => Some(SupremacyReadout::NotRecorded { reason: "Naval region control was not recorded".into() }),
                }
            } else {
                None
            };
            SupremacyRegion {
                id: id.to_string(),
                name,
                pinned,
                own_ships,
                enemy_ships,
                readout,
                runs: runs_vec,
            }
        };
        let mut rank_missing = None;
        let mut ranked_view = false;
        let mut hidden_active = 0u32;
        let mut active_unlisted = ActiveRegions::default();
        let selected_regions: Vec<SupremacyRegion> = if let Some(explicit) = &params.regions {
            explicit.iter().map(|&id| make_region(id, true, runs.remove(&id).unwrap_or_default())).collect()
        } else if params.top.is_some() || params.pinned_regions.is_some() || params.hidden_regions.is_some() {
            let mut seen = HashSet::new();
            let pinned_list: Vec<u32> = params.pinned_regions.iter().flatten().copied().filter(|id| seen.insert(*id)).collect();
            let hidden: HashSet<u32> = params.hidden_regions.iter().flatten().copied().collect();
            ranked_view = true;
            let scored = match (&last_day_unknown, last_day_index) {
                (Some(reason), _) => Err(reason.clone()),
                (None, Some(day)) => ranked(runs.iter().filter(|(id, _)| !seen.contains(id)).map(|(&id, region_runs)| (id, activity_on(region_runs, day)))),
                (None, None) => Ok(Vec::new()),
            };
            let scored = scored.unwrap_or_else(|reason| {
                rank_missing = Some(reason);
                Vec::new()
            });
            let chosen = automatic(scored, &hidden, params.top.unwrap_or(usize::MAX), rank_missing.clone());
            hidden_active = chosen.hidden;
            active_unlisted = chosen.active_unlisted;
            let scored = chosen.listed;
            let mut out = Vec::with_capacity(pinned_list.len() + scored.len());
            for id in pinned_list {
                out.push(make_region(id, true, runs.remove(&id).unwrap_or_default()));
            }
            for id in scored {
                out.push(make_region(id, false, runs.remove(&id).unwrap_or_default()));
            }
            out
        } else {
            all_choices
                .into_iter()
                .map(|choice| {
                    let id: u32 = choice.id.parse().unwrap_or_default();
                    make_region(id, false, runs.remove(&id).unwrap_or_default())
                })
                .collect()
        };
        let legend = supremacy_legend(&selected_regions, &viewer_label);
        let gaps: Vec<HourSpan> = query
            .timeline
            .unheld
            .iter()
            .filter_map(|gap| {
                let a = gap.first.max(first);
                let b = gap.last.min(last);
                (a <= b).then_some(HourSpan { first_hour: a, last_hour: b })
            })
            .collect();
        warnings.extend(names.lines("sea region names"));
        warnings.extend(notes.lines("naval supremacy fields"));
        let reply = NavalSupremacyReply {
            first_hour: first,
            last_hour: last,
            valid_through: last,
            first_day_hour: first_day * DAY,
            day_hours: DAY,
            days: count,
            unknown,
            gaps,
            viewer_name: viewer_look.name,
            viewer_name_missing: viewer_look.name_missing,
            viewer_color: viewer_look.color,
            viewer_color_missing: viewer_look.color_missing,
            legend,
            empty: no_regions(!ranked_view || !selected_regions.is_empty(), rank_missing.as_deref(), hidden_active, 0),
            regions: selected_regions,
            rank_missing,
            active_unlisted,
            scale: SCALE,
            warnings,
        };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}
