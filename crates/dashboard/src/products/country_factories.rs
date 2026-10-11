//! Every country's factories over one branch, for the Factories widget: each of
//! the three recorded factory groups as the game last recalculated them, hour by hour.
//!
//! The values are the recorded `CountryFactories` rows (table 257, hourly): the
//! stored totals the top bar, factory tooltips and factory triggers read
//! (see `engine::factory_group` and `engine::civ_factory_consumer`). A group's
//! buildings count through their content flags (`military_production`,
//! `naval_production`, `general_production`), in the states the country controls
//! and its offsite buildings, so occupied land is included and its own land
//! held by an enemy is left out.
//!
//! Every game view truncates the fixed5 totals toward zero, `(int)(value /
//! 100000)`, so the reply carries each truncated as the game shows it: T, C, O and
//! D below are the truncated total, controlled, owned and damaged factories. The
//! counts the widget offers are formulas over these (widget README):
//!
//! - free after withheld: T − D − consumer goods − trade imports − national
//!   projects − licence imports − market contracts − given to the overlord −
//!   given to other countries for the civilian group, and T − D − given to the
//!   overlord − given to other countries for the military and naval groups;
//! - built: T, the construction tab's `Total`;
//! - the top bar's group total: T - D - given to the overlord - given to others.
//!
//! The game adds every withheld count above into the group's `unavailable`,
//! which the reply does not repeat: an hour whose recorded `unavailable`
//! differs from those parts is counted in a warning rather than hidden.
//!
//! Runs compress equal consecutive hours and never bridge a gap, an hour whose
//! tables do not tell (the country or factory table incomplete, the links unable
//! to resolve the tag) or a change of the country identity under a tag. The
//! table is read every hour by cadence (`slot_read`), even when the shared
//! read is timed against another table. Values start at the hour supplying them.
//!
//! A request names what the widget draws, so a reply never carries every
//! country's history: either chosen tags, or the top N by a count, ranked here
//! over every country at every hour a value changes.
//! A top-N reply carries each country that was ever among them, with the hours
//! it was (`membership`) and its runs cut to those hours, so at most N countries'
//! runs cover any hour. The reply's size is counted before it is built and
//! refused past [`MAX_REPLY_BYTES`], inside the transport's bound.

use super::{
    LoadContext, Product,
    country_groups::HourSpan,
    country_history::{Identity, observe_identity, unique_by_slot},
    validate_tag,
};
use crate::{
    Source,
    catalogue::{self, Catalogue},
    metadata::{IdentityTrack, TaggedCountry, slot_read, tagged_countries, validate_campaign},
};
use installed_content::buildings::Buildings;
use replay::{
    container::Replay,
    frame::FrameDecoder,
    rows::{CountryFactoriesRow, TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct CountryFactories;

/// The fixed5 scale of the recorded totals (`engine::factory_group`: i64 x100000).
const FIXED5: i64 = 100_000;

const CACHED_BRANCHES: usize = 2;
/// The most a reply's series may take serialised: well inside the transport's
/// 32 MiB with the groups, the fields and the envelope.
pub use super::MAX_PRODUCT_REPLY_BYTES as MAX_REPLY_BYTES;
/// JSON of one run besides its numbers: the keys, braces and commas.
const RUN_OVERHEAD: usize = 64;

/// One of the game's recalculated factory groups. Building membership,
/// including modded types, comes from the content flag [`FactoryGroupKey::flag`].
/// The fourth group (positive `rocket_production`) is not recorded; content
/// that counts in it is refused by name rather than treated as one of these groups.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, TS)]
#[serde(rename_all = "camelCase")]
pub enum FactoryGroupKey {
    Military,
    Naval,
    Civilian,
}

impl FactoryGroupKey {
    pub const ALL: [Self; 3] = [Self::Military, Self::Naval, Self::Civilian];

    /// The building template flag that counts a building in this group
    /// (`installed_content::buildings::FactoryFlags`).
    pub fn flag(self) -> &'static str {
        match self {
            Self::Military => "military_production",
            Self::Naval => "naval_production",
            Self::Civilian => "general_production",
        }
    }

    fn counts(self, flags: &installed_content::buildings::FactoryFlags) -> bool {
        match self {
            Self::Military => flags.military(),
            Self::Naval => flags.naval(),
            Self::Civilian => flags.civilian(),
        }
    }
}

/// One recorded part of a group, each a whole number of factories.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum FactoryPart {
    /// trunc(total): the construction tab's `Total`.
    Total,
    /// trunc(controlled): factories of the states it controls and its offsite buildings.
    Controlled,
    /// trunc(owned): the part of those in states it owns; `From occupation` is
    /// controlled − owned.
    Owned,
    /// trunc(damaged).
    Damaged,
    FromSubjects,
    /// Given to the overlord.
    ToOverlord,
    FromOthers,
    /// Given to other countries.
    ToOthers,
    /// Civilian only: factories the country's resource exports earn.
    TradeExports,
    /// Civilian only: factories its licences earn.
    LicenseExports,
    /// Civilian only, the five consumers in the order the game runs them.
    ConsumerGoods,
    ContractPayments,
    SpecialProjects,
    LicenseImports,
    TradeImports,
}

/// What one position of a run's `values` holds.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactoryField {
    pub group: FactoryGroupKey,
    pub part: FactoryPart,
}

