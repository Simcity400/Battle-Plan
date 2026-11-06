//! Requested countries' casualty totals over a branch, sampled without interpolation.
//!
//! Casualty totals add to the victim and include land manpower, destroyed
//! divisions, ships, aircraft and railway-gun damage. Manpower loss without
//! an attacker is copied into every victim relation of the war type. Such loss
//! can come from active damage, immediate damage, naval transfers and paradrop
//! disruption. Ordinary attrition removes equipment rather than manpower;
//! unknown losses are not attrition. War-side and diplomacy displays sum known
//! plus maximum unknown per country and render CASUALTIES_UNKNOWN separately.
//!
//! We do not mistake that maximum for a lifetime total. Only a zero first baseline
//! and agreeing increments on relations spanning consecutive reads establish an
//! exact unknown total. A gap, nonzero initial baseline, reset, disagreement, or
//! loss of all spanning relations leaves it unknown thereafter. New zero counters
//! cover only their new relations: losses between old and new relations may be missing.
//! WarRelations cadence comes from replay_data::cadence; reads use metadata::slot_read. Ended and
//! disappeared relations retain their counters inside the shared scan.
//! If a relation present at the last read before a gap is absent afterwards,
//! both endpoint totals become partial recorded subtotals from that sample onwards.
//! A relation already absent before the gap does not acquire this flag. Even uninterrupted
//! hourly reads cannot capture a war's final tick if it ends between reads.
//!
//! Requests name chosen tags, all recorded countries, or a user-selected top N by
//! dealt + known taken. Unknown losses affect display only, never ranking.
//! Background preparation retains sampled totals for every native holder.
//! Pair windows share its native lifetime runs. The recorded-product scoreboard
//! adapter also consumes the country branch scan. A top reply includes every
//! country ever selected, its membership, and runs clipped to it: at most N series cover an hour.
//!
//! Every hourly read confirms country totals. Equal consecutive reads merge;
//! carried hours keep their supplying read, never backwards or across a gap.
//! Top membership ranks these same hourly values, so ranking and display agree.
//! Numbers are exact i128 sums of recorded i64 counters; nothing is interpolated.
//! Each typed columnar series is counted before adding it to the bounded reply.
//! Refusals measure JSON bytes against MAX_PRODUCT_REPLY_BYTES.
//! Wire series use base values and integer delta columns; country identities are
//! named only when they change, and stretch starts follow gaps or identity changes.
//! Campaign names and CASUALTIES_UNKNOWN use the recorded content catalogue.
//! Naming projections are retained during this same decode, only on change.
//! Labels resolve through the recorded content stack; rank-position runs
//! are prepared once for any requested N. Equal samples merge, with implicit
//! hourly confirmations; repeated numeric deltas compress without losing points.
//! A tag changing holder starts a separate wire series, keeping unknown recovery
//! and the partial-history flag scoped to its opaque country identity.
//! Explicit picks follow all recorded holders of the chosen tags through renames;
//! unrelated holders of an alias are clipped from that projection.

