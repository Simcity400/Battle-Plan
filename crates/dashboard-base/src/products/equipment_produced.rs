//! Every country's equipment produced over one branch, for the Equipment produced
//! widget: the units each country's production lines delivered, or the factory IC
//! they applied, as a running total, per day, per week or per month, grouped by
//! equipment type (the content's archetypes), exact variant or domain.
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
//! **Refitted ships** are delivered by ship refit lines (the output's recorded
//! purpose): they rebuild a ship the country already had as another variant, so
//! they count only when the request asks (`refits`). A production tick whose
//! purpose was not recorded counts as new production.
//!
//! **Applied IC** (`count: ic`) is the same scan's exact factory IC input: the
//! `ProductionLineOutput` rows' recorded `spent_raw`, the progress the game's
//! production tick added to each line, excluding instant progress (fixed5,
//! [`equipment::EQUIPMENT_DECIMALS`]), as the Production spending widget shows it.
//! IC is one currency, so every type adds up exactly. It is not the value of the
//! units delivered: a unit's cost belongs to the line that made it (its
//! organisation's bonus and the modifiers then) and is read once a day, not at each
//! delivery, so that value cannot be reproduced exactly. Where the recording holds
//! no applied input (an older format) or an input was lost, the IC is unknown from
//! then on in its observation, never a partial sum.
//!
//! **Per day** at a read is its running total minus the running total read
//! exactly one day (24 hours) before: one production tick. **Per week**, minus the
//! running total read exactly seven days (168 hours) before; **per month**, minus the one read at the
//! same hour and day of the month before (the month's last day when it is
//! shorter; engine calendar, 365-day years, `engine_native::cdate`). Production ticks
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
//! A request names what the widget draws: chosen tags or the top N by the included
//! types' measure, ranked over every country at every hour a value changes
//! (`ranking::top_membership`), a country ranking only once it has produced
//! something (a zero is not ranked); a top-N reply carries each country ever among
//! them with its values cut to the hours it was (`membership`). A request naming
//! neither replies the types alone. The reply's size is counted before it is built
//! and refused past [`MAX_REPLY_BYTES`].

use super::{
    LoadContext, Product,
    country_groups::HourSpan,
    equipment::{self, Domain, StockVariantName, Texts},
    validate_tag,
};
use crate::{Source, ranking};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::atomic::{AtomicBool, Ordering},
};
use ts_rs::TS;

pub struct EquipmentProduced;

/// The most a reply's series may take serialised: well inside the transport's
/// 32 MiB with the keys and the envelope.
pub use super::MAX_PRODUCT_REPLY_BYTES as MAX_REPLY_BYTES;
/// JSON of one series and of one key's values besides their numbers.
const SERIES_OVERHEAD: usize = 128;
const VALUES_OVERHEAD: usize = 64;
/// The largest magnitude a reply's numbers carry exactly (JavaScript's safe integers).
const MAX_EXACT: i64 = (1 << 53) - 1;
const DAY_HOURS: i32 = 24;
const WEEK_HOURS: i32 = 7 * DAY_HOURS;

/// Which window the values count (spec: per day by default in the widget).
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum ProducedMeasure {
    /// Units delivered since the observation began.
    Total,
    /// Units delivered in the day (one production tick) before.
    Day,
    /// Units delivered in the seven days before.
    Week,
    /// Units delivered since the same moment of the month before.
    Month,
}

/// What the values count: units delivered, or the factory IC applied to the lines.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum ProducedCount {
    /// Units delivered: whole numbers.
    #[default]
    Units,
    /// Applied factory IC, fixed5: one currency, so different types add up.
    Ic,
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
    /// What the values count and the top N rank by; units when left out.
    #[serde(default)]
    pub count: ProducedCount,
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
    /// Independently exact equipment-line subtotal when Rocket coverage makes
    /// the combined group unknown. It is not a bound on signed delivered units.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub equipment_values: Option<ProducedCoded>,
}

/// One country's values: sample `i` holds from `first_hours[i]` to
/// `last_hours[i]`; a key without values is 0 at every sample.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProducedSeries {
    pub tag: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub identities: Option<Vec<super::country_history::CountryIdentityRun>>,
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
    pub count: ProducedCount,
    /// Fixed-point decimals of every value: 0 for units, 5 for IC.
    pub decimals: u8,
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

