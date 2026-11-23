
use dashboard_base::products::Country;
use dashboard_base::{products::{LoadContext, Product}, queries::country_ids};
use dashboard_world::products::groups::MembershipRun;
use super::sunk;
use dashboard_base::Source;
use dashboard_base::{
    catalogue::Refusals,
    products::battles::{BattleBound, BattleCountry, BattleEnd, BattleMoment, BattlePlace, BattleRow, BattleSide, BattleValue, BattleWarning, BattleWindow, BattleWinner},
    span::HourSpan,
};
use dashboard_query::{Query, read};
use replay_lanes::{at::Seeker, lanes::{self, Cell, Keys, Step}};
use replay_recording::columnar_frames::Recording;
use replay_data::rows::{NavalResultObservationRow as Observation, NavalResultRow, Recorded, TableId, tables};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const BATTLE_COLUMNS: [usize; 7] = [0, 1, 2, 3, 4, 5, 6];
const PARTICIPANT_COLUMNS: [usize; 3] = [0, 1, 2];
const MEASURES: [&str; 3] = ["Ships sunk", "Peak ships", "Length"];

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NavalBattlesParams {
    pub branch_id: String,
    pub window: usize,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NavalBattlesManifestParams {
    pub branch_id: String,
}

#[derive(Clone, Debug, PartialEq, Default, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NavalBattleCountryFilter {
    pub countries: Option<Vec<Country>>,
    pub membership: Option<Vec<MembershipRun>>,
    pub unobserved: Vec<HourSpan>,
}

