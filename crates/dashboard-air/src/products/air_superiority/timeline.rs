
use super::{
    AirCountryStyle, AirMode, AirSide, AirSuperiorityAt, AirSuperiorityParams, BAD_BELOW, GOOD_ABOVE, HourScores, RecordedSideResolver, SCALE, SideResolver,
    assignment::AssignedSide,
    compose::{Cell, Compose, Pair, Roster, View},
    country_style_at, hour_scores,
    index::{self, AirIndex},
    names, region_name, side_of,
};
use dashboard_base::{Source, products::{LoadContext, MAX_PRODUCT_REPLY_BYTES, Product, at_hour::{self, AtHour, Read}}, span::HourSpan};
use dashboard_query::{Query, identity::Holders, timeline::Stretch};
use rayon::prelude::*;
use replay_lanes::lanes::{self, Chart};
use replay_columnar::manifest::Status;
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
    #[doc = " The regions shown, ranked; the most active when none."]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub regions: Option<Vec<u32>>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub pinned_regions: Option<Vec<u32>>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub hidden_regions: Option<Vec<u32>>,
    #[doc = " The air theatres whose regions are shown; all when none."]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub theatres: Option<Vec<String>>,
    #[doc = " How many of the most active regions are shown, without `regions`."]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub top: Option<usize>,
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
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AirDayCell {
    Reason { reason: String },
    Sides(AirDaySides),
    View { side: Option<u32>, leader: Option<String>, friendly: String, enemy: String },
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
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub viewpoint_country_style: Option<AirCountryStyle>,
    pub gaps: Vec<HourSpan>,
    pub rows: Vec<AirTimelineRow>,
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
            Some(Ok(regions)) if !regions.contains(&region) => Some("outside-theatres"),
            Some(Ok(_)) => None,
        }
    }

    fn each(&self, regions: &[u32], through: usize, cancelled: &AtomicBool, mut each: impl FnMut(usize, Option<&Composed<'_>>)) -> io::Result<()> {
        let empty = Arc::default();
        let mut rows = Vec::new();
        for (at, day) in self.days.iter().enumerate().take(through) {
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

    fn activity(&self, day: usize, composed: &Composed<'_>, region: u32, keep: Option<&HashSet<&str>>) -> u128 {
        if self.theatre_reason(day, region).is_some() {
            return 0;
        }
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

    pub fn totals(&self, regions: &[u32], counted: usize, cancelled: &AtomicBool) -> io::Result<Vec<u128>> {
        let keep = self.keep();
        let mut totals = vec![0u128; regions.len()];
        self.each(regions, counted, cancelled, |day, composed| {
            let Some(composed) = composed else { return };
            for (total, &region) in totals.iter_mut().zip(regions) {
                *total += self.activity(day, composed, region, keep.as_ref());
            }
        })?;
        Ok(totals)
    }

    pub fn rank(&self, candidates: &[u32], bounds: Option<&HashMap<u32, u128>>, counted: usize, top: Option<usize>, cancelled: &AtomicBool) -> io::Result<Vec<u32>> {
        if top == Some(0) {
            return Ok(Vec::new());
        }
        let mut exact: Vec<(u32, u128)> = Vec::new();
        match (bounds, top) {
            (Some(bounds), Some(top)) => {
                let mut order: Vec<(u32, u128)> = candidates.iter().map(|&region| (region, bounds.get(&region).copied().unwrap_or(0))).collect();
                order.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                let batch = 2 * top;
                let mut next = 0;
                while next < order.len() {
                    if order[next].1 == 0 {
                        exact.extend(order[next..].iter().map(|&(region, _)| (region, 0)));
                        break;
                    }
                    let end = (next + batch).min(order.len());
                    let regions: Vec<u32> = order[next..end].iter().filter(|(_, bound)| *bound > 0).map(|(region, _)| *region).collect();
                    let totals = self.totals(&regions, counted, cancelled)?;
                    exact.extend(regions.into_iter().zip(totals));
                    exact.extend(order[next..end].iter().filter(|(_, bound)| *bound == 0).map(|&(region, _)| (region, 0)));
                    next = end;
                    if exact.len() >= top {
                        exact.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                        if order.get(next).is_none_or(|&(_, bound)| bound < exact[top - 1].1) {
                            break;
                        }
                    }
                }
            }
            _ => exact = candidates.iter().copied().zip(self.totals(candidates, counted, cancelled)?).collect(),
        }
        exact.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        if let Some(top) = top {
            exact.truncate(top);
        }
        Ok(exact.into_iter().map(|(region, _)| region).collect())
    }

    #[cfg(test)]
    pub fn shown(&self, regions: &[u32], index: &AirIndex, cancelled: &AtomicBool) -> io::Result<Shown> {
        let mut null = super::NullSideResolver;
        self.shown_with_resolver(regions, index, &mut null, cancelled)
    }

    pub(crate) fn shown_with_resolver(&self, regions: &[u32], index: &AirIndex, resolver: &mut dyn SideResolver, cancelled: &AtomicBool) -> io::Result<Shown> {
        let keep = self.keep();
        let mut dictionary = Dictionary::default();
        let mut cells: Vec<Vec<(u32, Option<AirDayCell>)>> = vec![Vec::new(); regions.len()];
        let mut excluded = vec![0u32; self.days.len()];
        let mut viewpoint = None;
        let mut latest_by_key: BTreeMap<String, (i32, u32)> = BTreeMap::new();
        self.each(regions, self.days.len(), cancelled, |day, composed| {
            for (row, &region) in cells.iter_mut().zip(regions) {
                let cell = composed.map(|composed| match self.theatre_reason(day, region) {
                    Some(reason) => AirDayCell::Reason { reason: reason.into() },
                    None => day_cell(composed.compose.cell(Some(composed.pairs(region))), composed, region, keep.as_ref(), &mut |position| {
                        dictionary.index(index, composed.epoch, composed.hour, composed.sides, position, resolver)
                    }),
                });
                if let (Some(composed), Some(AirDayCell::Sides(sides_cell))) = (composed, &cell) {
                    for side in sides_cell.parts.iter().map(|part| part.side).chain(sides_cell.unknown.iter().copied()).chain(sides_cell.no_air_object.iter().copied()) {
                        let key = &dictionary.sides[side as usize].key;
                        match latest_by_key.get_mut(key) {
                            Some(entry) if composed.hour >= entry.0 => *entry = (composed.hour, side),
                            Some(_) => {}
                            None => {
                                latest_by_key.insert(key.clone(), (composed.hour, side));
                            }
                        }
                    }
                }
                match row.last_mut() {
                    Some((repeat, held)) if *held == cell => *repeat += 1,
                    _ => row.push((1, cell)),
                }
            }
            if let Some(composed) = composed {
                if matches!(self.params.mode.unwrap_or_default(), AirMode::Sides) {
                    excluded[day] = composed.compose.excluded() as u32;
                }
                match self.view(composed.sides, composed.hour) {
                    View::Side(Some(position)) => {
                        viewpoint = Some(dictionary.index(index, composed.epoch, composed.hour, composed.sides, position, resolver));
                    }
                    View::Side(None) if composed.has_roster => viewpoint = None,
                    _ => {}
                }
            }
        })?;
        let legend = match self.params.mode.unwrap_or_default() {
            AirMode::Sides => latest_by_key.into_values().map(|(_, side)| side).collect(),
            AirMode::Viewpoint => viewpoint.into_iter().collect(),
        };
        let last_hour = self.days.last().and_then(|day| day.as_ref().map(|day| day.hour));
        let readouts = cells
            .iter()
            .map(|runs| readout_of(runs.last().and_then(|(_, cell)| cell.as_ref()), last_hour))
            .collect();
        Ok(Shown { cells, excluded, sides: dictionary.sides, legend, viewpoint, readouts })
    }
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
        AirDayCell::Sides(sides) if !sides.unknown.is_empty() => AirReadout::NotRecorded { reason: "A side's air score was not recorded".into() },
        AirDayCell::Sides(sides) => {
            let mut total: i128 = 0;
            let mut best: Option<(u32, i128)> = None;
            for part in &sides.parts {
                let score = part.friendly.parse::<i128>().unwrap_or(0).max(0);
                total += score;
                if best.is_none_or(|(_, top)| score > top) {
                    best = Some((part.side, score));
                }
            }
            if total == 0 {
                return AirReadout::NoActivity { as_of_hour };
            }
            let Some((side, score)) = best else {
                return AirReadout::NoActivity { as_of_hour };
            };
            if score * 2 > total {
                let share_bp = ((score * 10_000 + total / 2) / total) as i32;
                AirReadout::Leading { side, share_bp, as_of_hour }
            } else {
                AirReadout::Contested { as_of_hour }
            }
        }
        AirDayCell::View { side, friendly, enemy, .. } => {
            let f = friendly.parse::<i128>().unwrap_or(0).max(0);
            let e = enemy.parse::<i128>().unwrap_or(0).max(0);
            let total = f + e;
            if total == 0 {
                return AirReadout::NoActivity { as_of_hour };
            }
            let share_bp = ((f * 10_000 + total / 2) / total) as i32;
            AirReadout::Viewpoint { side: *side, share_bp, as_of_hour }
        }
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

#[derive(Default)]
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
        Cell::Sides(cell) => AirDayCell::Sides(AirDaySides {
            parts: cell
                .parts
                .iter()
                .filter(|(position, ..)| kept(*position))
                .map(|(position, tag, _)| AirDayPart { side: place(*position), tag: tag.clone(), friendly: find(pairs, tag).map_or(0, |pair| pair.friendly).to_string() })
                .collect(),
            viewpoints: cell.viewpoints.iter().filter(|(position, _)| kept(*position)).map(|(position, tag)| (place(*position), tag.clone())).collect(),
            unknown: cell.unknown.iter().filter(|position| kept(**position)).map(|position| place(*position)).collect(),
            no_air_object: cell.no_air_object.iter().filter(|position| kept(**position)).map(|position| place(*position)).collect(),
            filtered: keep.is_some(),
        }),
        Cell::View { side, leader, row } => {
            let read = row.and(leader.as_deref()).and_then(|tag| find(pairs, tag));
            AirDayCell::View {
                side: side.map(&mut *place),
                friendly: read.map_or(0, |pair| pair.friendly).to_string(),
                enemy: read.map_or(0, |pair| pair.enemy).to_string(),
                leader,
            }
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

struct Activity {
    day: i32,
    sums: [i64; 2],
    totals: [i64; 2],
}

impl Activity {
    fn before(&self, day: i32, column: usize) -> i128 {
        i128::from(self.totals[column]) + i128::from(self.sums[column]) * i128::from(day.saturating_sub(self.day).max(0))
    }
}

fn activity_at(query: &Query<'_>, run: u32, hour: i32) -> Option<HashMap<u32, Activity>> {
    let table = TableId::AirRegionActivity;
    let snapshot = dashboard_query::cost::storage(|| lanes::at(query.replay.recording(), run, table, hour, None)).ok()?;
    if !matches!(snapshot.status, Status::Complete) {
        return None;
    }
    let view = snapshot.view();
    let field = |name: &str| dashboard_declare::field(table, name).ok();
    let columns = [field("day")?, field("friendly")?, field("enemy")?, field("friendly_total")?, field("enemy_total")?];
    let mut out = HashMap::new();
    for &(key, slot) in &snapshot.rows {
        let number = |at: usize| view.number(columns[at], slot);
        let row = Activity { day: i32::try_from(number(0)?).ok()?, sums: [number(1)?, number(2)?], totals: [number(3)?, number(4)?] };
        out.insert(key as u32, row);
    }
    Some(out)
}

fn bounds(query: &Query<'_>, stretches: &[Stretch], viewpoint: bool) -> Option<HashMap<u32, u128>> {
    let mut out: HashMap<u32, i128> = HashMap::new();
    for stretch in stretches {
        let read = dashboard_query::cost::storage(|| lanes::reads(query.replay.recording(), stretch.run, TableId::AirRegionActivity, stretch.first, stretch.last)).ok()?;
        if read != [(stretch.first, stretch.last)] {
            return None;
        }
        let (start, end) = (activity_at(query, stretch.run, stretch.first)?, activity_at(query, stretch.run, stretch.last)?);
        let (first_day, last_day) = (stretch.first.div_euclid(DAY), stretch.last.div_euclid(DAY));
        let columns: &[usize] = if viewpoint { &[0, 1] } else { &[0] };
        for (region, row) in &end {
            let through: i128 = columns.iter().map(|&column| row.before(last_day, column) + i128::from(row.sums[column])).sum();
            let before: i128 = start.get(region).map_or(0, |row| columns.iter().map(|&column| row.before(first_day, column)).sum());
            *out.entry(*region).or_default() += through - before;
        }
    }
    Some(out.into_iter().map(|(region, bound)| (region, u128::try_from(bound).unwrap_or(0))).collect())
}

fn timeline(source: &Source, params: &AirTimelineParams, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<AirTimelineReply> {
    let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid air branch"))?;
    let query = dashboard_base::queries::query(source, branch)?;
    let index = index::index(source, branch).map_err(|error| dashboard_core::refused(format!("Air sides are not known: {error}")))?;
    dashboard_text::metadata::check_cancel(cancelled)?;
    let (view_first, view_last) = (params.first_hour, params.last_hour);
    let first_day = view_first.div_euclid(DAY);
    let first_hour = first_day.checked_mul(DAY).ok_or_else(|| dashboard_core::invalid("Air timeline start is outside the calendar range"))?;
    let count = (i64::from(view_last.div_euclid(DAY)) - i64::from(first_day) + 1) as usize;
    if count.saturating_mul(2).saturating_sub(1) > MAX_PRODUCT_REPLY_BYTES {
        return Err(dashboard_core::refused("The air timeline's days pass the reply budget: choose a shorter range"));
    }
    let names = names(source, load);
    let mut warnings = names.warnings.clone();
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
                let pinned = params.regions.is_some() || pinned_set.contains(&region);
                AirTimelineRow { region, name, name_missing, pinned, readout, cells }
            })
            .collect()
    };
    let top = params.regions.is_none().then(|| params.top.unwrap_or(TOP));

    let stretches: Vec<Stretch> = query.timeline.within(first_hour, view_last).collect();
    let mut days: Vec<Option<DayAt>> = vec![None; count];
    for (at, stretch) in stretches.iter().enumerate() {
        for day in stretch.first.div_euclid(DAY)..=stretch.last.div_euclid(DAY) {
            days[(day - first_day) as usize] = Some(DayAt { hour: stretch.last.min(day * DAY + DAY - 1), part: at });
        }
    }
    let parts = stretches.par_iter().map(|stretch| part(&query, stretch)).collect::<io::Result<Vec<_>>>().map_err(|error| dashboard_core::refused(format!("Air scores are not summarized in this recording: {error}")))?;
    dashboard_text::metadata::check_cancel(cancelled)?;
    let links: Vec<Option<Links>> = stretches.par_iter().map(|stretch| links(&query, stretch)).collect();

    let mut at_hours: HashMap<usize, DayRows> = HashMap::new();
    let mut unread: HashSet<usize> = HashSet::new();
    for (at, read) in days.iter().enumerate() {
        let Some(read) = read else { continue };
        if read.hour != stretches[read.part].last || (read.hour + 1).rem_euclid(BUCKET) == 0 {
            continue;
        }
        let reads = [Read::all(TableId::RaidAirScores), Read::all(TableId::CountryLinks)];
        let frame = at_hour::frame_with(&query, read.hour, &reads, AirSuperiorityAt::columns, "Air superiority")?;
        match dashboard_text::metadata::recorded_rows::<tables::RaidAirScores>(&frame) {
            Ok(rows) => {
                let complete = frame.complete::<tables::RaidAirScores>().is_ok();
                let links = frame.complete::<tables::CountryLinks>().ok().map(|links| Arc::new(links.to_vec()));
                at_hours.insert(at, (rows.to_vec(), complete, links));
            }
            Err(error) => {
                warnings.push(match at + 1 == count {
                    true => format!("Air scores are not recorded at the view's last hour: {error}"),
                    false => format!("Air scores are not recorded at hour {}, where a stretch of the timeline ends: {error}", read.hour),
                });
                unread.insert(at);
            }
        }
    }
    let charted = Charted { parts: &parts, days: &days, first_day, at_hour: &at_hours };
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
        reads.push(Some(DayRead { hour: day.hour, witnesses: charted.witnesses(at), complete: true, links }));
    }
    dashboard_text::metadata::check_cancel(cancelled)?;

    let chosen_theatres = params.theatres.as_deref().filter(|ids| !ids.is_empty());
    let theatres: Option<Vec<Option<Result<HashSet<u32>, ()>>>> = chosen_theatres.map(|ids| theatre_days(&query, &stretches, &reads, &days, ids));
    let composer = Days { index: &index, holders: &query.holders, days: &reads, rows: &charted, params, theatres: theatres.as_deref() };

    let mut candidates: BTreeSet<u32> = match &params.regions {
        Some(regions) => regions.iter().copied().collect(),
        None => parts.iter().flat_map(|part| part.regions.keys().copied()).chain(at_hours.values().flat_map(|(rows, ..)| rows.iter().map(|row| row.region))).collect(),
    };
    if let Some(theatres) = &theatres {
        let known: Vec<&HashSet<u32>> = theatres.iter().flatten().filter_map(|read| read.as_ref().ok()).collect();
        if known.len() == theatres.iter().flatten().count() {
            let allowed: HashSet<u32> = known.into_iter().flatten().copied().collect();
            candidates.retain(|region| allowed.contains(region));
        }
    }
    let candidates: Vec<u32> = candidates.into_iter().filter(|region| params.regions.is_some() || (!pinned_set.contains(region) && !hidden_set.contains(region))).collect();
    let counted = if count <= 1 || view_last.rem_euclid(DAY) == DAY - 1 { count } else { count - 1 };
    let viewpoint = params.mode == Some(AirMode::Viewpoint);
    let bounds = top.and_then(|_| bounds(&query, &stretches, viewpoint));
    let mut ranked = composer.rank(&candidates, bounds.as_ref(), counted, top, cancelled)?;
    let shown = if params.regions.is_none() && !pinned_list.is_empty() {
        let mut combined = pinned_list.clone();
        combined.append(&mut ranked);
        combined
    } else {
        ranked
    };
    let mut resolver = RecordedSideResolver::new(source, &query, load, cancelled, &mut warnings)?;
    let cells = composer.shown_with_resolver(&shown, &index, &mut resolver, cancelled)?;
    let viewpoint_country_style = (params.mode == Some(AirMode::Viewpoint) && params.viewpoint_country == Some(true)).then(|| {
        let tag = super::viewpoint_name(params.viewpoint.as_ref(), true, &query.holders, view_last);
        (!tag.is_empty()).then(|| country_style_at(tag, view_last, &mut resolver))
    }).flatten();
    let rows = named(&shown, cells.cells, cells.readouts.into_iter().map(Some).collect());
    Ok(AirTimelineReply {
        first_hour,
        last_hour: view_last,
        day_hours: DAY,
        days: reads.iter().map(|read| read.as_ref().map(|read| read.hour)).collect(),
        excluded: cells.excluded,
        sides: cells.sides,
        legend: cells.legend,
        viewpoint: cells.viewpoint,
        viewpoint_country_style,
        gaps,
        rows,
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
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, firstHour: Start, lastHour: End, mode, viewpoint, viewpointCountry, sides, regions, pinnedRegions, hiddenRegions, theatres, top, tile, "regions[]", "pinnedRegions[]", "hiddenRegions[]"];
    type Reply = AirTimelineReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![AirTimelineParams::decl(), AirTimelineReply::decl(), AirTimelineRow::decl(), AirDayCell::decl(), AirDaySides::decl(), AirDayPart::decl(), AirReadout::decl(), AirCountryStyle::decl(), HourSpan::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        timeline(source, params, context, cancelled)
    }
}
