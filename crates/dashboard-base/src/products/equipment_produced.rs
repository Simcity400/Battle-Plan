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
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct EquipmentProduced;

/// The most a reply's series may take serialised: well inside the transport's
/// 32 MiB with the keys and the envelope.
pub use super::MAX_PRODUCT_REPLY_BYTES as MAX_REPLY_BYTES;
/// JSON of one series and of one key's values besides their numbers.
const SERIES_OVERHEAD: usize = 128;
const VALUES_OVERHEAD: usize = 64;
/// The bytes one summary bucket takes serialised, at most.
const SUMMARY_BUCKET_BYTES: usize = 160;
/// The largest magnitude a reply's numbers carry exactly (JavaScript's safe integers).
const MAX_EXACT: i64 = (1 << 53) - 1;
const DAY_HOURS: i32 = 24;
/// Groups whose totals add up by sample in place rather than by sorted changes.
const DENSE_GROUPS: usize = 2;
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
    /// The chart's view: only the samples it shows, and where they are denser than its width,
    /// each line as a summary on the shared chart grid; none, every sample.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub view: Option<SeriesView>,
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
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, TS)]
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
    /// Where the view's samples are denser than its width: the total over the view on the
    /// shared chart grid, every sample's exact value held in its bucket; the samples are
    /// then the last one shown and the last with a known total alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub summary: Option<ChartSummary>,
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
use crate::prepared_chart::ChartSummary;
use crate::prepared_production::history::{CountPlan, Counted};
use crate::series_view::SeriesView;
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
    recording_warnings_of(
        scanned.variants.warnings(),
        &RecordingCounts {
            reads: !scanned.reads.is_empty(),
            unread: scanned.unread,
            unread_reason: scanned.unread_reason.clone(),
            tainted_from: scanned.tainted.iter().next().map(|(_, from)| *from),
            missed: scanned.missed,
            inexact: scanned.inexact,
            lost: scanned.lost,
            unattributed: scanned.unattributed,
            unlisted: scanned.unlisted,
            observations: scanned.observations.len(),
            unnamed: scanned.unnamed,
            rocket_reads: !scanned.rocket.reads.is_empty(),
            rocket_unread: scanned.rocket.unread,
            rocket_unattributed: scanned.rocket.unattributed,
            rocket_inconsistent: scanned.rocket.inconsistent,
        },
    )
}

/// What [`recording_warnings_of`] reports of a recording's production reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RecordingCounts {
    pub reads: bool,
    pub unread: usize,
    pub unread_reason: Option<String>,
    /// The hour totals of the first tainted observation are unknown from.
    pub tainted_from: Option<i32>,
    pub missed: usize,
    pub inexact: usize,
    pub lost: usize,
    pub unattributed: usize,
    pub unlisted: usize,
    pub observations: usize,
    pub unnamed: usize,
    pub rocket_reads: bool,
    pub rocket_unread: usize,
    pub rocket_unattributed: usize,
    pub rocket_inconsistent: usize,
}