impl NavalBattleCountryFilter {
    pub fn matches(&self, holders: &dashboard_query::identity::Holders, sides: &[BattleSide], hour: i32) -> bool {
        if self.countries.is_none() && self.membership.is_none() || sides.iter().any(|side| side.countries_unknown.is_some()) {
            return true;
        }
        let unobserved = self.unobserved.iter().any(|span| span.first_hour <= hour && hour <= span.last_hour);
        sides.iter().flat_map(|side| &side.countries).any(|country| {
            self.countries.as_ref().is_none_or(|countries| countries.iter().any(|chosen| holders.goes_by(chosen, &country.tag, hour)))
                && self.membership.as_ref().is_none_or(|runs| unobserved || runs.iter().any(|run| run.tag == country.tag && run.first_hour <= hour && hour <= run.last_hour))
        })
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
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

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NavalBattleRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub province_id: Option<u32>,
    pub row: BattleRow,
    pub peak_hour: Option<i32>,
    #[doc = " Ships sunk, Peak ships and Length."]
    pub measures: Vec<BattleValue>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NavalBattleHistory {
    pub id: String,
    pub province_id: Option<u32>,
    pub runs: Vec<NavalBattleRun>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NavalBattlesReply {
    pub window: usize,
    pub interval: BattleWindow,
    pub rows: Vec<NavalBattleHistory>,
    #[doc = " The window's hours whose live combats were read."]
    pub observed: Vec<(i32, i32)>,
    pub warnings: Vec<BattleWarning>,
    pub content_warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NavalBattlesManifestReply {
    pub windows: Vec<BattleWindow>,
    pub available: bool,
    pub unavailable: Option<String>,
    pub content_warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
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

fn step_at(steps: &[Step], hour: i32) -> Option<&[Cell]> {
    let at = steps.partition_point(|step| step.hour <= hour).checked_sub(1)?;
    steps[at].cells.as_deref()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BattleCells {
    pub first_hour: i32,
    pub peak: Option<i64>,
    pub peak_hour: Option<i32>,
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SideAt {
    pub ships: i64,
    pub convoys: i64,
    pub counts_unknown: bool,
    pub countries: BTreeSet<String>,
    pub unknown: bool,
}

pub fn sides_at<'a>(participants: impl Iterator<Item = (u128, Option<&'a [Cell]>)>, tag: &dyn Fn(u32) -> Option<String>) -> [SideAt; 2] {
    let mut sides: [SideAt; 2] = Default::default();
    for (key, cells) in participants {
        let Some(cells) = cells else { continue };
        let side = &mut sides[((key >> 32) & 1) as usize];
        match cells.first().copied().and_then(number) {
            Some(0) => side.convoys += 1,
            Some(_) => side.ships += 1,
            None => side.counts_unknown = true,
        }
        match cells.get(1).copied().and_then(number).and_then(|owner| u32::try_from(owner).ok()).and_then(tag) {
            Some(tag) => {
                side.countries.insert(tag);
            }
            None => side.unknown = true,
        }
    }
    sides
}

pub fn sides_at_truncated<'a>(participants: impl Iterator<Item = (u128, Option<&'a [Cell]>)>, tag: &dyn Fn(u32) -> Option<String>) -> [SideAt; 2] {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ParticipantMode {
    Complete,
    Truncated,
    Missing,
}

impl ParticipantMode {
    fn of(status: replay_columnar::manifest::Status) -> Self {
        match status {
            replay_columnar::manifest::Status::Complete => Self::Complete,
            replay_columnar::manifest::Status::Truncated { .. } => Self::Truncated,
            replay_columnar::manifest::Status::Unavailable { .. } | replay_columnar::manifest::Status::NotRecorded => Self::Missing,
        }
    }
}

pub fn live_run(id: &str, province_id: Option<u32>, named_at: Option<i32>, cells: &BattleCells, sides: Option<&[SideAt; 2]>, first: i32, last: i32) -> NavalBattleRun {
    let reason = "Participants not recorded";
    NavalBattleRun {
        first_hour: first,
        last_hour: last,
        province_id,
        row: BattleRow {
            id: id.to_owned(),
            place: BattlePlace { name: None, hour: cells.first_hour, named_at: province_id.and(named_at).filter(|&hour| hour != cells.first_hour) },
            start: Some(BattleMoment { hour: cells.first_hour, bound: BattleBound::By }),
            end: None,
            winner: BattleWinner::Unknown { unknown: "Result not recorded".into() },
            sides: (0..2)
                .map(|which| {
                    let side = sides.map(|sides| &sides[which]);
                    let bound = |value: fn(&SideAt) -> i64| match side {
                        Some(side) if !side.counts_unknown => (Some(value(side)), false),
                        Some(side) if value(side) > 0 => (Some(value(side)), true),
                        _ => (None, false),
                    };
                    let (ships, ships_at_least) = bound(|side| side.ships);
                    let (convoys, convoys_at_least) = bound(|side| side.convoys);
                    BattleSide {
                        label: if which == 0 { "Attackers" } else { "Defenders" }.into(),
                        countries: side.into_iter().flat_map(|side| &side.countries).map(|tag| BattleCountry { tag: tag.clone(), hour: last }).collect(),
                        countries_unknown: side.is_none_or(|side| side.unknown).then(|| "Countries not recorded".into()),
                        counts: vec![count("Ships", ships, ships_at_least, reason), count("Convoys", convoys, convoys_at_least, reason)],
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

pub fn battle_id(combat: u64, first_hour: i32) -> String {
    format!("combat:{combat}:{first_hour}")
}

pub fn parse_id(id: &str) -> Option<(u64, i32)> {
    let mut parts = id.strip_prefix("combat:")?.split(':');
    let combat = parts.next()?.parse().ok()?;
    let first = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((combat, first))
}

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

fn live_battles(source: &Source, query: &Query<'_>, run: u32, first: i32, last: i32, cancelled: &AtomicBool) -> io::Result<Vec<NavalBattleHistory>> {
    let recording = query.replay.recording();
    let lives = dashboard_base::queries::lifetimes(source, run, TableId::NavalBattles)?;
    let keys: Vec<u128> = lives.iter().filter(|&&(_, born, gone)| born <= i64::from(last) && gone >= i64::from(first)).map(|&(key, ..)| key).collect::<BTreeSet<_>>().into_iter().collect();
    if keys.is_empty() {
        return Ok(Vec::new());
    }
    let battles = lanes::history(recording, run, TableId::NavalBattles, &BATTLE_COLUMNS, Keys::These(&keys), first, last)?;
    let provinces = lanes::history(recording, run, TableId::NavalCombats, &[0], Keys::These(&keys), first, last)?;
    dashboard_text::metadata::check_cancel(cancelled)?;
    let participants = lanes::history(recording, run, TableId::NavalCombatParticipants, &PARTICIPANT_COLUMNS, Keys::All, first, last)?;
    let participant_statuses = lanes::statuses(recording, run, TableId::NavalCombatParticipants, first, last)?;
    let mut participant_modes: Vec<(i32, i32, ParticipantMode)> = Vec::new();
    for held in participant_statuses {
        let mode = ParticipantMode::of(held.status);
        match participant_modes.last_mut() {
            Some(previous) if previous.2 == mode && previous.1.checked_add(1) == Some(held.first) => previous.1 = held.last,
            _ => participant_modes.push((held.first, held.last, mode)),
        }
    }
    let mut by_combat: HashMap<u64, Vec<&(u128, Vec<Step>)>> = HashMap::new();
    for held in &participants.keys {
        by_combat.entry((held.0 >> 33) as u64).or_default().push(held);
    }
    let tags = Tags::new(query, run)?;
    let province_of: HashMap<u128, &Vec<Step>> = provinces.keys.iter().map(|(key, steps)| (*key, steps)).collect();
    let mut out: BTreeMap<(u64, i32), NavalBattleHistory> = BTreeMap::new();
    let mut province_seen: HashMap<(u64, i32), (Option<u32>, Option<i32>)> = HashMap::new();
    for (key, steps) in &battles.keys {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let combat = *key as u64;
        let members = by_combat.get(&combat).map(Vec::as_slice).unwrap_or_default();
        let province_steps = province_of.get(key).map(|steps| steps.as_slice()).unwrap_or_default();
        let mut main_edges: Vec<i32> = steps
            .iter()
            .map(|step| step.hour)
            .chain(members.iter().flat_map(|(_, steps)| steps.iter().map(|step| step.hour)))
            .filter(|hour| (first..=last).contains(hour))
            .collect();
        main_edges.push(first);
        main_edges.sort_unstable();
        main_edges.dedup();
        let mut edges: Vec<i32> = main_edges
            .iter()
            .copied()
            .chain(province_steps.iter().map(|step| step.hour))
            .chain(participant_modes.iter().flat_map(|&(span_first, span_last, _)| [span_first, span_last.saturating_add(1)]))
            .filter(|hour| (first..=last).contains(hour))
            .collect();
        edges.sort_unstable();
        edges.dedup();
        let mut mode_at = 0usize;
        let mut main_edge_at = 0usize;
        for (index, &start) in edges.iter().enumerate() {
            let end = edges.get(index + 1).map_or(last, |next| next - 1);
            let Some(cells) = step_at(steps, start).and_then(BattleCells::of) else { continue };
            while mode_at < participant_modes.len() && participant_modes[mode_at].1 < start {
                mode_at += 1;
            }
            let participant_mode = participant_modes
                .get(mode_at)
                .filter(|&&(span_first, span_last, _)| span_first <= start && start <= span_last)
                .map(|&(_, _, mode)| mode);
            while main_edge_at + 1 < main_edges.len() && main_edges[main_edge_at + 1] <= start {
                main_edge_at += 1;
            }
            let main_hour = main_edges[main_edge_at];
            let sides = match (cells.peak.is_some(), participant_mode) {
                (_, Some(ParticipantMode::Complete)) => {
                    Some(sides_at(members.iter().map(|(key, steps)| (*key, step_at(steps, start))), &|raw| tags.at(raw, start)))
                }
                (true, Some(ParticipantMode::Truncated)) => {
                    Some(sides_at_truncated(members.iter().map(|(key, steps)| (*key, step_at(steps, start))), &|raw| tags.at(raw, main_hour)))
                }
                _ => None,
            };
            let province_id = step_at(province_steps, start).and_then(|cells| cells.first().copied().and_then(number)).and_then(|province| u32::try_from(province).ok());
            let named_at = match province_seen.entry((combat, cells.first_hour)) {
                std::collections::hash_map::Entry::Vacant(slot) => {
                    let observed = province_id.and_then(|_| (start != first || start == cells.first_hour).then_some(start));
                    slot.insert((province_id, observed));
                    observed
                }
                std::collections::hash_map::Entry::Occupied(mut slot) => {
                    let (previous_province, previous_named) = *slot.get();
                    let observed = if province_id != previous_province { province_id.map(|_| start) } else { previous_named };
                    slot.insert((province_id, observed));
                    observed
                }
            };
            let id = battle_id(combat, cells.first_hour);
            let history = out.entry((combat, cells.first_hour)).or_insert_with(|| NavalBattleHistory {
                id: id.clone(),
                province_id,
                runs: Vec::new(),
            });
            if province_id.is_some() {
                history.province_id = province_id;
            }
            let next = live_run(&id, province_id, named_at, &cells, sides.as_ref(), start, end);
            match history.runs.last_mut() {
                Some(previous) if previous.last_hour + 1 == start && same(previous, &next) => {
                    let first_hour = previous.first_hour;
                    *previous = live_run(&id, province_id, named_at, &cells, sides.as_ref(), first_hour, end);
                }
                _ => history.runs.push(next),
            }
        }
    }
    Ok(out.into_values().collect())
}

fn same(a: &NavalBattleRun, b: &NavalBattleRun) -> bool {
    let shown = |run: &NavalBattleRun| {
        serde_json::to_string(&(
            run.province_id,
            run.row.place.named_at,
            &run.row.sides.iter().map(|side| (&side.counts, &side.losses, side.countries.iter().map(|c| &c.tag).collect::<Vec<_>>(), &side.countries_unknown)).collect::<Vec<_>>(),
            run.peak_hour,
            &run.measures[..2],
        ))
        .ok()
    };
    shown(a) == shown(b)
}

fn results(source: &Source, query: &Query<'_>, run: u32, first: i32, last: i32) -> io::Result<Vec<NavalBattleHistory>> {
    let recording = query.replay.recording();
    let lives = dashboard_base::queries::lifetimes(source, run, TableId::NavalResults)?;
    let ids: Vec<u128> = lives.iter().filter(|&&(_, born, gone)| born <= i64::from(last) && gone >= i64::from(first)).map(|&(key, ..)| key).collect::<BTreeSet<_>>().into_iter().collect();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let frame = replay_lanes::at::frame(recording, run, last, &[
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
        province_id: Some(row.province),
        runs: vec![NavalBattleRun {
            first_hour: row.result_hour,
            last_hour: last,
            province_id: Some(row.province),
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

fn observed(recording: &Recording, run: u32, first: i32, last: i32) -> io::Result<Vec<(i32, i32)>> {
    lanes::reads(recording, run, TableId::NavalCombats, first, last)
}

fn windows(query: &Query<'_>) -> io::Result<Vec<BattleWindow>> {
    Ok(dashboard_world::products::army_windows::windows(query)?.into_iter().map(|span| BattleWindow { first_hour: span.first_hour, last_hour: span.last_hour }).collect())
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
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId];
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
        let query = dashboard_base::queries::query(source, branch(&params.branch_id)?)?;
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
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, window];
    type Reply = NavalBattlesReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![NavalBattlesParams::decl(), NavalBattlesReply::decl(), NavalBattleHistory::decl(), NavalBattleRun::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        valid_branch(&params.branch_id)
    }

    fn load(source: &Source, params: &Self::Params, _context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let query = dashboard_base::queries::query(source, branch(&params.branch_id)?)?;
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
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, firstHour: Start, lastHour: End, topN, measure, "countries.membership[].firstHour": Start, "countries.membership[].lastHour": End, "countries.unobserved[].firstHour": Start, "countries.unobserved[].lastHour": End, "countries.countries[]"];
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
        let query = dashboard_base::queries::query(source, branch(&params.branch_id)?)?;
        let recording = query.replay.recording();
        let (from, to) = (params.first_hour, params.last_hour);
        let mut ranked: Vec<Ranked> = Vec::new();
        for stretch in query.timeline.within(from, to) {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let lives = dashboard_base::queries::lifetimes(source, stretch.run, TableId::NavalBattles)?;
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
            dashboard_text::metadata::check_cancel(cancelled)?;
            let low = u128::from(battle.combat) << 33;
            let snapshots = read::at(recording, battle.run, &[read::Read { table: TableId::NavalCombatParticipants, columns: Some(&PARTICIPANT_COLUMNS[..]), keys: Keys::Range(low, low | ((1u128 << 33) - 1)) }], battle.hour)?;
            let snapshot = snapshots.first().ok_or_else(|| invalid("Naval combat participants were not read"))?;
            let view = snapshot.view();
            let named = match tags.entry(battle.run) {
                std::collections::hash_map::Entry::Occupied(held) => held.into_mut(),
                std::collections::hash_map::Entry::Vacant(slot) => slot.insert(Tags::new(&query, battle.run)?),
            };
            let cells: Vec<(u128, Vec<Cell>)> = snapshot.rows.iter().map(|&(key, slot)| (key, PARTICIPANT_COLUMNS.iter().map(|&column| view.number(column, slot).map_or(Cell::Unrecorded, Cell::Number)).collect())).collect();
            let sides = match (battle.cells.peak.is_some(), snapshot.status) {
                (_, replay_columnar::manifest::Status::Complete) => {
                    Some(sides_at(cells.iter().map(|(key, cells)| (*key, Some(cells.as_slice()))), &|raw| named.at(raw, battle.hour)))
                }
                (true, replay_columnar::manifest::Status::Truncated { .. }) => {
                    Some(sides_at_truncated(cells.iter().map(|(key, cells)| (*key, Some(cells.as_slice()))), &|raw| named.at(raw, battle.hour)))
                }
                _ => None,
            };
            let id = battle_id(battle.combat, battle.cells.first_hour);
            let mut run = live_run(&id, None, None, &battle.cells, sides.as_ref(), battle.hour, battle.hour);
            if !params.countries.matches(&query.holders, &run.row.sides, battle.hour) {
                continue;
            }
            let province = read::at(recording, battle.run, &[read::Read { table: TableId::NavalCombats, columns: Some(&[0][..]), keys: Keys::These(&[u128::from(battle.combat)][..]) }], battle.hour)?;
            let province_id = province.first().and_then(|snapshot| snapshot.rows.first().and_then(|&(_, slot)| snapshot.view().number(0, slot))).and_then(|province| u32::try_from(province).ok());
            run.province_id = province_id;
            rows.push(NavalBattleHistory { id, province_id, runs: vec![run] });
        }
        let census_hour = dashboard_land::products::land_battles::last_read(&query, TableId::NavalCombats, to)?;
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
    #[doc = " The hour its fleets are read at: the hour asked."]
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
                        .and_then(|text| content_reader_localisation::localisation::display_text(text.as_bytes()))
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
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, "battles[].hour": Read];
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
        let query = dashboard_base::queries::query(source, branch(&params.branch_id)?)?;
        let recording = query.replay.recording();
        let content = sunk::Content::load(source, context);
        let mut refused = Refusals::default();
        let mut reply = NavalBattleDetailsReply { rows: Vec::with_capacity(params.battles.len()), warnings: content.warnings() };
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
        let mut seeker = Seeker::new(recording)?;
        for (&(run, first, hour), battles) in &at {
            dashboard_text::metadata::check_cancel(cancelled)?;
            if stretch != Some((run, first)) {
                stretch = Some((run, first));
                reads = lanes::reads(recording, run, TableId::SunkShips, first, last[&(run, first)])?;
                lists = HashMap::new();
            }
            let combats: Vec<u64> = battles.iter().map(|&(_, combat)| combat).collect();
            for (&(index, _), fleets) in battles.iter().zip(fleets_at(&mut seeker, run, hour, &combats, &content, &mut refused)?) {
                reply.rows[index].fleets = fleets;
            }
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
