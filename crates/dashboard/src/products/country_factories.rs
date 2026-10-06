//! Every country's factories over one branch, for the Factories widget: each of
//! the game's three factory groups as the game last recalculated it, hour by hour.
//!
//! The values are the recorded `CountryFactories` rows (table 257, hourly): the
//! stored totals the top bar, the factory tooltips and the factory triggers read
//! (`CProductionStatus` +0x2b0.., see `engine::factory_group` and
//! `engine::civ_factory_consumer`). A group's buildings count through their
//! content flags (`military_production`, `naval_production`,
//! `general_production`), in the states the country controls and its offsite
//! buildings, so the territory counted is the game's own: occupied land
//! included, its own land held by an enemy left out.
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
//! - the top bar's group total: T − D − given to the overlord − given to others
//!   (`CTopBar::BuildFactoryStatusTooltip@14187f760`).
//!
//! The game adds every withheld count above into the group's `unavailable`
//! (`RecalculateFactoryTotals@140e57f10`), which the reply does not repeat: an
//! hour whose recorded `unavailable` differs from those parts is counted in a
//! warning rather than hidden.
//!
//! Runs compress equal consecutive hours and never bridge a gap, an hour whose
//! tables do not tell (the country or factory table incomplete, the links unable
//! to resolve the tag) or a change of the country identity under a tag. The
//! table is read every hour by cadence (`slot_read`), even when the shared
//! read is timed against another table. Values start at the hour supplying them.
//!
//! A request names what the widget draws, so a reply never carries every
//! country's history: either chosen tags (at most [`MAX_COUNTRIES`]), or the top
//! N by a count, ranked here over every country at every hour a value changes.
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
/// Runs the scan keeps over every country before it refuses, bounding its memory.
const MAX_RUNS: usize = 1_000_000;
const CACHED_BRANCHES: usize = 2;
const MAX_NAME: usize = 256;
/// Most countries one request names, and the largest N of a top-N request.
pub const MAX_COUNTRIES: usize = 30;
/// The most a reply's series may take serialised: well inside the transport's
/// 32 MiB with the groups, the fields and the envelope.
pub use super::MAX_PRODUCT_REPLY_BYTES as MAX_REPLY_BYTES;
/// JSON of one run besides its numbers: the keys, braces and commas.
const RUN_OVERHEAD: usize = 64;

/// One of the game's factory groups (`CProductionStatus` +0x2b0, +0x310, +0x370,
/// zeroed by `ResetFactoryTotals@140e51760` and rebuilt by
/// `RecalculateFactoryTotals@140e57f10`). These three are engine structures, not
/// content, so a mod cannot add a fourth: which buildings count in each, a mod's
/// own factory types included, comes from the content flag [`FactoryGroupKey::flag`].
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, TS)]
#[serde(rename_all = "camelCase")]
pub enum FactoryGroupKey {
    Military,
    Naval,
    Civilian,
}

impl FactoryGroupKey {
    pub const ALL: [Self; 3] = [Self::Military, Self::Naval, Self::Civilian];

