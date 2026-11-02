//! Recording-window land census; names resolve only for drawn rows.
//!
//! Combat day starts at signed i32 -1 and duration at zero. Resolved rounds
//! increment duration; continued rounds advance day. Save/load stores these
//! counters, not dates. Neither establishes an absolute start. Dates below
//! are observation bounds.
//!
//! Combatants are created attacker then defender. The complete side roster
//! includes committed, reserve and neither-role participants. LandCombats
//! records its full u64 army handles. Country sets keep logical owners and
//! distinct positive roster owners: both identify recorded participants,
//! never a winner. Raw u32 references resolve at that read.
//!
//! The complete u64 identity, branch and observed episode keep reused handles
//! separate. The combat stores its province, whose u32 ID is recorded.
//! LandCombats/LandCombatSides are hourly (replay_data::cadence); slot_read dates
//! successful reads. LandCombatProgress supplies hourly loss counters and
//! LandCombatResults supplies post-flush losses and the finalizer verdict.
//! The finalizer uses side bit 1 and a signed qword result. Disappearance alone
//! never selects a winner or finalizes losses.

//! Each valid unfinished combat resolves one round per game hour. Duration
//! therefore counts resolved hourly rounds; day counts continued rounds, not
//! elapsed days. Length uses the recorded duration counter as a minimum;
//! after a gap its baseline resets at the new segment's first read, so neither
//! counter bridges the gap.
//!
//! Participant lifecycle records remaining manpower, while active damage
//! records removed manpower. Combat Log damage accumulates that i32 value.
//! Retained combat data stores two i32 losses with the selected role but no
//! stable live-combat reference. The live observer supplies hourly manpower
//! and fixed5 variant losses; the finalizer's pending-damage flush supplies
//! terminal counters. War-wide totals cannot attribute them.
use dashboard_base::{
    products::{
        MAX_PRODUCT_REPLY_BYTES, Product, ReplyWarnings,
        battles::{self, BattleBound, BattleEnd, BattleMoment, BattleWarning, BattleWindow},
    },
    source::Source,
};
use dashboard_core::{config::LoadContext, transport::json_len};
use dashboard_text::metadata::{LinkTags, check_cancel};
use replay::container::Replay;
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};
use ts_rs::TS;
#[cfg(any(test, feature = "test-support"))]
use {
    dashboard_text::metadata::{slot_read, validate_campaign},
    replay_data::{rows::TableId, rows::tables},
    replay_frame::frame::{DecodedFrame, FrameDecoder},
    std::collections::VecDeque,
};

pub mod emit;
pub mod facts;
mod losses;
pub mod prepared;
mod served;
mod terrain;
mod warnings;
pub use terrain::LandBattleTerrains;
pub use terrain::check_content as check_terrain_content;