use super::equipment::production::{Entry, Purpose, VariantType, variant_types_for};
pub use super::equipment::production::{Named, ScannedProduced, TagHistory, Total, scan};

/// The hour one calendar month before `hour`: the same day and hour of the month
/// before, or that month's last day when it is shorter (engine calendar: 365-day
/// years from `engine_native::cdate::HOUR_EPOCH`). None before the calendar begins.
pub fn month_before(hour: i32) -> Option<i32> {
    use engine_native::{cdate::HOUR_EPOCH, cdate::MONTH_LENGTHS};
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

/// Exact all-type running totals with the single-stat widget's default refit
/// selection. Reuses its bases and measured arithmetic over the cached shared
/// production walk; missing or tainted samples stay unknown, including zero.
/// Delivered units are the fixed5 logistics-accounting amount. The recorder
/// observes those deliveries; the game keeps no per-line total.
#[cfg(any(test, feature = "test-support"))]
pub struct CountryTotalRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub amount: Option<i64>,
    pub starts_segment: bool,
}

#[cfg(any(test, feature = "test-support"))]
pub fn country_totals(
    scanned: &ScannedProduced,
    tags: &[String],
    cancelled: &AtomicBool,
) -> io::Result<BTreeMap<String, Vec<CountryTotalRun>>> {
    let mut out = BTreeMap::new();
    for tag in tags {
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Equipment totals cancelled",
            ));
        }
        let Some(track) = scanned.tags.get(tag) else {
            continue;
        };
        let base = bases(track, ProducedMeasure::Total, &scanned.tainted);
        let equipment = measured(track, &base, true, ProducedCount::Units, |entry| {
            entry.1 != Purpose::Refit
        })?;
        let rocket = scanned.rocket.tags.get(tag);
        let rocket_values = rocket
            .map(|track| {
                measured(
                    track,
                    &bases(track, ProducedMeasure::Total, &BTreeMap::new()),
                    true,
                    ProducedCount::Units,
                    |_| true,
                )
            })
            .transpose()?;
        let values = equipment
            .into_iter()
            .enumerate()
            .map(|(sample, equipment)| {
                let rocket = rocket.and_then(|rocket| {
                    let index = rocket
                        .hours
                        .partition_point(|hour| *hour <= track.hours[sample])
                        .checked_sub(1)?;
                    ((rocket.identities[index].0, rocket.identities[index].1)
                        == (track.identities[sample].0, track.identities[sample].1)
                        && rocket.last_hours[index] >= track.last_hours[sample]
                        && same_origin(
                            scanned,
                            track.identities[sample].2,
                            rocket.identities[index].2,
                        ))
                    .then(|| rocket_values.as_ref().expect("Rocket values")[index])
                    .flatten()
                });
                add_units(equipment, rocket)
            })
            .collect::<io::Result<Vec<_>>>()?;
        let runs = track
            .hours
            .iter()
            .enumerate()
            .map(|(sample, first)| CountryTotalRun {
                first_hour: *first,
                last_hour: track.last_hours[sample],
                amount: values[sample],
                starts_segment: track.starts[sample] || sample > 0 && base[sample - 1].is_none(),
            })
            .collect();
        out.insert(tag.clone(), runs);
    }
    Ok(out)
}

/// Counted recording limitations shared by every consumer of delivered totals.
/// Content/type diagnostics remain with the consumer that resolves those facts.
pub fn recording_warnings(scanned: &ScannedProduced) -> Vec<String> {
    recording_warnings_with_unnamed(scanned).0
}

