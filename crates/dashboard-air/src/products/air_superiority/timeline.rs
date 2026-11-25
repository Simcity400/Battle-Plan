
use super::{
    AirMode, AirSide, AirSuperiorityAt, AirSuperiorityParams, BAD_BELOW, GOOD_ABOVE, HourScores, RecordedSideResolver, SCALE, SideResolver,
    assignment::AssignedSide,
    compose::{Cell, Compose, Pair, Roster, View},
    hour_scores,
    index::{self, AirIndex},
    names, region_name, side_of,
};
use dashboard_base::{Source, products::{LoadContext, MAX_PRODUCT_REPLY_BYTES, Product, at_hour::{self, AtHour, Read}, regions::{ActiveRegions, NoRegions, automatic, no_regions}}, span::HourSpan};
use dashboard_query::{Query, identity::Holders, timeline::Stretch};
use rayon::prelude::*;
use replay_lanes::lanes::{self, Chart};
use replay_data::rows::{CountryLinkRow, RaidAirScoreRow, TableId, tables};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[cfg(test)]
mod tests;

pub const DAY: i32 = replay_data::cadence::STAGGER_PERIOD;
const BUCKET: i32 = replay_columnar::summary::BASE_HOURS as i32;
const SERIES: &str = "air-scores";
const TOP: usize = 10;
const ABSENT: u32 = u32::MAX;

#[derive(Clone, Debug, PartialEq, Default, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AirTimelineParams {
    pub branch_id: String,
    pub first_hour: i32,
    pub last_hour: i32,
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub mode: Option<AirMode>,
    #[doc = " A side's key, or a country (its slot or tag) with `viewpointCountry`."]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub viewpoint: Option<dashboard_base::products::Country>,
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub viewpoint_country: Option<bool>,
    #[doc = " The side keys compared; all when none."]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub sides: Option<Vec<String>>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub pinned_regions: Option<Vec<u32>>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub hidden_regions: Option<Vec<u32>>,
    #[doc = " The air theatres whose regions are shown; all when none."]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub theatres: Option<Vec<String>>,
    #[doc = " How many of the regions most active at `lastHour` follow the user's regions."]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub top: Option<usize>,
    #[doc = " Every region with a score in the view, unranked, and `absent`: the cells of a region without one."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub every: Option<bool>,
}

