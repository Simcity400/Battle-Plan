//! Naval battles: each live battle from the recorder's NavalBattles (a combat handle read every hour
//! at one province: its first hour, peak and sunk floors) beside its sides as NavalCombatParticipants
//! hold them hour by hour, and each retained result whose battle ended in the window, joined to the
//! battle its source named only through NavalResultObservations. A window reads the battles' rows
//! and their participants' changes over its hours ([`replay::lanes::history`]); the ranking reads
//! each battle's last state ([`replay::lanes::at_each`]) and the sides of the rows it shows.

use super::super::{LoadContext, Product, country_ids, groups::MembershipRun};
use super::sunk;
use crate::Source;
use dashboard_base::{
    catalogue::Refusals,
    products::battles::{BattleBound, BattleCountry, BattleEnd, BattleMoment, BattlePlace, BattleRow, BattleSide, BattleValue, BattleWarning, BattleWindow, BattleWinner},
    span::HourSpan,
};
use dashboard_query::{Query, read};
use replay::{
    at::Seeker,
    columnar_frames::Recording,
    lanes::{self, Cell, Keys, Step},
};
use replay_data::rows::{NavalResultObservationRow as Observation, NavalResultRow, Recorded, TableId, tables};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

/// NavalBattles' columns: first hour, peak ships and hour, then the sunk and convoys-sunk floors
/// of attackers and defenders.
const BATTLE_COLUMNS: [usize; 7] = [0, 1, 2, 3, 4, 5, 6];
/// NavalCombatParticipants' ship, owner and state.
const PARTICIPANT_COLUMNS: [usize; 3] = [0, 1, 2];
const MEASURES: [&str; 3] = ["Ships sunk", "Peak ships", "Length"];

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NavalBattlesParams {
    pub branch_id: String,
    pub window: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NavalBattlesManifestParams {
    pub branch_id: String,
}

/// Countries chosen by tag, or by membership runs; every country when neither.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NavalBattleCountryFilter {
    pub tags: Option<Vec<String>>,
    pub membership: Option<Vec<MembershipRun>>,
    pub unobserved: Vec<HourSpan>,
}

