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
//! LandCombats/LandCombatSides are hourly (replay::cadence); slot_read dates
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
use super::{
    LoadContext, MAX_PRODUCT_REPLY_BYTES, Product, ReplyWarnings,
    battles::{self, BattleBound, BattleEnd, BattleMoment, BattleWarning, BattleWindow},
};
use crate::{
    Source,
    metadata::{LinkTags, check_cancel, slot_read, validate_campaign},
    transport::json_len,
};
use replay::{
    container::Replay,
    frame::{DecodedFrame, FrameDecoder},
    rows::{TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    io,
    sync::{Arc, atomic::AtomicBool},
};
use ts_rs::TS;

mod losses;
mod terrain;
pub use terrain::LandBattleTerrains;
pub(crate) use terrain::check_content as check_terrain_content;

pub struct LandBattles;
pub struct LandBattlesManifest;
pub struct LandBattlesRanking;
/// One battle's whole course, for a drill.
pub struct LandBattleCourses;
const CENSUS_TABLES: [TableId; 13] = [
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
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LandBattlePeak {
    #[ts(type = "number")]
    pub value: u64,
    pub hour: i32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
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
#[derive(Clone, Debug, Serialize, TS)]
pub struct LandBattleLossRead {
    pub hour: i32,
    /// True only for the post-flush FinalizeCombat read, never inferred from absence.
    pub terminal: bool,
    /// Attacker then defender, or why neither side's counters were read.
    pub sides: LandLossTotals,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(untagged)]
pub enum LandLossTotals {
    Read([LandLossTotal; 2]),
    Unread(String),
}
/// One side's counters in the census: manpower and the exact signed fixed5 sum
/// of every design's equipment loss. Per-design amounts are in the battle's course.
#[derive(Clone, Debug, Serialize, TS)]
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
#[derive(Clone, Debug, Serialize, TS)]
pub struct LandBattleCourseRead {
    pub hour: i32,
    /// True only for the post-flush FinalizeCombat read, never inferred from absence.
    pub terminal: bool,
    /// Attacker then defender, or why neither side's counters were read.
    pub sides: LandLossSides,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(untagged)]
pub enum LandLossSides {
    Read([LandLossSide; 2]),
    Unread(String),
}
/// One side's counters. Variants pair a reply design index with the exact signed
/// fixed5 amount, in recorded order; the equipment total is their sum.
#[derive(Clone, Debug, Serialize, TS)]
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
#[derive(Clone, Debug, Serialize, TS)]
#[serde(untagged)]
pub enum LandManpower {
    Known(i32),
    Unknown(String),
}
#[derive(Clone, Debug, Serialize, TS)]
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
#[derive(Clone, Debug, Serialize, TS)]
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
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, TS)]
pub struct LandEquipmentDesign {
    /// The design's name from the content stack; null when it cannot be named.
    pub name: Option<String>,
    pub variant: String,
}
#[derive(Clone, Debug, Serialize, TS)]
pub struct LandBattleResultRead {
    pub hour: i32,
    pub duration: i32,
    pub winner: battles::BattleWinner,
    pub countries: [Vec<String>; 2],
    #[serde(rename = "countriesUnknown")]
    pub countries_unknown: [bool; 2],
}
#[derive(Clone, Debug, Serialize, TS)]
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
#[derive(Clone, Debug, Serialize, TS)]
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
    pub membership: Option<Vec<super::country_groups::MembershipRun>>,
    pub unobserved: Vec<super::country_groups::HourSpan>,
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
#[derive(Clone, Debug, Serialize, TS)]
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
#[derive(Clone, Debug, Serialize, TS)]
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
        self.peak_by(hour, |counts| u64::from(counts[0]) + u64::from(counts[1]))
    }
    fn side_peak_before(&self, hour: i32, side: usize) -> Option<LandBattlePeak> {
        self.peak_by(hour, |counts| u64::from(counts[side]))
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
#[derive(Default)]
struct Census {
    episodes: Vec<Episode>,
    open: HashMap<u64, usize>,
    links: LinkTags,
    observed: Vec<(i32, i32)>,
    warnings: BTreeMap<(i32, String), usize>,
    latest: HashMap<u64, usize>,
    variants: super::equipment::VariantCatalogue,
    catalogue: Option<Arc<crate::catalogue::Catalogue>>,
    content_warnings: Vec<String>,
    designs: losses::Designs,
}
impl Census {
    fn warn(&mut self, hour: i32, reason: impl Into<String>) {
        *self.warnings.entry((hour, reason.into())).or_default() += 1;
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
    fn gap(&mut self, hour: i32) {
        for (_, at) in self.open.drain() {
            self.episodes[at].lose_losses(hour);
            self.episodes[at].end = Some((
                hour,
                BattleEnd::Unknown {
                    unknown: "Ending not recorded".into(),
                },
            ));
        }
    }
    fn observe(&mut self, frame: &DecodedFrame) -> io::Result<()> {
        let hour = frame.hour;
        self.variants.observe(frame);
        let labels = self.links.update(frame).is_ok();
        let previous = self.observed.last().map(|(_, last)| *last);
        let adjacent = previous.and_then(|last| last.checked_add(1)) == Some(hour);
        if previous.is_some() && !adjacent {
            self.gap(
                previous
                    .and_then(|last| last.checked_add(1))
                    .unwrap_or(hour),
            );
        }
        if !slot_read(&frame.metrics, TableId::LandCombats, hour) {
            self.gap(hour);
            self.observe_results(frame, labels);
            return Ok(());
        }
        let Ok(rows) = frame.complete::<tables::LandCombats>() else {
            self.warn(
                hour,
                "LandCombats unavailable or incomplete; ending not recorded",
            );
            self.gap(hour);
            self.observe_results(frame, labels);
            return Ok(());
        };
        // The decoder refuses tables without unique keys, so ids are distinct.
        let ids: HashSet<u64> = rows.iter().map(|row| row.id).collect();
        if let Some((_, last)) = self
            .observed
            .last_mut()
            .filter(|(_, last)| last.checked_add(1) == Some(hour))
        {
            *last = hour;
        } else {
            self.observed.push((hour, hour));
        }
        let sides = slot_read(&frame.metrics, TableId::LandCombatSides, hour)
            .then(|| frame.complete::<tables::LandCombatSides>().ok())
            .flatten();
        let side_index: HashMap<u64, _> = sides
            .into_iter()
            .flatten()
            .map(|side| (side.combat, side))
            .collect();
        let absent: Vec<_> = self
            .open
            .keys()
            .filter(|id| !ids.contains(id))
            .copied()
            .collect();
        for id in absent {
            let at = self.open.remove(&id).expect("open battle");
            // A gap already closes open episodes as unknown; only adjacent absence reaches here.
            self.episodes[at].end = Some((
                hour,
                BattleEnd::Moment(BattleMoment {
                    hour,
                    bound: BattleBound::Exact,
                }),
            ));
        }
        for row in rows {
            let side = side_index.get(&row.id);
            let mut countries = [Vec::new(), Vec::new()];
            let mut countries_unknown = [!labels || side.is_none(); 2];
            if labels && let Some(side) = side {
                for (index, (primary, secondary)) in [
                    (&side.attacker_primary, &side.attacker_secondary),
                    (&side.defender_primary, &side.defender_secondary),
                ]
                .into_iter()
                .enumerate()
                {
                    countries_unknown[index] = primary.is_empty() && secondary.is_empty();
                    for raw in primary.iter().chain(secondary) {
                        match self.links.get(*raw) {
                            Some(tag) => countries[index].push(tag.clone()),
                            None => {
                                countries_unknown[index] = true;
                                self.warn(
                                    hour,
                                    format!("Participant country reference {raw} unresolved"),
                                );
                            }
                        }
                    }
                    countries[index].sort_unstable();
                    countries[index].dedup();
                }
            }
            if countries_unknown.contains(&true) {
                self.warn(
                    hour,
                    "Participant countries not completely recorded at supplying read",
                );
            }
            let distinct = |handles: &[u64]| -> io::Result<u32> {
                u32::try_from(handles.iter().collect::<HashSet<_>>().len())
                    .map_err(|_| io::Error::other("Recorded roster exceeds its native count"))
            };
            let counts = [distinct(&row.attackers)?, distinct(&row.defenders)?];
            let continuing = self.open.get(&row.id).copied().filter(|at| {
                let episode = &self.episodes[*at];
                episode.province == row.province
                    && episode
                        .runs
                        .last()
                        .is_none_or(|run| i64::from(row.duration) >= run.duration_at(run.last_hour))
            });
            let at = match continuing {
                Some(at) => at,
                None => {
                    if let Some(old) = self.open.remove(&row.id) {
                        self.episodes[old].end = Some((
                            hour,
                            BattleEnd::Moment(BattleMoment {
                                hour,
                                bound: BattleBound::Exact,
                            }),
                        ));
                    }
                    let at = self.episodes.len();
                    self.episodes.push(Episode {
                        id: format!("{}:{}:{hour}", frame.branch, row.id),
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
                    });
                    self.open.insert(row.id, at);
                    self.latest.insert(row.id, at);
                    at
                }
            };
            let runs = &mut self.episodes[at].runs;
            if let Some(run) = runs.last_mut().filter(|run| {
                run.last_hour.checked_add(1) == Some(hour)
                    && run.counts == counts
                    && run.countries == countries
                    && run.countries_unknown == countries_unknown
                    && run.duration_at(hour) == i64::from(row.duration)
            }) {
                run.last_hour = hour;
            } else {
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
        self.observe_losses(frame);
        self.observe_results(frame, labels);
        Ok(())
    }
    fn warnings_in(&self, first: i32, last: i32) -> (Vec<String>, Vec<(i32, usize, usize)>) {
        let mut reasons = Numbered::default();
        let warnings = self
            .warnings
            .iter()
            .filter(|((hour, _), _)| *hour >= first && *hour <= last)
            .map(|((hour, reason), count)| (*hour, reasons.index(reason.as_str()), *count))
            .collect();
        (
            reasons.into_vec().into_iter().map(String::from).collect(),
            warnings,
        )
    }
    fn reply(
        &self,
        params: &LandBattlesParams,
        windows: &[BattleWindow],
    ) -> io::Result<LandBattlesReply> {
        let window = windows
            .get(params.window)
            .ok_or_else(|| crate::invalid("Battle window not in this recording"))?;
        let mut rows = Vec::new();
        let observed = self
            .observed
            .iter()
            .filter(|(first, last)| *first <= window.last_hour && *last >= window.first_hour)
            .map(|(first, last)| {
                (
                    (*first).max(window.first_hour),
                    (*last).min(window.last_hour),
                )
            })
            .collect();
        for episode in &self.episodes {
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
        bounded(LandBattlesReply {
            window: params.window,
            interval: window.clone(),
            rows,
            observed,
            reasons,
            warnings,
            content_warnings: self.content_warnings(),
        })
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
        let mut ranked = Vec::new();
        for episode in &self.episodes {
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
            .observed
            .iter()
            .filter(|(first, _)| *first <= params.last_hour)
            .map(|(_, last)| (*last).min(params.last_hour))
            .max();
        bounded(LandBattlesRankingReply {
            rows,
            census_hour,
            effective_measure: effective.into(),
            content_warnings: self.content_warnings(),
            warnings: {
                let mut summary: BTreeMap<&str, (i32, usize)> = BTreeMap::new();
                for ((hour, reason), count) in &self.warnings {
                    if *hour >= params.first_hour && *hour <= params.last_hour {
                        let entry = summary.entry(reason).or_insert((*hour, 0));
                        entry.1 += count;
                    }
                }
                summary
                    .into_iter()
                    .map(|(reason, (hour, count))| BattleWarning {
                        hour,
                        reason: reason.into(),
                        count,
                    })
                    .collect()
            },
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
    fn course(&self, id: &str) -> io::Result<LandBattleCourseReply> {
        let episode = self
            .episodes
            .iter()
            .find(|episode| episode.id == id)
            .ok_or_else(|| crate::invalid("Battle not in this recording branch"))?;
        let mut designs = Numbered::default();
        let battle = episode.course(&mut designs);
        bounded(LandBattleCourseReply {
            battle,
            designs: self.reply_designs(designs),
            content_warnings: self.content_warnings(),
        })
    }
}
fn bounded<T: Serialize>(reply: T) -> io::Result<T> {
    json_len(&reply, MAX_PRODUCT_REPLY_BYTES).ok_or_else(|| crate::invalid(format!(
        "Battle reply exceeds MAX_PRODUCT_REPLY_BYTES ({MAX_PRODUCT_REPLY_BYTES} bytes); no battles were omitted."
    )))?;
    Ok(reply)
}
fn scan(
    replay: &Replay,
    branch: u64,
    cancelled: &AtomicBool,
    catalogue: Option<Arc<crate::catalogue::Catalogue>>,
) -> io::Result<Census> {
    let mut census = Census {
        catalogue,
        ..Census::default()
    };
    crate::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &CENSUS_TABLES,
        cancelled,
        |blocks| {
            for (block, frames) in blocks {
                check_cancel(cancelled)?;
                let mut decoder = FrameDecoder::for_tables(&CENSUS_TABLES);
                let Ok(frames) = frames else {
                    census.gap(block.first_hour);
                    census.warn(block.first_hour, "Unread replay block; ending not recorded");
                    continue;
                };
                for frame in frames {
                    check_cancel(cancelled)?;
                    let Ok(decoded) = decoder.advance(&frame) else {
                        census.gap(frame.hour);
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
fn recording_available(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<bool> {
    let selected = [
        TableId::Campaign,
        TableId::LandCombats,
        TableId::LandCombatResults,
        TableId::LandEventCoverage,
    ];
    crate::blocks::scan_blocks(
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
                    // A complete empty read also proves that this recording has the battle census.
                    if slot_read(&decoded.metrics, TableId::LandCombats, decoded.hour)
                        && decoded.complete::<tables::LandCombats>().is_ok()
                    {
                        return Ok(true);
                    }
                    if slot_read(&decoded.metrics, TableId::LandCombatResults, decoded.hour)
                        && decoded
                            .complete::<tables::LandCombatResults>()
                            .is_ok_and(|rows| !rows.is_empty())
                        && decoded
                            .complete::<tables::LandEventCoverage>()
                            .is_ok_and(|rows| {
                                rows.iter()
                                    .any(|row| row.complete && row.to_hour == decoded.hour)
                            })
                    {
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
        .map_err(|_| crate::invalid("Invalid battle branch"))?;
    if !source
        .replay_ref()
        .branches
        .iter()
        .any(|entry| entry.id == branch)
    {
        return Err(crate::invalid("Branch not in this recording"));
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
struct Cache {
    branches: VecDeque<(u64, Arc<Census>)>,
}
impl Cache {
    fn get(
        &mut self,
        branch: u64,
        catalogue: Option<&Arc<crate::catalogue::Catalogue>>,
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
fn census(
    source: &Source,
    branch: u64,
    cancelled: &AtomicBool,
    context: &LoadContext,
) -> io::Result<Arc<Census>> {
    let (catalogue, warning) = match context.content.as_ref() {
        Some(options) => match crate::catalogue::catalogue(source, options) {
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
    };
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
