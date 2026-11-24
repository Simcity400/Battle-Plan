
use super::{Amount, ApmBucket, ApmGroup, ApmMembership, ApmParams, ApmValue, Coverage, index, merge_intervals, player_id, player_name, query, tag};
use dashboard_base::Source;
use crate::products::campaign_clock;
use dashboard_follow::head::{Head, HeadRows, HeadValue};
use dashboard_query::{
    chart::ChartBucket,
    series::MissingSpan,
    tile::{self, SeriesTileReply, TileLine},
};
use replay_capture_spool::commands::Index;
use replay_data::Hour;
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};

pub const SERIES: &str = "player-apm";

pub const HEADS: &[(&str, dashboard_base::products::Fields, fn() -> dashboard_query::fields::Walk)] = &[
    (SERIES, dashboard_base::fields![branchId, group, tags, players, firstHour: Start, lastHour: End, windowHours, "membership[].firstHour": Start, "membership[].lastHour": End],
        dashboard_query::fields::walk::<ApmHeadParams>),
    ("campaign-clock", dashboard_base::fields![branchId, firstHour: Start, lastHour: End], dashboard_query::fields::walk::<ClockHeadParams>),
];

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApmHeadParams {
    pub branch_id: String,
    pub group: ApmGroup,
    pub tags: Option<Vec<String>>,
    pub membership: Option<Vec<ApmMembership>>,
    #[serde(default)]
    pub players: Vec<String>,
    pub first_hour: i32,
    pub last_hour: i32,
    pub window_hours: i32,
}

struct Version {
    actor: u64,
    version: usize,
    from: u64,
    to: u64,
    name: Option<String>,
    name_missing: Option<String>,
}

struct Part {
    series: usize,
    tag: String,
    ranges: Vec<(u64, u64)>,
    versions: Vec<Version>,
}

struct Piece {
    timeline: usize,
    hours: (i32, i32),
    parts: Vec<Part>,
}

#[derive(Default)]
struct Played {
    amount: Amount,
    shown: Option<(u64, u64)>,
    tag: String,
    name: Option<String>,
    name_missing: Option<String>,
}

type Laid = BTreeMap<(String, String), Part>;

pub const APM_COLUMNS: [&str; 11] = [
    "apm",
    "confirmedApm",
    "orders",
    "pausedOrders",
    "unpausedMinutes",
    "complete",
    "gaps",
    "tag",
    "hour",
    "name",
    "nameMissing",
];

pub struct ApmHead {
    index: Arc<Index>,
    ids: Vec<String>,
    pieces: Vec<Piece>,
    reach: Vec<i32>,
    memberships: Option<BTreeMap<String, Vec<(i32, i32)>>>,
    coverage: Coverage,
    first: i32,
    last: i32,
    window: i32,
    visits: u64,
}

