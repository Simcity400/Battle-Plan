//! Every country's equipment produced over one branch, for the Equipment produced
//! widget: the units each country's production lines delivered, as a running
//! total, per week or per month, grouped by equipment type (the content's
//! archetypes), exact variant or domain.
//!
//! **Units delivered** come from `ProductionLineOutput`, the collector's count of
//! every production tick per line (`collector::extended::line_output`; the game
//! keeps none per line). A row is one (line, country, variant, organisation) a line
//! produced under, and its `units` are the units delivered since `since_hour`, the
//! sample that began the branch's observation; its `country` is the producer read
//! at the tick. A line's rows are published at every read while it lives and once
//! more, `ended`, at the census that no longer lists it; this product keeps every
//! row's last count for the rest of the branch, so a deleted line, and a line
//! made and deleted between two reads, still count in their producer's total.
//!
//! A country's total of a variant is the sum of its rows. It is exact only while
//! every row is: a row that is not exact left a tick out (the collector could not
//! count it), and a row that disappears without its `ended` publication lost its
//! final count, so from that read on the variant's total, and every group holding
//! it, is unknown, never a partial sum. The totals count from `since_hour`: what a
//! country made before the recording began is not known, so the running total is
//! what it delivered since then (the reply's `observations`).
//!
//! **Refitted ships** are delivered by ship refit lines (`ProductionLines.kind`
//! `KIND_SHIP_REFIT`): they rebuild a ship the country already had as another
//! variant, so they count only when the request asks (`refits`). A line no census
//! listed (made and deleted between two reads) has no recorded kind and counts
//! as new production.
//!
//! **Per week** at a read is its running total minus the running total read
//! exactly seven days (168 hours) before; **per month**, minus the one read at the
//! same hour and day of the month before (the month's last day when it is
//! shorter; engine calendar, 365-day years, `engine::cdate`). Production ticks
//! once a day and the output is read once a day in the census's slot, so such a
//! difference counts exactly the days between. Where no read of the same country
//! identity and observation was taken at that hour (the first week or month, after
//! a gap or a change of identity), the value is unknown.
//!
//! The output is read in the production lines' daily slot and carried in between
//! (`metadata::slot_read`). A value starts only at a read, is carried over the
//! following hours of the same country identity while the table stays readable,
//! and ends at a gap, an incomplete table or a change of identity.
//!
//! **Equipment types** come from content (`equipment::archetype_of`): a variant's
//! recorded equipment token resolved to its authored type and grouped by that
//! type's archetype, in the equipment database's order, with its domain by the
//! engine's classifiers. A type content does not resolve is kept apart under `equipment::UNRESOLVED_MARK`
//! and its token, never merged into another, and its own group stays exact. As
//! the stockpile decides, such a variant could belong to any type (or, when it is
//! counted, to any arm), so while it holds a nonzero or unknown value every
//! resolved type's (or arm's) value is unknown. A country's total of the included
//! types does not depend on how its variants split, so it is sent apart (`total`)
//! and stays exact, unless included types are chosen and an unresolved variant
//! left out of them holds something (it could belong to one of them).
//!
//! A request names what the widget draws: chosen tags (at most
//! [`MAX_COUNTRIES`]) or the top N by the included types' measure, ranked over
//! every country at every hour a value changes (`ranking::top_membership`), a
//! country ranking only once it has produced something (a zero is not ranked); a
//! top-N reply carries each country ever among them with its values cut to the
//! hours it was (`membership`). A request naming neither replies the types
//! alone. The reply's size is counted before it is built and refused past
//! [`MAX_REPLY_BYTES`].

use super::{
    LoadContext, Product,
    country_groups::HourSpan,
    equipment::{self, Domain, StockVariantName, Texts, VariantCatalogue},
    validate_tag,
};
use crate::{
    Source, catalogue,
    metadata::{TaggedCountry, link_slot, slot_read, tagged_countries, validate_campaign},
    ranking,
};
use replay::{
    container::Replay,
    frame::{FrameDecoder, TableView},
    rows::{TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct EquipmentProduced;

/// Changes the scan keeps over every country before it refuses, bounding its
/// memory far above a long campaign's daily deliveries.
const MAX_CHANGES: usize = 8_000_000;
const CACHED_BRANCHES: usize = 2;
/// Most countries one request names, and the largest N of a top-N request.
pub const MAX_COUNTRIES: usize = 30;
/// Most types one request includes, and the longest type key.
pub const MAX_INCLUDED: usize = 64;
const MAX_KEY: usize = 256;
/// The most a reply's series may take serialised: well inside the transport's
/// 32 MiB with the keys and the envelope.
pub const MAX_REPLY_BYTES: usize = 24 * 1024 * 1024;
/// JSON of one series and of one key's values besides their numbers.
const SERIES_OVERHEAD: usize = 128;
const VALUES_OVERHEAD: usize = 64;
/// The largest magnitude a reply's numbers carry exactly (JavaScript's safe integers).
const MAX_EXACT: i64 = (1 << 53) - 1;
const WEEK_HOURS: i32 = 7 * 24;

/// What the values count (spec: running total by default).
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum ProducedMeasure {
    /// Units delivered since the observation began.
    Total,
    /// Units delivered in the seven days before.
    Week,
    /// Units delivered since the same moment of the month before.
    Month,
}

/// What the values are split by.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum ProducedGrouping {
    /// The content's archetypes (the game's equipment types).
    Type,
    /// Each exact variant.
    Variant,
    /// Land, air or naval.
    Domain,
}

/// Chosen countries, or the top N (never both); with neither, the types only.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProducedParams {
    pub branch_id: String,
    pub tags: Vec<String>,
    pub top: Option<u32>,
    pub measure: ProducedMeasure,
    pub grouping: ProducedGrouping,
    /// The type keys counted (`types` keys); none counts every type.
    pub included: Vec<String>,
    /// Count ships refit lines deliver.
    pub refits: bool,
}

/// An equipment type any country produced in the branch.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProducedType {
    /// The archetype key, or `equipment::UNRESOLVED_MARK` and the recorded token when content does not
    /// resolve it.
    pub key: String,
    /// The localised key; null when content does not name it.
    pub name: Option<String>,
    /// Null when content does not resolve the type.
    pub domain: Option<Domain>,
}

/// One group the values are split by.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProducedKey {
    /// A type key, a variant's native handle as a decimal string, or a domain
    /// (`land`, `air`, `naval`, or `equipment::UNRESOLVED_ARM` for types content does not resolve).
    pub key: String,
    /// A type's localised name; null for variants and domains, or when content
    /// does not name it.
    pub name: Option<String>,
    /// The type key a variant belongs to; null for types and domains.
    pub type_key: Option<String>,
    pub domain: Option<Domain>,
    /// A variant's names as each read of the variant table recorded them.
    pub names: Vec<StockVariantName>,
}

