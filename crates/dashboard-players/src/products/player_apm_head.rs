
use super::{Amount, ApmGroup, ApmMembership, ApmValue, Coverage, Minutes, MINUTES_OVERFLOW, Rate, hourly_amounts, index, merge_intervals, player_id, player_name, tag};
use dashboard_base::Source;
use crate::products::campaign_clock;
use dashboard_follow::head::{Head, HeadRows, HeadValue};
use dashboard_query::{
    chart::ChartBucket,
    series::MissingSpan,
    tile::{self, SeriesTileReply, TileLine},
};
use replay_capture_spool::commands::{Counts, Index};
use replay_data::Hour;
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
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

#[derive(Clone, Default)]
struct HourSlice {
    counts: Counts,
    minutes: Minutes,
    incomplete: bool,
    paused_incomplete: bool,
    shown: Option<(u64, u64)>,
    tag: String,
    name: Option<String>,
    name_missing: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Ranked {
    rate: std::cmp::Reverse<Option<Rate>>,
    series: usize,
}

#[derive(Default)]
struct WindowSeries {
    hours: VecDeque<(i32, HourSlice)>,
    offset: usize,
    newest: VecDeque<usize>,
    total: Amount,
    incomplete: usize,
    paused_incomplete: usize,
    cached: Option<(Ranked, ApmValue)>,
    dirty: bool,
}

impl WindowSeries {
    fn enter(&mut self, hour: i32, slice: HourSlice) -> Result<(), String> {
        self.total.minutes = self.total.minutes.combine(slice.minutes, u128::checked_add).ok_or(MINUTES_OVERFLOW)?;
        self.total.counts.add(slice.counts);
        self.incomplete += usize::from(slice.incomplete);
        self.paused_incomplete += usize::from(slice.paused_incomplete);
        while self.newest.back().is_some_and(|&at| self.hours[at - self.offset].1.shown <= slice.shown) {
            self.newest.pop_back();
        }
        self.newest.push_back(self.offset + self.hours.len());
        self.hours.push_back((hour, slice));
        Ok(())
    }

    fn expire(&mut self) -> Result<(), String> {
        let (_, slice) = self.hours.pop_front().expect("a recorded boundary hour");
        self.total.minutes = self.total.minutes.combine(slice.minutes, u128::checked_sub).ok_or(MINUTES_OVERFLOW)?;
        let Counts { orders, paused_orders, unresolved, unclassified, paused_unresolved } = slice.counts;
        let counts = &mut self.total.counts;
        counts.orders -= orders;
        counts.paused_orders -= paused_orders;
        counts.unresolved -= unresolved;
        counts.unclassified -= unclassified;
        counts.paused_unresolved -= paused_unresolved;
        self.incomplete -= usize::from(slice.incomplete);
        self.paused_incomplete -= usize::from(slice.paused_incomplete);
        if self.newest.front() == Some(&self.offset) { self.newest.pop_front(); }
        self.offset += 1;
        Ok(())
    }

    fn mark(&mut self, series: usize, changed: &mut Vec<usize>) {
        if !self.dirty {
            self.dirty = true;
            changed.push(series);
        }
    }

    fn summary(&mut self) -> ApmValue {
        self.total.first_hour = self.hours.front().map(|(hour, _)| *hour);
        self.total.last_hour = self.hours.back().map(|(hour, _)| *hour);
        self.total.incomplete = self.incomplete != 0;
        self.total.paused_incomplete = self.paused_incomplete != 0;
        self.total.value()
    }

    fn identity(&self) -> &HourSlice {
        &self.hours[self.newest[0] - self.offset].1
    }
}

type Laid = BTreeMap<(String, String), Part>;

pub const APM_COLUMNS: [&str; 14] = [
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
    "confirmedOrders",
    "unpausedMinutesNum",
    "unpausedMinutesDen",
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
    cached_window: Option<(i32, i32)>,
    #[cfg(test)]
    participants_ranked: usize,
    cached_whole: bool,
    participants: HashMap<usize, WindowSeries>,
    boundaries: VecDeque<(i32, Vec<usize>)>,
    ranking: BTreeSet<Ranked>,
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
        Ok(Self {
            index,
            ids,
            pieces,
            reach,
            memberships,
            coverage,
            first: params.first_hour,
            last: params.last_hour,
            window: params.window_hours,
            cached_window: None,
            #[cfg(test)]
            participants_ranked: 0,
            cached_whole: true,
            participants: HashMap::new(),
            boundaries: VecDeque::new(),
            ranking: BTreeSet::new(),
        })
    }