impl ApmHead {
    pub fn new(index: Arc<Index>, params: &ApmHeadParams) -> io::Result<Self> {
        if params.window_hours < 1 || params.first_hour > params.last_hour {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid eAPM window"));
        }
        let wanted_tags: Option<BTreeSet<&str>> = params.tags.as_ref().map(|tags| tags.iter().map(String::as_str).collect());
        let wanted_players: BTreeSet<&str> = params.players.iter().map(String::as_str).collect();
        let mut laid: Vec<(usize, (i32, i32), Laid)> = Vec::new();
        for (at, timeline) in index.timelines.iter().enumerate() {
            let mut parts = Laid::new();
            for (&actor, history) in &timeline.actors {
                let player = player_id(timeline, actor, history);
                if params.group == ApmGroup::Player && !wanted_players.is_empty() && !wanted_players.contains(player.as_str()) {
                    continue;
                }
                let departed = timeline.departures.get(&actor).copied().unwrap_or(u64::MAX);
                for (version, observation) in history.iter().enumerate() {
                    let Some(tag) = tag(&observation.tag) else { continue };
                    if wanted_tags.as_ref().is_some_and(|tags| !tags.contains(tag)) {
                        continue;
                    }
                    let end = history.get(version + 1).map_or(departed, |next| next.at_qpc).min(departed);
                    let id = if params.group == ApmGroup::Country { tag.to_owned() } else { player.clone() };
                    let (name, name_missing) = if params.group == ApmGroup::Country {
                        (Some(tag.to_owned()), None)
                    } else {
                        match player_name(observation) {
                            Ok(name) => (Some(name), None),
                            Err(reason) => (None, Some(reason)),
                        }
                    };
                    let part = parts.entry((id, tag.to_owned())).or_insert_with(|| Part { series: 0, tag: tag.to_owned(), ranges: Vec::new(), versions: Vec::new() });
                    part.ranges.push((observation.at_qpc, end));
                    part.versions.push(Version { actor, version, from: observation.at_qpc, to: end, name, name_missing });
                }
            }
            for part in parts.values_mut() {
                merge_intervals(&mut part.ranges);
            }
            laid.push((at, timeline.hours, parts));
        }
        let ids: Vec<String> = laid.iter().flat_map(|(_, _, parts)| parts.keys().map(|(id, _)| id.clone())).collect::<BTreeSet<_>>().into_iter().collect();
        let mut pieces: Vec<Piece> = laid
            .into_iter()
            .map(|(timeline, hours, parts)| Piece {
                timeline,
                hours,
                parts: parts.into_iter().map(|((id, _), mut part)| { part.series = ids.binary_search(&id).expect("a laid id"); part }).collect(),
            })
            .collect();
        pieces.sort_by_key(|piece| piece.hours.0);
        let mut most = i32::MIN;
        let reach = pieces.iter().map(|piece| { most = most.max(piece.hours.1); most }).collect();
        let memberships = params.membership.as_ref().map(|members| {
            let mut by_tag: BTreeMap<String, Vec<(i32, i32)>> = BTreeMap::new();
            for member in members {
                by_tag.entry(member.tag.clone()).or_default().push((member.first_hour, member.last_hour));
            }
            for runs in by_tag.values_mut() {
                merge_intervals(runs);
            }
            by_tag
        });
        let coverage = Coverage::new(&index, params.first_hour, params.last_hour)?;
        Ok(Self { index, ids, pieces, reach, memberships, coverage, first: params.first_hour, last: params.last_hour, window: params.window_hours, visits: 0 })
    }

    pub fn visits(&self) -> u64 {
        self.visits
    }

    fn amounts(&mut self, first: i32, last: i32) -> BTreeMap<usize, Played> {
        let mut out: BTreeMap<usize, Played> = BTreeMap::new();
        let upper = self.pieces.partition_point(|piece| piece.hours.0 <= last);
        let lower = self.reach.partition_point(|&reach| reach < first);
        let mut visits = 0;
        for piece in &self.pieces[lower.min(upper)..upper] {
            let timeline = &self.index.timelines[piece.timeline];
            let (lo, hi) = (first.max(piece.hours.0), last.min(piece.hours.1));
            if lo > hi {
                continue;
            }
            visits += 1;
            if timeline.bounds(lo, hi).is_none() {
                continue;
            }
            for part in &piece.parts {
                let spans: Vec<(i32, i32)> = match &self.memberships {
                    None => vec![(lo, hi)],
                    Some(members) => members.get(&part.tag).into_iter().flatten().map(|&(from, to)| (from.max(lo), to.min(hi))).filter(|(from, to)| from <= to).collect(),
                };
                let mut present = Vec::new();
                for version in &part.versions {
                    visits += 1;
                    let seen = match &self.memberships {
                        None => timeline.measure(first, last, version.from, version.to).is_some(),
                        Some(members) => members.get(&part.tag).is_some_and(|runs| runs.iter().any(|&(member_first, member_last)| {
                            let (from, to) = (first.max(member_first), last.min(member_last));
                            from <= to && timeline.measure(from, to, version.from, version.to).is_some()
                        })),
                    };
                    if seen {
                        present.push(version);
                    }
                }
                if present.is_empty() {
                    continue;
                }
                let latest = present.iter().max_by_key(|version| version.from).expect("present is non-empty");
                let display_at = (timeline.session, latest.from);
                let entry = out.entry(part.series).or_default();
                if entry.shown.is_none_or(|shown| shown <= display_at) {
                    entry.shown = Some(display_at);
                    entry.tag.clone_from(&part.tag);
                    entry.name.clone_from(&latest.name);
                    entry.name_missing.clone_from(&latest.name_missing);
                }
                for &(from_hour, to_hour) in &spans {
                    for &(from, to) in &part.ranges {
                        visits += 1;
                        if let Some(measured) = timeline.measure(from_hour, to_hour, from, to) {
                            entry.amount.observe(&measured);
                            visits += 1;
                            entry.amount.paused_incomplete |= timeline.paused_unknown_hours.range(measured.first_hour..=measured.last_hour).next().is_some();
                        }
                    }
                }
                for version in present {
                    let Some(counts) = timeline.counts.get(&(version.actor, version.version)) else { continue };
                    for &(from, to) in &spans {
                        visits += 1;
                        let (count, incomplete) = counts.range(from, to);
                        entry.amount.counts.add(count);
                        entry.amount.incomplete |= incomplete;
                    }
                }
            }
        }
        visits += 1;
        let whole = self.coverage.observed(first, last);
        for played in out.values_mut() {
            played.amount.gaps |= !whole;
        }
        self.visits += visits;
        out.retain(|_, played| played.amount.first_hour.is_some());
        out
    }