/// A value at each of a series' samples, delta coded: 0 before `from`; from there
/// the running sum of `deltas`, the last sum holding once they end. `unknown` holds
/// pairs of sample indices, `[start, end)`, where the value is not known: a sample
/// there is unknown whatever the sum says (its delta is 0).
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProducedCoded {
    pub from: u32,
    #[ts(type = "Array<number>")]
    pub deltas: Vec<i64>,
    pub unknown: Vec<u32>,
}

/// One group's values over a series' samples.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProducedValues {
    /// Its position in the reply's `keys`.
    pub key: u32,
    pub values: ProducedCoded,
}

/// One country's values: sample `i` holds from `first_hours[i]` to
/// `last_hours[i]`; a key without values is 0 at every sample.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProducedSeries {
    pub tag: String,
    pub first_hours: Vec<i32>,
    pub last_hours: Vec<i32>,
    /// Samples that start a new line: after a gap, an hour the tables could not
    /// tell or another country identity.
    pub breaks: Vec<u32>,
    /// Each group's values. A type's or an arm's is unknown wherever a variant whose
    /// type content does not resolve holds a nonzero or unknown value (it could belong
    /// to any of them); the group of unresolved types itself stays exact.
    pub values: Vec<ProducedValues>,
    /// The country's total of every included type: the sum of what its counted
    /// variants delivered, exact however they split into groups. Unknown where a
    /// count is, or where a variant left out of the included types could belong to an
    /// included one. Null when it is 0 at every sample.
    pub total: Option<ProducedCoded>,
    /// For a top-N reply, the hours the country was among the top N, in order;
    /// its samples are cut to them. Null for chosen countries.
    pub membership: Option<Vec<HourSpan>>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EquipmentProducedReply {
    pub branch_id: String,
    pub measure: ProducedMeasure,
    pub grouping: ProducedGrouping,
    /// Every type produced in the branch, in the equipment database's order,
    /// unresolved ones last.
    pub types: Vec<ProducedType>,
    /// The groups the series hold, in the game's order.
    pub keys: Vec<ProducedKey>,
    pub series: Vec<ProducedSeries>,
    /// Hours where at least one requested country's included total is exact
    /// (including zero) and no requested country with a piece covering the hour
    /// has an unknown total. Top N considers all scanned countries before
    /// filtering; explicit tags consider only those tags. Week/month totals need
    /// a complete window.
    pub known: Vec<HourSpan>,
    /// Hours the output was read, ascending: a value shown later is as of the
    /// latest read before it.
    pub reads: Vec<i32>,
    /// The hour each observation began: running totals count from it.
    pub observations: Vec<i32>,
    pub warnings: Vec<String>,
}

/// A country identity within one observation: its canonical slot, native id and
/// the observation (a restart begins another, whose totals start again).
type Identity = (u32, u32, u32);
/// What a country produced: a variant, and whether ship refit lines made it.
type Entry = (u64, bool);

/// A country's total of one entry: its units and how many of its rows are not
/// counted exactly (the total is exact only at zero).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Total {
    pub units: i64,
    pub inexact: u32,
}

/// One tag's reads of the output.
#[derive(Clone, Debug, Default)]
pub struct TagHistory {
    /// Each sample's read hour, ascending, and the last hour it holds.
    pub hours: Vec<i32>,
    pub last_hours: Vec<i32>,
    /// Whether a sample starts a new line.
    pub starts: Vec<bool>,
    pub identities: Vec<Identity>,
    /// Each entry's total at the samples where it changed.
    pub changes: BTreeMap<Entry, Vec<(u32, Total)>>,
    current: BTreeMap<Entry, Total>,
    /// The identity and totals version the last sample took.
    taken: Option<(Identity, u64)>,
    /// The identity and hour the last sample was extended to.
    last: Option<(Identity, i32)>,
}

/// One branch's scan.
#[derive(Clone, Debug, Default)]
pub struct ScannedProduced {
    pub tags: BTreeMap<String, TagHistory>,
    pub reads: Vec<i32>,
    pub observations: Vec<i32>,
    pub variants: VariantCatalogue,
    /// Rows whose producer the links could not resolve to a tagged country.
    pub unattributed: usize,
    /// Rows the collector could not count exactly.
    pub inexact: usize,
    /// Rows that disappeared without their final count.
    pub lost: usize,
    /// Lines with output that no census listed, so their kind is not recorded.
    pub unlisted: usize,
    /// Reads of the production lines without a readable output, and why.
    pub unread: usize,
    pub unread_reason: Option<String>,
    /// Tags that are not text.
    pub unnamed: usize,
    /// The hour from which each observation's totals are unknown, by observation: a
    /// frame that could hold an output publication could not be ingested (its block
    /// or frame unreadable, a read whose country, link or output table is not
    /// complete, or a row whose producer cannot be told once it holds units). The
    /// collector drops an ended line's rows after one written publication, so what
    /// such a publication held is lost for good: no total is resumed after it.
    pub tainted: BTreeMap<u32, i32>,
    /// Frames and reads that could not be ingested.
    pub missed: usize,
}

impl ScannedProduced {
    /// Marks every total of the current observation unknown from `hour` on.
    fn taint(&mut self, observation: u32, hour: i32) {
        self.missed += 1;
        self.tainted.entry(observation).or_insert(hour);
    }
}

struct RowState {
    identity: Option<Identity>,
    entry: Entry,
    units: i64,
    exact: bool,
    ended: bool,
}

fn budget() -> io::Error {
    io::Error::other("The equipment produced history exceeds its memory budget")
}

/// Adds a row's new count to its producer's total.
fn apply(
    totals: &mut HashMap<Identity, (u64, BTreeMap<Entry, Total>)>,
    version: &mut u64,
    state: &RowState,
    units: i64,
    exact: bool,
) {
    let Some(identity) = state.identity else {
        return;
    };
    if units == state.units && exact == state.exact {
        return;
    }
    *version += 1;
    let (changed, entries) = totals.entry(identity).or_default();
    *changed = *version;
    let total = entries.entry(state.entry).or_default();
    total.units = total
        .units
        .saturating_add(units.saturating_sub(state.units));
    total.inexact = total.inexact + u32::from(!exact) - u32::from(!state.exact);
}

