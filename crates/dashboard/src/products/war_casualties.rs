//! Requested countries' casualty totals over a branch, sampled without interpolation.
//!
//! AddCasualties@140d04b50 adds to the victim's +0x50/+0x58. Its callers include
//! land manpower (140c5b350), destroyed divisions (140c58750), SinkShip@140d637c0,
//! KillAirplanes@140f4e400 and railway-gun damage: these are not land-only totals.
//! AddUnknownCasualties@140d051d0 has one caller, 140c5b350: a manpower loss with
//! no attacker is copied into every victim relation of the native war type.
//! The chain 140c7fd50 -> 140c5b330 -> 140c5b350 is reached from active damage
//! (140c7bfc0, immediate damage and naval transfers) and paradrop disruption
//! (140c7ae10 <- ApplyParadropDisruption@140c7b090). Ordinary ApplyAttrition
//! (140c5fe80 -> 140c5e9c0) removes equipment, not manpower; unknown is not attrition.
//! GetUnknownCasualties@140d0fc40 feeds the war-side display at 141e64090:
//! sum known plus maximum unknown per country. The diplomacy tooltip builders
//! 140d19cf0/140d1a550 do the same and render CASUALTIES_UNKNOWN separately.
//!
//! We do not mistake that maximum for a lifetime total. Only a zero first baseline
//! and agreeing increments on relations spanning consecutive reads establish an
//! exact unknown total. A gap, nonzero initial baseline, reset, disagreement, or
//! loss of all spanning relations leaves it unknown thereafter. New zero counters
//! cover only their new relations: losses between old and new relations may be missing.
//! WarRelations cadence comes from replay::cadence; reads use metadata::slot_read. Ended and
//! disappeared relations retain their counters inside the scan. No pairs leave it.
//! If a relation present at the last read before a gap is absent afterwards,
//! both endpoint totals remain lower bounds from that sample onwards. A relation
//! already absent before the gap does not acquire this flag. Even uninterrupted
//! hourly reads cannot capture a war's final tick if it ends between reads.
//!
//! Requests name chosen tags, all recorded countries, or a user-selected top N by
//! dealt + known taken. Unknown losses affect display only, never ranking.
//! Per-source branch scans retain sampled country totals and are shared
//! across selections. No per-pair history is retained. A top reply includes every
//! country ever selected, its membership, and runs clipped to it: at most N series cover an hour.
//!
//! Points use the last recorded hour of a game day (hour.rem_euclid(24) == 23),
//! also the first read of a readable stretch, the last readable hour before a
//! gap/unreadable census or identity change,
//! and the branch's last readable hour. Each retains its exact readHour, holding
//! until the next point, never backwards or across a gap. Top membership is
//! ranked from these same daily samples, so ranking and displayed values agree.
//! Numbers are exact i128 sums of recorded i64 counters; nothing is interpolated.
//! Each typed columnar series is counted before adding it to the bounded reply.
//! Refusals measure JSON bytes against MAX_PRODUCT_REPLY_BYTES.
//! Wire series use base values and integer delta columns; country identities are
//! named only when they change, and stretch starts follow gaps or identity changes.
//! Campaign names and CASUALTIES_UNKNOWN use the recorded content catalogue.
//! Naming projections are retained during this same decode, only on change.
//! Labels resolve only for reply tags; full ranking windows
//! are cached once for any requested N. Equal samples merge, with implicit
//! day-end confirmations.
//! A tag changing holder starts a separate wire series, keeping unknown recovery
//! and the monotonic lower-bound flag scoped to its opaque country identity.