impl AirTimelineParams {
    fn moment(&self) -> AirSuperiorityParams {
        AirSuperiorityParams {
            branch_id: self.branch_id.clone(),
            mode: self.mode,
            viewpoint: self.viewpoint.clone(),
            viewpoint_country: self.viewpoint_country,
            sides: self.sides.clone(),
            pinned_regions: None,
            hidden_regions: None,
            theatres: self.theatres.clone(),
            order: None,
        }
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        self.moment().validate()?;
        if self.first_hour > self.last_hour {
            return Err("Air timeline hours are reversed");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AirDayPart {
    pub side: u32,
    pub tag: String,
    pub friendly: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AirDaySides {
    pub parts: Vec<AirDayPart>,
    #[doc = " A leaderless side and the country whose read it shows."]
    pub viewpoints: Vec<(u32, String)>,
    pub unknown: Vec<u32>,
    pub no_air_object: Vec<u32>,
    #[doc = " Only the chosen sides are compared."]
    pub filtered: bool,
    #[doc = " Who leads the day, compared in exact integers."]
    pub lead: AirLead,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AirLead {
    Leading { side: u32, share_bp: i32 },
    Contested,
    NoActivity,
    Unknown,
}

impl AirDaySides {
    pub fn new(parts: Vec<AirDayPart>, viewpoints: Vec<(u32, String)>, unknown: Vec<u32>, no_air_object: Vec<u32>, filtered: bool) -> Self {
        let lead = if unknown.is_empty() { lead_of(&parts) } else { AirLead::Unknown };
        Self { parts, viewpoints, unknown, no_air_object, filtered, lead }
    }
}

fn lead_of(parts: &[AirDayPart]) -> AirLead {
    let mut total: i128 = 0;
    let mut best: Option<(u32, i128)> = None;
    for part in parts {
        let score = part.friendly.parse::<i128>().unwrap_or(0).max(0);
        total += score;
        if best.is_none_or(|(_, top)| score > top) {
            best = Some((part.side, score));
        }
    }
    match best {
        Some((side, score)) if total > 0 && score * 2 > total => AirLead::Leading { side, share_bp: ((score * 10_000 + total / 2) / total) as i32 },
        _ if total > 0 => AirLead::Contested,
        _ => AirLead::NoActivity,
    }
}

fn view_share(friendly: &str, enemy: &str) -> Option<i32> {
    let f = friendly.parse::<i128>().unwrap_or(0).max(0);
    let total = f + enemy.parse::<i128>().unwrap_or(0).max(0);
    (total > 0).then(|| ((f * 10_000 + total / 2) / total) as i32)
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AirDayCell {
    Reason { reason: String },
    Sides(AirDaySides),
    View {
        side: Option<u32>,
        leader: Option<String>,
        friendly: String,
        enemy: String,
        #[doc = " The friendly share in basis points, from exact integers; none without activity."]
        share_bp: Option<i32>,
    },
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AirReadout {
    Leading { side: u32, share_bp: i32, as_of_hour: i32 },
    Contested { as_of_hour: i32 },
    Viewpoint { side: Option<u32>, share_bp: i32, as_of_hour: i32 },
    NoActivity { as_of_hour: i32 },
    NotRecorded { reason: String },
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AirTimelineRow {
    pub region: u32,
    pub name: Option<String>,
    #[doc = " Why the name is not shown."]
    pub name_missing: Option<String>,
    pub pinned: bool,
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub readout: Option<AirReadout>,
    #[doc = " Runs of days: how many, and their cell; none for days the recording holds no hour of."]
    pub cells: Vec<(u32, Option<AirDayCell>)>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AirTimelineReply {
    #[doc = " The first hour of the view's first day."]
    pub first_hour: i32,
    pub last_hour: i32,
    pub day_hours: i32,
    #[doc = " Each day's read hour: its last recorded hour in the view; none where none is recorded."]
    pub days: Vec<Option<i32>>,
    #[doc = " Each day's sides of the comparison left out, without an air object."]
    pub excluded: Vec<u32>,
    #[doc = " The sides the cells name, by index."]
    pub sides: Vec<AirSide>,
    pub legend: Vec<u32>,
    #[doc = " The viewpoint side's index, at the last day it is present."]
    pub viewpoint: Option<u32>,
    pub gaps: Vec<HourSpan>,
    pub rows: Vec<AirTimelineRow>,
    #[doc = " Why the automatic rows are not known at `lastHour`; none are listed then, and the user's regions stay."]
    pub rank_missing: Option<String>,
    #[doc = " Why no row is listed; set only when `rows` is empty."]
    pub empty: Option<NoRegions>,
    pub active_unlisted: ActiveRegions,
    #[doc = " With `every`: the days of a region without a score, as runs."]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub absent: Option<Vec<(u32, Option<AirDayCell>)>>,
    pub scale: i32,
    pub bad_below: i32,
    pub good_above: i32,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(AirTimelineReply);

pub(crate) struct DayRead {
    pub hour: i32,
    pub witnesses: Vec<(u32, u32, i64, i64)>,
    pub complete: bool,
    pub links: Option<Arc<Vec<CountryLinkRow>>>,
}

pub(crate) trait RegionRows {
    fn rows(&self, day: usize, region: u32, out: &mut Vec<RaidAirScoreRow>);
}

fn score((country, region, friendly, enemy): (u32, u32, i64, i64)) -> RaidAirScoreRow {
    RaidAirScoreRow { country, region, friendly, enemy, ..Default::default() }
}

pub(crate) struct Composed<'c> {
    pub epoch: usize,
    pub hour: i32,
    pub has_roster: bool,
    pub sides: &'c [AssignedSide],
    pub compose: &'c Compose<'c>,
    pub scores: &'c HourScores,
}

impl Composed<'_> {
    fn pairs(&self, region: u32) -> &[Pair] {
        self.scores.pairs.get(&region).map_or(&[], Vec::as_slice)
    }
}

const NOT_RECORDED: &str = "Air scores were not recorded at this hour, so the most active regions are not known";
const READ_IN_PART: &str = "Air scores were read only in part at this hour, so the most active regions are not known";
const THEATRES_NOT_RECORDED: &str = "Air theatres were not recorded at this hour, so the most active regions in them are not known";

pub(crate) struct AtHourRank {
    pub regions: Vec<u32>,
    pub missing: Option<String>,
    pub outside: u32,
}

impl AtHourRank {
    fn unranked(reason: &str) -> Self {
        Self { regions: Vec::new(), missing: Some(reason.to_owned()), outside: 0 }
    }
}

pub(crate) struct Days<'a> {
    pub index: &'a AirIndex,
    pub holders: &'a Holders,
    pub days: &'a [Option<DayRead>],
    pub rows: &'a dyn RegionRows,
    pub params: &'a AirTimelineParams,
    pub theatres: Option<&'a [Option<Result<HashSet<u32>, ()>>]>,
}

impl<'a> Days<'a> {
    fn keep(&self) -> Option<HashSet<&'a str>> {
        self.params.sides.as_ref().filter(|keys| !keys.is_empty()).map(|keys| keys.iter().map(String::as_str).collect())
    }

    fn view(&self, sides: &[AssignedSide], hour: i32) -> View<'a> {
        let viewpoint = super::viewpoint_name(self.params.viewpoint.as_ref(), self.params.viewpoint_country == Some(true), self.holders, hour);
        match self.params.mode.unwrap_or_default() {
            AirMode::Sides => View::Sides,
            AirMode::Viewpoint if self.params.viewpoint_country == Some(true) => View::Country(viewpoint),
            AirMode::Viewpoint => View::Side(sides.iter().position(|side| side.key == viewpoint).map(|at| at as u32)),
        }
    }

    fn theatre_reason(&self, day: usize, region: u32) -> Option<&'static str> {
        match self.theatres?.get(day)? {
            None | Some(Err(())) => Some("theatres-not-recorded"),
            Some(Ok(regions)) if region != ABSENT && !regions.contains(&region) => Some("outside-theatres"),
            Some(Ok(_)) => None,
        }
    }

    fn each(&self, regions: &[u32], days: std::ops::Range<usize>, cancelled: &AtomicBool, mut each: impl FnMut(usize, Option<&Composed<'_>>)) -> io::Result<()> {
        let empty = Arc::default();
        let mut rows = Vec::new();
        for (at, day) in self.days.iter().enumerate().take(days.end).skip(days.start) {
            if at % 64 == 0 {
                dashboard_text::metadata::check_cancel(cancelled)?;
            }
            let Some(day) = day else {
                each(at, None);
                continue;
            };
            let epoch = self.index.epochs.partition_point(|epoch| epoch.last < day.hour);
            let held = self.index.epochs.get(epoch).filter(|held| held.first <= day.hour);
            rows.clear();
            rows.extend(day.witnesses.iter().copied().map(score));
            for &region in regions {
                self.rows.rows(at, region, &mut rows);
            }
            let present = held.map_or(&empty, |held| &held.present);
            let scores = hour_scores(&rows, day.complete, day.links.as_deref().map(Vec::as_slice), self.holders, present, day.hour);
            let sides: &[AssignedSide] = held.and_then(|held| held.sides.as_deref()).map_or(&[], Vec::as_slice);
            let roster = held.and_then(|held| held.sides.as_deref()).map(|sides| Roster::new(sides));
            let compose = Compose::new(roster.as_ref(), &scores.operands, self.view(sides, day.hour));
            each(at, Some(&Composed { epoch, hour: day.hour, has_roster: roster.is_some(), sides, compose: &compose, scores: &scores }));
        }
        Ok(())
    }

    fn activity(&self, composed: &Composed<'_>, region: u32, keep: Option<&HashSet<&str>>) -> u128 {
        let pairs = composed.pairs(region);
        match (self.params.mode.unwrap_or_default(), keep) {
            (AirMode::Sides, None) => composed.compose.activity(pairs),
            (AirMode::Sides, Some(keys)) => {
                let mut total = 0;
                composed.compose.scores(pairs, |position, score| {
                    if keys.contains(composed.sides[position as usize].key.as_str()) {
                        total += score;
                    }
                });
                total
            }
            (AirMode::Viewpoint, _) => composed.compose.view_activity(pairs),
        }
    }

    pub fn rank_at(&self, candidates: &[u32], cancelled: &AtomicBool) -> io::Result<AtHourRank> {
        let Some(day) = self.days.len().checked_sub(1) else {
            return Ok(AtHourRank::unranked(NOT_RECORDED));
        };
        match &self.days[day] {
            Some(read) if read.hour != self.params.last_hour => return Ok(AtHourRank::unranked(NOT_RECORDED)),
            Some(read) if !read.complete => return Ok(AtHourRank::unranked(READ_IN_PART)),
            None => return Ok(AtHourRank::unranked(NOT_RECORDED)),
            Some(_) => {}
        }
        let keep = self.keep();
        let mut active: Vec<(u32, u128)> = Vec::new();
        let mut outside = 0u32;
        let mut theatres_unknown = false;
        self.each(candidates, day..day + 1, cancelled, |at, composed| {
            let Some(composed) = composed else { return };
            for &region in candidates {
                let amount = self.activity(composed, region, keep.as_ref());
                if amount == 0 {
                    continue;
                }
                match self.theatre_reason(at, region) {
                    None => active.push((region, amount)),
                    Some("outside-theatres") => outside += 1,
                    Some(_) => theatres_unknown = true,
                }
            }
        })?;
        if theatres_unknown {
            return Ok(AtHourRank::unranked(THEATRES_NOT_RECORDED));
        }
        active.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        Ok(AtHourRank { regions: active.into_iter().map(|(region, _)| region).collect(), missing: None, outside })
    }

    #[cfg(test)]
    pub fn shown(&self, regions: &[u32], index: &AirIndex, cancelled: &AtomicBool) -> io::Result<Shown> {
        let mut null = super::NullSideResolver;
        self.shown_with_resolver(regions, index, &mut null, cancelled)
    }

    pub(crate) fn shown_with_resolver(&self, regions: &[u32], index: &AirIndex, resolver: &mut dyn SideResolver, cancelled: &AtomicBool) -> io::Result<Shown> {
        let mut dictionary = Dictionary::default();
        let composed = self.compose(regions, index, &mut dictionary, resolver, cancelled)?;
        let hours: Vec<Option<i32>> = self.days.iter().map(|day| day.as_ref().map(|day| day.hour)).collect();
        Ok(finish(composed, &hours, &dictionary.sides, self.params.mode.unwrap_or_default()))
    }

    fn compose(&self, regions: &[u32], index: &AirIndex, dictionary: &mut Dictionary, resolver: &mut dyn SideResolver, cancelled: &AtomicBool) -> io::Result<Composition> {
        let keep = self.keep();
        let mut cells: Vec<Vec<(u32, Option<AirDayCell>)>> = vec![Vec::new(); regions.len()];
        let mut excluded = vec![0u32; self.days.len()];
        let mut viewpoints = vec![DayViewpoint::Keep; self.days.len()];
        self.each(regions, 0..self.days.len(), cancelled, |day, composed| {
            for (row, &region) in cells.iter_mut().zip(regions) {
                let cell = composed.map(|composed| match self.theatre_reason(day, region) {
                    Some(reason) => AirDayCell::Reason { reason: reason.into() },
                    None => day_cell(composed.compose.cell(Some(composed.pairs(region))), composed, region, keep.as_ref(), &mut |position| {
                        dictionary.index(index, composed.epoch, composed.hour, composed.sides, position, resolver)
                    }),
                });
                push_days(row, (1, cell));
            }
            if let Some(composed) = composed {
                if matches!(self.params.mode.unwrap_or_default(), AirMode::Sides) {
                    excluded[day] = composed.compose.excluded() as u32;
                }
                viewpoints[day] = match self.view(composed.sides, composed.hour) {
                    View::Side(Some(position)) => DayViewpoint::Side(dictionary.index(index, composed.epoch, composed.hour, composed.sides, position, resolver)),
                    View::Side(None) if composed.has_roster => DayViewpoint::Clear,
                    _ => DayViewpoint::Keep,
                };
            }
        })?;
        Ok(Composition { cells, excluded, viewpoints })
    }
}

type DayRuns = Vec<(u32, Option<AirDayCell>)>;

fn push_days(row: &mut DayRuns, (count, cell): (u32, Option<AirDayCell>)) {
    match row.last_mut() {
        Some((repeat, held)) if *held == cell => *repeat += count,
        _ => row.push((count, cell)),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DayViewpoint {
    Keep,
    Clear,
    Side(u32),
}

struct Composition {
    cells: Vec<DayRuns>,
    excluded: Vec<u32>,
    viewpoints: Vec<DayViewpoint>,
}

fn renumber(cell: &mut AirDayCell, order: &[u32]) {
    let at = |side: &mut u32| *side = order[*side as usize];
    match cell {
        AirDayCell::Reason { .. } => {}
        AirDayCell::Sides(sides) => {
            sides.parts.iter_mut().for_each(|part| at(&mut part.side));
            sides.viewpoints.iter_mut().for_each(|(side, _)| at(side));
            sides.unknown.iter_mut().for_each(at);
            sides.no_air_object.iter_mut().for_each(at);
            if let AirLead::Leading { side, .. } = &mut sides.lead {
                at(side);
            }
        }
        AirDayCell::View { side, .. } => side.iter_mut().for_each(at),
    }
}

fn finish(composition: Composition, hours: &[Option<i32>], sides: &[AirSide], mode: AirMode) -> Shown {
    let Composition { mut cells, excluded, viewpoints } = composition;
    let mut order: Vec<Option<u32>> = vec![None; sides.len()];
    let mut placed: Vec<u32> = Vec::with_capacity(sides.len());
    let mut meet = |side: u32| {
        if order[side as usize].is_none() {
            order[side as usize] = Some(placed.len() as u32);
            placed.push(side);
        }
    };
    let mut latest_by_key: BTreeMap<&str, (i32, u32)> = BTreeMap::new();
    let mut viewpoint = None;
    let mut cursors = vec![(0usize, 0u32); cells.len()];
    for (day, hour) in hours.iter().enumerate() {
        for (row, cursor) in cells.iter().zip(cursors.iter_mut()) {
            let Some((count, cell)) = row.get(cursor.0) else { continue };
            cursor.1 += 1;
            if cursor.1 == *count {
                *cursor = (cursor.0 + 1, 0);
            }
            match cell {
                Some(AirDayCell::Sides(cell)) => {
                    cell.parts.iter().map(|part| part.side).chain(cell.viewpoints.iter().map(|(side, _)| *side)).chain(cell.unknown.iter().copied()).chain(cell.no_air_object.iter().copied()).for_each(&mut meet);
                    if let Some(hour) = *hour {
                        for side in cell.parts.iter().map(|part| part.side).chain(cell.unknown.iter().copied()).chain(cell.no_air_object.iter().copied()) {
                            let key = sides[side as usize].key.as_str();
                            match latest_by_key.get_mut(key) {
                                Some(entry) if hour >= entry.0 => *entry = (hour, side),
                                Some(_) => {}
                                None => {
                                    latest_by_key.insert(key, (hour, side));
                                }
                            }
                        }
                    }
                }
                Some(AirDayCell::View { side: Some(side), .. }) => meet(*side),
                _ => {}
            }
        }
        match viewpoints[day] {
            DayViewpoint::Side(side) => {
                meet(side);
                viewpoint = Some(side);
            }
            DayViewpoint::Clear => viewpoint = None,
            DayViewpoint::Keep => {}
        }
    }
    for side in 0..sides.len() as u32 {
        meet(side);
    }
    let order: Vec<u32> = order.into_iter().map(|at| at.unwrap_or_default()).collect();
    for row in &mut cells {
        for (_, cell) in row.iter_mut() {
            if let Some(cell) = cell {
                renumber(cell, &order);
            }
        }
    }
    let legend = match mode {
        AirMode::Sides => latest_by_key.into_values().map(|(_, side)| order[side as usize]).collect(),
        AirMode::Viewpoint => viewpoint.into_iter().map(|side: u32| order[side as usize]).collect(),
    };
    let last_hour = hours.last().copied().flatten();
    let readouts = cells.iter().map(|runs| readout_of(runs.last().and_then(|(_, cell)| cell.as_ref()), last_hour)).collect();
    Shown { cells, excluded, sides: placed.iter().map(|&side| sides[side as usize].clone()).collect(), legend, viewpoint: viewpoint.map(|side| order[side as usize]), readouts }
}

pub(crate) fn readout_of(cell: Option<&AirDayCell>, hour: Option<i32>) -> AirReadout {
    let Some(as_of_hour) = hour else {
        return AirReadout::NotRecorded { reason: "Not recorded at this date".into() };
    };
    let Some(cell) = cell else {
        return AirReadout::NotRecorded { reason: "Not recorded at this date".into() };
    };
    match cell {
        AirDayCell::Reason { reason } => AirReadout::NotRecorded { reason: reason.clone() },
        AirDayCell::Sides(sides) => match sides.lead {
            AirLead::Unknown => AirReadout::NotRecorded { reason: "A side's air score was not recorded".into() },
            AirLead::NoActivity => AirReadout::NoActivity { as_of_hour },
            AirLead::Contested => AirReadout::Contested { as_of_hour },
            AirLead::Leading { side, share_bp } => AirReadout::Leading { side, share_bp, as_of_hour },
        },
        AirDayCell::View { side, share_bp, .. } => match share_bp {
            Some(share_bp) => AirReadout::Viewpoint { side: *side, share_bp: *share_bp, as_of_hour },
            None => AirReadout::NoActivity { as_of_hour },
        },
    }
}

pub(crate) struct Shown {
    pub cells: Vec<Vec<(u32, Option<AirDayCell>)>>,
    pub excluded: Vec<u32>,
    pub sides: Vec<AirSide>,
    pub legend: Vec<u32>,
    pub viewpoint: Option<u32>,
    pub readouts: Vec<AirReadout>,
}

#[derive(Clone, Default)]
struct Dictionary {
    sides: Vec<AirSide>,
    at: HashMap<(usize, u32, i32), u32>,
}

impl Dictionary {
    fn index(&mut self, index: &AirIndex, epoch: usize, hour: i32, sides: &[AssignedSide], position: u32, resolver: &mut dyn SideResolver) -> u32 {
        if let Some(&at) = self.at.get(&(epoch, position, hour)) {
            return at;
        }
        let side = side_of(index, &sides[position as usize], hour, resolver);
        let at = match self.sides.iter().position(|held| *held == side) {
            Some(at) => at as u32,
            None => {
                self.sides.push(side);
                (self.sides.len() - 1) as u32
            }
        };
        self.at.insert((epoch, position, hour), at);
        at
    }
}

fn find<'p>(pairs: &'p [Pair], tag: &str) -> Option<&'p Pair> {
    pairs.binary_search_by(|held| held.tag.as_str().cmp(tag)).ok().map(|at| &pairs[at])
}

fn day_cell(cell: Cell, composed: &Composed<'_>, region: u32, keep: Option<&HashSet<&str>>, place: &mut dyn FnMut(u32) -> u32) -> AirDayCell {
    let pairs = composed.pairs(region);
    let kept = |position: u32| keep.is_none_or(|keys| keys.contains(composed.sides[position as usize].key.as_str()));
    match cell {
        Cell::Reason(reason) => AirDayCell::Reason { reason: reason.to_owned() },
        Cell::Sides(cell) => AirDayCell::Sides(AirDaySides::new(
            cell.parts
                .iter()
                .filter(|(position, ..)| kept(*position))
                .map(|(position, tag, _)| AirDayPart { side: place(*position), tag: tag.clone(), friendly: find(pairs, tag).map_or(0, |pair| pair.friendly).to_string() })
                .collect(),
            cell.viewpoints.iter().filter(|(position, _)| kept(*position)).map(|(position, tag)| (place(*position), tag.clone())).collect(),
            cell.unknown.iter().filter(|position| kept(**position)).map(|position| place(*position)).collect(),
            cell.no_air_object.iter().filter(|position| kept(**position)).map(|position| place(*position)).collect(),
            keep.is_some(),
        )),
        Cell::View { side, leader, row } => {
            let read = row.and(leader.as_deref()).and_then(|tag| find(pairs, tag));
            let (friendly, enemy) = (read.map_or(0, |pair| pair.friendly).to_string(), read.map_or(0, |pair| pair.enemy).to_string());
            AirDayCell::View { side: side.map(&mut *place), share_bp: view_share(&friendly, &enemy), friendly, enemy, leader }
        }
    }
}

struct Part {
    friendly: Chart,
    enemy_of: Vec<Option<usize>>,
    enemy: Chart,
    regions: HashMap<u32, Vec<usize>>,
}

fn at_end(chart: &Chart, at: usize, day: i32, hour: i32) -> Option<i64> {
    let bin = (i64::from(day) * i64::from(DAY) - chart.first_bin_hour).div_euclid(i64::from(DAY));
    let bin = chart.values(at).get(usize::try_from(bin).ok()?)?;
    (bin.hours > 0 && bin.last_bucket == hour.div_euclid(BUCKET)).then_some(bin.last)
}

#[derive(Clone, Copy)]
struct DayAt {
    hour: i32,
    part: usize,
}

type DayRows = (Vec<RaidAirScoreRow>, bool, Option<Arc<Vec<CountryLinkRow>>>);

struct Charted<'p> {
    parts: &'p [Part],
    days: &'p [Option<DayAt>],
    first_day: i32,
    at_hour: &'p HashMap<usize, DayRows>,
}

impl Charted<'_> {
    fn row(part: &Part, at: usize, day: i32, hour: i32) -> Option<(u32, u32, i64, i64)> {
        let key = part.friendly.keys[at];
        let friendly = at_end(&part.friendly, at, day, hour)?;
        let enemy = part.enemy_of[at].and_then(|enemy| at_end(&part.enemy, enemy, day, hour)).unwrap_or(0);
        Some(((key >> 32) as u32, key as u32, friendly, enemy))
    }

    fn witnesses(&self, day: usize) -> Vec<(u32, u32, i64, i64)> {
        if let Some((rows, ..)) = self.at_hour.get(&day) {
            return rows.iter().map(|row| (row.country, row.region, row.friendly, row.enemy)).collect();
        }
        let Some(read) = self.days[day] else { return Vec::new() };
        let part = &self.parts[read.part];
        let mut out: Vec<(u32, u32, i64, i64)> = Vec::new();
        for at in 0..part.friendly.keys.len() {
            let Some(row) = Self::row(part, at, self.first_day + day as i32, read.hour) else { continue };
            match out.last_mut() {
                Some(held) if held.0 == row.0 => {
                    if row.2 < 0 || row.3 < 0 {
                        *held = row;
                    }
                }
                _ => out.push(row),
            }
        }
        out
    }
}

impl RegionRows for Charted<'_> {
    fn rows(&self, day: usize, region: u32, out: &mut Vec<RaidAirScoreRow>) {
        if let Some((rows, ..)) = self.at_hour.get(&day) {
            out.extend(rows.iter().filter(|row| row.region == region).cloned());
            return;
        }
        let Some(read) = self.days[day] else { return };
        let part = &self.parts[read.part];
        for &at in part.regions.get(&region).into_iter().flatten() {
            if let Some(row) = Self::row(part, at, self.first_day + day as i32, read.hour) {
                out.push(score(row));
            }
        }
    }
}

fn chart(query: &Query<'_>, stretch: &Stretch, measure: &str) -> io::Result<Chart> {
    let series = dashboard_declare::series(SERIES).ok_or_else(|| dashboard_core::invalid("The air scores series is not declared"))?;
    let measure = series.measure(measure).ok_or_else(|| dashboard_core::invalid("The air scores series has no such measure"))?;
    let source = dashboard_declare::summary_id(TableId::RaidAirScores, measure)?;
    dashboard_query::cost::storage(|| lanes::chart_at(query.replay.recording(), stretch.run, TableId::RaidAirScores, source, None, stretch.first, stretch.last, 0, i64::from(DAY)))
}

fn part(query: &Query<'_>, stretch: &Stretch) -> io::Result<Part> {
    let friendly = chart(query, stretch, "friendly")?;
    let enemy = chart(query, stretch, "enemy")?;
    let enemy_at: HashMap<u128, usize> = enemy.keys.iter().enumerate().map(|(at, &key)| (key, at)).collect();
    let enemy_of = friendly.keys.iter().map(|key| enemy_at.get(key).copied()).collect();
    let mut regions: HashMap<u32, Vec<usize>> = HashMap::new();
    for (at, &key) in friendly.keys.iter().enumerate() {
        regions.entry(key as u32).or_default().push(at);
    }
    Ok(Part { friendly, enemy_of, enemy, regions })
}

struct Links {
    runs: Vec<(u32, i32, i32, u32)>,
    cuts: Vec<i32>,
}

fn links(query: &Query<'_>, stretch: &Stretch) -> Option<Links> {
    let slot = dashboard_declare::field(TableId::CountryLinks, "canonical_slot").ok()?;
    let history = dashboard_query::cost::storage(|| lanes::history(query.replay.recording(), stretch.run, TableId::CountryLinks, &[slot], lanes::Keys::All, stretch.first, stretch.last)).ok()?;
    let mut runs = Vec::new();
    let mut cuts = BTreeSet::new();
    for (key, held) in history.runs(slot)? {
        for (first, last, cell) in held {
            if let Some(lanes::Cell::Number(slot)) = cell {
                runs.push((key as u32, first, last, slot as u32));
                cuts.insert(first);
                cuts.insert(last.saturating_add(1));
            }
        }
    }
    Some(Links { runs, cuts: cuts.into_iter().collect() })
}

impl Links {
    fn at(&self, hour: i32) -> Vec<CountryLinkRow> {
        let mut out: Vec<CountryLinkRow> = self.runs.iter().filter(|(_, first, last, _)| *first <= hour && hour <= *last).map(|&(id, _, _, slot)| CountryLinkRow { id, canonical_slot: slot }).collect();
        out.sort_by_key(|link| link.id);
        out
    }
}

fn theatres_in(query: &Query<'_>, run: u32, first: i32, last: i32, theatres: &[u64]) -> Option<(Vec<(i32, i32)>, Vec<(u32, i32, i32)>)> {
    let table = TableId::AirTheatreRegions;
    let recording = query.replay.recording();
    let ranges: Vec<(u128, u128)> = theatres.iter().map(|&theatre| (u128::from(theatre) << 32, u128::from(theatre) << 32 | u128::from(u32::MAX))).collect();
    let column = dashboard_declare::field(table, "wings").ok()?;
    let read = dashboard_query::cost::storage(|| lanes::reads(recording, run, table, first, last)).ok()?;
    let history = dashboard_query::cost::storage(|| lanes::history(recording, run, table, &[column], lanes::Keys::Ranges(&ranges), first, last)).ok()?;
    let runs = history.runs(column)?.into_iter().flat_map(|(key, runs)| runs.into_iter().filter(|run| run.2.is_some()).map(move |(first, last, _)| (key as u32, first, last))).collect();
    Some((read, runs))
}

fn theatre_days(query: &Query<'_>, stretches: &[Stretch], days: &[Option<DayRead>], at: &[Option<DayAt>], ids: &[String]) -> Vec<Option<Result<HashSet<u32>, ()>>> {
    let theatres: Vec<u64> = ids.iter().filter_map(|id| id.parse().ok()).collect::<BTreeSet<u64>>().into_iter().collect();
    let held: Vec<Option<(Vec<(i32, i32)>, Vec<(u32, i32, i32)>)>> = stretches.iter().map(|stretch| theatres_in(query, stretch.run, stretch.first, stretch.last, &theatres)).collect();
    days.iter()
        .zip(at)
        .map(|(read, at)| {
            let (read, at) = (read.as_ref()?, at.as_ref()?);
            Some(match &held[at.part] {
                Some((reads, runs)) if reads.iter().any(|(first, last)| *first <= read.hour && read.hour <= *last) => {
                    Ok(runs.iter().filter(|(_, first, last)| *first <= read.hour && read.hour <= *last).map(|(region, ..)| *region).collect())
                }
                _ => Err(()),
            })
        })
        .collect()
}

struct Window {
    first_day: i32,
    days: Vec<Option<DayAt>>,
    parts: Vec<Part>,
    at_hours: HashMap<usize, DayRows>,
    reads: Vec<Option<DayRead>>,
    theatres: Option<Vec<Option<Result<HashSet<u32>, ()>>>>,
    unread: Vec<(usize, i32, String)>,
}

impl Window {
    fn charted(&self) -> Charted<'_> {
        Charted { parts: &self.parts, days: &self.days, first_day: self.first_day, at_hour: &self.at_hours }
    }

    fn candidates(&self) -> BTreeSet<u32> {
        self.parts.iter().flat_map(|part| part.regions.keys().copied()).chain(self.at_hours.values().flat_map(|(rows, ..)| rows.iter().map(|row| row.region))).collect()
    }
}

fn window(query: &Query<'_>, params: &AirTimelineParams, view_first: i32, view_last: i32, cancelled: &AtomicBool) -> io::Result<Window> {
    let first_day = view_first.div_euclid(DAY);
    let first_hour = first_day.checked_mul(DAY).ok_or_else(|| dashboard_core::invalid("Air timeline start is outside the calendar range"))?;
    let count = (i64::from(view_last.div_euclid(DAY)) - i64::from(first_day) + 1) as usize;
    if count.saturating_mul(2).saturating_sub(1) > MAX_PRODUCT_REPLY_BYTES {
        return Err(dashboard_core::refused("The air timeline's days pass the reply budget: choose a shorter range"));
    }
    let stretches: Vec<Stretch> = query.timeline.within(first_hour, view_last).collect();
    let mut days: Vec<Option<DayAt>> = vec![None; count];
    for (at, stretch) in stretches.iter().enumerate() {
        for day in stretch.first.div_euclid(DAY)..=stretch.last.div_euclid(DAY) {
            days[(day - first_day) as usize] = Some(DayAt { hour: stretch.last.min(day * DAY + DAY - 1), part: at });
        }
    }
    let parts = stretches.par_iter().map(|stretch| part(query, stretch)).collect::<io::Result<Vec<_>>>().map_err(|error| dashboard_core::refused(format!("Air scores are not summarized in this recording: {error}")))?;
    dashboard_text::metadata::check_cancel(cancelled)?;
    let links: Vec<Option<Links>> = stretches.par_iter().map(|stretch| links(query, stretch)).collect();

    let mut at_hours: HashMap<usize, DayRows> = HashMap::new();
    let mut unread: HashSet<usize> = HashSet::new();
    let mut unread_notes: Vec<(usize, i32, String)> = Vec::new();
    for (at, read) in days.iter().enumerate() {
        let Some(read) = read else { continue };
        if read.hour != stretches[read.part].last || (read.hour + 1).rem_euclid(BUCKET) == 0 {
            continue;
        }
        let reads = [Read::all(TableId::RaidAirScores), Read::all(TableId::CountryLinks)];
        let frame = at_hour::frame_with(query, read.hour, &reads, AirSuperiorityAt::columns, "Air superiority")?;
        match dashboard_text::metadata::recorded_rows::<tables::RaidAirScores>(&frame) {
            Ok(rows) => {
                let complete = frame.complete::<tables::RaidAirScores>().is_ok();
                let links = frame.complete::<tables::CountryLinks>().ok().map(|links| Arc::new(links.to_vec()));
                at_hours.insert(at, (rows.to_vec(), complete, links));
            }
            Err(error) => {
                unread_notes.push((at, read.hour, error.to_string()));
                unread.insert(at);
            }
        }
    }
    let charted = Charted { parts: &parts, days: &days, first_day, at_hour: &at_hours };
    let recording = query.replay.recording();
    let statuses: Vec<Vec<(i32, i32, bool)>> = stretches
        .iter()
        .map(|stretch| {
            dashboard_query::cost::storage(|| replay_lanes::lanes::statuses(recording, stretch.run, TableId::RaidAirScores, stretch.first, stretch.last))
                .map(|held| held.into_iter().map(|status| (status.first, status.last, replay_recording::columnar_frames::complete(status.status))).collect())
        })
        .collect::<io::Result<_>>()?;
    let whole = |part: usize, hour: i32| statuses[part].iter().any(|&(first, last, complete)| complete && first <= hour && hour <= last);
    let mut held_links: HashMap<(usize, usize), Arc<Vec<CountryLinkRow>>> = HashMap::new();
    let mut reads: Vec<Option<DayRead>> = Vec::with_capacity(count);
    for (at, day) in days.iter().enumerate() {
        let Some(day) = day else {
            reads.push(None);
            continue;
        };
        if let Some((_, complete, links)) = at_hours.get(&at) {
            reads.push(Some(DayRead { hour: day.hour, witnesses: charted.witnesses(at), complete: *complete, links: links.clone() }));
            continue;
        }
        if unread.contains(&at) {
            reads.push(None);
            continue;
        }
        let links = links[day.part].as_ref().map(|links| {
            let cut = links.cuts.partition_point(|cut| *cut <= day.hour);
            Arc::clone(held_links.entry((day.part, cut)).or_insert_with(|| Arc::new(links.at(day.hour))))
        });
        reads.push(Some(DayRead { hour: day.hour, witnesses: charted.witnesses(at), complete: whole(day.part, day.hour), links }));
    }
    dashboard_text::metadata::check_cancel(cancelled)?;
    let theatres = params.theatres.as_deref().filter(|ids| !ids.is_empty()).map(|ids| theatre_days(query, &stretches, &reads, &days, ids));
    Ok(Window { first_day, days, parts, at_hours, reads, theatres, unread: unread_notes })
}

#[derive(Clone)]
struct AirHistory {
    through: i32,
    hours: Vec<Option<i32>>,
    excluded: Vec<u32>,
    viewpoints: Vec<DayViewpoint>,
    theatres: Option<Vec<Option<Result<HashSet<u32>, ()>>>>,
    dictionary: Dictionary,
    regions: BTreeMap<u32, DayRuns>,
    absent: DayRuns,
    unread: Vec<(usize, i32, String)>,
    used: u64,
}

impl AirHistory {
    #[expect(clippy::too_many_arguments, reason = "one read of the history's days: the query, the side index, the view, and the regions and sides it already holds")]
    fn read(query: &Query<'_>, index: &AirIndex, params: &AirTimelineParams, (first, last): (i32, i32), held: &[u32], mut dictionary: Dictionary, resolver: &mut dyn SideResolver, cancelled: &AtomicBool) -> io::Result<Self> {
        let window = window(query, params, first, last, cancelled)?;
        let mut regions: Vec<u32> = window.candidates().into_iter().chain(held.iter().copied()).collect::<BTreeSet<u32>>().into_iter().collect();
        regions.push(ABSENT);
        let charted = window.charted();
        let days = Days { index, holders: &query.holders, days: &window.reads, rows: &charted, params, theatres: window.theatres.as_deref() };
        let Composition { mut cells, excluded, viewpoints } = days.compose(&regions, index, &mut dictionary, resolver, cancelled)?;
        let absent = cells.pop().unwrap_or_default();
        regions.pop();
        Ok(Self {
            through: last,
            hours: window.reads.iter().map(|read| read.as_ref().map(|read| read.hour)).collect(),
            excluded,
            viewpoints,
            theatres: window.theatres,
            dictionary,
            regions: regions.into_iter().zip(cells).collect(),
            absent,
            unread: window.unread,
            used: 0,
        })
    }