const GROUP_PARTS: [FactoryPart; 8] = [
    FactoryPart::Total,
    FactoryPart::Controlled,
    FactoryPart::Owned,
    FactoryPart::Damaged,
    FactoryPart::FromSubjects,
    FactoryPart::ToOverlord,
    FactoryPart::FromOthers,
    FactoryPart::ToOthers,
];
const CIVILIAN_PARTS: [FactoryPart; 7] = [
    FactoryPart::TradeExports,
    FactoryPart::LicenseExports,
    FactoryPart::ConsumerGoods,
    FactoryPart::ContractPayments,
    FactoryPart::SpecialProjects,
    FactoryPart::LicenseImports,
    FactoryPart::TradeImports,
];

/// Every field of a run's values, in order.
pub fn fields() -> Vec<FactoryField> {
    FactoryGroupKey::ALL
        .into_iter()
        .flat_map(|group| GROUP_PARTS.map(|part| FactoryField { group, part }))
        .chain(CIVILIAN_PARTS.map(|part| FactoryField {
            group: FactoryGroupKey::Civilian,
            part,
        }))
        .collect()
}

/// A building type whose content flag counts it in a group.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactoryBuilding {
    /// The content's building name.
    pub token: String,
    /// Its localised name; null when the content has none.
    pub name: Option<String>,
}

/// A factory group and the content building types that count in it, in the
/// content's declaration order. Empty when the content is not known.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactoryGroupInfo {
    pub key: FactoryGroupKey,
    /// The content flag that counts a building in it.
    pub flag: String,
    pub buildings: Vec<FactoryBuilding>,
}

/// Recorded factory values held over consecutive hours.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactoryRun {
    pub first_hour: i32,
    pub last_hour: i32,
    /// The run follows a gap, an hour the tables could not tell or another
    /// country identity rather than continuing the previous run.
    pub starts_segment: bool,
    /// Whole factories, one per [`CountryFactoriesReply::fields`] entry.
    pub values: Vec<i32>,
}

/// Supplying factory read, independent of equal value compression.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactoryReadRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub read_hour: i32,
    pub hourly: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactorySeries {
    pub tag: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub identities: Option<Vec<super::country_history::CountryIdentityRun>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reads: Option<Vec<FactoryReadRun>>,
    pub runs: Vec<FactoryRun>,
    /// Unbroken country identities from Countries and CountryLinks, never cut to top-N
    /// membership. Appearance samples must stay within the segment at their hour.
    pub segments: Vec<HourSpan>,
    /// For a top-N reply, the hours the country was among the top N, in order;
    /// its runs are cut to them. Null for chosen countries.
    pub membership: Option<Vec<HourSpan>>,
}

/// Which count ranks a top-N request (user decision, 29 Sep; module notes).
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum FactoryCount {
    /// Free after withheld: built − damaged − everything withheld.
    Free,
    /// The game's `Total`.
    Built,
    /// The top bar's group total: built − damaged − given away.
    TopBar,
}

/// The top N countries by a count of some groups, at every hour.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactoryTop {
    pub count: u32,
    pub mode: FactoryCount,
    /// The groups summed; none sums all three.
    pub groups: Vec<FactoryGroupKey>,
}

/// Chosen countries, or the top N (never both). With neither, the reply names
/// the groups only, for a settings list.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactoriesParams {
    pub branch_id: String,
    pub tags: Vec<String>,
    pub top: Option<FactoryTop>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CountryFactoriesReply {
    pub branch_id: String,
    /// The three groups, with the content building types counted in each.
    pub groups: Vec<FactoryGroupInfo>,
    /// What each position of a run's values holds.
    pub fields: Vec<FactoryField>,
    /// The countries asked for that were recorded, or every country ever in the
    /// top N, by tag.
    pub series: Vec<FactorySeries>,
    pub warnings: Vec<String>,
}

/// One branch's scan.
#[derive(Clone, Debug, Default)]
pub struct ScannedFactories {
    pub series: BTreeMap<String, Vec<FactoryRun>>,
    pub segments: BTreeMap<String, Vec<HourSpan>>,
    pub reads: BTreeMap<String, Vec<FactoryReadRun>>,
    /// Country-hours whose recorded `unavailable` is not the sum of the withheld
    /// parts, by group.
    pub unexplained: BTreeMap<FactoryGroupKey, usize>,
    /// Rows the links could not resolve to a country, or that more than one
    /// row named, or whose totals do not fit a whole number the game shows.
    pub unreadable: usize,
    /// Tags that are not text.
    pub unnamed: usize,
}

/// The group's recorded fields.
struct Group {
    total_raw: i64,
    controlled_raw: i64,
    owned_raw: i64,
    damaged_raw: i64,
    unavailable: i32,
    from_subjects: i32,
    to_overlord: i32,
    from_others: i32,
    to_others: i32,
}