/// Scans one branch once for every country.
pub fn scan(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<ScannedProduced> {
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::CountryAppearance,
        TableId::LexicalTokens,
        TableId::VariantDefinitions,
        TableId::ProductionLines,
        TableId::ProductionLineOutput,
    ];
    let cancel = || {
        io::Error::new(
            io::ErrorKind::Interrupted,
            "Equipment produced request cancelled",
        )
    };
    let mut result = ScannedProduced::default();
    let mut rows: HashMap<(u64, u32), RowState> = HashMap::new();
    // Rows still expected at the next read: listed, not ended and exact.
    let mut live: HashSet<(u64, u32)> = HashSet::new();
    let mut totals: HashMap<Identity, (u64, BTreeMap<Entry, Total>)> = HashMap::new();
    let mut kinds: HashMap<u64, u8> = HashMap::new();
    let mut unlisted: HashSet<u64> = HashSet::new();
    let (mut version, mut observation, mut changes) = (0u64, 0u32, 0usize);
    for block in replay.index.iter().filter(|block| block.branch == branch) {
        if cancelled.load(Ordering::Acquire) {
            return Err(cancel());
        }
        let mut decoder = FrameDecoder::for_tables(&selected);
        // A block or frame that cannot be read may have held a publication.
        let Ok(frames) = replay.read_block_selected(block, &selected) else {
            result.taint(observation, block.first_hour);
            for track in result.tags.values_mut() {
                track.last = None;
            }
            continue;
        };
        for frame in frames {
            if cancelled.load(Ordering::Acquire) {
                return Err(cancel());
            }
            let decoded = match decoder.advance(&frame) {
                Ok(decoded) if validate_campaign(decoded, replay).is_ok() => decoded,
                _ => {
                    result.taint(observation, frame.hour);
                    for track in result.tags.values_mut() {
                        track.last = None;
                    }
                    break;
                }
            };
            let hour = frame.hour;
            result.variants.observe(decoded);
            let read = slot_read(
                &decoded.metrics,
                TableId::ProductionLineOutput,
                decoded.hour,
            );
            if read && let Ok(lines) = decoded.complete::<tables::ProductionLines>() {
                for line in lines {
                    kinds.insert(line.handle, line.kind);
                }
            }
            // An hour the tables cannot tell extends nothing, so nothing continues across it.
            let (Ok(countries), Ok(links), Ok(output)) = (
                decoded.complete::<tables::Countries>(),
                decoded.complete::<tables::CountryLinks>(),
                decoded.complete::<tables::ProductionLineOutput>(),
            ) else {
                if read {
                    // The publication this read held, if any, is not ingested: whatever
                    // lines ended in it are lost, for a producer that cannot be told.
                    result.taint(observation, hour);
                    if decoded.complete::<tables::ProductionLines>().is_ok() {
                        result.unread += 1;
                    }
                    if result.unread_reason.is_none()
                        && let TableView::Unavailable { reason } =
                            &*decoded.tables[TableId::ProductionLineOutput.index()]
                    {
                        result.unread_reason = Some(reason.clone());
                    }
                }
                for track in result.tags.values_mut() {
                    track.last = None;
                }
                continue;
            };
            let tagged: Vec<(String, Identity)> = tagged_countries(countries, links)
                .into_iter()
                .filter_map(|(tag, lookup)| {
                    let TaggedCountry::Found(country, slot) = lookup else {
                        return None;
                    };
                    match std::str::from_utf8(tag) {
                        Ok(tag) => Some((tag.to_owned(), (slot, country.id, observation))),
                        Err(_) => {
                            result.unnamed += 1;
                            None
                        }
                    }
                })
                .collect();
            if !read {
                // Carried: the last read still holds, only for the same identity.
                let present: HashMap<&str, Identity> = tagged
                    .iter()
                    .map(|(tag, identity)| (tag.as_str(), *identity))
                    .collect();
                for (tag, track) in &mut result.tags {
                    let continues = present.get(tag.as_str()).copied().filter(|identity| {
                        track.last.is_some_and(|(seen, at)| {
                            seen == *identity && at.checked_add(1) == Some(hour)
                        })
                    });
                    track.last = continues.map(|identity| (identity, hour));
                    if continues.is_some()
                        && let Some(last) = track.last_hours.last_mut()
                    {
                        *last = hour;
                    }
                }
                continue;
            }
            result.reads.push(hour);
            // A new observation restarts every count; totals never cross it.
            if let Some(since) = output.iter().map(|row| row.since_hour).min() {
                match result.observations.last() {
                    None => result.observations.push(since),
                    Some(current) if *current != since => {
                        result.observations.push(since);
                        observation += 1;
                        rows.clear();
                        live.clear();
                        totals.clear();
                    }
                    Some(_) => {}
                }
            }
            let observed: Vec<(String, Identity)> = tagged
                .into_iter()
                .map(|(tag, (slot, id, _))| (tag, (slot, id, observation)))
                .collect();
            let by_slot: HashMap<u32, Identity> = observed
                .iter()
                .map(|(_, identity)| (identity.0, *identity))
                .collect();
            let mut seen = HashSet::with_capacity(output.len());
            for row in output {
                let key = (row.line, row.ordinal);
                seen.insert(key);
                let refit = match kinds.get(&row.line) {
                    Some(kind) => *kind == engine::cequipment_production_line::KIND_SHIP_REFIT,
                    None => {
                        unlisted.insert(row.line);
                        false
                    }
                };
                let producer = link_slot(links, row.country)
                    .and_then(|slot| by_slot.get(&slot))
                    .copied();
                let state = rows.entry(key).or_insert_with(|| {
                    if producer.is_none() {
                        result.unattributed += 1;
                    }
                    RowState {
                        identity: producer,
                        entry: (row.variant, refit),
                        units: 0,
                        exact: true,
                        ended: false,
                    }
                });
                // A producer the links could not tell before may be told now, while
                // nothing was delivered yet; units of a producer never told are lost to
                // some country that cannot be named, so no total is known after them.
                if state.identity.is_none() && state.units == 0 && state.exact {
                    state.identity = producer;
                }
                if state.identity.is_none() && (row.units != 0 || !row.exact) {
                    result.taint(observation, hour);
                }
                // Not exact once: a tick is left out of every later count.
                let exact = state.exact && row.exact;
                if state.exact && !exact {
                    result.inexact += 1;
                }
                apply(&mut totals, &mut version, state, row.units, exact);
                state.units = row.units;
                state.exact = exact;
                state.ended |= row.ended;
            }
            // A row gone without its ended publication lost its final count.
            for key in live.iter().filter(|key| !seen.contains(*key)) {
                if let Some(state) = rows.get_mut(key) {
                    result.lost += 1;
                    let units = state.units;
                    apply(&mut totals, &mut version, state, units, false);
                    state.exact = false;
                }
            }
            live = seen
                .into_iter()
                .filter(|key| {
                    rows.get(key)
                        .is_some_and(|state| !state.ended && state.exact)
                })
                .collect();
            // Each tag's sample: its country's totals at this read.
            let present: HashSet<&str> = observed.iter().map(|(tag, _)| tag.as_str()).collect();
            for track in result
                .tags
                .iter_mut()
                .filter(|(tag, _)| !present.contains(tag.as_str()))
                .map(|(_, track)| track)
            {
                track.last = None;
            }
            for (tag, identity) in observed {
                let track = result.tags.entry(tag).or_default();
                let starts = !track
                    .last
                    .is_some_and(|(seen, at)| seen == identity && at.checked_add(1) == Some(hour));
                let sample = u32::try_from(track.hours.len()).map_err(|_| budget())?;
                track.hours.push(hour);
                track.last_hours.push(hour);
                track.starts.push(starts);
                track.identities.push(identity);
                track.last = Some((identity, hour));
                let (changed, entries) = totals
                    .get(&identity)
                    .map_or((0, None), |(changed, entries)| (*changed, Some(entries)));
                if track.taken == Some((identity, changed)) {
                    continue;
                }
                track.taken = Some((identity, changed));
                let empty = BTreeMap::new();
                let entries = entries.unwrap_or(&empty);
                let keys: BTreeSet<Entry> = entries
                    .keys()
                    .chain(track.current.keys())
                    .copied()
                    .collect();
                for entry in keys {
                    let now = entries.get(&entry).copied().unwrap_or_default();
                    if track.current.get(&entry).copied().unwrap_or_default() != now {
                        changes += 1;
                        if changes > MAX_CHANGES {
                            return Err(budget());
                        }
                        track.changes.entry(entry).or_default().push((sample, now));
                    }
                }
                track.current = entries.clone();
            }
        }
    }
    result.unlisted = unlisted
        .iter()
        .filter(|line| !kinds.contains_key(*line))
        .count();
    Ok(result)
}

