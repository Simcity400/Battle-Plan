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
    pub province: Option<u32>,
    pub countries: [Vec<u32>; 2],
    pub countries_hour: [i32; 2],
    pub read_hour: i32,
    pub ships_sunk: [Option<u32>; 2],
    pub ic_lost: [Option<f64>; 2],
    pub ships: [Option<u32>; 2],
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
    sunk: [Option<u32>; 2],
    ic: [Option<i64>; 2],
    result: NavalBattleStatus,
    result_id: Option<u64>,
    detail_hour: Option<i32>,
}

#[derive(Default)]
struct Line {
    key: String,
    reads: Steps<Snapshot>,
    joined: Option<i32>,
}

impl Line {
    fn end_live(&mut self, hour: i32) {
        if let Some((_, previous)) = self.reads.last()
            && previous.result == NavalBattleStatus::Ongoing
        {
            let mut ended = previous.clone();
            ended.result = NavalBattleStatus::Unknown;
            ended.detail_hour = Some(hour - 1);
            push(&mut self.reads, hour, ended);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Battle {
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
            let last = next_battle.map_or(span.last, |s| s.hour - 1);
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
                let mut countries = [Vec::new(), Vec::new()];
                for (key, reads) in roster {
                    let Some(cells) = at(reads, hour) else {
                        continue;
                    };
                    let side = ((key >> 32) & 1) as usize;
                    if let Some(n) = &mut ships[side] {
                        match number(cells, 0) {
                            Some(0) => {}
                            Some(_) => *n += 1,
                            None => ships[side] = None,
                        }
                    }
                    country(&mut countries[side], count(cells, 1).and_then(|raw| links.slot_at(raw, hour)));
                    if sunk_ship(cells) {
                        lost.entry(*key).or_insert_with(|| prices.of(*key, hour));
                    }
                }
                let sunk = [count(cells, 1), count(cells, 2)];
                push(
                    &mut line.reads,
                    hour,
                    Snapshot {
                        province: at(places, hour).and_then(|v| count(v, 0)),
                        ships,
                        countries,
                        countries_hour: [None; 2],
                        sunk,
                        ic: ic_lost(lost, sunk),
                        result: NavalBattleStatus::Ongoing,
                        result_id: None,
                        detail_hour: None,
                    },
                );
            }
            if last < last_hour && !next_battle.and_then(|s| s.cells.as_deref()).is_some_and(|c| number(c, 0) == Some(i64::from(first_hour))) {
                line.end_live(last + 1);
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
                let sunk = [count(obs, 5), count(obs, 7)];
                let observed = number(obs, 3).and_then(|h| i32::try_from(h).ok()).and_then(|end| line.reads.iter().rev().find(|(at, _)| *at <= end));
                let ic = std::array::from_fn(|i| observed.filter(|(_, prior)| prior.sunk[i] == sunk[i]).and_then(|(_, prior)| prior.ic[i]));
                let status = match number(cells, 2) {
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
                        ships: [count(obs, 4), count(obs, 6)],
                        sunk,
                        ic,
                        result: status,
                        result_id: Some(*key as u64),
                        detail_hour: Some(hour),
                    },
                );
            }
        }
    }
}

fn scan(source: &Source, query: &Query<'_>, cancelled: &AtomicBool) -> io::Result<Index<Line>> {
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
            gaps.extend(
                dashboard_land::products::division_log::unread(&read, span.first, span.last)
                    .into_iter()
                    .map(|(first_hour, last_hour)| MissingSpan {
                        first_hour,
                        last_hour,
                        reason: absent(table).unwrap_or(reason).to_owned(),
                    }),
            );
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
            &history(TableId::NavalBattles, &[0, 3, 4])?,
            &history(TableId::NavalCombats, &[0])?,
            &participants,
            &prices,
        );
        result_lines(
            &mut lines,
            span,
            &links,
            &history(TableId::NavalResults, &[0, 1, 8])?,
            &history(TableId::NavalResultObservations, &[0, 1, 3, 4, 7, 9, 11, 13])?,
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
        for (at, read) in &line.reads {
            push(&mut states, *at, read.result as u8);
            push(&mut sunk, *at, total(read.sunk));
            push(&mut ic, *at, read.ic[0].zip(read.ic[1]).map(|(a, b)| a.saturating_add(b)));
            push(&mut ships, *at, total(read.ships));
            roles.extend(read.countries.iter().flatten().map(|slot| Role { slot: Some(*slot), at: *at }));
        }
        if let Some(joined) = line.joined {
            push(&mut states, joined, JOINED);
        }
        line.key = battle.key();
        entries.push(Occurrence {
            id: entries.len() as u128,
            date: battle.date(),
            observed,
            kind: 0,
            roles: roles.into(),
            subjects: Box::default(),
            states: states.into(),
            sort_keys: [SortKey::dated_numbers(sunk), SortKey::dated_numbers(ic), SortKey::dated_numbers(ships)].into(),
            text: Box::default(),
            payload: line,
        });
    }
    Index::new(entries, gaps, &[NavalBattleStatus::Ongoing as u8])
}

#[derive(Default)]
struct Branches(HashMap<u64, Arc<Index<Line>>>);

struct Prepared {
    index: Arc<Index<Line>>,
    selection: Option<Selection>,
}

fn prepare(source: &Source, query: &Query<'_>, load: &LoadContext, params: &NavalBattlesTailParams, cancelled: &AtomicBool) -> io::Result<Prepared> {
    let state = source.state::<Branches>();
    let mut branches = state.lock().unwrap_or_else(PoisonError::into_inner);
    let index = match branches.0.entry(query.timeline.id) {
        std::collections::hash_map::Entry::Occupied(e) => Arc::clone(e.get()),
        std::collections::hash_map::Entry::Vacant(e) => Arc::clone(e.insert(Arc::new(scan(source, query, cancelled)?))),
    };
    drop(branches);
    let selection = dashboard_world::products::group_members::dated(source, query, load, params.countries.as_deref(), params.group.as_ref(), cancelled)?;
    Ok(Prepared { index, selection })
}