impl NavalBattleCountryFilter {
    /// Whether a battle whose sides hold these countries at `hour` is chosen: a side not known
    /// whole is never left out, as its countries may be.
    pub fn matches(&self, sides: &[BattleSide], hour: i32) -> bool {
        if self.tags.is_none() && self.membership.is_none() || sides.iter().any(|side| side.countries_unknown.is_some()) {
            return true;
        }
        let unobserved = self.unobserved.iter().any(|span| span.first_hour <= hour && hour <= span.last_hour);
        sides.iter().flat_map(|side| &side.countries).any(|country| {
            self.tags.as_ref().is_none_or(|tags| tags.contains(&country.tag))
                && self.membership.as_ref().is_none_or(|runs| unobserved || runs.iter().any(|run| run.tag == country.tag && run.first_hour <= hour && hour <= run.last_hour))
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NavalBattlesRankingParams {
    pub branch_id: String,
    pub first_hour: i32,
    pub last_hour: i32,
    pub top_n: f64,
    pub measure: String,
    pub countries: NavalBattleCountryFilter,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NavalBattleRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub row: BattleRow,
    pub peak_hour: Option<i32>,
    /// Ships sunk, Peak ships and Length.
    pub measures: Vec<BattleValue>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NavalBattleHistory {
    pub id: String,
    pub province_id: u32,
    pub runs: Vec<NavalBattleRun>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NavalBattlesReply {
    pub window: usize,
    pub interval: BattleWindow,
    pub rows: Vec<NavalBattleHistory>,
    /// The window's hours whose live combats were read.
    pub observed: Vec<(i32, i32)>,
    pub warnings: Vec<BattleWarning>,
    pub content_warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NavalBattlesManifestReply {
    pub windows: Vec<BattleWindow>,
    pub available: bool,
    pub unavailable: Option<String>,
    pub content_warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NavalBattlesRankingReply {
    pub rows: Vec<NavalBattleHistory>,
    pub census_hour: Option<i32>,
    pub effective_measure: String,
    pub warnings: Vec<BattleWarning>,
    pub content_warnings: Vec<String>,
}

macro_rules! content_warnings {
    ($($reply:ty),+) => {$(
        impl dashboard_base::products::ReplyWarnings for $reply {
            fn warnings_mut(&mut self) -> &mut Vec<String> {
                &mut self.content_warnings
            }
        }
    )+};
}
content_warnings!(NavalBattlesReply, NavalBattlesManifestReply, NavalBattlesRankingReply);

fn count(label: &str, value: Option<i64>, at_least: bool, reason: &str) -> BattleValue {
    match value {
        Some(value) => BattleValue::Known { label: label.into(), value: value as f64, at_least: at_least.then_some(true) },
        None => BattleValue::Unknown { label: label.into(), value: (), unknown: reason.into() },
    }
}

fn recorded<T: Copy>(cell: &Recorded<T>) -> Option<T> {
    match cell {
        Recorded::Value(value) => Some(*value),
        Recorded::NotRecorded => None,
    }
}

fn number(cell: Cell) -> Option<i64> {
    match cell {
        Cell::Number(value) => Some(value),
        _ => None,
    }
}

/// The cells `steps` hold at `hour`; none where the row is not shown.
fn step_at(steps: &[Step], hour: i32) -> Option<&[Cell]> {
    let at = steps.partition_point(|step| step.hour <= hour).checked_sub(1)?;
    steps[at].cells.as_deref()
}

/// A battle row's cells at an hour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BattleCells {
    pub first_hour: i32,
    pub peak: Option<i64>,
    pub peak_hour: Option<i32>,
    /// Sunk and convoys sunk, attackers then defenders.
    pub sunk: [Option<i64>; 2],
    pub convoys_sunk: [Option<i64>; 2],
}

impl BattleCells {
    pub fn of(cells: &[Cell]) -> Option<Self> {
        let get = |at: usize| cells.get(at).copied().and_then(number);
        Some(Self {
            first_hour: i32::try_from(get(0)?).ok()?,
            peak: get(1),
            peak_hour: get(2).and_then(|hour| i32::try_from(hour).ok()),
            sunk: [get(3), get(4)],
            convoys_sunk: [get(5), get(6)],
        })
    }

    pub fn ships_sunk(&self) -> Option<i64> {
        self.sunk[0]?.checked_add(self.sunk[1]?)
    }
}

/// One side of a battle at an hour, from its participants.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SideAt {
    pub ships: i64,
    pub convoys: i64,
    pub countries: BTreeSet<String>,
    pub unknown: bool,
}

/// The sides `participants` (a combat's rows, keyed side above battle slot) hold at `hour`: a
/// participant with no ship is a convoy; an owner no tag names leaves its side's countries unknown.
pub fn sides_at<'a>(participants: impl Iterator<Item = (u128, Option<&'a [Cell]>)>, tag: &dyn Fn(u32) -> Option<String>) -> [SideAt; 2] {
    let mut sides: [SideAt; 2] = Default::default();
    for (key, cells) in participants {
        let Some(cells) = cells else { continue };
        let side = &mut sides[((key >> 32) & 1) as usize];
        let (Some(ship), Some(owner)) = (cells.first().copied().and_then(number), cells.get(1).copied().and_then(number)) else {
            side.unknown = true;
            continue;
        };
        if ship == 0 {
            side.convoys += 1;
        } else {
            side.ships += 1;
        }
        match tag(owner as u32) {
            Some(tag) => {
                side.countries.insert(tag);
            }
            None => side.unknown = true,
        }
    }
    sides
}

/// A live battle's run as the reply draws it.
pub fn live_run(id: &str, cells: &BattleCells, sides: Option<&[SideAt; 2]>, first: i32, last: i32) -> NavalBattleRun {
    let reason = "Participants not recorded";
    NavalBattleRun {
        first_hour: first,
        last_hour: last,
        row: BattleRow {
            id: id.to_owned(),
            place: BattlePlace { name: None, hour: cells.first_hour, named_at: None },
            start: Some(BattleMoment { hour: cells.first_hour, bound: BattleBound::By }),
            end: None,
            winner: BattleWinner::Unknown { unknown: "Result not recorded".into() },
            sides: (0..2)
                .map(|which| {
                    let side = sides.map(|sides| &sides[which]);
                    BattleSide {
                        label: if which == 0 { "Attackers" } else { "Defenders" }.into(),
                        countries: side.into_iter().flat_map(|side| &side.countries).map(|tag| BattleCountry { tag: tag.clone(), hour: last }).collect(),
                        countries_unknown: side.is_none_or(|side| side.unknown).then(|| "Countries not recorded".into()),
                        counts: vec![count("Ships", side.map(|side| side.ships), false, reason), count("Convoys", side.map(|side| side.convoys), false, reason)],
                        losses: vec![count("Sunk", cells.sunk[which], true, reason), count("Convoys sunk", cells.convoys_sunk[which], true, reason)],
                    }
                })
                .collect(),
        },
        peak_hour: cells.peak_hour,
        measures: vec![
            count("Ships sunk", cells.ships_sunk(), true, reason),
            count("Peak ships", cells.peak, true, "Simultaneous participants not recorded"),
            count("Length", Some(i64::from(last) - i64::from(cells.first_hour)), true, "Not recorded"),
        ],
    }
}

/// A battle's id: its combat handle and first hour.
pub fn battle_id(combat: u64, first_hour: i32) -> String {
    format!("combat:{combat}:{first_hour}")
}

/// A battle id's combat handle and first hour.
pub fn parse_id(id: &str) -> Option<(u64, i32)> {
    let mut parts = id.strip_prefix("combat:")?.split(':');
    let combat = parts.next()?.parse().ok()?;
    let first = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((combat, first))
}

/// Raw country ids named at an hour, through the run's links and the timeline's holders.
struct Tags<'q> {
    links: country_ids::RunLinks,
    names: country_ids::Names,
    _query: std::marker::PhantomData<&'q ()>,
}

impl<'q> Tags<'q> {
    fn new(query: &'q Query<'_>, run: u32) -> io::Result<Self> {
        Ok(Self { links: country_ids::run_links(query.replay.recording(), run)?, names: country_ids::Names::new(&query.holders), _query: std::marker::PhantomData })
    }

    fn at(&self, raw: u32, hour: i32) -> Option<String> {
        self.names.tag_at(&self.links, raw, hour).map(str::to_owned)
    }
}

/// The battles of `first..=last` of `run`, each with its runs, by combat then first hour.
fn live_battles(source: &Source, query: &Query<'_>, run: u32, first: i32, last: i32, cancelled: &AtomicBool) -> io::Result<Vec<NavalBattleHistory>> {
    let recording = query.replay.recording();
    let lives = crate::answer::lifetimes(source, run, TableId::NavalBattles)?;
    let keys: Vec<u128> = lives.iter().filter(|&&(_, born, gone)| born <= i64::from(last) && gone >= i64::from(first)).map(|&(key, ..)| key).collect::<BTreeSet<_>>().into_iter().collect();
    if keys.is_empty() {
        return Ok(Vec::new());
    }
    let battles = lanes::history(recording, run, TableId::NavalBattles, &BATTLE_COLUMNS, Keys::These(&keys), first, last)?;
    let provinces = lanes::history(recording, run, TableId::NavalCombats, &[0], Keys::These(&keys), first, last)?;
    crate::metadata::check_cancel(cancelled)?;
    let participants = lanes::history(recording, run, TableId::NavalCombatParticipants, &PARTICIPANT_COLUMNS, Keys::All, first, last)?;
    let mut by_combat: HashMap<u64, Vec<&(u128, Vec<Step>)>> = HashMap::new();
    for held in &participants.keys {
        by_combat.entry((held.0 >> 33) as u64).or_default().push(held);
    }
    let tags = Tags::new(query, run)?;
    let province_of: HashMap<u128, &Vec<Step>> = provinces.keys.iter().map(|(key, steps)| (*key, steps)).collect();
    let mut out: BTreeMap<(u64, i32), NavalBattleHistory> = BTreeMap::new();
    for (key, steps) in &battles.keys {
        crate::metadata::check_cancel(cancelled)?;
        let combat = *key as u64;
        let members = by_combat.get(&combat).map(Vec::as_slice).unwrap_or_default();
        let mut edges: Vec<i32> = steps.iter().map(|step| step.hour).chain(members.iter().flat_map(|(_, steps)| steps.iter().map(|step| step.hour))).filter(|hour| (first..=last).contains(hour)).collect();
        edges.push(first);
        edges.sort_unstable();
        edges.dedup();
        for (index, &start) in edges.iter().enumerate() {
            let end = edges.get(index + 1).map_or(last, |next| next - 1);
            let Some(cells) = step_at(steps, start).and_then(BattleCells::of) else { continue };
            let sides = cells.peak.is_some().then(|| sides_at(members.iter().map(|(key, steps)| (*key, step_at(steps, start))), &|raw| tags.at(raw, start)));
            let id = battle_id(combat, cells.first_hour);
            let history = out.entry((combat, cells.first_hour)).or_insert_with(|| NavalBattleHistory {
                id: id.clone(),
                province_id: province_of.get(key).and_then(|steps| step_at(steps, start)).and_then(|cells| cells.first().copied().and_then(number)).unwrap_or(0) as u32,
                runs: Vec::new(),
            });
            let next = live_run(&id, &cells, sides.as_ref(), start, end);
            match history.runs.last_mut() {
                // A run is cut where what it shows changes; its length is read at its last hour.
                Some(previous) if previous.last_hour + 1 == start && same(previous, &next) => {
                    let first_hour = previous.first_hour;
                    *previous = live_run(&id, &cells, sides.as_ref(), first_hour, end);
                }
                _ => history.runs.push(next),
            }
        }
    }
    Ok(out.into_values().collect())
}

/// Whether two runs show the same battle state but for their hours and length.
fn same(a: &NavalBattleRun, b: &NavalBattleRun) -> bool {
    let shown = |run: &NavalBattleRun| serde_json::to_string(&(&run.row.sides.iter().map(|side| (&side.counts, &side.losses, side.countries.iter().map(|c| &c.tag).collect::<Vec<_>>(), &side.countries_unknown)).collect::<Vec<_>>(), run.peak_hour, &run.measures[..2])).ok();
    shown(a) == shown(b)
}

/// The retained results whose battle ended within `first..=last` of `run`, each one run from its
/// result hour.
fn results(source: &Source, query: &Query<'_>, run: u32, first: i32, last: i32) -> io::Result<Vec<NavalBattleHistory>> {
    let recording = query.replay.recording();
    let lives = crate::answer::lifetimes(source, run, TableId::NavalResults)?;
    let ids: Vec<u128> = lives.iter().filter(|&&(_, born, gone)| born <= i64::from(last) && gone >= i64::from(first)).map(|&(key, ..)| key).collect::<BTreeSet<_>>().into_iter().collect();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let frame = replay::at::frame(recording, run, last, &[
        read::Read { table: TableId::NavalResults, columns: None, keys: Keys::These(&ids) },
        read::Read { table: TableId::NavalResultObservations, columns: None, keys: Keys::These(&ids) },
        read::Read::all(TableId::NavalResultCountries),
    ])?;
    let Ok(rows) = frame.complete::<tables::NavalResults>() else { return Ok(Vec::new()) };
    let observations: HashMap<u64, &Observation> = frame.complete::<tables::NavalResultObservations>().map(|rows| rows.iter().map(|row| (row.result, row)).collect()).unwrap_or_default();
    let countries = frame.complete::<tables::NavalResultCountries>().ok();
    let tags = Tags::new(query, run)?;
    let mut out = Vec::new();
    for row in rows.iter().filter(|row| (first..=last).contains(&row.result_hour)) {
        let observed = observations.get(&row.id).copied().filter(|observed| observed.refusal == Observation::OBSERVED);
        let named = [0u8, 1].map(|which| {
            countries.map(|rows| rows.iter().filter(|country| country.result == row.id && country.list == which).map(|country| tags.at(country.country, row.result_hour.min(last))).collect())
        });
        out.push(result_history(row, observed, named, last));
    }
    Ok(out)
}

/// A retained result's battle: one run from its result hour through `last`. `named` holds each
/// side's countries as its list named them (none where the lists were not read, a tag none where
/// no country held the id); like every run's, they are dated at the run's last hour.
pub fn result_history(row: &NavalResultRow, observed: Option<&Observation>, named: [Option<Vec<Option<String>>>; 2], last: i32) -> NavalBattleHistory {
    let value = |cell: &Recorded<u32>| -> Option<i64> { recorded(cell).map(i64::from) };
    let start = observed.and_then(|observed| recorded(&observed.first_hour));
    let end_read = observed.and_then(|observed| recorded(&observed.last_hour));
    let roster = "Ship roster not recorded";
    let sides: Vec<BattleSide> = named
        .iter()
        .enumerate()
        .map(|(which, named)| {
            let (ships, convoys, sunk, convoys_sunk) = match observed {
                Some(o) if which == 0 => (value(&o.attacker_ships), value(&o.attacker_convoys), value(&o.attacker_sunk), value(&o.attacker_convoys_sunk)),
                Some(o) => (value(&o.defender_ships), value(&o.defender_convoys), value(&o.defender_sunk), value(&o.defender_convoys_sunk)),
                None => (None, None, None, None),
            };
            BattleSide {
                label: if which == 0 { "Side 1" } else { "Side 2" }.into(),
                countries: named.iter().flatten().flatten().collect::<BTreeSet<_>>().into_iter().map(|tag| BattleCountry { tag: tag.clone(), hour: last }).collect(),
                countries_unknown: named.as_ref().is_none_or(|named| named.iter().any(Option::is_none)).then(|| "Countries not recorded".into()),
                counts: vec![count("Ships", ships, false, roster), count("Convoys", convoys, false, roster)],
                losses: vec![count("Sunk", sunk, false, roster), count("Convoys sunk", convoys_sunk, false, roster)],
            }
        })
        .collect();
    let reason = "Result not observed";
    let sunk: Option<i64> = observed.and_then(|o| value(&o.attacker_sunk)?.checked_add(value(&o.defender_sunk)?));
    let id = format!("result:{}", row.id);
    NavalBattleHistory {
        id: id.clone(),
        province_id: row.province,
        runs: vec![NavalBattleRun {
            first_hour: row.result_hour,
            last_hour: last,
            row: BattleRow {
                id,
                place: BattlePlace { name: None, hour: row.result_hour, named_at: None },
                start: start.map(|hour| BattleMoment { hour, bound: BattleBound::By }),
                end: Some(BattleEnd::Moment(BattleMoment { hour: row.result_hour, bound: BattleBound::Exact })),
                winner: match row.winning_side {
                    1 => BattleWinner::Side { side: 0 },
                    2 => BattleWinner::Side { side: 1 },
                    _ => BattleWinner::Unknown { unknown: "No winner recorded".into() },
                },
                sides,
            },
            peak_hour: observed.and_then(|o| recorded(&o.peak_hour)),
            measures: vec![
                count("Ships sunk", sunk, false, reason),
                count("Peak ships", observed.and_then(|o| value(&o.peak_ships)), true, reason),
                count("Length", start.zip(end_read).map(|(start, end)| i64::from(end) - i64::from(start)), true, reason),
            ],
        }],
    }
}

/// Runs of hours `table` was read within `first..=last`.
fn observed(recording: &Recording, run: u32, first: i32, last: i32) -> io::Result<Vec<(i32, i32)>> {
    lanes::reads(recording, run, TableId::NavalCombats, first, last)
}

fn windows(query: &Query<'_>) -> io::Result<Vec<BattleWindow>> {
    Ok(super::super::army::windows(query)?.into_iter().map(|span| BattleWindow { first_hour: span.first_hour, last_hour: span.last_hour }).collect())
}

fn branch(branch_id: &str) -> io::Result<u64> {
    branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))
}

fn valid_branch(branch_id: &str) -> Result<(), &'static str> {
    branch_id.parse::<u64>().map(drop).map_err(|_| "Invalid branch")
}