/// `warnings`, then the counted recording limitations, and where the unnamed
/// holders' warning goes.
pub(crate) fn recording_warnings_of(
    mut warnings: Vec<String>,
    counts: &RecordingCounts,
) -> (Vec<String>, usize) {
    if !counts.reads {
        warnings.push(match &counts.unread_reason {
            Some(reason) => format!(
                "Equipment produced is not in this recording: the production output was not recorded ({reason})."
            ),
            None => "Equipment produced is not in this recording: it has no production output (ProductionLineOutput).".to_owned(),
        });
    } else if counts.unread > 0 {
        warnings.push(format!(
            "{} reads of the production lines have no readable output{}; every total is unknown from the first of them on (an ended line's final count is published only once).",
            counts.unread,
            counts
                .unread_reason
                .as_ref()
                .map(|reason| format!(" ({reason})"))
                .unwrap_or_default()
        ));
    }
    if let Some(from) = counts.tainted_from {
        warnings.push(format!(
            "{} recorded frames, reads or output rows could not be ingested (a frame unreadable, a read without complete countries, links or output, or units of a producer the links cannot tell); what an output publication held is published only once, so every total is unknown from hour {from} on, until the collector's observation restarts.",
            counts.missed,
        ));
    }
    if counts.inexact > 0 {
        warnings.push(format!(
            "{} production output rows left out a tick the collector could not count; their country's total of that variant is unknown from then on.",
            counts.inexact
        ));
    }
    if counts.lost > 0 {
        warnings.push(format!(
            "{} production output rows disappeared without their final count; their country's total of that variant is unknown from then on.",
            counts.lost
        ));
    }
    if counts.unattributed > 0 {
        warnings.push(format!(
            "{} production output rows name a producer the links do not resolve to a tagged country; they are left out.",
            counts.unattributed
        ));
    }
    if counts.unlisted > 0 {
        warnings.push(format!(
            "{} lines produced in ticks whose purpose was not recorded; what they delivered counts as new production.",
            counts.unlisted
        ));
    }
    if counts.observations > 1 {
        warnings.push(format!(
            "The production output observation restarted {} times in this branch; running totals start again at each.",
            counts.observations - 1
        ));
    }
    let unnamed_index = warnings.len();
    warnings.extend(unnamed_recording_warning(counts.unnamed));
    if !counts.rocket_reads {
        warnings.push("Rocket delivery coverage is not recorded or readable; combined production totals are not known. Independently known equipment-line subtotals remain available.".into());
    } else if counts.rocket_unread > 0 {
        warnings.push(format!("{} Rocket publications lack delivery coverage; combined quantities at those reads are not known.", counts.rocket_unread));
    }
    if counts.rocket_unattributed > 0 {
        warnings.push(format!("{} retained Rocket rows have no identified producing country; their delivery amounts are not reassigned.", counts.rocket_unattributed));
    }
    if counts.rocket_inconsistent > 0 {
        warnings.push(format!("{} retained Rocket rows regressed counters or changed amounts without new events; the affected scope's measure stays unknown.", counts.rocket_inconsistent));
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
    let overflow =
        || io::Error::other("Produced equipment group exceeds its exact integer range");
    // A few groups add up by sample in place; more sort their changes by sample.
    let dense = groups <= DENSE_GROUPS;
    let last = |lists: &mut dyn Iterator<Item = (&Entry, Option<u32>)>| {
        lists
            .filter(|(entry, _)| group_of(entry).is_some())
            .filter_map(|(_, last)| last)
            .max()
    };
    let samples = if !dense {
        0
    } else {
        match count {
            ProducedCount::Units => last(
                &mut track
                    .changes
                    .iter()
                    .map(|(entry, list)| (entry, list.last().map(|change| change.0))),
            ),
            ProducedCount::Ic => last(
                &mut track
                    .cost_changes
                    .iter()
                    .map(|(entry, list)| (entry, list.last().map(|change| change.0))),
            ),
        }
        .map_or(0, |sample| sample as usize + 1)
    };
    let mut cells: Vec<(i128, i64, bool)> = if dense {
        vec![(0, 0, false); groups * samples]
    } else {
        Vec::new()
    };
    // Each change as (sample, group, units, inexact rows) less its entry's total before it.
    let mut events: Vec<(u32, usize, i128, i64)> = Vec::new();
    let mut add = |group: usize, totals: &mut dyn Iterator<Item = (u32, Total)>| {
        let mut before = Total::default();
        for (sample, total) in totals {
            let units = total.units.checked_sub(before.units).ok_or_else(overflow)?;
            let inexact = i64::from(total.inexact) - i64::from(before.inexact);
            if dense {
                let cell = &mut cells[group * samples + sample as usize];
                cell.0 = cell.0.checked_add(units).ok_or_else(overflow)?;
                cell.1 += inexact;
                cell.2 = true;
            } else {
                events.push((sample, group, units, inexact));
            }
            before = total;
        }
        io::Result::Ok(())
    };
    match count {
        ProducedCount::Units => {
            for (entry, list) in &track.changes {
                if let Some(group) = group_of(entry) {
                    add(group, &mut list.iter().copied())?;
                }
            }
        }
        ProducedCount::Ic => {
            // An applied total with a reason stays unknown: its amount stopped
            // counting then, so it never stands as a partial sum.
            for (entry, list) in &track.cost_changes {
                if let Some(group) = group_of(entry) {
                    add(
                        group,
                        &mut list.iter().map(|(sample, cost)| {
                            (
                                *sample,
                                Total {
                                    units: cost.raw,
                                    inexact: u32::from(cost.reason.is_some()),
                                },
                            )
                        }),
                    )?;
                }
            }
        }
    }
    let mut out: Vec<Vec<(u32, i128, bool)>> = vec![Vec::new(); groups];
    let mut note = |group: usize, value: (u32, i128, bool)| {
        let list: &mut Vec<(u32, i128, bool)> = &mut out[group];
        if list
            .last()
            .is_none_or(|last| (last.1, last.2) != (value.1, value.2))
        {
            list.push(value);
        }
    };
    if dense {
        for group in 0..groups {
            let mut sum = (0i128, 0i64);
            for (sample, cell) in cells[group * samples..(group + 1) * samples]
                .iter()
                .enumerate()
            {
                if !cell.2 {
                    continue;
                }
                sum.0 = sum.0.checked_add(cell.0).ok_or_else(overflow)?;
                sum.1 += cell.1;
                note(group, (sample as u32, sum.0, sum.1 == 0));
            }
        }
        return Ok(out);
    }
    events.sort_unstable_by_key(|(sample, group, _, _)| (*sample, *group));
    let mut sums = vec![(0i128, 0i64); groups];
    let mut index = 0;
    while index < events.len() {
        let (sample, group, _, _) = events[index];
        let sum = &mut sums[group];
        while let Some(&(_, _, units, inexact)) = events
            .get(index)
            .filter(|event| (event.0, event.1) == (sample, group))
        {
            sum.0 = sum.0.checked_add(units).ok_or_else(overflow)?;
            sum.1 += inexact;
            index += 1;
        }
        note(group, (sample, sum.0, sum.1 == 0));
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
    // Hours rise, so each target is found on from the last; one behind it is searched for.
    let mut cursor = 0usize;
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
            if cursor > 0 && track.hours[cursor - 1] >= target {
                cursor = track.hours.partition_point(|hour| *hour < target);
            }
            while track.hours.get(cursor).is_some_and(|hour| *hour < target) {
                cursor += 1;
            }
            let base = cursor;
            (track.hours.get(base) == Some(&target)
                && track.identities[base] == identity
                && breaks[sample] == breaks[base])
                .then_some(base)
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

/// What a series shows at one of its samples: its hours, whether it starts a line, and the
/// piece it is.
#[derive(Clone, Debug)]
struct Unit {
    first: i32,
    last: i32,
    starts: bool,
    pieces: std::ops::Range<usize>,
}

/// The view a request's samples show in, on the shared chart grid from `bounds`, and whether
/// they are denser than its width.
#[derive(Clone, Copy)]
struct Shown {
    view: SeriesView,
    bounds: (i32, i32),
    dense: bool,
}

/// The units `pieces` show: each piece, those the view shows where there is one.
fn units(pieces: &[Piece], shown: Option<Shown>) -> Vec<Unit> {
    pieces
        .iter()
        .enumerate()
        .filter(|(_, piece)| shown.is_none_or(|shown| shown.view.shows(piece.first, piece.last)))
        .map(|(index, piece)| Unit {
            first: piece.first,
            last: piece.last,
            starts: piece.starts,
            pieces: index..index + 1,
        })
        .collect()
}

/// Units cut to spans: a unit keeps its line break only where it starts itself.
fn clip(units: &[Unit], spans: &[HourSpan]) -> Vec<Unit> {
    let mut out = Vec::new();
    let mut start = 0;
    for span in spans {
        while units
            .get(start)
            .is_some_and(|unit| unit.last < span.first_hour)
        {
            start += 1;
        }
        for unit in units[start..]
            .iter()
            .take_while(|unit| unit.first <= span.last_hour)
        {
            let first = unit.first.max(span.first_hour);
            let last = unit.last.min(span.last_hour);
            if first <= last {
                out.push(Unit {
                    first,
                    last,
                    starts: unit.starts && first == unit.first,
                    pieces: unit.pieces.clone(),
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
    let mut steps = Steps::new(&sums[0]);
    bases
        .iter()
        .enumerate()
        .map(|(sample, base)| match base {
            Some(base) => steps.measure(sample, *base, total),
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
    /// By token class of a prepared history: whether its type is counted, and whether it is
    /// an unresolved type left out that could belong to an included one.
    class_counted: Vec<bool>,
    class_outside: Vec<bool>,
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
                    // Named once the reply keeps it.
                    names: Vec::new(),
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
    let includes_resolved = types
        .values()
        .any(|kind| kind.ordinal.is_some() && params.included.contains(&kind.key));
    let (class_counted, class_outside) = scanned
        .class_names
        .iter()
        .map(|name| {
            let (kind, _) = match name.strip_prefix('#').and_then(|handle| handle.parse::<u64>().ok()) {
                Some(handle) => super::equipment::production::variant_type("", handle, named),
                None => super::equipment::production::variant_type(name, 0, named),
            };
            let counted = params.included.is_empty() || params.included.contains(&kind.key);
            (counted, includes_resolved && kind.ordinal.is_none() && !counted)
        })
        .unzip();
    Grouped {
        keys: groups.into_values().collect(),
        of,
        ambiguous,
        unresolved: types
            .iter()
            .filter(|(_, kind)| kind.ordinal.is_none())
            .map(|(variant, _)| *variant)
            .collect(),
        includes_resolved,
        class_counted,
        class_outside,
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
    // Counted entries and left-out ones are apart: an entry the grouping holds is not left out.
    let sums = group_totals(
        track,
        params.count,
        |entry| {
            if grouped.of.contains_key(entry) {
                Some(0)
            } else {
                grouped.left_out(params, entry).then_some(1)
            }
        },
        2,
    )?;
    let (mut counted, mut outside) = (Steps::new(&sums[0]), Steps::new(&sums[1]));
    bases
        .iter()
        .enumerate()
        .map(|(sample, base)| match base {
            Some(base) => {
                let sum = counted.measure(sample, *base, total)?;
                let outside = outside.measure(sample, *base, total)?;
                Ok(sum.filter(|_| outside == Some(0)))
            }
            None => Ok(None),
        })
        .collect()
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

/// `groups`: whether each group's measure is read, or only the totals.
fn rocket_measures(
    scanned: &ScannedProduced,
    rockets: &BTreeMap<String, TagHistory>,
    tag: &str,
    equipment: &TagHistory,
    grouped: &Grouped,
    params: &ProducedParams,
    groups: bool,
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
    let Some(rocket) = rockets.get(tag) else {
        return Ok(result);
    };
    let bases = bases(rocket, params.measure, &BTreeMap::new());
    let total = params.measure == ProducedMeasure::Total;
    let scope = measured(rocket, &bases, total, params.count, |entry| entry.0 == 0)?;
    let (sums, unsure) = if groups {
        (
            group_totals(
                rocket,
                params.count,
                |entry| grouped.of.get(entry).copied(),
                grouped.keys.len(),
            )?,
            holding(&measured(rocket, &bases, total, params.count, |entry| {
                grouped.could_belong(params, entry)
            })?),
        )
    } else {
        (Vec::new(), Vec::new())
    };
    let totals = included_total(rocket, &bases, grouped, params)?;
    let mut position = 0usize;
    for (sample, &hour) in equipment.hours.iter().enumerate() {
        if position > 0 && rocket.hours[position - 1] > hour {
            position = rocket.hours.partition_point(|read| *read <= hour);
        }
        while rocket.hours.get(position).is_some_and(|read| *read <= hour) {
            position += 1;
        }
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

/// The top N's members and the hours each is one, and the coverage of every country's
/// included total over the hours the view shows.
pub(crate) struct Ranked {
    members: BTreeMap<String, Vec<HourSpan>>,
    coverage: Coverage,
}

/// [`included_total`] from a prepared history's counted totals.
fn counted_total(
    counted: &Counted,
    bases: &[Option<usize>],
    total: bool,
) -> io::Result<Vec<Option<i64>>> {
    let (mut sums, mut outside) = (Steps::new(&counted.sums), Steps::new(&counted.outside));
    bases
        .iter()
        .enumerate()
        .map(|(sample, base)| match base {
            Some(base) => {
                let sum = sums.measure(sample, *base, total)?;
                let left = outside.measure(sample, *base, total)?;
                Ok(sum.filter(|_| left == Some(0)))
            }
            None => Ok(None),
        })
        .collect()
}

/// A group's totals read at rising samples, as [`measure_at`] reads them: each change found on
/// from the last read's.
struct Steps<'a> {
    changes: &'a [(u32, i128, bool)],
    at: usize,
    base: usize,
}

impl<'a> Steps<'a> {
    fn new(changes: &'a [(u32, i128, bool)]) -> Self {
        Self {
            changes,
            at: 0,
            base: 0,
        }
    }

    /// The changes at or before `sample`, found on from `from`.
    fn seek(changes: &[(u32, i128, bool)], from: usize, sample: usize) -> usize {
        let mut at = from;
        if at > 0 && changes[at - 1].0 as usize > sample {
            at = changes.partition_point(|(change, _, _)| (*change as usize) <= sample);
        }
        while changes.get(at).is_some_and(|(change, _, _)| *change as usize <= sample) {
            at += 1;
        }
        at
    }

    fn total(&self, position: usize) -> (i128, bool) {
        position
            .checked_sub(1)
            .map_or((0, true), |index| (self.changes[index].1, self.changes[index].2))
    }

    fn measure(&mut self, sample: usize, base: usize, total: bool) -> io::Result<Option<i64>> {
        self.at = Self::seek(self.changes, self.at, sample);
        let (units, exact) = self.total(self.at);
        if !exact {
            return Ok(None);
        }
        let value = if total {
            units
        } else {
            self.base = Self::seek(self.changes, self.base, base);
            units.checked_sub(self.total(self.base).0).ok_or_else(|| {
                io::Error::other("Produced equipment range exceeds its exact integer range")
            })?
        };
        i64::try_from(value)
            .map(Some)
            .map_err(|_| io::Error::other("Produced equipment units exceed their exact integer range"))
    }
}

/// Every country's included total, ranked at every hour. A country ranks only once it has
/// produced something (user decision, 29 Sep): a zero, like an unknown value, is not ranked.
#[allow(clippy::too_many_arguments, reason = "A scan's countries or a prepared history's")]
fn rank(
    scanned: &ScannedProduced,
    light: &BTreeMap<String, TagHistory>,
    rockets: &BTreeMap<String, TagHistory>,
    grouped: &Grouped,
    params: &ProducedParams,
    view: Option<Shown>,
    prepared: Option<&Ranking>,
    cancelled: &AtomicBool,
) -> io::Result<Ranked> {
    let count = params.top.unwrap_or_default();
    let shows = |piece: &Piece| view.is_none_or(|shown| shown.view.shows(piece.first, piece.last));
    let tainted = taints(scanned, params.count);
    let total = params.measure == ProducedMeasure::Total;
    let none = Ranking::default();
    let Ranking { counted, whole } = prepared.unwrap_or(&none);
    let tags: Vec<(&String, &TagHistory)> = light.iter().collect();
    let ranked = crate::workers::map(&tags, "equipment produced ranking", |&(tag, track)| {
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Equipment produced request cancelled",
            ));
        }
        let bases = bases(track, params.measure, &tainted);
        let rocket = rocket_measures(scanned, rockets, tag, track, grouped, params, false)?;
        let included = match (counted.get(tag), whole.get(tag)) {
            (Some(counted), _) => counted_total(counted, &bases, total)?,
            (None, Some(history)) => included_total(history, &bases, grouped, params)?,
            (None, None) if prepared.is_some() => {
                return Err(io::Error::other(format!(
                    "{tag}: the prepared history holds no totals to rank it by"
                )));
            }
            (None, None) => included_total(track, &bases, grouped, params)?,
        };
        let totals = included
            .into_iter()
            .enumerate()
            .map(|(sample, e)| add_units(e, rocket.total_at(sample)))
            .collect::<io::Result<Vec<_>>>()?;
        let pieces = pieces(track, &bases);
        let mut spans = Spans::default();
        for piece in pieces.iter().filter(|piece| shows(piece)) {
            spans.record(piece, totals[piece.sample].is_some());
        }
        // Unranked runs are left out and equal adjacent ones joined: the membership is
        // the same at every hour, and its spans join adjacent hours.
        let mut runs: Vec<(i32, i32, Option<i64>)> = Vec::new();
        for piece in &pieces {
            let Some(value) = totals[piece.sample].filter(|value| *value != 0) else {
                continue;
            };
            match runs.last_mut() {
                Some(run) if run.2 == Some(value) && run.1.checked_add(1) == Some(piece.first) => {
                    run.1 = piece.last;
                }
                _ => runs.push((piece.first, piece.last, Some(value))),
            }
        }
        Ok((runs, spans))
    })?;
    let mut coverage = Coverage::default();
    let mut series = BTreeMap::new();
    for ((tag, _), (runs, spans)) in tags.iter().zip(ranked) {
        coverage.take(spans);
        series.insert((*tag).clone(), runs);
    }
    let members = top_in_parts(&series, count as usize, &scanned.reads, cancelled)?;
    Ok(Ranked { members, coverage })
}

/// [`ranking::top_membership`] of `series` over parts of its hours at once: the membership at
/// an hour is of the runs holding it alone, and each country's spans join across the parts.
fn top_in_parts(
    series: &BTreeMap<String, Vec<(i32, i32, Option<i64>)>>,
    count: usize,
    reads: &[i32],
    cancelled: &AtomicBool,
) -> io::Result<BTreeMap<String, Vec<HourSpan>>> {
    let parts = std::thread::available_parallelism().map_or(1, |cores| (cores.get() / 2).max(1));
    let mut ranges: Vec<(i32, i32)> = Vec::with_capacity(parts);
    let mut low = i32::MIN;
    for part in 1..parts {
        if let Some(&bound) = reads.get(reads.len() * part / parts)
            && bound > low
        {
            ranges.push((low, bound - 1));
            low = bound;
        }
    }
    ranges.push((low, i32::MAX));
    let members = crate::workers::map(&ranges, "equipment produced ranking", |&(low, high)| {
        let part: BTreeMap<String, Vec<(i32, i32, Option<i64>)>> = series
            .iter()
            .filter_map(|(tag, runs)| {
                let start = runs.partition_point(|run| run.1 < low);
                let clipped: Vec<_> = runs[start..]
                    .iter()
                    .take_while(|run| run.0 <= high)
                    .map(|run| (run.0.max(low), run.1.min(high), run.2))
                    .collect();
                (!clipped.is_empty()).then(|| (tag.clone(), clipped))
            })
            .collect();
        ranking::top_membership(&part, count, |run| (run.0, run.1), |run| run.2, cancelled)
    })?;
    let mut out: BTreeMap<String, Vec<HourSpan>> = BTreeMap::new();
    for part in members {
        for (tag, spans) in part {
            let list = out.entry(tag).or_default();
            for span in spans {
                match list.last_mut() {
                    Some(last) if last.last_hour.checked_add(1) == Some(span.first_hour) => {
                        last.last_hour = span.last_hour;
                    }
                    _ => list.push(span),
                }
            }
        }
    }
    Ok(out)
}

fn requested(
    scanned: &ScannedProduced,
    grouped: &Grouped,
    params: &ProducedParams,
    view: Option<Shown>,
    ranked: Option<Ranked>,
    cancelled: &AtomicBool,
) -> io::Result<Requested> {
    requested_in(
        scanned,
        &scanned.tags,
        &scanned.rocket.tags,
        grouped,
        params,
        view,
        ranked,
        cancelled,
    )
}

/// [`requested`] with the shown countries' histories taken from `tags` and `rockets`.
#[allow(clippy::too_many_arguments, reason = "A scan's countries or a prepared history's")]
fn requested_in(
    scanned: &ScannedProduced,
    tags: &BTreeMap<String, TagHistory>,
    rockets: &BTreeMap<String, TagHistory>,
    grouped: &Grouped,
    params: &ProducedParams,
    view: Option<Shown>,
    ranked: Option<Ranked>,
    cancelled: &AtomicBool,
) -> io::Result<Requested> {
    let total = params.measure == ProducedMeasure::Total;
    let decimals = match params.count {
        ProducedCount::Units => 0,
        ProducedCount::Ic => equipment::EQUIPMENT_DECIMALS,
    };
    let tainted = taints(scanned, params.count);
    let mut coverage = Coverage::default();
    // The countries and the hours each is shown.
    let shown: Vec<(&String, &TagHistory, Option<Vec<HourSpan>>)> = match params.top {
        Some(_) => {
            let Ranked {
                members,
                coverage: ranked_coverage,
            } = match ranked {
                Some(ranked) => ranked,
                None => rank(
                    scanned,
                    &scanned.tags,
                    &scanned.rocket.tags,
                    grouped,
                    params,
                    view,
                    None,
                    cancelled,
                )?,
            };
            coverage = ranked_coverage;
            members
                .into_iter()
                .filter_map(|(tag, spans)| {
                    tags.get_key_value(&tag)
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
                    tags.get_key_value(tag)
                        .map(|(tag, track)| (tag, track, None))
                })
                .collect()
        }
    };
    // Each country's series on its own, then the reply's budget over them in order.
    let built = crate::workers::map(&shown, "equipment produced series", |(tag, track, membership)| {
        let (tag, track, membership) = (*tag, *track, membership.clone());
        let mut bytes = 0usize;
        let mut used: Vec<usize> = Vec::new();
        let mut spans = Spans::default();
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Equipment produced request cancelled",
            ));
        }
        let bases = bases(track, params.measure, &tainted);
        let pieces = pieces(track, &bases);
        let all = units(&pieces, view);
        let mut shown = match &membership {
            Some(spans) => clip(&all, spans),
            None => all,
        };
        let sample_of = |index: usize| pieces[index].sample;
        let rocket = rocket_measures(scanned, rockets, tag, track, grouped, params, true)?;
        let totals = included_total(track, &bases, grouped, params)?
            .into_iter()
            .enumerate()
            .map(|(sample, e)| add_units(e, rocket.total_at(sample)))
            .collect::<io::Result<Vec<_>>>()?;
        if membership.is_none() {
            for unit in &shown {
                spans.record(&pieces[unit.pieces.start], totals[sample_of(unit.pieces.start)].is_some());
            }
        }
        // Denser than the view's width: the total goes as its summary, every known sample's
        // exact value in its bucket, and the samples sent are the last shown and the last known.
        // Every value is still worked out and checked at every sample, as were they all sent.
        let mut kept: Option<Vec<usize>> = None;
        let summary = match view.filter(|shown| shown.dense) {
            Some(Shown { view, bounds, .. }) => {
                let runs: Vec<crate::prepared::series::Run> = shown
                    .iter()
                    .filter_map(|unit| {
                        totals[sample_of(unit.pieces.start)].map(|value| crate::prepared::series::Run {
                            first_hour: unit.first,
                            last_hour: unit.last,
                            value: i128::from(value),
                            break_before: unit.starts,
                        })
                    })
                    .collect();
                let mut summary = ChartSummary {
                    from: view.from,
                    to: view.to,
                    width: view.width,
                    buckets: Vec::new(),
                    runs: None,
                    approximate: Vec::new(),
                };
                crate::prepared::series::merge_tail(&mut summary, bounds, &runs, None, decimals, None)?;
                // The last sample shown stays, as every line's latest, and the last with a known
                // total, where the line's end label sits.
                let last = shown.len().checked_sub(1);
                let known = shown
                    .iter()
                    .rposition(|unit| totals[sample_of(unit.pieces.start)].is_some());
                kept = Some(
                    (0..shown.len())
                        .filter(|index| Some(*index) == last || Some(*index) == known)
                        .collect(),
                );
                bytes += SUMMARY_BUCKET_BYTES * summary.buckets.iter().flatten().count();
                Some(summary)
            }
            None => None,
        };
        // The pieces the shown units hold, each once.
        let mut held: Vec<usize> = shown.iter().flat_map(|unit| unit.pieces.clone()).collect();
        held.dedup();
        let groups = group_totals(
            track,
            params.count,
            |entry| grouped.of.get(entry).copied(),
            grouped.keys.len(),
        )?;
        // Where a variant of an unresolved type holds anything: every resolved type or
        // arm it could belong to is unknown then.
        let unsure = holding(&measured(track, &bases, total, params.count, |entry| {
            grouped.could_belong(params, entry)
        })?);
        let any_unsure = held.iter().any(|&index| unsure[sample_of(index)]);
        // A group without Rocket sums reads 0 at a sample whose Rocket read is joined, and sure
        // where the group is ambiguous: known once for every such group.
        let unjoined = held
            .iter()
            .any(|&index| rocket.indices[sample_of(index)].is_none());
        let unsure_joined = held.iter().any(|&index| {
            rocket.indices[sample_of(index)].is_some_and(|joined| rocket.unsure[joined])
        });
        let rocket_zero = |key: usize| -> io::Result<bool> {
            if rocket.sums[key].is_empty() {
                return Ok(!unjoined && !(grouped.ambiguous[key] && unsure_joined));
            }
            let mut zero = true;
            for &index in &held {
                zero &= rocket.group_at(key, sample_of(index), grouped.ambiguous[key])? == Some(0);
            }
            Ok(zero)
        };
        let mut values = Vec::new();
        for (key, changes) in groups.iter().enumerate() {
            let ambiguous = grouped.ambiguous[key] && any_unsure;
            if changes.is_empty() && !ambiguous && rocket_zero(key)? {
                continue;
            }
            let equipment_of = |index: usize| {
                let sample = sample_of(index);
                if ambiguous && unsure[sample] {
                    return Ok(None);
                }
                measure_at(changes, sample, bases[sample].unwrap_or(sample), total)
            };
            let equipment_at: Vec<Option<i64>> = shown
                .iter()
                .map(|unit| equipment_of(unit.pieces.start))
                .collect::<io::Result<_>>()?;
            let at: Vec<Option<i64>> = shown
                .iter()
                .zip(&equipment_at)
                .map(|(unit, &value)| {
                    add_units(
                        value,
                        rocket.group_at(key, sample_of(unit.pieces.start), grouped.ambiguous[key])?,
                    )
                })
                .collect::<io::Result<_>>()?;
            let (equipment_at, at) = match &kept {
                Some(kept) => {
                    // Refused where every sample's values would be, as were they all sent.
                    encode(&at)?;
                    if at
                        .iter()
                        .zip(&equipment_at)
                        .any(|(combined, equipment)| combined.is_none() && equipment.is_some())
                    {
                        encode(&equipment_at)?;
                    }
                    let pick = |values: &[Option<i64>]| kept.iter().map(|&index| values[index]).collect::<Vec<_>>();
                    (pick(&equipment_at), pick(&at))
                }
                None => (equipment_at, at),
            };
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
                used.push(key);
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
        if let Some(kept) = kept {
            encode(
                &shown
                    .iter()
                    .map(|unit| totals[sample_of(unit.pieces.start)])
                    .collect::<Vec<_>>(),
            )?;
            shown = kept.into_iter().map(|index| shown[index].clone()).collect();
        }
        let total_at: Vec<Option<i64>> = shown
            .iter()
            .map(|unit| totals[sample_of(unit.pieces.start)])
            .collect();
        let total_coded = encode(&total_at)?;
        bytes += total_coded.as_ref().map_or(0, coded_bytes);
        let item = ProducedSeries {
            tag: tag.clone(),
            identities: None,
            first_hours: shown.iter().map(|unit| unit.first).collect(),
            last_hours: shown.iter().map(|unit| unit.last).collect(),
            breaks: shown
                .iter()
                .enumerate()
                .filter(|(_, unit)| unit.starts)
                .map(|(index, _)| index as u32)
                .collect(),
            values,
            total: total_coded,
            membership,
            summary,
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
        Ok((item, used, bytes, spans))
    })?;
    let mut used = vec![false; grouped.keys.len()];
    let mut series = Vec::with_capacity(built.len());
    let mut bytes = 0usize;
    for (item, keys, size, spans) in built {
        for key in keys {
            used[key] = true;
        }
        coverage.take(spans);
        bytes += size;
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

/// One country's shown pieces merged into spans, exact and unknown apart, which a [`Coverage`]
/// takes whole.
#[derive(Default)]
struct Spans {
    exact: Vec<(i32, i32)>,
    unknown: Vec<(i32, i32)>,
}

impl Spans {
    /// A piece after every one recorded.
    fn record(&mut self, piece: &Piece, known: bool) {
        let spans = if known {
            &mut self.exact
        } else {
            &mut self.unknown
        };
        match spans.last_mut() {
            Some(last) if piece.first <= last.1.saturating_add(1) => last.1 = last.1.max(piece.last),
            _ => spans.push((piece.first, piece.last)),
        }
    }
}

impl Coverage {
    fn take(&mut self, spans: Spans) {
        for (into, spans) in [(&mut self.exact, spans.exact), (&mut self.unknown, spans.unknown)] {
            for (first, last) in spans {
                into.entry(first)
                    .and_modify(|end| *end = (*end).max(last))
                    .or_insert(last);
            }
        }
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
    warnings: Vec<String>,
    cancelled: &AtomicBool,
) -> io::Result<EquipmentProducedReply> {
    reply_with(scanned, params, named, warnings, cancelled, None)
}

/// What a reply from a prepared history reads besides its light scan: the totals a ranking
/// counts, and the countries it shows whole.
pub(crate) trait Prepared: Sync {
    /// Every country's totals a ranking by `plan` sums, and those it reads whole instead.
    fn counted(&self, plan: &CountPlan<'_>) -> io::Result<(BTreeMap<String, Counted>, Vec<String>)>;
    /// Every country's samples and Rocket output.
    fn light(&self) -> io::Result<Histories>;
    /// Some countries' histories whole, their samples and Rocket output taken from `light`
    /// where it holds them.
    fn whole(&self, tags: &[String], light: Option<&Histories>) -> io::Result<Histories>;
}

/// What a ranking reads of a prepared history beyond its countries' samples: each country's
/// counted totals, but for the countries read whole, whose classes cannot stand for their
/// designs.
#[derive(Default)]
pub(crate) struct Ranking {
    counted: BTreeMap<String, Counted>,
    whole: BTreeMap<String, TagHistory>,
}

impl Ranking {
    fn read(prepared: &dyn Prepared, plan: &CountPlan<'_>) -> io::Result<Self> {
        let (counted, moved) = prepared.counted(plan)?;
        Ok(Self {
            counted,
            whole: prepared.whole(&moved, None)?.tags,
        })
    }
}

/// Countries' histories a reply reads beyond a prepared history's light scan: their equipment
/// lines' and their Rocket output's.
#[derive(Default)]
pub(crate) struct Histories {
    pub tags: BTreeMap<String, TagHistory>,
    pub rockets: BTreeMap<String, TagHistory>,
}

/// [`reply`] from a prepared history's light scan, its countries' totals left to `prepared`: the
/// top N ranked on the totals it counts, then the countries shown read whole.
pub(crate) fn reply_with(
    scanned: &ScannedProduced,
    params: &ProducedParams,
    named: Named,
    mut warnings: Vec<String>,
    cancelled: &AtomicBool,
    prepared: Option<&dyn Prepared>,
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
    let view = params
        .view
        .map(|view| {
            let bounds = (scanned.reads.first().copied().unwrap_or(view.hours().0), i32::MAX);
            let (exact, _) =
                crate::prepared::series::summary_bins(bounds, (view.from, view.to, view.width))?;
            io::Result::Ok(Shown {
                view,
                bounds,
                dense: !exact,
            })
        })
        .transpose()?;
    let mut requested = if params.top.is_some() || !params.tags.is_empty() {
        match prepared {
            None => requested(scanned, &grouped, params, view, None, cancelled)?,
            Some(prepared) => {
                let mut light = None;
                let (shown, ranked) = match params.top {
                    Some(_) => {
                        let light = light.insert(prepared.light()?);
                        let ranking = Ranking::read(
                            prepared,
                            &CountPlan {
                                costs: params.count == ProducedCount::Ic,
                                refits: params.refits,
                                counted: &grouped.class_counted,
                                outside: &grouped.class_outside,
                            },
                        )?;
                        let ranked = rank(
                            scanned,
                            &light.tags,
                            &light.rockets,
                            &grouped,
                            params,
                            view,
                            Some(&ranking),
                            cancelled,
                        )?;
                        (ranked.members.keys().cloned().collect::<Vec<_>>(), Some(ranked))
                    }
                    None => (params.tags.clone(), None),
                };
                let read = prepared.whole(&shown, light.as_ref())?;
                requested_in(
                    scanned,
                    &read.tags,
                    &read.rockets,
                    &grouped,
                    params,
                    view,
                    ranked,
                    cancelled,
                )?
            }
        }
    } else {
        Requested::default()
    };
    if params.grouping == ProducedGrouping::Variant {
        for key in &mut requested.keys {
            let variant = key
                .key
                .parse()
                .map_err(|_| io::Error::other("A variant key is not its handle"))?;
            key.names = scanned.variants.names(variant, named.texts);
        }
    }
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
            SeriesView::decl(),
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
        if params.view.is_some_and(|view| !view.valid()) {
            return Err("Invalid series view");
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
        let reply = answer(
            source,
            params,
            context,
            &scanned,
            cancelled,
            || super::country_history::branch_histories(source, branch, false, &[], cancelled),
            None,
        )?;
        super::check_reply_budget(&reply).map_err(|_| {
            io::Error::other("These countries' production history exceeds one exact reply")
        })?;
        Ok(reply)
    }
}

/// The reply to `params` from a branch's scan, its chosen countries' identities from
/// `histories`, typed and named with the content `context` gives.
pub(crate) fn answer(
    source: &Source,
    params: &ProducedParams,
    context: &LoadContext,
    scanned: &ScannedProduced,
    cancelled: &AtomicBool,
    histories: impl FnOnce() -> io::Result<Arc<super::country_history::RecordedHistories>>,
    prepared: Option<&dyn Prepared>,
) -> io::Result<EquipmentProducedReply> {
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
        let history = histories()?;
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
    let mut reply = reply_with(scanned, &projection, named, warnings, cancelled, prepared)?;
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
    Ok(reply)
}

#[cfg(test)]
use replay_data::rows::TableId;
#[cfg(test)]
use replay_frame::frame::TableView;
pub mod prepared;
#[cfg(test)]
pub(crate) mod tests;