fn group(row: &CountryFactoriesRow, key: FactoryGroupKey) -> Group {
    match key {
        FactoryGroupKey::Military => Group {
            total_raw: row.military_total_raw,
            controlled_raw: row.military_controlled_raw,
            owned_raw: row.military_owned_raw,
            damaged_raw: row.military_damaged_raw,
            unavailable: row.military_unavailable,
            from_subjects: row.military_from_subjects,
            to_overlord: row.military_to_overlord,
            from_others: row.military_from_others,
            to_others: row.military_to_others,
        },
        FactoryGroupKey::Naval => Group {
            total_raw: row.naval_total_raw,
            controlled_raw: row.naval_controlled_raw,
            owned_raw: row.naval_owned_raw,
            damaged_raw: row.naval_damaged_raw,
            unavailable: row.naval_unavailable,
            from_subjects: row.naval_from_subjects,
            to_overlord: row.naval_to_overlord,
            from_others: row.naval_from_others,
            to_others: row.naval_to_others,
        },
        FactoryGroupKey::Civilian => Group {
            total_raw: row.civilian_total_raw,
            controlled_raw: row.civilian_controlled_raw,
            owned_raw: row.civilian_owned_raw,
            damaged_raw: row.civilian_damaged_raw,
            unavailable: row.civilian_unavailable,
            from_subjects: row.civilian_from_subjects,
            to_overlord: row.civilian_to_overlord,
            from_others: row.civilian_from_others,
            to_others: row.civilian_to_others,
        },
    }
}

/// `(int)(value / 100000)`, as every game view shows a fixed5 total; none when it
/// does not fit.
fn truncated(raw: i64) -> Option<i32> {
    i32::try_from(raw / FIXED5).ok()
}

/// The row's values in [`fields`] order, and which groups' recorded `unavailable`
/// is not the sum of their withheld parts; none when a total does not fit.
fn values(row: &CountryFactoriesRow) -> Option<(Vec<i32>, Vec<FactoryGroupKey>)> {
    let mut values = Vec::with_capacity(GROUP_PARTS.len() * 3 + CIVILIAN_PARTS.len());
    let mut unexplained = Vec::new();
    let consumers = [
        row.consumer_goods,
        row.contract_payments,
        row.special_projects,
        row.license_imports,
        row.trade_imports,
    ];
    for key in FactoryGroupKey::ALL {
        let group = group(row, key);
        values.extend([
            truncated(group.total_raw)?,
            truncated(group.controlled_raw)?,
            truncated(group.owned_raw)?,
            truncated(group.damaged_raw)?,
            group.from_subjects,
            group.to_overlord,
            group.from_others,
            group.to_others,
        ]);
        let withheld = i64::from(group.to_overlord)
            + i64::from(group.to_others)
            + match key {
                FactoryGroupKey::Civilian => consumers.iter().copied().map(i64::from).sum(),
                FactoryGroupKey::Military | FactoryGroupKey::Naval => 0,
            };
        if withheld != i64::from(group.unavailable) {
            unexplained.push(key);
        }
    }
    values.extend([row.civilian_trade_exports, row.civilian_license_exports]);
    values.extend(consumers);
    Some((values, unexplained))
}

/// One tag's runs, and the identity and hour its last run was extended to.
#[derive(Default)]
struct Track {
    runs: Vec<FactoryRun>,
    last: Option<(Identity, i32)>,
    segments: Vec<HourSpan>,
    last_identity: IdentityTrack,
    reads: Vec<FactoryReadRun>,
}

fn append_read(
    reads: &mut Vec<FactoryReadRun>,
    hour: i32,
    read_hour: i32,
    hourly: bool,
    continues: bool,
) {
    if let Some(last) = reads.last_mut()
        && continues
        && last.last_hour.checked_add(1) == Some(hour)
        && last.hourly == hourly
        && (hourly || last.read_hour == read_hour)
    {
        last.last_hour = hour;
    } else {
        reads.push(FactoryReadRun {
            first_hour: hour,
            last_hour: hour,
            read_hour,
            hourly,
        });
    }
}

/// Scans one branch once for every country.
pub fn scan(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<ScannedFactories> {
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::CountryFactories,
    ];
    let cancel = || io::Error::new(io::ErrorKind::Interrupted, "Factories request cancelled");
    let mut result = ScannedFactories::default();
    let mut tracks: BTreeMap<String, Track> = BTreeMap::new();
    crate::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                if cancelled.load(Ordering::Acquire) {
                    return Err(cancel());
                }
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
                    continue;
                };
                for frame in frames {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(cancel());
                    }
                    let Ok(decoded) = decoder.advance(&frame) else {
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        break;
                    }
                    let hour = frame.hour;
                    // An hour the tables cannot tell extends no track, so nothing continues
                    // across it: a run continues only from the hour before.
                    let (Ok(countries), Ok(links)) = (
                        decoded.complete::<tables::Countries>(),
                        decoded.complete::<tables::CountryLinks>(),
                    ) else {
                        continue;
                    };
                    let rows = decoded.complete::<tables::CountryFactories>().ok();
                    let read = slot_read(&decoded.metrics, TableId::CountryFactories, decoded.hour);
                    // Each row by the slot its link names; a slot two rows name is unknown.
                    let by_slot =
                        unique_by_slot(rows.filter(|_| read).into_iter().flatten(), links, |row| {
                            row.country
                        });
                    result.unreadable += by_slot.unresolved_rows + by_slot.duplicate_rows;
                    for (tag, lookup) in tagged_countries(countries, links) {
                        let TaggedCountry::Found(country, slot) = lookup else {
                            continue;
                        };
                        let Ok(name) = std::str::from_utf8(tag) else {
                            result.unnamed += 1;
                            continue;
                        };
                        if !tracks.contains_key(name) {
                            tracks.insert(name.to_owned(), Track::default());
                        }
                        let track = tracks.get_mut(name).expect("inserted above");
                        let identity = (slot, country.id);
                        observe_identity(
                            &mut track.segments,
                            &mut track.last_identity,
                            identity,
                            hour,
                        );
                        // Factory availability must not change a known country identity.
                        if rows.is_none() {
                            continue;
                        }
                        let continues = track.last.is_some_and(|(seen, at)| {
                            seen == identity && at.checked_add(1) == Some(hour)
                        });
                        if !read {
                            // Carried: the last read still holds, only for the same identity.
                            if continues && let Some(run) = track.runs.last_mut() {
                                run.last_hour = hour;
                                track.last = Some((identity, hour));
                                if let Some(previous) = track.reads.last() {
                                    let supplying = if previous.hourly {
                                        previous.last_hour
                                    } else {
                                        previous.read_hour
                                    };
                                    append_read(&mut track.reads, hour, supplying, false, true);
                                }
                            }
                            continue;
                        }
                        let Some(Some(row)) = by_slot.rows.get(&slot) else {
                            continue;
                        };
                        let Some((values, unexplained)) = values(row) else {
                            result.unreadable += 1;
                            continue;
                        };
                        for key in unexplained {
                            *result.unexplained.entry(key).or_default() += 1;
                        }
                        track.last = Some((identity, hour));
                        append_read(&mut track.reads, hour, hour, true, continues);
                        match track.runs.last_mut() {
                            Some(run) if continues && run.values == values => run.last_hour = hour,
                            _ => {
                                track.runs.push(FactoryRun {
                                    first_hour: hour,
                                    last_hour: hour,
                                    starts_segment: !continues,
                                    values,
                                });
                            }
                        }
                    }
                }
            }
            Ok(())
        },
    )?;
    for (tag, track) in tracks {
        if !track.runs.is_empty() {
            result.segments.insert(tag.clone(), track.segments);
            result.reads.insert(tag.clone(), track.reads);
            result.series.insert(tag, track.runs);
        }
    }
    Ok(result)
}