    fn window_at(&self, hour: Hour) -> Option<(i32, i32)> {
        let last = hour.min(self.last);
        let first = self.first.max(hour.saturating_sub(self.window - 1));
        (first <= last).then_some((first, last))
    }

    fn member_at(&self, tag: &str, hour: i32) -> bool {
        match &self.memberships {
            None => true,
            Some(members) => members.get(tag).is_some_and(|runs| {
                let at = runs.partition_point(|&(_, end)| end < hour);
                runs.get(at).is_some_and(|&(start, end)| start <= hour && hour <= end)
            }),
        }
    }

    fn hour_slices(&self, hour: i32) -> Result<BTreeMap<usize, HourSlice>, String> {
        let mut out: BTreeMap<usize, HourSlice> = BTreeMap::new();
        let upper = self.pieces.partition_point(|piece| piece.hours.0 <= hour);
        let lower = self.reach.partition_point(|&reach| reach < hour);
        for piece in &self.pieces[lower.min(upper)..upper] {
            if hour < piece.hours.0 || hour > piece.hours.1 {
                continue;
            }
            let timeline = &self.index.timelines[piece.timeline];
            if timeline.bounds(hour, hour).is_none() {
                continue;
            }
            for part in &piece.parts {
                if !self.member_at(&part.tag, hour) {
                    continue;
                }
                let mut present = Vec::new();
                for version in &part.versions {
                    if timeline.measure(hour, hour, version.from, version.to).is_some() {
                        present.push(version);
                    }
                }
                if present.is_empty() {
                    continue;
                }
                let latest = present.iter().max_by_key(|v| v.from).expect("present is non-empty");
                let display_at = (timeline.session, latest.from);
                let slice = out.entry(part.series).or_default();
                if slice.shown.is_none_or(|shown| shown <= display_at) {
                    slice.shown = Some(display_at);
                    slice.tag.clone_from(&part.tag);
                    slice.name.clone_from(&latest.name);
                    slice.name_missing.clone_from(&latest.name_missing);
                }
                for &(from, to) in &part.ranges {
                    if let Some(measured) = timeline.measure(hour, hour, from, to) {
                        slice.minutes.add_ticks(measured.ticks, measured.frequency)?;
                        slice.incomplete |= !measured.complete;
                        slice.paused_incomplete |= timeline.paused_unknown_hours.range(hour..=hour).next().is_some();
                    }
                }
                for version in present {
                    let Some(counts) = timeline.counts.get(&(version.actor, version.version)) else { continue };
                    let (count, incomplete) = counts.range(hour, hour);
                    slice.counts.add(count);
                    slice.incomplete |= incomplete;
                }
            }
        }
        Ok(out)
    }

    fn add_hour(&mut self, hour: i32, changed: &mut Vec<usize>) -> Result<(), String> {
        let slices = self.hour_slices(hour)?;
        if slices.is_empty() { return Ok(()); }
        let mut ids = Vec::with_capacity(slices.len());
        for (id, slice) in slices {
            let participant = self.participants.entry(id).or_default();
            participant.enter(hour, slice)?;
            participant.mark(id, changed);
            ids.push(id);
        }
        #[cfg(test)]
        dashboard_base::products::at_hour::count_rows(|| ids.len() as u64);
        self.boundaries.push_back((hour, ids));
        Ok(())
    }

    fn expire_before(&mut self, first: i32, changed: &mut Vec<usize>) -> Result<(), String> {
        while self.boundaries.front().is_some_and(|(hour, _)| *hour < first) {
            let (_, ids) = self.boundaries.pop_front().unwrap();
            #[cfg(test)]
            dashboard_base::products::at_hour::count_rows(|| ids.len() as u64);
            for id in ids {
                let participant = self.participants.get_mut(&id).expect("a retained participant");
                participant.expire()?;
                participant.mark(id, changed);
            }
        }
        Ok(())
    }