/// The insertion point lets endpoint pages retain hourly evidence instead of text.
pub(crate) fn recording_warnings_with_unnamed(scanned: &ScannedProduced) -> (Vec<String>, usize) {
    let mut warnings = scanned.variants.warnings();
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
            "{} lines produced in ticks whose purpose was not recorded; what they delivered counts as new production.",
            scanned.unlisted
        ));
    }
    if scanned.observations.len() > 1 {
        warnings.push(format!(
            "The production output observation restarted {} times in this branch; running totals start again at each.",
            scanned.observations.len() - 1
        ));
    }
    let unnamed_index = warnings.len();
    warnings.extend(unnamed_recording_warning(scanned.unnamed));
    if scanned.rocket.reads.is_empty() {
        warnings.push("Rocket delivery coverage is not recorded or readable; combined production totals are not known. Independently known equipment-line subtotals remain available.".into());
    } else if scanned.rocket.unread > 0 {
        warnings.push(format!("{} Rocket publications lack delivery coverage; combined quantities at those reads are not known.", scanned.rocket.unread));
    }
    if scanned.rocket.unattributed > 0 {
        warnings.push(format!("{} retained Rocket rows have no identified producing country; their delivery amounts are not reassigned.", scanned.rocket.unattributed));
    }
    if scanned.rocket.inconsistent > 0 {
        warnings.push(format!("{} retained Rocket rows regressed counters or changed amounts without new events; the affected scope's measure stays unknown.", scanned.rocket.inconsistent));
    }
    (warnings, unnamed_index)
}

pub(crate) fn unnamed_recording_warning(unnamed: usize) -> Option<String> {
    (unnamed > 0)
        .then(|| format!("{unnamed} country-hours have a tag that is not text; they are left out."))
}

