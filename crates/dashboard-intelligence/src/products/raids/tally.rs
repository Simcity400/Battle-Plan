use super::{
    Content, Identity, RAID_COLUMNS, RAID_TYPE, RaidTallyMetric, TARGET_COLUMNS, TargetIdentity,
    cell_at, coalesce_spans, has_hour_in_range, lane_value, metric_unattributed, missing, pieces, scope_of, steps_of,
    successful, validate,
};
use dashboard_base::{
    Source,
    products::{Country, LoadContext, Product},
};
use dashboard_query::{Query, series::MissingSpan};
use engine_native_air::craid_instance as native;
use replay_data::rows::TableId;
use replay_lanes::lanes::{self, Keys};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

fn default_metric() -> RaidTallyMetric {
    RaidTallyMetric::Both
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RaidTallyParams {
    pub branch_id: String,
    #[serde(default)]
    pub countries: Vec<Country>,
    #[serde(default)]
    pub top: Option<u32>,
    #[serde(default = "default_metric")]
    pub metric: RaidTallyMetric,
    pub hour: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RaidTallySpan {
    pub first_hour: i32,
    pub last_hour: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RaidTallySlotRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub launched: u32,
    pub succeeded: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RaidTallySlotSeries {
    pub slot: u32,
    pub runs: Vec<RaidTallySlotRun>,
    pub membership: Option<Vec<RaidTallySpan>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RaidTallyReply {
    pub hour: i32,
    pub next_change: Option<i32>,
    pub available: Option<bool>,
    pub unavailable: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub rank_missing: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub rank_missing_from: Option<i32>,
    pub unattributed: u32,
    pub series: Vec<RaidTallySlotSeries>,
    pub missing: Vec<MissingSpan>,
    #[serde(default)]
    pub partial: Vec<MissingSpan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub partial_launched: Option<Vec<MissingSpan>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub partial_succeeded: Option<Vec<MissingSpan>>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(RaidTallyReply);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EventKind {
    Launched,
    Succeeded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TallyEvent {
    hour: i32,
    slot: u32,
    kind: EventKind,
}

struct BranchTallyIndex {
    branch_first: i32,
    branch_last: i32,
    first_raid_hour: Option<i32>,
    events: Vec<TallyEvent>,
    unattributed_launched: Vec<i32>,
    unattributed_succeeded: Vec<i32>,
    unattributed_both: Vec<i32>,
    missing: Vec<MissingSpan>,
}

#[derive(Default)]
struct TallyCache(HashMap<String, Arc<BranchTallyIndex>>);

struct TrackedEpisode {
    last_hour: i32,
    last_stretch: usize,
    identity: Identity,
    phase: Option<i32>,
    launched_hour: Option<i32>,
    succeeded_hour: Option<i32>,
}

fn build_branch_index(source: &Source, query: &Query<'_>) -> io::Result<BranchTallyIndex> {
    let recording = query.replay.recording();
    let branch_first = query.timeline.first;
    let branch_last = query.timeline.last;
    let missing_spans = missing(query, branch_first, branch_last)?;

    let mut tracked: Vec<TrackedEpisode> = Vec::new();
    let mut active: HashMap<u128, usize> = HashMap::new();
    let mut complete_all: Vec<(i32, i32)> = Vec::new();
    let mut first_raid_hour: Option<i32> = None;

    for (stretch_idx, stretch) in query.timeline.within(branch_first, branch_last).enumerate() {
        let run = stretch.run;
        let end = stretch.last.min(branch_last);
        let stretch_complete = coalesce_spans(dashboard_query::cost::storage(|| {
            lanes::complete_reads(recording, run, TableId::Raids, stretch.first, end)
        })?);
        complete_all.extend_from_slice(&stretch_complete);

        let lives = dashboard_base::queries::lifetimes(source, run, TableId::Raids)?;
        let stretch_first = query.timeline.at(stretch.first).map_or(stretch.first, |held| held.first);
        let met: Vec<(u128, i32, i32)> = lives
            .iter()
            .filter_map(|&(key, raw_first, raw_last)| {
                let raw_first = i32::try_from(raw_first).ok()?.max(stretch_first);
                let last = i32::try_from(raw_last).ok()?.min(end);
                (raw_first <= last).then_some((key, raw_first, last))
            })
            .collect();
        if met.is_empty() {
            continue;
        }
        for &(_, raw_first, _) in &met {
            first_raid_hour = Some(first_raid_hour.map_or(raw_first, |prev| prev.min(raw_first)));
        }

        let keys: Vec<u128> = met.iter().map(|(key, ..)| *key).collect::<BTreeSet<_>>().into_iter().collect();
        let start = met.iter().map(|(_, raw_first, _)| *raw_first).min().unwrap_or(stretch.first);
        let raids = dashboard_query::cost::storage(|| {
            lanes::history(recording, run, TableId::Raids, &RAID_COLUMNS, Keys::These(&keys), start, end)
        })?;
        let targets = dashboard_query::cost::storage(|| {
            lanes::history(recording, run, TableId::RaidTargets, &TARGET_COLUMNS, Keys::These(&keys), start, end)
        })?;
        let mut tokens: BTreeSet<u128> = BTreeSet::new();
        for &key in &keys {
            for cells in steps_of(&targets, key).into_iter().flatten().filter_map(|step| step.cells.as_deref()) {
                if let Some(token) = lane_value(cells.get(RAID_TYPE).copied()).flatten().filter(|&t| t != 0) {
                    tokens.insert(token as u128);
                }
            }
        }
        let lexical = if tokens.is_empty() {
            lanes::History::default()
        } else {
            let ids: Vec<u128> = tokens.into_iter().collect();
            dashboard_query::cost::storage(|| {
                lanes::history(recording, run, TableId::LexicalTokens, &[0], Keys::These(&ids), start, end)
            })?
        };
        let spelled: Vec<i32> = lexical.keys.iter().flat_map(|(_, steps)| steps.iter().map(|step| step.hour)).collect();
        let literal_at = |token: u32, hour: i32| -> Option<String> {
            let cell = cell_at(&lexical, steps_of(&lexical, u128::from(token)), 0, hour)?;
            let literal = String::from_utf8_lossy(lexical.text(cell)?).into_owned();
            (!literal.is_empty()).then_some(literal)
        };
        let target_at = |key: u128, at: i32| -> (Option<TargetIdentity>, Option<u32>) {
            let steps = steps_of(&targets, key);
            let scope = scope_of(|column| lane_value(cell_at(&targets, steps, column, at)));
            let token = lane_value(cell_at(&targets, steps, RAID_TYPE, at)).flatten().filter(|&t| t != 0).map(|t| t as u32);
            (scope.as_ref().map(TargetIdentity::of), token)
        };
        let absent = |after: i32, before: i32| {
            has_hour_in_range(&complete_all, after.saturating_add(1), before.saturating_sub(1))
        };

        for &(key, _, last) in &met {
            let target_steps = steps_of(&targets, key);
            let mut splits: Vec<i32> = target_steps
                .into_iter()
                .flatten()
                .map(|step| step.hour)
                .chain(spelled.iter().copied())
                .collect();
            splits.sort_unstable();
            splits.dedup();
            let seen = pieces(steps_of(&raids, key).map_or(&[][..], Vec::as_slice), last, &splits);
            for piece in &seen {
                if let Some(&idx) = active.get(&key)
                    && absent(tracked[idx].last_hour, piece.first)
                {
                    active.remove(&key);
                }
                let Some(unit) = &piece.unit else {
                    continue;
                };
                let (target, token) = target_at(key, piece.first);
                let identity = Identity {
                    raider: unit.raider,
                    target,
                    kind: token.and_then(|token| literal_at(token, piece.first)),
                };
                let admitted = active.get(&key).copied().filter(|&idx| {
                    let acc = &tracked[idx];
                    let continuous = acc.last_stretch == stretch_idx && acc.last_hour + 1 == piece.first;
                    !acc.identity.contradicts(&identity)
                        && !acc.phase.is_some_and(|phase| unit.phase < phase)
                        && (continuous || acc.identity.matches(&identity))
                });
                let idx = admitted.unwrap_or_else(|| {
                    tracked.push(TrackedEpisode {
                        last_hour: piece.first,
                        last_stretch: stretch_idx,
                        identity: Identity::default(),
                        phase: None,
                        launched_hour: None,
                        succeeded_hour: None,
                    });
                    active.insert(key, tracked.len() - 1);
                    tracked.len() - 1
                });
                let acc = &mut tracked[idx];
                acc.last_hour = piece.last;
                acc.last_stretch = stretch_idx;
                acc.identity.fill(&identity);
                acc.phase = Some(unit.phase);
                if acc.launched_hour.is_none() && unit.launched {
                    acc.launched_hour = Some(piece.first);
                }
                if acc.succeeded_hour.is_none() && unit.phase == native::PHASE_ENDED && successful(unit.outcome) {
                    acc.succeeded_hour = Some(piece.first);
                }
            }
        }
    }

    let mut events = Vec::new();
    let mut unattributed_launched = Vec::new();
    let mut unattributed_succeeded = Vec::new();
    let mut unattributed_both = Vec::new();
    for ep in tracked {
        match ep.identity.raider {
            Some(slot) => {
                if let Some(hour) = ep.launched_hour {
                    events.push(TallyEvent { hour, slot, kind: EventKind::Launched });
                }
                if let Some(hour) = ep.succeeded_hour {
                    events.push(TallyEvent { hour, slot, kind: EventKind::Succeeded });
                }
            }
            None => {
                let mut first: Option<i32> = None;
                if let Some(hour) = ep.launched_hour {
                    unattributed_launched.push(hour);
                    first = Some(hour);
                }
                if let Some(hour) = ep.succeeded_hour {
                    unattributed_succeeded.push(hour);
                    first = Some(first.map_or(hour, |prev| prev.min(hour)));
                }
                if let Some(hour) = first {
                    unattributed_both.push(hour);
                }
            }
        }
    }
    events.sort_by_key(|ev| (ev.hour, ev.slot, matches!(ev.kind, EventKind::Succeeded)));
    unattributed_launched.sort_unstable();
    unattributed_succeeded.sort_unstable();
    unattributed_both.sort_unstable();

    Ok(BranchTallyIndex {
        branch_first,
        branch_last,
        first_raid_hour,
        events,
        unattributed_launched,
        unattributed_succeeded,
        unattributed_both,
        missing: missing_spans,
    })
}

fn cached_branch_index(source: &Source, branch_id: &str, query: &Query<'_>) -> io::Result<Arc<BranchTallyIndex>> {
    let state = source.state::<TallyCache>();
    {
        let cache = state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = cache.0.get(branch_id) {
            return Ok(Arc::clone(existing));
        }
    }
    let built = Arc::new(build_branch_index(source, query)?);
    let mut cache = state.lock().unwrap_or_else(|e| e.into_inner());
    let entry = cache.0.entry(branch_id.to_owned()).or_insert_with(|| Arc::clone(&built));
    Ok(Arc::clone(entry))
}

fn recorded_segments(first: i32, last: i32, missing: &[MissingSpan]) -> Vec<(i32, i32)> {
    if first > last {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut at = first;
    for gap in missing {
        if gap.last_hour < at || gap.first_hour > last {
            continue;
        }
        if gap.first_hour > at {
            out.push((at, gap.first_hour - 1));
        }
        if gap.last_hour >= last {
            return out;
        }
        at = gap.last_hour.saturating_add(1);
    }
    if at <= last {
        out.push((at, last));
    }
    out
}

fn metric_rank_key(metric: RaidTallyMetric, launched: u32, succeeded: u32) -> (u32, u32) {
    match metric {
        RaidTallyMetric::Succeeded => (succeeded, 0),
        RaidTallyMetric::Launched => (launched, 0),
        RaidTallyMetric::Both => (launched, succeeded),
    }
}

fn metric_nonzero(metric: RaidTallyMetric, launched: u32, succeeded: u32) -> bool {
    match metric {
        RaidTallyMetric::Both => launched > 0 || succeeded > 0,
        RaidTallyMetric::Launched => launched > 0,
        RaidTallyMetric::Succeeded => succeeded > 0,
    }
}

fn build_slot_runs(events: &[TallyEvent], slot: u32, segments: &[(i32, i32)]) -> Vec<RaidTallySlotRun> {
    let slot_events: Vec<TallyEvent> = events.iter().copied().filter(|ev| ev.slot == slot).collect();
    let mut runs = Vec::new();
    let (mut launched, mut succeeded, mut next) = (0u32, 0u32, 0usize);
    for &(seg_first, seg_last) in segments {
        let mut cur = seg_first;
        while next < slot_events.len() && slot_events[next].hour <= cur {
            match slot_events[next].kind {
                EventKind::Launched => launched += 1,
                EventKind::Succeeded => succeeded += 1,
            }
            next += 1;
        }
        while cur <= seg_last {
            let change = slot_events.get(next).map_or(i32::MAX, |ev| ev.hour);
            let run_last = seg_last.min(change.saturating_sub(1));
            runs.push(RaidTallySlotRun { first_hour: cur, last_hour: run_last, launched, succeeded });
            if run_last == seg_last {
                break;
            }
            cur = run_last.saturating_add(1);
            while next < slot_events.len() && slot_events[next].hour <= cur {
                match slot_events[next].kind {
                    EventKind::Launched => launched += 1,
                    EventKind::Succeeded => succeeded += 1,
                }
                next += 1;
            }
        }
    }
    runs
}

fn clip_membership_to_segments(spans: &[RaidTallySpan], segments: &[(i32, i32)]) -> Vec<RaidTallySpan> {
    let mut out: Vec<RaidTallySpan> = Vec::new();
    for span in spans {
        for &(seg_first, seg_last) in segments {
            let a = span.first_hour.max(seg_first);
            let b = span.last_hour.min(seg_last);
            if a <= b {
                match out.last_mut() {
                    Some(prev) if prev.last_hour.saturating_add(1) >= a => prev.last_hour = prev.last_hour.max(b),
                    _ => out.push(RaidTallySpan { first_hour: a, last_hour: b }),
                }
            }
        }
    }
    out
}

fn compute_top_membership(
    events: &[TallyEvent],
    segments: &[(i32, i32)],
    top: usize,
    metric: RaidTallyMetric,
) -> (Vec<u32>, BTreeMap<u32, Vec<RaidTallySpan>>) {
    if top == 0 || segments.is_empty() {
        return (Vec::new(), BTreeMap::new());
    }
    let last_recorded = segments.last().unwrap().1;
    let first_recorded = segments.first().unwrap().0;
    let mut counts: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
    let mut out: BTreeMap<u32, Vec<RaidTallySpan>> = BTreeMap::new();
    let mut members: Vec<u32> = Vec::new();
    let mut idx = 0usize;

    while idx < events.len() {
        let ev_hour = events[idx].hour;
        if ev_hour > last_recorded {
            break;
        }
        while idx < events.len() && events[idx].hour == ev_hour {
            let ev = events[idx];
            let entry = counts.entry(ev.slot).or_insert((0, 0));
            match ev.kind {
                EventKind::Launched => entry.0 += 1,
                EventKind::Succeeded => entry.1 += 1,
            }
            idx += 1;
        }
        let mut ranked: Vec<(u32, (u32, u32))> = counts
            .iter()
            .filter(|&(_, &(l, s))| metric_nonzero(metric, l, s))
            .map(|(&slot, &pair)| (slot, pair))
            .collect();
        ranked.sort_by(|a, b| {
            metric_rank_key(metric, b.1.0, b.1.1)
                .cmp(&metric_rank_key(metric, a.1.0, a.1.1))
                .then_with(|| a.0.cmp(&b.0))
        });
        let next: Vec<u32> = ranked.into_iter().take(top).map(|(slot, _)| slot).collect();
        let start = ev_hour.max(first_recorded);
        for &slot in &members {
            if !next.contains(&slot)
                && let Some(spans) = out.get_mut(&slot)
                && let Some(last) = spans.last_mut()
            {
                last.last_hour = start.saturating_sub(1);
            }
        }
        for &slot in &next {
            if !members.contains(&slot) {
                out.entry(slot).or_default().push(RaidTallySpan { first_hour: start, last_hour: last_recorded });
            }
        }
        members = next;
    }

    for spans in out.values_mut() {
        spans.retain(|s| s.first_hour <= s.last_hour);
        *spans = clip_membership_to_segments(spans, segments);
    }
    out.retain(|_, spans| !spans.is_empty());

    let mut ordered_slots: Vec<u32> = out.keys().copied().collect();
    ordered_slots.sort_by(|a, b| {
        let ca = counts.get(a).copied().unwrap_or((0, 0));
        let cb = counts.get(b).copied().unwrap_or((0, 0));
        metric_rank_key(metric, cb.0, cb.1)
            .cmp(&metric_rank_key(metric, ca.0, ca.1))
            .then_with(|| a.cmp(b))
    });
    (ordered_slots, out)
}

fn resolve_explicit_slots(query: &Query<'_>, countries: &[Country], hour: i32) -> (Vec<u32>, Option<i32>) {
    let mut slots = Vec::new();
    let mut seen = BTreeSet::new();
    let mut next_holder_change: Option<i32> = None;
    for country in countries {
        match country {
            Country::Slot(slot) => {
                if seen.insert(*slot) {
                    slots.push(*slot);
                }
            }
            Country::Tag(tag) => {
                for run in query.holders.runs(tag.as_str()) {
                    if run.first <= hour {
                        if let dashboard_query::identity::Holder::Found { slot, .. } = run.holder
                            && seen.insert(slot)
                        {
                            slots.push(slot);
                        }
                        if run.last < query.timeline.last && run.last >= hour {
                            let edge = run.last.saturating_add(1);
                            next_holder_change = Some(next_holder_change.map_or(edge, |prev| prev.min(edge)));
                        }
                    } else {
                        next_holder_change = Some(next_holder_change.map_or(run.first, |prev| prev.min(run.first)));
                    }
                }
            }
        }
    }
    (slots, next_holder_change)
}

fn compute_next_change(
    index: &BranchTallyIndex,
    hour: i32,
    top_mode: bool,
    explicit_slots: &[u32],
    next_holder_change: Option<i32>,
    metric: RaidTallyMetric,
) -> Option<i32> {
    if hour >= index.branch_last {
        return None;
    }
    let mut best: Option<i32> = None;
    let mut consider = |candidate: i32| {
        if candidate > hour && candidate <= index.branch_last {
            best = Some(best.map_or(candidate, |prev| prev.min(candidate)));
        }
    };
    if let Some(h) = next_holder_change {
        consider(h);
    }
    if let Some(first_raid) = index.first_raid_hour {
        consider(first_raid);
    }
    for span in &index.missing {
        consider(span.first_hour);
        if span.last_hour < index.branch_last {
            consider(span.last_hour.saturating_add(1));
        }
    }
    if matches!(metric, RaidTallyMetric::Launched | RaidTallyMetric::Both) {
        let at = index.unattributed_launched.partition_point(|&h| h <= hour);
        if let Some(&h) = index.unattributed_launched.get(at) {
            consider(h);
        }
    }
    if matches!(metric, RaidTallyMetric::Succeeded | RaidTallyMetric::Both) {
        let at = index.unattributed_succeeded.partition_point(|&h| h <= hour);
        if let Some(&h) = index.unattributed_succeeded.get(at) {
            consider(h);
        }
    }
    let ev_start = index.events.partition_point(|ev| ev.hour <= hour);
    if top_mode {
        if let Some(ev) = index.events.get(ev_start) {
            consider(ev.hour);
        }
    } else {
        let wanted: BTreeSet<u32> = explicit_slots.iter().copied().collect();
        for ev in &index.events[ev_start..] {
            if wanted.contains(&ev.slot) {
                consider(ev.hour);
                break;
            }
        }
    }
    best
}

fn partial_spans(segments: &[(i32, i32)], missing: &[MissingSpan], first_unattributed: Option<i32>) -> Vec<MissingSpan> {
    let first_gap = missing.first();
    if first_gap.is_none() && first_unattributed.is_none() {
        return Vec::new();
    }
    let gap_reason = first_gap.map(|gap| format!("Raid history is incomplete up to this date: {}", gap.reason));
    let mut out: Vec<MissingSpan> = Vec::new();
    for &(seg_first, seg_last) in segments {
        let mut cursor = seg_first;
        while cursor <= seg_last {
            let end = match first_unattributed {
                Some(u) if u > cursor && u <= seg_last => u - 1,
                _ => seg_last,
            };
            let reason = if first_gap.is_some_and(|gap| cursor > gap.last_hour) {
                gap_reason.as_deref()
            } else if first_unattributed.is_some_and(|u| cursor >= u) {
                Some("Some raids up to this date have an unrecorded raider.")
            } else {
                None
            };
            if let Some(reason) = reason {
                if let Some(prev) = out.last_mut()
                    && prev.reason == reason
                    && prev.last_hour.saturating_add(1) == cursor
                {
                    prev.last_hour = end;
                } else {
                    out.push(MissingSpan { first_hour: cursor, last_hour: end, reason: reason.to_owned() });
                }
            }
            if end == seg_last {
                break;
            }
            cursor = end.saturating_add(1);
        }
    }
    out
}

fn unattributed_for_metric(
    metric: RaidTallyMetric,
    launched: &[i32],
    succeeded: &[i32],
    both: &[i32],
    through: i32,
) -> (u32, Option<i32>) {
    let launched_count = launched.partition_point(|&h| h <= through) as u32;
    let succeeded_count = succeeded.partition_point(|&h| h <= through) as u32;
    let both_count = both.partition_point(|&h| h <= through) as u32;
    let count = metric_unattributed(metric, launched_count, succeeded_count, both_count);
    let first = match metric {
        RaidTallyMetric::Launched => launched.first().copied(),
        RaidTallyMetric::Succeeded => succeeded.first().copied(),
        RaidTallyMetric::Both => both.first().copied(),
    }
    .filter(|&h| h <= through);
    (count, first)
}

fn metric_partial_spans(
    metric: RaidTallyMetric,
    segments: &[(i32, i32)],
    missing: &[MissingSpan],
    index: &BranchTallyIndex,
    through: i32,
) -> (Option<Vec<MissingSpan>>, Option<Vec<MissingSpan>>) {
    if metric != RaidTallyMetric::Both {
        return (None, None);
    }
    let (_, first_ul) = unattributed_for_metric(
        RaidTallyMetric::Launched,
        &index.unattributed_launched,
        &index.unattributed_succeeded,
        &index.unattributed_both,
        through,
    );
    let (_, first_us) = unattributed_for_metric(
        RaidTallyMetric::Succeeded,
        &index.unattributed_launched,
        &index.unattributed_succeeded,
        &index.unattributed_both,
        through,
    );
    (
        Some(partial_spans(segments, missing, first_ul)),
        Some(partial_spans(segments, missing, first_us)),
    )
}

pub struct RaidTally;

impl Product for RaidTally {
    const KEY: &'static str = "raid-tally";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = RaidTallyParams;
    const FIELDS: dashboard_base::products::Fields =
        dashboard_base::fields![branchId, countries, top, metric, hour: Read, "countries[]"];
    type Reply = RaidTallyReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            RaidTallyParams::decl(),
            RaidTallySpan::decl(),
            RaidTallySlotRun::decl(),
            RaidTallySlotSeries::decl(),
            RaidTallyReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        validate(&params.branch_id, &params.countries)
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let query = dashboard_base::queries::query(
            source,
            params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?,
        )?;
        let index = cached_branch_index(source, &params.branch_id, &query)?;
        let hour = params.hour;
        let through = hour.min(index.branch_last);

        let content = Content::load(source, context);
        let none = content.raids.as_ref().is_ok_and(|raids| raids.types.is_empty() && raids.uncertain.is_empty());
        let available = if none {
            Some(false)
        } else if index.first_raid_hour.is_some_and(|h| h <= through) {
            Some(true)
        } else {
            None
        };
        let unavailable = none.then(|| "The content declares no raid type.".to_owned());

        let mut warnings: BTreeSet<String> = BTreeSet::new();
        match &content.raids {
            Ok(raids) => warnings.extend(raids.misreads.iter().map(|(_, text)| text.clone())),
            Err(reason) if content.catalogue.is_ok() => {
                warnings.insert(format!("Raid types are unavailable: {reason}"));
            }
            Err(_) => {}
        }

        let bounded_missing: Vec<MissingSpan> = index
            .missing
            .iter()
            .filter(|span| span.first_hour <= through)
            .map(|span| MissingSpan {
                first_hour: span.first_hour,
                last_hour: span.last_hour.min(through),
                reason: span.reason.clone(),
            })
            .collect();

        let segments = recorded_segments(index.branch_first, through, &bounded_missing);
        let ev_end = index.events.partition_point(|ev| ev.hour <= through);
        let bounded_events = &index.events[..ev_end];
        let (unattributed, first_unattributed) = unattributed_for_metric(
            params.metric,
            &index.unattributed_launched,
            &index.unattributed_succeeded,
            &index.unattributed_both,
            through,
        );
        let partial = partial_spans(&segments, &bounded_missing, first_unattributed);
        let (partial_launched, partial_succeeded) =
            metric_partial_spans(params.metric, &segments, &bounded_missing, &index, through);

        let (series, next_change, rank_missing, rank_missing_from) = if let Some(top) = params.top {
            let first_gap = bounded_missing.first().map(|gap| {
                (gap.first_hour, format!("Raid history is incomplete up to this date: {}", gap.reason))
            });
            let first_unattr = (unattributed > 0)
                .then_some(first_unattributed)
                .flatten()
                .map(|u_hour| {
                    (
                        u_hour,
                        format!("{unattributed} raid(s) up to this date have an unrecorded raider, so the top ranking is withheld"),
                    )
                });
            let rank_uncertain = match (first_gap, first_unattr) {
                (Some(g), Some(u)) => Some(if g.0 <= u.0 { g } else { u }),
                (Some(g), None) => Some(g),
                (None, Some(u)) => Some(u),
                (None, None) => None,
            };
            let next = compute_next_change(&index, through, true, &[], None, params.metric);
            if let Some((uncertain_hour, reason)) = rank_uncertain {
                let proven_through = uncertain_hour.saturating_sub(1);
                let built = if proven_through >= index.branch_first {
                    let proven_segments = recorded_segments(index.branch_first, proven_through, &bounded_missing);
                    let proven_ev_end = index.events.partition_point(|ev| ev.hour <= proven_through);
                    let proven_events = &index.events[..proven_ev_end];
                    let (ordered_slots, membership_map) =
                        compute_top_membership(proven_events, &proven_segments, top as usize, params.metric);
                    ordered_slots
                        .into_iter()
                        .map(|slot| RaidTallySlotSeries {
                            slot,
                            runs: build_slot_runs(proven_events, slot, &proven_segments),
                            membership: membership_map.get(&slot).cloned(),
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                (built, next, Some(reason), Some(uncertain_hour))
            } else {
                let (ordered_slots, membership_map) =
                    compute_top_membership(bounded_events, &segments, top as usize, params.metric);
                let built: Vec<RaidTallySlotSeries> = ordered_slots
                    .into_iter()
                    .map(|slot| RaidTallySlotSeries {
                        slot,
                        runs: build_slot_runs(bounded_events, slot, &segments),
                        membership: membership_map.get(&slot).cloned(),
                    })
                    .collect();
                (built, next, None, None)
            }
        } else {
            let (slots, next_holder) = resolve_explicit_slots(&query, &params.countries, through);
            let built: Vec<RaidTallySlotSeries> = slots
                .iter()
                .map(|&slot| RaidTallySlotSeries {
                    slot,
                    runs: build_slot_runs(bounded_events, slot, &segments),
                    membership: None,
                })
                .collect();
            let next = compute_next_change(&index, through, false, &slots, next_holder, params.metric);
            (built, next, None, None)
        };

        let reply = RaidTallyReply {
            hour,
            next_change,
            available,
            unavailable,
            rank_missing,
            rank_missing_from,
            unattributed,
            series,
            missing: bounded_missing,
            partial,
            partial_launched,
            partial_succeeded,
            warnings: warnings.into_iter().collect(),
        };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unattributed_launch_and_later_success_follow_metric_specific_attribution() {
        let index = BranchTallyIndex {
            branch_first: 10,
            branch_last: 14,
            first_raid_hour: Some(10),
            events: vec![
                TallyEvent { hour: 10, slot: 7, kind: EventKind::Launched },
                TallyEvent { hour: 11, slot: 7, kind: EventKind::Succeeded },
            ],
            unattributed_launched: vec![10],
            unattributed_succeeded: vec![12],
            unattributed_both: vec![10, 12],
            missing: Vec::new(),
        };
        let segs_11 = recorded_segments(10, 11, &[]);
        let (succ_count_11, succ_first_11) = unattributed_for_metric(
            RaidTallyMetric::Succeeded,
            &index.unattributed_launched,
            &index.unattributed_succeeded,
            &index.unattributed_both,
            11,
        );
        assert_eq!((succ_count_11, succ_first_11), (0, None));
        assert!(partial_spans(&segs_11, &[], succ_first_11).is_empty());
        assert_eq!(compute_next_change(&index, 11, true, &[], None, RaidTallyMetric::Succeeded), Some(12));

        let (launch_count_11, launch_first_11) = unattributed_for_metric(
            RaidTallyMetric::Launched,
            &index.unattributed_launched,
            &index.unattributed_succeeded,
            &index.unattributed_both,
            11,
        );
        assert_eq!((launch_count_11, launch_first_11), (1, Some(10)));
        let launch_partial = partial_spans(&segs_11, &[], launch_first_11);
        assert_eq!(launch_partial.len(), 1);
        assert_eq!((launch_partial[0].first_hour, launch_partial[0].last_hour), (10, 11));

        let segs_12 = recorded_segments(10, 12, &[]);
        let (succ_count_12, succ_first_12) = unattributed_for_metric(
            RaidTallyMetric::Succeeded,
            &index.unattributed_launched,
            &index.unattributed_succeeded,
            &index.unattributed_both,
            12,
        );
        assert_eq!((succ_count_12, succ_first_12), (1, Some(12)));
        let succ_partial = partial_spans(&segs_12, &[], succ_first_12);
        assert_eq!(succ_partial.len(), 1);
        assert_eq!((succ_partial[0].first_hour, succ_partial[0].last_hour), (12, 12));

        let (both_count_12, both_first_12) = unattributed_for_metric(
            RaidTallyMetric::Both,
            &index.unattributed_launched,
            &index.unattributed_succeeded,
            &index.unattributed_both,
            12,
        );
        assert_eq!((both_count_12, both_first_12), (2, Some(10)), "distinct unattributed launch and success episodes count as 2 in Both mode");

        assert_eq!(metric_rank_key(RaidTallyMetric::Succeeded, 5, 1), metric_rank_key(RaidTallyMetric::Succeeded, 1, 1), "Succeeded tiebreak ignores launches");
        assert_eq!(metric_rank_key(RaidTallyMetric::Launched, 1, 5), metric_rank_key(RaidTallyMetric::Launched, 1, 0), "Launched tiebreak ignores successes");
        assert!(metric_rank_key(RaidTallyMetric::Both, 1, 2) > metric_rank_key(RaidTallyMetric::Both, 1, 1));

        let (pl_11, ps_11) = metric_partial_spans(RaidTallyMetric::Both, &segs_11, &[], &index, 11);
        assert_eq!(pl_11.as_deref().map(|s| s.iter().map(|x| (x.first_hour, x.last_hour)).collect::<Vec<_>>()), Some(vec![(10, 11)]));
        assert_eq!(ps_11.as_deref(), Some(&[][..]), "Both mode leaves succeeded partial spans empty when only launches are unattributed");
        let (pl_12, ps_12) = metric_partial_spans(RaidTallyMetric::Both, &segs_12, &[], &index, 12);
        assert_eq!(pl_12.as_deref().map(|s| s.iter().map(|x| (x.first_hour, x.last_hour)).collect::<Vec<_>>()), Some(vec![(10, 12)]));
        assert_eq!(ps_12.as_deref().map(|s| s.iter().map(|x| (x.first_hour, x.last_hour)).collect::<Vec<_>>()), Some(vec![(12, 12)]));
    }
}