/// The hour one calendar month before `hour`: the same day and hour of the month
/// before, or that month's last day when it is shorter (engine calendar: 365-day
/// years from `engine::cdate::HOUR_EPOCH`). None before the calendar begins.
pub fn month_before(hour: i32) -> Option<i32> {
    use engine::cdate::{HOUR_EPOCH, MONTH_LENGTHS};
    let elapsed = i64::from(hour) - i64::from(HOUR_EPOCH);
    if elapsed < 0 {
        return None;
    }
    let (days, hour_of_day) = (elapsed / 24, elapsed % 24);
    let (year, mut day) = (days / 365, days % 365);
    let mut month = 0;
    while day >= i64::from(MONTH_LENGTHS[month]) {
        day -= i64::from(MONTH_LENGTHS[month]);
        month += 1;
    }
    let (year, month) = if month == 0 {
        (year - 1, 11)
    } else {
        (year, month - 1)
    };
    if year < 0 {
        return None;
    }
    let day = day.min(i64::from(MONTH_LENGTHS[month]) - 1);
    let before: i64 = MONTH_LENGTHS[..month]
        .iter()
        .map(|days| i64::from(*days))
        .sum();
    i32::try_from(i64::from(HOUR_EPOCH) + (year * 365 + before + day) * 24 + hour_of_day).ok()
}

/// A variant's type resolved through content.
#[derive(Clone, Debug)]
struct VariantType {
    /// The archetype key, or `UNRESOLVED_MARK` and its token (or its handle without one).
    key: String,
    /// Its place in the equipment database; none when unresolved.
    ordinal: Option<usize>,
    domain: Option<Domain>,
}

/// Each group's total at the samples where it changed, from the entries in it
/// (`group_of` gives an entry's group, none leaves it out): its units and
/// whether every row behind it is exact.
fn group_totals(
    track: &TagHistory,
    group_of: impl Fn(&Entry) -> Option<usize>,
    groups: usize,
) -> Vec<Vec<(u32, i64, bool)>> {
    let mut events: Vec<(u32, usize, &Entry, Total)> = Vec::new();
    for (entry, list) in &track.changes {
        if let Some(group) = group_of(entry) {
            events.extend(
                list.iter()
                    .map(|(sample, total)| (*sample, group, entry, *total)),
            );
        }
    }
    events.sort_by_key(|(sample, group, _, _)| (*sample, *group));
    let mut current: HashMap<&Entry, Total> = HashMap::new();
    let mut sums = vec![(0i64, 0u32); groups];
    let mut out: Vec<Vec<(u32, i64, bool)>> = vec![Vec::new(); groups];
    let mut index = 0;
    while index < events.len() {
        let sample = events[index].0;
        let mut touched = Vec::new();
        while index < events.len() && events[index].0 == sample {
            let (_, group, entry, total) = events[index];
            let before = current.insert(entry, total).unwrap_or_default();
            let sum = &mut sums[group];
            sum.0 = sum
                .0
                .saturating_add(total.units.saturating_sub(before.units));
            sum.1 = sum.1 + total.inexact - before.inexact;
            touched.push(group);
            index += 1;
        }
        touched.dedup();
        for group in touched {
            let (units, inexact) = sums[group];
            let value = (sample, units, inexact == 0);
            let list = &mut out[group];
            if list
                .last()
                .is_none_or(|last| (last.1, last.2) != (value.1, value.2))
            {
                list.push(value);
            }
        }
    }
    out
}

/// A group's total at a sample: its latest change at or before it, else none made yet.
fn total_at(changes: &[(u32, i64, bool)], sample: usize) -> (i64, bool) {
    let position = changes.partition_point(|(at, _, _)| (*at as usize) <= sample);
    position
        .checked_sub(1)
        .map_or((0, true), |index| (changes[index].1, changes[index].2))
}

/// For each sample, the sample the measure subtracts (itself for a running
/// total); none where the value is not known: at or after the hour its
/// observation's totals became unknown (`tainted`), or, for a week or a month,
/// where no read of the same identity was taken exactly that long before, or the
/// recording broke anywhere in between (a gap, an hour the tables could not tell,
/// another identity or an observation restart: a sample starting a new line).
fn bases(
    track: &TagHistory,
    measure: ProducedMeasure,
    tainted: &BTreeMap<u32, i32>,
) -> Vec<Option<usize>> {
    // Samples starting a new line up to each sample, to find a break in a window.
    let breaks: Vec<usize> = track
        .starts
        .iter()
        .scan(0, |count, starts| {
            *count += usize::from(*starts);
            Some(*count)
        })
        .collect();
    (0..track.hours.len())
        .map(|sample| {
            let identity = track.identities[sample];
            if tainted
                .get(&identity.2)
                .is_some_and(|from| track.hours[sample] >= *from)
            {
                return None;
            }
            let target = match measure {
                ProducedMeasure::Total => return Some(sample),
                ProducedMeasure::Week => track.hours[sample].checked_sub(WEEK_HOURS)?,
                ProducedMeasure::Month => month_before(track.hours[sample])?,
            };
            let base = track.hours.binary_search(&target).ok()?;
            (track.identities[base] == identity && breaks[sample] == breaks[base]).then_some(base)
        })
        .collect()
}

/// The measure of a group at a sample: unknown where its total is not exact.
fn measure_at(
    changes: &[(u32, i64, bool)],
    sample: usize,
    base: usize,
    total: bool,
) -> Option<i64> {
    let (units, exact) = total_at(changes, sample);
    if !exact {
        return None;
    }
    Some(if total {
        units
    } else {
        units.saturating_sub(total_at(changes, base).0)
    })
}

/// A piece of a sample shown: its hours, whether it starts a line, and the sample.
#[derive(Clone, Copy, Debug)]
struct Piece {
    first: i32,
    last: i32,
    starts: bool,
    sample: usize,
}

/// The samples whose measure is defined, as pieces over their hours.
fn pieces(track: &TagHistory, bases: &[Option<usize>]) -> Vec<Piece> {
    let mut out = Vec::new();
    for sample in 0..track.hours.len() {
        if bases[sample].is_none() {
            continue;
        }
        let follows = sample > 0 && bases[sample - 1].is_some();
        out.push(Piece {
            first: track.hours[sample],
            last: track.last_hours[sample],
            starts: track.starts[sample] || !follows,
            sample,
        });
    }
    out
}