pub struct NavalBattles;
pub struct NavalBattlesManifest;
pub struct NavalBattlesRanking;
pub struct NavalBattleDetails;

impl Product for NavalBattlesManifest {
    const KEY: &'static str = "naval-battles-manifest";
    const CONTENT_DEPENDENT: bool = false;
    type Params = NavalBattlesManifestParams;
    type Reply = NavalBattlesManifestReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        let mut types = vec![NavalBattlesManifestParams::decl(), NavalBattlesManifestReply::decl()];
        types.extend(dashboard_base::products::battles::types());
        types
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        valid_branch(&params.branch_id)
    }

    fn load(source: &Source, params: &Self::Params, _context: &LoadContext, _cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let query = crate::answer::query(source, branch(&params.branch_id)?)?;
        let windows = windows(&query)?;
        let recording = query.replay.recording();
        let mut read_any = false;
        for stretch in &query.timeline.stretches {
            if !observed(recording, stretch.run, stretch.first, stretch.last)?.is_empty() {
                read_any = true;
                break;
            }
        }
        Ok(NavalBattlesManifestReply {
            windows,
            available: read_any,
            unavailable: (!read_any).then(|| "Naval combats were not recorded on this branch".into()),
            content_warnings: Vec::new(),
        })
    }
}

impl Product for NavalBattles {
    const KEY: &'static str = "naval-battles";
    const CONTENT_DEPENDENT: bool = false;
    type Params = NavalBattlesParams;
    type Reply = NavalBattlesReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![NavalBattlesParams::decl(), NavalBattlesReply::decl(), NavalBattleHistory::decl(), NavalBattleRun::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        valid_branch(&params.branch_id)
    }