/// A part's position in a run's values ([`fields`] order).
fn position(group: FactoryGroupKey, part: FactoryPart) -> Option<usize> {
    let group_index = FactoryGroupKey::ALL.iter().position(|key| *key == group)?;
    GROUP_PARTS
        .iter()
        .position(|item| *item == part)
        .map(|index| group_index * GROUP_PARTS.len() + index)
        .or_else(|| {
            (group == FactoryGroupKey::Civilian)
                .then(|| CIVILIAN_PARTS.iter().position(|item| *item == part))
                .flatten()
                .map(|index| FactoryGroupKey::ALL.len() * GROUP_PARTS.len() + index)
        })
}

/// A group's count under a mode, as the widget counts it (`factoriesData.ts`).
pub fn group_count(values: &[i32], group: FactoryGroupKey, mode: FactoryCount) -> i64 {
    let get = |part| {
        position(group, part)
            .and_then(|index| values.get(index))
            .map_or(0, |value| i64::from(*value))
    };
    let total = get(FactoryPart::Total);
    let given = get(FactoryPart::ToOverlord) + get(FactoryPart::ToOthers);
    let consumers = match group {
        FactoryGroupKey::Civilian => [
            FactoryPart::ConsumerGoods,
            FactoryPart::ContractPayments,
            FactoryPart::SpecialProjects,
            FactoryPart::LicenseImports,
            FactoryPart::TradeImports,
        ]
        .into_iter()
        .map(get)
        .sum(),
        FactoryGroupKey::Military | FactoryGroupKey::Naval => 0,
    };
    match mode {
        FactoryCount::Built => total,
        FactoryCount::TopBar => total - get(FactoryPart::Damaged) - given,
        FactoryCount::Free => total - get(FactoryPart::Damaged) - given - consumers,
    }
}

/// The hours each country was among the top N by `top`'s count, by tag: at
/// every hour the recorded values can change, every country recorded then is
/// ranked (highest first, ties by tag), so between two such hours the top N
/// stay the same. A country not recorded at an hour is not ranked there.
pub fn top_membership(
    series: &BTreeMap<String, Vec<FactoryRun>>,
    top: &FactoryTop,
    cancelled: &AtomicBool,
) -> io::Result<BTreeMap<String, Vec<HourSpan>>> {
    let groups: &[FactoryGroupKey] = if top.groups.is_empty() {
        &FactoryGroupKey::ALL
    } else {
        &top.groups
    };
    crate::ranking::top_membership(
        series,
        top.count as usize,
        |run| (run.first_hour, run.last_hour),
        |run| {
            Some(
                groups
                    .iter()
                    .map(|group| group_count(&run.values, *group, top.mode))
                    .sum::<i64>(),
            )
        },
        cancelled,
    )
}

/// Runs cut to the spans: a piece keeps its run's line break only where the run
/// itself starts. Both are in hour order and never overlap (a scan's runs, a top-N
/// membership), so one pass over both cuts them: linear in the runs and spans, and
/// stopped by `cancelled`.
pub fn clip_runs(
    runs: &[FactoryRun],
    spans: &[HourSpan],
    cancelled: &AtomicBool,
) -> io::Result<Vec<FactoryRun>> {
    let mut clipped = Vec::new();
    crate::ranking::for_each_piece(
        runs,
        spans,
        |run| (run.first_hour, run.last_hour),
        cancelled,
        |run, first_hour, last_hour| {
            clipped.push(FactoryRun {
                first_hour,
                last_hour,
                starts_segment: run.starts_segment && first_hour == run.first_hour,
                values: run.values.clone(),
            });
            Ok(())
        },
    )?;
    Ok(clipped)
}