/// Pieces cut to spans: a piece keeps its line break only where it starts itself.
fn clip(pieces: &[Piece], spans: &[HourSpan]) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut start = 0;
    for span in spans {
        while pieces
            .get(start)
            .is_some_and(|piece| piece.last < span.first_hour)
        {
            start += 1;
        }
        for piece in pieces[start..]
            .iter()
            .take_while(|piece| piece.first <= span.last_hour)
        {
            let first = piece.first.max(span.first_hour);
            let last = piece.last.min(span.last_hour);
            if first <= last {
                out.push(Piece {
                    first,
                    last,
                    starts: piece.starts && first == piece.first,
                    sample: piece.sample,
                });
            }
        }
    }
    out
}

/// Values delta coded; none when they are a known zero at every sample. An unknown
/// sample is listed in a range and holds the last known sum (delta 0), so the
/// deltas after it stay relative to the last known value.
fn encode(values: &[Option<i64>]) -> Option<ProducedCoded> {
    let mut unknown = Vec::new();
    for (index, value) in values.iter().enumerate() {
        if value.is_some() {
            continue;
        }
        let index = index as u32;
        match unknown.last_mut() {
            Some(end) if *end == index => *end = index + 1,
            _ => unknown.extend([index, index + 1]),
        }
    }
    let from = values
        .iter()
        .position(|value| value.is_some_and(|value| value != 0))
        .unwrap_or(values.len());
    if from == values.len() && unknown.is_empty() {
        return None;
    }
    let mut previous = 0i64;
    let mut deltas: Vec<i64> = values[from..]
        .iter()
        .map(|value| match value {
            Some(value) => {
                let delta = value.saturating_sub(previous);
                previous = *value;
                delta
            }
            None => 0,
        })
        .collect();
    while deltas.last() == Some(&0) {
        deltas.pop();
    }
    Some(ProducedCoded {
        from: from as u32,
        deltas,
        unknown,
    })
}

/// The bytes a coded value takes serialised, at least.
fn coded_bytes(coded: &ProducedCoded) -> usize {
    VALUES_OVERHEAD
        + coded
            .deltas
            .iter()
            .map(|delta| digits(*delta))
            .sum::<usize>()
        + coded
            .unknown
            .iter()
            .map(|index| digits(i64::from(*index)))
            .sum::<usize>()
}

/// The measure at every sample of a set of entries summed (`counted` picks them):
/// none where its base read is missing or its total is not exact.
fn measured(
    track: &TagHistory,
    bases: &[Option<usize>],
    total: bool,
    counted: impl Fn(&Entry) -> bool,
) -> Vec<Option<i64>> {
    let sums = group_totals(track, |entry| counted(entry).then_some(0), 1);
    bases
        .iter()
        .enumerate()
        .map(|(sample, base)| base.and_then(|base| measure_at(&sums[0], sample, base, total)))
        .collect()
}

/// Digits and a comma: at least as many bytes as a number takes serialised.
fn digits(value: i64) -> usize {
    value.to_string().len() + 1
}

/// What a reply types and names with, from the recorded content stack: its
/// equipment types and its localisation. Either may be missing (no game folders,
/// or content that could not be read); then types stay unresolved and names
/// unknown, never assumed.
#[derive(Clone, Copy, Default)]
pub struct Named<'a> {
    pub units: Option<&'a installed_content::equipment::Database>,
    pub texts: Option<&'a dyn Texts>,
}

impl Named<'_> {
    /// A resolved type's localised name; none for an unresolved one.
    fn type_name(&self, key: &str) -> Option<String> {
        self.texts
            .filter(|_| !key.starts_with(equipment::UNRESOLVED_MARK))
            .and_then(|texts| texts.text(key))
            .filter(|name| !name.is_empty() && name.len() <= MAX_KEY)
    }
}

/// Every produced variant's type, why types could not be resolved, and how many
/// variants content does not type.
fn variant_types(
    scanned: &ScannedProduced,
    named: Named,
) -> (HashMap<u64, VariantType>, BTreeSet<String>, usize) {
    let mut types = HashMap::new();
    let mut untyped = BTreeSet::new();
    let mut unresolved_count = 0;
    let variants: BTreeSet<u64> = scanned
        .tags
        .values()
        .flat_map(|track| track.changes.keys().map(|(variant, _)| *variant))
        .collect();
    for variant in variants {
        let token = scanned.variants.token(variant);
        let resolved = match (named.units, token.is_empty()) {
            (_, true) => Err(format!(
                "variant {variant} has no recorded definition in this branch"
            )),
            (None, false) => Err(format!("{token}: content is unavailable")),
            (Some(units), false) => equipment::archetype_of(units, token),
        };
        // Kept apart under the shared mark (as the stockpile keys them): its token, or
        // its handle without one.
        let unresolved = if token.is_empty() {
            format!("{}{variant}", equipment::UNRESOLVED_MARK)
        } else {
            format!("{}{token}", equipment::UNRESOLVED_MARK)
        };
        types.insert(
            variant,
            match resolved {
                Ok((key, ordinal, domain)) => VariantType {
                    key,
                    ordinal: Some(ordinal),
                    domain: Some(domain),
                },
                Err(reason) => {
                    untyped.insert(reason);
                    unresolved_count += 1;
                    VariantType {
                        key: unresolved,
                        ordinal: None,
                        domain: None,
                    }
                }
            },
        );
    }
    (types, untyped, unresolved_count)
}

fn domain_key(domain: Option<Domain>) -> &'static str {
    match domain {
        Some(Domain::Land) => "land",
        Some(Domain::Air) => "air",
        Some(Domain::Naval) => "naval",
        None => equipment::UNRESOLVED_ARM,
    }
}

/// The groups of a grouping, in the game's order, with each entry's group.
struct Grouped {
    keys: Vec<ProducedKey>,
    of: HashMap<Entry, usize>,
    /// Whether a group is a type or an arm content resolves, whose total a variant of
    /// an unresolved type could belong to (never for a variant or an unresolved group).
    ambiguous: Vec<bool>,
    /// Variants whose type content does not resolve.
    unresolved: HashSet<u64>,
    /// Whether the included types name a type content resolves: only such a type
    /// could hold a left-out unresolved variant (an unresolved type's group is its
    /// recorded token, so its membership is exact).
    includes_resolved: bool,
}

impl Grouped {
    /// Whether an unresolved entry could belong to the resolved groups, as the
    /// stockpile decides (`variantsComplete`, `decidesCompleteness`): to every type
    /// whatever types are included, and to an arm only when it is itself counted.
    fn could_belong(&self, params: &ProducedParams, entry: &Entry) -> bool {
        self.unresolved.contains(&entry.0)
            && (params.refits || !entry.1)
            && match params.grouping {
                ProducedGrouping::Type => true,
                ProducedGrouping::Domain => self.of.contains_key(entry),
                ProducedGrouping::Variant => false,
            }
    }