    fn refresh(&mut self, changed: Vec<usize>) {
        for series in changed {
            let participant = self.participants.get_mut(&series).expect("a changed participant");
            let old = participant.cached.take().map(|(rank, _)| rank);
            if participant.hours.is_empty() {
                if let Some(old) = old { self.ranking.remove(&old); }
                self.participants.remove(&series);
                continue;
            }
            let value = participant.summary();
            let rank = Ranked { series, rate: std::cmp::Reverse(participant.total.rate()) };
            if old != Some(rank) {
                if let Some(old) = old { self.ranking.remove(&old); }
                self.ranking.insert(rank);
                #[cfg(test)]
                { self.participants_ranked += 1; }
            }
            participant.cached = Some((rank, value));
            participant.dirty = false;
        }
    }

    fn seed_window(&mut self, first: i32, last: i32, changed: &mut Vec<usize>) -> Result<(), String> {
        self.participants.clear();
        self.boundaries.clear();
        self.ranking.clear();
        let upper = self.pieces.partition_point(|piece| piece.hours.0 <= last);
        let lower = self.reach.partition_point(|&reach| reach < first);
        let mut hours_to_visit: Vec<i32> = Vec::new();
        for piece in &self.pieces[lower.min(upper)..upper] {
            let (lo, hi) = (first.max(piece.hours.0), last.min(piece.hours.1));
            if lo > hi {
                continue;
            }
            let timeline = &self.index.timelines[piece.timeline];
            let s_from = timeline.spans.partition_point(|s| s.hour < lo);
            let s_to = timeline.spans.partition_point(|s| s.hour <= hi);
            hours_to_visit.extend(timeline.spans[s_from..s_to].iter().map(|s| s.hour));
        }
        hours_to_visit.sort_unstable();
        hours_to_visit.dedup();
        for hour in hours_to_visit {
            self.add_hour(hour, changed)?;
        }
        Ok(())
    }

    fn update_window(&mut self, hour: Hour, reset: bool) -> Result<(), String> {
        if !self.index.analysis.recorded {
            return Err("Player orders were not recorded in this campaign.".into());
        }
        let (first, last) = self.window_at(hour).ok_or("This time is outside the recording.")?;
        if !reset && self.cached_window == Some((first, last)) { return Ok(()); }
        let previous = self.cached_window.take();
        let mut changed = Vec::new();
        if !reset && let Some((old_first, old_last)) = previous
            && first >= old_first && last >= old_last
            && (i64::from(first) - i64::from(old_first)) + (i64::from(last) - i64::from(old_last)) <= 8
        {
            self.expire_before(first, &mut changed)?;
            let entering = (i64::from(old_last) + 1).max(i64::from(first));
            for h in entering..=i64::from(last) {
                self.add_hour(h as i32, &mut changed)?;
            }
        } else {
            self.seed_window(first, last, &mut changed)?;
        }
        self.refresh(changed);
        self.cached_whole = self.coverage.observed(first, last);
        self.cached_window = Some((first, last));
        Ok(())
    }
}

impl HeadValue for ApmHead {
    fn columns(&self) -> usize { APM_COLUMNS.len() }

    fn answer(&mut self, hour: Hour, reset: bool, rows: &mut HeadRows<'_>) -> Result<(), String> {
        self.update_window(hour, reset)?;
        for rank in &self.ranking {
            let participant = &self.participants[&rank.series];
            let (_, value) = participant.cached.as_ref().expect("a ranked participant");
            let look = participant.identity();
            rows.row(rank.series as u128, &self.ids[rank.series]);
            rows.float(value.apm);
            rows.float(value.confirmed_apm);
            rows.number(value.orders.as_deref().and_then(|orders| orders.parse().ok()));
            rows.number(value.paused_orders.as_deref().and_then(|orders| orders.parse().ok()));
            rows.float(value.unpaused_minutes);
            rows.flag(value.complete);
            rows.flag(!self.cached_whole);
            rows.text(Some(&look.tag));
            rows.number(participant.total.last_hour.map(i64::from));
            rows.text(look.name.as_deref());
            rows.text(look.name_missing.as_deref());
            rows.text(value.confirmed_orders.as_deref());
            rows.text(value.unpaused_minutes_num.as_deref());
            rows.text(value.unpaused_minutes_den.as_deref());
        }
        Ok(())
    }
}