/// At least as many bytes as a run over these hours with these values takes serialised.
fn piece_bytes(first_hour: i32, last_hour: i32, values: &[i32]) -> usize {
    // i32::MIN is the longest number: 11 characters, and a comma.
    let digits = |value: i32| value.to_string().len() + 1;
    RUN_OVERHEAD
        + digits(first_hour)
        + digits(last_hour)
        + values.iter().map(|value| digits(*value)).sum::<usize>()
}

/// At least as many bytes as one series takes serialised, besides its runs.
fn series_overhead(tag: &str, membership: Option<&[HourSpan]>, segments: usize) -> usize {
    // A span: its keys and braces, two hours of at most 11 characters.
    RUN_OVERHEAD + tag.len() + membership.map_or(0, |spans| spans.len() * 52) + segments * 52
}

/// At least as many bytes as the series take serialised.
pub fn series_bytes(series: &[FactorySeries]) -> usize {
    series
        .iter()
        .map(|item| {
            series_overhead(&item.tag, item.membership.as_deref(), item.segments.len())
                + item
                    .reads
                    .as_ref()
                    .map_or(0, |reads| 16 + reads.len() * 128)
                + item
                    .identities
                    .as_ref()
                    .map_or(0, |runs| 24 + runs.len() * 128)
                + item
                    .runs
                    .iter()
                    .map(|run| piece_bytes(run.first_hour, run.last_hour, &run.values))
                    .sum::<usize>()
        })
        .sum()
}

/// One series of a reply as it is cut from the scan: its tag, the scanned runs
/// (borrowed), and the hours to cut them to (none: all of them).
type Part<'a> = (
    String,
    &'a [FactoryRun],
    &'a [HourSpan],
    Option<Vec<HourSpan>>,
);

/// What a request's series are cut from: each tag asked for that was recorded (in
/// the order asked, once each) with no cut, or each country ever in the top N with
/// the hours it was.
fn requested_parts<'a>(
    scanned: &'a ScannedFactories,
    params: &FactoriesParams,
    cancelled: &AtomicBool,
) -> io::Result<Vec<Part<'a>>> {
    Ok(match &params.top {
        Some(top) => top_membership(&scanned.series, top, cancelled)?
            .into_iter()
            .map(|(tag, spans)| {
                let runs = scanned.series[&tag].as_slice();
                let segments = scanned
                    .segments
                    .get(&tag)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                (tag, runs, segments, Some(spans))
            })
            .collect(),
        None => {
            let mut seen = Vec::new();
            params
                .tags
                .iter()
                .filter(|tag| {
                    let fresh = !seen.contains(tag);
                    seen.push(*tag);
                    fresh
                })
                .filter_map(|tag| {
                    scanned.series.get(tag).map(|runs| {
                        (
                            tag.clone(),
                            runs.as_slice(),
                            scanned
                                .segments
                                .get(tag)
                                .map(Vec::as_slice)
                                .unwrap_or_default(),
                            None,
                        )
                    })
                })
                .collect()
        }
    })
}

/// At least as many bytes as the request's reply series take serialised, counted
/// from the scan without copying any of it.
pub fn requested_bytes(
    scanned: &ScannedFactories,
    params: &FactoriesParams,
    cancelled: &AtomicBool,
) -> io::Result<usize> {
    let parts = requested_parts(scanned, params, cancelled)?;
    Ok(parts_bytes(&parts, cancelled)? + read_bytes(scanned, &parts))
}

type Parts<'a> = [Part<'a>];

fn read_bytes(scanned: &ScannedFactories, parts: &Parts<'_>) -> usize {
    parts
        .iter()
        .map(|(tag, _, _, _)| {
            scanned
                .reads
                .get(tag)
                .map_or(0, |reads| 16 + reads.len() * 128)
        })
        .sum()
}

fn parts_bytes(parts: &Parts<'_>, cancelled: &AtomicBool) -> io::Result<usize> {
    let mut bytes = 0;
    for (tag, runs, segments, membership) in parts {
        bytes += series_overhead(tag, membership.as_deref(), segments.len());
        match membership {
            Some(spans) => crate::ranking::for_each_piece(
                runs,
                spans,
                |run| (run.first_hour, run.last_hour),
                cancelled,
                |run, first, last| {
                    bytes += piece_bytes(first, last, &run.values);
                    Ok(())
                },
            )?,
            None => {
                bytes += runs
                    .iter()
                    .map(|run| piece_bytes(run.first_hour, run.last_hour, &run.values))
                    .sum::<usize>();
            }
        }
    }
    Ok(bytes)
}

/// The series a request asks for: the chosen tags recorded in the branch, in the
/// order asked, or every country ever in the top N with its runs cut to the hours
/// it was. Its size is counted from the scan first, and a reply past
/// [`MAX_REPLY_BYTES`] is refused before any of the history is copied.
pub fn requested_series(
    scanned: &ScannedFactories,
    params: &FactoriesParams,
    cancelled: &AtomicBool,
) -> io::Result<Vec<FactorySeries>> {
    let parts = requested_parts(scanned, params, cancelled)?;
    let bytes = parts_bytes(&parts, cancelled)? + read_bytes(scanned, &parts);
    if bytes > MAX_REPLY_BYTES {
        return Err(io::Error::other(format!(
            "These countries' factory history takes {} MiB, more than one reply holds ({} MiB); choose fewer countries.",
            bytes.div_ceil(1024 * 1024),
            MAX_REPLY_BYTES / (1024 * 1024)
        )));
    }
    parts
        .into_iter()
        .map(|(tag, runs, segments, membership)| {
            Ok(FactorySeries {
                identities: None,
                reads: scanned.reads.get(&tag).cloned(),
                segments: segments.to_vec(),
                runs: match &membership {
                    Some(spans) => clip_runs(runs, spans, cancelled)?,
                    None => runs.to_vec(),
                },
                tag,
                membership,
            })
        })
        .collect()
}

