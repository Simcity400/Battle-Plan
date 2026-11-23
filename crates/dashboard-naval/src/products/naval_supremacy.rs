
use dashboard_base::products::{Country, validate_country};
use dashboard_base::products::{LoadContext, Product};
use dashboard_world::products::{place_names, sea_regions::{self, SeaContent}};
#[cfg(any(test, feature = "export-types"))]
use dashboard_world::products::sea_regions::Choice;
use dashboard_base::{Source, catalogue::Refusals};
use dashboard_query::identity::Holder;
use replay_lanes::{lanes, points::Cell};
use replay_data::rows::{NavalRegionControlRow, TableId};
use replay_derive::naval_region_control;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
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
    pub window: u32,
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
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SupremacyRegion {
    pub id: String,
    pub name: String,
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
    #[doc = " Day 0 starts at this hour (a multiple of `dayHours`); day `n` covers the `n`th day after."]
    pub first_day_hour: i32,
    pub day_hours: i32,
    pub days: u32,
    pub unknown: Vec<UnknownDays>,
    pub regions: Vec<SupremacyRegion>,
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

fn region_runs(history: &lanes::History, viewer: u32, days: &[(u32, i32)], regions: &BTreeSet<u32>, sea: &SeaContent, notes: &mut Refusals, out: &mut BTreeMap<u32, Vec<(u32, u32, Values)>>) {
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
    for region in all {
        let minimum = sea.minimum(region);
        let (own, every) = (own.get(&region).copied().unwrap_or(none), every.get(&region).copied().unwrap_or(none));
        let (mut at_own, mut at_every) = (0, 0);
        let mut made: Option<((Option<usize>, Option<usize>), Values)> = None;
        let runs = out.entry(region).or_default();
        for &(day, hour) in days {
            let shown = (step_at(own, &mut at_own, hour), step_at(every, &mut at_every, hour));
            let values = match &made {
                Some((held, values)) if *held == shown => values.clone(),
                _ => {
                    let row = shown.0.and_then(|at| own[at].cells.as_deref()).or_else(|| shown.1.and_then(|at| every[at].cells.as_deref()));
                    let operands = row.map_or(Operands::ZERO, |cells| Operands::read(history, cells));
                    let values = region_values(&operands, &minimum, &sea.ratio, notes, &format!("{hour}:{region}"));
                    made = Some((shown, values.clone()));
                    values
                }
            };
            match runs.last_mut() {
                Some((_, last, held)) if *last + 1 == day && *held == values => *last = day,
                _ => runs.push((day, day, values)),
            }
        }
    }
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
                .map(|(first_day, last_day, values)| SupremacyRun { first_day, last_day, control: values.control, friendly: text(values.friendly), enemy: text(values.enemy), share: text(values.share), own: text(values.own) })
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

impl Product for NavalSupremacy {
    const KEY: &'static str = "naval-supremacy";
    const CONTENT_DEPENDENT: bool = true;
    type Params = NavalSupremacyParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, window];
    type Reply = NavalSupremacyReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        if dashboard_world::products::windows::hours(params.window).is_none() {
            return Err("Invalid naval supremacy window");
        }
        validate_country(&params.country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![NavalSupremacyParams::decl(), SupremacyRun::decl(), SupremacyRegion::decl(), UnknownDays::decl(), Choice::decl(), NavalSupremacyReply::decl()]
    }

    fn load(source: &Source, params: &Self::Params, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let places = place_names::Content::read(source, load)?;
        let sea = SeaContent::read(source, load)?;
        let (first, last) = dashboard_world::products::windows::within(&query, params.window)?;
        let outside = first > last;
        let (first, last) = if outside { dashboard_world::products::windows::hours(params.window).unwrap_or((first, last)) } else { (first, last) };
        let first_day = first.div_euclid(DAY);
        let count = if outside { 0 } else { (last.div_euclid(DAY) - first_day + 1) as u32 };
        let hours = day_hours(&query.timeline, first, last);
        let regions: BTreeSet<u32> = sea.sea.clone().unwrap_or_default();
        let recording = query.replay.recording();
        let mut notes = Refusals::default();
        let mut pieces: BTreeMap<u32, Vec<(u32, u32, Values)>> = BTreeMap::new();
        let mut known: BTreeSet<u32> = BTreeSet::new();
        let mut refused: BTreeMap<u32, String> = BTreeMap::new();
        let mut segment: Vec<(u32, i32)> = Vec::new();
        let mut held: Option<(u32, u32)> = None;
        let flush = |held: Option<(u32, u32)>, segment: &mut Vec<(u32, i32)>, pieces: &mut BTreeMap<u32, Vec<(u32, u32, Values)>>, known: &mut BTreeSet<u32>, refused: &mut BTreeMap<u32, String>, notes: &mut Refusals| -> io::Result<()> {
            let (Some((run, viewer)), Some(&(_, from)), Some(&(_, to))) = (held, segment.first(), segment.last()) else { return Ok(()) };
            dashboard_text::metadata::check_cancel(cancelled)?;
            let every = u128::from(NavalRegionControlRow::EVERY_VIEWER) << 32;
            let ranges = [(u128::from(viewer) << 32, u128::from(viewer) << 32 | u128::from(u32::MAX)), (every, every | u128::from(u32::MAX))];
            let history = dashboard_query::cost::storage(|| lanes::history(recording, run, TableId::NavalRegionControl, &COLUMNS, lanes::Keys::Ranges(&ranges), from, to))?;
            let statuses = dashboard_query::cost::storage(|| lanes::statuses(recording, run, TableId::NavalRegionControl, from, to))?;
            let available = |hour: i32| statuses.iter().any(|status| status.first <= hour && hour <= status.last && replay_recording::columnar_frames::available(status.status));
            let (shown, unavailable): (Vec<(u32, i32)>, Vec<(u32, i32)>) = segment.drain(..).partition(|&(_, hour)| available(hour));
            refused.extend(unavailable.into_iter().map(|(day, _)| (day, "Naval region control was not available on this day".to_owned())));
            known.extend(shown.iter().map(|(day, _)| *day));
            region_runs(&history, viewer, &shown, &regions, &sea, notes, pieces);
            Ok(())
        };
        for (day, (run, hour)) in hours {
            match query.holders.at(&params.country, hour).map(|run| run.holder) {
                Some(Holder::Found { slot, .. }) => {
                    if held != Some((run, slot)) {
                        flush(held, &mut segment, &mut pieces, &mut known, &mut refused, &mut notes)?;
                        held = Some((run, slot));
                    }
                    segment.push((day, hour));
                }
                _ => {
                    refused.insert(day, "The viewer's tag is held by no country on this day".to_owned());
                }
            }
        }
        flush(held, &mut segment, &mut pieces, &mut known, &mut refused, &mut notes)?;
        let mut unknown: Vec<UnknownDays> = Vec::new();
        let gap = |day: u32, reason: String, unknown: &mut Vec<UnknownDays>| match unknown.last_mut() {
            Some(span) if span.last_day + 1 == day && span.reason == reason => span.last_day = day,
            _ => unknown.push(UnknownDays { first_day: day, last_day: day, reason }),
        };
        for day in 0..count {
            if known.contains(&day) {
                continue;
            }
            match refused.remove(&day) {
                Some(reason) => gap(day, reason, &mut unknown),
                None => {
                    let hour = (first_day + day as i32) * DAY;
                    let reason = query.timeline.unheld.iter().find(|unheld| unheld.first <= hour + DAY - 1 && hour <= unheld.last).map_or_else(|| "The day was not recorded".to_owned(), |unheld| unheld.reason.clone());
                    gap(day, reason, &mut unknown);
                }
            }
        }
        let mut runs = runs(pieces);
        let mut names = Refusals::default();
        let choices = sea_regions::choices(&places, &sea, runs.keys().copied(), &mut names);
        let regions = choices
            .into_iter()
            .map(|choice| {
                let id: u32 = choice.id.parse().unwrap_or_default();
                SupremacyRegion { runs: runs.remove(&id).unwrap_or_default(), id: choice.id, name: choice.name }
            })
            .collect();
        let mut warnings = sea.notes.clone();
        warnings.extend(places.warnings());
        warnings.extend(names.lines("sea region names"));
        warnings.extend(notes.lines("naval supremacy fields"));
        let reply = NavalSupremacyReply { first_hour: first, last_hour: last, first_day_hour: first_day * DAY, day_hours: DAY, days: count, unknown, regions, scale: SCALE, warnings };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}