    /// Whether an unresolved entry left out of the included types could belong to an
    /// included one, so the included total is not known while it holds anything.
    fn left_out(&self, params: &ProducedParams, entry: &Entry) -> bool {
        self.includes_resolved
            && self.unresolved.contains(&entry.0)
            && (params.refits || !entry.1)
            && !self.of.contains_key(entry)
    }
}

fn grouped(
    scanned: &ScannedProduced,
    types: &HashMap<u64, VariantType>,
    named: Named,
    params: &ProducedParams,
) -> Grouped {
    let texts = named.texts;
    let type_name = |key: &str| named.type_name(key);
    let entries: BTreeSet<Entry> = scanned
        .tags
        .values()
        .flat_map(|track| track.changes.keys().copied())
        .filter(|entry| params.refits || !entry.1)
        .filter(|entry| {
            params.included.is_empty()
                || types
                    .get(&entry.0)
                    .is_some_and(|kind| params.included.contains(&kind.key))
        })
        .collect();
    // Each group's sort key: its type's database place (unresolved last), then its own.
    let mut groups: BTreeMap<(usize, String, u64), ProducedKey> = BTreeMap::new();
    let mut of_key: HashMap<Entry, (usize, String, u64)> = HashMap::new();
    let mut resolved: HashMap<(usize, String, u64), bool> = HashMap::new();
    for entry in entries {
        let Some(kind) = types.get(&entry.0) else {
            continue;
        };
        let ordinal = kind.ordinal.unwrap_or(usize::MAX);
        let (sort, key) = match params.grouping {
            ProducedGrouping::Type => (
                (ordinal, kind.key.clone(), 0),
                ProducedKey {
                    key: kind.key.clone(),
                    name: type_name(&kind.key),
                    type_key: None,
                    domain: kind.domain,
                    names: Vec::new(),
                },
            ),
            ProducedGrouping::Variant => (
                (ordinal, kind.key.clone(), entry.0),
                ProducedKey {
                    key: entry.0.to_string(),
                    name: None,
                    type_key: Some(kind.key.clone()),
                    domain: kind.domain,
                    names: scanned.variants.names(entry.0, texts),
                },
            ),
            ProducedGrouping::Domain => {
                let rank = match kind.domain {
                    Some(Domain::Land) => 0,
                    Some(Domain::Air) => 1,
                    Some(Domain::Naval) => 2,
                    None => 3,
                };
                (
                    (rank, String::new(), 0),
                    ProducedKey {
                        key: domain_key(kind.domain).to_owned(),
                        name: None,
                        type_key: None,
                        domain: kind.domain,
                        names: Vec::new(),
                    },
                )
            }
        };
        resolved.insert(
            sort.clone(),
            params.grouping != ProducedGrouping::Variant && kind.ordinal.is_some(),
        );
        groups.entry(sort.clone()).or_insert(key);
        of_key.insert(entry, sort);
    }
    let positions: HashMap<&(usize, String, u64), usize> = groups
        .keys()
        .enumerate()
        .map(|(index, sort)| (sort, index))
        .collect();
    let of = of_key
        .iter()
        .map(|(entry, sort)| (*entry, positions[sort]))
        .collect();
    let ambiguous = groups.keys().map(|sort| resolved[sort]).collect();
    Grouped {
        keys: groups.into_values().collect(),
        of,
        ambiguous,
        unresolved: types
            .iter()
            .filter(|(_, kind)| kind.ordinal.is_none())
            .map(|(variant, _)| *variant)
            .collect(),
        includes_resolved: types
            .values()
            .any(|kind| kind.ordinal.is_some() && params.included.contains(&kind.key)),
    }
}

/// Where a set of entries holds a nonzero or unknown value (at every sample), so
/// what they could belong to is not known.
fn holding(values: &[Option<i64>]) -> Vec<bool> {
    values.iter().map(|value| *value != Some(0)).collect()
}

/// A country's included total at every sample (exact however its variants split
/// into groups), unknown where a left-out unresolved variant holds anything.
fn included_total(
    track: &TagHistory,
    bases: &[Option<usize>],
    grouped: &Grouped,
    params: &ProducedParams,
) -> Vec<Option<i64>> {
    let total = params.measure == ProducedMeasure::Total;
    let sums = measured(track, bases, total, |entry| grouped.of.contains_key(entry));
    let outside = holding(&measured(track, bases, total, |entry| {
        grouped.left_out(params, entry)
    }));
    sums.into_iter()
        .zip(outside)
        .map(|(sum, outside)| sum.filter(|_| !outside))
        .collect()
}