    fn unscored(&self, region: u32) -> DayRuns {
        let mut out = DayRuns::new();
        let mut day = 0usize;
        for (count, cell) in &self.absent {
            for _ in 0..*count {
                let outside = cell.is_some() && matches!(self.theatres.as_ref().and_then(|theatres| theatres.get(day)), Some(Some(Ok(regions))) if !regions.contains(&region));
                push_days(&mut out, (1, if outside { Some(AirDayCell::Reason { reason: "outside-theatres".into() }) } else { cell.clone() }));
                day += 1;
            }
        }
        out
    }

    fn join(&mut self, next: Self) {
        let offset = self.hours.len();
        for (region, runs) in next.regions {
            if !self.regions.contains_key(&region) {
                let earlier = self.unscored(region);
                self.regions.insert(region, earlier);
            }
            if let Some(held) = self.regions.get_mut(&region) {
                runs.into_iter().for_each(|run| push_days(held, run));
            }
        }
        next.absent.into_iter().for_each(|run| push_days(&mut self.absent, run));
        self.hours.extend(next.hours);
        self.excluded.extend(next.excluded);
        self.viewpoints.extend(next.viewpoints);
        if let (Some(held), Some(theatres)) = (self.theatres.as_mut(), next.theatres) {
            held.extend(theatres);
        }
        self.unread.extend(next.unread.into_iter().map(|(at, hour, error)| (at + offset, hour, error)));
        self.dictionary = next.dictionary;
        self.through = next.through;
    }
}

#[derive(Default)]
struct HeldHistories {
    histories: HashMap<String, AirHistory>,
    uses: u64,
}

const HELD_HISTORIES: usize = 16;

fn history(source: &Source, query: &Query<'_>, index: &AirIndex, params: &AirTimelineParams, load: &LoadContext, resolver: &mut dyn SideResolver, cancelled: &AtomicBool) -> io::Result<AirHistory> {
    let (first, last) = (params.first_hour, params.last_hour);
    let content = load.content.as_ref().map(|options| (dashboard_base::content::content_key(Some(options)), options.presentation.identity().clone()));
    let key = format!("{}|{}|{:?}|{:?}|{:?}|{:?}|{:?}|{content:?}", params.branch_id, first.div_euclid(DAY), params.mode, params.viewpoint, params.viewpoint_country, params.sides, params.theatres);
    let state = source.state::<HeldHistories>();
    let held = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).histories.get(&key).filter(|held| held.through == last || (held.through < last && (held.through + 1).rem_euclid(DAY) == 0)).cloned();
    let history = match held {
        Some(held) if held.through == last => held,
        Some(mut held) => {
            let regions: Vec<u32> = held.regions.keys().copied().collect();
            let next = AirHistory::read(query, index, params, (held.through + 1, last), &regions, held.dictionary.clone(), resolver, cancelled)?;
            held.join(next);
            held
        }
        None => AirHistory::read(query, index, params, (first, last), &[], Dictionary::default(), resolver, cancelled)?,
    };
    let mut state = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    state.uses += 1;
    let used = state.uses;
    if state.histories.len() >= HELD_HISTORIES
        && !state.histories.contains_key(&key)
        && let Some(oldest) = state.histories.iter().min_by_key(|(_, held)| held.used).map(|(key, _)| key.clone())
    {
        state.histories.remove(&oldest);
    }
    state.histories.insert(key, AirHistory { used, ..history.clone() });
    Ok(history)
}

