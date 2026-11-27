use super::{Columns, Combats, Key, at, combats, ended_battles, live_battles};
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
use rayon::prelude::*;
use replay_data::rows::{LandEquipmentLoss, TableId, VariantStatsRow};
use replay_lanes::lanes::{self, Cell, Keys, Reading};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, PoisonError, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LandBattlesTailParams {
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
pub enum LandBattleStatus {
    Ongoing,
    AttackerWon,
    DefenderWon,
    Unknown,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LandBattleLogRow {
    pub key: String,
    pub hour: i32,
    pub exact: bool,
    pub province: Option<u32>,
    pub state: Option<u32>,
    pub countries: [Vec<u32>; 2],
    pub countries_unknown: [bool; 2],
    pub countries_hour: [i32; 2],
    pub casualties: [Option<i32>; 2],
    pub ic_lost: [Option<f64>; 2],
    pub losses_so_far: bool,
    pub divisions: [Option<u32>; 2],
    pub result: LandBattleStatus,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LandBattlesTailReply {
    pub branch_id: String,
    pub total: String,
    pub offset: String,
    pub rows: Vec<LandBattleLogRow>,
    pub missing: Vec<PlacedGap>,
    pub next_change: Option<i32>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(LandBattlesTailReply);

impl OccurrenceReply for LandBattlesTailReply {
    fn next_change(&self) -> Option<i32> {
        self.next_change
    }
    fn row_count(&self) -> usize {
        self.rows.len()
    }
}

const SORT_COLUMNS: &[&str] = &["casualties", "icLost", "divisions"];
type Steps<T> = Vec<(i32, T)>;

fn push<T: PartialEq>(steps: &mut Steps<T>, hour: i32, value: T) {
    if steps.last().is_some_and(|(at, _)| *at == hour) {
        steps.pop();
    }
    if steps.last().is_none_or(|(_, held)| *held != value) {
        steps.push((hour, value));
    }
}

fn extend<T: PartialEq>(steps: &mut Steps<T>, new: Steps<T>) {
    for (hour, value) in new {
        push(steps, hour, value);
    }
}

#[derive(Default, PartialEq)]
struct Sides {
    read: [Option<i32>; 2],
    countries: [Vec<u32>; 2],
    unknown: [bool; 2],
}

#[derive(Clone, Copy, Default, PartialEq)]
struct Losses {
    manpower: [Option<i32>; 2],
    ic_raw: [Option<i64>; 2],
    so_far: bool,
}

#[derive(Default)]
struct Line {
    written: Option<i32>,
    exact: bool,
    states: Steps<u8>,
    place: Steps<(Option<u32>, Option<u32>)>,
    divisions: Steps<[Option<u32>; 2]>,
    sides: Steps<Sides>,
    losses: Steps<Losses>,
}

fn ic(pool: Option<&[LandEquipmentLoss]>, costs: &lanes::History, hour: i32) -> Option<i64> {
    let mut total = 0_i128;
    let amount = pool?;
    for item in amount {
        if item.amount_raw == 0 {
            continue;
        }
        let Reading::Cells([Cell::Number(price)]) = costs.reading(u128::from(item.variant), hour) else {
            return None;
        };
        total = total.checked_add(i128::from(*price) * i128::from(item.amount_raw) / 100_000)?;
    }
    i64::try_from(total).ok()
}

fn known_sides(slots: &[Option<Vec<u32>>; 2], unknown: u8, mut slot: impl FnMut(u32) -> Option<u32>) -> Sides {
    let mut sides = Sides::default();
    for (i, side) in slots.iter().enumerate() {
        sides.unknown[i] = unknown & (1 << i) != 0 || side.as_ref().is_none_or(Vec::is_empty);
        for &raw in side.iter().flatten() {
            if let Some(slot) = slot(raw) {
                if !sides.countries[i].contains(&slot) {
                    sides.countries[i].push(slot);
                }
            } else {
                sides.unknown[i] = true;
            }
        }
    }
    sides
}

fn during<T>(steps: &[(i32, T)], first: i32, last: i32) -> impl Iterator<Item = (i32, i32, Option<&T>)> {
    let start = steps.partition_point(|&(h, _)| h <= first);
    let end = steps.partition_point(|&(h, _)| h <= last);
    (start..=end).map(move |i| {
        let hour = if i == start { first } else { steps[i - 1].0 };
        let through = if i == end { last } else { steps[i].0 - 1 };
        (hour, through, i.checked_sub(1).map(|i| &steps[i].1))
    })
}

struct Readings<'a> {
    links: &'a RunLinks,
    costs: &'a lanes::History,
    provinces: &'a lanes::History,
}

impl Readings<'_> {
    fn place(&self, province: Option<u32>, hour: i32) -> (Option<u32>, Option<u32>) {
        let state = province.and_then(|id| match self.provinces.reading(u128::from(id), hour) {
            Reading::Cells([Cell::Number(state)]) => u32::try_from(*state).ok().filter(|id| *id != 0),
            _ => None,
        });
        (province, state)
    }
}

struct BlockInput {
    found: BTreeMap<Key, super::Battle>,
    data: Combats,
}

fn prepare_block(query: &Query<'_>, columns: &Columns, span: Stretch, cancelled: &AtomicBool) -> io::Result<BlockInput> {
    check_cancel(cancelled)?;
    let recording = query.replay.recording();
    let mut found = BTreeMap::new();
    if recording.withheld_reason(TableId::LandBattles.index()).is_none() {
        live_battles(query, columns, span.run, span.first, span.last, None, &mut found)?;
    }
    if recording.withheld_reason(TableId::LandBattleEnds.index()).is_none() {
        ended_battles(query, columns, span.run, span.first, span.last, None, &mut found)?;
    }
    let mut handles: Vec<u64> = found.keys().map(|key| key.0).collect();
    handles.dedup();
    let data = combats(query, columns, span.run, span.first, span.last, &handles, false)?;
    Ok(BlockInput { found, data })
}

fn scan_block(query: &Query<'_>, BlockInput { found, data }: BlockInput, readings: &Readings<'_>, cancelled: &AtomicBool) -> io::Result<Vec<(Key, Line)>> {
    check_cancel(cancelled)?;
    let mut out = Vec::with_capacity(found.len());
    for (key, battle) in found {
        let (combat, _, _) = key;
        let mut line = Line {
            exact: battle.exact,
            ..Line::default()
        };
        for (first, last) in battle.live {
            push(&mut line.states, first, LandBattleStatus::Ongoing as u8);
            for (hour, _, read) in during(data.census.get(&combat).map_or(&[][..], Vec::as_slice), first, last) {
                let read = read.and_then(Option::as_ref);
                push(&mut line.place, hour, readings.place(read.and_then(|r| r.province), key.1));
                push(
                    &mut line.divisions,
                    hour,
                    read.map_or([None; 2], |r| r.counts),
                );
            }
            for (hour, through, sides) in during(data.sides.get(&combat).map_or(&[][..], Vec::as_slice), first, last) {
                let slots = sides.and_then(Option::as_ref).unwrap_or(&[None, None]);
                let mut changes = vec![hour];
                for &raw in slots.iter().flatten().flatten() {
                    changes.extend(readings.links.within(raw, hour, through).skip(1).map(|(_, h, _)| h));
                }
                changes.sort_unstable();
                changes.dedup();
                for h in changes {
                    push(&mut line.sides, h, known_sides(slots, 0, |raw| readings.links.slot_at(raw, h)));
                }
            }
            for (hour, through, read) in during(data.losses.get(&combat).map_or(&[][..], Vec::as_slice), first, last) {
                let Some(read) = read.and_then(Option::as_ref) else {
                    push(&mut line.losses, hour, Losses { so_far: true, ..Losses::default() });
                    continue;
                };
                let mut changes = vec![hour];
                for item in read.equipment.iter().flatten().flatten().filter(|item| item.amount_raw != 0) {
                    if let Ok(i) = readings.costs.keys.binary_search_by_key(&u128::from(item.variant), |(id, _)| *id) {
                        let steps = &readings.costs.keys[i].1;
                        let after = steps.partition_point(|step| step.hour <= hour);
                        changes.extend(steps[after..].iter().take_while(|step| step.hour <= through).map(|step| step.hour));
                    }
                }
                changes.sort_unstable();
                changes.dedup();
                for h in changes {
                    push(
                        &mut line.losses,
                        h,
                        Losses {
                            manpower: read.manpower,
                            ic_raw: read.equipment.each_ref().map(|pool| ic(pool.as_deref(), readings.costs, h)),
                            so_far: true,
                        },
                    );
                }
            }
            if last < query.timeline.last {
                push(&mut line.states, last + 1, LandBattleStatus::Unknown as u8);
            }
        }
        if let Some(end) = battle.end {
            let status = match (end.resulted(), end.winners) {
                (true, Some(1)) => LandBattleStatus::AttackerWon,
                (true, Some(2)) => LandBattleStatus::DefenderWon,
                _ => LandBattleStatus::Unknown,
            };
            let hour = end.written;
            line.states.retain(|(h, _)| *h < hour);
            push(&mut line.states, hour, status as u8);
            if let Some(province) = end.province {
                push(&mut line.place, hour, readings.place(Some(province), key.1));
            }
            let mut sides = known_sides(&end.sides, end.countries_unknown, Some);
            sides.read = [end.last_read; 2];
            if end.resulted() {
                let result = known_sides(&end.result_sides, end.result_unknown, Some);
                for (i, countries) in result.countries.into_iter().enumerate() {
                    if !countries.is_empty() || sides.countries[i].is_empty() {
                        sides.countries[i] = countries;
                        sides.read[i] = end.end_hour;
                    }
                    sides.unknown[i] = result.unknown[i];
                }
            }
            push(&mut line.sides, hour, sides);
            push(
                &mut line.losses,
                hour,
                Losses {
                    manpower: end.terminal,
                    ic_raw: end.terminal_equipment.each_ref().map(|pool| ic(pool.as_deref(), readings.costs, hour)),
                    so_far: false,
                },
            );
        }
        if !line.states.is_empty() {
            out.push((key, line));
        }
    }
    Ok(out)
}

fn dated<T>(steps: &Steps<T>, value: impl Fn(&T) -> Option<i64>) -> SortKey {
    let mut keys = Vec::new();
    for (hour, step) in steps {
        push(&mut keys, *hour, value(step));
    }
    SortKey::dated_numbers(keys)
}

fn sum<T: Copy + Into<i64>>(pair: [Option<T>; 2]) -> Option<i64> {
    pair[0]?.into().checked_add(pair[1]?.into())
}

fn scan(source: &Source, query: &Query<'_>, cancelled: &AtomicBool) -> io::Result<Index<Line>> {
    let recording = query.replay.recording();
    let mut gaps: Vec<MissingSpan> = query
        .timeline
        .unheld
        .iter()
        .map(|g| MissingSpan {
            first_hour: g.first,
            last_hour: g.last,
            reason: g.reason.clone(),
        })
        .collect();
    let mut lines = HashMap::<Key, Line>::new();
    let columns = Columns::new()?;
    for stretch in &query.timeline.stretches {
        for (table, reason) in [
            (TableId::LandBattles, "Land battle census incomplete"),
            (TableId::LandBattleEnds, "Land battle endings incomplete"),
        ] {
            let absent = recording.withheld_reason(table.index()).or_else(|| recording.predates(table.index()));
            let read = if absent.is_some() {
                Vec::new()
            } else {
                lanes::complete_reads(recording, stretch.run, table, stretch.first, stretch.last)?
            };
            gaps.extend(
                super::super::division_log::unread(&read, stretch.first, stretch.last)
                    .into_iter()
                    .map(|(first_hour, last_hour)| MissingSpan {
                        first_hour,
                        last_hour,
                        reason: absent.unwrap_or(reason).to_owned(),
                    }),
            );
        }
        let links = queries::links(source, stretch.run)?;
        let origin = recording.run(stretch.run)?.first_hour;
        let spans = dashboard_world::products::army_windows::coalesce(origin, [(stretch.first, stretch.last)]);
        let inputs: Vec<_> = spans
            .par_iter()
            .map(|s| {
                prepare_block(
                    query,
                    &columns,
                    Stretch {
                        first: s.first_hour,
                        last: s.last_hour,
                        ..*stretch
                    },
                    cancelled,
                )
            })
            .collect::<io::Result<_>>()?;
        let mut province_keys = BTreeSet::new();
        let mut variant_keys = BTreeSet::new();
        let mut add_variants = |equipment: &[Option<Vec<LandEquipmentLoss>>; 2]| {
            variant_keys.extend(equipment.iter().flatten().flatten().filter(|item| item.amount_raw != 0).map(|item| u128::from(item.variant)));
        };
        for BlockInput { found, data } in &inputs {
            for read in data.census.values().flatten().filter_map(|(_, read)| read.as_ref()) {
                province_keys.extend(read.province.map(u128::from));
            }
            for read in data.losses.values().flatten().filter_map(|(_, read)| read.as_ref()) {
                add_variants(&read.equipment);
            }
            for end in found.values().filter_map(|battle| battle.end.as_ref()) {
                province_keys.extend(end.province.map(u128::from));
                add_variants(&end.terminal_equipment);
            }
        }
        let history = |table: TableId, columns: &[usize], keys: BTreeSet<u128>| {
            if keys.is_empty() || recording.predates(table.index()).is_some() || recording.withheld_reason(table.index()).is_some() {
                return Ok(lanes::History::default());
            }
            let keys: Vec<_> = keys.into_iter().collect();
            lanes::history(recording, stretch.run, table, columns, Keys::These(&keys), stretch.first, stretch.last)
        };
        let costs = history(TableId::VariantStats, &[VariantStatsRow::BUILD_COST_IC], variant_keys)?;
        let provinces = history(TableId::Provinces, &[dashboard_declare::field(TableId::Provinces, "state")?], province_keys)?;
        let readings = Readings { links: &links, costs: &costs, provinces: &provinces };
        let blocks: Vec<_> = inputs.into_par_iter().map(|input| scan_block(query, input, &readings, cancelled)).collect::<io::Result<_>>()?;
        for (key, part) in blocks.into_iter().flatten() {
            let line = lines.entry(key).or_default();
            if line.states.is_empty() {
                line.exact = part.exact;
            }
            extend(&mut line.states, part.states);
            extend(&mut line.place, part.place);
            extend(&mut line.divisions, part.divisions);
            extend(&mut line.sides, part.sides);
            extend(&mut line.losses, part.losses);
        }
    }
    let mut entries = Vec::with_capacity(lines.len());
    for ((combat, start, written), mut line) in lines {
        let observed = line.states[0].0;
        let id = (u128::from(combat) << 64) | (u128::from(start as u32) << 32) | u128::from(written.map_or(0, |h| h as u32));
        line.written = written;
        line.place.shrink_to_fit();
        line.divisions.shrink_to_fit();
        line.sides.shrink_to_fit();
        line.losses.shrink_to_fit();
        let roles = line
            .sides
            .iter()
            .flat_map(|(at, sides)| sides.countries.iter().flatten().map(|slot| Role::at(Some(*slot), *at)))
            .collect();
        let sort_keys = [
            dated(&line.losses, |loss| sum(loss.manpower)),
            dated(&line.losses, |loss| sum(loss.ic_raw)),
            dated(&line.divisions, |counts| sum(*counts)),
        ]
        .into();
        entries.push(Occurrence {
            id,
            date: start,
            observed,
            kind: 0,
            roles,
            subjects: Box::default(),
            states: std::mem::take(&mut line.states).into(),
            sort_keys,
            text: Default::default(),
            payload: line,
        });
    }
    Index::new(entries, gaps, &[LandBattleStatus::Ongoing as u8])
}

#[derive(Default)]
struct Branches(HashMap<u64, Arc<Index<Line>>>);

struct Prepared {
    branch: u64,
    index: Arc<Index<Line>>,
    selection: Option<Selection>,
}

fn index(source: &Source, query: &Query<'_>, cancelled: &AtomicBool) -> io::Result<Arc<Index<Line>>> {
    let state = source.state::<Branches>();
    let mut branches = state.lock().unwrap_or_else(PoisonError::into_inner);
    Ok(match branches.0.entry(query.timeline.id) {
        std::collections::hash_map::Entry::Occupied(e) => Arc::clone(e.get()),
        std::collections::hash_map::Entry::Vacant(e) => Arc::clone(e.insert(Arc::new(scan(source, query, cancelled)?))),
    })
}

pub struct Battle {
    pub combat: u64,
    pub start: i32,
    pub written: Option<i32>,
    pub observed: i32,
    pub province: Option<u32>,
}

pub fn battles(source: &Source, query: &Query<'_>, cancelled: &AtomicBool) -> io::Result<Vec<Battle>> {
    let index = index(source, query, cancelled)?;
    Ok((0..index.len() as u32).map(|i| {
        let entry = index.get(i);
        Battle { combat: (entry.id >> 64) as u64, start: entry.date, written: entry.payload.written,
            observed: entry.observed, province: entry.payload.place.first().and_then(|(_, (province, _))| *province) }
    }).collect())
}

fn prepare(source: &Source, query: &Query<'_>, load: &LoadContext, params: &LandBattlesTailParams, cancelled: &AtomicBool) -> io::Result<Prepared> {
    let index = index(source, query, cancelled)?;
    let selection =
        dashboard_world::products::group_members::dated(source, query, load, params.countries.as_deref(), params.group.as_ref(), cancelled)?;
    Ok(Prepared { branch: query.timeline.id, index, selection })
}

fn reply(prepared: &Prepared, holders: &Holders, params: &LandBattlesTailParams, hour: i32) -> io::Result<LandBattlesTailReply> {
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
        states: None,
        search: None,
        order,
        live_first: matches!(order, Order::Date(_)),
        offset,
        limit,
        chips: &[],
    };
    let answer = prepared.index.ask(holders, &ask)?;
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
            let state = entry.states.partition_point(|step| step.0 <= hour);
            next(entry.states.get(state).map(|step| step.0));
            let partition = line.sides.partition_point(|(h, _)| *h <= hour);
            let sides = partition.checked_sub(1).map(|i| &line.sides[i]);
            next(line.sides.get(partition).map(|p| p.0));
            let (province, state) = at(&line.place, hour).copied().unwrap_or_default();
            next(line.place.get(line.place.partition_point(|(h, _)| *h <= hour)).map(|p| p.0));
            let divisions = at(&line.divisions, hour).copied().unwrap_or_default();
            next(line.divisions.get(line.divisions.partition_point(|(h, _)| *h <= hour)).map(|p| p.0));
            let losses = at(&line.losses, hour).copied().unwrap_or_default();
            next(line.losses.get(line.losses.partition_point(|(h, _)| *h <= hour)).map(|p| p.0));
            let result = match shown.state {
                Some(s) if s == LandBattleStatus::Ongoing as u8 => LandBattleStatus::Ongoing,
                Some(s) if s == LandBattleStatus::AttackerWon as u8 => LandBattleStatus::AttackerWon,
                Some(s) if s == LandBattleStatus::DefenderWon as u8 => LandBattleStatus::DefenderWon,
                _ => LandBattleStatus::Unknown,
            };
            LandBattleLogRow {
                key: match line.written {
                    Some(written) => format!("{}:{}:{}:{written}", prepared.branch, (entry.id >> 64) as u64, entry.date),
                    None => format!("{}:{}:{}", prepared.branch, (entry.id >> 64) as u64, entry.date),
                },
                hour: entry.date,
                exact: line.exact,
                province,
                state,
                divisions,
                casualties: losses.manpower,
                ic_lost: losses.ic_raw.map(|v| v.map(|v| v as f64 / 100_000.0)),
                losses_so_far: losses.so_far,
                countries: sides.map_or_else(Default::default, |s| s.1.countries.clone()),
                countries_unknown: sides.map_or([true; 2], |s| s.1.unknown),
                countries_hour: sides.map_or([entry.observed; 2], |(h, s)| s.read.map(|at| at.unwrap_or(*h))),
                result,
            }
        })
        .collect();
    let mut boundary = answer.next_change;
    if ask.live_first {
        while let Some(candidate) = boundary.filter(|at| visible_change.is_none_or(|visible| *at < visible)) {
            ask.hour = candidate;
            let future = prepared.index.ask(holders, &ask)?;
            if future.total != answer.total || future.rows != answer.rows || future.gaps != answer.gaps || future.unattributed != answer.unattributed {
                break;
            }
            boundary = future.next_change;
        }
    }
    let next_change = boundary.into_iter().chain(visible_change).min();
    let warnings = (answer.unattributed > 0)
        .then(|| format!("Battles without a recorded country for this filter: {}", answer.unattributed))
        .into_iter()
        .collect();
    let reply = LandBattlesTailReply {
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

pub struct LandBattlesTail;

impl Product for LandBattlesTail {
    const KEY: &'static str = "land-battles-tail";
    const CONTENT_DEPENDENT: bool = false;
    const INTERACTIVE: bool = true;
    type Params = LandBattlesTailParams;
    const FIELDS: dashboard_base::products::Fields =
        dashboard_base::fields![branchId, hour: Read, from: Start, to: End, countries, "countries[]", group, offset, sort, limit];
    type Reply = LandBattlesTailReply;
    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            LandBattlesTailParams::decl(),
            LandBattlesTailReply::decl(),
            LandBattleLogRow::decl(),
            LandBattleStatus::decl(),
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
    request.as_object_mut().ok_or("Invalid land-battles-tail panel")?.remove("panel");
    let params: LandBattlesTailParams =
        dashboard_query::named::named(&request).map_err(|error| format!("Invalid land-battles-tail panel: {error}"))?;
    LandBattlesTail::validate(&params)?;
    let branch = params.branch_id.parse().map_err(|_| "Invalid branch")?;
    let query = dashboard_base::queries::query(source, branch).map_err(|error| error.to_string())?;
    let (finish, prepared) = dashboard_base::products::Finish::open(source, load, LandBattlesTail::CONTENT_DEPENDENT, || prepare(source, &query, load, &params, &AtomicBool::new(false)));
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