/// The series a request asks for and the keys they hold.
fn requested(
    scanned: &ScannedProduced,
    grouped: &Grouped,
    params: &ProducedParams,
    cancelled: &AtomicBool,
) -> io::Result<Requested> {
    let total = params.measure == ProducedMeasure::Total;
    let mut coverage = Coverage::default();
    // The countries and the hours each is shown.
    let shown: Vec<(&String, &TagHistory, Option<Vec<HourSpan>>)> = match params.top {
        Some(count) => {
            // Every country's included total, ranked at every hour. A country ranks
            // only once it has produced something (user decision, 29 Sep): a zero, like
            // an unknown value, is not ranked.
            let ranked: BTreeMap<String, Vec<(i32, i32, Option<i64>)>> = scanned
                .tags
                .iter()
                .map(|(tag, track)| {
                    let bases = bases(track, params.measure, &scanned.tainted);
                    let totals = included_total(track, &bases, grouped, params);
                    let runs = pieces(track, &bases)
                        .into_iter()
                        .map(|piece| {
                            let value = totals[piece.sample];
                            coverage.record(&piece, value);
                            let value = value.filter(|value| *value != 0);
                            (piece.first, piece.last, value)
                        })
                        .collect();
                    (tag.clone(), runs)
                })
                .collect();
            let members = ranking::top_membership(
                &ranked,
                count as usize,
                |run| (run.0, run.1),
                |run| run.2,
                cancelled,
            )?;
            members
                .into_iter()
                .filter_map(|(tag, spans)| {
                    scanned
                        .tags
                        .get_key_value(&tag)
                        .map(|(tag, track)| (tag, track, Some(spans)))
                })
                .collect()
        }
        None => {
            let mut seen = HashSet::new();
            params
                .tags
                .iter()
                .filter(|tag| seen.insert(tag.as_str()))
                .filter_map(|tag| {
                    scanned
                        .tags
                        .get_key_value(tag)
                        .map(|(tag, track)| (tag, track, None))
                })
                .collect()
        }
    };
    let mut used = vec![false; grouped.keys.len()];
    let mut series = Vec::with_capacity(shown.len());
    let mut bytes = 0usize;
    for (tag, track, membership) in shown {
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Equipment produced request cancelled",
            ));
        }
        let bases = bases(track, params.measure, &scanned.tainted);
        let all = pieces(track, &bases);
        let pieces = match &membership {
            Some(spans) => clip(&all, spans),
            None => all,
        };
        let groups = group_totals(
            track,
            |entry| grouped.of.get(entry).copied(),
            grouped.keys.len(),
        );
        // Where a variant of an unresolved type holds anything: every resolved type or
        // arm it could belong to is unknown then.
        let unsure = holding(&measured(track, &bases, total, |entry| {
            grouped.could_belong(params, entry)
        }));
        let exact = |at: &[Option<i64>]| -> io::Result<()> {
            if at.iter().flatten().any(|value| value.abs() > MAX_EXACT) {
                return Err(io::Error::other(
                    "A production total is too large to reply exactly",
                ));
            }
            Ok(())
        };
        let any_unsure = pieces.iter().any(|piece| unsure[piece.sample]);
        let mut values = Vec::new();
        for (key, changes) in groups.iter().enumerate() {
            let ambiguous = grouped.ambiguous[key] && any_unsure;
            if changes.is_empty() && !ambiguous {
                continue;
            }
            let at: Vec<Option<i64>> = pieces
                .iter()
                .map(|piece| {
                    if ambiguous && unsure[piece.sample] {
                        return None;
                    }
                    measure_at(
                        changes,
                        piece.sample,
                        bases[piece.sample].unwrap_or(piece.sample),
                        total,
                    )
                })
                .collect();
            exact(&at)?;
            if let Some(coded) = encode(&at) {
                used[key] = true;
                bytes += coded_bytes(&coded);
                values.push(ProducedValues {
                    key: key as u32,
                    values: coded,
                });
            }
        }
        let totals = included_total(track, &bases, grouped, params);
        if membership.is_none() {
            for piece in &pieces {
                coverage.record(piece, totals[piece.sample]);
            }
        }
        let total_at: Vec<Option<i64>> = pieces.iter().map(|piece| totals[piece.sample]).collect();
        exact(&total_at)?;
        let total_coded = encode(&total_at);
        bytes += total_coded.as_ref().map_or(0, coded_bytes);
        let item = ProducedSeries {
            tag: tag.clone(),
            first_hours: pieces.iter().map(|piece| piece.first).collect(),
            last_hours: pieces.iter().map(|piece| piece.last).collect(),
            breaks: pieces
                .iter()
                .enumerate()
                .filter(|(_, piece)| piece.starts)
                .map(|(index, _)| index as u32)
                .collect(),
            values,
            total: total_coded,
            membership,
        };
        bytes += SERIES_OVERHEAD
            + item.tag.len()
            + item
                .first_hours
                .iter()
                .chain(&item.last_hours)
                .map(|hour| digits(i64::from(*hour)))
                .sum::<usize>()
            + item.breaks.iter().map(|index| digits(i64::from(*index))).sum::<usize>()
            // A span: its keys and braces, two hours of at most 11 characters.
            + item.membership.as_ref().map_or(0, |spans| spans.len() * 52);
        if bytes > MAX_REPLY_BYTES {
            return Err(io::Error::other(format!(
                "These countries' production history takes more than one reply holds ({} MiB); choose fewer countries or group by type.",
                MAX_REPLY_BYTES / (1024 * 1024)
            )));
        }
        series.push(item);
    }
    // Only the keys the series hold, renumbered in order.
    let renumber: Vec<Option<u32>> = used
        .iter()
        .scan(0u32, |next, used| {
            Some(used.then(|| {
                *next += 1;
                *next - 1
            }))
        })
        .collect();
    for item in &mut series {
        for values in &mut item.values {
            values.key = renumber[values.key as usize].expect("a used key");
        }
    }
    let keys = grouped
        .keys
        .iter()
        .zip(&used)
        .filter(|(_, used)| **used)
        .map(|(key, _)| key.clone())
        .collect();
    let spans = coverage.known();
    // A span's keys, braces and two signed hours fit in 52 bytes.
    if bytes.saturating_add(spans.len().saturating_mul(52)) > MAX_REPLY_BYTES {
        return Err(io::Error::other(
            "The production history and its known windows exceed the reply budget; choose fewer countries or group by type.",
        ));
    }
    Ok(Requested {
        keys,
        series,
        known: spans,
    })
}

#[derive(Default)]
struct Requested {
    keys: Vec<ProducedKey>,
    series: Vec<ProducedSeries>,
    known: Vec<HourSpan>,
}

/// Exact pieces minus unknown pieces, before ranking removes countries.
#[derive(Default)]
struct Coverage {
    exact: BTreeMap<i32, i32>,
    unknown: BTreeMap<i32, i32>,
}

impl Coverage {
    fn record(&mut self, piece: &Piece, total: Option<i64>) {
        let spans = if total.is_some() {
            &mut self.exact
        } else {
            &mut self.unknown
        };
        spans
            .entry(piece.first)
            .and_modify(|end| *end = (*end).max(piece.last))
            .or_insert(piece.last);
    }

    fn known(self) -> Vec<HourSpan> {
        let merge = |spans: BTreeMap<i32, i32>| {
            let mut merged: Vec<HourSpan> = Vec::new();
            for (first_hour, last_hour) in spans {
                match merged.last_mut() {
                    Some(span) if first_hour <= span.last_hour.saturating_add(1) => {
                        span.last_hour = span.last_hour.max(last_hour);
                    }
                    _ => merged.push(HourSpan {
                        first_hour,
                        last_hour,
                    }),
                }
            }
            merged
        };
        let mut unknown = merge(self.unknown).into_iter().peekable();
        let mut known = Vec::new();
        for span in merge(self.exact) {
            // Widen the cursor so a piece ending at i32::MAX can be subtracted.
            let mut first = i64::from(span.first_hour);
            let last = i64::from(span.last_hour);
            while let Some(span) = unknown.peek() {
                if i64::from(span.last_hour) < first {
                    unknown.next();
                    continue;
                }
                if i64::from(span.first_hour) > last {
                    break;
                }
                if i64::from(span.first_hour) > first {
                    known.push(HourSpan {
                        first_hour: first as i32,
                        last_hour: span.first_hour - 1,
                    });
                }
                first = first.max(i64::from(span.last_hour) + 1);
                if first > last {
                    break;
                }
                unknown.next();
            }
            if first <= last {
                known.push(HourSpan {
                    first_hour: first as i32,
                    last_hour: last as i32,
                });
            }
        }
        known
    }
}

/// Every type produced in the branch, in the database's order, unresolved last.
fn produced_types(types: &HashMap<u64, VariantType>, named: Named) -> Vec<ProducedType> {
    let mut listed: BTreeMap<(usize, String), ProducedType> = BTreeMap::new();
    for kind in types.values() {
        listed
            .entry((kind.ordinal.unwrap_or(usize::MAX), kind.key.clone()))
            .or_insert_with(|| ProducedType {
                key: kind.key.clone(),
                name: named.type_name(&kind.key),
                domain: kind.domain,
            });
    }
    listed.into_values().collect()
}