use super::{
    LoadContext, MAX_PRODUCT_REPLY_BYTES, Product, country_groups::HourSpan, validate_tag,
};
use crate::{
    Source,
    metadata::{link_slot, mapped_slot, slot_read, validate_campaign},
};
use replay::{
    cadence::{self, Cadence},
    container::Replay,
    frame::FrameDecoder,
    rows::{TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    io,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct WarCasualties;
// Native diplomacy tooltip entry, 140d19cf0/140d1a550.
pub(crate) const UNKNOWN_LABEL: &str = "CASUALTIES_UNKNOWN";

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CasualtyTop {
    pub count: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CasualtyParams {
    pub branch_id: String,
    pub tags: Vec<String>,
    pub top: Option<CasualtyTop>,
}

/// Exact base and safe integer deltas; the client accumulates with BigInt.
/// Exceptionally large deltas stay decimal strings rather than losing precision.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(untagged)]
pub enum CasualtyDelta {
    Small(#[ts(type = "number")] i64),
    Wide(String),
}
#[derive(Clone, Debug, Default, Serialize, TS)]
pub struct CasualtyValues(pub String, pub Vec<CasualtyDelta>);

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CasualtyIdentity {
    pub first_hour: i32,
    pub id: String,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CasualtySeries {
    pub tag: String,
    /// First read followed by positive hour deltas. Equal confirmations are implicit.
    #[ts(type = "Array<number>")]
    pub reads: Vec<i64>,
    /// [point index, hold endpoint, exceptional confirming read]. Holding is not a read.
    pub ends: Vec<(u32, i32, Option<i32>)>,
    pub dealt: CasualtyValues,
    pub taken: CasualtyValues,
    pub unknown: Option<CasualtyValues>,
    /// Unknown values cease to be exact at this hour (null means still exact).
    pub unknown_until: Option<i32>,
    pub at_least_from: Option<i32>,
    pub identities: Vec<CasualtyIdentity>,
    /// Every resolved name over the branch, for one stable campaign label column.
    pub labels: Vec<String>,
    /// Top-N membership; null for chosen tags. Runs are cut to these hours.
    pub membership: Option<Vec<HourSpan>>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CasualtyReply {
    pub branch_id: String,
    pub series: Vec<CasualtySeries>,
    /// Hours where the relation census and endpoint identities were complete.
    pub observed: Vec<HourSpan>,
    pub warnings: Vec<String>,
    pub unknown_label: String,
    pub has_gap: bool,
}

impl Product for WarCasualties {
    const KEY: &'static str = "war-casualties";
    const CONTENT_DEPENDENT: bool = true;
    type Params = CasualtyParams;
    type Reply = CasualtyReply;
    fn types() -> Vec<String> {
        vec![
            CasualtyTop::decl(),
            CasualtyParams::decl(),
            CasualtyDelta::decl(),
            CasualtyValues::decl(),
            CasualtyIdentity::decl(),
            CasualtySeries::decl(),
            CasualtyReply::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        for tag in &params.tags {
            validate_tag(tag)?;
        }
        if let Some(top) = &params.top
            && (!params.tags.is_empty() || top.count == 0)
        {
            return Err("Invalid top countries");
        }
        Ok(())
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        Self::validate(params).map_err(io::Error::other)?;
        check_cancel(cancelled)?;
        let branch = params.branch_id.parse().expect("validated branch");
        let scanned = scanned_branch(source, branch, cancelled)?;
        let mut unknown_label = UNKNOWN_LABEL.to_owned();
        let mut labels = None;
        let mut warnings = Vec::new();
        if let Some(options) = &context.content {
            match crate::catalogue::catalogue(source, options) {
                Ok(catalogue) => {
                    unknown_label = catalogue
                        .localised(UNKNOWN_LABEL, crate::catalogue::View::Plain)
                        .filter(|label| !label.trim().is_empty())
                        .unwrap_or_else(|| UNKNOWN_LABEL.into());
                    labels = Some(branch_labels(
                        source,
                        branch,
                        &catalogue,
                        &scanned,
                        &scanned
                            .memberships(params)
                            .keys()
                            .cloned()
                            .collect::<Vec<_>>(),
                        cancelled,
                    )?);
                    warnings.extend(catalogue.verification.iter().cloned());
                    warnings.extend(catalogue.warnings.iter().cloned());
                    warnings.extend(catalogue.refusal_lines());
                }
                Err(reason) => warnings.push(format!("Casualty labels use their keys: {reason}")),
            }
        } else {
            warnings.push("Game content is unavailable; casualty labels use their keys.".into());
        }
        scanned.reply(
            params,
            cancelled,
            labels.as_deref(),
            &unknown_label,
            warnings,
        )
    }
}

type Identity = (u32, u32);
type PairKey = (usize, usize, i32);

#[derive(Default)]
struct Unknown {
    total: Option<i64>,
    seen: bool,
    // This read's agreement check. Pairs retain their previous counter and read
    // sequence, replacing a separately allocated map for every country/hour.
    present: bool,
    all_zero: bool,
    delta: Option<i64>,
    valid: bool,
    new_max: i64,
}
impl Unknown {
    fn begin(&mut self) {
        self.present = false;
        self.all_zero = true;
        self.delta = None;
        self.valid = true;
        self.new_max = 0;
    }
    fn relation(&mut self, value: i64, previous: Option<i64>) {
        self.present = true;
        self.all_zero &= value == 0;
        if let Some(previous) = previous {
            match value.checked_sub(previous) {
                Some(delta) if delta >= 0 && self.delta.is_none_or(|seen| seen == delta) => {
                    self.delta = Some(delta)
                }
                _ => self.valid = false,
            }
        } else {
            self.valid &= value >= 0;
            self.new_max = self.new_max.max(value);
        }
    }
    fn finish(&mut self) {
        if !self.present {
            return;
        }
        if !self.seen {
            self.total = self.all_zero.then_some(0);
            self.seen = true;
        } else if let Some(total) = self.total {
            self.total = self
                .delta
                .filter(|delta| self.valid && self.new_max <= *delta)
                .and_then(|delta| total.checked_add(delta));
        }
    }
}

#[derive(Default)]
struct Country {
    dealt: i128,
    taken: i128,
    relations: usize,
    unknown: Unknown,
    at_least: bool,
    touched: u64,
}
struct HeldPair {
    values: [i64; 4],
    read: u64,
}

#[derive(Clone, Debug, Default)]
struct Sample {
    first: i32,
    last: i32,
    confirmed: i32,
    identity: Identity,
    dealt: i128,
    taken: i128,
    unknown: Option<i64>,
    at_least: bool,
}
fn daily_confirmation(first: i32, last: i32) -> i32 {
    let last = i64::from(last);
    i64::from(first).max(last - (last + 1).rem_euclid(24)) as i32
}
impl Sample {
    fn exceptional_confirmation(&self) -> Option<i32> {
        (self.confirmed != daily_confirmation(self.first, self.last)).then_some(self.confirmed)
    }
}
// Packed signed integer deltas avoid padding and retain wide country sums.
// Typical daily deltas take 1–3 bytes; no per-day strings or run headers survive.
fn put(bytes: &mut Vec<u8>, value: i128) {
    let mut value = ((value << 1) ^ (value >> 127)) as u128;
    while value >= 128 {
        bytes.push(value as u8 | 128);
        value >>= 7;
    }
    bytes.push(value as u8);
}
fn get(bytes: &mut &[u8]) -> i128 {
    let mut value = 0u128;
    let mut shift = 0;
    loop {
        let byte = bytes[0];
        *bytes = &bytes[1..];
        value |= u128::from(byte & 127) << shift;
        if byte < 128 {
            break;
        }
        shift += 7;
    }
    ((value >> 1) as i128) ^ -((value & 1) as i128)
}
#[derive(Default)]
struct Track {
    packed: Vec<u8>,
    previous: Sample,
    tail: Option<Sample>,
    checkpoints: Vec<Checkpoint>,
    packed_runs: usize,
}
struct Checkpoint {
    first: i32,
    offset: usize,
    previous: Sample,
}
impl Track {
    fn sample(&mut self, hour: i32, identity: Identity, country: &Country) -> bool {
        if let Some(last) = &mut self.tail {
            if last.first == hour {
                return false;
            }
            if last.last >= hour {
                last.last = hour - 1;
            }
            if last.last.checked_add(1) == Some(hour)
                && last.identity == identity
                && last.dealt == country.dealt
                && last.taken == country.taken
                && last.unknown == country.unknown.total
                && last.at_least == country.at_least
            {
                last.last = hour;
                last.confirmed = hour;
                return false;
            }
        }
        self.flush();
        self.tail = Some(Sample {
            first: hour,
            last: hour,
            confirmed: hour,
            identity,
            dealt: country.dealt,
            taken: country.taken,
            unknown: country.unknown.total,
            at_least: country.at_least,
        });
        true
    }
    fn flush(&mut self) {
        let Some(run) = self.tail.take() else {
            return;
        };
        // Bound per-country lookup work without expanding the packed history.
        if self.packed_runs.is_multiple_of(256) {
            self.checkpoints.push(Checkpoint {
                first: run.first,
                offset: self.packed.len(),
                previous: self.previous.clone(),
            });
        }
        self.packed_runs += 1;
        put(
            &mut self.packed,
            i128::from(run.first) - i128::from(self.previous.first),
        );
        put(
            &mut self.packed,
            i128::from(run.last) - i128::from(run.first),
        );
        let changed = run.identity != self.previous.identity;
        self.packed.push(
            u8::from(run.unknown.is_some())
                | (u8::from(run.at_least) << 1)
                | (u8::from(changed) << 2)
                | (u8::from(run.exceptional_confirmation().is_some()) << 3),
        );
        if let Some(confirmed) = run.exceptional_confirmation() {
            put(
                &mut self.packed,
                i128::from(confirmed) - i128::from(run.first),
            );
        }
        if changed {
            put(&mut self.packed, i128::from(run.identity.0));
            put(&mut self.packed, i128::from(run.identity.1));
        }
        put(&mut self.packed, run.dealt - self.previous.dealt);
        put(&mut self.packed, run.taken - self.previous.taken);
        if let Some(value) = run.unknown {
            put(
                &mut self.packed,
                i128::from(value) - i128::from(self.previous.unknown.unwrap_or(0)),
            );
        }
        self.previous = run;
    }
    fn iter(&self) -> impl Iterator<Item = Sample> + '_ {
        self.iter_from(None)
    }
    fn at(&self, hour: i32) -> Option<Sample> {
        if let Some(tail) = &self.tail
            && tail.first <= hour
        {
            return (hour <= tail.last).then(|| tail.clone());
        }
        let before = self
            .checkpoints
            .partition_point(|point| point.first <= hour);
        let checkpoint = before.checked_sub(1).map(|index| &self.checkpoints[index]);
        self.iter_from(checkpoint)
            .take_while(|run| run.first <= hour)
            .find(|run| hour <= run.last)
    }
    fn iter_from(&self, checkpoint: Option<&Checkpoint>) -> impl Iterator<Item = Sample> + '_ {
        let mut bytes = &self.packed[checkpoint.map_or(0, |point| point.offset)..];
        let mut previous = checkpoint.map_or_else(Sample::default, |point| point.previous.clone());
        let mut tail = self.tail.clone();
        std::iter::from_fn(move || {
            if bytes.is_empty() {
                return tail.take();
            }
            let first = (i128::from(previous.first) + get(&mut bytes)) as i32;
            let last = (i128::from(first) + get(&mut bytes)) as i32;
            let flags = bytes[0];
            bytes = &bytes[1..];
            let confirmed = if flags & 8 != 0 {
                (i128::from(first) + get(&mut bytes)) as i32
            } else {
                daily_confirmation(first, last)
            };
            let identity = if flags & 4 != 0 {
                (get(&mut bytes) as u32, get(&mut bytes) as u32)
            } else {
                previous.identity
            };
            let dealt = previous.dealt + get(&mut bytes);
            let taken = previous.taken + get(&mut bytes);
            let unknown = (flags & 1 != 0)
                .then(|| (i128::from(previous.unknown.unwrap_or(0)) + get(&mut bytes)) as i64);
            previous = Sample {
                first,
                last,
                confirmed,
                identity,
                dealt,
                taken,
                unknown,
                at_least: flags & 2 != 0,
            };
            Some(previous.clone())
        })
    }
}

struct Scan {
    identities: Vec<Identity>,
    indices: HashMap<Identity, usize>,
    countries: Vec<Country>,
    pairs: HashMap<PairKey, HeldPair>,
    live: BTreeMap<String, usize>,
    tracks: BTreeMap<String, Track>,
    bounds: BTreeSet<i64>,
    last_bound: Option<i32>,
    active: Vec<String>,
    active_dirty: bool,
    observed: Vec<HourSpan>,
    hour: Option<i32>,
    last_read: Option<i32>,
    read: u64,
    incomplete: bool,
    gap_read: Option<u64>,
    touched: Vec<usize>,
    stretch_start: bool,
    changed_tags: Vec<String>,
    names: Vec<replay::frame::DecodedFrame>,
}
impl Scan {
    fn new() -> Self {
        Self {
            identities: Vec::new(),
            indices: HashMap::new(),
            countries: Vec::new(),
            pairs: HashMap::new(),
            live: BTreeMap::new(),
            tracks: BTreeMap::new(),
            bounds: BTreeSet::new(),
            last_bound: None,
            active: Vec::new(),
            active_dirty: true,
            observed: Vec::new(),
            hour: None,
            last_read: None,
            read: 0,
            incomplete: false,
            gap_read: None,
            touched: Vec::new(),
            stretch_start: false,
            changed_tags: Vec::new(),
            names: Vec::new(),
        }
    }
    fn country(&mut self, identity: Identity) -> usize {
        *self.indices.entry(identity).or_insert_with(|| {
            let index = self.countries.len();
            self.identities.push(identity);
            self.countries.push(Country::default());
            index
        })
    }
    fn sample_tag(&mut self, tag: &str, hour: i32) {
        let index = self.live[tag];
        if self.countries[index].relations == 0 {
            return;
        }
        if self.tracks.entry(tag.into()).or_default().sample(
            hour,
            self.identities[index],
            &self.countries[index],
        ) {
            self.bound(hour);
        }
    }
    fn bound(&mut self, hour: i32) {
        if self.last_bound != Some(hour) {
            self.bounds.insert(i64::from(hour));
            self.last_bound = Some(hour);
        }
    }
    fn sample(&mut self) {
        let Some(hour) = self.last_read else {
            if let Some(endpoint) = self.hour {
                self.hold(endpoint);
            }
            return;
        };
        self.hold(hour);
        let mut changed = false;
        for (tag, &index) in &self.live {
            let country = &self.countries[index];
            if country.relations == 0 {
                continue;
            }
            let track = match self.tracks.get_mut(tag) {
                Some(track) => track,
                None => self.tracks.entry(tag.clone()).or_default(),
            };
            changed |= track.sample(hour, self.identities[index], country);
        }
        if changed {
            self.bound(hour);
        }
        if self.active_dirty {
            self.active.clear();
            self.active.extend(
                self.live
                    .iter()
                    .filter(|(_, index)| self.countries[**index].relations > 0)
                    .map(|(tag, _)| tag.clone()),
            );
            self.active_dirty = false;
        }
        if let Some(endpoint) = self.hour {
            self.hold(endpoint);
        }
    }
    fn hold(&mut self, hour: i32) {
        for tag in &self.active {
            if let Some(last) = &mut self.tracks.get_mut(tag).unwrap().tail {
                last.last = hour;
            }
        }
    }
    fn gap(&mut self) {
        self.sample();
        self.active.clear();
        self.active_dirty = true;
        self.hour = None;
        self.last_read = None;
        self.stretch_start = true;
        self.incomplete = true;
        self.gap_read.get_or_insert(self.read);
        for country in &mut self.countries {
            country.unknown.total = None;
        }
        self.read += 1; // No spanning read can establish an increment across a gap.
    }
    fn begin_hour(&mut self, hour: i32, live: &BTreeMap<String, usize>, changed: bool, read: bool) {
        if self
            .hour
            .is_some_and(|before| before.checked_add(1) != Some(hour))
        {
            self.gap();
        }
        if changed && !read {
            // Close the old identities at their read before accepting a carried
            // census. Returning holders need their own next read, never a backfill.
            self.sample();
            self.last_read = None;
        }
        if changed {
            self.active_dirty = true;
            if let Some(previous_hour) = self.hour {
                self.hold(previous_hour);
                let lost: Vec<_> = self
                    .live
                    .iter()
                    .filter(|(tag, index)| live.get(*tag) != Some(*index))
                    .map(|(tag, _)| tag.clone())
                    .collect();
                let identity_boundary = !lost.is_empty();
                for tag in lost {
                    if let Some(read_hour) = self.last_read {
                        self.sample_tag(&tag, read_hour);
                        if let Some(tail) = self
                            .tracks
                            .get_mut(&tag)
                            .and_then(|track| track.tail.as_mut())
                        {
                            tail.last = previous_hour;
                        }
                    }
                }
                if identity_boundary {
                    self.bound(hour);
                }
                self.active
                    .retain(|tag| live.get(tag) == self.live.get(tag));
            }
            let changed_tags = live
                .iter()
                .filter(|(tag, index)| self.live.get(*tag) != Some(*index))
                .map(|(tag, _)| tag.clone())
                .collect::<Vec<_>>();
            for tag in changed_tags {
                if !self.changed_tags.contains(&tag) {
                    self.changed_tags.push(tag);
                }
            }
            self.live.clone_from(live);
        }
        self.stretch_start |= self.hour.is_none();
        self.hour = Some(hour);
        span_push(&mut self.observed, hour);
        if read {
            self.last_read = Some(hour);
            self.read += 1;
            self.touched.clear();
        }
    }
    fn relation(&mut self, first: usize, second: usize, start: i32, values: [i64; 4]) {
        for index in [first, second] {
            let country = &mut self.countries[index];
            if country.touched != self.read {
                country.touched = self.read;
                country.unknown.begin();
                self.touched.push(index);
            }
        }
        let pair = self.pairs.entry((first, second, start)).or_insert_with(|| {
            self.active_dirty |=
                self.countries[first].relations == 0 || self.countries[second].relations == 0;
            self.countries[first].relations += 1;
            self.countries[second].relations += 1;
            HeldPair {
                values: [0; 4],
                read: 0,
            }
        });
        let spans = pair.read != 0 && pair.read + 1 == self.read;
        let old = pair.values;
        self.countries[first].taken += i128::from(values[0]) - i128::from(old[0]);
        self.countries[first].dealt += i128::from(values[1]) - i128::from(old[1]);
        self.countries[second].taken += i128::from(values[1]) - i128::from(old[1]);
        self.countries[second].dealt += i128::from(values[0]) - i128::from(old[0]);
        self.countries[first]
            .unknown
            .relation(values[2], spans.then_some(old[2]));
        self.countries[second]
            .unknown
            .relation(values[3], spans.then_some(old[3]));
        pair.values = values;
        pair.read = self.read;
    }
    fn end_hour(&mut self, read: bool) {
        if read {
            for &index in &self.touched {
                self.countries[index].unknown.finish();
            }
            if let Some(before_gap) = self.gap_read.take() {
                for (&(first, second, _), pair) in &self.pairs {
                    if pair.read == before_gap {
                        self.countries[first].at_least = true;
                        self.countries[second].at_least = true;
                    }
                }
            }
        }
        if read && (self.stretch_start || self.hour.is_some_and(|hour| hour.rem_euclid(24) == 23)) {
            self.sample();
            self.changed_tags.clear();
        } else if read {
            let changed = std::mem::take(&mut self.changed_tags);
            for tag in changed {
                if !self.live.contains_key(&tag) {
                    continue;
                }
                self.sample_tag(&tag, self.hour.unwrap());
                if self.countries[self.live[&tag]].relations > 0 && !self.active.contains(&tag) {
                    self.active.push(tag);
                }
            }
        }
        if read {
            self.stretch_start = false;
        }
    }
    fn finish(mut self, cancelled: &AtomicBool) -> io::Result<ScannedCasualties> {
        self.sample();
        // Sampling already collected changed-value and identity boundaries.
        // Observation endpoints close held runs without revisiting every track.
        let mut bounds = self.bounds;
        for span in &self.observed {
            bounds.insert(i64::from(span.first_hour));
            bounds.insert(i64::from(span.last_hour) + 1);
        }
        let bounds: Vec<_> = bounds.into_iter().collect();
        let tracks: Vec<_> = self
            .tracks
            .into_iter()
            .map(|(tag, mut track)| {
                track.packed.shrink_to_fit();
                track.checkpoints.shrink_to_fit();
                (tag, track)
            })
            .collect();
        let mut iterators: Vec<_> = tracks.iter().map(|(_, track)| track.iter()).collect();
        let mut current: Vec<_> = iterators.iter_mut().map(Iterator::next).collect();
        let mut windows: Vec<TopWindow> = Vec::new();
        for bounds in bounds.windows(2) {
            check_cancel(cancelled)?;
            let first = bounds[0] as i32;
            let last = (bounds[1] - 1) as i32;
            let mut ranked = Vec::new();
            for (index, run) in current.iter_mut().enumerate() {
                while run.as_ref().is_some_and(|run| run.last < first) {
                    *run = iterators[index].next();
                }
                if let Some(run) = run
                    && run.first <= first
                {
                    ranked.push((run.dealt + run.taken, index as u32));
                }
            }
            ranked.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            if !ranked.is_empty() {
                let order: Vec<_> = ranked.into_iter().map(|(_, index)| index).collect();
                // Values can change daily without changing the full ranking.
                // Merge equal adjacent orders; gaps still close the window.
                if let Some(previous) = windows.last_mut()
                    && previous.last.checked_add(1) == Some(first)
                    && previous.order == order
                {
                    previous.last = last;
                } else {
                    windows.push(TopWindow { first, last, order });
                }
            }
        }
        drop(iterators);
        Ok(ScannedCasualties {
            tracks,
            observed: self.observed,
            incomplete: self.incomplete,
            windows,
            names: self.names,
        })
    }
}
struct TopWindow {
    first: i32,
    last: i32,
    order: Vec<u32>,
}
/// Sampled casualty history shared by widgets through the source's branch cache.
pub struct ScannedCasualties {
    tracks: Vec<(String, Track)>,
    observed: Vec<HourSpan>,
    incomplete: bool,
    windows: Vec<TopWindow>,
    names: Vec<replay::frame::DecodedFrame>,
}

/// Exact recorded totals at a sampled read, before the optional unknown addition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CasualtyMeasure {
    pub read_hour: i32,
    pub dealt: i128,
    /// Known losses; add `unknown` only when that value is available.
    pub taken: i128,
    /// Losses with no recorded attacker; `None` means unrecoverable, never zero.
    pub unknown: Option<i64>,
    /// Dealt and known taken are lower bounds (display with "≥").
    pub at_least: bool,
}
type ScanResult = Result<Arc<ScannedCasualties>, (io::ErrorKind, String)>;
#[derive(Default)]
struct InFlight {
    result: Mutex<Option<ScanResult>>,
    ready: Condvar,
}
impl InFlight {
    fn wait(&self, cancelled: &AtomicBool) -> io::Result<Arc<ScannedCasualties>> {
        let mut result = self
            .result
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        loop {
            check_cancel(cancelled)?;
            if let Some(result) = result.as_ref() {
                return result
                    .clone()
                    .map_err(|(kind, message)| io::Error::new(kind, message));
            }
            result = self
                .ready
                .wait_timeout(result, std::time::Duration::from_millis(20))
                .unwrap_or_else(|error| error.into_inner())
                .0;
        }
    }
}
#[derive(Default)]
struct Scans {
    entries: VecDeque<(u64, Arc<ScannedCasualties>)>,
    in_flight: HashMap<u64, Arc<InFlight>>,
}
/// Get the same branch scan used by the Casualties product.
pub fn scanned_branch(
    source: &Source,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Arc<ScannedCasualties>> {
    check_cancel(cancelled)?;
    let replay = source.replay();
    if !replay.branches.iter().any(|recorded| recorded.id == branch) {
        return Err(io::Error::other("Unknown casualty branch"));
    }
    let state = source.state::<Scans>();
    let (flight, owner) = {
        let mut scans = state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(index) = scans
            .entries
            .iter()
            .position(|(cached, _)| *cached == branch)
        {
            let entry = scans.entries.remove(index).unwrap();
            let scanned = Arc::clone(&entry.1);
            scans.entries.push_back(entry);
            return Ok(scanned);
        }
        if let Some(flight) = scans.in_flight.get(&branch) {
            (Arc::clone(flight), false)
        } else {
            let flight = Arc::new(InFlight::default());
            scans.in_flight.insert(branch, Arc::clone(&flight));
            (flight, true)
        }
    };
    if !owner {
        return flight.wait(cancelled);
    }
    let result = scan_frames(&replay, branch, cancelled)
        .and_then(|scan| scan.finish(cancelled))
        .map(Arc::new);
    {
        let mut scans = state.lock().unwrap_or_else(|error| error.into_inner());
        if let Ok(scanned) = &result {
            scans.entries.push_back((branch, Arc::clone(scanned)));
            while scans.entries.len() > 2 {
                scans.entries.pop_front();
            }
        }
        *flight
            .result
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(
            result
                .as_ref()
                .map(Arc::clone)
                .map_err(|error| (error.kind(), error.to_string())),
        );
        scans.in_flight.remove(&branch);
    }
    flight.ready.notify_all();
    result
}
fn delta(value: i128) -> CasualtyDelta {
    if value.unsigned_abs() <= 9_007_199_254_740_991 {
        CasualtyDelta::Small(value as i64)
    } else {
        CasualtyDelta::Wide(value.to_string())
    }
}
fn check_cancel(cancelled: &AtomicBool) -> io::Result<()> {
    if cancelled.load(Ordering::Acquire) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Casualty request cancelled",
        ))
    } else {
        Ok(())
    }
}
fn json_bytes(value: &impl Serialize) -> io::Result<usize> {
    crate::transport::json_len(value, usize::MAX)
        .ok_or_else(|| io::Error::other("Casualty reply size overflow"))
}
fn check_reply_bytes(bytes: usize) -> io::Result<()> {
    if bytes > MAX_PRODUCT_REPLY_BYTES {
        Err(io::Error::other(format!(
            "Casualty history takes {bytes} bytes, more than one reply holds ({MAX_PRODUCT_REPLY_BYTES} bytes); choose fewer countries."
        )))
    } else {
        Ok(())
    }
}
impl ScannedCasualties {
    /// Whether missing hours could hide entire wars; show the branch-wide caveat.
    pub fn has_gap(&self) -> bool {
        self.incomplete
    }

    /// One country's shared casualties measure, held and dated exactly as in the
    /// product reply. No sample or a recording gap returns `None`, never zero.
    pub fn country_at(&self, tag: &str, hour: i32) -> Option<CasualtyMeasure> {
        let observed = self
            .observed
            .partition_point(|span| span.first_hour <= hour);
        if observed == 0 || self.observed[observed - 1].last_hour < hour {
            return None;
        }
        let index = self
            .tracks
            .binary_search_by(|(name, _)| name.as_str().cmp(tag))
            .ok()?;
        let run = self.tracks[index].1.at(hour)?;
        let read_hour =
            daily_confirmation(run.first, hour.min(run.confirmed)).max(if run.confirmed <= hour {
                run.confirmed
            } else {
                run.first
            });
        Some(CasualtyMeasure {
            read_hour,
            dealt: run.dealt,
            taken: run.taken,
            unknown: run.unknown,
            at_least: run.at_least,
        })
    }

    fn memberships(&self, params: &CasualtyParams) -> BTreeMap<String, Option<Vec<HourSpan>>> {
        let mut selected: BTreeMap<String, Option<Vec<HourSpan>>> = BTreeMap::new();
        if let Some(top) = &params.top {
            for window in &self.windows {
                for &index in window.order.iter().take(top.count as usize) {
                    let spans = selected
                        .entry(self.tracks[index as usize].0.clone())
                        .or_insert_with(|| Some(Vec::new()))
                        .as_mut()
                        .unwrap();
                    if let Some(last) = spans.last_mut()
                        && last.last_hour.checked_add(1) == Some(window.first)
                    {
                        last.last_hour = window.last;
                    } else {
                        spans.push(HourSpan {
                            first_hour: window.first,
                            last_hour: window.last,
                        });
                    }
                }
            }
        } else if params.tags.is_empty() {
            selected.extend(self.tracks.iter().map(|(tag, _)| (tag.clone(), None)));
        } else {
            for tag in &params.tags {
                if self
                    .tracks
                    .binary_search_by(|(name, _)| name.cmp(tag))
                    .is_ok()
                {
                    selected.insert(tag.clone(), None);
                }
            }
        }
        selected
    }
    fn series(
        &self,
        tag: &str,
        membership: Option<Vec<HourSpan>>,
        labels: Option<&CampaignLabels>,
        cancelled: &AtomicBool,
    ) -> io::Result<Vec<CasualtySeries>> {
        let track = &self.tracks[self
            .tracks
            .binary_search_by(|(name, _)| name.as_str().cmp(tag))
            .unwrap()]
        .1;
        let mut runs = track.iter().peekable();
        let mut series = Vec::new();
        while let Some(first) = runs.peek() {
            let identity = first.identity;
            let group = std::iter::from_fn(|| {
                runs.peek().filter(|run| run.identity == identity)?;
                runs.next()
            });
            let mut built = Self::build_series(
                tag,
                membership.clone(),
                labels,
                group,
                &self.observed,
                cancelled,
            )?;
            if built.reads.is_empty() {
                continue;
            }
            if let Some(spans) = &mut built.membership {
                let first = built.reads[0] as i32;
                let last = built.ends.last().unwrap().1;
                spans.retain_mut(|span| {
                    span.first_hour = span.first_hour.max(first);
                    span.last_hour = span.last_hour.min(last);
                    span.first_hour <= span.last_hour
                });
            }
            series.push(built);
        }
        Ok(series)
    }
    fn build_series(
        tag: &str,
        membership: Option<Vec<HourSpan>>,
        labels: Option<&CampaignLabels>,
        runs: impl Iterator<Item = Sample>,
        observed: &[HourSpan],
        cancelled: &AtomicBool,
    ) -> io::Result<CasualtySeries> {
        let spans = membership.as_deref().unwrap_or(observed);
        let mut series = CasualtySeries {
            tag: tag.into(),
            reads: Vec::new(),
            ends: Vec::new(),
            dealt: CasualtyValues::default(),
            taken: CasualtyValues::default(),
            unknown: None,
            unknown_until: None,
            at_least_from: None,
            identities: Vec::new(),
            labels: labels
                .and_then(|l| l.get(tag))
                .cloned()
                .unwrap_or_else(|| vec![tag.into()]),
            membership: None,
        };
        let mut cursor = 0;
        let mut previous: Option<Sample> = None;
        for run in runs {
            check_cancel(cancelled)?;
            while spans
                .get(cursor)
                .is_some_and(|span| span.last_hour < run.first)
            {
                cursor += 1;
            }
            let mut at = cursor;
            while let Some(span) = spans.get(at) {
                if span.first_hour > run.last {
                    break;
                }
                let first = run.first.max(span.first_hour);
                let last = run.last.min(span.last_hour);
                if first <= last {
                    // Keep one point per run even when membership splits it. Its end is
                    // clipped on the client to each membership and observation span.
                    if previous.as_ref().is_none_or(|p| p.first != run.first) {
                        let index = series.reads.len();
                        if let Some(p) = &previous {
                            if i64::from(p.last) != i64::from(run.first) - 1
                                || p.exceptional_confirmation().is_some()
                            {
                                series.ends.push((
                                    (index - 1) as u32,
                                    p.last,
                                    p.exceptional_confirmation(),
                                ));
                            }
                            series.reads.push(i64::from(run.first) - i64::from(p.first));
                            series.dealt.1.push(delta(run.dealt - p.dealt));
                            series.taken.1.push(delta(run.taken - p.taken));
                            if let (Some(values), Some(value)) = (&mut series.unknown, run.unknown)
                            {
                                values.1.push(delta(
                                    i128::from(value) - i128::from(p.unknown.unwrap_or(0)),
                                ));
                            }
                        } else {
                            series.reads.push(i64::from(run.first));
                            series.dealt.0 = run.dealt.to_string();
                            series.taken.0 = run.taken.to_string();
                            series.unknown = run
                                .unknown
                                .map(|value| CasualtyValues(value.to_string(), Vec::new()));
                        }
                        if run.unknown.is_none() {
                            series.unknown_until.get_or_insert(run.first);
                        }
                        if run.at_least {
                            series.at_least_from.get_or_insert(run.first);
                        }
                        if previous.as_ref().is_none_or(|p| p.identity != run.identity) {
                            series.identities.push(CasualtyIdentity {
                                first_hour: run.first,
                                id: format!("{}:{}", run.identity.0, run.identity.1),
                            });
                        }
                        previous = Some(run.clone());
                    }
                }
                at += 1;
            }
        }
        if let Some(last) = previous {
            series.ends.push((
                (series.reads.len() - 1) as u32,
                last.last,
                last.exceptional_confirmation(),
            ));
        }
        series.membership = membership;
        Ok(series)
    }
    fn reply(
        &self,
        params: &CasualtyParams,
        cancelled: &AtomicBool,
        labels: Option<&CampaignLabels>,
        unknown_label: &str,
        mut warnings: Vec<String>,
    ) -> io::Result<CasualtyReply> {
        if self.incomplete {
            warnings
                .push("Casualties are unknown in missing or incomplete recording hours.".into());
        }
        let selected = self.memberships(params);
        for tag in &params.tags {
            if !selected.contains_key(tag) {
                warnings.push(format!("{tag}: no sampled casualties in this branch"));
            }
        }
        let mut reply = CasualtyReply {
            branch_id: params.branch_id.clone(),
            series: Vec::new(),
            observed: self.observed.clone(),
            warnings,
            unknown_label: unknown_label.into(),
            has_gap: self.incomplete,
        };
        let mut bytes = json_bytes(&reply)?;
        check_reply_bytes(bytes)?;
        // Count each actual typed columnar series before adding it to the reply.
        for (tag, membership) in selected {
            for series in self.series(&tag, membership, labels, cancelled)? {
                bytes += json_bytes(&series)? + usize::from(!reply.series.is_empty());
                check_reply_bytes(bytes)?;
                reply.series.push(series);
            }
        }
        Ok(reply)
    }
}

type CampaignLabels = BTreeMap<String, Vec<String>>;
#[derive(Default)]
struct LabelScans {
    entries: VecDeque<(u64, Arc<crate::catalogue::Catalogue>, Arc<CampaignLabels>)>,
}

fn branch_labels(
    source: &Source,
    branch: u64,
    catalogue: &Arc<crate::catalogue::Catalogue>,
    scanned: &ScannedCasualties,
    tags: &[String],
    cancelled: &AtomicBool,
) -> io::Result<Arc<CampaignLabels>> {
    let state = source.state::<LabelScans>();
    let mut cached = state.lock().unwrap_or_else(|error| error.into_inner());
    cached
        .entries
        .retain(|(_, content, _)| Arc::ptr_eq(content, catalogue));
    let mut labels = if let Some(index) = cached.entries.iter().position(|(at, _, _)| *at == branch)
    {
        let (_, _, labels) = cached.entries.remove(index).unwrap();
        if tags.iter().all(|tag| labels.contains_key(tag)) {
            cached
                .entries
                .push_back((branch, Arc::clone(catalogue), Arc::clone(&labels)));
            return Ok(labels);
        }
        Arc::try_unwrap(labels).unwrap_or_else(|labels| (*labels).clone())
    } else {
        CampaignLabels::new()
    };
    let wanted: BTreeSet<_> = tags
        .iter()
        .filter(|tag| !labels.contains_key(*tag))
        .map(String::as_str)
        .collect();
    let mut operands = BTreeMap::new();
    // Snapshots were collected alongside the casualty scan, only when actual
    // naming inputs changed. No second branch decode is needed for any selection.
    for decoded in &scanned.names {
        if wanted.is_empty() {
            break;
        }
        check_cancel(cancelled)?;
        let names = installed_content::country_names::CountryNames::read(decoded, &catalogue.localisation);
        let Ok(countries) = decoded.complete::<tables::Countries>() else {
            continue;
        };
        for country in countries {
            let Ok(tag) = std::str::from_utf8(&country.tag) else {
                continue;
            };
            if !wanted.contains(tag) {
                continue;
            }
            let mut inputs = Vec::new();
            let mut slot = Some(country.canonical_slot);
            let mut seen = BTreeSet::new();
            while let Some(at) = slot
                && seen.insert(at)
            {
                let input = names.operands(at).cloned();
                slot = input
                    .as_ref()
                    .and_then(|input| input.as_ref().ok())
                    .and_then(|input| input.overlord);
                inputs.push((at, input));
            }
            if operands.get(tag) == Some(&inputs) {
                continue;
            }
            operands.insert(tag.to_owned(), inputs);
            let name = names
                .name(country.canonical_slot)
                .and_then(|name| crate::catalogue::plain_text(&name))
                .unwrap_or_else(|reason| {
                    catalogue.refuse(&reason, tag);
                    tag.into()
                });
            let choices = labels.entry(tag.into()).or_default();
            if !choices.contains(&name) {
                choices.push(name);
            }
        }
    }
    let labels = Arc::new(labels);
    cached
        .entries
        .push_back((branch, Arc::clone(catalogue), Arc::clone(&labels)));
    while cached.entries.len() > 2 {
        cached.entries.pop_front();
    }
    Ok(labels)
}

const NAME_TABLES: [TableId; 6] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::Politics,
    TableId::PoliticalParties,
    TableId::Autonomy,
];
fn same_rows<T: replay::rows::TableSchema>(
    a: &replay::frame::DecodedFrame,
    b: &replay::frame::DecodedFrame,
    equal: impl Fn(&T::Row, &T::Row) -> bool,
) -> bool {
    if replay::frame::SharedView::ptr_eq(&a.tables[T::ID.index()], &b.tables[T::ID.index()]) {
        return true;
    }
    match (a.complete::<T>(), b.complete::<T>()) {
        (Ok(a), Ok(b)) => a.len() == b.len() && a.iter().zip(b).all(|(a, b)| equal(a, b)),
        _ => a.tables[T::ID.index()] == b.tables[T::ID.index()],
    }
}
fn same_name_inputs(a: &replay::frame::DecodedFrame, b: &replay::frame::DecodedFrame) -> bool {
    same_rows::<tables::CountryAppearance>(a, b, |a, b| {
        (
            a.country,
            &a.cosmetic_tag,
            &a.name_tag,
            &a.leader_ideology,
            a.overlord,
        ) == (
            b.country,
            &b.cosmetic_tag,
            &b.name_tag,
            &b.leader_ideology,
            b.overlord,
        )
    }) && same_rows::<tables::Politics>(a, b, |a, b| {
        (a.country, a.ruling_party) == (b.country, b.ruling_party)
    }) && same_rows::<tables::PoliticalParties>(a, b, |a, b| {
        (a.country, a.id, &a.token) == (b.country, b.id, &b.token)
    }) && same_rows::<tables::Autonomy>(a, b, |a, b| (a.subject, &a.state) == (b.subject, &b.state))
}
fn span_push(spans: &mut Vec<HourSpan>, hour: i32) {
    if let Some(last) = spans.last_mut()
        && last.last_hour.checked_add(1) == Some(hour)
    {
        last.last_hour = hour;
    } else {
        spans.push(HourSpan {
            first_hour: hour,
            last_hour: hour,
        });
    }
}

fn check_cadence(cadence: Cadence) -> io::Result<()> {
    if cadence != Cadence::Hourly {
        return Err(io::Error::other(
            "Unsupported WarRelations cadence: casualty totals require hourly reads",
        ));
    }
    Ok(())
}
type Census = (Vec<(u32, u32, Vec<u8>)>, replay::frame::SharedView);
const SCAN_TABLES: [TableId; 8] = [
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::WarRelations,
    TableId::CountryAppearance,
    TableId::Politics,
    TableId::PoliticalParties,
    TableId::Autonomy,
];
struct ScanFrames {
    scan: Scan,
    census: Option<Census>,
    live: BTreeMap<String, usize>,
    by_slot: HashMap<u32, usize>,
    seen: HashSet<PairKey>,
    endpoints: Vec<(usize, usize)>,
    relation_keys: Vec<(u32, u32, i32)>,
}
impl ScanFrames {
    fn new(replay: &Replay, branch: u64) -> Self {
        let mut scan = Scan::new();
        scan.incomplete = replay.gaps.iter().any(|gap| gap.branch == branch);
        Self {
            scan,
            census: None,
            live: BTreeMap::new(),
            by_slot: HashMap::new(),
            seen: HashSet::new(),
            endpoints: Vec::new(),
            relation_keys: Vec::new(),
        }
    }
    fn frame(&mut self, decoded: &replay::frame::DecodedFrame, replay: &Replay) {
        if validate_campaign(decoded, replay).is_err() {
            self.scan.gap();
            return;
        }
        let (Ok(countries), Ok(links), Ok(relations)) = (
            decoded.complete::<tables::Countries>(),
            decoded.complete::<tables::CountryLinks>(),
            decoded.complete::<tables::WarRelations>(),
        ) else {
            self.scan.gap();
            return;
        };
        let mut valid = true;
        let changed = self.census.as_ref().is_none_or(|(before, before_links)| {
            before.len() != countries.len()
                || before.iter().zip(countries).any(|((id, slot, tag), row)| {
                    *id != row.id || *slot != row.canonical_slot || *tag != row.tag
                })
                || before_links != &decoded.tables[TableId::CountryLinks.index()]
        });
        if changed {
            self.live.clear();
            self.by_slot.clear();
            for country in countries {
                let (Ok(slot), Ok(tag)) = (
                    mapped_slot(country, links),
                    std::str::from_utf8(&country.tag),
                ) else {
                    valid = false;
                    continue;
                };
                let index = self.scan.country((slot, country.id));
                valid &= self.by_slot.insert(slot, index).is_none()
                    && self.live.insert(tag.to_owned(), index).is_none();
            }
        }
        if !valid {
            self.census = None;
            self.scan.gap();
            return;
        }
        if changed {
            self.census = Some((
                countries
                    .iter()
                    .map(|row| (row.id, row.canonical_slot, row.tag.clone()))
                    .collect(),
                decoded.tables[TableId::CountryLinks.index()].clone(),
            ));
        } else if let Some((_, before_links)) = &mut self.census {
            before_links.clone_from(&decoded.tables[TableId::CountryLinks.index()]);
        }
        if self
            .scan
            .names
            .last()
            .is_none_or(|before| changed || !same_name_inputs(before, decoded))
        {
            let mut snapshot = decoded.clone();
            for table in TableId::ALL {
                if !NAME_TABLES.contains(&table) {
                    snapshot.tables[table.index()] = replay::frame::TableView::Unavailable {
                        reason: "outside name snapshot".into(),
                    }
                    .into();
                }
            }
            self.scan.names.push(snapshot);
        } else if let Some(before) = self.scan.names.last_mut()
            && NAME_TABLES.iter().any(|table| {
                *table != TableId::Countries
                    && !replay::frame::SharedView::ptr_eq(
                        &before.tables[table.index()],
                        &decoded.tables[table.index()],
                    )
            })
        {
            // Equivalent inputs can share the newest table views. Otherwise
            // every hour after a keyframe repeats full row comparisons.
            for table in NAME_TABLES {
                if table != TableId::Countries {
                    before.tables[table.index()].clone_from(&decoded.tables[table.index()]);
                }
            }
        }
        // Counters change hourly; endpoint joins and duplicate checks change
        // only with a relation key or the country/link census.
        let relation_key = |row: &replay::rows::WarRelationRow| {
            (row.first_country, row.second_country, row.start_hour)
        };
        if changed
            || self.relation_keys.len() != relations.len()
            || self
                .relation_keys
                .iter()
                .zip(relations)
                .any(|(key, row)| *key != relation_key(row))
        {
            let endpoint =
                |raw| link_slot(links, raw).and_then(|slot| self.by_slot.get(&slot).copied());
            self.seen.clear();
            self.endpoints.clear();
            for row in relations {
                match (endpoint(row.first_country), endpoint(row.second_country)) {
                    (Some(first), Some(second)) if first != second => {
                        valid &= self.seen.insert((first, second, row.start_hour));
                        self.endpoints.push((first, second));
                    }
                    _ => valid = false,
                }
            }
            self.relation_keys.clear();
            self.relation_keys
                .extend(relations.iter().map(relation_key));
        }
        if !valid {
            self.census = None;
            self.scan.gap();
            return;
        }
        let read = slot_read(&decoded.metrics, TableId::WarRelations, decoded.hour);
        self.scan
            .begin_hour(decoded.hour, &self.live, changed, read);
        if read {
            for (row, &(first, second)) in relations.iter().zip(&self.endpoints) {
                self.scan.relation(
                    first,
                    second,
                    row.start_hour,
                    [
                        row.first_casualties,
                        row.second_casualties,
                        row.first_unknown_casualties,
                        row.second_unknown_casualties,
                    ],
                );
            }
        }
        self.scan.end_hour(read);
    }
}
fn scan_frames(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<Scan> {
    check_cadence(cadence::cadence(TableId::WarRelations))?;
    let mut builder = ScanFrames::new(replay, branch);
    for block in replay.index.iter().filter(|block| block.branch == branch) {
        check_cancel(cancelled)?;
        let mut decoder = FrameDecoder::for_tables(&SCAN_TABLES);
        let Ok(frames) = replay.read_block_selected(block, &SCAN_TABLES) else {
            builder.scan.gap();
            continue;
        };
        for frame in frames {
            check_cancel(cancelled)?;
            let Ok(decoded) = decoder.advance(&frame) else {
                builder.scan.gap();
                break;
            };
            builder.frame(decoded, replay);
        }
    }
    Ok(builder.scan)
}

#[cfg(test)]
mod tests;