use dashboard_base::source::Source;
use dashboard_base::{
    products::LoadContext, products::MAX_PRODUCT_REPLY_BYTES, products::Product,
    products::country_groups::HourSpan, products::validate_tag,
};
use dashboard_text::metadata::{link_slot, mapped_slot, slot_read, validate_campaign};
use installed_content_country_names::{country_names::CountryNames, country_names::Operands};
use installed_content_localisation::localisation::Localisation;
use replay::container::Replay;
use replay_data::{cadence, cadence::Cadence, rows::TableId, rows::tables};
use replay_frame::{frame::FrameDecoder, frame::SharedView, frame::TableView, frame::ViewIdentity};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct WarCasualties;
pub mod breakdown;
pub mod emit;
pub mod facts;
pub mod pairs;
pub mod prepared;
pub mod served;
// Diplomacy tooltip localisation entry.
pub const UNKNOWN_LABEL: &str = "CASUALTIES_UNKNOWN";

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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(untagged)]
pub enum CasualtyDelta {
    Small(#[ts(type = "number")] i64),
    Wide(String),
    /// [count, exact delta] repeats the same delta without dropping hourly points.
    Repeated(u32, String),
}
#[derive(Clone, Debug, Default, Serialize, TS)]
pub struct CasualtyValues(pub String, pub Vec<CasualtyDelta>);
impl CasualtyValues {
    fn compact(&mut self) {
        let mut deltas = std::mem::take(&mut self.1).into_iter().peekable();
        while let Some(delta) = deltas.next() {
            let mut count = 1;
            while deltas.peek() == Some(&delta) {
                deltas.next();
                count += 1;
            }
            if count >= 4 {
                let value = match delta {
                    CasualtyDelta::Small(value) => value.to_string(),
                    CasualtyDelta::Wide(value) => value,
                    CasualtyDelta::Repeated(_, _) => unreachable!("already compacted"),
                };
                self.1.push(CasualtyDelta::Repeated(count, value));
            } else {
                self.1.extend(std::iter::repeat_n(delta, count as usize));
            }
        }
    }
}

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
    /// [point index, endpoint, last read if held]. Otherwise every hour is confirmed.
    pub ends: Vec<(u32, i32, Option<i32>)>,
    pub dealt: CasualtyValues,
    pub taken: CasualtyValues,
    pub unknown: Option<CasualtyValues>,
    /// Unknown values cease to be exact at this hour (null means still exact).
    pub unknown_until: Option<i32>,
    /// Legacy wire name: partial recorded history begins here, not a lower bound.
    pub at_least_from: Option<i32>,
    pub identities: Vec<CasualtyIdentity>,
    /// Every resolved name over the branch, for one stable campaign label column.
    pub labels: Vec<String>,
    /// Top-N or selected-holder membership; null when the whole tag track applies.
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
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
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
    #[cfg(any(test, feature = "test-support"))]
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
        let mut unknown_label = "Unknown attacker".to_owned();
        let mut labels = None;
        let mut warnings = Vec::new();
        if let Some(options) = &context.content {
            match dashboard_base::catalogue::catalogue(source, options) {
                Ok(catalogue) => {
                    unknown_label = catalogue
                        .localised(UNKNOWN_LABEL, dashboard_text::catalogue::View::Plain)
                        .unwrap_or_else(|| "Unknown attacker".into());
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
                Err(reason) => warnings.push(format!("Casualty titles are unavailable: {reason}")),
            }
        } else {
            warnings.push("Game content is unavailable; casualty titles are unavailable.".into());
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
/// A tag's naming operands (`CountryNames::operands`): its slot's, then each
/// overlord's in turn, the only slots its name reads.
type NameInputs = Vec<(u32, Option<Result<Operands, String>>)>;

#[derive(Default, Serialize, Deserialize)]
struct Unknown {
    total: Option<i64>,
    seen: bool,
    // This read's agreement check. Pairs retain their previous counter and read
    // sequence, replacing a separately allocated map for every country/hour.
    #[serde(skip)]
    present: bool,
    #[serde(skip)]
    all_zero: bool,
    #[serde(skip)]
    delta: Option<i64>,
    #[serde(skip)]
    valid: bool,
    #[serde(skip)]
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
    /// [`Self::relation`] of a spanning relation read again unchanged, for a country already
    /// seen, whose total never reads `all_zero`.
    fn unchanged(&mut self) {
        self.present = true;
        match self.delta {
            None => self.delta = Some(0),
            Some(0) => {}
            Some(_) => self.valid = false,
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
            self.total = self.seen_total(Some(total));
        }
    }
    /// `total` after this read's agreement check, as [`Self::finish`] moves
    /// the total of a country already seen.
    fn seen_total(&self, total: Option<i64>) -> Option<i64> {
        if !self.present {
            return total;
        }
        total.and_then(|total| {
            self.delta
                .filter(|delta| self.valid && self.new_max <= *delta)
                .and_then(|delta| total.checked_add(delta))
        })
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Country {
    dealt: i128,
    taken: i128,
    relations: usize,
    unknown: Unknown,
    at_least: bool,
    touched: u64,
}
#[derive(Serialize, Deserialize)]
struct HeldPair {
    values: [i64; 4],
    read: u64,
    hour: i32,
    /// Dense first-sighting order, indexing per-pair caches.
    ordinal: usize,
    generation: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
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
impl Sample {
    fn measure_at(&self, hour: i32) -> CasualtyMeasure {
        CasualtyMeasure {
            read_hour: hour.min(self.confirmed),
            dealt: self.dealt,
            taken: self.taken,
            unknown: self.unknown,
            at_least: self.at_least,
        }
    }

    fn exceptional_confirmation(&self) -> Option<i32> {
        (self.confirmed != self.last).then_some(self.confirmed)
    }
}
// Packed signed integer deltas avoid padding and retain wide country sums.
// Typical hourly deltas take 1–3 bytes; no per-hour strings or run headers survive.
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
#[derive(Default, Serialize, Deserialize)]
struct Track {
    packed: Vec<u8>,
    previous: Sample,
    tail: Option<Sample>,
    checkpoints: Vec<Checkpoint>,
    packed_runs: usize,
}
#[derive(Serialize, Deserialize)]
struct Checkpoint {
    first: i32,
    offset: usize,
    previous: Sample,
}
impl Track {
    fn sample(&mut self, hour: i32, identity: Identity, country: &Country) -> bool {
        if let Some(last) = &mut self.tail {
            if last.confirmed == hour {
                return false;
            }
            if last.last >= hour {
                last.last = hour - 1;
            }
            if last.last.checked_add(1) == Some(hour)
                && last.confirmed == last.last
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
    #[cfg(any(test, feature = "test-support"))]
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
                last
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

#[derive(Serialize, Deserialize)]
struct Scan {
    identities: Vec<Identity>,
    #[serde(with = "dashboard_core::map_pairs")]
    indices: HashMap<Identity, usize>,
    countries: Vec<Country>,
    #[serde(with = "dashboard_core::map_pairs")]
    pairs: HashMap<PairKey, HeldPair>,
    live: BTreeMap<String, usize>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_sets")]
    holders: BTreeMap<String, HashSet<Identity>>,
    tracks: BTreeMap<String, Track>,
    bounds: BTreeSet<i64>,
    last_bound: Option<i32>,
    active: BTreeSet<String>,
    active_dirty: bool,
    observed: Vec<HourSpan>,
    hour: Option<i32>,
    last_read: Option<i32>,
    read: u64,
    incomplete: bool,
    gap_read: Option<u64>,
    touched: Vec<usize>,
    #[serde(skip)]
    partial_changed: Vec<usize>,
    /// Each tag's naming operands where they change, in snapshot and row order.
    #[serde(skip)]
    names: Vec<(String, NameInputs)>,
    generation: u64,
    pair_frontier: u64,
    pair_hour: i32,
    #[serde(skip)]
    previous_generation: Option<u64>,
}
impl Scan {
    fn new() -> Self {
        Self {
            identities: Vec::new(),
            indices: HashMap::new(),
            countries: Vec::new(),
            pairs: HashMap::new(),
            live: BTreeMap::new(),
            holders: BTreeMap::new(),
            tracks: BTreeMap::new(),
            bounds: BTreeSet::new(),
            last_bound: None,
            active: BTreeSet::new(),
            active_dirty: true,
            observed: Vec::new(),
            hour: None,
            last_read: None,
            read: 0,
            incomplete: false,
            gap_read: None,
            touched: Vec::new(),
            partial_changed: Vec::new(),
            names: Vec::new(),
            generation: 0,
            pair_frontier: 0,
            pair_hour: 0,
            previous_generation: None,
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
    /// Open samples share this scan's accepted and supplying frontiers. Touch
    /// only the affected tags between publication/gap boundaries.
    fn materialize(&mut self, tags: &BTreeSet<String>) {
        for tag in tags {
            self.materialize_tag(tag);
        }
    }
    /// [`Self::materialize`] of one tag.
    fn materialize_tag(&mut self, tag: &str) {
        let Some(hour) = self.hour else {
            return;
        };
        if !self.active.contains(tag) {
            return;
        }
        if let Some(tail) = self
            .tracks
            .get_mut(tag)
            .and_then(|track| track.tail.as_mut())
        {
            tail.last = hour;
            if let Some(read) = self.last_read {
                tail.confirmed = tail.confirmed.max(read);
            }
        }
    }
    fn seal(&mut self) {
        self.materialize(&self.active.clone());
    }
    fn sample_changes(&mut self, tags: &BTreeSet<String>, read: bool) {
        if read {
            let hour = self.last_read.expect("supplying read");
            for tag in tags {
                if self.live.contains_key(tag) {
                    self.sample_tag(tag, hour);
                }
            }
        }
        if self.active_dirty {
            self.active = self
                .live
                .iter()
                .filter(|(_, index)| self.countries[**index].relations > 0)
                .map(|(tag, _)| tag.clone())
                .collect();
            self.active_dirty = false;
        }
        let Some(hour) = self.hour else {
            return;
        };
        for tag in tags {
            if self.active.contains(tag)
                && let Some(tail) = self
                    .tracks
                    .get_mut(tag)
                    .and_then(|track| track.tail.as_mut())
            {
                tail.last = hour;
            }
        }
    }
    fn gap(&mut self) {
        self.seal();
        self.sample();
        self.active.clear();
        self.active_dirty = true;
        self.hour = None;
        self.last_read = None;
        self.incomplete = true;
        self.gap_read.get_or_insert(self.read);
        for country in &mut self.countries {
            country.unknown.total = None;
        }
        self.read += 1; // No spanning read can establish an increment across a gap.
    }
    /// Hours `first..=last`, each an hourly supplying read changing no
    /// census, relation, naming input or read mode after the hour observed
    /// last ([`ScanFrames::quiet_continues`]): what observing each does, in
    /// constant time. Each authenticates and confirms its read, which moves
    /// the read sequence and the relation frontier without touching a pair.
    fn advance_quiet(&mut self, first: i32, last: i32) {
        let hours = u64::try_from(i64::from(last) - i64::from(first) + 1).unwrap_or(0);
        if let Some(span) = self.observed.last_mut()
            && span.last_hour.checked_add(1) == Some(first)
        {
            span.last_hour = last;
        } else {
            self.observed.push(HourSpan {
                first_hour: first,
                last_hour: last,
            });
        }
        self.hour = Some(last);
        self.last_read = Some(last);
        self.read += hours;
        self.touched.clear();
        self.partial_changed.clear();
        self.previous_generation = None;
        self.pair_frontier = self.read;
        self.pair_hour = last;
    }
    /// The relation lifetime `first`-`second` from `start` read `values` at an hour continuing
    /// the last read with the same relations ([`ScanFrames::moved`]): as [`Self::relation`] reads
    /// it, the lifetime staying in the supplying generation, whose last read is the frontier.
    fn relation_moved(
        &mut self,
        first: usize,
        second: usize,
        start: i32,
        values: [i64; 4],
    ) -> io::Result<()> {
        let (first, second, values) = if first < second {
            (first, second, values)
        } else {
            (second, first, [values[1], values[0], values[3], values[2]])
        };
        for index in [first, second] {
            let country = &mut self.countries[index];
            if country.touched != self.read {
                country.touched = self.read;
                country.unknown.begin();
                self.touched.push(index);
            }
        }
        let generation = self.generation;
        let pair = self
            .pairs
            .get_mut(&(first, second, start))
            .filter(|pair| pair.generation == generation)
            .ok_or_else(|| io::Error::other("A moved casualty relation was not read last"))?;
        let spans = self.pair_frontier != 0 && self.pair_frontier + 1 == self.read;
        let old = pair.values;
        pair.values = values;
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
        Ok(())
    }
    #[cfg(test)]
    fn begin_hour(&mut self, hour: i32, live: &BTreeMap<String, usize>, changed: bool, read: bool) {
        self.begin_hour_inputs(hour, live, changed, read, false);
    }
    fn begin_hour_inputs(
        &mut self,
        hour: i32,
        live: &BTreeMap<String, usize>,
        changed: bool,
        read: bool,
        sparse: bool,
    ) {
        if self
            .hour
            .is_some_and(|before| before.checked_add(1) != Some(hour))
        {
            self.gap();
        }
        if changed && !read {
            // Close the old identities at their read before accepting a carried
            // census. Returning holders need their own next read, never a backfill.
            if sparse {
                self.seal();
            } else {
                self.sample();
            }
            self.last_read = None;
        }
        if changed {
            self.active_dirty = true;
            if let Some(previous_hour) = self.hour {
                if !sparse {
                    self.hold(previous_hour);
                }
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
                // Census holders include peaceful countries before their first relation.
                self.holders
                    .entry(tag.clone())
                    .or_default()
                    .insert(self.identities[live[&tag]]);
            }
            self.live.clone_from(live);
        }
        self.hour = Some(hour);
        span_push(&mut self.observed, hour);
        if read {
            self.last_read = Some(hour);
            self.read += 1;
            self.touched.clear();
        }
    }
    fn relation(&mut self, first: usize, second: usize, start: i32, values: [i64; 4]) {
        // Casualties select by victim, independent of endpoint order.
        let (first, second, values) = if first < second {
            (first, second, values)
        } else {
            (second, first, [values[1], values[0], values[3], values[2]])
        };
        for index in [first, second] {
            let country = &mut self.countries[index];
            if country.touched != self.read {
                country.touched = self.read;
                country.unknown.begin();
                self.touched.push(index);
            }
        }
        let ordinal = self.pairs.len();
        let pair = self.pairs.entry((first, second, start)).or_insert_with(|| {
            self.active_dirty |=
                self.countries[first].relations == 0 || self.countries[second].relations == 0;
            self.countries[first].relations += 1;
            self.countries[second].relations += 1;
            HeldPair {
                values: [0; 4],
                read: 0,
                hour: self.hour.expect("relation hour"),
                ordinal,
                generation: u64::MAX,
            }
        });
        let previous = if self.previous_generation == Some(pair.generation) {
            self.pair_frontier
        } else {
            pair.read
        };
        let spans = previous != 0 && previous + 1 == self.read;
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
        pair.hour = self.hour.expect("relation hour");
        if self.previous_generation.is_some() {
            pair.generation = self.generation;
        }
    }
    fn relations_begin(&mut self) {
        self.previous_generation = Some(self.generation);
        self.generation += 1;
    }
    fn relations_advance(&mut self) {
        self.previous_generation = None;
        self.pair_frontier = self.read;
        self.pair_hour = self.hour.expect("relation frontier");
    }
    fn relation_hour(&self, pair: &HeldPair) -> i32 {
        if pair.generation == self.generation {
            self.pair_hour
        } else {
            pair.hour
        }
    }
    #[cfg(test)]
    fn end_hour(&mut self, read: bool) {
        self.end_hour_inputs(read, false);
    }
    fn end_hour_inputs(&mut self, read: bool, sparse: bool) {
        self.partial_changed.clear();
        if read {
            for &index in &self.touched {
                self.countries[index].unknown.finish();
            }
            if let Some(before_gap) = self.gap_read.take() {
                // Key order, not hash order: `touched` is saved state.
                let mut pairs: Vec<_> = self.pairs.iter().collect();
                pairs.sort_unstable_by_key(|(key, _)| **key);
                for (&(first, second, _), pair) in pairs {
                    let read = if self.previous_generation == Some(pair.generation)
                        || (self.previous_generation.is_none()
                            && pair.generation == self.generation)
                    {
                        self.pair_frontier
                    } else {
                        pair.read
                    };
                    if read == before_gap {
                        if !self.countries[first].at_least {
                            self.touched.push(first);
                            self.partial_changed.push(first);
                        }
                        if !self.countries[second].at_least {
                            self.touched.push(second);
                            self.partial_changed.push(second);
                        }
                        self.countries[first].at_least = true;
                        self.countries[second].at_least = true;
                    }
                }
            }
        }
        if read && !sparse {
            self.sample();
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
                // Values can change hourly without changing the full ranking.
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
            #[cfg(any(test, feature = "test-support"))]
            names: self.names,
            holders: self.holders,
            #[cfg(any(test, feature = "test-support"))]
            pairs: pairs::PairHistory::default(),
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
    #[cfg(any(test, feature = "test-support"))]
    names: Vec<(String, NameInputs)>,
    holders: BTreeMap<String, HashSet<Identity>>,
    #[cfg(any(test, feature = "test-support"))]
    pairs: pairs::PairHistory,
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
    /// Legacy name: dealt and known taken have partial recorded history.
    /// Missing corrections can decrease totals, so this establishes no lower bound.
    pub at_least: bool,
}
/// A held value with one supplying read, shared by downstream history products.
/// Gaps and holder changes start a new segment; confirmations remain dated even
/// when the totals have not changed.
#[cfg(any(test, feature = "test-support"))]
pub struct CasualtyMeasureRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub starts_segment: bool,
    pub value: CasualtyMeasure,
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
/// One scan per branch of the source revision, and the scans under way.
#[derive(Default)]
struct Scans {
    entries: BTreeMap<u64, Arc<ScannedCasualties>>,
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
        if let Some(scanned) = scans.entries.get(&branch) {
            return Ok(Arc::clone(scanned));
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
        .and_then(|(scan, pairs)| {
            let finished = scan.finish(cancelled)?;
            #[cfg(any(test, feature = "test-support"))]
            let mut finished = finished;
            #[cfg(any(test, feature = "test-support"))]
            {
                finished.pairs = pairs;
            }
            #[cfg(not(test))]
            let _ = pairs;
            Ok(finished)
        })
        .map(Arc::new);
    {
        let mut scans = state.lock().unwrap_or_else(|error| error.into_inner());
        if let Ok(scanned) = &result {
            scans.entries.insert(branch, Arc::clone(scanned));
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
    dashboard_core::transport::json_len(value, usize::MAX)
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
    // Explicit picks follow native holders through recorded tag renames.
    fn selected_identities(&self, tags: &[String]) -> HashSet<Identity> {
        tags.iter()
            .filter_map(|tag| self.holders.get(tag))
            .flat_map(|holders| holders.iter().copied())
            .collect()
    }

    /// Whether missing hours could hide entire wars; show the branch-wide caveat.
    pub fn has_gap(&self) -> bool {
        self.incomplete
    }

    /// One country's shared casualties measure, held and dated exactly as in the
    /// product reply. No sample or a recording gap returns `None`, never zero.
    #[cfg(any(test, feature = "test-support"))]
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
        Some(run.measure_at(hour))
    }

    /// Walk the same sampled values as `country_at`, without a new replay scan
    /// or expanding the stored history. Only observed coverage is emitted. The caller
    /// can retain compressed totals and construct dates only for returned tags.
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_each_country_run(
        &self,
        tag: &str,
        cancelled: &AtomicBool,
        mut emit: impl FnMut(CasualtyMeasureRun) -> io::Result<()>,
    ) -> io::Result<()> {
        check_cancel(cancelled)?;
        let Ok(index) = self
            .tracks
            .binary_search_by(|(name, _)| name.as_str().cmp(tag))
        else {
            return Ok(());
        };
        let mut observed = 0;
        let mut previous: Option<(i32, Identity)> = None;
        for sample in self.tracks[index].1.iter() {
            check_cancel(cancelled)?;
            while self
                .observed
                .get(observed)
                .is_some_and(|span| span.last_hour < sample.first)
            {
                observed += 1;
            }
            for span in self.observed[observed..]
                .iter()
                .take_while(|span| span.first_hour <= sample.last)
            {
                let mut first = i64::from(sample.first.max(span.first_hour));
                let last = i64::from(sample.last.min(span.last_hour));
                while first <= last {
                    check_cancel(cancelled)?;
                    // Each hourly confirmation supplies its own date. A held
                    // tail keeps its last read without inventing confirmations.
                    let next = if first < i64::from(sample.confirmed) {
                        first + 1
                    } else {
                        last + 1
                    };
                    let end = last.min(next - 1) as i32;
                    let first_hour = first as i32;
                    let starts_segment = previous.is_none_or(|(last, identity)| {
                        last.checked_add(1) != Some(first_hour) || identity != sample.identity
                    });
                    emit(CasualtyMeasureRun {
                        first_hour,
                        last_hour: end,
                        starts_segment,
                        value: sample.measure_at(first_hour),
                    })?;
                    previous = Some((end, sample.identity));
                    // Widen before advancing so a native i32::MAX endpoint is
                    // emitted once and cannot wrap into another interval.
                    first = i64::from(end) + 1;
                }
            }
        }
        Ok(())
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
            let identities = self.selected_identities(&params.tags);
            for (tag, track) in &self.tracks {
                if self
                    .holders
                    .get(tag)
                    .is_none_or(|holders| holders.is_disjoint(&identities))
                {
                    continue;
                }
                let mut spans: Vec<HourSpan> = Vec::new();
                let mut whole_track = true;
                for run in track.iter() {
                    if !identities.contains(&run.identity) {
                        whole_track = false;
                        continue;
                    }
                    if let Some(last) = spans.last_mut()
                        && last.last_hour.checked_add(1) == Some(run.first)
                    {
                        last.last_hour = run.last;
                    } else {
                        spans.push(HourSpan {
                            first_hour: run.first,
                            last_hour: run.last,
                        });
                    }
                }
                if !spans.is_empty() {
                    selected.insert(tag.clone(), (!whole_track).then_some(spans));
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
                .unwrap_or_else(|| vec!["Country name unavailable".into()]),
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
        series.dealt.compact();
        series.taken.compact();
        if let Some(unknown) = &mut series.unknown {
            unknown.compact();
        }
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
        if !params.tags.is_empty() {
            let sampled: HashSet<_> = selected
                .keys()
                .flat_map(|tag| {
                    let index = self
                        .tracks
                        .binary_search_by(|(name, _)| name.cmp(tag))
                        .unwrap();
                    self.tracks[index].1.iter().map(|run| run.identity)
                })
                .collect();
            for tag in &params.tags {
                if !self.holders.get(tag).is_some_and(|holders| {
                    holders.iter().any(|identity| sampled.contains(identity))
                }) {
                    warnings.push(format!("{tag}: no sampled casualties in this branch"));
                }
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
fn country_label(
    catalogue: &dashboard_text::catalogue::Catalogue,
    tag: &str,
    inputs: &NameInputs,
) -> String {
    let operands = inputs
        .iter()
        .filter_map(|(slot, input)| Some((*slot, input.clone()?)))
        .collect();
    CountryNames::of_operands(&catalogue.localisation, operands)
        .name(inputs[0].0)
        .and_then(|name| dashboard_text::catalogue::plain_text(&name))
        .unwrap_or_else(|reason| {
            catalogue.refuse(&reason, tag);
            "Country name unavailable".to_owned()
        })
}
/// Each branch's labels under the catalogue they were named with; another catalogue replaces them.
#[derive(Default)]
#[cfg(any(test, feature = "test-support"))]
struct LabelScans {
    entries: BTreeMap<u64, (Arc<dashboard_text::catalogue::Catalogue>, Arc<CampaignLabels>)>,
}

#[cfg(any(test, feature = "test-support"))]
fn branch_labels(
    source: &Source,
    branch: u64,
    catalogue: &Arc<dashboard_text::catalogue::Catalogue>,
    scanned: &ScannedCasualties,
    tags: &[String],
    cancelled: &AtomicBool,
) -> io::Result<Arc<CampaignLabels>> {
    let state = source.state::<LabelScans>();
    let mut cached = state.lock().unwrap_or_else(|error| error.into_inner());
    cached
        .entries
        .retain(|_, (content, _)| Arc::ptr_eq(content, catalogue));
    let mut labels = if let Some((_, labels)) = cached.entries.remove(&branch) {
        if tags.iter().all(|tag| labels.contains_key(tag)) {
            cached
                .entries
                .insert(branch, (Arc::clone(catalogue), Arc::clone(&labels)));
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
    // The casualty scan kept each tag's naming operands where they changed.
    // No second branch decode is needed for any selection.
    for (tag, inputs) in &scanned.names {
        if wanted.is_empty() {
            break;
        }
        check_cancel(cancelled)?;
        if !wanted.contains(tag.as_str()) {
            continue;
        }
        let name = country_label(catalogue, tag, inputs);
        let choices = labels.entry(tag.clone()).or_default();
        if !choices.contains(&name) {
            choices.push(name);
        }
    }
    let labels = Arc::new(labels);
    cached
        .entries
        .insert(branch, (Arc::clone(catalogue), Arc::clone(&labels)));
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
/// The naming tables the census does not already compare.
const NAME_INPUTS: [TableId; 4] = [
    TableId::CountryAppearance,
    TableId::Politics,
    TableId::PoliticalParties,
    TableId::Autonomy,
];
/// Whether two views of `table` agree on every field `CountryNames::read`
/// reads: complete tables by those fields, other views whole.
pub fn same_name_rows(table: TableId, a: &SharedView, b: &SharedView) -> bool {
    fn rows<T: replay_data::rows::TableSchema>(
        a: &SharedView,
        b: &SharedView,
        equal: impl Fn(&T::Row, &T::Row) -> bool,
    ) -> bool {
        let (TableView::Complete(x), TableView::Complete(y)) = (&**a, &**b) else {
            return a == b;
        };
        match (x.rows::<T>(), y.rows::<T>()) {
            (Some(x), Some(y)) => x.len() == y.len() && x.iter().zip(y).all(|(x, y)| equal(x, y)),
            _ => a == b,
        }
    }
    if SharedView::ptr_eq(a, b) {
        return true;
    }
    match table {
        TableId::Countries => rows::<tables::Countries>(a, b, |a, b| {
            (a.id, a.canonical_slot, &a.tag) == (b.id, b.canonical_slot, &b.tag)
        }),
        TableId::CountryAppearance => rows::<tables::CountryAppearance>(a, b, |a, b| {
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
        }),
        TableId::Politics => rows::<tables::Politics>(a, b, |a, b| {
            (a.country, a.ruling_party) == (b.country, b.ruling_party)
        }),
        TableId::PoliticalParties => rows::<tables::PoliticalParties>(a, b, |a, b| {
            (a.country, a.id, &a.token) == (b.country, b.id, &b.token)
        }),
        TableId::Autonomy => {
            rows::<tables::Autonomy>(a, b, |a, b| (a.subject, &a.state) == (b.subject, &b.state))
        }
        _ => a == b,
    }
}
/// What naming reads of `frame`, as the census and [`same_name_rows`] compare
/// it: equal digests mean continued observation would not name again. The
/// census's Countries fields and whole CountryLinks view, then each naming
/// input's compared fields when complete, else its whole view.
fn name_digest(frame: &replay_frame::frame::DecodedFrame) -> [u8; 16] {
    fn put(hasher: &mut blake3::Hasher, text: &str) {
        hasher.update(&(text.len() as u64).to_le_bytes());
        hasher.update(text.as_bytes());
    }
    fn rows<T: replay_data::rows::TableSchema>(
        hasher: &mut blake3::Hasher,
        view: &SharedView,
        fields: impl Fn(&T::Row) -> String,
    ) {
        if let TableView::Complete(rows) = &**view
            && let Some(rows) = rows.rows::<T>()
        {
            put(hasher, &format!("complete {}", rows.len()));
            for row in rows {
                put(hasher, &fields(row));
            }
        } else {
            put(hasher, &format!("view {:?}", &**view));
        }
    }
    let mut hasher = blake3::Hasher::new();
    let views = &frame.tables;
    rows::<tables::Countries>(&mut hasher, &views[TableId::Countries.index()], |row| {
        format!("{:?}", (row.id, row.canonical_slot, &row.tag))
    });
    put(
        &mut hasher,
        &format!("view {:?}", &*views[TableId::CountryLinks.index()]),
    );
    rows::<tables::CountryAppearance>(
        &mut hasher,
        &views[TableId::CountryAppearance.index()],
        |row| {
            format!(
                "{:?}",
                (
                    row.country,
                    &row.cosmetic_tag,
                    &row.name_tag,
                    &row.leader_ideology,
                    row.overlord,
                )
            )
        },
    );
    rows::<tables::Politics>(&mut hasher, &views[TableId::Politics.index()], |row| {
        format!("{:?}", (row.country, row.ruling_party))
    });
    rows::<tables::PoliticalParties>(
        &mut hasher,
        &views[TableId::PoliticalParties.index()],
        |row| format!("{:?}", (row.country, row.id, &row.token)),
    );
    rows::<tables::Autonomy>(&mut hasher, &views[TableId::Autonomy.index()], |row| {
        format!("{:?}", (row.subject, &row.state))
    });
    let mut digest = [0; 16];
    digest.copy_from_slice(&hasher.finalize().as_bytes()[..16]);
    digest
}
/// `seen` names the last compared hour's views, which already agreed with `a`.
fn same_name_inputs(
    a: &replay_frame::frame::DecodedFrame,
    b: &replay_frame::frame::DecodedFrame,
    seen: &[ViewIdentity],
) -> bool {
    NAME_INPUTS.iter().enumerate().all(|(index, table)| {
        let view = &b.tables[table.index()];
        seen.get(index).is_some_and(|seen| seen.is(view))
            || same_name_rows(*table, &a.tables[table.index()], view)
    })
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
        return Err(dashboard_core::refused(
            "Unsupported WarRelations cadence: casualty totals require hourly reads",
        ));
    }
    Ok(())
}
type Census = (Vec<(u32, u32, Vec<u8>)>, replay_frame::frame::SharedView);
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
#[derive(Serialize, Deserialize)]
struct ScanFrames {
    scan: Scan,
    #[serde(skip)]
    census: Option<Census>,
    #[serde(skip)]
    live: BTreeMap<String, usize>,
    #[serde(skip)]
    by_slot: HashMap<u32, usize>,
    #[serde(skip)]
    seen: HashSet<PairKey>,
    #[serde(skip)]
    endpoints: Vec<(usize, usize)>,
    #[serde(skip)]
    relation_keys: Vec<(u32, u32, i32)>,
    #[serde(skip)]
    current_tags: Vec<Option<String>>,
    /// The pairs the last processed read supplied. A restored builder has none
    /// until its first processed read: those pairs carry the scan's generation.
    #[serde(skip)]
    supplying_pairs: Option<BTreeSet<PairKey>>,
    #[serde(skip)]
    supplying_keys_current: bool,
    #[serde(skip)]
    changed_pairs: BTreeSet<PairKey>,
    #[serde(skip)]
    changed_endpoints: BTreeSet<usize>,
    #[serde(skip)]
    changed_tags: BTreeSet<String>,
    #[serde(skip)]
    pending_relations: bool,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    work: dashboard_base::prepared::observer_tests::Work,
    /// The newest naming snapshot, and each tag's latest entry in `scan.names`.
    #[serde(skip)]
    name_snapshot: Option<replay_frame::frame::DecodedFrame>,
    #[serde(skip)]
    name_entries: HashMap<String, usize>,
    /// The last compared hour's NAME_INPUTS views, agreeing with the newest snapshot.
    #[serde(skip)]
    name_views: Vec<ViewIdentity>,
    /// [`name_digest`] of the newest snapshot, which a restored builder keeps
    /// in its place: it names again only where continued observation would,
    /// since operand reasons carry the hour they were read.
    #[serde(default)]
    name_digest: Option<[u8; 16]>,
    /// Each country's relations in the rows last joined ([`Self::endpoints`]), by index.
    #[serde(skip)]
    degrees: Vec<u32>,
    /// Each country's relations [`Self::moved`] read this hour, by index; zero after it.
    #[serde(skip)]
    moved_count: Vec<u32>,
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
            current_tags: Vec::new(),
            supplying_pairs: Some(BTreeSet::new()),
            supplying_keys_current: true,
            changed_pairs: BTreeSet::new(),
            changed_endpoints: BTreeSet::new(),
            changed_tags: BTreeSet::new(),
            pending_relations: true,
            #[cfg(any(test, feature = "test-support"))]
            work: Default::default(),
            name_snapshot: None,
            name_entries: HashMap::new(),
            name_views: Vec::new(),
            name_digest: None,
            degrees: Vec::new(),
            moved_count: Vec::new(),
        }
    }
    /// Appends each tag whose naming operands differ from its latest entry. The
    /// operands read no localisation; labels resolve them with the content's.
    fn names(&mut self, snapshot: &replay_frame::frame::DecodedFrame) {
        let localisation = Localisation::default();
        let names = CountryNames::read(snapshot, &localisation);
        let Ok(countries) = snapshot.complete::<tables::Countries>() else {
            return;
        };
        for country in countries {
            let Ok(tag) = std::str::from_utf8(&country.tag) else {
                continue;
            };
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
            if self
                .name_entries
                .get(tag)
                .is_some_and(|&entry| self.scan.names[entry].1 == inputs)
            {
                continue;
            }
            self.name_entries
                .insert(tag.to_owned(), self.scan.names.len());
            self.scan.names.push((tag.to_owned(), inputs));
        }
    }
    /// Drops an invalid census. Continued observation names again at the next
    /// valid read, so a restored builder must too.
    fn forget_census(&mut self) {
        self.census = None;
        self.name_digest = None;
    }
    /// Whether hours from `first` continue the hour observed last, a
    /// supplying read with a valid census and naming snapshot and no
    /// relations, active set or gap recovery pending: then an hour at which
    /// no field this scan reads changed, its relations were not decoded
    /// again and WarRelations was read changes nothing but the scan's
    /// frontiers ([`Self::advance_quiet`]).
    fn quiet_continues(&self, first: i32) -> bool {
        let scan = &self.scan;
        scan.hour.and_then(|hour| hour.checked_add(1)) == Some(first)
            && scan.last_read == scan.hour
            && scan.gap_read.is_none()
            && !scan.active_dirty
            && !self.pending_relations
            && self.census.is_some()
            && self.name_snapshot.is_some()
    }
    /// Hours `first..=last` that [`Self::quiet_continues`] allows.
    fn advance_quiet(&mut self, first: i32, last: i32) {
        self.changed_tags.clear();
        self.changed_pairs.clear();
        self.changed_endpoints.clear();
        self.scan.advance_quiet(first, last);
    }
    fn frame(&mut self, decoded: &replay_frame::frame::DecodedFrame, replay: &Replay) {
        self.frame_inputs(decoded, replay, None);
    }
    fn frame_changes(
        &mut self,
        decoded: &replay_frame::frame::DecodedFrame,
        replay: &Replay,
        changes: &dashboard_base::prepared::frame_changes::FrameChanges,
    ) {
        self.frame_inputs(decoded, replay, Some(changes));
    }
    fn frame_inputs(
        &mut self,
        decoded: &replay_frame::frame::DecodedFrame,
        replay: &Replay,
        changes: Option<&dashboard_base::prepared::frame_changes::FrameChanges>,
    ) {
        let sparse = changes.is_some();
        self.changed_tags.clear();
        self.changed_pairs.clear();
        self.changed_endpoints.clear();
        if validate_campaign(decoded, replay).is_err() {
            self.changed_tags.extend(self.scan.active.iter().cloned());
            self.scan.gap();
            return;
        }
        let (Ok(countries), Ok(links), Ok(relations)) = (
            decoded.complete::<tables::Countries>(),
            decoded.complete::<tables::CountryLinks>(),
            decoded.complete::<tables::WarRelations>(),
        ) else {
            self.changed_tags.extend(self.scan.active.iter().cloned());
            self.scan.gap();
            return;
        };
        let mut valid = true;
        let census_changed = self.census.is_none()
            || changes.is_none_or(|changes| {
                [TableId::Countries, TableId::CountryLinks]
                    .iter()
                    .any(|table| changes.rows_changed[table.index()])
            });
        let changed = census_changed
            && self.census.as_ref().is_none_or(|(before, before_links)| {
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
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.work.row_visits += 1;
                }
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
            self.changed_tags.extend(self.scan.active.iter().cloned());
            self.forget_census();
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
        if changed || self.current_tags.is_empty() {
            self.current_tags = vec![None; self.scan.identities.len()];
            for (tag, &index) in &self.live {
                self.current_tags[index] = Some(tag.clone());
            }
        }
        let names_changed = changes.is_none_or(|changes| {
            NAME_INPUTS
                .iter()
                .any(|table| changes.rows_changed[table.index()])
        });
        let rename = match &self.name_snapshot {
            Some(before) => {
                changed || (names_changed && !same_name_inputs(before, decoded, &self.name_views))
            }
            // A restored builder has no snapshot. Continued observation names
            // again only when the census or a naming input differs from its
            // snapshot; re-reading anyway would record reasons for this hour.
            None => self
                .name_digest
                .is_none_or(|saved| saved != name_digest(decoded)),
        };
        if rename || self.name_snapshot.is_none() {
            let mut snapshot = decoded.clone();
            for table in TableId::ALL {
                if !NAME_TABLES.contains(&table) {
                    snapshot.tables[table.index()] = replay_frame::frame::TableView::Unavailable {
                        reason: "outside name snapshot".into(),
                    }
                    .into();
                }
            }
            if rename {
                self.names(&snapshot);
                self.name_digest = Some(name_digest(&snapshot));
            }
            self.name_snapshot = Some(snapshot);
        }
        // Identities spare unchanged views a row comparison without holding
        // them, which would copy every naming table the next hour changes.
        self.name_views = NAME_INPUTS
            .iter()
            .map(|table| decoded.tables[table.index()].identity())
            .collect();
        // Counters change hourly; endpoint joins and duplicate checks change
        // only with a relation key or the country/link census.
        let relation_key = |row: &replay_data::rows::WarRelationRow| {
            (row.first_country, row.second_country, row.start_hour)
        };
        if changed
            || (changes.is_none_or(|changes| changes.rows_changed[TableId::WarRelations.index()])
                && (self.relation_keys.len() != relations.len()
                    || self
                        .relation_keys
                        .iter()
                        .zip(relations)
                        .any(|(key, row)| *key != relation_key(row))))
        {
            // Keep the previous supplying set until the next processed read,
            // including when the new keys first appear on a non-read hour.
            self.supplying_keys_current = false;
            let endpoint =
                |raw| link_slot(links, raw).and_then(|slot| self.by_slot.get(&slot).copied());
            self.seen.clear();
            self.endpoints.clear();
            for row in relations {
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.work.row_visits += 1;
                }
                match (endpoint(row.first_country), endpoint(row.second_country)) {
                    (Some(first), Some(second)) if first != second => {
                        valid &= self.seen.insert((
                            first.min(second),
                            first.max(second),
                            row.start_hour,
                        ));
                        self.endpoints.push((first, second));
                    }
                    _ => valid = false,
                }
            }
            self.relation_keys.clear();
            self.relation_keys
                .extend(relations.iter().map(relation_key));
            self.degrees.clear();
            self.degrees.resize(self.scan.identities.len(), 0);
            for &(first, second) in &self.endpoints {
                self.degrees[first] += 1;
                self.degrees[second] += 1;
            }
        }
        if !valid {
            self.changed_tags.extend(self.scan.active.iter().cloned());
            self.forget_census();
            self.scan.gap();
            return;
        }
        let read = slot_read(
            replay.format,
            &decoded.metrics,
            TableId::WarRelations,
            decoded.hour,
        );
        self.pending_relations |= !sparse
            || changed
            || changes.is_some_and(|changes| changes.rows_changed[TableId::WarRelations.index()])
            || self
                .scan
                .hour
                .is_none_or(|hour| hour.checked_add(1) != Some(decoded.hour));
        let process = read && self.pending_relations;
        // A resumed observer rebuilds its lookup tables from this frame. Only
        // a changed native census closes continuity on a carried read.
        let holders_changed = changed && self.scan.live != self.live;
        if holders_changed {
            for (tag, &index) in &self.scan.live {
                if self.live.get(tag) != Some(&index) {
                    self.changed_tags.insert(tag.clone());
                    self.changed_endpoints.insert(index);
                }
            }
            for (tag, &index) in &self.live {
                if self.scan.live.get(tag) != Some(&index) {
                    self.changed_tags.insert(tag.clone());
                    self.changed_endpoints.insert(index);
                }
            }
        }
        // A changed supplying mode changes every active observation's proof.
        // Equal mode merely advances the scan's single frontier.
        if self
            .scan
            .hour
            .is_none_or(|hour| hour.checked_add(1) != Some(decoded.hour))
            || self
                .scan
                .hour
                .is_some_and(|hour| self.scan.last_read == Some(hour))
                != read
            || (holders_changed && !read)
        {
            self.changed_tags.extend(self.scan.active.iter().cloned());
        }
        let mut retired = BTreeSet::new();
        if process {
            for &(first, second) in &self.endpoints {
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.work.row_visits += 1;
                }
                for index in [first, second] {
                    if let Some(tag) = &self.current_tags[index]
                        && !self.changed_tags.contains(tag)
                    {
                        self.changed_tags.insert(tag.clone());
                    }
                }
            }
            if !self.supplying_keys_current || self.supplying_pairs.is_none() {
                let supplying: BTreeSet<_> = relations
                    .iter()
                    .zip(&self.endpoints)
                    .map(|(row, &(first, second))| {
                        (first.min(second), first.max(second), row.start_hour)
                    })
                    .collect();
                let generation = self.scan.generation;
                let previous = self.supplying_pairs.take().unwrap_or_else(|| {
                    self.scan
                        .pairs
                        .iter()
                        .filter(|(_, pair)| pair.generation == generation)
                        .map(|(&key, _)| key)
                        .collect()
                });
                retired.extend(previous.difference(&supplying).copied());
                self.supplying_pairs = Some(supplying);
                self.supplying_keys_current = true;
            }
            self.changed_pairs.extend(
                self.supplying_pairs
                    .as_ref()
                    .expect("supplying relation keys")
                    .iter()
                    .copied(),
            );
            self.changed_pairs.extend(retired.iter().copied());
        }
        if self.scan.gap_read.is_some() {
            self.changed_tags.extend(self.scan.active.iter().cloned());
        }
        if sparse {
            self.scan.materialize(&self.changed_tags);
        }
        self.scan
            .begin_hour_inputs(decoded.hour, &self.live, holders_changed, read, sparse);
        if process {
            if sparse {
                for key in retired {
                    if let Some(pair) = self.scan.pairs.get_mut(&key) {
                        pair.read = self.scan.pair_frontier;
                        pair.hour = self.scan.pair_hour;
                        pair.generation = u64::MAX;
                    }
                }
                self.scan.relations_begin();
            }
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
        self.scan.end_hour_inputs(read, sparse);
        if read && sparse {
            self.scan.relations_advance();
            self.pending_relations = false;
        }
        if sparse {
            // Lost-in-gap partial flags also affect every lifetime of that holder.
            for &index in &self.scan.partial_changed {
                self.changed_endpoints.insert(index);
                if let Some(tag) = &self.current_tags[index] {
                    self.changed_tags.insert(tag.clone());
                }
            }
            self.scan.sample_changes(&self.changed_tags, read);
        }
    }
    /// An hour continuing the hour observed last as an hourly supplying read, with the same
    /// census and relation lifetimes and no gap to recover: what [`Self::frame_changes`] does
    /// there, reading only the relations at `moved` (their places in the rows joined last, and
    /// their counters as those rows order them) and the countries they join. Every other
    /// relation was read again unchanged. `names` holds the naming tables' views where a naming
    /// input was written.
    fn moved(
        &mut self,
        hour: i32,
        moved: &[(u32, [i64; 4])],
        names: Option<(Vec<(TableId, SharedView)>, replay_frame::frame::Metrics)>,
    ) -> io::Result<()> {
        self.changed_tags.clear();
        self.changed_pairs.clear();
        self.changed_endpoints.clear();
        if !self.quiet_continues(hour)
            || !self.supplying_keys_current
            || self.supplying_pairs.is_none()
        {
            return Err(io::Error::other(
                "A casualty hour read by its changes does not continue the hour before",
            ));
        }
        if let Some((views, metrics)) = names
            && let Some(before) = &self.name_snapshot
            && !views.iter().all(|(table, view)| {
                !NAME_INPUTS.contains(table)
                    || same_name_rows(*table, &before.tables[table.index()], view)
            })
        {
            let mut snapshot = before.clone();
            snapshot.hour = hour;
            snapshot.metrics = metrics;
            for (table, view) in views {
                snapshot.tables[table.index()] = view;
            }
            self.names(&snapshot);
            self.name_digest = Some(name_digest(&snapshot));
            self.name_snapshot = Some(snapshot);
        }
        self.name_views.clear();
        for &(place, _) in moved {
            let (Some(&(first, second)), Some(&(_, _, start))) = (
                self.endpoints.get(place as usize),
                self.relation_keys.get(place as usize),
            ) else {
                return Err(io::Error::other("A moved casualty relation was not joined"));
            };
            for index in [first, second] {
                if let Some(Some(tag)) = self.current_tags.get(index) {
                    self.changed_tags.insert(tag.clone());
                }
            }
            self.changed_pairs
                .insert((first.min(second), first.max(second), start));
        }
        self.scan.materialize(&self.changed_tags);
        self.scan
            .begin_hour_inputs(hour, &self.live, false, true, true);
        self.moved_count.resize(self.scan.identities.len(), 0);
        for &(place, values) in moved {
            let (first, second) = self.endpoints[place as usize];
            self.moved_count[first] += 1;
            self.moved_count[second] += 1;
            let start = self.relation_keys[place as usize].2;
            self.scan.relation_moved(first, second, start, values)?;
        }
        // A country's relations read again unchanged agree on no unknown losses.
        for &index in &self.scan.touched {
            if self.degrees.get(index).copied().unwrap_or(0) > self.moved_count[index] {
                self.scan.countries[index].unknown.unchanged();
            }
            self.moved_count[index] = 0;
        }
        self.scan.end_hour_inputs(true, true);
        self.scan.relations_advance();
        self.scan.sample_changes(&self.changed_tags, true);
        Ok(())
    }
}

/// Whether [`ScanFrames::frame_inputs`] joins `frame` rather than taking it as a gap, in a
/// recording of `campaign`: its campaign, complete countries, links and relations, each country
/// mapped by its links to a slot and a tag of its own, and each relation between two of them,
/// each lifetime once. A function of the frame alone, as the scan's joins are.
pub(super) fn joins(frame: &replay_frame::frame::DecodedFrame, campaign: &[u8]) -> bool {
    if let TableView::Complete(rows) = &*frame.tables[TableId::Campaign.index()]
        && rows
            .rows::<tables::Campaign>()
            .is_none_or(|rows| rows.len() != 1 || rows[0].id != campaign)
    {
        return false;
    }
    let (Ok(countries), Ok(links), Ok(relations)) = (
        frame.complete::<tables::Countries>(),
        frame.complete::<tables::CountryLinks>(),
        frame.complete::<tables::WarRelations>(),
    ) else {
        return false;
    };
    let slot = |raw: u32| {
        links
            .binary_search_by_key(&raw, |link| link.id)
            .ok()
            .map(|at| links[at].canonical_slot)
    };
    let mut slots = HashSet::with_capacity(countries.len());
    let mut tags = HashSet::with_capacity(countries.len());
    for country in countries {
        if slot(country.id) != Some(country.canonical_slot)
            || std::str::from_utf8(&country.tag).is_err()
            || !slots.insert(country.canonical_slot)
            || !tags.insert(country.tag.as_slice())
        {
            return false;
        }
    }
    let endpoint = |raw: u32| slot(raw).filter(|slot| slots.contains(slot));
    let mut lifetimes = HashSet::with_capacity(relations.len());
    relations.iter().all(
        |row| match (endpoint(row.first_country), endpoint(row.second_country)) {
            (Some(first), Some(second)) if first != second => {
                lifetimes.insert((first.min(second), first.max(second), row.start_hour))
            }
            _ => false,
        },
    )
}
fn scan_frames(
    replay: &Replay,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<(Scan, pairs::PairHistory)> {
    let pairs = pairs::PairHistory::default();
    #[cfg(any(test, feature = "test-support"))]
    let mut pairs = pairs;
    check_cadence(cadence::cadence_in(TableId::WarRelations, replay.format))?;
    let mut builder = ScanFrames::new(replay, branch);
    dashboard_core::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &SCAN_TABLES,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                check_cancel(cancelled)?;
                let mut decoder = FrameDecoder::for_tables(&SCAN_TABLES);
                let Ok(frames) = frames else {
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
                    #[cfg(any(test, feature = "test-support"))]
                    pairs.observe(&builder.scan);
                }
            }
            Ok(())
        },
    )?;
    #[cfg(any(test, feature = "test-support"))]
    pairs.shrink_to_fit();
    Ok((builder.scan, pairs))
}

#[cfg(test)]
impl Scan {
    /// What a publication reads of the scan, its open samples sealed: two scans publishing alike
    /// hold the same, however lazily they extended their runs or numbered their generations.
    fn parity(&self) -> serde_json::Value {
        let tracks: BTreeMap<&String, Vec<serde_json::Value>> = self
            .tracks
            .iter()
            .map(|(tag, track)| {
                let mut runs: Vec<Sample> = track.iter().collect();
                if track.tail.is_some()
                    && self.active.contains(tag)
                    && let (Some(hour), Some(last)) = (self.hour, runs.last_mut())
                {
                    last.last = hour;
                    if let Some(read) = self.last_read {
                        last.confirmed = last.confirmed.max(read);
                    }
                }
                let runs = runs
                    .iter()
                    .map(|run| {
                        serde_json::json!([
                            run.first,
                            run.last,
                            run.confirmed,
                            run.identity,
                            run.dealt.to_string(),
                            run.taken.to_string(),
                            run.unknown,
                            run.at_least,
                        ])
                    })
                    .collect();
                (tag, runs)
            })
            .collect();
        let pairs: BTreeMap<PairKey, serde_json::Value> = self
            .pairs
            .iter()
            .map(|(&key, pair)| {
                let read = if pair.generation == self.generation {
                    self.pair_frontier
                } else {
                    pair.read
                };
                let value =
                    serde_json::json!([pair.values, pair.ordinal, read, self.relation_hour(pair)]);
                (key, value)
            })
            .collect();
        let holders: BTreeMap<&String, BTreeSet<&Identity>> = self
            .holders
            .iter()
            .map(|(tag, identities)| (tag, identities.iter().collect()))
            .collect();
        serde_json::json!({
            "identities": self.identities,
            "countries": self.countries.iter().map(|country| serde_json::json!([
                country.dealt.to_string(),
                country.taken.to_string(),
                country.relations,
                country.unknown.total,
                country.unknown.seen,
                country.at_least,
            ])).collect::<Vec<_>>(),
            "pairs": pairs.into_iter().collect::<Vec<_>>(),
            "live": self.live,
            "holders": holders,
            "tracks": tracks,
            "bounds": self.bounds,
            "active": self.active,
            "observed": self.observed,
            "hour": self.hour,
            "lastRead": self.last_read,
            "read": self.read,
            "incomplete": self.incomplete,
            "gapRead": self.gap_read,
            "pairFrontier": self.pair_frontier,
            "pairHour": self.pair_hour,
            "names": self.names.iter().map(|(tag, inputs)| (tag, inputs.len())).collect::<Vec<_>>(),
        })
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_fixtures;
#[cfg(test)]
mod tests;
