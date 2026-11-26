use super::{ProjectKind, RunLines, SpendingGap, col, gap_of, hour, id32, number};
use dashboard_base::{Source, queries::country_ids::RunLinks};
use dashboard_query::identity::Holders;
use replay_data::{
    codec::Decoder,
    rows::{ConstructionProjectRow as P, ConstructionRise, TableId},
    value::Value,
};
use replay_lanes::lanes::{Cell, Keys, Step};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io};

const KEYS_PER_READ: usize = 8192;
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum EndKind {
    Finished,
    Removed,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum UnknownEnd {
    NotRecorded,
    SharedTarget,
    NothingLeft,
    CompletionNotRecorded,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProjectEnd {
    pub kind: EndKind,
    pub hour: Option<i32>,
    pub known_at: i32,
    pub unknown: Option<UnknownEnd>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LevelsBuilt {
    pub hour: Option<i32>,
    pub known_at: i32,
    pub count: u32,
    pub repair: bool,
    pub carried: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProjectRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub factories: i32,
    pub remaining: i32,
    pub repair_levels: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConstructionOutputRun {
    pub from: i32,
    pub to: i32,
    pub construction_raw: Option<i64>,
    pub repair_raw: Option<i64>,
    pub reason: Option<SpendingGap>,
    pub before_observation: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OutputOrigin {
    pub tag: String,
    pub country_slot: Option<u32>,
    pub building: Option<u32>,
    pub state: Option<u32>,
    pub to_state: Option<u32>,
    pub province: Option<u32>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ProjectSource {
    Census,
    OutputAlone,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConstructionProject {
    pub id: String,
    pub kind: ProjectKind,
    pub source: ProjectSource,
    pub census_read: Option<i32>,
    pub output_origin: Option<OutputOrigin>,
    pub queue_ordinal: Option<u32>,
    pub tag: String,
    pub country_slot: Option<u32>,
    pub building: Option<u32>,
    pub repair: Option<bool>,
    pub state: Option<u32>,
    pub to_state: Option<u32>,
    pub province: Option<u32>,
    pub route_provinces: Option<u32>,
    pub base_level: Option<i32>,
    pub target_level: Option<i32>,
    pub cost_raw: Option<i64>,
    pub created_hour: Option<i32>,
    pub first_read: i32,
    pub earlier_unknown: bool,
    pub runs: Vec<ProjectRun>,
    pub built: Vec<LevelsBuilt>,
    pub built_unread: Option<Vec<(i32, i32)>>,
    pub repaired_unread: Option<Vec<(i32, i32)>>,
    pub unaccounted: Vec<LevelsBuilt>,
    pub possible: Vec<LevelsBuilt>,
    pub added: Vec<LevelsBuilt>,
    pub end: Option<ProjectEnd>,
    pub first_repair_tick: Option<i32>,
    pub outputs: Vec<ConstructionOutputRun>,
    pub counters_read: u8,
    pub opening: [Option<(i32, u32)>; 2],
}

const COUNTERS: [usize; 7] = [col::BUILT, col::REPAIRED, col::UNACCOUNTED_NEW, col::UNACCOUNTED_REPAIR, col::POSSIBLE, col::ADDED_NEW, col::ADDED_REPAIR];

fn join_runs<T>(
    mut a: Vec<T>,
    b: Vec<T>,
    first: impl Fn(&T) -> i32,
    last: impl Fn(&T) -> i32,
    same: impl Fn(&T, &T) -> bool,
    extend: impl Fn(&mut T, &T),
) -> Vec<T> {
    for item in b {
        match a.last_mut() {
            Some(held) if i64::from(last(held)) + 1 == i64::from(first(&item)) && same(held, &item) => extend(held, &item),
            Some(held) if last(held) < first(&item) => a.push(item),
            None => a.push(item),
            _ => {}
        }
    }
    a
}

fn join_levels(mut x: Vec<LevelsBuilt>, y: Vec<LevelsBuilt>, read: [bool; 2], opening: [Option<(i32, u32)>; 2]) -> Vec<LevelsBuilt> {
    let mut prior = [0u32; 2];
    for levels in &x {
        prior[usize::from(levels.repair)] = prior[usize::from(levels.repair)].saturating_add(levels.count);
    }
    let mut reconciled = [false; 2];
    for (at, mut levels) in y.iter().cloned().enumerate() {
        let side = usize::from(levels.repair);
        if let (true, Some((hour, total))) = (read[side], opening[side])
            && levels.known_at == hour
        {
            if !reconciled[side] {
                reconciled[side] = true;
                let delta = total.saturating_sub(prior[side]);
                let observed: Vec<&LevelsBuilt> = y[at..].iter().filter(|held| usize::from(held.repair) == side && held.known_at == hour).collect();
                let dated: u32 = observed.iter().filter(|held| !held.carried).map(|held| held.count).sum();
                prior[side] = prior[side].saturating_add(delta);
                if delta > 0 && dated == delta {
                    x.extend(observed.into_iter().filter(|held| !held.carried).cloned());
                } else if delta > 0 {
                    x.push(LevelsBuilt { hour: None, known_at: hour, count: delta, repair: side == 1, carried: false });
                }
            }
            continue;
        }
        if !levels.carried {
            prior[side] = prior[side].saturating_add(levels.count);
            x.push(levels);
            continue;
        }
        let delta = levels.count.saturating_sub(prior[side]);
        if delta > 0 {
            prior[side] = prior[side].saturating_add(delta);
            if read[side] {
                levels.count = delta;
                levels.carried = false;
            }
            x.push(levels);
        }
    }
    x
}

fn join_unread(a: Option<Vec<(i32, i32)>>, b: Option<Vec<(i32, i32)>>) -> Option<Vec<(i32, i32)>> {
    let merged = join_runs(a.unwrap_or_default(), b.unwrap_or_default(), |s| s.0, |s| s.1, |_, _| true, |x, y| x.1 = y.1);
    (!merged.is_empty()).then_some(merged)
}

pub(crate) fn join_project(a: ConstructionProject, b: ConstructionProject) -> ConstructionProject {
    let read = |new: usize, repair: usize| [a.counters_read & (1 << new) != 0, a.counters_read & (1 << repair) != 0];
    let read = [read(0, 1), read(2, 3), read(4, 4), read(5, 6)];
    let output_origin = a.output_origin.or_else(|| {
        if a.source == ProjectSource::OutputAlone && b.source == ProjectSource::Census {
            Some(OutputOrigin { tag: a.tag, country_slot: a.country_slot, building: a.building, state: a.state, to_state: a.to_state, province: a.province })
        } else {
            b.output_origin
        }
    });
    let runs = join_runs(
        a.runs,
        b.runs,
        |r| r.first_hour,
        |r| r.last_hour,
        |x, y| x.factories == y.factories && x.remaining == y.remaining && x.repair_levels == y.repair_levels,
        |x, y| x.last_hour = y.last_hour,
    );
    let outputs = join_runs(
        a.outputs,
        b.outputs,
        |r| r.from,
        |r| r.to,
        |x, y| x.construction_raw == y.construction_raw && x.repair_raw == y.repair_raw && x.reason == y.reason && x.before_observation == y.before_observation,
        |x, y| x.to = y.to,
    );
    ConstructionProject {
        kind: if b.kind == ProjectKind::Unknown { a.kind } else { b.kind },
        source: if b.source == ProjectSource::Unknown { a.source } else { b.source },
        output_origin,
        country_slot: b.country_slot.or(a.country_slot),
        repair: a.repair.or(b.repair),
        built_unread: join_unread(a.built_unread, b.built_unread),
        repaired_unread: join_unread(a.repaired_unread, b.repaired_unread),
        runs,
        outputs,
        built: join_levels(a.built, b.built, read[0], b.opening),
        unaccounted: join_levels(a.unaccounted, b.unaccounted, read[1], [None; 2]),
        possible: join_levels(a.possible, b.possible, read[2], [None; 2]),
        added: join_levels(a.added, b.added, read[3], [None; 2]),
        opening: a.opening,
        counters_read: a.counters_read | b.counters_read,
        end: b.end.or(a.end),
        ..b
    }
}

pub(crate) fn window_projects(
    source: &Source,
    run: u32,
    from: i32,
    to: i32,
    lines: &RunLines,
    holders: &Holders,
    links: &RunLinks,
) -> io::Result<Vec<ConstructionProject>> {
    let recording = source.replay_ref().recording();
    let mut keys: Vec<u128> = lines.within(from, to).map(|line| u128::from(line.id)).collect();
    if keys.is_empty() {
        return Ok(Vec::new());
    }
    keys.sort_unstable();
    let statuses = dashboard_query::cost::storage(|| replay_lanes::lanes::statuses(recording, run, TableId::ConstructionProjects, from, to))?;
    let truncated: Vec<(i32, i32)> = statuses
        .into_iter()
        .filter(|span| matches!(span.status, replay_columnar::manifest::Status::Truncated { .. }))
        .map(|span| (span.first, span.last))
        .collect();
    let resolve = |country: u32, at: i32| {
        let slot = links.slot_at(country, at);
        let tag = slot.and_then(|s| holders.slot_tag(s, at)).map(str::to_owned);
        (slot, tag)
    };
    let mut out = Vec::with_capacity(keys.len());
    for chunk in keys.chunks(KEYS_PER_READ) {
        let history = dashboard_query::cost::storage(|| replay_lanes::lanes::history(recording, run, TableId::ConstructionProjects, &col::ALL, Keys::These(chunk), from, to))?;
        let mut asks = Vec::new();
        for (key, steps) in &history.keys {
            asks.extend(rise_hours(steps).into_iter().map(|hour| (hour, *key)));
        }
        let mut rises: HashMap<(u128, i32), Rises> = HashMap::new();
        if !asks.is_empty() {
            dashboard_query::cost::storage(|| {
                replay_lanes::lanes::at_each(recording, run, TableId::ConstructionProjects, Some(&[col::BUILT_RISES, col::REPAIRED_RISES][..]), &asks, &mut |hour, key, row| {
                    if let Some((view, slot)) = row {
                        let list = |column: usize| -> io::Result<Vec<ConstructionRise>> {
                            let Some(bytes) = view.list(column, slot) else { return Ok(Vec::new()) };
                            let mut input = Decoder::new(bytes);
                            let rises = <Vec<ConstructionRise> as Value>::decode(&mut input)?;
                            input.finish()?;
                            Ok(rises)
                        };
                        rises.insert((key, hour), Rises { built: list(col::BUILT_RISES)?, repaired: list(col::REPAIRED_RISES)? });
                    }
                    Ok(())
                })
            })?;
        }
        for (key, steps) in &history.keys {
            if let Some(project) = assemble_inner(*key as u64, steps, history.to, &truncated, &|hour| rises.get(&(*key, hour)), &resolve, false) {
                out.push(*project);
            }
        }
    }
    out.sort_by(|a, b| (a.first_read, a.kind, a.id.len(), &a.id).cmp(&(b.first_read, b.kind, b.id.len(), &b.id)));
    Ok(out)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Rises {
    pub built: Vec<ConstructionRise>,
    pub repaired: Vec<ConstructionRise>,
}

fn shown<'a>(steps: &'a [Step], to: i32) -> Vec<(i32, i32, &'a [Cell])> {
    steps
        .iter()
        .enumerate()
        .filter_map(|(at, step)| {
            let cells = step.cells.as_deref()?;
            let last = steps.get(at + 1).map_or(to, |next| next.hour - 1);
            Some((step.hour, last, cells))
        })
        .collect()
}

pub(crate) fn rise_hours(steps: &[Step]) -> Vec<i32> {
    let mut out = Vec::new();
    let mut held: (Option<i64>, Option<i64>) = (None, None);
    let continued = steps.iter().find_map(|step| step.cells.as_deref().map(|cells| hour(cells, col::FIRST_READ).is_some_and(|first| first < step.hour))).unwrap_or(false);
    let mut saw_unread = (continued, continued);
    for step in steps {
        let Some(cells) = step.cells.as_deref() else { continue };
        let built = number(cells, col::BUILT);
        let repaired = number(cells, col::REPAIRED);
        let rose = matches!((held.0, built), (Some(before), Some(now)) if now > before)
            || matches!((held.1, repaired), (Some(before), Some(now)) if now > before)
            || (held.0.is_none() && saw_unread.0 && built.is_some_and(|now| now > 0))
            || (held.1.is_none() && saw_unread.1 && repaired.is_some_and(|now| now > 0));
        if rose {
            out.push(step.hour);
        }
        match built {
            Some(now) => held.0 = Some(now),
            None => saw_unread.0 = true,
        }
        match repaired {
            Some(now) => held.1 = Some(now),
            None => saw_unread.1 = true,
        }
    }
    out
}

fn in_runs(runs: &[(i32, i32)], hour: i32) -> bool {
    runs.get(runs.partition_point(|&(_, last)| last < hour)).is_some_and(|&(first, _)| first <= hour)
}

fn without_spans(spans: Vec<(i32, i32)>, excluded: &[(i32, i32)]) -> Vec<(i32, i32)> {
    if excluded.is_empty() {
        return spans;
    }
    let mut out = Vec::with_capacity(spans.len());
    for (mut start, end) in spans {
        let mut done = false;
        for &(ex_first, ex_last) in excluded {
            if ex_last < start {
                continue;
            }
            if ex_first > end {
                break;
            }
            if ex_first > start {
                out.push((start, ex_first - 1));
            }
            if ex_last >= end {
                done = true;
                break;
            }
            start = ex_last + 1;
        }
        if !done && start <= end {
            out.push((start, end));
        }
    }
    out
}

#[cfg(test)]
pub(crate) enum Assembled {
    Project(Box<ConstructionProject>),
    Unnamed,
    Hidden,
}

fn end_of(cells: &[Cell], at: i32) -> Option<ProjectEnd> {
    let kind = match u8::try_from(number(cells, col::END)?).ok()? {
        P::FINISHED => EndKind::Finished,
        P::REMOVED => EndKind::Removed,
        P::UNKNOWN_END => EndKind::Unknown,
        _ => return None,
    };
    let unknown = match number(cells, col::END_REASON).and_then(|reason| u8::try_from(reason).ok()) {
        Some(P::NOT_RECORDED) => Some(UnknownEnd::NotRecorded),
        Some(P::SHARED_TARGET) => Some(UnknownEnd::SharedTarget),
        Some(P::NOTHING_LEFT) => Some(UnknownEnd::NothingLeft),
        Some(P::COMPLETION_NOT_RECORDED) => Some(UnknownEnd::CompletionNotRecorded),
        _ => None,
    };
    Some(ProjectEnd { kind, hour: hour(cells, col::END_HOUR), known_at: hour(cells, col::END_KNOWN_AT).unwrap_or(at), unknown })
}

fn rises_of(shown: &[(i32, i32, &[Cell])], column: usize, repair: bool, dated: &dyn Fn(i32) -> Option<Vec<ConstructionRise>>, continued: bool) -> Vec<LevelsBuilt> {
    let mut out = Vec::new();
    let mut held: Option<i64> = None;
    let mut saw_unread = continued;
    for &(at, _, cells) in shown {
        let Some(now) = number(cells, column) else {
            saw_unread = true;
            continue;
        };
        match held {
            None if now > 0 => {
                let total = u32::try_from(now).unwrap_or(u32::MAX);
                let list = if saw_unread { dated(at).unwrap_or_default() } else { Vec::new() };
                let list_sum: u64 = list.iter().map(|rise| u64::from(rise.count)).sum();
                if !list.is_empty() && list_sum <= u64::from(total) {
                    let carried = total - list_sum as u32;
                    if carried > 0 {
                        out.push(LevelsBuilt { hour: None, known_at: at, count: carried, repair, carried: true });
                    }
                    out.extend(list.iter().map(|rise| LevelsBuilt { hour: Some(rise.hour), known_at: at, count: rise.count, repair, carried: false }));
                } else {
                    out.push(LevelsBuilt { hour: None, known_at: at, count: total, repair, carried: true });
                }
            }
            Some(before) if now > before => {
                let delta = u32::try_from(now - before).unwrap_or(u32::MAX);
                let list = dated(at).unwrap_or_default();
                if !list.is_empty() && list.iter().map(|rise| u64::from(rise.count)).sum::<u64>() == u64::from(delta) {
                    out.extend(list.iter().map(|rise| LevelsBuilt { hour: Some(rise.hour), known_at: at, count: rise.count, repair, carried: false }));
                } else {
                    out.push(LevelsBuilt { hour: None, known_at: at, count: delta, repair, carried: false });
                }
            }
            _ => {}
        }
        held = Some(held.map_or(now, |before| before.max(now)));
    }
    out
}

fn merged(mut a: Vec<LevelsBuilt>, b: Vec<LevelsBuilt>) -> Vec<LevelsBuilt> {
    a.extend(b);
    a.sort_by_key(|levels| (levels.known_at, !levels.carried, levels.repair, levels.hour));
    a
}

fn assemble_inner<'r>(
    id: u64,
    steps: &[Step],
    to: i32,
    truncated: &[(i32, i32)],
    rises: &dyn Fn(i32) -> Option<&'r Rises>,
    resolve_owner: &dyn Fn(u32, i32) -> (Option<u32>, Option<String>),
    require_tag: bool,
) -> Option<Box<ConstructionProject>> {
    let shown = shown(steps, to);
    let (Some(&(_, _, first)), Some(&(last_at, _, last))) = (shown.first(), shown.last()) else { return None };
    let first_read = hour(last, col::FIRST_READ).unwrap_or(shown[0].0);
    let (country_slot, owner_tag) = match id32(last, col::COUNTRY) {
        Some(country) => resolve_owner(country, first_read),
        None => (None, None),
    };
    if require_tag && owner_tag.is_none() {
        return None;
    }
    let owner = owner_tag.unwrap_or_default();
    let output_alone = |cells: &[Cell]| match number(cells, col::OUTPUT_ALONE) {
        Some(0) => Some(false),
        Some(1) => Some(true),
        _ => None,
    };
    let source = match shown.iter().rev().find_map(|(_, _, cells)| output_alone(cells)) {
        Some(false) => ProjectSource::Census,
        Some(true) => ProjectSource::OutputAlone,
        None => ProjectSource::Unknown,
    };
    let output_origin = (output_alone(first) == Some(true) && source == ProjectSource::Census).then(|| {
        let (origin_slot, origin_tag) = id32(first, col::COUNTRY).map_or((country_slot, None), |country| resolve_owner(country, first_read));
        OutputOrigin {
            tag: origin_tag.unwrap_or_else(|| owner.clone()),
            country_slot: origin_slot.or(country_slot),
            building: id32(first, col::BUILDING_TYPE),
            state: id32(first, col::TARGET_STATE),
            to_state: id32(first, col::TO_STATE),
            province: id32(first, col::PROVINCE),
        }
    });
    let kind = shown
        .iter()
        .rev()
        .find_map(|(_, _, cells)| match number(cells, col::KIND).and_then(|value| u8::try_from(value).ok()) {
            Some(P::BUILDING) => Some(ProjectKind::Building),
            Some(P::RAILWAY) => Some(ProjectKind::Railway),
            _ => None,
        })
        .unwrap_or(ProjectKind::Unknown);
    let open = |cells: &[Cell]| number(cells, col::END).is_none_or(|end| end == i64::from(P::OPEN));
    let mut runs: Vec<ProjectRun> = Vec::new();
    for &(at, through, cells) in &shown {
        if output_alone(cells) == Some(true) || !open(cells) {
            continue;
        }
        let (Some(factories), Some(remaining), Some(repair_levels)) = (hour(cells, col::FACTORIES), hour(cells, col::REMAINING), hour(cells, col::REPAIR_LEVELS)) else { continue };
        match runs.last_mut() {
            Some(run) if run.last_hour + 1 == at && (run.factories, run.remaining, run.repair_levels) == (factories, remaining, repair_levels) => run.last_hour = through,
            _ => runs.push(ProjectRun { first_hour: at, last_hour: through, factories, remaining, repair_levels }),
        }
    }
    let mut outputs: Vec<ConstructionOutputRun> = Vec::new();
    for &(at, through, cells) in &shown {
        let since = hour(cells, col::OUTPUT_SINCE);
        let reason = number(cells, col::OUTPUT_REASON).and_then(|reason| u8::try_from(reason).ok()).and_then(gap_of);
        if since.is_none() && reason.is_some_and(|reason| reason == SpendingGap::NotRecorded) {
            continue;
        }
        let raw = |column: usize| if reason.is_none() { number(cells, column) } else { None };
        let run = ConstructionOutputRun {
            from: at,
            to: through,
            construction_raw: raw(col::CONSTRUCTION_RAW),
            repair_raw: raw(col::REPAIR_RAW),
            reason,
            before_observation: since.is_some() && number(cells, col::OBSERVED_SINCE_CREATION) == Some(0),
        };
        match outputs.last_mut() {
            Some(held)
                if held.to + 1 == at
                    && (&held.construction_raw, &held.repair_raw, held.reason, held.before_observation)
                        == (&run.construction_raw, &run.repair_raw, run.reason, run.before_observation) =>
            {
                held.to = through
            }
            _ => outputs.push(run),
        }
    }
    let continued = first_read < shown[0].0;
    let built = |at: i32| rises(at).map(|rises| rises.built.clone());
    let repaired = |at: i32| rises(at).map(|rises| rises.repaired.clone());
    let undated = |_: i32| -> Option<Vec<ConstructionRise>> { None };
    let census_first = shown.iter().find(|(_, _, cells)| output_alone(cells) == Some(false)).map(|(_, _, cells)| *cells);
    let counter_unread = |column: usize| -> Option<Vec<(i32, i32)>> {
        let mut spans: Vec<(i32, i32)> = Vec::new();
        let mut uncertain_since: Option<i32> = None;
        for step in steps {
            let Some(cells) = step.cells.as_deref() else {
                if in_runs(truncated, step.hour)
                    && let Some(start) = uncertain_since.take()
                {
                    spans.push((start, step.hour - 1));
                }
                continue;
            };
            if output_alone(cells) == Some(true) {
                continue;
            }
            if number(cells, column).is_some() {
                if let Some(start) = uncertain_since.take() {
                    spans.push((start, step.hour - 1));
                }
            } else {
                uncertain_since.get_or_insert(step.hour);
            }
        }
        if let Some(start) = uncertain_since {
            spans.push((start, to));
        }
        let spans = without_spans(spans, truncated);
        (!spans.is_empty()).then_some(spans)
    };
    Some(Box::new(ConstructionProject {
        id: id.to_string(),
        kind,
        source,
        census_read: shown.iter().rev().find_map(|(_, _, cells)| hour(cells, col::CENSUS_READ)),
        output_origin,
        queue_ordinal: id32(last, col::QUEUE_ORDINAL),
        tag: owner,
        country_slot,
        building: id32(last, col::BUILDING_TYPE),
        repair: census_first.and_then(|cells| number(cells, col::REPAIR_LEVELS).map(|levels| levels > 0)),
        state: id32(last, col::TARGET_STATE),
        to_state: id32(last, col::TO_STATE),
        province: id32(last, col::PROVINCE),
        route_provinces: id32(last, col::ROUTE_PROVINCES),
        base_level: hour(last, col::BASE_LEVEL),
        target_level: hour(last, col::TARGET_LEVEL),
        cost_raw: number(last, col::COST),
        created_hour: hour(last, col::CREATED_HOUR),
        first_read,
        earlier_unknown: number(last, col::EARLIER_UNKNOWN) == Some(1),
        runs,
        built: merged(rises_of(&shown, col::BUILT, false, &built, continued), rises_of(&shown, col::REPAIRED, true, &repaired, continued)),
        built_unread: counter_unread(col::BUILT),
        repaired_unread: counter_unread(col::REPAIRED),
        unaccounted: merged(rises_of(&shown, col::UNACCOUNTED_NEW, false, &undated, false), rises_of(&shown, col::UNACCOUNTED_REPAIR, true, &undated, false)),
        possible: rises_of(&shown, col::POSSIBLE, false, &undated, false),
        added: merged(rises_of(&shown, col::ADDED_NEW, false, &undated, false), rises_of(&shown, col::ADDED_REPAIR, true, &undated, false)),
        end: end_of(last, last_at),
        first_repair_tick: hour(last, col::FIRST_REPAIR_TICK),
        outputs,
        opening: [col::BUILT, col::REPAIRED].map(|column| {
            shown.iter().find_map(|&(at, _, cells)| number(cells, column).map(|count| (at, u32::try_from(count).unwrap_or(u32::MAX))))
        }),
        counters_read: COUNTERS
            .iter()
            .enumerate()
            .filter(|&(_, &column)| shown.iter().any(|&(_, _, cells)| number(cells, column).is_some()))
            .fold(0, |bits, (at, _)| bits | 1 << at),
    }))
}

#[cfg(test)]
pub(crate) fn assemble<'r>(
    id: u64,
    steps: &[Step],
    to: i32,
    truncated: &[(i32, i32)],
    rises: &dyn Fn(i32) -> Option<&'r Rises>,
    tag: &dyn Fn(u32, i32) -> Option<String>,
) -> Assembled {
    if shown(steps, to).is_empty() {
        return Assembled::Hidden;
    }
    let resolve = |country: u32, at: i32| (Some(country), tag(country, at));
    match assemble_inner(id, steps, to, truncated, rises, &resolve, true) {
        Some(project) => Assembled::Project(project),
        None => Assembled::Unnamed,
    }
}