    /// The building template flag that counts a building in this group: engine
    /// tokens the template reader `1414a9220` reads (`military_production` token
    /// 0x2f5c into `CBuildingTemplate` +0x2fc, `naval_production` 0x2f5b into +0x2f8,
    /// a building to the group (`installed_content::buildings::FactoryFlags`).
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

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactorySeries {
    pub tag: String,
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
    let mut runs = 0usize;
    let mut segments = 0usize;
    for block in replay.index.iter().filter(|block| block.branch == branch) {
        if cancelled.load(Ordering::Acquire) {
            return Err(cancel());
        }
        let mut decoder = FrameDecoder::for_tables(&selected);
        let Ok(frames) = replay.read_block_selected(block, &selected) else {
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
                if !observe_identity(
                    &mut track.segments,
                    &mut track.last_identity,
                    identity,
                    hour,
                ) {
                    segments += 1;
                    if segments > MAX_RUNS {
                        return Err(io::Error::other(
                            "The country identity history exceeds its memory budget",
                        ));
                    }
                }
                // Factory availability must not change a known country identity.
                if rows.is_none() {
                    continue;
                }
                let continues = track
                    .last
                    .is_some_and(|(seen, at)| seen == identity && at.checked_add(1) == Some(hour));
                if !read {
                    // Carried: the last read still holds, only for the same identity.
                    if continues && let Some(run) = track.runs.last_mut() {
                        run.last_hour = hour;
                        track.last = Some((identity, hour));
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
                match track.runs.last_mut() {
                    Some(run) if continues && run.values == values => run.last_hour = hour,
                    _ => {
                        runs += 1;
                        if runs > MAX_RUNS {
                            return Err(io::Error::other(
                                "The factory history exceeds its memory budget",
                            ));
                        }
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
    for (tag, track) in tracks {
        if !track.runs.is_empty() {
            result.segments.insert(tag.clone(), track.segments);
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
                    .sum(),
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
    parts_bytes(&requested_parts(scanned, params, cancelled)?, cancelled)
}

type Parts<'a> = [Part<'a>];

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
    let bytes = parts_bytes(&parts, cancelled)?;
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
                    name: localise(token).filter(|name| !name.is_empty() && name.len() <= MAX_NAME),
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

fn content_groups(catalogue: &Catalogue) -> (Vec<FactoryGroupInfo>, Vec<String>) {
    let mut warnings = catalogue.verification.clone();
    warnings.extend(catalogue.warnings.iter().cloned());
    let (groups, notes) = describe_groups(&catalogue.content.layers, |token| {
        // CBuildingTemplate::constructor@1414a4470 localises the raw key with
        // localization_substitution_record::consumer@142230F20 (no script scope)
        // into +0x210. get_display_name@1414a7570 returns that cached name to
        // CProductionLineBuildingItem::vta48_vf0@141d40750 in the construction UI.
        // A type without localisation stays unnamed.
        catalogue.localisation.get(token)?;
        catalogue.localised(token, catalogue::View::Plain)
    });
    warnings.extend(notes);
    warnings.extend(catalogue.refusal_lines());
    (groups, warnings)
}

/// The groups as the recorded content stack names them: every building type any
/// layer declares with a group's flag (a mod's own factory types included, a
/// vanilla one a mod removed left out), named through `localise`. What cannot be
/// read is said in a warning and named visibly: a type with no name by its token, a
/// group no building counts in by its flag, and the whole list by the flags when
/// the building files cannot be read (such as an unsupported modifier block).
pub fn describe_groups(
    layers: &installed_content::ResolvedLayers,
    localise: impl Fn(&str) -> Option<String>,
) -> (Vec<FactoryGroupInfo>, Vec<String>) {
    let mut warnings = Vec::new();
    match installed_content::buildings::read(layers) {
        Ok(buildings) => {
            for token in &buildings.ambiguous_factory_types {
                warnings.push(format!(
                    "{token} is declared more than once with different factory flags; which group it counts in is not known, so it names none."
                ));
            }
            let groups = group_buildings(&buildings, localise);
            for group in &groups {
                if group.buildings.is_empty() {
                    warnings.push(format!(
                        "No building in the recorded content has {}; its group is named by the flag.",
                        group.flag
                    ));
                }
                for building in group
                    .buildings
                    .iter()
                    .filter(|building| building.name.is_none())
                {
                    warnings.push(format!(
                        "{}: no name in the recorded content; shown by its token.",
                        building.token
                    ));
                }
            }
            (groups, warnings)
        }
        Err(error) => {
            warnings.push(format!(
                "Factory types are unavailable, so the groups are named by their flags: {error}"
            ));
            (unnamed_groups(), warnings)
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
            FactorySeries::decl(),
            CountryFactoriesReply::decl(),
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
        if let Some(top) = &params.top {
            let mut groups = top.groups.clone();
            groups.sort_unstable();
            groups.dedup();
            if !params.tags.is_empty()
                || top.count == 0
                || top.count as usize > MAX_COUNTRIES
                || groups.len() != top.groups.len()
            {
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
        Ok(CountryFactoriesReply {
            branch_id: params.branch_id.clone(),
            groups,
            fields: fields(),
            series: requested_series(&scanned, params, cancelled)?,
            warnings,
        })
    }
}

#[cfg(test)]
mod tests;