pub struct LandBattles;
pub struct LandBattlesManifest;
pub struct LandBattlesRanking;
/// One battle's whole course, for a drill.
pub struct LandBattleCourses;
/// Every table the row census reads.
#[cfg(any(test, feature = "test-support"))]
pub const CENSUS_TABLES: [TableId; 14] = [
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::LandCombats,
    TableId::LandCombatSides,
    TableId::LandCombatProgress,
    TableId::LandCombatResults,
    TableId::LandEventCoverage,
    TableId::CountryAppearance,
    TableId::LexicalTokens,
    TableId::VariantDefinitions,
    TableId::VariantModules,
    TableId::VariantUpgrades,
    TableId::VariantChanges,
];

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LandBattlesParams {
    pub branch_id: String,
    pub window: usize,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LandBattlesManifestParams {
    pub branch_id: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LandBattlePeak {
    #[ts(type = "number")]
    pub value: u64,
    pub hour: i32,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LandBattleRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub counts: [u32; 2],
    pub roster_unknown: bool,
    pub countries: [Vec<String>; 2],
    pub countries_unknown: [bool; 2],
    /// Exact native counter at first_hour; every subsequent recorded read steps by one.
    pub duration: i32,
}
impl LandBattleRun {
    fn duration_at(&self, hour: i32) -> i64 {
        i64::from(self.duration) + i64::from(hour) - i64::from(self.first_hour)
    }
    fn clipped(&self, first: i32, last: i32) -> Self {
        Self {
            first_hour: first,
            last_hour: last,
            duration: self.duration_at(first) as i32,
            ..self.clone()
        }
    }
}
/// Recorded cumulative observer counters, with explicit unknown reads at gaps.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
pub struct LandBattleLossRead {
    pub hour: i32,
    /// True only for the post-flush FinalizeCombat read, never inferred from absence.
    pub terminal: bool,
    /// Attacker then defender, or why neither side's counters were read.
    pub sides: LandLossTotals,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(untagged)]
pub enum LandLossTotals {
    Read([LandLossTotal; 2]),
    Unread(String),
}
/// One side's counters in the census: manpower and the exact signed fixed5 sum
/// of every design's equipment loss. Per-design amounts are in the battle's course.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(untagged)]
pub enum LandLossTotal {
    Known {
        manpower: LandManpower,
        equipment: String,
    },
    /// Why the equipment pool is unknown.
    Unknown {
        manpower: LandManpower,
        unknown: String,
    },
}
/// One read of a battle's course, with each design's equipment loss.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
pub struct LandBattleCourseRead {
    pub hour: i32,
    /// True only for the post-flush FinalizeCombat read, never inferred from absence.
    pub terminal: bool,
    /// Attacker then defender, or why neither side's counters were read.
    pub sides: LandLossSides,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(untagged)]
pub enum LandLossSides {
    Read([LandLossSide; 2]),
    Unread(String),
}
/// One side's counters. Variants pair a reply design index with the exact signed
/// fixed5 amount, in recorded order; the equipment total is their sum.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(untagged)]
pub enum LandLossSide {
    /// Every variant: the side's first read in the reply, or its labels or order changed.
    Variants {
        manpower: LandManpower,
        variants: Vec<(usize, String)>,
    },
    /// The positions whose amounts changed since the side's previous read in the reply.
    Changed {
        manpower: LandManpower,
        changed: Vec<(usize, String)>,
    },
    /// Why the equipment pool is unknown.
    Unknown {
        manpower: LandManpower,
        unknown: String,
    },
}
/// Manpower lost as recorded, or why it is unknown.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(untagged)]
pub enum LandManpower {
    Known(i32),
    Unknown(String),
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LandBattleHistory {
    pub id: String,
    pub province_id: u32,
    pub start: BattleMoment,
    pub end: Option<BattleEnd>,
    pub end_unknown_hour: Option<i32>,
    pub counter_base: i32,
    pub prior_peak: Option<LandBattlePeak>,
    /// Each side's most divisions at once before the reply's runs, attacker then defender.
    pub prior_side_peaks: [Option<LandBattlePeak>; 2],
    pub runs: Vec<LandBattleRun>,
    pub losses: Vec<LandBattleLossRead>,
    pub result: Option<LandBattleResultRead>,
}
/// A battle's whole observed course: every run and every loss read by design.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LandBattleCourse {
    pub id: String,
    pub province_id: u32,
    pub start: BattleMoment,
    pub end: Option<BattleEnd>,
    pub end_unknown_hour: Option<i32>,
    pub counter_base: i32,
    pub runs: Vec<LandBattleRun>,
    pub losses: Vec<LandBattleCourseRead>,
    pub result: Option<LandBattleResultRead>,
}
/// An equipment design as a loss read names it. Same-named designs stay apart by handle.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize, TS)]
pub struct LandEquipmentDesign {
    /// The design's name from the content stack; null when it cannot be named.
    pub name: Option<String>,
    pub variant: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
pub struct LandBattleResultRead {
    pub hour: i32,
    pub duration: i32,
    pub winner: battles::BattleWinner,
    pub countries: [Vec<String>; 2],
    #[serde(rename = "countriesUnknown")]
    pub countries_unknown: [bool; 2],
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LandBattlesManifestReply {
    /// Windows come from this recording's replay blocks, never a chosen duration.
    pub windows: Vec<BattleWindow>,
    pub available: bool,
    pub unavailable: Option<String>,
    pub content_warnings: Vec<String>,
}
impl ReplyWarnings for LandBattlesManifestReply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        &mut self.content_warnings
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LandBattlesReply {
    pub window: usize,
    pub interval: BattleWindow,
    pub rows: Vec<LandBattleHistory>,
    pub observed: Vec<(i32, i32)>,
    /// Each warning reason once, for `warnings` to refer to by index.
    pub reasons: Vec<String>,
    /// [hour, reason, count] in hour then reason order.
    pub warnings: Vec<(i32, usize, usize)>,
    pub content_warnings: Vec<String>,
}
impl ReplyWarnings for LandBattlesReply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        &mut self.content_warnings
    }
}
/// Same dated country predicate as client countryMatch, needed before server top-N.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LandBattleCountryFilter {
    pub tags: Option<Vec<String>>,
    pub membership: Option<Vec<dashboard_base::products::country_groups::MembershipRun>>,
    pub unobserved: Vec<dashboard_base::products::country_groups::HourSpan>,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LandBattlesRankingParams {
    pub branch_id: String,
    pub first_hour: i32,
    pub last_hour: i32,
    pub top_n: f64,
    pub measure: String,
    pub countries: LandBattleCountryFilter,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LandBattlesRankingReply {
    pub rows: Vec<LandBattleHistory>,
    pub census_hour: Option<i32>,
    pub effective_measure: String,
    pub warnings: Vec<BattleWarning>,
    pub content_warnings: Vec<String>,
}
impl ReplyWarnings for LandBattlesRankingReply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        &mut self.content_warnings
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LandBattleCourseParams {
    pub branch_id: String,
    /// A battle id from this branch's census.
    pub id: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LandBattleCourseReply {
    pub battle: LandBattleCourse,
    /// Equipment designs, which the course's reads refer to by index.
    pub designs: Vec<LandEquipmentDesign>,
    pub content_warnings: Vec<String>,
}
impl ReplyWarnings for LandBattleCourseReply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        &mut self.content_warnings
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Episode {
    id: String,
    province: u32,
    start: BattleMoment,
    end: Option<(i32, BattleEnd)>,
    counter_base: i32,
    runs: Vec<LandBattleRun>,
    losses: Vec<losses::LossRead>,
    result: Option<LandBattleResultRead>,
    loss_inputs: Option<[losses::SideLossInputs; 2]>,
    prior_peaks: [Option<LandBattlePeak>; 3],
    /// The newest page of the runs and loss reads paged out before these
    /// (`prepared::page_out`), whose peaks `prior_peaks` holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    history: Option<u64>,
}
/// A finalizer for a combat whose newest battle may be paged out: publication
/// reads that battle and ends it, or keeps this finalizer's own battle.
#[derive(Serialize, Deserialize)]
struct Pending {
    combat: u64,
    from_hour: i32,
    /// The finalizer's own battle, already ended, at the ordinal it was given.
    at: usize,
    own: Episode,
}
/// Keys numbered in first-use order, so a reply sends each text once.
#[derive(Default)]
struct Numbered<K>(HashMap<K, usize>);
impl<K: Eq + std::hash::Hash> Numbered<K> {
    fn index(&mut self, key: K) -> usize {
        let next = self.0.len();
        *self.0.entry(key).or_insert(next)
    }
    fn into_vec(self) -> Vec<K> {
        let mut keys: Vec<_> = self.0.into_iter().collect();
        keys.sort_unstable_by_key(|(_, index)| *index);
        keys.into_iter().map(|(key, _)| key).collect()
    }
}
impl Episode {
    /// Equal native counters do not interpolate: the oracle starts a new run
    /// at every subsequent read. Expand only accepted hours, never a gap.
    fn expand_constant(&mut self, through: i32) -> usize {
        let Some(last) = self.runs.last() else {
            return 0;
        };
        let duration = last.duration_at(last.last_hour) as i32;
        let mut hour = last.last_hour;
        let mut emitted = 0;
        while hour < through {
            hour += 1;
            let last = self.runs.last().expect("accepted live run");
            self.runs.push(LandBattleRun {
                first_hour: hour,
                last_hour: hour,
                duration,
                ..last.clone()
            });
            emitted += 1;
        }
        emitted
    }
    /// `peaks_through` dates the prior peaks: the total and each side's most
    /// divisions at once in runs starting at or before it.
    fn history(
        &self,
        runs: Vec<LandBattleRun>,
        last: i32,
        peaks_through: Option<i32>,
    ) -> LandBattleHistory {
        let end = self.end.as_ref().filter(|(hour, _)| *hour <= last);
        LandBattleHistory {
            id: self.id.clone(),
            province_id: self.province,
            start: self.start.clone(),
            end: end.map(|(_, end)| end.clone()),
            end_unknown_hour: end
                .and_then(|(hour, end)| matches!(end, BattleEnd::Unknown { .. }).then_some(*hour)),
            losses: self.loss_reads(runs[0].first_hour, last),
            result: self.result.clone().filter(|result| result.hour <= last),
            counter_base: self.counter_base,
            prior_peak: peaks_through.and_then(|hour| self.peak_before(hour)),
            prior_side_peaks: std::array::from_fn(|side| {
                peaks_through.and_then(|hour| self.side_peak_before(hour, side))
            }),
            runs,
        }
    }
    /// Every run and every loss read, with each design's amount.
    fn course(&self, designs: &mut Numbered<usize>) -> LandBattleCourse {
        let end = self.end.as_ref();
        let mut previous = [None, None];
        LandBattleCourse {
            id: self.id.clone(),
            province_id: self.province,
            start: self.start.clone(),
            end: end.map(|(_, end)| end.clone()),
            end_unknown_hour: end
                .and_then(|(hour, end)| matches!(end, BattleEnd::Unknown { .. }).then_some(*hour)),
            counter_base: self.counter_base,
            runs: self.runs.clone(),
            losses: self
                .losses
                .iter()
                .map(|read| read.wire(&mut previous, designs))
                .collect(),
            result: self.result.clone(),
        }
    }
    fn loss_at(&self, hour: i32) -> Option<&losses::LossRead> {
        self.losses.iter().rev().find(|read| read.hour <= hour)
    }
    fn loss_reads(&self, first: i32, last: i32) -> Vec<LandBattleLossRead> {
        // Carry one prior supplying read, not the entire casualty history.
        self.loss_at(first)
            .into_iter()
            .chain(
                self.losses
                    .iter()
                    .filter(|read| read.hour > first && read.hour <= last),
            )
            .map(losses::LossRead::total)
            .collect()
    }
    fn casualties_at(&self, hour: i32) -> Option<f64> {
        self.loss_at(hour)?.casualties()
    }
    fn peak_before(&self, hour: i32) -> Option<LandBattlePeak> {
        self.with_prior(
            0,
            hour,
            self.peak_by(hour, |counts| u64::from(counts[0]) + u64::from(counts[1])),
        )
    }
    fn side_peak_before(&self, hour: i32, side: usize) -> Option<LandBattlePeak> {
        self.with_prior(
            side + 1,
            hour,
            self.peak_by(hour, |counts| u64::from(counts[side])),
        )
    }
    fn with_prior(
        &self,
        index: usize,
        hour: i32,
        peak: Option<LandBattlePeak>,
    ) -> Option<LandBattlePeak> {
        let prior = self.prior_peaks[index]
            .clone()
            .filter(|peak| peak.hour <= hour);
        match (prior, peak) {
            (Some(prior), Some(peak)) if peak.value > prior.value => Some(peak),
            (Some(prior), _) => Some(prior),
            (None, peak) => peak,
        }
    }
    /// Folds `runs`, which precede every run held, into the earlier peaks.
    fn fold_peaks(&mut self, runs: &[LandBattleRun]) {
        for run in runs.iter().filter(|run| !run.roster_unknown) {
            let [attackers, defenders] = run.counts.map(u64::from);
            for (peak, value) in
                self.prior_peaks
                    .iter_mut()
                    .zip([attackers + defenders, attackers, defenders])
            {
                if peak.as_ref().is_none_or(|peak| value > peak.value) {
                    *peak = Some(LandBattlePeak {
                        value,
                        hour: run.first_hour,
                    });
                }
            }
        }
    }
    /// The largest known roster value in runs starting at or before `hour`, at its earliest hour.
    fn peak_by(&self, hour: i32, value: impl Fn(&[u32; 2]) -> u64) -> Option<LandBattlePeak> {
        self.runs
            .iter()
            .take_while(|run| run.first_hour <= hour)
            .fold(None, |peak: Option<LandBattlePeak>, run| {
                if run.roster_unknown {
                    return peak;
                }
                let value = value(&run.counts);
                if peak.as_ref().is_none_or(|peak| value > peak.value) {
                    Some(LandBattlePeak {
                        value,
                        hour: run.first_hour,
                    })
                } else {
                    peak
                }
            })
    }
}
#[derive(Default, Serialize, Deserialize)]
pub struct Census {
    /// The recording's format, which dates its daily tables
    /// ([`dashboard_text::metadata::slot_read`]).
    #[serde(skip)]
    format: replay_data::format::FormatVersion,
    /// Battles in memory by ordinal, their order of creation: every open one
    /// and every one a publication may still draw (`prepared::page_out`).
    episodes: BTreeMap<usize, Episode>,
    /// Battles created, so the next one's ordinal.
    created: usize,
    /// The earliest start of any battle, which opens the first ranking interval.
    first_start: Option<i32>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    open: HashMap<u64, usize>,
    links: LinkTags,
    observed: Vec<(i32, i32)>,
    warnings: warnings::Warnings,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    latest: HashMap<u64, usize>,
    variants: dashboard_base::products::equipment::VariantCatalogue,
    #[serde(skip)]
    catalogue: Option<Arc<dashboard_text::catalogue::Catalogue>>,
    content_warnings: Vec<String>,
    /// The content refusals prepared observation's own reads produced, which
    /// its published warnings report.
    #[serde(default)]
    refusals: dashboard_text::catalogue::OwnRefusals,
    designs: losses::Designs,
    /// Each variant's design and whether it is named, as its current names give them.
    #[serde(skip)]
    named: HashMap<u64, (usize, bool)>,
    /// The hour and variant of each unnamed design read this hour, not warned of yet.
    #[serde(skip)]
    unnamed: Vec<(i32, u64)>,
    /// Ordinals whose course pages are published.
    keyed: usize,
    /// Battles that ended before this hour, and warnings dated before it,
    /// are paged out to the prepared store.
    paged_before: Option<i32>,
    pending: Vec<Pending>,
    windows: dashboard_base::prepared::series::Accumulator,
    reads: dashboard_base::prepared::series::Accumulator,
    available: bool,
    #[serde(skip)]
    configured: bool,
    #[serde(skip)]
    dirty: BTreeSet<usize>,
    /// Only successful adjacent reads with identical live operands advance it.
    /// observed retains the materialized endpoint until the carry is expanded.
    deferred_through: Option<i32>,
    live_warnings: BTreeMap<String, usize>,
    #[serde(skip)]
    labels_known: bool,
    /// Current resolved participant operands (R2b), rebuilt from A3 on restore.
    #[serde(skip)]
    live_sides: HashMap<u64, LiveSide>,
    #[serde(skip)]
    live_sides_read: bool,
    #[cfg(test)]
    #[serde(skip)]
    work: dashboard_base::prepared::observer_tests::Work,
}
#[derive(Clone)]
struct LiveSide {
    countries: [Vec<String>; 2],
    unknown: [bool; 2],
    warnings: BTreeMap<String, usize>,
}
impl LiveSide {
    fn new(
        side: Option<&replay_data::rows::LandCombatSideRow>,
        links: &LinkTags,
        labels: bool,
    ) -> Self {
        let mut countries = [Vec::new(), Vec::new()];
        let mut unknown = [!labels || side.is_none(); 2];
        let mut warnings = BTreeMap::<String, usize>::new();
        if labels && let Some(side) = side {
            for (index, (primary, secondary)) in [
                (&side.attacker_primary, &side.attacker_secondary),
                (&side.defender_primary, &side.defender_secondary),
            ]
            .into_iter()
            .enumerate()
            {
                unknown[index] = primary.is_empty() && secondary.is_empty();
                for raw in primary.iter().chain(secondary) {
                    match links.get(*raw) {
                        Some(tag) => countries[index].push(tag.clone()),
                        None => {
                            unknown[index] = true;
                            *warnings
                                .entry(format!("Participant country reference {raw} unresolved"))
                                .or_default() += 1;
                        }
                    }
                }
                countries[index].sort_unstable();
                countries[index].dedup();
            }
        }
        if unknown.contains(&true) {
            *warnings
                .entry("Participant countries not completely recorded at supplying read".into())
                .or_default() += 1;
        }
        Self {
            countries,
            unknown,
            warnings,
        }
    }
}
/// A battle the census holds in memory.
fn held(episodes: &mut BTreeMap<usize, Episode>, at: usize) -> &mut Episode {
    episodes.get_mut(&at).expect("battle held in memory")
}
impl Census {
    fn warn(&mut self, hour: i32, reason: impl AsRef<str> + Into<String>) {
        self.warnings.add(hour, reason, 1);
    }
    /// One LandCombats row at a read: its battle goes on, or a new one starts at `hour`.
    #[allow(clippy::too_many_arguments)]
    fn read_combat(
        &mut self,
        branch: u64,
        hour: i32,
        adjacent: bool,
        row: facts::Combat,
        countries: [Vec<String>; 2],
        countries_unknown: [bool; 2],
        opened: &mut Vec<u64>,
    ) {
        let current = self.open.get(&row.id).map(|at| (*at, &self.episodes[at]));
        let continuing = current
            .filter(|(_, episode)| {
                episode.province == row.province
                    && episode
                        .runs
                        .last()
                        .is_none_or(|run| i64::from(row.duration) >= run.duration_at(run.last_hour))
            })
            .map(|(at, _)| at);
        let at = match continuing {
            Some(at) => at,
            None => {
                opened.push(row.id);
                if let Some(old) = self.open.remove(&row.id) {
                    self.dirty.insert(old);
                    held(&mut self.episodes, old).end = Some((
                        hour,
                        BattleEnd::Moment(BattleMoment {
                            hour,
                            bound: BattleBound::Exact,
                        }),
                    ));
                }
                let at = self.begin(hour);
                self.episodes.insert(
                    at,
                    Episode {
                        id: format!("{branch}:{}:{hour}", row.id),
                        province: row.province,
                        start: BattleMoment {
                            hour,
                            bound: if adjacent {
                                BattleBound::Exact
                            } else {
                                BattleBound::By
                            },
                        },
                        end: None,
                        counter_base: if adjacent { 0 } else { row.duration },
                        runs: Vec::with_capacity(1),
                        losses: Vec::new(),
                        result: None,
                        loss_inputs: None,
                        prior_peaks: [None, None, None],
                        history: None,
                    },
                );
                self.open.insert(row.id, at);
                self.latest.insert(row.id, at);
                at
            }
        };
        self.dirty.insert(at);
        let counts = row.counts;
        let runs = &mut held(&mut self.episodes, at).runs;
        if let Some(run) = runs.last_mut().filter(|run| {
            run.last_hour.checked_add(1) == Some(hour)
                && run.counts == counts
                && run.countries == countries
                && run.countries_unknown == countries_unknown
                && run.duration_at(hour) == i64::from(row.duration)
        }) {
            run.last_hour = hour;
        } else {
            #[cfg(test)]
            {
                self.work.emitted += 1;
            }
            runs.push(LandBattleRun {
                first_hour: hour,
                last_hour: hour,
                counts,
                roster_unknown: false,
                countries,
                countries_unknown,
                duration: row.duration,
            });
        }
    }
    /// The ordinal of a new battle starting at `hour`.
    fn begin(&mut self, hour: i32) -> usize {
        self.started(hour);
        self.reserve()
    }
    fn reserve(&mut self) -> usize {
        self.created += 1;
        self.created - 1
    }
    fn started(&mut self, hour: i32) {
        self.first_start = Some(self.first_start.map_or(hour, |first| first.min(hour)));
    }
    fn advance(&mut self, hour: i32) {
        self.deferred_through = Some(hour);
    }
    fn materialize(&mut self) -> io::Result<()> {
        let Some(through) = self.deferred_through.take() else {
            return Ok(());
        };
        for &at in self.open.values() {
            let emitted = held(&mut self.episodes, at).expand_constant(through);
            if emitted != 0 {
                self.dirty.insert(at);
            }
            #[cfg(test)]
            {
                self.work.history_visits += 1;
                self.work.emitted += emitted;
            }
        }
        let (_, last) = self.observed.last_mut().expect("accepted live census");
        while *last < through {
            *last += 1;
            self.reads.event(0, *last, &*last)?;
            for (reason, count) in &self.live_warnings {
                self.warnings.add(*last, reason.as_str(), *count);
            }
        }
        Ok(())
    }
    fn effective_episode<'a>(&self, episode: &'a Episode) -> Cow<'a, Episode> {
        if let Some(through) = self.deferred_through
            && episode.end.is_none()
            && episode
                .runs
                .last()
                .is_some_and(|run| run.last_hour < through)
        {
            let mut episode = episode.clone();
            episode.expand_constant(through);
            Cow::Owned(episode)
        } else {
            Cow::Borrowed(episode)
        }
    }
    fn observed_spans(&self) -> impl DoubleEndedIterator<Item = (i32, i32)> + '_ {
        self.observed
            .iter()
            .enumerate()
            .map(|(at, &(first, last))| {
                let last = if at + 1 == self.observed.len() {
                    self.deferred_through.unwrap_or(last)
                } else {
                    last
                };
                (first, last)
            })
    }
    fn warning_counts(&self, first: i32, last: i32) -> warnings::Warnings {
        self.counts_in(&self.warnings, first, last)
    }
    /// `warnings` dated `first` through `last`, with the reads deferred since
    /// the last census read.
    fn counts_in(
        &self,
        warnings: &warnings::Warnings,
        first: i32,
        last: i32,
    ) -> warnings::Warnings {
        let mut warnings = warnings.between(first, last);
        if let Some(through) = self.deferred_through {
            let base = self.observed.last().expect("accepted live census").1;
            let mut hour = base;
            while hour < through.min(last) {
                hour += 1;
                if hour >= first {
                    for (reason, count) in &self.live_warnings {
                        warnings.add(hour, reason.as_str(), *count);
                    }
                }
            }
        }
        warnings
    }
    fn reply_designs(&self, used: Numbered<usize>) -> Vec<LandEquipmentDesign> {
        used.into_vec()
            .into_iter()
            .map(|design| self.designs.values[design].clone())
            .collect()
    }
    fn content_warnings(&self) -> Vec<String> {
        let mut warnings = self.content_warnings.clone();
        warnings.extend(self.variants.warnings());
        if let Some(catalogue) = &self.catalogue {
            warnings.extend(catalogue.verification.iter().cloned());
            warnings.extend(catalogue.warnings.iter().cloned());
            warnings.extend(catalogue.refusal_lines());
        }
        warnings
    }
    fn gap(&mut self, hour: i32) -> io::Result<()> {
        self.materialize()?;
        for (_, at) in self.open.drain() {
            self.dirty.insert(at);
            let episode = held(&mut self.episodes, at);
            episode.lose_losses(hour);
            episode.end = Some((
                hour,
                BattleEnd::Unknown {
                    unknown: "Ending not recorded".into(),
                },
            ));
        }
        Ok(())
    }
    /// The row census, the parity oracle of the native one ([`emit`]): observes a decoded
    /// frame with no change facts, every table treated as changed.
    #[cfg(any(test, feature = "test-support"))]
    pub fn observe(&mut self, frame: &DecodedFrame) -> io::Result<()> {
        self.observe_with_changes(frame, None)
    }
    /// [`Self::observe`] with the hour's change facts, as the row walk read them.
    #[cfg(any(test, feature = "test-support"))]
    pub fn observe_prepared(
        &mut self,
        frame: &DecodedFrame,
        changes: &dashboard_base::prepared::frame_changes::FrameChanges,
    ) -> io::Result<()> {
        self.observe_with_changes(frame, Some(changes))
    }
    /// Observes as one consumer of the shared content: its published
    /// warnings name the refusals of its own reads.
    #[cfg(any(test, feature = "test-support"))]
    fn observe_with_changes(
        &mut self,
        frame: &DecodedFrame,
        changes: Option<&dashboard_base::prepared::frame_changes::FrameChanges>,
    ) -> io::Result<()> {
        let mut own = std::mem::take(&mut self.refusals);
        let observed = dashboard_text::catalogue::own_refusals(&mut own, || {
            self.observe_own_changes(frame, changes)
        });
        self.warn_unnamed();
        self.refusals = own;
        observed
    }
    #[cfg(any(test, feature = "test-support"))]
    fn observe_own_changes(
        &mut self,
        frame: &DecodedFrame,
        changes: Option<&dashboard_base::prepared::frame_changes::FrameChanges>,
    ) -> io::Result<()> {
        let changed =
            |table: TableId| changes.is_none_or(|changes| changes.rows_changed[table.index()]);
        let hour = frame.hour;
        // Expand older accepted reads before setting this batch's first hour.
        // In particular, a restored carry can precede this supplying frame.
        self.available |= frame_available(self.format, frame);
        // With the hour's change facts the variants read only what changed.
        match changes {
            Some(changes) => {
                let changed = self.variants.observe_changed(self.format, frame, changes);
                self.renamed(&changed);
            }
            None => {
                self.variants.observe(self.format, frame);
                self.named.clear();
            }
        }
        let update = self.links.update(frame);
        let labels = update.is_ok();
        let links_changed = labels != self.labels_known || matches!(update, Ok(true));
        self.labels_known = labels;
        let sides_read = changes.map_or_else(
            || slot_read(self.format, &frame.metrics, TableId::LandCombatSides, hour),
            |changes| changes.slot_read[TableId::LandCombatSides.index()],
        );
        let sides_changed = changed(TableId::LandCombatSides)
            || links_changed
            || sides_read != self.live_sides_read;
        if sides_changed {
            let sides = sides_read
                .then(|| frame.complete::<tables::LandCombatSides>().ok())
                .flatten();
            #[cfg(test)]
            {
                self.work.row_visits += sides.map_or(0, <[_]>::len);
            }
            self.live_sides = sides
                .into_iter()
                .flatten()
                .map(|side| (side.combat, LiveSide::new(Some(side), &self.links, labels)))
                .collect();
            self.live_sides_read = sides_read;
        }
        let previous = self
            .deferred_through
            .or_else(|| self.observed.last().map(|(_, last)| *last));
        let adjacent = previous.and_then(|last| last.checked_add(1)) == Some(hour);
        if previous.is_some() && !adjacent {
            self.gap(
                previous
                    .and_then(|last| last.checked_add(1))
                    .unwrap_or(hour),
            )?;
        }
        if !slot_read(self.format, &frame.metrics, TableId::LandCombats, hour) {
            self.gap(hour)?;
            self.observe_results(frame, labels)?;
            self.reads.through(hour);
            return Ok(());
        }
        let Ok(rows) = frame.complete::<tables::LandCombats>() else {
            self.materialize()?;
            self.warn(
                hour,
                "LandCombats unavailable or incomplete; ending not recorded",
            );
            self.gap(hour)?;
            self.observe_results(frame, labels)?;
            self.reads.through(hour);
            return Ok(());
        };
        if adjacent
            && !links_changed
            && !changed(TableId::LandCombats)
            && !sides_changed
            && self.open.len() == rows.len()
        {
            self.advance(hour);
            if changed(TableId::LandCombatProgress) {
                self.observe_losses(frame, None);
            }
            self.observe_results(frame, labels)?;
            self.reads.through(hour);
            return Ok(());
        }
        self.materialize()?;
        self.live_warnings.clear();
        #[cfg(test)]
        {
            self.work.row_visits += rows.len();
        }
        self.reads.event(0, hour, &hour)?;
        // The decoder refuses tables without unique keys, so ids are distinct.
        if let Some((_, last)) = self
            .observed
            .last_mut()
            .filter(|(_, last)| last.checked_add(1) == Some(hour))
        {
            *last = hour;
        } else {
            self.observed.push((hour, hour));
        }
        let absent: Vec<_> = if changed(TableId::LandCombats) {
            let ids: HashSet<u64> = rows.iter().map(|row| row.id).collect();
            self.open
                .keys()
                .filter(|id| !ids.contains(id))
                .copied()
                .collect()
        } else {
            Vec::new()
        };
        for id in absent {
            let at = self.open.remove(&id).expect("open battle");
            // A gap already closes open episodes as unknown; only adjacent absence reaches here.
            self.dirty.insert(at);
            held(&mut self.episodes, at).end = Some((
                hour,
                BattleEnd::Moment(BattleMoment {
                    hour,
                    bound: BattleBound::Exact,
                }),
            ));
        }
        let mut opened = Vec::new();
        static ROSTER_FIELDS: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
        let roster_fields = *ROSTER_FIELDS.get_or_init(|| {
            dashboard_base::prepared::frame_changes::field_mask(
                TableId::LandCombats,
                &["attackers", "defenders"],
            )
        });
        let roster_changes = adjacent
            .then(|| changes.and_then(|changes| changes.rows(TableId::LandCombats)))
            .flatten();
        for row in rows {
            let side = self
                .live_sides
                .get(&row.id)
                .cloned()
                .unwrap_or_else(|| LiveSide::new(None, &self.links, labels));
            for (reason, count) in side.warnings {
                *self.live_warnings.entry(reason.clone()).or_default() += count;
                self.warnings.add(hour, reason, count);
            }
            let distinct = |handles: &[u64]| -> io::Result<u32> {
                u32::try_from(handles.iter().collect::<HashSet<_>>().len())
                    .map_err(|_| io::Error::other("Recorded roster exceeds its native count"))
            };
            // Duration and other combat fields move hourly. A delta that
            // leaves both rosters alone preserves their distinct counts.
            let current = self.open.get(&row.id).map(|at| &self.episodes[at]);
            let counts = roster_changes.and_then(|(changed, deleted)| {
                let key = u128::from(row.id);
                if deleted.binary_search(&key).is_ok() {
                    return None;
                }
                if let Ok(at) = changed.binary_search_by_key(&key, |row| row.key)
                    && changed[at].mask & roster_fields != 0
                {
                    return None;
                }
                current?.runs.last().map(|run| run.counts)
            });
            let counts = match counts {
                Some(counts) => counts,
                None => {
                    #[cfg(test)]
                    dashboard_base::prepared::prepass::note("land.rosters");
                    [distinct(&row.attackers)?, distinct(&row.defenders)?]
                }
            };
            let combat = facts::Combat {
                id: row.id,
                province: row.province,
                duration: row.duration,
                counts,
            };
            self.read_combat(
                frame.branch,
                hour,
                adjacent,
                combat,
                side.countries,
                side.unknown,
                &mut opened,
            );
        }
        if changed(TableId::LandCombatProgress) {
            self.observe_losses(frame, None);
        } else if !opened.is_empty() {
            self.observe_losses(frame, Some(&opened));
        }
        self.observe_results(frame, labels)?;
        self.reads.through(hour);
        Ok(())
    }
    fn warnings_in(&self, first: i32, last: i32) -> (Vec<String>, Vec<(i32, usize, usize)>) {
        let mut reasons = Numbered::default();
        let counts = self.warning_counts(first, last);
        let warnings = counts
            .hourly()
            .into_iter()
            .map(|(hour, reason, count)| (hour, reasons.index(reason), count))
            .collect();
        (
            reasons.into_vec().into_iter().map(String::from).collect(),
            warnings,
        )
    }
    #[cfg(any(test, feature = "test-support"))]
    fn reply(
        &self,
        params: &LandBattlesParams,
        windows: &[BattleWindow],
    ) -> io::Result<LandBattlesReply> {
        let window = windows
            .get(params.window)
            .ok_or_else(|| dashboard_core::invalid("Battle window not in this recording"))?;
        self.reply_window(params.window, window)
    }
    fn reply_window(&self, ordinal: usize, window: &BattleWindow) -> io::Result<LandBattlesReply> {
        bounded(self.window_reply(ordinal, window))
    }
    /// [`Self::reply_window`] before its size is checked.
    fn window_reply(&self, ordinal: usize, window: &BattleWindow) -> LandBattlesReply {
        let mut rows = Vec::new();
        let observed = self
            .observed_spans()
            .filter(|(first, last)| *first <= window.last_hour && *last >= window.first_hour)
            .map(|(first, last)| (first.max(window.first_hour), last.min(window.last_hour)))
            .collect();
        for episode in self.episodes.values() {
            let episode = self.effective_episode(episode);
            if episode.start.hour > window.last_hour {
                continue;
            }
            let end_in_window = episode
                .end
                .as_ref()
                .is_some_and(|(hour, _)| *hour >= window.first_hour && *hour <= window.last_hour);
            let mut runs: Vec<_> = episode
                .runs
                .iter()
                .filter(|run| {
                    run.first_hour <= window.last_hour && run.last_hour >= window.first_hour
                })
                .map(|run| {
                    run.clipped(
                        run.first_hour.max(window.first_hour),
                        run.last_hour.min(window.last_hour),
                    )
                })
                .collect();
            if runs.is_empty() && end_in_window {
                let last = episode.runs.last().expect("observed episode");
                runs.push(last.clipped(last.last_hour, last.last_hour));
            }
            if runs.is_empty() {
                continue;
            }
            rows.push(episode.history(runs, window.last_hour, window.first_hour.checked_sub(1)));
        }
        let (reasons, warnings) = self.warnings_in(window.first_hour, window.last_hour);
        LandBattlesReply {
            window: ordinal,
            interval: window.clone(),
            rows,
            observed,
            reasons,
            warnings,
            content_warnings: self.content_warnings(),
        }
    }
    fn ranked(
        &self,
        params: &LandBattlesRankingParams,
        cancelled: &AtomicBool,
    ) -> io::Result<LandBattlesRankingReply> {
        let tags: Option<HashSet<_>> = params
            .countries
            .tags
            .as_ref()
            .map(|tags| tags.iter().map(String::as_str).collect());
        let mut membership: HashMap<&str, Vec<(i32, i32)>> = HashMap::new();
        if let Some(runs) = &params.countries.membership {
            for run in runs {
                membership
                    .entry(&run.tag)
                    .or_default()
                    .push((run.first_hour, run.last_hour));
            }
        }
        // Each side's countries match at their own supplying read.
        let matches = |sides: [(&[String], bool, i32); 2]| {
            sides.iter().any(|(_, unknown, _)| *unknown)
                || sides.iter().any(|(countries, _, read)| {
                    countries.iter().any(|tag| {
                        tags.as_ref().is_none_or(|tags| tags.contains(tag.as_str()))
                            && (params.countries.membership.is_none()
                                || !params.countries.unobserved.iter().any(|span| {
                                    span.first_hour <= *read && span.last_hour >= *read
                                }) && membership.get(tag.as_str()).is_some_and(|runs| {
                                    runs.iter()
                                        .any(|(first, last)| *first <= *read && *last >= *read)
                                }))
                    })
                })
        };
        let episodes: Vec<_> = self
            .episodes
            .values()
            .map(|episode| self.effective_episode(episode))
            .collect();
        let mut ranked = Vec::new();
        for episode in episodes.iter().map(|episode| episode.as_ref()) {
            check_cancel(cancelled)?;
            let Some(run) = episode
                .runs
                .iter()
                .rev()
                .find(|run| run.first_hour <= params.last_hour)
            else {
                continue;
            };
            let read = run.last_hour.min(params.last_hour);
            let result = episode
                .result
                .as_ref()
                .filter(|result| result.hour <= params.last_hour);
            let end_in_range = episode
                .end
                .as_ref()
                .is_some_and(|(hour, _)| *hour >= params.first_hour && *hour <= params.last_hour);
            let country_match = matches(std::array::from_fn(|index| {
                participants(result, run, read, index)
            }));
            if read < params.first_hour && !end_in_range || !country_match {
                continue;
            }
            let peak = episode.peak_before(params.last_hour);
            let duration = result.map_or_else(
                || run.duration_at(read),
                |result| i64::from(result.duration),
            );
            let length = (i64::from(result.map_or(read, |result| result.hour))
                - i64::from(episode.start.hour))
            .max(duration - i64::from(episode.counter_base))
            .max(0) as u64;
            ranked.push((
                episode.casualties_at(params.last_hour),
                length,
                episode,
                run,
                read,
                peak,
            ));
        }
        let effective = match params.measure.as_str() {
            "Casualties" if ranked.iter().any(|entry| entry.0.is_some()) => "Casualties",
            "Length" => "Length",
            _ => "Peak size",
        };
        let value = |entry: &(
            Option<f64>,
            u64,
            &Episode,
            &LandBattleRun,
            i32,
            Option<LandBattlePeak>,
        )| match effective {
            "Casualties" => entry.0,
            "Length" => Some(entry.1 as f64),
            _ => entry.5.as_ref().map(|peak| peak.value as f64),
        };
        ranked.sort_unstable_by(|a, b| {
            value(b)
                .partial_cmp(&value(a))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.2.id.cmp(&b.2.id))
        });
        // A huge user N saturates at usize::MAX, which cannot omit any Vec entry.
        let rows = ranked
            .into_iter()
            .take(params.top_n as usize)
            .map(|(_, _, episode, run, read, _)| {
                episode.history(
                    vec![run.clipped(read, read)],
                    params.last_hour,
                    Some(params.last_hour),
                )
            })
            .collect();
        let census_hour = self
            .observed_spans()
            .filter(|(first, _)| *first <= params.last_hour)
            .map(|(_, last)| last.min(params.last_hour))
            .max();
        bounded(LandBattlesRankingReply {
            rows,
            census_hour,
            effective_measure: effective.into(),
            content_warnings: self.content_warnings(),
            warnings: self
                .warning_counts(params.first_hour, params.last_hour)
                .totals()
                .map(|(reason, hour, count)| BattleWarning {
                    hour,
                    reason: reason.into(),
                    count,
                })
                .collect(),
        })
    }
}
/// One side's participants as of a ranking read: the result's countries, unless the
/// result does not record that side while the last hourly read did. Each comes with
/// whether it is unknown and the hour it was read.
fn participants<'a>(
    result: Option<&'a LandBattleResultRead>,
    run: &'a LandBattleRun,
    read: i32,
    index: usize,
) -> (&'a [String], bool, i32) {
    match result {
        Some(result) if !result.countries_unknown[index] || run.countries_unknown[index] => (
            result.countries[index].as_slice(),
            result.countries_unknown[index],
            result.hour,
        ),
        _ => (
            run.countries[index].as_slice(),
            run.countries_unknown[index],
            read,
        ),
    }
}
impl Census {
    #[cfg(any(test, feature = "test-support"))]
    fn course(&self, id: &str) -> io::Result<LandBattleCourseReply> {
        let episode = self
            .episodes
            .values()
            .find(|episode| episode.id == id)
            .ok_or_else(|| dashboard_core::invalid("Battle not in this recording branch"))?;
        let mut designs = Numbered::default();
        let battle = self.effective_episode(episode).course(&mut designs);
        bounded(LandBattleCourseReply {
            battle,
            designs: self.reply_designs(designs),
            content_warnings: self.content_warnings(),
        })
    }
}
fn bounded<T: Serialize>(reply: T) -> io::Result<T> {
    json_len(&reply, MAX_PRODUCT_REPLY_BYTES).ok_or_else(|| dashboard_core::invalid(format!(
        "Battle reply exceeds MAX_PRODUCT_REPLY_BYTES ({MAX_PRODUCT_REPLY_BYTES} bytes); no battles were omitted."
    )))?;
    Ok(reply)
}
#[cfg(any(test, feature = "test-support"))]
fn scan(
    replay: &Replay,
    branch: u64,
    cancelled: &AtomicBool,
    catalogue: Option<Arc<dashboard_text::catalogue::Catalogue>>,
) -> io::Result<Census> {
    let mut census = Census {
        format: replay.format,
        catalogue,
        ..Census::default()
    };
    dashboard_core::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &CENSUS_TABLES,
        cancelled,
        |blocks| {
            for (block, frames) in blocks {
                check_cancel(cancelled)?;
                let mut decoder = FrameDecoder::for_tables(&CENSUS_TABLES);
                let Ok(frames) = frames else {
                    census.gap(block.first_hour)?;
                    census.warn(block.first_hour, "Unread replay block; ending not recorded");
                    continue;
                };
                for frame in frames {
                    check_cancel(cancelled)?;
                    let Ok(decoded) = decoder.advance(&frame) else {
                        census.gap(frame.hour)?;
                        census.warn(frame.hour, "Unread replay frame; ending not recorded");
                        break;
                    };
                    validate_campaign(decoded, replay)?;
                    census.observe(decoded)?;
                }
            }
            Ok(())
        },
    )?;
    Ok(census)
}
#[cfg(any(test, feature = "test-support"))]
fn frame_available(format: replay_data::format::FormatVersion, frame: &DecodedFrame) -> bool {
    slot_read(format, &frame.metrics, TableId::LandCombats, frame.hour)
        && frame.complete::<tables::LandCombats>().is_ok()
        || slot_read(
            format,
            &frame.metrics,
            TableId::LandCombatResults,
            frame.hour,
        ) && frame
            .complete::<tables::LandCombatResults>()
            .is_ok_and(|rows| !rows.is_empty())
            && frame
                .complete::<tables::LandEventCoverage>()
                .is_ok_and(|rows| {
                    rows.iter()
                        .any(|row| row.complete && row.to_hour == frame.hour)
                })
}
#[cfg(any(test, feature = "test-support"))]
fn recording_available(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<bool> {
    let selected = [
        TableId::Campaign,
        TableId::LandCombats,
        TableId::LandCombatResults,
        TableId::LandEventCoverage,
    ];
    dashboard_core::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                check_cancel(cancelled)?;
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
                    continue;
                };
                for frame in frames {
                    check_cancel(cancelled)?;
                    let Ok(decoded) = decoder.advance(&frame) else {
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        break;
                    }
                    if frame_available(replay.format, decoded) {
                        return Ok(true);
                    }
                }
            }
            Ok(false)
        },
    )
}
fn branch_id(source: &Source, branch: &str) -> io::Result<u64> {
    let branch = branch
        .parse::<u64>()
        .map_err(|_| dashboard_core::invalid("Invalid battle branch"))?;
    if !source
        .replay_ref()
        .branches
        .iter()
        .any(|entry| entry.id == branch)
    {
        return Err(dashboard_core::invalid("Branch not in this recording"));
    }
    Ok(branch)
}
fn windows(replay: &Replay, branch: u64) -> Vec<BattleWindow> {
    replay
        .index
        .iter()
        .filter(|block| block.branch == branch)
        .map(|block| BattleWindow {
            first_hour: block.first_hour,
            last_hour: block.last_hour,
        })
        .collect()
}
#[derive(Default)]
#[cfg(any(test, feature = "test-support"))]
struct Cache {
    branches: VecDeque<(u64, Arc<Census>)>,
}
#[cfg(any(test, feature = "test-support"))]
impl Cache {
    fn get(
        &mut self,
        branch: u64,
        catalogue: Option<&Arc<dashboard_text::catalogue::Catalogue>>,
    ) -> Option<Arc<Census>> {
        let at = self.branches.iter().position(|(id, census)| {
            *id == branch
                && match (&census.catalogue, catalogue) {
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                    (None, None) => true,
                    _ => false,
                }
        })?;
        let entry = self.branches.remove(at).expect("cached branch");
        let census = Arc::clone(&entry.1);
        self.branches.push_back(entry);
        Some(census)
    }
    fn insert(&mut self, branch: u64, census: Arc<Census>) {
        self.branches.retain(|(id, _)| *id != branch);
        self.branches.push_back((branch, census));
        while self.branches.len() > 2 {
            self.branches.pop_front();
        }
    }
}
/// The catalogue land battles name equipment designs from, or why they cannot be named.
pub fn battle_content(
    source: &Source,
    context: &LoadContext,
) -> (
    Option<Arc<dashboard_text::catalogue::Catalogue>>,
    Option<String>,
) {
    match context.content.as_ref() {
        Some(options) => match dashboard_base::catalogue::catalogue(source, options) {
            Ok(catalogue) => (Some(catalogue), None),
            Err(reason) => (
                None,
                Some(format!("Land equipment names unavailable: {reason}")),
            ),
        },
        None => (
            None,
            Some("Game content unavailable; land equipment names are not resolved.".into()),
        ),
    }
}
#[cfg(any(test, feature = "test-support"))]
fn census(
    source: &Source,
    branch: u64,
    cancelled: &AtomicBool,
    context: &LoadContext,
) -> io::Result<Arc<Census>> {
    let (catalogue, warning) = battle_content(source, context);
    let state = source.state::<Cache>();
    let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(census) = cache.get(branch, catalogue.as_ref()) {
        return Ok(census);
    }
    let mut census = scan(source.replay_ref(), branch, cancelled, catalogue)?;
    census.content_warnings.extend(warning);
    let census = Arc::new(census);
    cache.insert(branch, Arc::clone(&census));
    Ok(census)
}
impl Product for LandBattles {
    const KEY: &'static str = "land-battles";
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
    const CONTENT_DEPENDENT: bool = true;
    type Params = LandBattlesParams;
    type Reply = LandBattlesReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid battle branch")?;
        Ok(())
    }
    fn types() -> Vec<String> {
        let mut types = battles::types();
        types.extend([
            LandBattlesParams::decl(),
            LandBattlePeak::decl(),
            LandBattleRun::decl(),
            LandManpower::decl(),
            LandLossTotal::decl(),
            LandLossTotals::decl(),
            LandBattleLossRead::decl(),
            LandBattleResultRead::decl(),
            LandBattleHistory::decl(),
            LandBattlesReply::decl(),
        ]);
        types
    }
    #[cfg(any(test, feature = "test-support"))]
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = branch_id(source, &params.branch_id)?;
        census(source, branch, cancelled, context)?
            .reply(params, &windows(source.replay_ref(), branch))
    }
}
impl Product for LandBattlesManifest {
    const KEY: &'static str = "land-battles-manifest";
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
    const CONTENT_DEPENDENT: bool = false;
    type Params = LandBattlesManifestParams;
    type Reply = LandBattlesManifestReply;
    fn types() -> Vec<String> {
        vec![
            LandBattlesManifestParams::decl(),
            LandBattlesManifestReply::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid battle branch")?;
        Ok(())
    }
    #[cfg(any(test, feature = "test-support"))]
    fn load(
        source: &Source,
        params: &Self::Params,
        _: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = branch_id(source, &params.branch_id)?;
        let available = recording_available(source.replay_ref(), branch, cancelled)?;
        bounded(LandBattlesManifestReply {
            windows: windows(source.replay_ref(), branch),
            available,
            unavailable: (!available).then(|| "Not in this recording".into()),
            content_warnings: Vec::new(),
        })
    }
}
impl Product for LandBattlesRanking {
    const KEY: &'static str = "land-battles-ranking";
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
    const CONTENT_DEPENDENT: bool = true;
    type Params = LandBattlesRankingParams;
    type Reply = LandBattlesRankingReply;
    fn types() -> Vec<String> {
        vec![
            LandBattleCountryFilter::decl(),
            LandBattlesRankingParams::decl(),
            LandBattlesRankingReply::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid battle branch")?;
        if params.first_hour > params.last_hour
            || !params.top_n.is_finite()
            || params.top_n < 1.0
            || params.top_n.fract() != 0.0
        {
            return Err("Invalid battle ranking range or top N");
        }
        if !["Casualties", "Peak size", "Length"].contains(&params.measure.as_str()) {
            return Err("Unknown battle measure");
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
        let branch = branch_id(source, &params.branch_id)?;
        check_cancel(cancelled)?;
        census(source, branch, cancelled, context)?.ranked(params, cancelled)
    }
}
impl Product for LandBattleCourses {
    const KEY: &'static str = "land-battle-course";
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
    const CONTENT_DEPENDENT: bool = true;
    type Params = LandBattleCourseParams;
    type Reply = LandBattleCourseReply;
    fn types() -> Vec<String> {
        vec![
            LandBattleCourseParams::decl(),
            LandLossSide::decl(),
            LandLossSides::decl(),
            LandBattleCourseRead::decl(),
            LandBattleCourse::decl(),
            LandEquipmentDesign::decl(),
            LandBattleCourseReply::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid battle branch")?;
        if params.id.is_empty() {
            return Err("Invalid battle id");
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
        let branch = branch_id(source, &params.branch_id)?;
        check_cancel(cancelled)?;
        census(source, branch, cancelled, context)?.course(&params.id)
    }
}
#[cfg(test)]
mod tests;