fn unread_warnings(unread: &[(usize, i32, String)], count: usize) -> Vec<String> {
    unread
        .iter()
        .map(|(at, hour, error)| match at + 1 == count {
            true => format!("Air scores are not recorded at the view's last hour: {error}"),
            false => format!("Air scores are not recorded at hour {hour}, where a stretch of the timeline ends: {error}"),
        })
        .collect()
}

fn timeline(source: &Source, params: &AirTimelineParams, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<AirTimelineReply> {
    let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid air branch"))?;
    let query = dashboard_base::queries::query(source, branch)?;
    let index = index::index(source, branch).map_err(|error| dashboard_core::refused(format!("Air sides are not known: {error}")))?;
    dashboard_text::metadata::check_cancel(cancelled)?;
    let (view_first, view_last) = (params.first_hour, params.last_hour);
    let names = names(source, load);
    let mut warnings = names.warnings.clone();
    let first_hour = view_first.div_euclid(DAY).checked_mul(DAY).ok_or_else(|| dashboard_core::invalid("Air timeline start is outside the calendar range"))?;
    let gaps: Vec<HourSpan> = query
        .timeline
        .unheld
        .iter()
        .filter_map(|gap| {
            let a = gap.first.max(first_hour);
            let b = gap.last.min(view_last);
            (a <= b).then_some(HourSpan { first_hour: a, last_hour: b })
        })
        .collect();
    let pinned_set: HashSet<u32> = params.pinned_regions.iter().flatten().copied().collect();
    let hidden_set: HashSet<u32> = params.hidden_regions.iter().flatten().copied().collect();
    let pinned_list: Vec<u32> = {
        let mut seen = HashSet::new();
        params.pinned_regions.iter().flatten().copied().filter(|id| seen.insert(*id)).collect()
    };
    let named = |regions: &[u32], cells: Vec<Vec<(u32, Option<AirDayCell>)>>, readouts: Vec<Option<AirReadout>>| -> Vec<AirTimelineRow> {
        regions
            .iter()
            .zip(cells.into_iter().chain(std::iter::repeat_with(Vec::new)))
            .zip(readouts.into_iter().chain(std::iter::repeat(None)))
            .map(|((&region, cells), readout)| {
                let (name, name_missing) = region_name(&names, region);
                let pinned = pinned_set.contains(&region);
                AirTimelineRow { region, name, name_missing, pinned, readout, cells }
            })
            .collect()
    };
    let every = params.every == Some(true);
    let mut rank_missing = None;
    let mut filtered = (0u32, 0u32);
    let mut active_unlisted = ActiveRegions::default();
    let mut resolved = Vec::new();
    let (shown, mut cells, days) = if every {
        let mut resolver = RecordedSideResolver::new(source, &query, load, cancelled, &mut resolved)?;
        let history = history(source, &query, &index, params, load, &mut resolver, cancelled)?;
        warnings.extend(unread_warnings(&history.unread, history.hours.len()));
        let mut shown: Vec<u32> = history.regions.keys().copied().collect();
        shown.push(ABSENT);
        let cells = history.regions.values().cloned().chain([history.absent.clone()]).collect();
        let composition = Composition { cells, excluded: history.excluded.clone(), viewpoints: history.viewpoints.clone() };
        let cells = finish(composition, &history.hours, &history.dictionary.sides, params.mode.unwrap_or_default());
        (shown, cells, history.hours)
    } else {
        let shown_window = window(&query, params, view_first, view_last, cancelled)?;
        warnings.extend(unread_warnings(&shown_window.unread, shown_window.reads.len()));
        let charted = shown_window.charted();
        let composer = Days { index: &index, holders: &query.holders, days: &shown_window.reads, rows: &charted, params, theatres: shown_window.theatres.as_deref() };
        let candidates: Vec<u32> = shown_window.candidates().into_iter().filter(|region| !pinned_set.contains(region)).collect();
        let ranked = composer.rank_at(&candidates, cancelled)?;
        filtered = (ranked.regions.iter().filter(|region| hidden_set.contains(region)).count() as u32, ranked.outside);
        rank_missing = ranked.missing;
        let chosen = automatic(ranked.regions, &hidden_set, params.top.unwrap_or(TOP), rank_missing.clone());
        active_unlisted = chosen.active_unlisted;
        let mut shown = pinned_list;
        shown.extend(chosen.listed);
        let mut resolver = RecordedSideResolver::new(source, &query, load, cancelled, &mut resolved)?;
        let cells = composer.shown_with_resolver(&shown, &index, &mut resolver, cancelled)?;
        (shown, cells, shown_window.reads.iter().map(|read| read.as_ref().map(|read| read.hour)).collect())
    };
    warnings.extend(resolved);
    let absent = every.then(|| cells.cells.pop().unwrap_or_default());
    if every {
        cells.readouts.pop();
    }
    let shown = &shown[..cells.cells.len()];
    let rows = named(shown, cells.cells, cells.readouts.into_iter().map(Some).collect());
    let empty = no_regions(every || !rows.is_empty(), rank_missing.as_deref(), filtered.0, filtered.1);
    Ok(AirTimelineReply {
        first_hour,
        last_hour: view_last,
        day_hours: DAY,
        days,
        excluded: cells.excluded,
        sides: cells.sides,
        legend: cells.legend,
        viewpoint: cells.viewpoint,
        gaps,
        rows,
        rank_missing,
        empty,
        active_unlisted,
        absent,
        scale: SCALE,
        bad_below: BAD_BELOW,
        good_above: GOOD_ABOVE,
        warnings,
    })
}

pub struct AirSuperiorityTimeline;

impl Product for AirSuperiorityTimeline {
    const KEY: &'static str = "air-superiority-timeline";
    const CONTENT_DEPENDENT: bool = true;
    type Params = AirTimelineParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, firstHour: Start, lastHour: End, every, mode, viewpoint, viewpointCountry, sides, pinnedRegions, hiddenRegions, theatres, top, "pinnedRegions[]", "hiddenRegions[]"];
    type Reply = AirTimelineReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![AirTimelineParams::decl(), AirTimelineReply::decl(), AirTimelineRow::decl(), AirDayCell::decl(), AirDaySides::decl(), AirLead::decl(), AirDayPart::decl(), AirReadout::decl(), ActiveRegions::decl(), HourSpan::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        timeline(source, params, context, cancelled)
    }
}