    fn window_at(&self, hour: Hour) -> Option<(i32, i32)> {
        let last = hour.min(self.last);
        let first = self.first.max(hour.saturating_sub(self.window - 1));
        (first <= last).then_some((first, last))
    }
}

impl HeadValue for ApmHead {
    fn columns(&self) -> usize {
        APM_COLUMNS.len()
    }

    fn answer(&mut self, hour: Hour, _reset: bool, rows: &mut HeadRows<'_>) -> Result<(), String> {
        if !self.index.analysis.recorded {
            return Err("Player orders were not recorded in this campaign.".into());
        }
        let (first, last) = self.window_at(hour).ok_or("This time is outside the recording.")?;
        for (series, Played { amount, tag, name, name_missing, .. }) in self.amounts(first, last) {
            let value: ApmValue = amount.value();
            rows.row(series as u128, &self.ids[series]);
            rows.float(value.apm);
            rows.float(value.confirmed_apm);
            rows.number(value.orders.and_then(|orders| orders.parse().ok()));
            rows.number(value.paused_orders.and_then(|orders| orders.parse().ok()));
            rows.float(value.unpaused_minutes);
            rows.flag(value.complete);
            rows.flag(value.gaps);
            rows.text(Some(&tag));
            rows.number(amount.last_hour.map(i64::from));
            rows.text(name.as_deref());
            rows.text(name_missing.as_deref());
        }
        Ok(())
    }
}

fn unknown_rate(value: &ApmValue) -> String {
    if !value.complete {
        "Player orders or unpaused time were not fully recorded for this period; the rate is unknown.".into()
    } else if value.unpaused_minutes == Some(0.0) {
        "No unpaused time was observed in this period.".into()
    } else {
        "It is unclear whether some orders count toward eAPM; the rate is unknown.".into()
    }
}

pub struct ClockHead {
    index: Arc<Index>,
    branch_id: String,
    first: i32,
    last: i32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClockHeadParams {
    pub branch_id: String,
    pub first_hour: i32,
    pub last_hour: i32,
}

pub const CLOCK_COLUMNS: [&str; 4] = ["recordedSeconds", "playingSeconds", "pausedSeconds", "complete"];

impl HeadValue for ClockHead {
    fn columns(&self) -> usize {
        CLOCK_COLUMNS.len()
    }