/// The content building types counted in each group, named from the localisation
/// (a building type's name is its localisation key). A type declared again with
/// different flags is left out, since which declaration the game uses is not known.
pub fn group_buildings(
    buildings: &Buildings,
    localise: impl Fn(&str) -> Option<String>,
) -> Vec<FactoryGroupInfo> {
    FactoryGroupKey::ALL
        .into_iter()
        .map(|key| FactoryGroupInfo {
            key,
            flag: key.flag().to_owned(),
            buildings: buildings
                .declaration_order
                .iter()
                .filter(|token| !buildings.ambiguous_factory_types.contains(*token))
                .filter(|token| {
                    buildings
                        .factory_flags
                        .get(*token)
                        .is_some_and(|flags| key.counts(flags))
                })
                .map(|token| FactoryBuilding {
                    token: token.clone(),
                    name: localise(token),
                })
                .collect(),
        })
        .collect()
}

fn unnamed_groups() -> Vec<FactoryGroupInfo> {
    FactoryGroupKey::ALL
        .into_iter()
        .map(|key| FactoryGroupInfo {
            key,
            flag: key.flag().to_owned(),
            buildings: Vec::new(),
        })
        .collect()
}

/// The branches scanned, most recent last, and how many scans have run. Its lock is
/// held while a branch is scanned, as the service's other products hold their
/// caches: requests for the product on one source run one scan at a time, so
/// widgets asking at once (each with its own top N) share the first one's scan
/// instead of each scanning the branch and holding its history.
#[derive(Default)]
struct Scans {
    entries: VecDeque<(u64, Arc<ScannedFactories>)>,
    runs: usize,
}

/// The groups named for the catalogue they were read from, and their warnings.
#[derive(Default)]
struct ContentCache {
    content: Option<(Arc<Catalogue>, Vec<FactoryGroupInfo>, Vec<String>)>,
}