/// The reply to a request from a branch's scan, typed and named with `named`;
/// `warnings` are the content's own, reported first.
pub fn reply(
    scanned: &ScannedProduced,
    params: &ProducedParams,
    named: Named,
    mut warnings: Vec<String>,
    cancelled: &AtomicBool,
) -> io::Result<EquipmentProducedReply> {
    let (types, untyped, unresolved) = variant_types(scanned, named);
    let resolved_keys: BTreeSet<&str> = types
        .values()
        .filter(|kind| kind.ordinal.is_some())
        .map(|kind| kind.key.as_str())
        .collect();
    warnings.extend(equipment::marked_keys_warning(resolved_keys));
    if unresolved > 0 {
        warnings.push(format!(
            "{unresolved} produced variants have an equipment type the recorded content does not resolve; each type is kept apart under its recorded token."
        ));
    }
    for reason in untyped.iter().take(8) {
        warnings.push(format!("An equipment type cannot be resolved: {reason}."));
    }
    if scanned.reads.is_empty() {
        warnings.push(match &scanned.unread_reason {
            Some(reason) => format!(
                "Equipment produced is not in this recording: the production output was not recorded ({reason})."
            ),
            None => "Equipment produced is not in this recording: it has no production output (ProductionLineOutput).".to_owned(),
        });
    } else if scanned.unread > 0 {
        warnings.push(format!(
            "{} reads of the production lines have no readable output{}; every total is unknown from the first of them on (an ended line's final count is published only once).",
            scanned.unread,
            scanned
                .unread_reason
                .as_ref()
                .map(|reason| format!(" ({reason})"))
                .unwrap_or_default()
        ));
    }
    if let Some((_, from)) = scanned.tainted.iter().next() {
        warnings.push(format!(
            "{} recorded frames, reads or output rows could not be ingested (a frame unreadable, a read without complete countries, links or output, or units of a producer the links cannot tell); what an output publication held is published only once, so every total is unknown from hour {from} on, until the collector's observation restarts.",
            scanned.missed,
        ));
    }
    if scanned.inexact > 0 {
        warnings.push(format!(
            "{} production output rows left out a tick the collector could not count; their country's total of that variant is unknown from then on.",
            scanned.inexact
        ));
    }
    if scanned.lost > 0 {
        warnings.push(format!(
            "{} production output rows disappeared without their final count; their country's total of that variant is unknown from then on.",
            scanned.lost
        ));
    }
    if scanned.unattributed > 0 {
        warnings.push(format!(
            "{} production output rows name a producer the links do not resolve to a tagged country; they are left out.",
            scanned.unattributed
        ));
    }
    if scanned.unlisted > 0 {
        warnings.push(format!(
            "{} lines were made and deleted between two reads, so their kind is not recorded; what they delivered counts as new production.",
            scanned.unlisted
        ));
    }
    if scanned.observations.len() > 1 {
        warnings.push(format!(
            "The production output observation restarted {} times in this branch; running totals start again at each.",
            scanned.observations.len() - 1
        ));
    }
    if scanned.unnamed > 0 {
        warnings.push(format!(
            "{} country-hours have a tag that is not text; they are left out.",
            scanned.unnamed
        ));
    }
    for tag in params
        .tags
        .iter()
        .filter(|tag| !scanned.tags.contains_key(*tag))
    {
        warnings.push(format!("{tag}: no recorded production in this branch"));
    }
    let grouped = grouped(scanned, &types, named, params);
    let requested = if params.top.is_some() || !params.tags.is_empty() {
        requested(scanned, &grouped, params, cancelled)?
    } else {
        Requested::default()
    };
    Ok(EquipmentProducedReply {
        branch_id: params.branch_id.clone(),
        measure: params.measure,
        grouping: params.grouping,
        types: produced_types(&types, named),
        keys: requested.keys,
        series: requested.series,
        known: requested.known,
        reads: scanned.reads.clone(),
        observations: scanned.observations.clone(),
        warnings,
    })
}

#[derive(Default)]
struct Cache {
    entries: VecDeque<(u64, Arc<ScannedProduced>)>,
}

impl Product for EquipmentProduced {
    const KEY: &'static str = "equipment-produced";
    const CONTENT_DEPENDENT: bool = true;
    type Params = ProducedParams;
    type Reply = EquipmentProducedReply;

    fn types() -> Vec<String> {
        vec![
            ProducedParams::decl(),
            ProducedMeasure::decl(),
            ProducedGrouping::decl(),
            ProducedType::decl(),
            ProducedKey::decl(),
            ProducedCoded::decl(),
            ProducedValues::decl(),
            ProducedSeries::decl(),
            EquipmentProducedReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        if params.tags.len() > MAX_COUNTRIES {
            return Err("Invalid country selection");
        }
        for tag in &params.tags {
            validate_tag(tag)?;
        }
        if let Some(count) = params.top
            && (!params.tags.is_empty() || count == 0 || count as usize > MAX_COUNTRIES)
        {
            return Err("Invalid top countries");
        }
        if params.included.len() > MAX_INCLUDED
            || params
                .included
                .iter()
                .any(|key| key.is_empty() || key.len() > MAX_KEY)
        {
            return Err("Invalid equipment types");
        }
        Ok(())
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().expect("validated branch");
        // The lock is held while a branch is scanned, as Factories holds its own:
        // requests arriving at once (several widgets, the settings list) share the
        // first one's scan instead of each scanning the branch.
        let scanned = {
            let state = source.state::<Cache>();
            let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
            match cache.entries.iter().find(|(cached, _)| *cached == branch) {
                Some((_, scanned)) => Arc::clone(scanned),
                None => {
                    let scanned = Arc::new(scan(&source.replay(), branch, cancelled)?);
                    cache.entries.push_back((branch, Arc::clone(&scanned)));
                    while cache.entries.len() > CACHED_BRANCHES {
                        cache.entries.pop_front();
                    }
                    scanned
                }
            }
        };
        let mut warnings = Vec::new();
        // Without the game folders the units still count, by variant and unnamed types.
        let (catalogue, units) = match context
            .content()
            .and_then(|content| catalogue::catalogue(source, content))
        {
            Ok(catalogue) => {
                warnings.extend(catalogue.verification.iter().cloned());
                warnings.extend(catalogue.warnings.iter().cloned());
                let units = match equipment::units(source, &catalogue) {
                    Ok(units) => {
                        // Definitions the reader refused are missing, named and counted.
                        warnings.extend(equipment::equipment_warnings(&units));
                        Some(units)
                    }
                    Err(error) => {
                        warnings.push(format!("Equipment types are unavailable: {error}"));
                        None
                    }
                };
                (Some(catalogue), units)
            }
            Err(error) => {
                warnings.push(format!(
                    "Equipment names and types are unavailable: {error}"
                ));
                (None, None)
            }
        };
        let named = Named {
            units: units.as_deref(),
            texts: catalogue
                .as_ref()
                .map(|catalogue| &**catalogue as &dyn Texts),
        };
        reply(&scanned, params, named, warnings, cancelled)
    }
}

#[cfg(test)]
mod tests;