    fn answer(&mut self, hour: Hour, _reset: bool, rows: &mut HeadRows<'_>) -> Result<(), String> {
        let last = hour.min(self.last);
        if last < self.first {
            return Err("This time is outside the recording.".into());
        }
        let params = campaign_clock::CampaignClockParams { branch_id: self.branch_id.clone(), first_hour: self.first, last_hour: last };
        let reply = campaign_clock::query(&self.index, &params, &AtomicBool::new(false)).map_err(|error| error.to_string())?;
        if !reply.available {
            return Err("Playing and paused time were not recorded in this campaign.".into());
        }
        if reply.recorded_seconds.is_none() {
            return Err(campaign_clock::OVERLAP_REASON.into());
        }
        rows.row(0, "clock");
        rows.float(reply.recorded_seconds);
        rows.float(reply.playing_seconds);
        rows.float(reply.paused_seconds);
        rows.flag(reply.complete);
        Ok(())
    }
}

pub fn head_of(source: &Source, asked: &Value) -> Option<Result<Head, String>> {
    let kind = asked.get("head")?.as_str()?;
    let params = asked.get("params").cloned().unwrap_or(Value::Null);
    let build = || -> io::Result<Head> {
        let cancelled = AtomicBool::new(false);
        match kind {
            SERIES => {
                let params: ApmHeadParams = dashboard_query::named::named(&params)?;
                let branch = params.branch_id.parse().map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid branch"))?;
                Ok(Head::new(ApmHead::new(index(source, branch, &cancelled)?, &params)?))
            }
            "campaign-clock" => {
                let params: ClockHeadParams = dashboard_query::named::named(&params)?;
                let branch = params.branch_id.parse().map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid branch"))?;
                Ok(Head::new(ClockHead { index: index(source, branch, &cancelled)?, branch_id: params.branch_id, first: params.first_hour, last: params.last_hour }))
            }
            other => Err(io::Error::new(io::ErrorKind::InvalidInput, format!("No head value {other}"))),
        }
    };
    Some(build().map_err(|error| error.to_string()))
}

pub fn tile_family_walk() -> dashboard_query::fields::Walk {
    dashboard_query::fields::walk::<TileFamily>()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TileFamily {
    group: ApmGroup,
    tags: Option<Vec<String>>,
    membership: Option<Vec<ApmMembership>>,
    #[serde(default)]
    players: Vec<String>,
    value: String,
}

pub fn tile_ask_walk() -> dashboard_query::fields::Walk {
    dashboard_query::fields::walk::<TileAsk>()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TileAsk {
    bin_hours: i64,
    index: i64,
    tags: Vec<String>,
}

pub fn tile(source: &Source, branch: u64, timeline_last: i32, params: &Value) -> io::Result<SeriesTileReply> {
    let invalid = |what: &str| io::Error::new(io::ErrorKind::InvalidInput, what.to_owned());
    let family: TileFamily = dashboard_query::named::named(params.get("family").ok_or_else(|| invalid("A tile names no family"))?)?;
    let ask: TileAsk = dashboard_query::named::named(params)?;
    if family.value != "apm" && family.value != "pausedOrders" {
        return Err(invalid("No eAPM tile value"));
    }
    let (first, last) = tile::hours(ask.bin_hours, ask.index)?;
    let held = index(source, branch, &AtomicBool::new(false))?;
    let reply = query(&held, &ApmParams {
        branch_id: branch.to_string(),
        group: family.group,
        registry: false,
        first_hour: Some(first),
        last_hour: Some(last),
        tags: family.tags,
        membership: family.membership,
        players: family.players,
        buckets: tile::BINS as u32,
    }, &AtomicBool::new(false))?;
    let mut buckets: Vec<Option<ChartBucket>> = vec![None; ask.tags.len() * tile::BINS];
    let mut lines = Vec::with_capacity(ask.tags.len());
    for (at, id) in ask.tags.iter().enumerate() {
        let mut missing = Vec::new();
        if let Some(series) = reply.series.iter().find(|series| &series.id == id) {
            let mut before = false;
            for ApmBucket { first_hour, last_hour, value } in &series.points {
                let bin = ((i64::from(*first_hour) - i64::from(first)) / ask.bin_hours) as usize;
                if bin >= tile::BINS {
                    continue;
                }
                let shown = if family.value == "apm" { value.apm } else { value.paused_orders.as_deref().and_then(|orders| orders.parse::<f64>().ok()) };
                let Some(shown) = shown else {
                    missing.push(MissingSpan { first_hour: *first_hour, last_hour: *last_hour, reason: unknown_rate(value) });
                    before = false;
                    continue;
                };
                buckets[at * tile::BINS + bin] = Some(ChartBucket { absolute_total: 0, first_hour: *first_hour, last_hour: *last_hour, first_value: shown, last_value: shown,
                    last_value_hour: *last_hour, min_hour: *first_hour, min_value: shown, max_hour: *first_hour, max_value: shown, first_break: !before, has_break: false });
                before = true;
            }
        }
        lines.push(TileLine { country: id.clone().into(), part: None, missing, parts: Vec::new(), marks: None });
    }
    let (immutable, filled) = tile::closure(first, last, timeline_last);
    let (bins, raw_values, hour_bytes) = tile::encode(&buckets, first, ask.bin_hours, 1.0);
    Ok(SeriesTileReply { branch_id: branch.to_string(), measure: family.value, decimals: 0, bin_hours: ask.bin_hours, index: ask.index, first_hour: first,
        immutable, filled, raw_values, hour_bytes, lines, bins: tile::base64(&bins), ranking: None })
}

#[cfg(test)]
#[path = "player_apm_head/tests.rs"]
mod tests;
