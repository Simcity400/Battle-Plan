use dashboard_base::products::{LoadContext, Product, check_reply_budget};
use dashboard_core::protocol::validate_tag;
use dashboard_base::Source;
use replay_capture_spool::commands::{self, Counts, Index, Timeline};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, Mutex, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct PlayerApm;

#[path = "player_apm_head.rs"]
pub mod head;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ApmGroup {
    Country,
    Player,
}

#[derive(Clone, Debug, Deserialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApmParams {
    pub branch_id: String,
    pub group: ApmGroup,
    #[serde(default)]
    pub registry: bool,
    #[serde(default)]
    pub hour: Option<i32>,
    pub first_hour: Option<i32>,
    pub last_hour: Option<i32>,
    pub tags: Option<Vec<String>>,
    pub membership: Option<Vec<ApmMembership>>,
    #[serde(default)]
    pub players: Vec<String>,
    pub buckets: u32,
}

#[derive(Clone, Debug, Deserialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApmMembership {
    pub tag: String,
    pub first_hour: i32,
    pub last_hour: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ApmPlayerRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub tag: String,
    pub name: Option<String>,
    pub name_missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ApmPlayer {
    pub id: String,
    pub name: Option<String>,
    pub name_missing: Option<String>,
    pub tags: Vec<String>,
    pub runs: Vec<ApmPlayerRun>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ApmValue {
    pub apm: Option<f64>,
    pub confirmed_apm: Option<f64>,
    pub orders: Option<String>,
    pub confirmed_orders: Option<String>,
    pub paused_orders: Option<String>,
    pub unpaused_minutes: Option<f64>,
    pub unpaused_minutes_num: Option<String>,
    pub unpaused_minutes_den: Option<String>,
    pub complete: bool,
    #[doc = " Part of the period no capture observed; orders and minutes both leave it out."]
    pub gaps: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ApmBucket {
    pub first_hour: i32,
    pub last_hour: i32,
    pub value: ApmValue,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ApmSeries {
    pub id: String,
    pub name: Option<String>,
    pub name_missing: Option<String>,
    pub tag: Option<String>,
    pub hour: i32,
    pub summary: ApmValue,
    pub points: Vec<ApmBucket>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ApmReply {
    pub available: bool,
    pub players: Vec<ApmPlayer>,
    pub series: Vec<ApmSeries>,
    pub warnings: Vec<String>,
}

#[derive(Default)]
struct Cache {
    branches: BTreeMap<u64, Arc<Index>>,
    gates: BTreeMap<u64, Arc<Mutex<()>>>,
}

pub(super) fn index(
    source: &Source,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Arc<Index>> {
    let state = source.state::<Cache>();
    let gate = {
        let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(index) = cache.branches.get(&branch) {
            return Ok(index.clone());
        }
        cache.gates.entry(branch).or_default().clone()
    };
    let _guard = gate.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(index) = state
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .branches
        .get(&branch)
    {
        return Ok(index.clone());
    }
    dashboard_text::metadata::check_cancel(cancelled)?;
    let replay = source.replay_ref();
    let mut index: Option<Index> = None;
    let pieces = replay.pieces(branch);
    let hours = replay::campaign::timelines::command_hours(&pieces);
    for (piece, (first, last)) in pieces.iter().zip(hours) {
        let mut held = commands::index_with(
            &replay.sessions,
            &replay.controls,
            piece.segment,
            || dashboard_text::metadata::check_cancel(cancelled),
        )?;
        for timeline in &mut held.timelines {
            timeline.hours = (first, last);
            let kept = timeline.bounds(first, last);
            timeline.actors.retain(|_, history| {
                history.retain(|observation| kept.is_some_and(|(_, to)| observation.at_qpc <= to));
                !history.is_empty()
            });
        }
        match &mut index {
            None => index = Some(held),
            Some(index) => {
                index.analysis.recorded |= held.analysis.recorded;
                index.analysis.lost_observations += held.analysis.lost_observations;
                index.analysis.unattributed_commands += held.analysis.unattributed_commands;
                index.analysis.players.extend(held.analysis.players);
                index.timelines.extend(held.timelines);
            }
        }
    }
    let index = Arc::new(match index {
        Some(index) => index,
        None => commands::index_with(&replay.sessions, &replay.controls, branch, || {
            dashboard_text::metadata::check_cancel(cancelled)
        })?,
    });
    dashboard_text::metadata::check_cancel(cancelled)?;
    state
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .branches
        .insert(branch, index.clone());
    Ok(index)
}

fn player_id(timeline: &Timeline, actor: u64, history: &[replay_recording::command_packet::Actor]) -> String {
    use std::fmt::Write;
    let user = &history[0].user;
    if user.is_empty() || history.iter().any(|observation| observation.user != *user) {
        return format!(
            "lifetime:{}:{}:{}:{actor}",
            timeline.session, timeline.capture, timeline.world
        );
    }
    let mut id = String::with_capacity(5 + 2 * user.len());
    id.push_str("user:");
    for byte in user {
        write!(id, "{byte:02x}").unwrap();
    }
    id
}

pub(super) fn player_name(observation: &replay_recording::command_packet::Actor) -> Result<String, String> {
    match std::str::from_utf8(&observation.name) {
        Ok(name) if !name.trim().is_empty() => Ok(name.to_owned()),
        Ok(_) => Err("Player name was not recorded.".into()),
        Err(_) => Err("Player name is not valid UTF-8.".into()),
    }
}

fn tag(bytes: &[u8]) -> Option<&str> {
    let tag = std::str::from_utf8(bytes).ok()?;
    validate_tag(tag).ok()?;
    Some(tag)
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        let r = a % b;
        a = b;
        b = r;
    }
    a
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Minutes {
    pub(super) num: u128,
    pub(super) den: u128,
}

impl Default for Minutes {
    fn default() -> Self {
        Self { num: 0, den: 1 }
    }
}

const MINUTES_OVERFLOW: &str = "The exact unpaused time for this window exceeds the supported range.";

impl Minutes {
    fn add_ticks(&mut self, ticks: u128, frequency: u64) -> Result<(), &'static str> {
        if ticks == 0 || frequency == 0 {
            return Ok(());
        }
        let den = u128::from(frequency) * 60;
        let divisor = gcd(ticks, den);
        let other = Self { num: ticks / divisor, den: den / divisor };
        *self = self.combine(other, u128::checked_add).ok_or(MINUTES_OVERFLOW)?;
        Ok(())
    }

    fn combine(self, other: Self, operation: fn(u128, u128) -> Option<u128>) -> Option<Self> {
        if other.num == 0 {
            return Some(self);
        }
        let divisor = gcd(self.den, other.den);
        let left = self.num.checked_mul(other.den / divisor)?;
        let right = other.num.checked_mul(self.den / divisor)?;
        let num = operation(left, right)?;
        let den = (self.den / divisor).checked_mul(other.den)?;
        let reduce = gcd(num, den);
        Some(Self { num: num / reduce, den: den / reduce })
    }

    pub(super) fn is_positive(&self) -> bool {
        self.num > 0
    }

    pub(super) fn as_f64(&self) -> f64 {
        self.num as f64 / self.den as f64
    }
}

#[derive(Clone, Copy)]
struct Rate {
    orders: u64,
    minutes: Minutes,
}

impl Ord for Rate {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let product = |orders: u64, den: u128, num: u128| {
            let orders = u128::from(orders);
            if let Some(value) = orders.checked_mul(den).and_then(|pair| pair.checked_mul(num)) {
                return (0, 0, value);
            }
            let (low, high) = den.carrying_mul(num, 0);
            let (low, carry) = low.carrying_mul(orders, 0);
            let (middle, high) = high.carrying_mul(orders, carry);
            (high, middle, low)
        };
        product(self.orders, self.minutes.den, other.minutes.num)
            .cmp(&product(other.orders, other.minutes.den, self.minutes.num))
    }
}

impl PartialOrd for Rate {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> { Some(self.cmp(other)) }
}

impl PartialEq for Rate {
    fn eq(&self, other: &Self) -> bool { self.cmp(other).is_eq() }
}

impl Eq for Rate {}

#[derive(Clone, Default)]
pub(super) struct Amount {
    pub(super) counts: Counts,
    pub(super) minutes: Minutes,
    pub(super) incomplete: bool,
    pub(super) gaps: bool,
    pub(super) paused_incomplete: bool,
    pub(super) first_hour: Option<i32>,
    pub(super) last_hour: Option<i32>,
}

impl Amount {
    fn observe(&mut self, measured: &commands::Measurement) -> io::Result<()> {
        self.minutes.add_ticks(measured.ticks, measured.frequency).map_err(io::Error::other)?;
        self.incomplete |= !measured.complete;
        self.first_hour = Some(
            self.first_hour
                .map_or(measured.first_hour, |first| first.min(measured.first_hour)),
        );
        self.last_hour = Some(
            self.last_hour
                .map_or(measured.last_hour, |last| last.max(measured.last_hour)),
        );
        Ok(())
    }

    fn complete(&self) -> bool {
        !self.incomplete && self.first_hour.is_some()
    }

    fn orders_known(&self) -> bool {
        self.complete() && self.counts.unresolved == 0 && self.counts.unclassified == 0
    }

    fn rate(&self) -> Option<Rate> {
        (self.orders_known() && self.minutes.is_positive()).then_some(Rate { orders: self.counts.orders, minutes: self.minutes })
    }

    fn value(&self) -> ApmValue {
        let complete = self.complete();
        let minutes_f64 = self.minutes.as_f64();
        let confirmed_apm =
            (complete && self.minutes.is_positive()).then(|| self.counts.orders as f64 / minutes_f64);
        let known = self.orders_known();
        ApmValue {
            apm: confirmed_apm.filter(|_| known),
            confirmed_apm,
            orders: known.then(|| self.counts.orders.to_string()),
            confirmed_orders: complete.then(|| self.counts.orders.to_string()),
            paused_orders: (complete
                && !self.paused_incomplete
                && self.counts.paused_unresolved == 0)
                .then(|| self.counts.paused_orders.to_string()),
            unpaused_minutes: complete.then_some(minutes_f64),
            unpaused_minutes_num: complete.then(|| self.minutes.num.to_string()),
            unpaused_minutes_den: complete.then(|| self.minutes.den.to_string()),
            complete,
            gaps: self.gaps,
        }
    }
}

#[derive(Default)]
struct Series {
    name: Option<String>,
    name_missing: Option<String>,
    tag: Option<String>,
    total: Amount,
    buckets: BTreeMap<i64, Amount>,
    display_at: (u64, u64),
}

fn merge_intervals<T: Ord + Copy>(intervals: &mut Vec<(T, T)>) {
    intervals.sort_unstable();
    let mut count = 0;
    for index in 0..intervals.len() {
        let (from, to) = intervals[index];
        if count != 0 && intervals[count - 1].1 >= from {
            intervals[count - 1].1 = intervals[count - 1].1.max(to);
        } else {
            intervals[count] = (from, to);
            count += 1;
        }
    }
    intervals.truncate(count);
}

struct Coverage {
    covered: Vec<(i32, i32)>,
}

impl Coverage {
    fn new(index: &Index, first: i32, last: i32) -> io::Result<Self> {
        let mut hours = Vec::new();
        let mut edges = Vec::new();
        for timeline in &index.timelines {
            let (Some(start), Some(end)) = (timeline.spans.first(), timeline.spans.last()) else {
                continue;
            };
            let (lo, hi) = timeline.hours;
            let (start, end) = (start.hour.max(lo), end.hour.min(hi));
            if start > end {
                continue;
            }
            engine_allocation::reserve(&mut edges, 2)?;
            edges.extend([start, end]);
            let from = timeline.spans.partition_point(|span| span.hour < first.max(lo));
            let to = timeline.spans.partition_point(|span| span.hour <= last.min(hi));
            engine_allocation::reserve(&mut hours, to.saturating_sub(from))?;
            hours.extend(
                timeline.spans[from.min(to)..to]
                    .iter()
                    .map(|span| span.hour),
            );
        }
        edges.sort_unstable();
        let mut seams = edges.get(1..edges.len().saturating_sub(1)).unwrap_or_default();
        hours.sort_unstable();
        let mut covered: Vec<(i32, i32)> = Vec::new();
        for hour in hours {
            while seams.first().is_some_and(|&seam| seam < hour) {
                seams = &seams[1..];
            }
            if seams.first() == Some(&hour) {
                continue;
            }
            match covered.last_mut() {
                Some((_, end)) if i64::from(hour) <= i64::from(*end) + 1 => *end = (*end).max(hour),
                _ => {
                    engine_allocation::reserve(&mut covered, 1)?;
                    covered.push((hour, hour));
                }
            }
        }
        Ok(Self { covered })
    }

    fn observed(&self, from: i32, to: i32) -> bool {
        let at = self.covered.partition_point(|&(_, end)| end < from);
        self.covered
            .get(at)
            .is_some_and(|&(start, end)| start <= from && to <= end)
    }

    pub(super) fn missing_spans(&self, from: i32, to: i32) -> Vec<(i32, i32)> {
        if from > to {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut cursor = i64::from(from);
        let end_all = i64::from(to);
        let start_at = self.covered.partition_point(|&(_, end)| end < from);
        for &(c_first, c_last) in &self.covered[start_at..] {
            if i64::from(c_first) > end_all {
                break;
            }
            if i64::from(c_first) > cursor {
                out.push((cursor as i32, (i64::from(c_first) - 1).min(end_all) as i32));
            }
            cursor = cursor.max(i64::from(c_last) + 1);
            if cursor > end_all {
                break;
            }
        }
        if cursor <= end_all {
            out.push((cursor as i32, end_all as i32));
        }
        out
    }
}

pub(super) fn hourly_amounts(
    index: &Index,
    group: ApmGroup,
    first: i32,
    last: i32,
    tags: Option<&[String]>,
    membership: Option<&[ApmMembership]>,
    players: &[String],
    wanted_ids: &BTreeSet<&str>,
) -> io::Result<BTreeMap<String, BTreeMap<i32, Amount>>> {
    let wanted_tags: Option<BTreeSet<&str>> = tags.map(|list| list.iter().map(String::as_str).collect());
    let wanted_players: BTreeSet<&str> = players.iter().map(String::as_str).collect();
    let mut memberships: BTreeMap<&str, Vec<(i32, i32)>> = BTreeMap::new();
    if let Some(members) = membership {
        for member in members {
            memberships.entry(&member.tag).or_default().push((member.first_hour, member.last_hour));
        }
        for runs in memberships.values_mut() {
            merge_intervals(runs);
        }
    }
    let mut out: BTreeMap<String, BTreeMap<i32, Amount>> = BTreeMap::new();
    for timeline in &index.timelines {
        let (lo, hi) = (first.max(timeline.hours.0), last.min(timeline.hours.1));
        if lo > hi {
            continue;
        }
        let Some((capture_from, capture_to)) = timeline.bounds(lo, hi) else {
            continue;
        };
        let span_from = timeline.spans.partition_point(|span| span.hour < lo);
        let span_to = timeline.spans.partition_point(|span| span.hour <= hi);
        let mut observed_hours: Vec<i32> = timeline.spans[span_from..span_to].iter().map(|s| s.hour).collect();
        observed_hours.sort_unstable();
        observed_hours.dedup();
        if observed_hours.is_empty() {
            continue;
        }
        let mut intervals: BTreeMap<(String, String), Vec<(u64, u64)>> = BTreeMap::new();
        let mut versions: BTreeMap<(u64, usize), (String, String)> = BTreeMap::new();
        for (&actor, history) in &timeline.actors {
            let player = player_id(timeline, actor, history);
            if group == ApmGroup::Player && !wanted_players.is_empty() && !wanted_players.contains(player.as_str()) {
                continue;
            }
            for (version, observation) in history.iter().enumerate() {
                let Some(tag) = tag(&observation.tag) else { continue };
                if wanted_tags.as_ref().is_some_and(|set| !set.contains(tag)) {
                    continue;
                }
                let id = if group == ApmGroup::Country { tag.to_owned() } else { player.clone() };
                if !wanted_ids.contains(id.as_str()) {
                    continue;
                }
                let departed = timeline.departures.get(&actor).copied().unwrap_or(u64::MAX);
                let end = history.get(version + 1).map_or(departed, |next| next.at_qpc).min(departed);
                if end < capture_from || observation.at_qpc > capture_to {
                    continue;
                }
                intervals.entry((id.clone(), tag.to_owned())).or_default().push((observation.at_qpc, end));
                versions.insert((actor, version), (id, tag.to_owned()));
            }
        }
        let member_contains = |tag: &str, h: i32| -> bool {
            match membership {
                None => true,
                Some(_) => memberships.get(tag).is_some_and(|runs| {
                    let at = runs.partition_point(|&(_, end)| end < h);
                    runs.get(at).is_some_and(|&(start, end)| start <= h && h <= end)
                }),
            }
        };
        for ((id, tag), mut ranges) in intervals {
            merge_intervals(&mut ranges);
            let entry = out.entry(id).or_default();
            for &h in &observed_hours {
                if !member_contains(&tag, h) {
                    continue;
                }
                for &(from, to) in &ranges {
                    if let Some(measured) = timeline.measure(h, h, from, to) {
                        let slot = entry.entry(h).or_default();
                        slot.observe(&measured)?;
                        slot.paused_incomplete |= timeline
                            .paused_unknown_hours
                            .range(h..=h)
                            .next()
                            .is_some();
                    }
                }
            }
        }
        for (key, (id, tag)) in &versions {
            let Some(counts) = timeline.counts.get(key) else { continue };
            let Some(hours_map) = out.get_mut(id) else { continue };
            for (&h, amount) in hours_map.iter_mut() {
                if h < lo || h > hi || !member_contains(tag, h) {
                    continue;
                }
                let (count, incomplete) = counts.range(h, h);
                amount.counts.add(count);
                amount.incomplete |= incomplete;
            }
        }
    }
    Ok(out)
}

impl Product for PlayerApm {
    const KEY: &'static str = "player-apm";
    const CONTENT_DEPENDENT: bool = false;
    const INTERACTIVE: bool = true;
    type Params = ApmParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, group, registry, hour: Read, firstHour: Start, lastHour: End, tags, players, buckets, "membership[].firstHour": Start, "membership[].lastHour": End];
    type Reply = ApmReply;

    fn validate(params: &ApmParams) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid branch")?;
        if params.buckets == 0 || params.buckets > 4096 {
            return Err("Invalid APM display resolution");
        }
        if !params.registry
            && params.hour.is_none()
            && !params
                .first_hour
                .zip(params.last_hour)
                .is_some_and(|(first, last)| first <= last)
        {
            return Err("Invalid APM time range");
        }
        if let Some(tags) = &params.tags {
            for tag in tags {
                validate_tag(tag)?;
            }
        }
        if let Some(members) = &params.membership {
            for member in members {
                validate_tag(&member.tag)?;
                if member.first_hour > member.last_hour {
                    return Err("Invalid APM country membership");
                }
            }
        }
        Ok(())
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            ApmParams::decl(),
            ApmReply::decl(),
            ApmGroup::decl(),
            ApmPlayerRun::decl(),
            ApmPlayer::decl(),
            ApmValue::decl(),
            ApmBucket::decl(),
            ApmSeries::decl(),
            ApmMembership::decl(),
        ]
    }

    fn load(
        source: &Source,
        params: &ApmParams,
        _context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<ApmReply> {
        let index = index(source, params.branch_id.parse().unwrap(), cancelled)?;
        query(&index, params, cancelled)
    }
}

pub fn query(index: &Index, params: &ApmParams, cancelled: &AtomicBool) -> io::Result<ApmReply> {
    PlayerApm::validate(params).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let mut reply = ApmReply {
        available: index.analysis.recorded,
        players: Vec::new(),
        series: Vec::new(),
        warnings: Vec::new(),
    };
    if params.registry {
        let mut players: BTreeMap<String, ((u64, u64), Option<String>, Option<String>, BTreeSet<String>, Vec<ApmPlayerRun>)> = BTreeMap::new();
        for timeline in &index.timelines {
            for (&actor, history) in &timeline.actors {
                let tags: Vec<_> = history.iter().filter_map(|actor| tag(&actor.tag)).collect();
                let Some(last) = history.last().filter(|_| !tags.is_empty()) else {
                    continue;
                };
                let entry = players
                    .entry(player_id(timeline, actor, history))
                    .or_default();
                let at = (timeline.session, last.at_qpc);
                if entry.0 <= at {
                    entry.0 = at;
                    match player_name(last) {
                        Ok(name) => {
                            entry.1 = Some(name);
                            entry.2 = None;
                        }
                        Err(reason) => {
                            entry.1 = None;
                            entry.2 = Some(reason);
                        }
                    }
                }
                entry.3.extend(tags.into_iter().map(str::to_owned));
                let departed = timeline.departures.get(&actor).copied().unwrap_or(u64::MAX);
                for (version, observation) in history.iter().enumerate() {
                    let Some(tag_str) = tag(&observation.tag) else {
                        continue;
                    };
                    let end = history
                        .get(version + 1)
                        .map_or(departed, |next| next.at_qpc)
                        .min(departed);
                    let Some(measured) = timeline.measure(timeline.hours.0, timeline.hours.1, observation.at_qpc, end) else {
                        continue;
                    };
                    let (name, name_missing) = match player_name(observation) {
                        Ok(name) => (Some(name), None),
                        Err(reason) => (None, Some(reason)),
                    };
                    entry.4.push(ApmPlayerRun {
                        first_hour: measured.first_hour,
                        last_hour: measured.last_hour,
                        tag: tag_str.to_owned(),
                        name,
                        name_missing,
                    });
                }
            }
        }
        reply.players = players
            .into_iter()
            .map(|(id, (_, name, name_missing, tags, mut runs))| {
                runs.sort_by_key(|run| (run.first_hour, run.last_hour));
                ApmPlayer {
                    id,
                    name,
                    name_missing,
                    tags: tags.into_iter().collect(),
                    runs,
                }
            })
            .collect();
        check_reply_budget(&reply)?;
        return Ok(reply);
    }
    let (first, last) = params.hour.map(|h| (h, h)).or_else(|| params.first_hour.zip(params.last_hour)).unwrap();
    let width = ((i64::from(last) - i64::from(first) + 1) + i64::from(params.buckets) - 1)
        / i64::from(params.buckets);
    let wanted_tags: Option<BTreeSet<_>> = params
        .tags
        .as_ref()
        .map(|tags| tags.iter().map(String::as_str).collect());
    let wanted_players: BTreeSet<_> = params.players.iter().map(String::as_str).collect();
    let mut memberships: BTreeMap<&str, Vec<(i32, i32)>> = BTreeMap::new();
    if let Some(members) = &params.membership {
        for member in members {
            memberships
                .entry(&member.tag)
                .or_default()
                .push((member.first_hour, member.last_hour));
        }
    }
    for runs in memberships.values_mut() {
        merge_intervals(runs);
    }
    let mut series: BTreeMap<String, Series> = BTreeMap::new();
    for timeline in &index.timelines {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let (lo, hi) = (first.max(timeline.hours.0), last.min(timeline.hours.1));
        if lo > hi {
            continue;
        }
        let Some((capture_from, capture_to)) = timeline.bounds(lo, hi) else {
            continue;
        };
        let within = |membership: Vec<(i32, i32)>| -> Vec<(i32, i32)> {
            membership
                .into_iter()
                .map(|(from, to)| (from.max(lo), to.min(hi)))
                .filter(|(from, to)| from <= to)
                .collect()
        };
        let mut intervals: BTreeMap<(String, String), Vec<(u64, u64)>> = BTreeMap::new();
        let mut versions: BTreeMap<(u64, usize), (String, String)> = BTreeMap::new();
        for (&actor, history) in &timeline.actors {
            let player = player_id(timeline, actor, history);
            if params.group == ApmGroup::Player
                && !wanted_players.is_empty()
                && !wanted_players.contains(player.as_str())
            {
                continue;
            }
            for (version, observation) in history.iter().enumerate() {
                let Some(tag) = tag(&observation.tag) else {
                    continue;
                };
                if wanted_tags.as_ref().is_some_and(|tags| !tags.contains(tag)) {
                    continue;
                }
                let end = history
                    .get(version + 1)
                    .map_or_else(
                        || timeline.departures.get(&actor).copied().unwrap_or(u64::MAX),
                        |next| next.at_qpc,
                    )
                    .min(timeline.departures.get(&actor).copied().unwrap_or(u64::MAX));
                if end < capture_from || observation.at_qpc > capture_to {
                    continue;
                }
                let present = if params.membership.is_none() {
                    timeline
                        .measure(first, last, observation.at_qpc, end)
                        .is_some()
                } else {
                    memberships.get(tag).is_some_and(|members| {
                        members.iter().any(|&(member_first, member_last)| {
                            let from = first.max(member_first);
                            let to = last.min(member_last);
                            from <= to
                                && timeline
                                    .measure(from, to, observation.at_qpc, end)
                                    .is_some()
                        })
                    })
                };
                if !present {
                    continue;
                }
                let id = if params.group == ApmGroup::Country {
                    tag.to_owned()
                } else {
                    player.clone()
                };
                let entry = series.entry(id.clone()).or_default();
                let display_at = (timeline.session, observation.at_qpc);
                if entry.display_at <= display_at {
                    if params.group == ApmGroup::Country {
                        entry.name = Some(tag.to_owned());
                        entry.name_missing = None;
                    } else {
                        match player_name(observation) {
                            Ok(name) => {
                                entry.name = Some(name);
                                entry.name_missing = None;
                            }
                            Err(reason) => {
                                entry.name = None;
                                entry.name_missing = Some(reason);
                            }
                        }
                    }
                    entry.tag = Some(tag.to_owned());
                    entry.display_at = display_at;
                }
                intervals
                    .entry((id.clone(), tag.to_owned()))
                    .or_default()
                    .push((observation.at_qpc, end));
                versions.insert((actor, version), (id, tag.to_owned()));
            }
        }
        for ((id, tag), mut ranges) in intervals {
            merge_intervals(&mut ranges);
            let item = series.get_mut(&id).unwrap();
            let membership = within(if params.membership.is_none() {
                vec![(first, last)]
            } else {
                memberships.get(tag.as_str()).cloned().unwrap_or_default()
            });
            for &(member_first, member_last) in &membership {
                let from_hour = first.max(member_first);
                let to_hour = last.min(member_last);
                if from_hour > to_hour {
                    continue;
                }
                for &(from, to) in &ranges {
                    if let Some(measured) = timeline.measure(from_hour, to_hour, from, to) {
                        item.total.observe(&measured)?;
                        item.total.paused_incomplete |= timeline
                            .paused_unknown_hours
                            .range(measured.first_hour..=measured.last_hour)
                            .next()
                            .is_some();
                    }
                }
            }
            for position in 0..=((i64::from(last) - i64::from(first)) / width) {
                let bucket_first = (i64::from(first) + position * width) as i32;
                let bucket_last =
                    (i64::from(first) + (position + 1) * width - 1).min(i64::from(last)) as i32;
                for &(member_first, member_last) in &membership {
                    let from_hour = bucket_first.max(member_first);
                    let to_hour = bucket_last.min(member_last);
                    if from_hour > to_hour {
                        continue;
                    }
                    for &(from, to) in &ranges {
                        let Some(measured) = timeline.measure(from_hour, to_hour, from, to) else {
                            continue;
                        };
                        item.buckets.entry(position).or_default().observe(&measured)?;
                        item.buckets.entry(position).or_default().paused_incomplete |= timeline
                            .paused_unknown_hours
                            .range(measured.first_hour..=measured.last_hour)
                            .next()
                            .is_some();
                    }
                }
            }
        }
        for (key, (id, tag)) in &versions {
            let Some(counts) = timeline.counts.get(key) else {
                continue;
            };
            let membership = within(if params.membership.is_none() {
                vec![(first, last)]
            } else {
                memberships.get(tag.as_str()).cloned().unwrap_or_default()
            });
            let item = series.get_mut(id).unwrap();
            let count_range = |from: i32, to: i32| {
                let mut total = Counts::default();
                let mut unknown = false;
                for &(member_first, member_last) in &membership {
                    let from = from.max(member_first);
                    let to = to.min(member_last);
                    if from <= to {
                        let (count, incomplete) = counts.range(from, to);
                        total.add(count);
                        unknown |= incomplete;
                    }
                }
                (total, unknown)
            };
            let (count, incomplete) = count_range(first, last);
            item.total.counts.add(count);
            item.total.incomplete |= incomplete;
            for position in 0..=((i64::from(last) - i64::from(first)) / width) {
                let from = (i64::from(first) + position * width) as i32;
                let to =
                    (i64::from(first) + (position + 1) * width - 1).min(i64::from(last)) as i32;
                let (count, incomplete) = count_range(from, to);
                if count != Counts::default() || incomplete {
                    let amount = item.buckets.entry(position).or_default();
                    amount.counts.add(count);
                    amount.incomplete |= incomplete;
                }
            }
        }
    }
    let coverage = Coverage::new(index, first, last)?;
    let whole = coverage.observed(first, last);
    let mut ranked: Vec<_> = series.into_iter().filter(|(_, series)| series.total.first_hour.is_some()).collect();
    ranked.sort_by_cached_key(|(_, series)| std::cmp::Reverse(series.total.rate()));
    reply.series = ranked.into_iter()
        .map(|(id, mut series)| {
            series.total.gaps |= !whole;
            ApmSeries {
                id,
                name: series.name,
                name_missing: series.name_missing,
                tag: series.tag,
                hour: series.total.last_hour.unwrap(),
                summary: series.total.value(),
                points: series
                    .buckets
                    .into_iter()
                    .map(|(bucket, mut amount)| {
                        let from = (i64::from(first) + bucket * width) as i32;
                        let to = (i64::from(first) + (bucket + 1) * width - 1).min(i64::from(last))
                            as i32;
                        amount.gaps |= !coverage.observed(from, to);
                        ApmBucket {
                            first_hour: amount.first_hour.unwrap_or(from),
                            last_hour: amount.last_hour.unwrap_or(to),
                            value: amount.value(),
                        }
                    })
                    .collect(),
            }
        })
        .collect();
    check_reply_budget(&reply)?;
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_recording::{command_packet::{Actor, Batch, Category, Event, Kind, Origin, Packet, Span}, control::ControlRecord};

    fn fixture(extra: impl FnOnce(&mut Batch)) -> (replay_data::identity::SessionStart, Batch) {
        let mut session = dashboard_base::test_fixtures::session(7);
        session.qpc_frequency = 1;
        let mut batch = Batch {
            kinds: vec![Kind {
                id: 1,
                token: 1,
                name: "synthetic order".into(),
                category: Category::Army,
            }],
            actors: [1, 2]
                .map(|id| Actor {
                    id,
                    player: id as i32,
                    country: 3,
                    since_qpc: 0,
                    at_qpc: 0,
                    user: Vec::new(),
                    name: b"same name".to_vec(),
                    tag: b"GER".to_vec(),
                })
                .into(),
            events: [(10, 1, false), (20, 2, false), (70, 1, true)]
                .map(|(qpc, actor, paused)| Event {
                    qpc,
                    hour: 100,
                    actor,
                    player: actor as i32,
                    kind: 1,
                    command_country: Some(3),
                    origin: Origin::PlayerOrder,
                    sequence: qpc as u32,
                    paused,
                })
                .into(),
            spans: vec![
                Span {
                    from_qpc: 0,
                    to_qpc: 60,
                    complete: true,
                    hour: 100,
                    paused: false,
                },
                Span {
                    from_qpc: 60,
                    to_qpc: 120,
                    complete: true,
                    hour: 100,
                    paused: true,
                },
                Span {
                    from_qpc: 120,
                    to_qpc: 180,
                    complete: true,
                    hour: 101,
                    paused: false,
                },
            ],
            ..Default::default()
        };
        extra(&mut batch);
        (session, batch)
    }

    fn params(group: ApmGroup, first: i32, last: i32) -> ApmParams {
        ApmParams {
            branch_id: "4".into(),
            group,
            registry: false,
            hour: None,
            first_hour: Some(first),
            last_hour: Some(last),
            tags: None,
            membership: None,
            players: Vec::new(),
            buckets: 2,
        }
    }

    fn packet(batch: &Batch, ordinal: u64, from: u64, to: u64, lost: u64) -> ControlRecord {
        ControlRecord::PlayerCommands {
            session: 7,
            packet: Packet::pack(4, 1, 2, ordinal, from, to, lost, batch).unwrap(),
        }
    }

    #[test]
    fn coop_country_counts_once_and_pauses_never_change_apm() {
        let (session, batch) = fixture(|_| {});
        let index = commands::index(&[session], &[packet(&batch, 0, 0, 180, 0)], 4).unwrap();
        let country = query(
            &index,
            &params(ApmGroup::Country, 100, 101),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(country.series.len(), 1);
        assert_eq!(country.series[0].summary.apm, Some(1.0));
        assert_eq!(country.series[0].summary.unpaused_minutes, Some(2.0));
        assert_eq!(
            country.series[0].summary.paused_orders.as_deref(),
            Some("1")
        );
        let players = query(
            &index,
            &params(ApmGroup::Player, 100, 101),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(players.series.len(), 2);
        assert_ne!(players.series[0].id, players.series[1].id);
        assert!(
            players
                .series
                .iter()
                .all(|series| series.summary.apm == Some(0.5))
        );
    }

    #[test]
    fn a_player_keeps_one_identity_across_reattachments_and_users_stay_distinct() {
        let (session, mut batch) = fixture(|batch| {
            batch.actors[0].user = b"first".to_vec();
            batch.actors[1].user = b"second".to_vec();
        });
        let first = packet(&batch, 0, 0, 180, 0);
        batch.actors[0].name = b"renamed".to_vec();
        for event in &mut batch.events {
            event.qpc += 1000;
        }
        for span in &mut batch.spans {
            span.from_qpc += 1000;
            span.to_qpc += 1000;
        }
        for actor in &mut batch.actors {
            actor.since_qpc = 1000;
            actor.at_qpc = 1000;
        }
        let second = ControlRecord::PlayerCommands {
            session: 7,
            packet: Packet::pack(4, 2, 2, 0, 1000, 1180, 0, &batch).unwrap(),
        };
        let index = commands::index(&[session], &[first, second], 4).unwrap();
        let mut registry = params(ApmGroup::Player, 100, 101);
        registry.registry = true;
        let players = query(&index, &registry, &AtomicBool::new(false))
            .unwrap()
            .players;
        assert_eq!(players.len(), 2);
        assert_eq!(players[0].name.as_deref(), Some("renamed"));
        assert_eq!(players[1].name.as_deref(), Some("same name"));
        let reply = query(
            &index,
            &params(ApmGroup::Player, 100, 101),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(reply.series.len(), 2);
        assert!(
            reply
                .series
                .iter()
                .all(|series| series.summary.apm == Some(0.5)
                    && series.summary.unpaused_minutes == Some(4.0))
        );
    }

    #[test]
    fn future_country_changes_and_paused_ambiguity_do_not_rewrite_history() {
        let (session, batch) = fixture(|batch| {
            batch.events[2].origin = Origin::Unresolved;
            let mut moved = batch.actors[0].clone();
            moved.country = 9;
            moved.tag = b"FRA".to_vec();
            moved.name = b"later name".to_vec();
            moved.at_qpc = 120;
            batch.actors.push(moved);
        });
        let index = commands::index(&[session], &[packet(&batch, 0, 0, 180, 0)], 4).unwrap();
        let reply = query(
            &index,
            &params(ApmGroup::Player, 100, 100),
            &AtomicBool::new(false),
        )
        .unwrap();
        let player = reply
            .series
            .iter()
            .find(|series| series.id.ends_with(":1"))
            .unwrap();
        assert_eq!(player.tag.as_deref(), Some("GER"));
        assert_eq!(player.name.as_deref(), Some("same name"));
        assert_eq!(player.summary.apm, Some(1.0));
        assert_eq!(player.summary.paused_orders, None);
    }

    #[test]
    fn loss_withholds_affected_duration_but_clean_later_ranges_remain_known() {
        let (session, batch) = fixture(|_| {});
        let interval = |from, to, hour| Batch {
            spans: vec![Span {
                from_qpc: from,
                to_qpc: to,
                hour,
                complete: true,
                paused: false,
            }],
            ..Default::default()
        };
        let index = commands::index(
            &[session],
            &[
                packet(&batch, 0, 0, 180, 0),
                packet(&interval(180, 240, 102), 1, 180, 240, 1),
                packet(&interval(240, 300, 103), 2, 240, 300, 0),
            ],
            4,
        )
        .unwrap();
        let bad = query(
            &index,
            &params(ApmGroup::Country, 102, 102),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(bad.series[0].summary.apm, None);
        assert_eq!(bad.series[0].summary.unpaused_minutes, None);
        let good = query(
            &index,
            &params(ApmGroup::Country, 103, 103),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(good.series[0].summary.apm, Some(0.0));
        assert_eq!(good.series[0].summary.unpaused_minutes, Some(1.0));
    }

    #[test]
    fn paused_unknown_issuer_only_withholds_paused_counts() {
        let (session, batch) = fixture(|batch| {
            batch.events[2].actor = 0;
            batch.events[2].player = 99;
            batch.events[2].origin = Origin::Unresolved;
        });
        let index = commands::index(&[session], &[packet(&batch, 0, 0, 180, 0)], 4).unwrap();
        let reply = query(
            &index,
            &params(ApmGroup::Country, 100, 101),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(reply.series[0].summary.apm, Some(1.0));
        assert_eq!(reply.series[0].summary.unpaused_minutes, Some(2.0));
        assert_eq!(reply.series[0].summary.paused_orders, None);
        assert!(
            index
                .analysis
                .players
                .iter()
                .all(|player| player.apm == Some(0.5))
        );
    }

    #[test]
    fn hours_without_command_coverage_are_left_out_and_flagged() {
        let (session, batch) = fixture(|_| {});
        let index = commands::index(&[session], &[packet(&batch, 0, 0, 180, 0)], 4).unwrap();
        let reply = query(
            &index,
            &params(ApmGroup::Country, 100, 103),
            &AtomicBool::new(false),
        )
        .unwrap();
        let series = &reply.series[0];
        assert_eq!(series.summary.apm, Some(1.0));
        assert!(series.summary.complete && series.summary.gaps);
        assert_eq!(series.points[0].value.apm, Some(1.0));
        assert!(!series.points[0].value.gaps);
        assert_eq!(series.points.len(), 1);
    }

    #[test]
    fn a_reattachment_within_an_hour_marks_that_hour_partly_unobserved() {
        let (session, batch) = fixture(|batch| {
            batch.spans.truncate(1);
            batch.events.truncate(2);
        });
        let mut later = batch.clone();
        later.spans[0] = Span {
            from_qpc: 600,
            to_qpc: 660,
            hour: 100,
            complete: true,
            paused: false,
        };
        later.spans.push(Span {
            from_qpc: 660,
            to_qpc: 720,
            hour: 101,
            complete: true,
            paused: false,
        });
        later.events.clear();
        for actor in &mut later.actors {
            actor.since_qpc = 600;
            actor.at_qpc = 600;
        }
        let second = ControlRecord::PlayerCommands {
            session: 7,
            packet: Packet::pack(4, 2, 2, 0, 600, 720, 0, &later).unwrap(),
        };
        let index = commands::index(&[session], &[packet(&batch, 0, 0, 60, 0), second], 4).unwrap();
        let mut params = params(ApmGroup::Country, 100, 101);
        let reply = query(&index, &params, &AtomicBool::new(false)).unwrap();
        let series = &reply.series[0];
        assert!(series.summary.complete && series.summary.gaps);
        assert_eq!(series.summary.unpaused_minutes, Some(3.0));
        params.buckets = 2;
        let reply = query(&index, &params, &AtomicBool::new(false)).unwrap();
        assert!(reply.series[0].points[0].value.gaps);
        assert!(!reply.series[0].points[1].value.gaps);
    }

    #[test]
    fn summary_coverage_does_not_depend_on_display_resolution() {
        let (session, batch) = fixture(|batch| {
            batch.events.pop();
            batch.spans.remove(1);
        });
        let index = commands::index(&[session], &[packet(&batch, 0, 0, 180, 0)], 4).unwrap();
        for buckets in [1, 2, 100] {
            let mut params = params(ApmGroup::Country, 100, 101);
            params.buckets = buckets;
            let reply = query(&index, &params, &AtomicBool::new(false)).unwrap();
            assert_eq!(reply.series[0].summary.apm, None);
            assert_eq!(reply.series[0].summary.unpaused_minutes, None);
        }
    }

    #[test]
    fn known_observer_period_is_excluded_without_becoming_capture_loss() {
        let (session, batch) = fixture(|batch| {
            batch.actors.truncate(1);
            let mut observer = batch.actors[0].clone();
            observer.country = 0;
            observer.tag.clear();
            observer.at_qpc = 60;
            let mut playing = batch.actors[0].clone();
            playing.at_qpc = 120;
            batch.actors.extend([observer, playing]);
            batch
                .events
                .retain(|event| event.actor == 1 && !event.paused);
            let mut order = batch.events[0];
            order.qpc = 130;
            order.hour = 102;
            order.sequence = 130;
            batch.events.push(order);
            batch.spans[1].hour = 101;
            batch.spans[1].paused = false;
            batch.spans[2].hour = 102;
        });
        let index = commands::index(&[session], &[packet(&batch, 0, 0, 180, 0)], 4).unwrap();
        let reply = query(
            &index,
            &params(ApmGroup::Country, 100, 102),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(reply.series[0].summary.apm, Some(1.0));
        assert_eq!(reply.series[0].summary.unpaused_minutes, Some(2.0));
    }

    #[test]
    fn sparse_queries_keep_disjoint_membership_and_resolution_independent_totals() {
        let (session, mut batch) = fixture(|batch| {
            batch.actors.truncate(1);
            batch.events.clear();
            batch.spans.clear();
        });
        for hour in 100..110 {
            let from = (hour - 100) as u64 * 60;
            batch.spans.push(Span {
                from_qpc: from,
                to_qpc: from + 60,
                hour,
                complete: true,
                paused: false,
            });
            for i in 0..hour - 99 {
                batch.events.push(Event {
                    qpc: from + i as u64,
                    hour,
                    actor: 1,
                    player: 1,
                    kind: 1,
                    command_country: Some(3),
                    origin: Origin::PlayerOrder,
                    sequence: from as u32 + i as u32,
                    paused: false,
                });
            }
        }
        let index = commands::index(&[session], &[packet(&batch, 0, 0, 600, 0)], 4).unwrap();
        for buckets in [1, 3, 10, 1000] {
            let mut params = params(ApmGroup::Country, 100, 109);
            params.buckets = buckets;
            params.membership = Some(vec![
                ApmMembership {
                    tag: "GER".into(),
                    first_hour: 101,
                    last_hour: 102,
                },
                ApmMembership {
                    tag: "GER".into(),
                    first_hour: 108,
                    last_hour: 109,
                },
            ]);
            let reply = query(&index, &params, &AtomicBool::new(false)).unwrap();
            let series = &reply.series[0];
            assert_eq!(series.summary.orders.as_deref(), Some("24"));
            assert_eq!(series.summary.unpaused_minutes, Some(4.0));
            assert_eq!(series.summary.apm, Some(6.0));
            assert_eq!(
                series
                    .points
                    .iter()
                    .map(|p| p.value.orders.as_ref().unwrap().parse::<u64>().unwrap())
                    .sum::<u64>(),
                24
            );
        }
    }

    #[test]
    fn exact_recorded_operands_preserve_rational_rate_without_float_drift() {
        let (session, batch) = fixture(|batch| {
            batch.actors.truncate(1);
            batch.spans = vec![Span {
                from_qpc: 0,
                to_qpc: 18,
                complete: true,
                hour: 100,
                paused: false,
            }];
            batch.events = [1, 5, 10]
                .map(|qpc| Event {
                    qpc,
                    hour: 100,
                    actor: 1,
                    player: 1,
                    kind: 1,
                    command_country: Some(3),
                    origin: Origin::PlayerOrder,
                    sequence: qpc as u32,
                    paused: false,
                })
                .into();
        });
        let index = commands::index(&[session], &[packet(&batch, 0, 0, 18, 0)], 4).unwrap();
        let reply = query(&index, &params(ApmGroup::Country, 100, 100), &AtomicBool::new(false)).unwrap();
        let summary = &reply.series[0].summary;
        assert_eq!(summary.orders.as_deref(), Some("3"));
        assert_eq!(summary.confirmed_orders.as_deref(), Some("3"));
        assert_eq!(summary.unpaused_minutes_num.as_deref(), Some("3"));
        assert_eq!(summary.unpaused_minutes_den.as_deref(), Some("10"));
        assert_eq!(summary.apm, Some(3.0 / (3.0 / 10.0)));
        let rate = |orders| Rate { orders, minutes: Minutes { num: u128::MAX - 1, den: u128::MAX } };
        assert!(rate(u64::MAX - 1) < rate(u64::MAX));
    }
}
