use super::battle_id;
use dashboard_base::{
    Source,
    products::{
        LoadContext, Product,
        occurrences::{self, OccurrenceReply, OccurrenceSort},
    },
    queries::{self, country_ids::RunLinks},
};
use dashboard_follow::panel::Panel;
use dashboard_query::{
    Query,
    identity::{Country, Holders},
    log::occurrence::{Ask, Index, Occurrence, Order, PlacedGap, Role, Selection, SortKey},
    series::{MissingSpan, ranked::GroupSelection},
    timeline::Stretch,
};
use dashboard_text::metadata::check_cancel;
use replay_columnar::manifest::Status;
use replay_data::rows::{NavalResultObservationRow, TableId, VariantStatsRow};
use replay_lanes::lanes::{self, Cell, History, Keys, Reading, Step};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::{Arc, PoisonError, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NavalBattlesTailParams {
    pub branch_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub hour: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub from: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub to: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub countries: Option<Vec<Country>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub group: Option<GroupSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub offset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub sort: Option<OccurrenceSort>,
    pub limit: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum NavalBattleStatus {
    Ongoing,
    AttackerWon,
    DefenderWon,
    Unknown,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NavalBattleLogRow {
    pub key: String,
    pub hour: i32,
    pub detail_id: String,
    pub detail_hour: Option<i32>,
    pub result_hour: Option<i32>,
    pub last_live_hour: Option<i32>,
    pub end_seen: bool,
    pub province: Option<u32>,
    pub countries: [Vec<u32>; 2],
    pub countries_hour: [i32; 2],
    pub read_hour: i32,
    pub ships_sunk: [Option<u32>; 2],
    pub convoys_sunk: [Option<u32>; 2],
    pub ic_lost: [Option<f64>; 2],
    pub ships: [Option<u32>; 2],
    pub convoys: [Option<u32>; 2],
    pub result: NavalBattleStatus,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NavalBattlesTailReply {
    pub branch_id: String,
    pub total: String,
    pub offset: String,
    pub rows: Vec<NavalBattleLogRow>,
    pub missing: Vec<PlacedGap>,
    pub next_change: Option<i32>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(NavalBattlesTailReply);

impl OccurrenceReply for NavalBattlesTailReply {
    fn next_change(&self) -> Option<i32> {
        self.next_change
    }
    fn row_count(&self) -> usize {
        self.rows.len()
    }
}

const JOINED: u8 = 4;
const SUNK: i64 = 0x20;
const SHOWN: [u8; 4] = [
    NavalBattleStatus::Ongoing as u8,
    NavalBattleStatus::Unknown as u8,
    NavalBattleStatus::AttackerWon as u8,
    NavalBattleStatus::DefenderWon as u8,
];
const SORT_COLUMNS: &[&str] = &["shipsSunk", "icLost", "ships"];
type Steps<T> = Vec<(i32, T)>;

fn push<T: PartialEq>(steps: &mut Steps<T>, hour: i32, value: T) {
    if steps.last().is_some_and(|(at, _)| *at == hour) {
        steps.pop();
    }
    if steps.last().is_none_or(|(_, held)| *held != value) {
        steps.push((hour, value));
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Snapshot {
    province: Option<u32>,
    countries: [Vec<u32>; 2],
    countries_hour: [Option<i32>; 2],
    ships: [Option<u32>; 2],
    convoys: [Option<u32>; 2],
    sunk: [Option<u32>; 2],
    convoys_sunk: [Option<u32>; 2],
    ic: [Option<i64>; 2],
    result: NavalBattleStatus,
    result_id: Option<u64>,
    port_strike: bool,
    detail_hour: Option<i32>,
    result_hour: Option<i32>,
    live_last: Option<i32>,
    end_seen: bool,
}

#[derive(Default)]
struct Line {
    key: String,
    battle: Option<Battle>,
    reads: Steps<Snapshot>,
    joined: Option<i32>,
}

impl Line {
    fn end_live(&mut self, hour: i32, seen: bool) {
        if let Some((_, previous)) = self.reads.last()
            && previous.result == NavalBattleStatus::Ongoing
        {
            let mut ended = previous.clone();
            ended.result = NavalBattleStatus::Unknown;
            ended.detail_hour = Some(hour - 1);
            ended.live_last = Some(hour - 1);
            ended.end_seen = seen;
            push(&mut self.reads, hour, ended);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Battle {
    Live(u64, i32),
    Result(u64, i32),
}

impl Battle {
    fn date(self) -> i32 {
        match self {
            Self::Live(_, date) | Self::Result(_, date) => date,
        }
    }

    fn key(self) -> String {
        match self {
            Self::Live(combat, first) => battle_id(combat, first),
            Self::Result(id, _) => format!("result:{id}"),
        }
    }
}

fn number(cells: &[Cell], column: usize) -> Option<i64> {
    match cells.get(column) {
        Some(Cell::Number(value)) => Some(*value),
        _ => None,
    }
}

fn count(cells: &[Cell], column: usize) -> Option<u32> {
    number(cells, column)?.try_into().ok()
}

fn at(steps: &[Step], hour: i32) -> Option<&[Cell]> {
    let i = steps.partition_point(|step| step.hour <= hour);
    steps.get(i.checked_sub(1)?)?.cells.as_deref()
}

fn steps(history: &History, key: u128) -> &[Step] {
    history.keys.binary_search_by_key(&key, |(k, _)| *k).ok().map_or(&[], |i| &history.keys[i].1)
}

fn members(history: &History, id: u128, shift: u32) -> &[(u128, Vec<Step>)] {
    let first = history.keys.partition_point(|(key, _)| key >> shift < id);
    let rest = &history.keys[first..];
    &rest[..rest.partition_point(|(key, _)| key >> shift == id)]
}

fn edges(steps: &[Step], first: i32, last: i32, hours: &mut Vec<i32>) {
    let from = steps.partition_point(|s| s.hour <= first);
    hours.extend(steps[from..].iter().take_while(|s| s.hour <= last).map(|s| s.hour));
}

fn complete(history: &History, hour: i32) -> bool {
    let at = history.statuses.partition_point(|span| span.last < hour);
    history.statuses.get(at).is_some_and(|s| s.first <= hour && s.status == Status::Complete)
}

fn country(countries: &mut Vec<u32>, slot: Option<u32>) {
    if let Some(slot) = slot
        && !countries.contains(&slot)
    {
        countries.push(slot);
    }
}

fn sunk_ship(cells: &[Cell]) -> bool {
    number(cells, 0).is_some_and(|ship| ship != 0) && number(cells, 2).is_some_and(|state| state & SUNK != 0)
}

fn sunk_ships(participants: &History) -> Vec<u128> {
    participants.keys.iter().filter(|(_, reads)| reads.iter().any(|read| read.cells.as_deref().is_some_and(sunk_ship))).map(|(key, _)| *key).collect()
}

#[derive(Default)]
struct Prices {
    designs: History,
    costs: History,
}

impl Prices {
    fn of(&self, member: u128, hour: i32) -> Option<i64> {
        let design = at(steps(&self.designs, member), hour).and_then(|cells| number(cells, 0)).filter(|design| *design != 0)?;
        match self.costs.reading(design as u64 as u128, hour) {
            Reading::Cells([Cell::Number(cost)]) => Some(*cost),
            _ => None,
        }
    }
}

fn ic_lost(lost: &BTreeMap<u128, Option<i64>>, sunk: [Option<u32>; 2]) -> [Option<i64>; 2] {
    std::array::from_fn(|side| {
        let (count, sum) = lost
            .iter()
            .filter(|(key, _)| ((*key >> 32) & 1) as usize == side)
            .fold((0_u32, Some(0_i128)), |(count, sum), (_, cost)| (count + 1, sum.zip(*cost).map(|(sum, cost)| sum + i128::from(cost))));
        (sunk[side] == Some(count)).then_some(sum).flatten().and_then(|sum| i64::try_from(sum).ok())
    })
}

#[expect(clippy::too_many_arguments, reason = "each history is one recorded table the battle line joins")]
fn live_lines(lines: &mut BTreeMap<Battle, Line>, span: Stretch, last_hour: i32, links: &RunLinks, battles: &History, provinces: &History, participants: &History, prices: &Prices) {
    for (combat, battle) in &battles.keys {
        let roster = members(participants, *combat, 33);
        let places = steps(provinces, *combat);
        let mut lost_by: HashMap<i32, BTreeMap<u128, Option<i64>>> = HashMap::new();
        for (i, step) in battle.iter().enumerate() {
            let Some(cells) = &step.cells else { continue };
            let Some(first_hour) = number(cells, 0).and_then(|v| i32::try_from(v).ok()) else {
                continue;
            };
            let first = step.hour.max(span.first).max(first_hour);
            let next_battle = battle.get(i + 1);
            let last = next_battle.map_or(span.last, |s| s.hour - 1).min(span.last);
            if first > last {
                continue;
            }
            let line = lines.entry(Battle::Live(*combat as u64, first_hour)).or_default();
            if line.reads.last().is_some_and(|(_, read)| read.result_id.is_some()) {
                continue;
            }
            let mut hours = vec![first];
            edges(places, first, last, &mut hours);
            for (_, reads) in roster {
                edges(reads, first, last, &mut hours);
                let start = reads.partition_point(|s| s.hour <= first).saturating_sub(1);
                for (j, read) in reads.iter().enumerate().skip(start).take_while(|(_, s)| s.hour <= last) {
                    if let Some(owner) = read.cells.as_deref().and_then(|c| count(c, 1)) {
                        let end = reads.get(j + 1).map_or(last, |s| last.min(s.hour - 1));
                        hours.extend(links.within(owner, read.hour.max(first), end).skip(1).map(|(_, h, _)| h));
                    }
                }
            }
            let i = participants.statuses.partition_point(|s| s.last < first);
            hours.extend(participants.statuses[i..].iter().take_while(|s| s.first <= last).map(|s| s.first.max(first)));
            hours.sort_unstable();
            hours.dedup();
            let lost = lost_by.entry(first_hour).or_default();
            for hour in hours {
                let mut ships = [complete(participants, hour).then_some(0); 2];
                let mut convoys = [complete(participants, hour).then_some(0); 2];
                let mut countries = [Vec::new(), Vec::new()];
                for (key, reads) in roster {
                    let Some(cells) = at(reads, hour) else {
                        continue;
                    };
                    let side = ((key >> 32) & 1) as usize;
                    match number(cells, 0) {
                        Some(0) => {
                            if let Some(n) = &mut convoys[side] {
                                *n += 1;
                            }
                        }
                        Some(_) => {
                            if let Some(n) = &mut ships[side] {
                                *n += 1;
                            }
                        }
                        None => {
                            ships[side] = None;
                            convoys[side] = None;
                        }
                    }
                    country(&mut countries[side], count(cells, 1).and_then(|raw| links.slot_at(raw, hour)));
                    if sunk_ship(cells) {
                        lost.entry(*key).or_insert_with(|| prices.of(*key, hour));
                    }
                }
                let sunk = [count(cells, 1), count(cells, 2)];
                let convoys_sunk = [count(cells, 3), count(cells, 4)];
                push(
                    &mut line.reads,
                    hour,
                    Snapshot {
                        province: at(places, hour).and_then(|v| count(v, 0)),
                        ships,
                        convoys,
                        countries,
                        countries_hour: [None; 2],
                        sunk,
                        convoys_sunk,
                        ic: ic_lost(lost, sunk),
                        result: NavalBattleStatus::Ongoing,
                        result_id: None,
                        port_strike: false,
                        detail_hour: None,
                        result_hour: None,
                        live_last: None,
                        end_seen: false,
                    },
                );
            }
            if last < last_hour && !next_battle.and_then(|s| s.cells.as_deref()).is_some_and(|c| number(c, 0) == Some(i64::from(first_hour))) {
                line.end_live(last + 1, last < span.last && complete(battles, last + 1));
            }
        }
    }
}

fn result_lines(lines: &mut BTreeMap<Battle, Line>, span: Stretch, links: &RunLinks, results: &History, observations: &History, countries: &History) {
    for (key, result) in &results.keys {
        let observation = steps(observations, *key);
        let lists = members(countries, *key, 40);
        let mut linked = None;
        for (i, step) in result.iter().enumerate() {
            let Some(cells) = &step.cells else { continue };
            let Some(date) = number(cells, 1).and_then(|h| i32::try_from(h).ok()) else {
                continue;
            };
            let first = step.hour.max(date);
            let named = date.max(span.first);
            let last = result.get(i + 1).map_or(span.last, |s| s.hour - 1);
            if first > last {
                continue;
            }
            let mut hours = vec![first];
            edges(observation, first, last, &mut hours);
            for (_, reads) in lists {
                edges(reads, first, last, &mut hours);
            }
            hours.sort_unstable();
            hours.dedup();
            for hour in hours {
                let obs = at(observation, hour)
                    .filter(|v| number(v, 0) == Some(i64::from(NavalResultObservationRow::OBSERVED)))
                    .unwrap_or(&[]);
                let result = Battle::Result(*key as u64, date);
                if let (Some(combat), Some(start)) = (number(obs, 1), number(obs, 2).and_then(|h| i32::try_from(h).ok()))
                    && combat != 0
                    && start <= date
                {
                    linked = Some(Battle::Live(combat as u64, start));
                    if let Some(earlier) = lines.get_mut(&result) {
                        earlier.joined.get_or_insert(hour);
                    }
                }
                let battle = linked.unwrap_or(result);
                let named_at = |when: i32| {
                    let mut sides = [Vec::new(), Vec::new()];
                    for (key, reads) in lists {
                        let side = ((key >> 32) & 255) as usize;
                        if let Some(list) = sides.get_mut(side)
                            && at(reads, hour).is_some()
                        {
                            country(list, links.slot_at(*key as u32, when));
                        }
                    }
                    sides
                };
                let mut sides = named_at(date);
                let result_hour = [&battle, &result]
                    .into_iter()
                    .filter_map(|of| lines.get(of))
                    .flat_map(|line| &line.reads)
                    .filter(|(at, read)| *at <= hour && read.result_id == Some(*key as u64))
                    .filter_map(|(_, read)| read.result_hour)
                    .min()
                    .unwrap_or(hour);
                let line = lines.entry(battle).or_default();
                line.reads.truncate(line.reads.partition_point(|(at, _)| *at <= hour));
                let mut late = None;
                let countries_hour = std::array::from_fn(|i| {
                    if !sides[i].is_empty() {
                        return Some(date);
                    }
                    if let Some((at, prior)) = line.reads.iter().rev().find(|(_, read)| !read.countries[i].is_empty()) {
                        sides[i].clone_from(&prior.countries[i]);
                        return prior.countries_hour[i].or(Some(*at));
                    }
                    if named == date {
                        return None;
                    }
                    let found: &[Vec<u32>; 2] = late.get_or_insert_with(|| named_at(named));
                    sides[i].clone_from(&found[i]);
                    (!sides[i].is_empty()).then_some(named)
                });
                let sunk = [count(obs, 6), count(obs, 10)];
                let convoys_sunk = [count(obs, 7), count(obs, 11)];
                let observed = number(obs, 3).and_then(|h| i32::try_from(h).ok()).and_then(|end| line.reads.iter().rev().find(|(at, _)| *at <= end));
                let (live_last, end_seen) = line
                    .reads
                    .iter()
                    .rev()
                    .find_map(|(_, read)| match read.live_last {
                        Some(last) => Some((Some(last), read.end_seen)),
                        None => (read.result == NavalBattleStatus::Ongoing).then_some((Some(hour), false)),
                    })
                    .unwrap_or((None, false));
                let ic = std::array::from_fn(|i| observed.filter(|(_, prior)| prior.sunk[i] == sunk[i]).and_then(|(_, prior)| prior.ic[i]));
                let status = match number(cells, 3) {
                    Some(1) => NavalBattleStatus::AttackerWon,
                    Some(2) => NavalBattleStatus::DefenderWon,
                    _ => NavalBattleStatus::Unknown,
                };
                push(
                    &mut line.reads,
                    hour,
                    Snapshot {
                        province: count(cells, 0),
                        countries: sides,
                        countries_hour,
                        ships: [count(obs, 4), count(obs, 8)],
                        convoys: [count(obs, 5), count(obs, 9)],
                        sunk,
                        convoys_sunk,
                        ic,
                        result: status,
                        result_id: Some(*key as u64),
                        port_strike: number(cells, 2).is_some_and(|flag| flag != 0),
                        detail_hour: Some(hour),
                        result_hour: Some(result_hour),
                        live_last,
                        end_seen,
                    },
                );
            }
        }
    }
}

struct Scanned {
    index: Index<Line>,
    live_gaps: Vec<MissingSpan>,
}

fn scan(source: &Source, query: &Query<'_>, cancelled: &AtomicBool) -> io::Result<Scanned> {
    let recording = query.replay.recording();
    let mut lines = BTreeMap::new();
    let mut gaps: Vec<_> = query
        .timeline
        .unheld
        .iter()
        .map(|g| MissingSpan {
            first_hour: g.first,
            last_hour: g.last,
            reason: g.reason.clone(),
        })
        .collect();
    let mut live_gaps = gaps.clone();
    for &span in &query.timeline.stretches {
        check_cancel(cancelled)?;
        let absent = |table: TableId| recording.withheld_reason(table.index()).or_else(|| recording.predates(table.index()));
        let history = |table, columns: &[usize]| {
            if absent(table).is_some() {
                Ok(History::default())
            } else {
                lanes::history(recording, span.run, table, columns, Keys::All, span.first, span.last)
            }
        };
        for (table, reason) in [(TableId::NavalBattles, "Naval battle census incomplete"), (TableId::NavalResults, "Naval results incomplete")] {
            let read = if absent(table).is_some() {
                Vec::new()
            } else {
                lanes::complete_reads(recording, span.run, table, span.first, span.last)?
            };
            let unread = dashboard_land::products::division_log::unread(&read, span.first, span.last).into_iter().map(|(first_hour, last_hour)| MissingSpan {
                first_hour,
                last_hour,
                reason: absent(table).unwrap_or(reason).to_owned(),
            });
            let start = gaps.len();
            gaps.extend(unread);
            if table == TableId::NavalBattles {
                live_gaps.extend_from_slice(&gaps[start..]);
            }
        }
        let links = queries::links(source, span.run)?;
        let participants = history(TableId::NavalCombatParticipants, &[0, 1, dashboard_declare::field(TableId::NavalCombatParticipants, "state")?])?;
        let sunk = sunk_ships(&participants);
        let mut prices = Prices::default();
        if !sunk.is_empty() && absent(TableId::NavalCombatShips).is_none() && absent(TableId::VariantStats).is_none() {
            let design = dashboard_declare::field(TableId::NavalCombatShips, "variant")?;
            prices.designs = lanes::history(recording, span.run, TableId::NavalCombatShips, &[design], Keys::These(&sunk), span.first, span.last)?;
            let designs: Vec<u128> = prices
                .designs
                .keys
                .iter()
                .flat_map(|(_, reads)| reads.iter().filter_map(|read| read.cells.as_deref().and_then(|cells| number(cells, 0))))
                .filter(|design| *design != 0)
                .map(|design| design as u64 as u128)
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            prices.costs = lanes::history(recording, span.run, TableId::VariantStats, &[VariantStatsRow::BUILD_COST_IC], Keys::These(&designs), span.first, span.last)?;
        }
        live_lines(
            &mut lines,
            span,
            query.timeline.last,
            &links,
            &history(TableId::NavalBattles, &[0, 3, 4, 5, 6])?,
            &history(TableId::NavalCombats, &[0])?,
            &participants,
            &prices,
        );
        result_lines(
            &mut lines,
            span,
            &links,
            &history(TableId::NavalResults, &[0, 1, 4, 8])?,
            &history(TableId::NavalResultObservations, &[0, 1, 3, 4, 7, 8, 9, 10, 11, 12, 13, 14])?,
            &history(TableId::NavalResultCountries, &[0])?,
        );
    }
    let mut entries = Vec::with_capacity(lines.len());
    for (battle, mut line) in lines {
        let Some(&(observed, _)) = line.reads.first() else {
            continue;
        };
        let mut states = Vec::new();
        let mut sunk = Vec::new();
        let mut ic = Vec::new();
        let mut ships = Vec::new();
        let mut roles = Vec::new();
        let total = |[a, b]: [Option<u32>; 2]| Some(i64::from(a?) + i64::from(b?));
        let vessels = |ships, convoys| total(ships).zip(total(convoys)).map(|(ships, convoys)| ships + convoys);
        for (at, read) in &line.reads {
            push(&mut states, *at, read.result as u8);
            push(&mut sunk, *at, vessels(read.sunk, read.convoys_sunk));
            push(&mut ic, *at, read.ic[0].zip(read.ic[1]).map(|(a, b)| a.saturating_add(b)));
            push(&mut ships, *at, vessels(read.ships, read.convoys));
            roles.extend(read.countries.iter().flatten().map(|slot| Role::at(Some(*slot), *at)));
        }
        if let Some(joined) = line.joined {
            push(&mut states, joined, JOINED);
        }
        line.key = battle.key();
        line.battle = Some(battle);
        entries.push(Occurrence {
            id: entries.len() as u128,
            date: battle.date(),
            observed,
            kind: 0,
            roles: roles.into(),
            subjects: Box::default(),
            states: states.into(),
            sort_keys: [SortKey::dated_numbers(sunk), SortKey::dated_numbers(ic), SortKey::dated_numbers(ships)].into(),
            text: Default::default(),
            payload: line,
        });
    }
    Ok(Scanned { index: Index::new(entries, gaps, &[NavalBattleStatus::Ongoing as u8])?, live_gaps })
}

#[derive(Default)]
struct Branches(HashMap<u64, Arc<Scanned>>);

struct Prepared {
    scanned: Arc<Scanned>,
    selection: Option<Selection>,
}

fn scanned(source: &Source, query: &Query<'_>, cancelled: &AtomicBool) -> io::Result<Arc<Scanned>> {
    let state = source.state::<Branches>();
    let mut branches = state.lock().unwrap_or_else(PoisonError::into_inner);
    Ok(match branches.0.entry(query.timeline.id) {
        std::collections::hash_map::Entry::Occupied(e) => Arc::clone(e.get()),
        std::collections::hash_map::Entry::Vacant(e) => Arc::clone(e.insert(Arc::new(scan(source, query, cancelled)?))),
    })
}

pub struct BattlePlace {
    pub battle: Battle,
    pub date: i32,
    pub observed: i32,
    pub province: Option<u32>,
    pub port_strike: bool,
    pub reports: Vec<u64>,
    pub live_hours: Option<(i32, i32)>,
}

pub fn battles(source: &Source, query: &Query<'_>, cancelled: &AtomicBool) -> io::Result<Vec<BattlePlace>> {
    let scanned = scanned(source, query, cancelled)?;
    let mut places = Vec::new();
    let mut reports = Vec::new();
    let mut by_report = HashMap::new();
    for i in 0..scanned.index.len() as u32 {
        let entry = scanned.index.get(i);
        let line = &entry.payload;
        let battle = line.battle.expect("published battle line has an identity");
        let (_, first) = &line.reads[0];
        let (_, last) = line.reads.last().expect("published battle line has reads");
        let mut place = BattlePlace {
            battle, date: entry.date, observed: entry.observed, province: first.province,
            port_strike: line.reads.iter().any(|(_, read)| read.port_strike),
            reports: Vec::new(),
            live_hours: last.live_last.or_else(|| (last.result == NavalBattleStatus::Ongoing).then_some(query.timeline.last))
                .map(|last| (entry.observed, last)),
        };
        if let Battle::Result(id, _) = battle {
            reports.push((id, place));
        } else {
            place.reports = line.reads.iter().filter_map(|(_, read)| read.result_id).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
            by_report.extend(place.reports.iter().map(|&id| (id, places.len())));
            places.push(place);
        }
    }
    for (id, report) in reports {
        let index = *by_report.entry(id).or_insert(places.len());
        if let Some(place) = places.get_mut(index) {
            place.port_strike |= report.port_strike;
            if (report.observed, report.date) < (place.observed, place.date) {
                (place.observed, place.date, place.province) = (report.observed, report.date, report.province);
            }
        } else {
            places.push(report);
        }
    }
    Ok(places)
}

fn prepare(source: &Source, query: &Query<'_>, load: &LoadContext, params: &NavalBattlesTailParams, cancelled: &AtomicBool) -> io::Result<Prepared> {
    let scanned = scanned(source, query, cancelled)?;
    let selection = dashboard_world::products::group_members::dated(source, query, load, params.countries.as_deref(), params.group.as_ref(), cancelled)?;
    Ok(Prepared { scanned, selection })
}

fn reply(prepared: &Prepared, holders: &Holders, params: &NavalBattlesTailParams, hour: i32) -> io::Result<NavalBattlesTailReply> {
    let (offset, limit) = occurrences::window(params.offset.as_deref(), params.limit)?;
    let (order, _) = occurrences::order(params.sort.as_ref(), SORT_COLUMNS)?;
    let mut ask = Ask {
        hour,
        from: params.from,
        to: params.to,
        gap_end: None,
        selection: prepared.selection.as_ref(),
        subject: None,
        kinds: None,
        states: Some(&SHOWN),
        search: None,
        order,
        live_first: matches!(order, Order::Date(_)),
        offset,
        limit,
        chips: &[],
    };
    let answer = prepared.scanned.index.ask(holders, &ask)?;
    let mut visible_change: Option<i32> = None;
    let rows = answer
        .rows
        .iter()
        .filter_map(|shown| {
            let (row, change) = log_row(prepared.scanned.index.get(shown.at), hour)?;
            visible_change = visible_change.into_iter().chain(change).min();
            Some(row)
        })
        .collect();
    let mut boundary = answer.next_change;
    while let Some(candidate) = boundary.filter(|at| visible_change.is_none_or(|visible| *at < visible)) {
        ask.hour = candidate;
        let future = prepared.scanned.index.ask(holders, &ask)?;
        if future.total != answer.total || future.rows != answer.rows || future.gaps != answer.gaps || future.unattributed != answer.unattributed {
            break;
        }
        boundary = future.next_change;
    }
    let next_change = boundary.into_iter().chain(visible_change).min();
    let warnings = (answer.unattributed > 0)
        .then(|| format!("Battles without a recorded country for this filter: {}", answer.unattributed))
        .into_iter()
        .collect();
    let reply = NavalBattlesTailReply {
        branch_id: params.branch_id.clone(),
        total: answer.total.to_string(),
        offset: answer.offset.to_string(),
        rows,
        missing: answer.gaps,
        next_change,
        warnings,
    };
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
}

fn log_row(entry: &Occurrence<Line>, hour: i32) -> Option<(NavalBattleLogRow, Option<i32>)> {
    let line = &entry.payload;
    let partition = line.reads.partition_point(|(at, _)| *at <= hour);
    let (at, read) = line.reads.get(partition.checked_sub(1)?)?;
    let row = NavalBattleLogRow {
        key: line.key.clone(),
        hour: entry.date,
        province: read.province,
        result: read.result,
        detail_id: read.result_id.map_or_else(|| line.key.clone(), |id| format!("result:{id}")),
        detail_hour: read.detail_hour,
        result_hour: read.result_hour,
        last_live_hour: read.live_last,
        end_seen: read.end_seen,
        ships_sunk: read.sunk,
        convoys_sunk: read.convoys_sunk,
        ic_lost: read.ic.map(|raw| raw.map(|raw| raw as f64 / 100_000.0)),
        ships: read.ships,
        convoys: read.convoys,
        countries: read.countries.clone(),
        countries_hour: read.countries_hour.map(|h| h.unwrap_or(*at)),
        read_hour: *at,
    };
    Some((row, line.reads.get(partition).map(|step| step.0)))
}

pub struct NavalBattlesTail;

impl Product for NavalBattlesTail {
    const KEY: &'static str = "naval-battles-tail";
    const CONTENT_DEPENDENT: bool = false;
    const INTERACTIVE: bool = true;
    type Params = NavalBattlesTailParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, hour: Read, from: Start, to: End, countries, "countries[]", group, offset, sort, limit];
    type Reply = NavalBattlesTailReply;
    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            NavalBattlesTailParams::decl(),
            NavalBattlesTailReply::decl(),
            NavalBattleLogRow::decl(),
            NavalBattleStatus::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        params.countries.iter().flatten().try_for_each(dashboard_base::products::validate_country)
    }

    fn load(source: &Source, params: &Self::Params, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        check_cancel(cancelled)?;
        let branch = params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let hour = occurrences::cutoff(&query, params.hour)?;
        let prepared = prepare(source, &query, load, params, cancelled)?;
        reply(&prepared, &query.holders, params, hour)
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NavalBattleCourseParams {
    pub branch_id: String,
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub hour: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NavalCourseRead {
    pub hour: i32,
    pub result: NavalBattleStatus,
    pub ships: [Option<u32>; 2],
    pub convoys: [Option<u32>; 2],
    pub ships_sunk: [Option<u32>; 2],
    pub convoys_sunk: [Option<u32>; 2],
    pub ic_lost: [Option<f64>; 2],
    pub resulted: bool,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NavalBattleCourseReply {
    pub branch_id: String,
    pub reads: Vec<NavalCourseRead>,
    pub missing: Vec<MissingSpan>,
    pub row: Option<NavalBattleLogRow>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(NavalBattleCourseReply);

fn entry<'a>(scanned: &'a Scanned, key: &str, hour: i32) -> Option<&'a Occurrence<Line>> {
    let index = &scanned.index;
    let entries = || (0..index.len()).map(|at| index.get(at as u32));
    let joined = super::parse_result_id(key).and_then(|id| {
        entries().find(|entry| entry.payload.key.starts_with("combat:") && entry.payload.reads.iter().any(|(at, read)| *at <= hour && read.result_id == Some(id)))
    });
    joined.or_else(|| entries().find(|entry| entry.payload.key == key))
}

fn course(scanned: &Scanned, entry: &Occurrence<Line>, branch_last: i32) -> (Vec<NavalCourseRead>, Vec<MissingSpan>) {
    let reads: Vec<_> = entry
        .payload
        .reads
        .iter()
        .map(|(hour, read)| NavalCourseRead {
            hour: *hour,
            result: read.result,
            ships: read.ships,
            convoys: read.convoys,
            ships_sunk: read.sunk,
            convoys_sunk: read.convoys_sunk,
            ic_lost: read.ic.map(|raw| raw.map(|raw| raw as f64 / 100_000.0)),
            resulted: read.result_id.is_some(),
        })
        .collect();
    let first = entry.date.min(reads.first().map_or(entry.date, |read| read.hour));
    let last = match (reads.iter().find(|read| read.resulted), reads.last()) {
        (Some(result), _) => result.hour - 1,
        (None, Some(read)) if read.result == NavalBattleStatus::Ongoing => branch_last.max(read.hour),
        (None, Some(read)) => read.hour,
        (None, None) => first,
    };
    let missing = scanned
        .live_gaps
        .iter()
        .filter(|gap| gap.first_hour <= last && gap.last_hour >= first)
        .map(|gap| MissingSpan { first_hour: gap.first_hour.max(first), last_hour: gap.last_hour.min(last), reason: gap.reason.clone() })
        .filter(|gap| gap.first_hour <= gap.last_hour)
        .collect();
    (reads, missing)
}

pub struct NavalBattleCourse;

impl Product for NavalBattleCourse {
    const KEY: &'static str = "naval-battle-course";
    const CONTENT_DEPENDENT: bool = false;
    type Params = NavalBattleCourseParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, key, hour: Read];
    type Reply = NavalBattleCourseReply;
    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![NavalBattleCourseParams::decl(), NavalCourseRead::decl(), NavalBattleCourseReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        let keyed = super::parse_id(&params.key).is_some() || super::parse_result_id(&params.key).is_some();
        keyed.then_some(()).ok_or("A naval battle course names a naval battle row's key")
    }

    fn load(source: &Source, params: &Self::Params, _load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        check_cancel(cancelled)?;
        let branch = params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let scanned = scanned(source, &query, cancelled)?;
        let hour = occurrences::cutoff(&query, params.hour)?;
        let found = entry(&scanned, &params.key, hour).ok_or_else(|| dashboard_core::invalid("No naval battle with this key in the branch"))?;
        let (reads, missing) = course(&scanned, found, query.timeline.last);
        let row = params.hour.and_then(|_| log_row(found, hour)).map(|(row, _)| row);
        let reply = NavalBattleCourseReply { branch_id: params.branch_id.clone(), reads, missing, row, warnings: Vec::new() };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

pub fn dated_panel(source: &Source, load: &LoadContext, asked: &serde_json::Value) -> Result<Box<dyn Panel>, String> {
    let mut request = asked.clone();
    request.as_object_mut().ok_or("Invalid naval-battles-tail panel")?.remove("panel");
    let params: NavalBattlesTailParams = dashboard_query::named::named(&request).map_err(|error| format!("Invalid naval-battles-tail panel: {error}"))?;
    NavalBattlesTail::validate(&params)?;
    let branch = params.branch_id.parse().map_err(|_| "Invalid branch")?;
    let query = dashboard_base::queries::query(source, branch).map_err(|error| error.to_string())?;
    let (finish, prepared) = dashboard_base::products::Finish::open(source, load, NavalBattlesTail::CONTENT_DEPENDENT, || prepare(source, &query, load, &params, &AtomicBool::new(false)));
    let prepared = prepared.map_err(|error| error.to_string())?;
    let (first, last, holders) = (query.timeline.first, query.timeline.last, query.holders);
    Ok(occurrences::occurrence_panel(source.tables(), finish, move |hour, cancelled| {
        check_cancel(cancelled)?;
        if hour < first {
            return Err(dashboard_core::invalid("The hour asked is before the branch"));
        }
        reply(&prepared, &holders, &params, hour.min(last))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_changed_battle_stops_running_at_its_deletion_step() {
        let battle = vec![
            Step { hour: 10, cells: Some(Box::new([Cell::Number(10), Cell::Number(0), Cell::Number(0)])) },
            Step { hour: 15, cells: Some(Box::new([Cell::Number(10), Cell::Number(1), Cell::Number(0)])) },
            Step { hour: 20, cells: None },
        ];
        let read = |last: i32| vec![lanes::StatusRun { first: 10, last, status: Status::Complete }];
        let battles = History { keys: vec![(7, battle)], statuses: read(40), ..History::default() };
        let span = Stretch { run: 1, first: 10, last: 40, join: dashboard_query::timeline::Join::Start };
        let mut lines = BTreeMap::new();
        live_lines(&mut lines, span, span.last, &RunLinks::default(), &battles, &History::default(), &History::default(), &Prices::default());
        let reads = &lines[&Battle::Live(7, 10)].reads;
        assert_eq!(reads.iter().map(|(hour, read)| (*hour, read.result)).collect::<Vec<_>>(), [
            (10, NavalBattleStatus::Ongoing), (15, NavalBattleStatus::Ongoing), (20, NavalBattleStatus::Unknown),
        ]);
        assert_eq!(reads.last().unwrap().1.detail_hour, Some(19));
        assert_eq!((reads.last().unwrap().1.live_last, reads.last().unwrap().1.end_seen), (Some(19), true), "a removal inside the stretch is recorded");
        let unread = History { statuses: read(19), ..battles.clone() };
        let mut lines = BTreeMap::new();
        live_lines(&mut lines, span, span.last, &RunLinks::default(), &unread, &History::default(), &History::default(), &Prices::default());
        let (_, read_gap) = lines[&Battle::Live(7, 10)].reads.last().unwrap();
        assert_eq!((read_gap.live_last, read_gap.end_seen), (Some(19), false), "a battle absent where the census was not read has no recorded removal");
        let stretch = Stretch { run: 1, first: 10, last: 17, join: dashboard_query::timeline::Join::Start };
        let mut lines = BTreeMap::new();
        live_lines(&mut lines, stretch, 40, &RunLinks::default(), &battles, &History::default(), &History::default(), &Prices::default());
        let (hour, read) = lines[&Battle::Live(7, 10)].reads.last().unwrap();
        assert_eq!((*hour, read.result, read.live_last, read.end_seen), (18, NavalBattleStatus::Unknown, Some(17), false), "a battle still live at a stretch's last hour has no recorded removal");
    }

    #[test]
    fn a_battle_course_shows_census_gaps_up_to_the_branch_end_while_the_battle_is_live() {
        let read = |result| Snapshot {
            province: None,
            countries: [Vec::new(), Vec::new()],
            countries_hour: [None; 2],
            ships: [Some(2), Some(1)],
            convoys: [Some(0); 2],
            sunk: [Some(0); 2],
            convoys_sunk: [Some(0); 2],
            ic: [Some(0); 2],
            result,
            result_id: None,
            port_strike: false,
            detail_hour: None,
            result_hour: None,
            live_last: None,
            end_seen: false,
        };
        let entry = |id: u128, key: &str, reads: Steps<Snapshot>| Occurrence {
            id,
            date: reads[0].0,
            observed: reads[0].0,
            kind: 0,
            roles: Box::default(),
            subjects: Box::default(),
            states: reads.iter().map(|(at, read)| (*at, read.result as u8)).collect::<Vec<_>>().into(),
            sort_keys: [SortKey::dated_numbers(Vec::new()), SortKey::dated_numbers(Vec::new()), SortKey::dated_numbers(Vec::new())].into(),
            text: Default::default(),
            payload: Line { key: key.into(), reads, ..Line::default() },
        };
        let gap = |first_hour, last_hour| MissingSpan { first_hour, last_hour, reason: "Naval battle census incomplete".into() };
        let scanned = Scanned {
            index: Index::new(
                vec![
                    entry(0, "combat:7:100", vec![(100, read(NavalBattleStatus::Ongoing))]),
                    entry(1, "combat:8:100", vec![(100, read(NavalBattleStatus::Ongoing)), (105, read(NavalBattleStatus::Unknown))]),
                    entry(2, "combat:6:100", vec![(100, read(NavalBattleStatus::Ongoing)), (111, Snapshot { result_id: Some(61), ..read(NavalBattleStatus::AttackerWon) })]),
                    entry(3, "result:62", vec![(111, Snapshot { result_id: Some(62), ..read(NavalBattleStatus::AttackerWon) })]),
                    entry(4, "result:61", vec![(111, Snapshot { result_id: Some(61), ..read(NavalBattleStatus::AttackerWon) })]),
                ],
                Vec::new(),
                &[NavalBattleStatus::Ongoing as u8],
            )
            .unwrap(),
            live_gaps: vec![gap(90, 95), gap(110, 112), gap(118, 140)],
        };
        let course_of = |key: &str| super::entry(&scanned, key, 120).map(|found| course(&scanned, found, 120));
        let (_, live) = course_of("combat:7:100").unwrap();
        assert_eq!(live, [gap(110, 112), gap(118, 120)], "a live battle's gaps run to the branch end, past its last change");
        let (_, ended) = course_of("combat:8:100").unwrap();
        assert!(ended.is_empty(), "an ended battle's gaps stop at its last reading");
        let (_, resulted) = course_of("combat:6:100").unwrap();
        assert_eq!(resulted, [gap(110, 110)], "census gaps stop where the battle's result is recorded");
        let (reads, only) = course_of("result:62").unwrap();
        assert_eq!((reads.len(), reads[0].hour, only.len()), (1, 111, 0), "a battle held only as a result has no hours before it for a gap to cover");
        assert!(course_of("combat:9:100").is_none());
        assert_eq!(super::entry(&scanned, "result:61", 111).map(|found| found.payload.key.as_str()), Some("combat:6:100"), "a result the table joined to its battle opens as that battle");
        assert_eq!(super::entry(&scanned, "result:61", 110).map(|found| found.payload.key.as_str()), Some("result:61"), "before the table joins the result it is still the result's own battle");
        assert_eq!(super::entry(&scanned, "result:62", 120).map(|found| found.payload.key.as_str()), Some("result:62"));
    }

    fn step(hour: i32, cells: Option<&[i64]>) -> Step {
        Step { hour, cells: cells.map(|cells| cells.iter().map(|&n| Cell::Number(n)).collect()) }
    }

    #[test]
    fn ic_lost_sums_each_sides_sunk_warship_designs_and_withholds_a_side_with_an_unpriced_design() {
        let sunk = SUNK;
        let member = |side: u128, slot: u128| (7 << 33) | (side << 32) | slot;
        let battles = History { keys: vec![(7, vec![step(10, Some(&[10, 0, 0])), step(12, Some(&[10, 1, 0])), step(14, Some(&[10, 1, 1]))])], ..History::default() };
        let participants = History {
            keys: vec![
                (member(0, 1), vec![step(10, Some(&[100, 1, 0])), step(12, Some(&[100, 1, sunk])), step(13, None)]),
                (member(1, 1), vec![step(10, Some(&[200, 2, 0])), step(14, Some(&[200, 2, sunk])), step(16, Some(&[200, 3, sunk]))]),
                (member(1, 2), vec![step(10, Some(&[0, 2, 0])), step(14, Some(&[0, 2, sunk]))]),
            ],
            ..History::default()
        };
        assert_eq!(sunk_ships(&participants), [member(0, 1), member(1, 1)]);
        let prices = Prices {
            designs: History { keys: vec![(member(0, 1), vec![step(10, Some(&[70]))]), (member(1, 1), vec![step(10, Some(&[71]))])], ..History::default() },
            costs: History {
                keys: vec![(70, vec![step(10, Some(&[92_250_000]))]), (71, vec![step(10, Some(&[])), step(15, Some(&[50_000_000]))])],
                statuses: vec![lanes::StatusRun { first: 10, last: 20, status: Status::Complete }],
                ..History::default()
            },
        };
        let span = Stretch { run: 1, first: 10, last: 20, join: dashboard_query::timeline::Join::Start };
        let mut lines = BTreeMap::new();
        live_lines(&mut lines, span, span.last, &RunLinks::default(), &battles, &History::default(), &participants, &prices);
        let reads = &lines[&Battle::Live(7, 10)].reads;
        assert_eq!(reads.iter().map(|(hour, read)| (*hour, read.ic)).collect::<Vec<_>>(), [
            (10, [Some(0), Some(0)]), (12, [Some(92_250_000), Some(0)]), (14, [Some(92_250_000), None]),
        ]);
    }

    #[test]
    fn a_linked_result_keeps_the_ic_lost_of_the_hour_it_observed() {
        let read = |sunk: [u32; 2], ic: [Option<i64>; 2]| Snapshot {
            province: None,
            countries: [Vec::new(), Vec::new()],
            countries_hour: [None; 2],
            ships: [Some(5); 2],
            convoys: [Some(0); 2],
            sunk: sunk.map(Some),
            convoys_sunk: [Some(0); 2],
            ic,
            result: NavalBattleStatus::Ongoing,
            result_id: None,
            port_strike: false,
            detail_hour: None,
            result_hour: None,
            live_last: None,
            end_seen: false,
        };
        let mut lines = BTreeMap::new();
        lines.insert(Battle::Live(7, 10), Line { reads: vec![(10, read([1, 0], [Some(5), Some(0)])), (12, read([2, 0], [None, Some(0)]))], ..Line::default() });
        let observed = i64::from(NavalResultObservationRow::OBSERVED);
        let results = History { keys: vec![(99, vec![step(13, Some(&[1, 13, 1, 1]))])], ..History::default() };
        let observations = History { keys: vec![(99, vec![step(13, Some(&[observed, 7, 10, 11, 5, 0, 1, 0, 5, 0, 0, 0]))])], ..History::default() };
        let span = Stretch { run: 1, first: 10, last: 20, join: dashboard_query::timeline::Join::Start };
        result_lines(&mut lines, span, &RunLinks::default(), &results, &observations, &History::default());
        let (hour, last) = lines[&Battle::Live(7, 10)].reads.last().unwrap();
        assert_eq!((*hour, last.result, last.sunk, last.ic), (13, NavalBattleStatus::AttackerWon, [Some(1), Some(0)], [Some(5), Some(0)]));
        assert!(last.port_strike);
        assert_eq!((last.live_last, last.end_seen), (Some(13), false), "a result read while the battle is still live bounds its live hours at that read");
        let mut lines = BTreeMap::new();
        let cut = Snapshot { result: NavalBattleStatus::Unknown, live_last: Some(17), ..read([1, 0], [Some(5), Some(0)]) };
        lines.insert(Battle::Live(7, 10), Line { reads: vec![(10, read([1, 0], [Some(5), Some(0)])), (18, cut), (20, read([1, 0], [Some(5), Some(0)]))], ..Line::default() });
        let results = History { keys: vec![(99, vec![step(21, Some(&[1, 21, 0, 1]))])], ..History::default() };
        let observations = History {
            keys: vec![(99, vec![step(21, Some(&[observed, 7, 10, 21, 5, 0, 1, 0, 5, 0, 0, 0])), step(23, Some(&[observed, 7, 10, 21, 5, 0, 2, 0, 5, 0, 0, 0]))])],
            ..History::default()
        };
        let span = Stretch { run: 2, first: 20, last: 30, join: dashboard_query::timeline::Join::Start };
        result_lines(&mut lines, span, &RunLinks::default(), &results, &observations, &History::default());
        let (hour, last) = lines[&Battle::Live(7, 10)].reads.last().unwrap();
        assert_eq!((*hour, last.live_last, last.end_seen), (23, Some(21), false), "a battle live again after a stretch ended takes its live bound from its newest read");
        assert_eq!(last.result_hour, Some(21), "a refreshed result keeps the hour it was first read");
        let mut lines = BTreeMap::new();
        let results = History { keys: vec![(77, vec![step(20, Some(&[1, 20, 0, 1])), step(23, Some(&[2, 20, 0, 1]))])], ..History::default() };
        let observations = History {
            keys: vec![(77, vec![step(20, Some(&[0])), step(23, Some(&[observed, 7, 10, 20, 5, 0, 1, 0, 5, 0, 0, 0]))])],
            ..History::default()
        };
        result_lines(&mut lines, span, &RunLinks::default(), &results, &observations, &History::default());
        assert_eq!(lines[&Battle::Result(77, 20)].reads[0].1.result_hour, Some(20));
        let (hour, linked) = lines[&Battle::Live(7, 10)].reads.last().unwrap();
        assert_eq!((*hour, linked.result_hour), (23, Some(20)), "a result linked to its battle later keeps the hour it was first read on its own");
    }

    #[test]
    fn a_convoy_only_side_carries_its_convoy_counts_in_live_and_result_rows() {
        let member = |side: u128, slot: u128| (7 << 33) | (side << 32) | slot;
        let battles = History {
            keys: vec![(7, vec![step(10, Some(&[10, 0, 0, 0, 0])), step(12, Some(&[10, 0, 0, 0, 2]))])],
            ..History::default()
        };
        let participants = History {
            keys: vec![
                (member(0, 1), vec![step(10, Some(&[100, 1, 0]))]),
                (member(0, 2), vec![step(10, Some(&[101, 1, 0]))]),
                (member(1, 1), vec![step(10, Some(&[0, 2, 0]))]),
                (member(1, 2), vec![step(10, Some(&[0, 2, 0])), step(12, Some(&[0, 2, SUNK]))]),
                (member(1, 3), vec![step(10, Some(&[0, 2, 0])), step(12, Some(&[0, 2, SUNK]))]),
            ],
            statuses: vec![lanes::StatusRun { first: 10, last: 20, status: Status::Complete }],
            ..History::default()
        };
        let span = Stretch { run: 1, first: 10, last: 20, join: dashboard_query::timeline::Join::Start };
        let mut lines = BTreeMap::new();
        live_lines(&mut lines, span, span.last, &RunLinks::default(), &battles, &History::default(), &participants, &Prices::default());
        let reads = &lines[&Battle::Live(7, 10)].reads;
        assert_eq!(reads[0].1.ships, [Some(2), Some(0)]);
        assert_eq!(reads[0].1.convoys, [Some(0), Some(3)]);
        assert_eq!(reads[0].1.sunk, [Some(0), Some(0)]);
        assert_eq!(reads[0].1.convoys_sunk, [Some(0), Some(0)]);
        assert_eq!(reads[1].1.convoys_sunk, [Some(0), Some(2)]);

        let observed = i64::from(NavalResultObservationRow::OBSERVED);
        let results = History { keys: vec![(99, vec![step(13, Some(&[1, 13, 0, 1]))])], ..History::default() };
        let observations = History { keys: vec![(99, vec![step(13, Some(&[observed, 7, 10, 12, 2, 0, 0, 0, 0, 3, 0, 2]))])], ..History::default() };
        result_lines(&mut lines, span, &RunLinks::default(), &results, &observations, &History::default());
        let ended = &lines[&Battle::Live(7, 10)].reads.last().unwrap().1;
        assert_eq!((ended.ships, ended.convoys, ended.sunk, ended.convoys_sunk), ([Some(2), Some(0)], [Some(0), Some(3)], [Some(0), Some(0)], [Some(0), Some(2)]));

        let incomplete = History { keys: participants.keys.clone(), ..History::default() };
        let mut unread = BTreeMap::new();
        live_lines(&mut unread, span, span.last, &RunLinks::default(), &battles, &History::default(), &incomplete, &Prices::default());
        let first = &unread[&Battle::Live(7, 10)].reads[0].1;
        assert_eq!((first.ships, first.convoys), ([None, None], [None, None]));
    }

    #[test]
    fn a_result_from_before_the_recording_names_its_sides_at_the_first_recorded_hour() {
        let results = History { keys: vec![(99, vec![step(10, Some(&[1, 5, 0, 2]))])], ..History::default() };
        let countries = History {
            keys: vec![((99 << 40) | 7, vec![step(10, Some(&[0]))]), ((99 << 40) | (1 << 32) | 8, vec![step(10, Some(&[0]))])],
            ..History::default()
        };
        let links = RunLinks { slots: HashMap::from([(7, vec![(10, 20, Some(3))]), (8, vec![(10, 20, Some(4))])]) };
        let span = Stretch { run: 1, first: 10, last: 20, join: dashboard_query::timeline::Join::Start };
        let mut lines = BTreeMap::new();
        result_lines(&mut lines, span, &links, &results, &History::default(), &countries);
        let (hour, read) = lines[&Battle::Result(99, 5)].reads.first().unwrap();
        assert_eq!((*hour, read.result, read.countries.clone(), read.countries_hour), (10, NavalBattleStatus::DefenderWon, [vec![3], vec![4]], [Some(10); 2]));
        let after = History { keys: vec![(99, vec![step(30, Some(&[1, 5, 0, 2]))])], ..History::default() };
        let lists = History {
            keys: vec![((99 << 40) | 7, vec![step(30, Some(&[0]))]), ((99 << 40) | (1 << 32) | 8, vec![step(30, Some(&[0]))])],
            ..History::default()
        };
        let relinked = RunLinks { slots: HashMap::from([(7, vec![(30, 40, Some(5))]), (8, vec![(30, 40, Some(6))])]) };
        let loaded = Stretch { run: 2, first: 30, last: 40, join: dashboard_query::timeline::Join::Start };
        result_lines(&mut lines, loaded, &relinked, &after, &History::default(), &lists);
        let (hour, read) = lines[&Battle::Result(99, 5)].reads.last().unwrap();
        assert_eq!((*hour, read.countries.clone(), read.countries_hour), (30, [vec![3], vec![4]], [Some(10); 2]));
    }
}