fn unknown_rate(value: &ApmValue) -> String {
    if !value.complete {
        "Player orders or unpaused time were not fully recorded for this period; the rate is unknown.".into()
    } else if value.unpaused_minutes_num.as_deref() == Some("0") || value.unpaused_minutes == Some(0.0) {
        "No unpaused time was observed in this period.".into()
    } else {
        "It is unclear whether some orders count toward eAPM; the rate is unknown.".into()
    }
}

pub struct ClockHead {
    index: Arc<Index>,
    coverage: Arc<campaign_clock::RecordedCoverage>,
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

impl ClockHead {
    pub(crate) fn from_source(source: &Source, params: ClockHeadParams, cancelled: &AtomicBool) -> io::Result<Self> {
        let branch = params
            .branch_id
            .parse()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid branch"))?;
        Ok(Self {
            index: index(source, branch, cancelled)?,
            coverage: campaign_clock::RecordedCoverage::for_branch(source, branch, cancelled)?,
            branch_id: params.branch_id,
            first: params.first_hour,
            last: params.last_hour,
        })
    }

    pub(crate) fn reply_at(&self, hour: Hour) -> Result<campaign_clock::CampaignClockReply, String> {
        let last = hour.min(self.last);
        if last < self.first {
            return Err("This time is outside the recording.".into());
        }
        let params = campaign_clock::CampaignClockParams {
            branch_id: self.branch_id.clone(),
            first_hour: self.first,
            last_hour: last,
        };
        let reply = campaign_clock::query(&self.index, &self.coverage, &params, &AtomicBool::new(false))
            .map_err(|error| error.to_string())?;
        if !reply.available {
            return Err("Playing and paused time were not recorded in this campaign.".into());
        }
        if reply.recorded_seconds.is_none() {
            return Err(campaign_clock::OVERLAP_REASON.into());
        }
        Ok(reply)
    }
}

impl HeadValue for ClockHead {
    fn columns(&self) -> usize {
        CLOCK_COLUMNS.len()
    }