fn reply(prepared: &Prepared, holders: &Holders, params: &NavalBattlesTailParams, hour: i32) -> io::Result<NavalBattlesTailReply> {
    let (offset, limit) = occurrences::window(params.offset.as_deref(), params.limit)?;
    let (order, _) = occurrences::order(params.sort.as_ref(), SORT_COLUMNS)?;
    let mut ask = Ask {
        hour,
        from: params.from,
        to: params.to,
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
    let gap_end = params.to.unwrap_or(i32::MAX);
    let answer = prepared.index.ask_with_gap_end(holders, &ask, gap_end)?;
    let mut visible_change: Option<i32> = None;
    let rows = answer
        .rows
        .iter()
        .map(|shown| {
            let entry = prepared.index.get(shown.at);
            let line = &entry.payload;
            let mut next = |candidate: Option<i32>| {
                visible_change = visible_change.into_iter().chain(candidate).min();
            };
            let partition = line.reads.partition_point(|(at, _)| *at <= hour);
            let (at, read) = &line.reads[partition - 1];
            next(line.reads.get(partition).map(|step| step.0));
            let result = read.result;
            NavalBattleLogRow {
                key: line.key.clone(),
                hour: entry.date,
                province: read.province,
                result,
                detail_id: read.result_id.map_or_else(|| line.key.clone(), |id| format!("result:{id}")),
                detail_hour: read.detail_hour,
                ships_sunk: read.sunk,
                ic_lost: read.ic.map(|raw| raw.map(|raw| raw as f64 / 100_000.0)),
                ships: read.ships,
                countries: read.countries.clone(),
                countries_hour: read.countries_hour.map(|h| h.unwrap_or(*at)),
                read_hour: *at,
            }
        })
        .collect();
    let mut boundary = answer.next_change;
    while let Some(candidate) = boundary.filter(|at| visible_change.is_none_or(|visible| *at < visible)) {
        ask.hour = candidate;
        let future = prepared.index.ask_with_gap_end(holders, &ask, gap_end)?;
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
        offset: offset.to_string(),
        rows,
        missing: answer.gaps,
        next_change,
        warnings,
    };
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
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

pub fn dated_panel(source: &Source, load: &LoadContext, asked: &serde_json::Value) -> Result<Box<dyn Panel>, String> {
    let mut request = asked.clone();
    request.as_object_mut().ok_or("Invalid naval-battles-tail panel")?.remove("panel");
    let params: NavalBattlesTailParams = dashboard_query::named::named(&request).map_err(|error| format!("Invalid naval-battles-tail panel: {error}"))?;
    NavalBattlesTail::validate(&params)?;
    let branch = params.branch_id.parse().map_err(|_| "Invalid branch")?;
    let query = dashboard_base::queries::query(source, branch).map_err(|error| error.to_string())?;
    let prepared = prepare(source, &query, load, &params, &AtomicBool::new(false)).map_err(|error| error.to_string())?;
    let (first, last, holders) = (query.timeline.first, query.timeline.last, query.holders);
    Ok(occurrences::occurrence_panel(source.tables(), move |hour, cancelled| {
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
        let battles = History { keys: vec![(7, battle)], ..History::default() };
        let span = Stretch { run: 1, first: 10, last: 40, join: dashboard_query::timeline::Join::Start };
        let mut lines = BTreeMap::new();
        live_lines(&mut lines, span, span.last, &RunLinks::default(), &battles, &History::default(), &History::default(), &Prices::default());
        let reads = &lines[&Battle::Live(7, 10)].reads;
        assert_eq!(reads.iter().map(|(hour, read)| (*hour, read.result)).collect::<Vec<_>>(), [
            (10, NavalBattleStatus::Ongoing), (15, NavalBattleStatus::Ongoing), (20, NavalBattleStatus::Unknown),
        ]);
        assert_eq!(reads.last().unwrap().1.detail_hour, Some(19));
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
            sunk: sunk.map(Some),
            ic,
            result: NavalBattleStatus::Ongoing,
            result_id: None,
            detail_hour: None,
        };
        let mut lines = BTreeMap::new();
        lines.insert(Battle::Live(7, 10), Line { reads: vec![(10, read([1, 0], [Some(5), Some(0)])), (12, read([2, 0], [None, Some(0)]))], ..Line::default() });
        let observed = i64::from(NavalResultObservationRow::OBSERVED);
        let results = History { keys: vec![(99, vec![step(13, Some(&[1, 13, 1]))])], ..History::default() };
        let observations = History { keys: vec![(99, vec![step(13, Some(&[observed, 7, 10, 11, 5, 1, 5, 0]))])], ..History::default() };
        let span = Stretch { run: 1, first: 10, last: 20, join: dashboard_query::timeline::Join::Start };
        result_lines(&mut lines, span, &RunLinks::default(), &results, &observations, &History::default());
        let (hour, last) = lines[&Battle::Live(7, 10)].reads.last().unwrap();
        assert_eq!((*hour, last.result, last.sunk, last.ic), (13, NavalBattleStatus::AttackerWon, [Some(1), Some(0)], [Some(5), Some(0)]));
    }

    #[test]
    fn a_result_from_before_the_recording_names_its_sides_at_the_first_recorded_hour() {
        let results = History { keys: vec![(99, vec![step(10, Some(&[1, 5, 2]))])], ..History::default() };
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
        let after = History { keys: vec![(99, vec![step(30, Some(&[1, 5, 2]))])], ..History::default() };
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