/// Each group's total at the samples where it changed, from the entries in it
/// (`group_of` gives an entry's group, none leaves it out): its units (or applied
/// IC, fixed5) and whether every row behind it is exact.
fn group_totals(
    track: &TagHistory,
    count: ProducedCount,
    group_of: impl Fn(&Entry) -> Option<usize>,
    groups: usize,
) -> io::Result<Vec<Vec<(u32, i128, bool)>>> {
    let mut events: Vec<(u32, usize, &Entry, Total)> = Vec::new();
    match count {
        ProducedCount::Units => {
            for (entry, list) in &track.changes {
                if let Some(group) = group_of(entry) {
                    events.extend(
                        list.iter()
                            .map(|(sample, total)| (*sample, group, entry, *total)),
                    );
                }
            }
        }
        ProducedCount::Ic => {
            // An applied total with a reason stays unknown: its amount stopped
            // counting then, so it never stands as a partial sum.
            for (entry, list) in &track.cost_changes {
                if let Some(group) = group_of(entry) {
                    events.extend(list.iter().map(|(sample, cost)| {
                        (
                            *sample,
                            group,
                            entry,
                            Total {
                                units: cost.raw,
                                inexact: u32::from(cost.reason.is_some()),
                            },
                        )
                    }));
                }
            }
        }
    }
    events.sort_by_key(|(sample, group, _, _)| (*sample, *group));
    let mut current: HashMap<&Entry, Total> = HashMap::new();
    let mut sums = vec![(0i128, 0u32); groups];
    let mut out: Vec<Vec<(u32, i128, bool)>> = vec![Vec::new(); groups];
    let mut index = 0;
    while index < events.len() {
        let sample = events[index].0;
        let mut touched = Vec::new();
        while index < events.len() && events[index].0 == sample {
            let (_, group, entry, total) = events[index];
            let before = current.insert(entry, total).unwrap_or_default();
            let sum = &mut sums[group];
            sum.0 = total
                .units
                .checked_sub(before.units)
                .and_then(|delta| sum.0.checked_add(delta))
                .ok_or_else(|| {
                    io::Error::other("Produced equipment group exceeds its exact integer range")
                })?;
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
    Ok(out)
}

/// A group's total at a sample: its latest change at or before it, else none made yet.
fn total_at(changes: &[(u32, i128, bool)], sample: usize) -> (i128, bool) {
    let position = changes.partition_point(|(at, _, _)| (*at as usize) <= sample);
    position
        .checked_sub(1)
        .map_or((0, true), |index| (changes[index].1, changes[index].2))
}

/// For each sample, the sample the measure subtracts (itself for a running
/// total); none where the value is not known: at or after the hour its
/// observation's totals became unknown (`tainted`), or, for a day, a week or a month,
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
                ProducedMeasure::Day => track.hours[sample].checked_sub(DAY_HOURS)?,
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
    changes: &[(u32, i128, bool)],
    sample: usize,
    base: usize,
    total: bool,
) -> io::Result<Option<i64>> {
    let (units, exact) = total_at(changes, sample);
    if !exact {
        return Ok(None);
    }
    let value = if total {
        units
    } else {
        units
            .checked_sub(total_at(changes, base).0)
            .ok_or_else(|| {
                io::Error::other("Produced equipment range exceeds its exact integer range")
            })?
    };
    i64::try_from(value)
        .map(Some)
        .map_err(|_| io::Error::other("Produced equipment units exceed their exact integer range"))
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
/// deltas after it stay relative to the last known value. Both samples and emitted
/// deltas must fit the reply's exact JavaScript integers.
fn encode(values: &[Option<i64>]) -> io::Result<Option<ProducedCoded>> {
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
        return Ok(None);
    }
    let mut previous = 0i64;
    let mut deltas: Vec<i64> = values[from..]
        .iter()
        .map(|value| match value {
            Some(value) => {
                if value.unsigned_abs() > MAX_EXACT as u64 {
                    return Err(io::Error::other(
                        "A production total is too large to reply exactly",
                    ));
                }
                let delta = i128::from(*value) - i128::from(previous);
                if delta.unsigned_abs() > MAX_EXACT as u128 {
                    return Err(io::Error::other(
                        "A production delta is too large to reply exactly",
                    ));
                }
                previous = *value;
                Ok(delta as i64)
            }
            None => Ok(0),
        })
        .collect::<io::Result<_>>()?;
    while deltas.last() == Some(&0) {
        deltas.pop();
    }
    Ok(Some(ProducedCoded {
        from: from as u32,
        deltas,
        unknown,
    }))
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
    count: ProducedCount,
    counted: impl Fn(&Entry) -> bool,
) -> io::Result<Vec<Option<i64>>> {
    let sums = group_totals(track, count, |entry| counted(entry).then_some(0), 1)?;
    bases
        .iter()
        .enumerate()
        .map(|(sample, base)| match base {
            Some(base) => measure_at(&sums[0], sample, *base, total),
            None => Ok(None),
        })
        .collect()
}

/// Digits and a comma: at least as many bytes as a number takes serialised.
fn digits(value: i64) -> usize {
    value.to_string().len() + 1
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
            && (params.refits || entry.1 != Purpose::Refit)
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
            && (params.refits || entry.1 != Purpose::Refit)
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
    let entries: BTreeSet<Entry> =
        scanned
            .tags
            .values()
            .flat_map(|track| counted_entries(track, params.count))
            .chain(scanned.rocket.tags.values().flat_map(|track| {
                counted_entries(track, params.count).filter(|entry| entry.0 != 0)
            }))
            .filter(|entry| params.refits || entry.1 != Purpose::Refit)
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

/// The entries a count reads in one country's history: those that delivered
/// units, and for applied IC also those still being built (input applied, no unit
/// delivered yet).
fn counted_entries(track: &TagHistory, count: ProducedCount) -> impl Iterator<Item = Entry> + '_ {
    let applied = match count {
        ProducedCount::Units => None,
        ProducedCount::Ic => Some(track.cost_changes.keys()),
    };
    track
        .changes
        .keys()
        .chain(applied.into_iter().flatten())
        .copied()
}

/// Every variant a count reads, over every country's equipment lines and Rocket
/// designs (the Rocket scope rows, variant zero, are no variant).
fn counted_variants(scanned: &ScannedProduced, count: ProducedCount) -> BTreeSet<u64> {
    scanned
        .tags
        .values()
        .chain(scanned.rocket.tags.values())
        .flat_map(|track| counted_entries(track, count))
        .map(|(variant, _)| variant)
        .filter(|variant| *variant != 0)
        .collect()
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
) -> io::Result<Vec<Option<i64>>> {
    let total = params.measure == ProducedMeasure::Total;
    let sums = measured(track, bases, total, params.count, |entry| {
        grouped.of.contains_key(entry)
    })?;
    let outside = holding(&measured(track, bases, total, params.count, |entry| {
        grouped.left_out(params, entry)
    })?);
    Ok(sums
        .into_iter()
        .zip(outside)
        .map(|(sum, outside)| sum.filter(|_| !outside))
        .collect())
}

/// The series a request asks for and the keys they hold.
struct RocketMeasures {
    sums: Vec<Vec<(u32, i128, bool)>>,
    bases: Vec<Option<usize>>,
    indices: Vec<Option<usize>>,
    unsure: Vec<bool>,
    totals: Vec<Option<i64>>,
    total: bool,
}

impl RocketMeasures {
    fn group_at(&self, key: usize, sample: usize, ambiguous: bool) -> io::Result<Option<i64>> {
        let Some(index) = self.indices[sample] else {
            return Ok(None);
        };
        if ambiguous && self.unsure[index] {
            return Ok(None);
        }
        measure_at(
            &self.sums[key],
            index,
            self.bases[index].expect("covered Rocket base"),
            self.total,
        )
    }

    fn total_at(&self, sample: usize) -> Option<i64> {
        self.indices[sample].and_then(|index| self.totals[index])
    }
}

fn rocket_measures(
    scanned: &ScannedProduced,
    tag: &str,
    equipment: &TagHistory,
    grouped: &Grouped,
    params: &ProducedParams,
) -> io::Result<RocketMeasures> {
    let count = equipment.hours.len();
    let mut result = RocketMeasures {
        sums: vec![Vec::new(); grouped.keys.len()],
        bases: Vec::new(),
        indices: vec![None; count],
        unsure: Vec::new(),
        totals: Vec::new(),
        total: params.measure == ProducedMeasure::Total,
    };
    let Some(rocket) = scanned.rocket.tags.get(tag) else {
        return Ok(result);
    };
    let bases = bases(rocket, params.measure, &BTreeMap::new());
    let total = params.measure == ProducedMeasure::Total;
    let scope = measured(rocket, &bases, total, params.count, |entry| entry.0 == 0)?;
    let sums = group_totals(
        rocket,
        params.count,
        |entry| grouped.of.get(entry).copied(),
        grouped.keys.len(),
    )?;
    let unsure = holding(&measured(rocket, &bases, total, params.count, |entry| {
        grouped.could_belong(params, entry)
    })?);
    let totals = included_total(rocket, &bases, grouped, params)?;
    for (sample, &hour) in equipment.hours.iter().enumerate() {
        let position = rocket.hours.partition_point(|read| *read <= hour);
        let Some(index) = position.checked_sub(1) else {
            continue;
        };
        if rocket.last_hours[index] < equipment.last_hours[sample]
            || (rocket.identities[index].0, rocket.identities[index].1)
                != (
                    equipment.identities[sample].0,
                    equipment.identities[sample].1,
                )
            || !same_origin(
                scanned,
                equipment.identities[sample].2,
                rocket.identities[index].2,
            )
            || scope[index].is_none()
        {
            continue;
        }
        if bases[index].is_none() {
            continue;
        }
        result.indices[sample] = Some(index);
    }
    result.sums = sums;
    result.bases = bases;
    result.unsure = unsure;
    result.totals = totals;
    Ok(result)
}

fn same_origin(scanned: &ScannedProduced, equipment: u32, rocket: u32) -> bool {
    match (
        scanned.observations.get(equipment as usize),
        scanned.rocket.observations.get(rocket as usize),
    ) {
        (Some(equipment), Some(rocket)) => equipment == rocket,
        // A complete source with no events has no row counting origin yet.
        _ => true,
    }
}

fn add_units(equipment: Option<i64>, rocket: Option<i64>) -> io::Result<Option<i64>> {
    match (equipment, rocket) {
        (Some(equipment), Some(rocket)) => {
            equipment.checked_add(rocket).map(Some).ok_or_else(|| {
                io::Error::other("Combined delivered units exceed their exact integer range")
            })
        }
        _ => Ok(None),
    }
}

/// The hour from which each observation's values are unknown for a count: the
/// delivered totals' lost publications, or for applied IC also an input the
/// recording does not hold (`ScannedProduced::spending_tainted`).
fn taints(scanned: &ScannedProduced, count: ProducedCount) -> BTreeMap<u32, i32> {
    match count {
        ProducedCount::Units => scanned.tainted.clone(),
        ProducedCount::Ic => scanned
            .spending_tainted
            .iter()
            .map(|(observation, (hour, _))| (*observation, *hour))
            .collect(),
    }
}

fn requested(
    scanned: &ScannedProduced,
    grouped: &Grouped,
    params: &ProducedParams,
    cancelled: &AtomicBool,
) -> io::Result<Requested> {
    let total = params.measure == ProducedMeasure::Total;
    let tainted = taints(scanned, params.count);
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
                    let bases = bases(track, params.measure, &tainted);
                    let rocket = rocket_measures(scanned, tag, track, grouped, params)?;
                    let totals = included_total(track, &bases, grouped, params)?
                        .into_iter()
                        .enumerate()
                        .map(|(sample, e)| add_units(e, rocket.total_at(sample)))
                        .collect::<io::Result<Vec<_>>>()?;
                    let runs = pieces(track, &bases)
                        .into_iter()
                        .map(|piece| {
                            let value = totals[piece.sample];
                            coverage.record(&piece, value);
                            let value = value.filter(|value| *value != 0);
                            (piece.first, piece.last, value)
                        })
                        .collect();
                    Ok((tag.clone(), runs))
                })
                .collect::<io::Result<_>>()?;
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
        let bases = bases(track, params.measure, &tainted);
        let all = pieces(track, &bases);
        let pieces = match &membership {
            Some(spans) => clip(&all, spans),
            None => all,
        };
        let groups = group_totals(
            track,
            params.count,
            |entry| grouped.of.get(entry).copied(),
            grouped.keys.len(),
        )?;
        let rocket = rocket_measures(scanned, tag, track, grouped, params)?;
        // Where a variant of an unresolved type holds anything: every resolved type or
        // arm it could belong to is unknown then.
        let unsure = holding(&measured(track, &bases, total, params.count, |entry| {
            grouped.could_belong(params, entry)
        })?);
        let any_unsure = pieces.iter().any(|piece| unsure[piece.sample]);
        let mut values = Vec::new();
        for (key, changes) in groups.iter().enumerate() {
            let ambiguous = grouped.ambiguous[key] && any_unsure;
            if changes.is_empty()
                && !ambiguous
                && pieces
                    .iter()
                    .map(|piece| rocket.group_at(key, piece.sample, grouped.ambiguous[key]))
                    .collect::<io::Result<Vec<_>>>()?
                    .iter()
                    .all(|value| *value == Some(0))
            {
                continue;
            }
            let equipment_at: Vec<Option<i64>> = pieces
                .iter()
                .map(|piece| {
                    if ambiguous && unsure[piece.sample] {
                        return Ok(None);
                    }
                    measure_at(
                        changes,
                        piece.sample,
                        bases[piece.sample].unwrap_or(piece.sample),
                        total,
                    )
                })
                .collect::<io::Result<_>>()?;
            let at: Vec<Option<i64>> = pieces
                .iter()
                .zip(&equipment_at)
                .map(|(piece, &value)| {
                    add_units(
                        value,
                        rocket.group_at(key, piece.sample, grouped.ambiguous[key])?,
                    )
                })
                .collect::<io::Result<_>>()?;
            let equipment_values = if at
                .iter()
                .zip(&equipment_at)
                .any(|(combined, equipment)| combined.is_none() && equipment.is_some())
            {
                // Exact zero has no coded column; an explicit empty code still
                // distinguishes the available subtotal from an absent one.
                Some(encode(&equipment_at)?.unwrap_or_default())
            } else {
                None
            };
            if let Some(coded) = encode(&at)? {
                used[key] = true;
                bytes += coded_bytes(&coded)
                    + equipment_values
                        .as_ref()
                        .map_or(0, |values| 24 + coded_bytes(values));
                values.push(ProducedValues {
                    key: key as u32,
                    values: coded,
                    equipment_values,
                });
            }
        }
        let totals = included_total(track, &bases, grouped, params)?
            .into_iter()
            .enumerate()
            .map(|(sample, e)| add_units(e, rocket.total_at(sample)))
            .collect::<io::Result<Vec<_>>>()?;
        if membership.is_none() {
            for piece in &pieces {
                coverage.record(piece, totals[piece.sample]);
            }
        }
        let total_at: Vec<Option<i64>> = pieces.iter().map(|piece| totals[piece.sample]).collect();
        let total_coded = encode(&total_at)?;
        bytes += total_coded.as_ref().map_or(0, coded_bytes);
        let item = ProducedSeries {
            tag: tag.clone(),
            identities: None,
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
    let (types, untyped, unresolved) = variant_types_for(
        &scanned.variants,
        counted_variants(scanned, params.count),
        named,
    );
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
    for reason in &untyped {
        warnings.push(format!("An equipment type cannot be resolved: {reason}."));
    }
    warnings.extend(recording_warnings(scanned));
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
        count: params.count,
        decimals: match params.count {
            ProducedCount::Units => 0,
            ProducedCount::Ic => equipment::EQUIPMENT_DECIMALS,
        },
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

impl Product for EquipmentProduced {
    const KEY: &'static str = "equipment-produced";
    const CONTENT_DEPENDENT: bool = true;
    type Params = ProducedParams;
    type Reply = EquipmentProducedReply;

    fn types() -> Vec<String> {
        vec![
            ProducedParams::decl(),
            ProducedMeasure::decl(),
            ProducedCount::decl(),
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
        for tag in &params.tags {
            validate_tag(tag)?;
        }
        if let Some(count) = params.top
            && (!params.tags.is_empty() || count == 0)
        {
            return Err("Invalid top countries");
        }
        if params.included.iter().any(|key| key.is_empty()) {
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
        let scanned = equipment::production::scanned(source, branch, cancelled)?;
        let mut warnings = Vec::new();
        let (catalogue, units) = equipment::production::content(source, context, &mut warnings);
        let named = Named {
            units: units.as_deref(),
            texts: catalogue
                .as_ref()
                .map(|catalogue| &**catalogue as &dyn Texts),
        };
        let mut projection = params.clone();
        let histories = if params.top.is_none() && !params.tags.is_empty() {
            let history =
                super::country_history::branch_histories(source, branch, false, &[], cancelled)?;
            projection.tags = super::country_history::selected_aliases(&history, &params.tags);
            // Alias projection leaves out requested tags without a recorded holder.
            let missing: BTreeSet<_> = params
                .tags
                .iter()
                .filter(|tag| {
                    !history.countries.contains_key(*tag) && !scanned.tags.contains_key(*tag)
                })
                .collect();
            for tag in missing {
                warnings.push(format!("{tag}: no recorded production in this branch"));
            }
            Some(history)
        } else {
            None
        };
        let mut reply = reply(&scanned, &projection, named, warnings, cancelled)?;
        if let Some(history) = histories {
            let metadata = projection
                .tags
                .iter()
                .map(|tag| {
                    128 + tag.len()
                        + history
                            .countries
                            .get(tag)
                            .map_or(0, |country| 24 + country.identities.len() * 128)
                })
                .sum::<usize>();
            if crate::transport::json_len(&reply, MAX_REPLY_BYTES)
                .is_none_or(|bytes| bytes.saturating_add(metadata) > MAX_REPLY_BYTES)
            {
                return Err(io::Error::other(
                    "These countries' production identities and history exceed one reply",
                ));
            }
            for tag in &projection.tags {
                if !reply.series.iter().any(|item| item.tag == *tag) {
                    reply.series.push(ProducedSeries {
                        tag: tag.clone(),
                        ..Default::default()
                    });
                }
            }
            for item in &mut reply.series {
                item.identities = Some(
                    history
                        .countries
                        .get(&item.tag)
                        .map_or_else(Vec::new, |country| country.identities.as_ref().clone()),
                );
            }
        }
        if let Some(catalogue) = catalogue {
            reply.warnings.extend(catalogue.refusal_lines());
        }
        super::check_reply_budget(&reply).map_err(|_| {
            io::Error::other("These countries' production history exceeds one exact reply")
        })?;
        Ok(reply)
    }
}

#[cfg(test)]
use replay_data::rows::TableId;
#[cfg(test)]
use replay_frame::frame::TableView;
#[cfg(test)]
use std::sync::Arc;
#[cfg(test)]
pub(crate) mod tests;