    fn answer(&mut self, hour: Hour, _reset: bool, rows: &mut HeadRows<'_>) -> Result<(), String> {
        let reply = self.reply_at(hour)?;
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
                Ok(Head::new(ClockHead::from_source(source, params, &cancelled)?))
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
    countries: Vec<String>,
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
    tile_from_index(&held, branch, timeline_last, &family, &ask, first, last)
}

const GAP_REASON: &str = "Part of this period was not recorded; its orders and minutes are both left out.";

fn merge_missing_spans(mut spans: Vec<MissingSpan>) -> Vec<MissingSpan> {
    spans.sort_by_key(|s| (s.first_hour, s.last_hour));
    let mut merged: Vec<MissingSpan> = Vec::with_capacity(spans.len());
    for span in spans {
        if span.first_hour > span.last_hour {
            continue;
        }
        if let Some(prev) = merged.last_mut()
            && i64::from(span.first_hour) <= i64::from(prev.last_hour) + 1
            && prev.reason == span.reason
        {
            prev.last_hour = prev.last_hour.max(span.last_hour);
        } else {
            merged.push(span);
        }
    }
    merged
}

fn tile_from_index(
    held: &Index,
    branch: u64,
    timeline_last: i32,
    family: &TileFamily,
    ask: &TileAsk,
    first: i32,
    last: i32,
) -> io::Result<SeriesTileReply> {
    if !held.analysis.recorded {
        let covered_last = last.min(timeline_last);
        let missing = if first <= covered_last {
            vec![MissingSpan { first_hour: first, last_hour: covered_last, reason: "Player orders were not recorded in this campaign.".into() }]
        } else {
            Vec::new()
        };
        let lines = ask
            .countries
            .iter()
            .map(|id| TileLine { country: id.clone().into(), part: None, missing: missing.clone(), partial: Vec::new(), parts: Vec::new(), marks: None })
            .collect();
        let (immutable, filled) = tile::closure(first, last, timeline_last);
        let buckets: Vec<Option<ChartBucket>> = vec![None; ask.countries.len() * tile::BINS];
        let (bins, raw_values, hour_bytes) = tile::encode(&buckets, first, ask.bin_hours, 1.0);
        return Ok(SeriesTileReply { branch_id: branch.to_string(), measure: family.value.clone(), decimals: 0, bin_hours: ask.bin_hours, index: ask.index, first_hour: first,
            immutable, filled, raw_values, hour_bytes, lines, bins: tile::base64(&bins), ranking: None });
    }
    let clipped_end = last.min(timeline_last);
    let wanted_ids: BTreeSet<&str> = ask.countries.iter().map(String::as_str).collect();
    let per_series = if first <= clipped_end {
        hourly_amounts(
            held,
            family.group,
            first,
            clipped_end,
            family.tags.as_deref(),
            family.membership.as_deref(),
            &family.players,
            &wanted_ids,
        )?
    } else {
        BTreeMap::new()
    };
    let coverage = Coverage::new(held, first, clipped_end.max(first))?;
    let gaps: Vec<_> = coverage.missing_spans(first, clipped_end).into_iter()
        .map(|(first_hour, last_hour)| MissingSpan { first_hour, last_hour, reason: GAP_REASON.into() })
        .collect();
    let mut buckets: Vec<Option<ChartBucket>> = vec![None; ask.countries.len() * tile::BINS];
    let mut lines = Vec::with_capacity(ask.countries.len());
    for (at, id) in ask.countries.iter().enumerate() {
        let mut missing = gaps.clone();
        if let Some(hours_map) = per_series.get(id) {
            let mut before = false;
            for bin in 0..tile::BINS {
                let bin_first = (i64::from(first) + bin as i64 * ask.bin_hours) as i32;
                if bin_first > clipped_end {
                    break;
                }
                let bin_last = (i64::from(first) + (bin as i64 + 1) * ask.bin_hours - 1).min(i64::from(clipped_end)) as i32;
                let mut agg: Option<(i32, f64, i32, f64, i32, f64, i32, f64)> = None;
                let mut has_break = !coverage.observed(bin_first, bin_last);
                for (&h, amount) in hours_map.range(bin_first..=bin_last) {
                    let val = amount.value();
                    let shown = if family.value == "apm" {
                        val.apm
                    } else {
                        val.paused_orders.as_deref().and_then(|orders| orders.parse::<f64>().ok())
                    };
                    if let Some(rate) = shown {
                        agg = Some(match agg {
                            None => (h, rate, h, rate, h, rate, h, rate),
                            Some((fh, fv, _, _, minh, minv, maxh, maxv)) => {
                                let (minh, minv) = if rate < minv { (h, rate) } else { (minh, minv) };
                                let (maxh, maxv) = if rate > maxv { (h, rate) } else { (maxh, maxv) };
                                (fh, fv, h, rate, minh, minv, maxh, maxv)
                            }
                        });
                    } else if coverage.observed(h, h) {
                        has_break = true;
                        let reason = if family.value == "apm" { unknown_rate(&val) } else { "Paused orders were not fully recorded for this period.".into() };
                        missing.push(MissingSpan { first_hour: h, last_hour: h, reason });
                    }
                }
                if let Some((fh, fv, lh, lv, minh, minv, maxh, maxv)) = agg {
                    buckets[at * tile::BINS + bin] = Some(ChartBucket {
                        absolute_total: 0,
                        first_hour: fh,
                        last_hour: lh,
                        first_value: fv,
                        last_value: lv,
                        last_value_hour: lh,
                        min_hour: minh,
                        min_value: minv,
                        max_hour: maxh,
                        max_value: maxv,
                        first_break: !before,
                        has_break,
                    });
                    before = true;
                } else {
                    before = false;
                }
            }
        }
        lines.push(TileLine { country: id.clone().into(), part: None, missing: merge_missing_spans(missing), partial: Vec::new(), parts: Vec::new(), marks: None });
    }
    let (immutable, filled) = tile::closure(first, last, timeline_last);
    let (bins, raw_values, hour_bytes) = tile::encode(&buckets, first, ask.bin_hours, 1.0);
    Ok(SeriesTileReply { branch_id: branch.to_string(), measure: family.value.clone(), decimals: 0, bin_hours: ask.bin_hours, index: ask.index, first_hour: first,
        immutable, filled, raw_values, hour_bytes, lines, bins: tile::base64(&bins), ranking: None })
}

#[cfg(test)]
#[path = "player_apm_head/tests.rs"]
mod tests;