/// The branch's scan: the cached one, or one scanned now while the lock keeps any
/// other request for it waiting for this scan.
pub(crate) fn scanned_branch(
    source: &Source,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Arc<ScannedFactories>> {
    let state = source.state::<Scans>();
    let mut scans = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((_, scanned)) = scans.entries.iter().find(|(cached, _)| *cached == branch) {
        return Ok(Arc::clone(scanned));
    }
    scans.runs += 1;
    let scanned = Arc::new(scan(&source.replay(), branch, cancelled)?);
    scans.entries.push_back((branch, Arc::clone(&scanned)));
    while scans.entries.len() > CACHED_BRANCHES {
        scans.entries.pop_front();
    }
    Ok(scanned)
}

/// How many branch scans this source has run, for tests of their sharing.
pub fn scans_run(source: &Source) -> usize {
    source
        .state::<Scans>()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .runs
}

pub(crate) fn content_groups(catalogue: &Catalogue) -> (Vec<FactoryGroupInfo>, Vec<String>) {
    let mut warnings = catalogue.verification.clone();
    warnings.extend(catalogue.warnings.iter().cloned());
    let (groups, notes, mut findings) =
        describe_groups_with_findings(&catalogue.content.layers, |token| {
            // The building's raw key is localised without script scope into its
            // cached display name for the construction UI. A type without
            // localisation stays unnamed.
            catalogue.localisation.get(token)?;
            catalogue.localised(token, catalogue::View::Plain)
        });
    for building in groups.iter().flat_map(|group| &group.buildings) {
        if building.name.is_none() && catalogue.localisation.get(&building.token).is_none() {
            findings.push(crate::content_check::Finding::new(
                crate::content_check::Kind::Content,
                format!("building {}", building.token),
                format!(
                    "{}: no name in the recorded content; title is unavailable.",
                    building.token
                ),
            ));
        }
    }
    for finding in findings {
        catalogue.note(finding);
    }
    warnings.extend(notes);
    warnings.extend(catalogue.refusal_lines());
    (groups, warnings)
}

/// The groups as the recorded content stack names them: every building type any
/// layer declares with a group's flag (a mod's own factory types included, a
/// vanilla one a mod removed left out), named through `localise`. What cannot be
/// read is said in a warning. Missing titles remain unavailable; definition keys
/// and group flags stay separate for diagnostics and selection.
fn describe_groups_with_findings(
    layers: &installed_content::ResolvedLayers,
    localise: impl Fn(&str) -> Option<String>,
) -> (
    Vec<FactoryGroupInfo>,
    Vec<String>,
    Vec<crate::content_check::Finding>,
) {
    use crate::content_check::{Finding, Kind};
    let mut warnings = Vec::new();
    let mut findings = Vec::new();
    match installed_content::buildings::read(layers) {
        Ok(buildings) => {
            for token in &buildings.ambiguous_factory_types {
                let message = format!(
                    "{token} is declared more than once with different factory flags; which group it counts in is not known, so it names none."
                );
                findings.push(Finding::new(
                    Kind::Ambiguous,
                    format!("building {token}"),
                    &message,
                ));
                warnings.push(message);
            }
            for token in &buildings.declaration_order {
                if !buildings.ambiguous_factory_types.contains(token)
                    && buildings
                        .factory_flags
                        .get(token)
                        .is_some_and(|flags| flags.rocket())
                {
                    let message = format!(
                        "{token} has positive rocket_production; its factory group was not recorded, so its rocket factory counts are unavailable."
                    );
                    let mut finding =
                        Finding::new(Kind::Refused, format!("building {token}"), &message);
                    finding.count = Some(1);
                    findings.push(finding);
                    warnings.push(message);
                }
            }
            let groups = group_buildings(&buildings, localise);
            for group in &groups {
                if group.buildings.is_empty() {
                    let message = format!(
                        "No building in the recorded content has {}; its group is named by the flag.",
                        group.flag
                    );
                    findings.push(Finding::new(
                        Kind::Fallback,
                        format!("factory group {}", group.flag),
                        &message,
                    ));
                    warnings.push(message);
                }
            }
            (groups, warnings, findings)
        }
        Err(error) => {
            let message = format!(
                "Factory types are unavailable, so the groups are named by their flags: {error}"
            );
            findings.push(Finding::new(Kind::Refused, "factory types", &message));
            warnings.push(message);
            (unnamed_groups(), warnings, findings)
        }
    }
}

impl Product for CountryFactories {
    const KEY: &'static str = "country-factories";
    const CONTENT_DEPENDENT: bool = true;
    type Params = FactoriesParams;
    type Reply = CountryFactoriesReply;

    fn types() -> Vec<String> {
        vec![
            FactoriesParams::decl(),
            FactoryCount::decl(),
            FactoryTop::decl(),
            FactoryGroupKey::decl(),
            FactoryPart::decl(),
            FactoryField::decl(),
            FactoryBuilding::decl(),
            FactoryGroupInfo::decl(),
            FactoryRun::decl(),
            FactoryReadRun::decl(),
            FactorySeries::decl(),
            CountryFactoriesReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        for tag in &params.tags {
            validate_tag(tag)?;
        }
        if let Some(top) = &params.top {
            let mut groups = top.groups.clone();
            groups.sort_unstable();
            groups.dedup();
            if !params.tags.is_empty() || top.count == 0 || groups.len() != top.groups.len() {
                return Err("Invalid top countries");
            }
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
        let mut warnings = Vec::new();
        // Without the game folders the counts still draw, the groups named by their flags.
        let groups = match context
            .content()
            .and_then(|content| catalogue::catalogue(source, content))
        {
            Ok(catalogue) => {
                let state = source.state::<ContentCache>();
                let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
                if !cache
                    .content
                    .as_ref()
                    .is_some_and(|(cached, _, _)| Arc::ptr_eq(cached, &catalogue))
                {
                    let (groups, notes) = content_groups(&catalogue);
                    cache.content = Some((Arc::clone(&catalogue), groups, notes));
                }
                let (_, groups, notes) = cache.content.as_ref().expect("content cached");
                warnings.extend(notes.iter().cloned());
                groups.clone()
            }
            Err(error) => {
                warnings.push(format!("Factory type names are unavailable: {error}"));
                unnamed_groups()
            }
        };
        // A request naming no country asks for the types alone (a settings list): they
        // come from content, so the history is not scanned for it.
        if params.tags.is_empty() && params.top.is_none() {
            return Ok(CountryFactoriesReply {
                branch_id: params.branch_id.clone(),
                groups,
                fields: fields(),
                series: Vec::new(),
                warnings,
            });
        }
        let scanned = scanned_branch(source, branch, cancelled)?;
        for (key, hours) in &scanned.unexplained {
            warnings.push(format!(
                "{hours} country-hours record {} factories withheld other than the sum of their parts; counts use the parts.",
                key.flag()
            ));
        }
        if scanned.unreadable > 0 {
            warnings.push(format!(
                "{} recorded factory rows name no single country the links resolve, or hold a total the game could not show; they are left out.",
                scanned.unreadable
            ));
        }
        if scanned.unnamed > 0 {
            warnings.push(format!(
                "{} country-hours have a tag that is not text; they are left out.",
                scanned.unnamed
            ));
        }
        if scanned.series.is_empty() {
            warnings.push("No factories were recorded in this branch.".to_owned());
        }
        for tag in params
            .tags
            .iter()
            .filter(|tag| !scanned.series.contains_key(*tag))
        {
            warnings.push(format!("{tag}: no recorded factories in this branch"));
        }
        let mut projection = params.clone();
        let histories = if params.top.is_none() && !params.tags.is_empty() {
            let history =
                super::country_history::branch_histories(source, branch, false, &[], cancelled)?;
            projection.tags = super::country_history::selected_aliases(&history, &params.tags);
            Some(history)
        } else {
            None
        };
        if let Some(history) = &histories {
            let metadata = projection
                .tags
                .iter()
                .map(|tag| {
                    128 + tag.len()
                        + history.countries.get(tag).map_or(0, |country| {
                            24 + country.identities.len() * 128 + country.segments.len() * 52
                        })
                })
                .sum::<usize>();
            if requested_bytes(&scanned, &projection, cancelled)?.saturating_add(metadata)
                > MAX_REPLY_BYTES
            {
                return Err(io::Error::other(
                    "These countries' factory identities and history exceed one reply",
                ));
            }
        }
        let mut series = requested_series(&scanned, &projection, cancelled)?;
        if let Some(history) = histories {
            for tag in &projection.tags {
                if !series.iter().any(|item| item.tag == *tag) {
                    series.push(FactorySeries {
                        tag: tag.clone(),
                        ..Default::default()
                    });
                }
            }
            for item in &mut series {
                if let Some(country) = history.countries.get(&item.tag) {
                    item.identities = Some(country.identities.as_ref().clone());
                    item.segments = country.segments.as_ref().clone();
                } else {
                    item.identities = Some(Vec::new());
                }
            }
        }
        let reply = CountryFactoriesReply {
            branch_id: params.branch_id.clone(),
            groups,
            fields: fields(),
            series,
            warnings,
        };
        if crate::transport::json_len(&reply, MAX_REPLY_BYTES).is_none() {
            return Err(io::Error::other(
                "These countries' factory history exceeds one exact reply",
            ));
        }
        Ok(reply)
    }
}

/// Clips stale factory coverage around proven country-absence reads. Zero
/// evidence wins at an overlap, so one country never contributes twice.
fn faction_measure_runs(
    recorded: &[FactoryRun],
    absent: &[HourSpan],
    mode: FactoryCount,
    groups: &[FactoryGroupKey],
    count: &mut usize,
    cancelled: &AtomicBool,
) -> io::Result<Vec<super::faction_totals::MeasureRun>> {
    use super::faction_totals::{MeasureRun, push_measure};
    let mut runs = Vec::new();
    let mut absent_index = 0usize;
    for run in recorded {
        let mut first = i64::from(run.first_hour);
        let last = i64::from(run.last_hour);
        let value = Some(
            groups
                .iter()
                .map(|group| group_count(&run.values, *group, mode))
                .sum(),
        );
        while absent
            .get(absent_index)
            .is_some_and(|span| i64::from(span.last_hour) < first)
        {
            absent_index += 1;
        }
        let mut index = absent_index;
        while let Some(span) = absent.get(index) {
            if i64::from(span.first_hour) > last {
                break;
            }
            if first < i64::from(span.first_hour) {
                push_measure(
                    &mut runs,
                    MeasureRun {
                        first_hour: first as i32,
                        last_hour: (i64::from(span.first_hour) - 1) as i32,
                        value,
                        starts_segment: run.starts_segment || first != i64::from(run.first_hour),
                    },
                    count,
                    cancelled,
                )?;
            }
            first = first.max(i64::from(span.last_hour) + 1);
            if first > last {
                break;
            }
            index += 1;
        }
        absent_index = index;
        if first <= last {
            push_measure(
                &mut runs,
                MeasureRun {
                    first_hour: first as i32,
                    last_hour: last as i32,
                    value,
                    starts_segment: run.starts_segment || first != i64::from(run.first_hour),
                },
                count,
                cancelled,
            )?;
        }
    }
    for span in absent {
        push_measure(
            &mut runs,
            MeasureRun {
                first_hour: span.first_hour,
                last_hour: span.last_hour,
                starts_segment: true,
                value: Some(0),
            },
            count,
            cancelled,
        )?;
    }
    runs.sort_by_key(|run| run.first_hour);
    debug_assert!(
        runs.windows(2)
            .all(|pair| pair[0].last_hour < pair[1].first_hour)
    );
    Ok(runs)
}

pub(crate) fn faction_tracks(
    source: &Source,
    branch: u64,
    tags: &[String],
    mode: FactoryCount,
    groups: &[FactoryGroupKey],
    history: &super::country_groups::FactionHistory,
    cancelled: &AtomicBool,
) -> io::Result<super::faction_totals::CountryMeasures> {
    use super::faction_totals::{CountryMeasures, CountryTrack, for_each_absent};
    let scanned = scanned_branch(source, branch, cancelled)?;
    let groups = if groups.is_empty() {
        &FactoryGroupKey::ALL
    } else {
        groups
    };
    let mut count = 0usize;
    let mut tracks = Vec::with_capacity(tags.len());
    for tag in tags {
        let mut absent = Vec::new();
        for_each_absent(
            &history.country_reads,
            history
                .country_presence
                .get(tag)
                .map(Vec::as_slice)
                .unwrap_or_default(),
            cancelled,
            |first_hour, last_hour| {
                absent.push(HourSpan {
                    first_hour,
                    last_hour,
                });
                Ok(())
            },
        )?;
        tracks.push(CountryTrack {
            tag: tag.clone(),
            runs: faction_measure_runs(
                scanned
                    .series
                    .get(tag)
                    .map(Vec::as_slice)
                    .unwrap_or_default(),
                &absent,
                mode,
                groups,
                &mut count,
                cancelled,
            )?,
        });
    }
    let mut warnings = Vec::new();
    if scanned.unreadable > 0 || scanned.unnamed > 0 {
        warnings.push(format!("{} factory reads could not name a country or show its count; affected faction totals are gaps.", scanned.unreadable + scanned.unnamed));
    }
    Ok(CountryMeasures {
        tracks,
        unit: Some("factories".into()),
        unavailable_reason: None,
        warnings,
    })
}

#[cfg(test)]
mod tests;