    fn load(source: &Source, params: &Self::Params, _context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let query = crate::answer::query(source, branch(&params.branch_id)?)?;
        let interval = windows(&query)?.get(params.window).cloned().ok_or_else(|| invalid("Unknown history window"))?;
        let (first, last) = (interval.first_hour, interval.last_hour);
        let run = query.timeline.at(first).ok_or_else(|| invalid("The window's hours are not recorded"))?.run;
        let mut rows = live_battles(source, &query, run, first, last, cancelled)?;
        rows.extend(results(source, &query, run, first, last)?);
        let reply = NavalBattlesReply {
            window: params.window,
            interval,
            rows,
            observed: observed(query.replay.recording(), run, first, last)?,
            warnings: Vec::new(),
            content_warnings: Vec::new(),
        };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

/// A ranked battle's last state: its combat, cells, and the hour they were read.
struct Ranked {
    combat: u64,
    cells: BattleCells,
    hour: i32,
    run: u32,
}

fn measure_of(ranked: &Ranked, measure: &str) -> Option<i64> {
    match measure {
        "Ships sunk" => ranked.cells.ships_sunk(),
        "Peak ships" => ranked.cells.peak,
        _ => Some(i64::from(ranked.hour) - i64::from(ranked.cells.first_hour)),
    }
}

impl Product for NavalBattlesRanking {
    const KEY: &'static str = "naval-battles-ranking";
    const CONTENT_DEPENDENT: bool = false;
    type Params = NavalBattlesRankingParams;
    type Reply = NavalBattlesRankingReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![NavalBattlesRankingParams::decl(), NavalBattlesRankingReply::decl(), NavalBattleCountryFilter::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        valid_branch(&params.branch_id)?;
        if !MEASURES.contains(&params.measure.as_str()) {
            return Err("Unknown naval battle measure");
        }
        if params.first_hour > params.last_hour || !(params.top_n.is_finite() && params.top_n >= 1.0) {
            return Err("Invalid naval battle ranking");
        }
        Ok(())
    }

    fn load(source: &Source, params: &Self::Params, _context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let query = crate::answer::query(source, branch(&params.branch_id)?)?;
        let recording = query.replay.recording();
        let (from, to) = (params.first_hour, params.last_hour);
        let mut ranked: Vec<Ranked> = Vec::new();
        for stretch in query.timeline.within(from, to) {
            crate::metadata::check_cancel(cancelled)?;
            let lives = crate::answer::lifetimes(source, stretch.run, TableId::NavalBattles)?;
            // Each battle as it stood at its last hour within the range.
            let asks: Vec<(i32, u128)> = lives
                .iter()
                .filter(|&&(_, born, gone)| born <= i64::from(stretch.last) && gone >= i64::from(stretch.first))
                .map(|&(key, _, gone)| (gone.min(i64::from(stretch.last)) as i32, key))
                .collect();
            lanes::at_each(recording, stretch.run, TableId::NavalBattles, None, &asks, &mut |hour, key, row| {
                let Some((view, slot)) = row else { return Ok(()) };
                let cells: Vec<Cell> = BATTLE_COLUMNS.iter().map(|&column| view.number(column, slot).map_or(Cell::Unrecorded, Cell::Number)).collect();
                if let Some(cells) = BattleCells::of(&cells) {
                    ranked.push(Ranked { combat: key as u64, cells, hour, run: stretch.run });
                }
                Ok(())
            })?;
        }
        ranked.sort_by(|a, b| measure_of(b, &params.measure).cmp(&measure_of(a, &params.measure)).then_with(|| a.hour.cmp(&b.hour)).then_with(|| a.combat.cmp(&b.combat)));
        let top = params.top_n as usize;
        let mut rows = Vec::new();
        let mut tags: HashMap<u32, Tags<'_>> = HashMap::new();
        for battle in &ranked {
            if rows.len() >= top {
                break;
            }
            crate::metadata::check_cancel(cancelled)?;
            let low = u128::from(battle.combat) << 33;
            let snapshots = read::at(recording, battle.run, &[read::Read { table: TableId::NavalCombatParticipants, columns: Some(&PARTICIPANT_COLUMNS[..]), keys: Keys::Range(low, low | ((1u128 << 33) - 1)) }], battle.hour)?;
            let snapshot = snapshots.first().ok_or_else(|| invalid("Naval combat participants were not read"))?;
            let view = snapshot.view();
            let named = match tags.entry(battle.run) {
                std::collections::hash_map::Entry::Occupied(held) => held.into_mut(),
                std::collections::hash_map::Entry::Vacant(slot) => slot.insert(Tags::new(&query, battle.run)?),
            };
            let cells: Vec<(u128, Vec<Cell>)> = snapshot.rows.iter().map(|&(key, slot)| (key, PARTICIPANT_COLUMNS.iter().map(|&column| view.number(column, slot).map_or(Cell::Unrecorded, Cell::Number)).collect())).collect();
            let sides = battle.cells.peak.is_some().then(|| sides_at(cells.iter().map(|(key, cells)| (*key, Some(cells.as_slice()))), &|raw| named.at(raw, battle.hour)));
            let id = battle_id(battle.combat, battle.cells.first_hour);
            let run = live_run(&id, &battle.cells, sides.as_ref(), battle.hour, battle.hour);
            if !params.countries.matches(&run.row.sides, battle.hour) {
                continue;
            }
            let province = read::at(recording, battle.run, &[read::Read { table: TableId::NavalCombats, columns: Some(&[0][..]), keys: Keys::These(&[u128::from(battle.combat)][..]) }], battle.hour)?;
            let province_id = province.first().and_then(|snapshot| snapshot.rows.first().and_then(|&(_, slot)| snapshot.view().number(0, slot))).unwrap_or(0) as u32;
            rows.push(NavalBattleHistory { id, province_id, runs: vec![run] });
        }
        let census_hour = super::super::land_battles::last_read(&query, TableId::NavalCombats, to)?;
        Ok(NavalBattlesRankingReply { rows, census_hour, effective_measure: params.measure.clone(), warnings: Vec::new(), content_warnings: Vec::new() })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NavalBattleDetailRequest {
    pub id: String,
    pub hour: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NavalBattleDetailsParams {
    pub branch_id: String,
    pub battles: Vec<NavalBattleDetailRequest>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
pub struct NavalBattleFleetNames {
    pub names: Vec<String>,
    pub unknown: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NavalBattleNamedShip {
    pub sink_hour: i32,
    pub known_at: i32,
    pub name: Option<String>,
    pub class_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NavalBattleDetail {
    pub id: String,
    pub hour: i32,
    /// The hour its fleets are read at: the hour asked.
    pub roster_hour: Option<i32>,
    pub fleets: [NavalBattleFleetNames; 2],
    pub named_ships: Vec<NavalBattleNamedShip>,
    pub ship_read: Option<i32>,
    pub unavailable: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
pub struct NavalBattleDetailsReply {
    pub rows: Vec<NavalBattleDetail>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(NavalBattleDetailsReply);

fn unknown_fleets(reason: &str) -> [NavalBattleFleetNames; 2] {
    std::array::from_fn(|_| NavalBattleFleetNames { names: Vec::new(), unknown: Some(reason.into()) })
}

/// Each side's fleets at `hour` in each of `combats` (in their order): its task forces' fleets, named as the game stored
/// them. Battles at one hour share the three reads.
fn fleets_at(seeker: &mut Seeker<'_>, run: u32, hour: i32, combats: &[u64], content: &sunk::Content, refused: &mut Refusals) -> io::Result<Vec<[NavalBattleFleetNames; 2]>> {
    let unknown = |reason: &str| combats.iter().map(|_| unknown_fleets(reason)).collect::<Vec<_>>();
    let Some(cat) = content.catalogue() else { return Ok(unknown("Fleet roster or content not recorded")) };
    let keys: Vec<u128> = combats.iter().map(|&combat| u128::from(combat)).collect::<BTreeSet<_>>().into_iter().collect();
    let mut frame_of = |table: TableId, keys: &[u128]| dashboard_query::cost::storage(|| seeker.frame(run, hour, &[read::Read { table, columns: None, keys: Keys::These(keys) }]));
    let frame = frame_of(TableId::NavalCombatDetails, &keys)?;
    let Ok(details) = frame.complete::<tables::NavalCombatDetails>() else { return Ok(unknown("Fleet identities not recorded")) };
    let detail_of: HashMap<u64, _> = details.iter().map(|row| (row.combat, row)).collect();
    let tasks: Vec<u128> = details.iter().flat_map(|row| row.sides.iter().flat_map(|side| side.units.iter().map(|&unit| u128::from(unit)))).collect::<BTreeSet<_>>().into_iter().collect();
    let frame = frame_of(TableId::TaskForces, &tasks)?;
    let Ok(forces) = frame.complete::<tables::TaskForces>() else { return Ok(unknown("Fleet identities not recorded")) };
    let fleet_of: HashMap<u64, u64> = forces.iter().map(|row| (row.id, row.fleet)).collect();
    let fleet_keys: Vec<u128> = fleet_of.values().map(|&fleet| u128::from(fleet)).collect::<BTreeSet<_>>().into_iter().collect();
    let frame = frame_of(TableId::Fleets, &fleet_keys)?;
    let Ok(fleets) = frame.complete::<tables::Fleets>() else { return Ok(unknown("Fleet identities not recorded")) };
    let names: HashMap<u64, &Recorded<Option<Vec<u8>>>> = fleets.iter().map(|row| (row.id, &row.name)).collect();
    let mut all = Vec::with_capacity(combats.len());
    for combat in combats {
        let Some(detail) = detail_of.get(combat) else {
            all.push(unknown_fleets("Fleet identities not recorded"));
            continue;
        };
        let mut out: [NavalBattleFleetNames; 2] = std::array::from_fn(|_| NavalBattleFleetNames { names: Vec::new(), unknown: None });
        for (side, input) in out.iter_mut().zip(&detail.sides) {
            let (mut seen, mut missing) = (BTreeSet::new(), 0);
            for unit in &input.units {
                let Some((fleet, name)) = fleet_of.get(unit).and_then(|fleet| Some((*fleet, *names.get(fleet)?))) else {
                    missing += 1;
                    continue;
                };
                if !seen.insert(fleet) {
                    continue;
                }
                match name {
                    Recorded::Value(Some(bytes)) => match dashboard_text::scripted_text::stored_text(&cat.localisation, &cat.recorded_language, bytes, "fleet name")
                        .and_then(|text| installed_content_localisation::localisation::display_text(text.as_bytes()))
                    {
                        Ok(name) if !name.is_empty() => side.names.push(name),
                        Ok(_) => missing += 1,
                        Err(reason) => {
                            refused.note(reason, format!("fleet {fleet}"));
                            missing += 1;
                        }
                    },
                    _ => missing += 1,
                }
            }
            if missing != 0 {
                side.unknown = Some(format!("{missing} fleet names not recorded"));
            }
        }
        all.push(out);
    }
    Ok(all)
}

impl Product for NavalBattleDetails {
    const KEY: &'static str = "naval-battle-details";
    const CONTENT_DEPENDENT: bool = true;
    type Params = NavalBattleDetailsParams;
    type Reply = NavalBattleDetailsReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![NavalBattleDetailsParams::decl(), NavalBattleDetailsReply::decl(), NavalBattleDetailRequest::decl(), NavalBattleFleetNames::decl(), NavalBattleNamedShip::decl(), NavalBattleDetail::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        valid_branch(&params.branch_id)?;
        if params.battles.iter().any(|battle| parse_id(&battle.id).is_none() && !battle.id.starts_with("result:")) {
            return Err("Invalid naval battle");
        }
        Ok(())
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let query = crate::answer::query(source, branch(&params.branch_id)?)?;
        let recording = query.replay.recording();
        let content = sunk::Content::load(source, context);
        let mut refused = Refusals::default();
        let mut reply = NavalBattleDetailsReply { rows: Vec::with_capacity(params.battles.len()), warnings: content.warnings() };
        // The battles asked at each hour of each stretch (its run and first hour): what battles at one hour share (their
        // fleets' reads, the sinking list and its names) is read once, and the sinking list's reads once a stretch.
        let mut at: BTreeMap<(u32, i32, i32), Vec<(usize, u64)>> = BTreeMap::new();
        for (index, asked) in params.battles.iter().enumerate() {
            let mut detail = NavalBattleDetail { id: asked.id.clone(), hour: asked.hour, roster_hour: None, fleets: unknown_fleets("Fleet roster not recorded"), named_ships: Vec::new(), ship_read: None, unavailable: None };
            match (parse_id(&asked.id), query.timeline.at(asked.hour)) {
                (Some((combat, _)), Some(stretch)) => {
                    detail.roster_hour = Some(asked.hour);
                    at.entry((stretch.run, stretch.first, asked.hour)).or_default().push((index, combat));
                }
                _ => detail.unavailable = Some("Details are kept for live battles at a recorded hour".into()),
            }
            reply.rows.push(detail);
        }
        let mut last: HashMap<(u32, i32), i32> = HashMap::new();
        for &(run, first, hour) in at.keys() {
            last.insert((run, first), hour);
        }
        let mut stretch: Option<(u32, i32)> = None;
        let (mut reads, mut lists) = (Vec::new(), HashMap::new());
        // The hours asked are read in order through one seeker: each table restored once a reply, not once an hour.
        let mut seeker = Seeker::new(recording)?;
        for (&(run, first, hour), battles) in &at {
            crate::metadata::check_cancel(cancelled)?;
            if stretch != Some((run, first)) {
                stretch = Some((run, first));
                reads = lanes::reads(recording, run, TableId::SunkShips, first, last[&(run, first)])?;
                lists = HashMap::new();
            }
            let combats: Vec<u64> = battles.iter().map(|&(_, combat)| combat).collect();
            for (&(index, _), fleets) in battles.iter().zip(fleets_at(&mut seeker, run, hour, &combats, &content, &mut refused)?) {
                reply.rows[index].fleets = fleets;
            }
            // The list as its last read at or before the hour held it: a frame at a block's first hour gave none.
            let through: Vec<(i32, i32)> = reads.iter().take_while(|read| read.0 <= hour).map(|&(start, end)| (start, end.min(hour))).collect();
            let Some(&(_, read)) = through.last() else { continue };
            if !lists.contains_key(&read) {
                lists.insert(read, sunk::sinkings_seen(&mut seeker, run, read)?);
            }
            let Some((read_at, entries)) = &lists[&read] else { continue };
            let asked: BTreeSet<u64> = combats.iter().copied().collect();
            let entries: Vec<_> = entries.iter().filter(|entry| asked.contains(&entry.battle)).cloned().collect();
            let named = sunk::ship_names(&mut seeker, run, hour, &entries, &content, &mut refused)?;
            for &(index, combat) in battles {
                let row = &mut reply.rows[index];
                row.ship_read = Some(*read_at);
                row.named_ships = entries
                    .iter()
                    .zip(&named)
                    .filter(|(entry, _)| entry.battle == combat)
                    .map(|(entry, (name, class_name))| NavalBattleNamedShip {
                        sink_hour: entry.sink_hour,
                        known_at: sunk::known_at(&through, entry.sink_hour).unwrap_or(*read_at),
                        name: name.clone(),
                        class_name: class_name.clone(),
                    })
                    .collect();
            }
        }
        reply.warnings.extend(refused.lines("Naval battle details"));
        Ok(reply)
    }
}
