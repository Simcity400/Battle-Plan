//! Live combats and retained results, joined only by NavalResultSources.
//! Persistent results are created separately; only transient strikes merge.
//! Never match by province, hour or members.

use dashboard_base::{
    products::{
        MAX_PRODUCT_REPLY_BYTES, Product, ReplyWarnings,
        battles::{
            self, BattleBound, BattleCountry, BattleEnd, BattleMoment, BattlePlace, BattleRow,
            BattleSide, BattleValue, BattleWarning, BattleWindow, BattleWinner,
        },
        country_groups::HourSpan,
    },
    source::Source,
};
use dashboard_core::{config::LoadContext, transport::json_len};
use dashboard_text::metadata::{LinkTags, check_cancel};
use replay::container::Replay;
use replay_data::{
    rows::NavalResultCountryRow, rows::NavalResultMemberRow, rows::NavalResultRow, rows::RowCodec,
    rows::TableId, rows::tables,
};
use replay_frame::{frame::DecodedFrame, frame::SharedView};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};
use ts_rs::TS;
#[cfg(any(test, feature = "test-support"))]
use {
    dashboard_text::metadata::validate_campaign, replay_frame::frame::FrameDecoder,
    std::collections::VecDeque,
};

pub mod columnar;
mod details;
pub mod prepared;
pub mod served;
pub use details::NavalBattleDetails;

pub struct NavalBattles;
pub struct NavalBattlesManifest;
pub struct NavalBattlesRanking;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NavalBattlesParams {
    pub branch_id: String,
    pub window: usize,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NavalBattlesManifestParams {
    pub branch_id: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NavalBattlesRankingParams {
    pub branch_id: String,
    pub first_hour: i32,
    pub last_hour: i32,
    pub top_n: f64,
    pub measure: String,
    pub countries: dashboard_land_battles::products::land_battles::LandBattleCountryFilter,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NavalBattleRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub row: BattleRow,
    pub peak_hour: Option<i32>,
    /// Ships sunk, Peak ships and Length; ended lengths use linked recorded reads.
    pub measures: Vec<BattleValue>,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NavalBattleHistory {
    pub id: String,
    pub province_id: u32,
    pub runs: Vec<NavalBattleRun>,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NavalBattlesReply {
    pub window: usize,
    pub interval: BattleWindow,
    pub rows: Vec<NavalBattleHistory>,
    pub observed: Vec<(i32, i32)>,
    pub warnings: Vec<BattleWarning>,
    pub content_warnings: Vec<String>,
}
impl ReplyWarnings for NavalBattlesReply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        &mut self.content_warnings
    }
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NavalBattlesManifestReply {
    pub windows: Vec<BattleWindow>,
    pub available: bool,
    pub unavailable: Option<String>,
    pub content_warnings: Vec<String>,
}
impl ReplyWarnings for NavalBattlesManifestReply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        &mut self.content_warnings
    }
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NavalBattlesRankingReply {
    pub rows: Vec<NavalBattleHistory>,
    pub census_hour: Option<i32>,
    pub effective_measure: String,
    pub warnings: Vec<BattleWarning>,
    pub content_warnings: Vec<String>,
}
impl ReplyWarnings for NavalBattlesRankingReply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        &mut self.content_warnings
    }
}
fn count(label: &str, value: Option<u64>, at_least: bool, reason: &str) -> BattleValue {
    match value {
        Some(value) => BattleValue::Known {
            label: label.into(),
            value: value as f64,
            at_least: at_least.then_some(true),
        },
        None => BattleValue::Unknown {
            label: label.into(),
            value: (),
            unknown: reason.into(),
        },
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
struct Quantities {
    ships: u64,
    convoys: u64,
    sunk: u64,
    convoys_sunk: u64,
}
impl Quantities {
    fn add(&mut self, convoy: bool, sunk: bool) {
        let total = if convoy {
            &mut self.convoys
        } else {
            &mut self.ships
        };
        *total += 1;
        if sunk {
            let lost = if convoy {
                &mut self.convoys_sunk
            } else {
                &mut self.sunk
            };
            *lost += 1;
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
struct SideSample {
    countries: Vec<String>,
    countries_unknown: Option<String>,
    quantities: Option<Quantities>,
    sunk: Option<u64>,
    convoys_sunk: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Snapshot {
    sides: [SideSample; 2],
    winner: Option<usize>,
    end: Option<i32>,
    peak: Option<u64>,
    peak_hour: Option<i32>,
    observed: Option<ObservedBattle>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct ObservedBattle {
    combat: u64,
    province: u32,
    first: i32,
    last: i32,
    peak: Option<u64>,
    peak_hour: Option<i32>,
    quantities: [Option<Quantities>; 2],
}
#[derive(Clone, Serialize, Deserialize)]
struct Run {
    first: i32,
    last: i32,
    sample: Snapshot,
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Kind {
    Combat,
    Result,
}
#[derive(Clone, Serialize, Deserialize)]
struct Episode {
    reference: u64,
    id: String,
    province: u32,
    first: i32,
    kind: Kind,
    resolved_read: Option<i32>,
    runs: Vec<Run>,
}

/// A row as a saved state keeps it: its key, and its fields as the wire encodes them.
#[derive(Serialize, Deserialize)]
struct SavedRow {
    key: [u64; 2],
    fields: Vec<u8>,
}

impl SavedRow {
    fn of<R: RowCodec>(row: &R) -> io::Result<Self> {
        let mut fields = Vec::new();
        row.encode_fields(&mut fields)?;
        let key = row.key();
        Ok(Self {
            key: [(key >> 64) as u64, key as u64],
            fields,
        })
    }

    fn row<R: RowCodec>(&self) -> io::Result<R> {
        let key = u128::from(self.key[0]) << 64 | u128::from(self.key[1]);
        R::decode_fields(key, &mut replay_data::codec::Decoder::new(&self.fields))
    }
}

fn saved_rows<R: RowCodec>(rows: &[R]) -> io::Result<Vec<SavedRow>> {
    rows.iter().map(SavedRow::of).collect()
}

fn loaded_rows<R: RowCodec>(rows: &[SavedRow]) -> io::Result<Vec<R>> {
    rows.iter().map(SavedRow::row).collect()
}

/// [`saved_rows`] as a field's serializer.
fn serialize_rows<R: RowCodec, S: serde::Serializer>(
    rows: &[R],
    out: S,
) -> Result<S::Ok, S::Error> {
    saved_rows(rows)
        .map_err(serde::ser::Error::custom)?
        .serialize(out)
}

/// [`loaded_rows`] as a field's deserializer.
fn deserialize_rows<'de, R: RowCodec, D: serde::Deserializer<'de>>(
    input: D,
) -> Result<Vec<R>, D::Error> {
    loaded_rows(&Vec::<SavedRow>::deserialize(input)?).map_err(serde::de::Error::custom)
}

#[derive(Clone, PartialEq)]
#[derive(replay_spool::Spooled)]
struct ResultInput {
    row: NavalResultRow,
    members: Option<Vec<NavalResultMemberRow>>,
    countries: Option<Vec<NavalResultCountryRow>>,
    classification: Option<[Vec<(u64, u8)>; 2]>,
}

/// A [`ResultInput`] as a saved state keeps it.
#[derive(Serialize, Deserialize)]
struct SavedInput {
    row: SavedRow,
    members: Option<Vec<SavedRow>>,
    countries: Option<Vec<SavedRow>>,
    classification: Option<[Vec<(u64, u8)>; 2]>,
}

impl Serialize for ResultInput {
    fn serialize<S: serde::Serializer>(&self, out: S) -> Result<S::Ok, S::Error> {
        let saved = (|| -> io::Result<SavedInput> {
            Ok(SavedInput {
                row: SavedRow::of(&self.row)?,
                members: self.members.as_deref().map(saved_rows).transpose()?,
                countries: self.countries.as_deref().map(saved_rows).transpose()?,
                classification: self.classification.clone(),
            })
        })()
        .map_err(serde::ser::Error::custom)?;
        saved.serialize(out)
    }
}

impl<'de> Deserialize<'de> for ResultInput {
    fn deserialize<D: serde::Deserializer<'de>>(input: D) -> Result<Self, D::Error> {
        let saved = SavedInput::deserialize(input)?;
        (|| -> io::Result<Self> {
            Ok(Self {
                row: saved.row.row()?,
                members: saved.members.as_deref().map(loaded_rows).transpose()?,
                countries: saved.countries.as_deref().map(loaded_rows).transpose()?,
                classification: saved.classification,
            })
        })()
        .map_err(serde::de::Error::custom)
    }
}

impl ResultInput {
    /// The bytes its rows hold on the heap.
    fn heap_bytes(&self) -> usize {
        self.members.as_ref().map_or(0, |rows| {
            rows.capacity() * size_of::<NavalResultMemberRow>()
        }) + self.countries.as_ref().map_or(0, |rows| {
            rows.capacity() * size_of::<NavalResultCountryRow>()
        }) + self.classification.as_ref().map_or(0, |sides| {
            sides
                .iter()
                .map(|side| side.capacity() * size_of::<(u64, u8)>())
                .sum()
        })
    }

    fn matches(
        &self,
        row: &NavalResultRow,
        members: Option<&[&NavalResultMemberRow]>,
        countries: Option<&[&NavalResultCountryRow]>,
        detail: Option<&replay_data::rows::NavalCombatResultDetailsRow>,
    ) -> bool {
        self.row == *row
            && match (&self.members, members) {
                (Some(old), Some(new)) => {
                    old.len() == new.len() && old.iter().zip(new).all(|(a, b)| a == *b)
                }
                (None, None) => true,
                _ => false,
            }
            && match (&self.countries, countries) {
                (Some(old), Some(new)) => {
                    old.len() == new.len() && old.iter().zip(new).all(|(a, b)| a == *b)
                }
                (None, None) => true,
                _ => false,
            }
            && match (&self.classification, detail) {
                (Some(old), Some(new)) => old.iter().zip(&new.sides).all(|(a, b)| {
                    a.len() == b.ships.len()
                        && a.iter()
                            .zip(&b.ships)
                            .all(|(a, b)| *a == (b.member, b.is_convoy))
                }),
                (None, None) => true,
                _ => false,
            }
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct Scan {
    episodes: Vec<Episode>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    results: HashMap<u64, usize>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    combats: HashMap<u64, usize>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    result_observations: HashMap<u64, Option<ObservedBattle>>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_set")]
    source_combats: HashSet<u64>,
    last_source: Option<(u64, i32)>,
    recorded_hours: Vec<HourSpan>,
    combat_hours: Vec<HourSpan>,
    result_hours: Vec<HourSpan>,
    tags: LinkTags,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    tag_identity: HashMap<u32, String>,
    links_known: bool,
    result_links_dirty: bool,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    result_inputs: HashMap<u64, ResultInput>,
    #[serde(skip)]
    result_views: Vec<SharedView>,
    result_through: Option<i32>,
    result_present: Vec<usize>,
    result_incomplete: usize,
    combat_incomplete: usize,
    #[serde(with = "dashboard_core::map_pairs")]
    refusals: BTreeMap<(i32, String), usize>,
    hour: i32,
    last_hour: Option<i32>,
    details: prepared::DetailPreparation,
    pages: BTreeMap<String, u64>,
    windows: dashboard_base::prepared::series::Accumulator,
    available: bool,
    dirty: BTreeSet<usize>,
    /// What the scan keeps of the hours it is given projected ([`columnar::Hour`]).
    projected: columnar::Carried,
}

fn span(spans: &mut Vec<HourSpan>, hour: i32) {
    if let Some(last) = spans
        .last_mut()
        .filter(|last| last.last_hour.checked_add(1) == Some(hour))
    {
        last.last_hour = hour;
    } else {
        spans.push(HourSpan {
            first_hour: hour,
            last_hour: hour,
        });
    }
}
fn grouped<'a, T: 'a>(
    rows: impl Iterator<Item = &'a T>,
    key: impl Fn(&T) -> u64,
) -> HashMap<u64, Vec<&'a T>> {
    let mut groups = HashMap::<u64, Vec<&T>>::new();
    for row in rows {
        groups.entry(key(row)).or_default().push(row);
    }
    groups
}
/// A side's countries: the tags `raws` name through `tags` when the links are known.
fn side_countries(
    tags: &LinkTags,
    links_known: bool,
    raws: impl Iterator<Item = u32>,
    complete: bool,
    side: &mut SideSample,
) {
    let mut named = BTreeSet::new();
    let mut known = links_known && complete;
    for raw in raws {
        if links_known
            && let Some(tag) = tags.get(raw)
            && !tag.contains('\0')
        {
            named.insert(tag.clone());
        } else {
            known = false;
        }
    }
    side.countries = named.into_iter().collect();
    side.countries_unknown = (!known).then(|| "Countries not recorded or unresolved".into());
}
/// Whether a sample lacks a side's quantities or countries.
fn incomplete(sides: &[SideSample; 2]) -> bool {
    sides
        .iter()
        .any(|s| s.quantities.is_none() || s.countries_unknown.is_some())
}
fn highest(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.max(b)),
        _ => a.or(b),
    }
}

impl Scan {
    fn run_last(&self, episode: &Episode, run: &Run) -> i32 {
        if episode.kind == Kind::Result
            && let Some(through) = self.result_through
            && episode
                .runs
                .last()
                .is_some_and(|last| std::ptr::eq(last, run))
            && self
                .results
                .get(&episode.reference)
                .is_some_and(|index| std::ptr::eq(&self.episodes[*index], episode))
            && self.result_inputs.contains_key(&episode.reference)
        {
            through
        } else {
            run.last
        }
    }
    fn extend_results(&mut self) {
        let Some(hour) = self.result_through.take() else {
            return;
        };
        for &index in &self.result_present {
            self.episodes[index]
                .runs
                .last_mut()
                .expect("deferred result read")
                .last = hour;
            self.dirty.insert(index);
        }
    }
    fn refuse(&mut self, reason: &str) {
        self.refuse_rows(reason, 1);
    }
    fn refuse_rows(&mut self, reason: &str, n: usize) {
        *self.refusals.entry((self.hour, reason.into())).or_default() += n;
    }
    fn countries(&self, raws: impl Iterator<Item = u32>, complete: bool, side: &mut SideSample) {
        side_countries(&self.tags, self.links_known, raws, complete, side);
    }
    fn push(&mut self, index: usize, hour: i32, sample: Snapshot) {
        self.dirty.insert(index);
        let episode = &mut self.episodes[index];
        if let Some(run) = episode.runs.last_mut()
            && run.last.checked_add(1) == Some(hour)
            && run.sample == sample
        {
            run.last = hour;
        } else {
            episode.runs.push(Run {
                first: hour,
                last: hour,
                sample,
            });
        }
    }
    /// A combat's sample at `hour` from its sides read then, on its episode's last sample.
    fn combat_sample(&self, index: usize, mut sides: [SideSample; 2], hour: i32) -> Snapshot {
        let current_peak = sides[0]
            .quantities
            .zip(sides[1].quantities)
            .map(|(a, b)| a.ships + b.ships);
        let previous = self.episodes[index].runs.last().map(|run| &run.sample);
        let peak = highest(previous.and_then(|p| p.peak), current_peak);
        let peak_hour = current_peak
            .filter(|peak| previous.and_then(|p| p.peak).is_none_or(|old| *peak > old))
            .map(|_| hour)
            .or_else(|| previous.and_then(|p| p.peak_hour));
        if let Some(previous) = previous {
            // Groups can be detached; no cumulative roster is promised.
            // Keep maxima as floors, never sum samples.
            for (side, old) in sides.iter_mut().zip(&previous.sides) {
                side.sunk = highest(side.sunk, old.sunk);
                side.convoys_sunk = highest(side.convoys_sunk, old.convoys_sunk);
            }
        }
        Snapshot {
            sides,
            winner: None,
            end: None,
            peak,
            peak_hour,
            observed: None,
        }
    }
    /// A result's sample from its recorded inputs.
    fn result_sample(&self, input: &ResultInput) -> Snapshot {
        let row = &input.row;
        let mut sides = [SideSample::default(), SideSample::default()];
        for (which, side) in sides.iter_mut().enumerate() {
            let classification: Option<HashMap<_, _>> = input
                .classification
                .as_ref()
                .map(|c| c[which].iter().copied().collect());
            let mut quantities = Quantities::default();
            side.quantities = input.members.as_ref().and_then(|members| {
                let classification = classification.as_ref()?;
                for member in members.iter().filter(|m| usize::from(m.side) == which) {
                    // Convoys also have nonzero references. The sunk_by vector supplies
                    // its own length; the presence test checks nonzero, not a ship count.
                    quantities.add(
                        *classification.get(&member.member)? != 0,
                        member.presence_value != 0,
                    );
                }
                Some(quantities)
            });
            side.sunk = side.quantities.map(|q| q.sunk);
            side.convoys_sunk = side.quantities.map(|q| q.convoys_sunk);
            // Lists 0/1 are sides, 2 is admitted/viewer.
            // Merging sides appends encounters without a leader rank.
            self.countries(
                input
                    .countries
                    .iter()
                    .flatten()
                    .filter(|r| usize::from(r.list) == which)
                    .map(|r| r.country),
                input.countries.is_some(),
                side,
            );
        }
        // Read the recorded winner selector, never recompute it.
        let winner = match row.winning_side {
            1 => Some(0),
            2 => Some(1),
            _ => None,
        };
        let observed = self
            .result_observations
            .get(&row.id)
            .cloned()
            .flatten()
            .filter(|observed| {
                observed.last <= row.result_hour
                    && observed.first <= row.result_hour
                    && observed.province == row.province
            });
        Snapshot {
            sides,
            winner,
            end: Some(row.result_hour),
            peak: None,
            peak_hour: None,
            observed,
        }
    }
    fn episode(&mut self, reference: u64, province: u32, kind: Kind, hour: i32) -> usize {
        let id = match kind {
            Kind::Combat => format!("combat:{reference}:{hour}"),
            Kind::Result if self.results.contains_key(&reference) => {
                format!("result:{reference}:{hour}")
            }
            Kind::Result => format!("result:{reference}"),
        };
        let index = self.episodes.len();
        self.episodes.push(Episode {
            reference,
            id,
            province,
            first: hour,
            kind,
            resolved_read: None,
            runs: vec![],
        });
        index
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn observe(&mut self, frame: &DecodedFrame) {
        self.observe_with_changes(frame, None);
    }
    pub fn observe_prepared(
        &mut self,
        frame: &DecodedFrame,
        changes: &dashboard_base::prepared::frame_changes::FrameChanges,
    ) {
        self.observe_with_changes(frame, Some(changes));
    }
    fn observe_with_changes(
        &mut self,
        frame: &DecodedFrame,
        changes: Option<&dashboard_base::prepared::frame_changes::FrameChanges>,
    ) {
        let cpu = ::replay_frame::cpu::scope_phase("naval:observe_apply");
        self.available |= frame_available(frame);
        let hour = frame.hour;
        self.hour = hour;
        span(&mut self.recorded_hours, hour);
        let previous_hour = self.last_hour;
        let contiguous = previous_hour.is_some_and(|last| last.checked_add(1) == Some(hour));
        if !contiguous {
            self.extend_results();
        }
        if self
            .last_hour
            .is_some_and(|last| last.checked_add(1) != Some(hour))
        {
            self.combats.clear();
        }
        self.last_hour = Some(hour);
        let previous_combats = self.combats.clone();
        let update = self.tags.update(frame);
        let links_known = update.is_ok();
        let identity_changed = matches!(update, Ok(true))
            && (self.tags.iter().count() != self.tag_identity.len()
                || self
                    .tags
                    .iter()
                    .any(|(raw, tag)| self.tag_identity.get(raw) != Some(tag)));
        let links_changed = links_known != self.links_known || identity_changed;
        self.result_links_dirty |= links_changed;
        if identity_changed {
            self.tag_identity = self
                .tags
                .iter()
                .map(|(raw, tag)| (*raw, tag.clone()))
                .collect();
        }
        self.links_known = links_known;

        drop(cpu);
        let cpu = ::replay_frame::cpu::scope_phase("naval:combat_projection");
        let unchanged_combat = contiguous
            && !links_changed
            && self
                .combat_hours
                .last()
                .is_some_and(|span| Some(span.last_hour) == previous_hour)
            && changes.is_some_and(|changes| {
                [
                    TableId::NavalCombats,
                    TableId::NavalCombatParticipants,
                    TableId::NavalCombatDetails,
                ]
                .iter()
                .all(|table| !changes.rows_changed[table.index()])
            });
        if unchanged_combat {
            span(&mut self.combat_hours, hour);
            // These samples are equal; only accepted coverage and Length advance.
            for &index in self.combats.values() {
                self.episodes[index]
                    .runs
                    .last_mut()
                    .expect("complete active census")
                    .last = hour;
                self.dirty.insert(index);
            }
            if self.combat_incomplete != 0 {
                self.refuse_rows(
                    "Active participant quantities or countries incomplete at read",
                    self.combat_incomplete,
                );
            }
        } else {
            // FrameDecoder validates unique table keys; the participant codec also
            // rejects side > 1 (replay_data::rows). These complete reads need no repeats.
            let participants = frame.complete::<tables::NavalCombatParticipants>().ok();
            let combat_details = frame.complete::<tables::NavalCombatDetails>().ok();
            let by_combat = grouped(participants.into_iter().flatten(), |row| row.combat);
            let combat_details: HashMap<_, _> = combat_details
                .into_iter()
                .flatten()
                .map(|row| (row.combat, row))
                .collect();
            if let Ok(rows) = frame.complete::<tables::NavalCombats>() {
                let present: HashSet<_> = rows.iter().map(|row| row.id).collect();
                span(&mut self.combat_hours, hour);
                // Ended combats have no display/measure consumer. Store no end;
                // only complete present reads can select a combat row.
                self.combats.retain(|id, _| present.contains(id));
                let mut missing = 0;
                for row in rows {
                    #[cfg(test)]
                    super::sinkings::work::WORK.with(|work| work.borrow_mut().combats += 1);
                    let index = self
                        .combats
                        .get(&row.id)
                        .copied()
                        .filter(|index| self.episodes[*index].province == row.province)
                        .unwrap_or_else(|| self.episode(row.id, row.province, Kind::Combat, hour));
                    self.combats.insert(row.id, index);
                    let mut sides = [SideSample::default(), SideSample::default()];
                    for (which, side) in sides.iter_mut().enumerate() {
                        let members: Vec<_> = by_combat
                            .get(&row.id)
                            .into_iter()
                            .flatten()
                            .filter(|row| usize::from(row.side) == which)
                            .copied()
                            .collect();
                        let mut values = Quantities::default();
                        // Combatant sides are 0 attacker / 1 defender. Sinking writes state 0x20
                        // without removing the member from its group; group cleanup skips
                        // terminal state 0x38.
                        for member in &members {
                            values.add(member.ship == 0, member.state & 0x20 != 0);
                        }
                        side.quantities = participants.map(|_| values);
                        side.sunk = side.quantities.map(|q| q.sunk);
                        side.convoys_sunk = side.quantities.map(|q| q.convoys_sunk);
                        let detail = combat_details
                            .get(&row.id)
                            .filter(|d| d.province == row.province);
                        // Both country lists are recorded.
                        self.countries(
                            members
                                .iter()
                                .map(|r| r.owner)
                                .chain(detail.into_iter().flat_map(|d| {
                                    d.sides[which]
                                        .summary
                                        .countries
                                        .iter()
                                        .chain(&d.sides[which].summary.secondary_countries)
                                        .copied()
                                })),
                            detail.is_some(),
                            side,
                        );
                    }
                    let sample = self.combat_sample(index, sides, hour);
                    missing += usize::from(incomplete(&sample.sides));
                    self.push(index, hour, sample);
                }
                self.combat_incomplete = missing;
                if missing != 0 {
                    self.refuse_rows(
                        "Active participant quantities or countries incomplete at read",
                        missing,
                    );
                }
            } else {
                self.refuse("Active combat census unavailable or incomplete");
                self.combats.clear();
            }
        }

        drop(cpu);
        let cpu = ::replay_frame::cpu::scope_phase("naval:settle_reconcile");
        if self.result_through.is_some()
            && !self
                .result_views
                .iter()
                .zip(RESULT_TABLES)
                .all(|(view, id)| SharedView::ptr_eq(view, &frame.tables[id.index()]))
        {
            self.extend_results();
        }
        self.observe_sources(frame, &previous_combats, previous_hour);
        drop(cpu);
        let _cpu = ::replay_frame::cpu::scope_phase("naval:results_projection");
        let Ok(rows) = frame.complete::<tables::NavalResults>() else {
            self.extend_results();
            self.refuse("Retained result census unavailable or incomplete");
            self.result_views.clear();
            return;
        };
        let links_changed = self.result_links_dirty;
        let unchanged = contiguous
            && !links_changed
            && self.result_views.len() == RESULT_TABLES.len()
            && self
                .result_views
                .iter()
                .zip(RESULT_TABLES)
                .all(|(view, id)| SharedView::ptr_eq(view, &frame.tables[id.index()]));
        if unchanged {
            span(&mut self.result_hours, hour);
            self.result_through = Some(hour);
            if self.result_incomplete != 0 {
                self.refuse_rows(
                    "Result member classification or countries incomplete at read",
                    self.result_incomplete,
                );
            }
            return;
        }
        self.extend_results();
        let ids: HashSet<_> = rows.iter().map(|row| row.id).collect();
        let invalid = rows
            .iter()
            .filter(|r| r.winning_side > 2 || r.losing_side > 2 || r.result_hour > hour)
            .count();
        if invalid != 0 {
            self.refuse_rows(
                "Invalid result selector or future date; read unavailable",
                invalid,
            );
            self.result_views.clear();
            return;
        }
        span(&mut self.result_hours, hour);
        // A country change during a failed census must still invalidate the
        // next successful result read, even if its raw result rows are equal.
        self.result_links_dirty = false;
        let views = RESULT_TABLES
            .iter()
            .map(|id| frame.tables[id.index()].clone())
            .collect::<Vec<_>>();
        self.result_views = views;
        let mut members = frame.complete::<tables::NavalResultMembers>().ok();
        if let Some(rows) = members {
            let invalid = rows.iter().filter(|r| r.side > 1).count();
            if invalid != 0 {
                self.refuse_rows("Invalid result member side; read unavailable", invalid);
            }
            if invalid != 0 {
                members = None;
            }
        }
        let mut countries = frame.complete::<tables::NavalResultCountries>().ok();
        if let Some(rows) = countries {
            let invalid = rows.iter().filter(|r| r.list > 2).count();
            if invalid != 0 {
                self.refuse_rows("Invalid result country list; read unavailable", invalid);
            }
            if invalid != 0 {
                countries = None;
            }
        }
        let mut details = frame.complete::<tables::NavalCombatResultDetails>().ok();
        if let Some(rows) = details {
            let invalid = rows
                .iter()
                .filter(|r| {
                    r.sides.iter().any(|side| {
                        let mut members = HashSet::new();
                        side.ships
                            .iter()
                            .any(|m| m.is_convoy > 1 || !members.insert(m.member))
                    })
                })
                .count();
            if invalid != 0 {
                self.refuse_rows(
                    "Invalid or duplicate result classification; read unavailable",
                    invalid,
                );
            }
            if invalid != 0 {
                details = None;
            }
        }
        let by_result = grouped(members.into_iter().flatten(), |row| row.result);
        let by_country = grouped(countries.into_iter().flatten(), |row| row.result);
        let details: HashMap<_, _> = details
            .into_iter()
            .flatten()
            .map(|row| (row.result, row))
            .collect();
        {
            let _cpu = ::replay_frame::cpu::scope_phase("naval:pruning");
            self.result_inputs.retain(|id, _| ids.contains(id));
        }
        let mut incomplete = 0;
        for row in rows {
            #[cfg(test)]
            super::sinkings::work::WORK.with(|work| work.borrow_mut().results += 1);
            let borrowed_members =
                members.map(|_| by_result.get(&row.id).map(Vec::as_slice).unwrap_or(&[]));
            let borrowed_countries =
                countries.map(|_| by_country.get(&row.id).map(Vec::as_slice).unwrap_or(&[]));
            let detail = details.get(&row.id).copied();
            let continuing = self
                .results
                .get(&row.id)
                .copied()
                .filter(|index| self.episodes[*index].province == row.province);
            let index = continuing.unwrap_or_else(|| {
                // A changed province is a new episode, never a permanent census refusal.
                let index = self.episode(row.id, row.province, Kind::Result, hour);
                self.results.insert(row.id, index);
                index
            });
            let input_changed = !self.result_inputs.get(&row.id).is_some_and(|input| {
                input.matches(row, borrowed_members, borrowed_countries, detail)
            });
            if continuing.is_some() && !links_changed && !input_changed {
                self.dirty.insert(index);
                let last = self.episodes[index].runs.last_mut().expect("result read");
                incomplete += usize::from(self::incomplete(&last.sample.sides));
                if last.last.checked_add(1) == Some(hour) {
                    last.last = hour;
                } else {
                    let sample = last.sample.clone();
                    self.push(index, hour, sample);
                }
                continue;
            }
            // Clone only after the borrowed input comparison detects a change.
            let new_input = input_changed.then(|| ResultInput {
                row: row.clone(),
                members: borrowed_members
                    .map(|rows| rows.iter().map(|row| (**row).clone()).collect()),
                countries: borrowed_countries
                    .map(|rows| rows.iter().map(|row| (**row).clone()).collect()),
                classification: detail.map(|d| {
                    std::array::from_fn(|which| {
                        d.sides[which]
                            .ships
                            .iter()
                            .map(|s| (s.member, s.is_convoy))
                            .collect()
                    })
                }),
            });
            let input = new_input
                .as_ref()
                .or_else(|| self.result_inputs.get(&row.id))
                .expect("result input");
            let sample = self.result_sample(input);
            incomplete += usize::from(self::incomplete(&sample.sides));
            self.push(index, hour, sample);
            if let Some(input) = new_input {
                self.result_inputs.insert(row.id, input);
            }
        }
        if incomplete != 0 {
            self.refuse_rows(
                "Result member classification or countries incomplete at read",
                incomplete,
            );
        }
        self.result_incomplete = incomplete;
        self.result_present = rows.iter().map(|row| self.results[&row.id]).collect();
    }

    fn observe_sources(
        &mut self,
        frame: &DecodedFrame,
        previous: &HashMap<u64, usize>,
        previous_hour: Option<i32>,
    ) {
        self.observe_source_rows(
            frame.complete::<tables::NavalResultSources>().ok(),
            frame.hour,
            previous,
            previous_hour,
        );
    }

    /// The hour's result sources, when complete. The results whose observation they set.
    fn observe_source_rows(
        &mut self,
        rows: Option<&[replay_data::rows::NavalResultSourceRow]>,
        hour: i32,
        previous: &HashMap<u64, usize>,
        previous_hour: Option<i32>,
    ) -> Vec<u64> {
        let mut observed = Vec::new();
        let Some(rows) = rows else {
            self.refuse("Naval result source interval not recorded or incomplete");
            return observed;
        };
        let mut ordered: Vec<_> = rows.iter().collect();
        ordered.sort_unstable_by_key(|row| row.id);
        for row in ordered {
            let valid = row.id != 0
                && self
                    .last_source
                    .is_none_or(|(id, hour)| row.id > id && row.game_hour >= hour)
                && row.game_hour <= hour
                && previous_hour.is_none_or(|hour| row.game_hour >= hour)
                && row
                    .combat_created_hour
                    .is_none_or(|hour| hour <= row.game_hour)
                && (!row.merged || row.transient);
            if !valid {
                self.result_observations.insert(row.result, None);
                observed.push(row.result);
                self.result_links_dirty = true;
                self.refuse("Invalid naval result source identity, hour or persistent merge");
                continue;
            }
            self.last_source = Some((row.id, row.game_hour));
            // Transient strikes use an invalid shared reference.
            // It cannot identify a recorded live combat.
            if row.transient {
                if row.combat_created_hour != Some(row.game_hour)
                    || self.combats.contains_key(&row.combat)
                    || previous.contains_key(&row.combat)
                {
                    self.refuse(
                        "Transient naval result source lacks a valid stack-local constructor",
                    );
                }
                continue;
            }
            if !self.source_combats.insert(row.combat)
                || self.result_observations.contains_key(&row.result)
                || self.results.contains_key(&row.result)
            {
                self.result_observations.insert(row.result, None);
                observed.push(row.result);
                self.result_views.clear();
                self.result_links_dirty = true;
                self.refuse("Duplicate persistent naval combat/result link");
                continue;
            }
            // The native full reference is the join. An adjacent complete read
            // identifies its episode; no gap or nearest-candidate search does.
            let index = self
                .combats
                .get(&row.combat)
                .or_else(|| previous.get(&row.combat))
                .copied();
            let observation = index.and_then(|index| {
                let episode = &self.episodes[index];
                let at = episode
                    .runs
                    .partition_point(|run| run.first <= row.game_hour);
                let run = at.checked_sub(1).map(|at| &episode.runs[at])?;
                if episode.first > row.game_hour
                    || row
                        .combat_created_hour
                        .is_some_and(|hour| hour > episode.first)
                {
                    return None;
                }
                Some(ObservedBattle {
                    combat: row.combat,
                    province: episode.province,
                    first: episode.first,
                    last: run.last.min(row.game_hour),
                    peak: run.sample.peak,
                    peak_hour: run.sample.peak_hour,
                    quantities: std::array::from_fn(|side| run.sample.sides[side].quantities),
                })
            });
            if let Some(index) = index {
                if observation.is_some() {
                    self.dirty.insert(index);
                    self.episodes[index].resolved_read = Some(hour);
                } else {
                    self.refuse("Naval source constructor does not match the recorded episode");
                }
            }
            self.result_observations.insert(row.result, observation);
            observed.push(row.result);
            self.result_views.clear();
            self.result_links_dirty = true;
        }
        observed
    }

    fn place(&self, episode: &Episode, run: &Run) -> BattlePlace {
        match episode.kind {
            Kind::Combat => BattlePlace {
                name: None,
                hour: episode.first,
                named_at: None,
            },
            Kind::Result => {
                if let Some(observed) = &run.sample.observed {
                    return BattlePlace {
                        name: None,
                        hour: observed.first,
                        named_at: None,
                    };
                }
                let hour = run.sample.end.expect("recorded result clock");
                let recorded = self
                    .recorded_hours
                    .iter()
                    .any(|span| span.first_hour <= hour && span.last_hour >= hour);
                let first_read = episode
                    .runs
                    .iter()
                    .find(|r| r.sample.end == Some(hour))
                    .expect("result supplying read")
                    .first;
                BattlePlace {
                    name: None,
                    hour,
                    named_at: (!recorded && first_read > hour).then_some(first_read),
                }
            }
        }
    }
    fn wire_run(&self, episode: &Episode, run: &Run, first: i32, last: i32) -> NavalBattleRun {
        let active = episode.kind == Kind::Combat;
        let reason = if active {
            "Participants not recorded"
        } else {
            "Result member classification not recorded"
        };
        let sunk = run.sample.sides[0]
            .sunk
            .zip(run.sample.sides[1].sunk)
            .map(|(a, b)| a + b);
        let observed = run.sample.observed.as_ref();
        let start = if active {
            Some(episode.first)
        } else {
            observed.map(|sample| sample.first)
        };
        let length = if active {
            start.map(|first| (i64::from(last) - i64::from(first)) as u64)
        } else {
            observed.map(|sample| (i64::from(sample.last) - i64::from(sample.first)) as u64)
        };
        NavalBattleRun {
            first_hour: first,
            last_hour: last,
            row: BattleRow {
                id: episode.id.clone(),
                place: self.place(episode, run),
                start: start.map(|hour| BattleMoment {
                    hour,
                    bound: BattleBound::By,
                }),
                end: run.sample.end.map(|hour| {
                    BattleEnd::Moment(BattleMoment {
                        hour,
                        bound: BattleBound::Exact,
                    })
                }),
                winner: run.sample.winner.map_or_else(
                    || BattleWinner::Unknown {
                        unknown: if active {
                            "Result not recorded"
                        } else {
                            "No winner recorded"
                        }
                        .into(),
                    },
                    |side| BattleWinner::Side { side },
                ),
                sides: run
                    .sample
                    .sides
                    .iter()
                    .enumerate()
                    .map(|(which, side)| {
                        let (quantities, roster) = if active {
                            (side.quantities, reason)
                        } else {
                            (
                                observed.and_then(|sample| sample.quantities[which]),
                                "Ship roster not recorded",
                            )
                        };
                        let counts = vec![
                            count("Ships", quantities.map(|q| q.ships), false, roster),
                            count("Convoys", quantities.map(|q| q.convoys), false, roster),
                        ];
                        BattleSide {
                            // Province merges may reverse alignment.
                            label: if active {
                                if which == 0 { "Attackers" } else { "Defenders" }
                            } else if which == 0 {
                                "Side 1"
                            } else {
                                "Side 2"
                            }
                            .into(),
                            // Retimed to min(playhead, last_hour) by the adapter.
                            countries: side
                                .countries
                                .iter()
                                .map(|tag| BattleCountry {
                                    tag: tag.clone(),
                                    hour: last,
                                })
                                .collect(),
                            countries_unknown: side.countries_unknown.clone(),
                            counts,
                            losses: vec![
                                count("Sunk", side.sunk, active, reason),
                                count("Convoys sunk", side.convoys_sunk, active, reason),
                            ],
                        }
                    })
                    .collect(),
            },
            peak_hour: observed.map_or(run.sample.peak_hour, |sample| sample.peak_hour),
            measures: vec![
                count("Ships sunk", sunk, active, reason),
                count(
                    "Peak ships",
                    observed.map_or(run.sample.peak, |sample| sample.peak),
                    true,
                    if active {
                        "Simultaneous participants not recorded"
                    } else {
                        "Not recorded"
                    },
                ),
                count("Length", length, true, "Not recorded"),
            ],
        }
    }
    fn observed_in(&self, first_hour: i32, last_hour: i32) -> Vec<(i32, i32)> {
        let mut observed = Vec::new();
        let (mut c, mut r) = (0, 0);
        while c < self.combat_hours.len() && r < self.result_hours.len() {
            let combat = &self.combat_hours[c];
            let result = &self.result_hours[r];
            let first = combat.first_hour.max(result.first_hour).max(first_hour);
            let last = combat.last_hour.min(result.last_hour).min(last_hour);
            if first <= last {
                observed.push((first, last));
            }
            if combat.last_hour <= result.last_hour {
                c += 1;
            } else {
                r += 1;
            }
        }
        observed
    }
    fn warnings_in(&self, first: i32, last: i32) -> Vec<BattleWarning> {
        self.refusals
            .iter()
            .filter(|((hour, _), _)| *hour >= first && *hour <= last)
            .map(|((hour, reason), count)| BattleWarning {
                hour: *hour,
                reason: reason.clone(),
                count: *count,
            })
            .collect()
    }
    fn history(&self, episode: &Episode, runs: Vec<NavalBattleRun>) -> NavalBattleHistory {
        NavalBattleHistory {
            id: episode.id.clone(),
            province_id: episode.province,
            runs,
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    fn reply(
        &self,
        params: &NavalBattlesParams,
        windows: &[BattleWindow],
    ) -> io::Result<NavalBattlesReply> {
        let window = windows
            .get(params.window)
            .ok_or_else(|| dashboard_core::invalid("Battle window not in this recording"))?;
        self.reply_window(params.window, window)
    }
    fn reply_window(&self, ordinal: usize, window: &BattleWindow) -> io::Result<NavalBattlesReply> {
        let mut rows = Vec::new();
        for episode in &self.episodes {
            let first = episode
                .runs
                .partition_point(|run| self.run_last(episode, run) < window.first_hour);
            let runs = episode.runs[first..]
                .iter()
                .take_while(|run| run.first <= window.last_hour)
                .filter_map(|run| {
                    let first = run.first.max(window.first_hour);
                    let last = self.run_last(episode, run).min(window.last_hour).min(
                        episode
                            .resolved_read
                            .and_then(|hour| hour.checked_sub(1))
                            .unwrap_or(i32::MAX),
                    );
                    (first <= last).then(|| self.wire_run(episode, run, first, last))
                })
                .collect::<Vec<_>>();
            if !runs.is_empty() {
                rows.push(self.history(episode, runs));
            }
        }
        bounded(NavalBattlesReply {
            window: ordinal,
            interval: window.clone(),
            rows,
            observed: self.observed_in(window.first_hour, window.last_hour),
            warnings: self.warnings_in(window.first_hour, window.last_hour),
            content_warnings: Vec::new(),
        })
    }
    fn ranked(
        &self,
        params: &NavalBattlesRankingParams,
        cancelled: &AtomicBool,
    ) -> io::Result<NavalBattlesRankingReply> {
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
        let matches =
            |sample: &Snapshot, read: i32| {
                if tags.is_none() && params.countries.membership.is_none() {
                    return true;
                }
                sample.sides.iter().any(|side| {
                    side.countries_unknown.is_some()
                        || side.countries.iter().any(|tag| {
                            tags.as_ref().is_none_or(|tags| tags.contains(tag.as_str()))
                                && (params.countries.membership.is_none()
                                    || !params.countries.unobserved.iter().any(|span| {
                                        span.first_hour <= read && span.last_hour >= read
                                    }) && membership.get(tag.as_str()).is_some_and(|runs| {
                                        runs.iter()
                                            .any(|(first, last)| *first <= read && *last >= read)
                                    }))
                        })
                })
            };
        let mut ranked = Vec::new();
        for episode in &self.episodes {
            check_cancel(cancelled)?;
            let at = episode
                .runs
                .partition_point(|run| run.first <= params.last_hour);
            let Some(run) = at.checked_sub(1).map(|index| &episode.runs[index]) else {
                continue;
            };
            // Only a complete present read can show a combat at the playhead.
            if episode.kind == Kind::Combat
                && (run.last < params.last_hour
                    || episode
                        .resolved_read
                        .is_some_and(|hour| hour <= params.last_hour))
            {
                continue;
            }
            let read = self.run_last(episode, run).min(params.last_hour);
            let in_range = match episode.kind {
                Kind::Combat => read >= params.first_hour,
                Kind::Result => run
                    .sample
                    .end
                    .is_some_and(|end| end >= params.first_hour && end <= params.last_hour),
            };
            if !in_range || !matches(&run.sample, read) {
                continue;
            }
            let measure = match params.measure.as_str() {
                "Ships sunk" => run.sample.sides[0]
                    .sunk
                    .zip(run.sample.sides[1].sunk)
                    .map(|(a, b)| a + b),
                "Peak ships" => run
                    .sample
                    .observed
                    .as_ref()
                    .map_or(run.sample.peak, |sample| sample.peak),
                // The counter is incremented and reset, so it cannot establish
                // an absolute original start.
                "Length" if episode.kind == Kind::Combat => {
                    Some((i64::from(read) - i64::from(episode.first)) as u64)
                }
                "Length" => run
                    .sample
                    .observed
                    .as_ref()
                    .map(|sample| (i64::from(sample.last) - i64::from(sample.first)) as u64),
                _ => None,
            };
            ranked.push((measure, episode, run, read));
        }
        ranked.sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
        // This is the user's top N, never a chosen cap; a huge f64 N saturates.
        let rows = ranked
            .into_iter()
            .take(params.top_n as usize)
            .map(|(_, episode, run, read)| {
                self.history(episode, vec![self.wire_run(episode, run, read, read)])
            })
            .collect();
        let census_hour = self
            .observed_in(i32::MIN, params.last_hour)
            .last()
            .map(|(_, last)| *last);
        let mut summary: BTreeMap<String, (i32, usize)> = BTreeMap::new();
        for warning in self.warnings_in(params.first_hour, params.last_hour) {
            let entry = summary.entry(warning.reason).or_insert((warning.hour, 0));
            entry.1 += warning.count;
        }
        bounded(NavalBattlesRankingReply {
            rows,
            census_hour,
            effective_measure: params.measure.clone(),
            warnings: summary
                .into_iter()
                .map(|(reason, (hour, count))| BattleWarning {
                    hour,
                    reason,
                    count,
                })
                .collect(),
            content_warnings: Vec::new(),
        })
    }
}

fn bounded<T: Serialize>(reply: T) -> io::Result<T> {
    json_len(&reply, MAX_PRODUCT_REPLY_BYTES).ok_or_else(|| dashboard_core::invalid(format!(
        "Naval battle reply exceeds MAX_PRODUCT_REPLY_BYTES ({MAX_PRODUCT_REPLY_BYTES} bytes); no battles were omitted."
    )))?;
    Ok(reply)
}

/// The tables prepared naval observation reads from a frame: the battle and
/// result census, the fleet rosters, the sunk ships and convoys, and the
/// tables sunk ships' place names read. Scripted place names also evaluate
/// the shared trigger kit's tables, which the dashboard adds.
pub const PREPARED_TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::NavalCombats,
    TableId::NavalCombatParticipants,
    TableId::NavalResults,
    TableId::NavalResultMembers,
    TableId::NavalResultCountries,
    TableId::NavalCombatDetails,
    TableId::NavalCombatResultDetails,
    TableId::NavalResultSources,
    TableId::TaskForces,
    TableId::Fleets,
    TableId::SunkShips,
    TableId::SunkConvoys,
    TableId::LexicalTokens,
    TableId::Provinces,
    TableId::ProvinceNames,
    TableId::States,
    TableId::ReservedCountryNaming,
    TableId::CountryAppearance,
];

const RESULT_TABLES: [TableId; 4] = [
    TableId::NavalResults,
    TableId::NavalResultMembers,
    TableId::NavalResultCountries,
    TableId::NavalCombatResultDetails,
];
const CENSUS_TABLES: [TableId; 11] = [
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::NavalCombats,
    TableId::NavalCombatParticipants,
    TableId::NavalResults,
    TableId::NavalResultMembers,
    TableId::NavalResultCountries,
    TableId::NavalCombatDetails,
    TableId::NavalCombatResultDetails,
    TableId::NavalResultSources,
];

#[cfg(any(test, feature = "test-support"))]
fn scan(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<Scan> {
    let mut scan = Scan::default();
    dashboard_core::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &CENSUS_TABLES,
        cancelled,
        |blocks| {
            for (block, frames) in blocks {
                check_cancel(cancelled)?;
                let frames = match frames {
                    Ok(frames) => frames,
                    Err(error) => {
                        scan.hour = block.first_hour;
                        scan.combats.clear();
                        scan.refuse(&format!("Naval battle block unavailable: {error}"));
                        continue;
                    }
                };
                let mut decoder = FrameDecoder::for_tables(&CENSUS_TABLES);
                for frame in frames {
                    check_cancel(cancelled)?;
                    let decoded = match decoder.advance(&frame) {
                        Ok(decoded) => decoded,
                        Err(error) => {
                            scan.hour = frame.hour;
                            scan.combats.clear();
                            scan.refuse(&format!("Naval battle baseline unavailable: {error}"));
                            break;
                        }
                    };
                    if let Err(error) = validate_campaign(decoded, replay) {
                        scan.hour = frame.hour;
                        scan.combats.clear();
                        scan.refuse(&format!("Naval battle campaign refused: {error}"));
                        break;
                    }
                    scan.observe(decoded);
                }
            }
            Ok(())
        },
    )?;
    scan.extend_results();
    Ok(scan)
}

fn frame_available(frame: &DecodedFrame) -> bool {
    frame.complete::<tables::NavalCombats>().is_ok()
        || frame.complete::<tables::NavalResults>().is_ok()
}
#[cfg(any(test, feature = "test-support"))]
fn recording_available(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<bool> {
    let selected = [
        TableId::Campaign,
        TableId::NavalCombats,
        TableId::NavalResults,
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
                    // Empty complete censuses also prove availability. Stop at the first.
                    if frame_available(decoded) {
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
        .parse()
        .map_err(|_| dashboard_core::invalid("Invalid battle branch"))?;
    if !source.replay_ref().branches.iter().any(|b| b.id == branch) {
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
    branches: VecDeque<(u64, Arc<Scan>)>,
}
#[cfg(any(test, feature = "test-support"))]
impl Cache {
    fn get(&mut self, branch: u64) -> Option<Arc<Scan>> {
        let at = self.branches.iter().position(|(id, _)| *id == branch)?;
        let entry = self.branches.remove(at).expect("cached branch");
        let scan = Arc::clone(&entry.1);
        self.branches.push_back(entry);
        Some(scan)
    }
    fn insert(&mut self, branch: u64, scan: Arc<Scan>) {
        self.branches.retain(|(id, _)| *id != branch);
        self.branches.push_back((branch, scan));
        while self.branches.len() > 2 {
            self.branches.pop_front();
        }
    }
}
#[cfg(any(test, feature = "test-support"))]
fn census(source: &Source, branch: u64, cancelled: &AtomicBool) -> io::Result<Arc<Scan>> {
    let state = source.state::<Cache>();
    let mut cache = state.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(scan) = cache.get(branch) {
        return Ok(scan);
    }
    let scan = Arc::new(scan(source.replay_ref(), branch, cancelled)?);
    cache.insert(branch, Arc::clone(&scan));
    Ok(scan)
}
impl Product for NavalBattles {
    const KEY: &'static str = "naval-battles";
    const CONTENT_DEPENDENT: bool = false;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
    type Params = NavalBattlesParams;
    type Reply = NavalBattlesReply;
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
            NavalBattlesParams::decl(),
            NavalBattleRun::decl(),
            NavalBattleHistory::decl(),
            NavalBattlesReply::decl(),
        ]);
        types
    }
    #[cfg(any(test, feature = "test-support"))]
    fn load(
        source: &Source,
        params: &Self::Params,
        _: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = branch_id(source, &params.branch_id)?;
        census(source, branch, cancelled)?.reply(params, &windows(source.replay_ref(), branch))
    }
}
impl Product for NavalBattlesManifest {
    const KEY: &'static str = "naval-battles-manifest";
    const CONTENT_DEPENDENT: bool = false;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
    type Params = NavalBattlesManifestParams;
    type Reply = NavalBattlesManifestReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid battle branch")?;
        Ok(())
    }
    fn types() -> Vec<String> {
        vec![
            NavalBattlesManifestParams::decl(),
            NavalBattlesManifestReply::decl(),
        ]
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
        bounded(NavalBattlesManifestReply {
            windows: windows(source.replay_ref(), branch),
            available,
            unavailable: (!available).then(|| "Not in this recording".into()),
            content_warnings: Vec::new(),
        })
    }
}
impl Product for NavalBattlesRanking {
    const KEY: &'static str = "naval-battles-ranking";
    const CONTENT_DEPENDENT: bool = false;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
    type Params = NavalBattlesRankingParams;
    type Reply = NavalBattlesRankingReply;
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
        if !["Ships sunk", "Peak ships", "Length"].contains(&params.measure.as_str()) {
            return Err("Unknown battle measure");
        }
        Ok(())
    }
    fn types() -> Vec<String> {
        vec![
            NavalBattlesRankingParams::decl(),
            NavalBattlesRankingReply::decl(),
        ]
    }
    #[cfg(any(test, feature = "test-support"))]
    fn load(
        source: &Source,
        params: &Self::Params,
        _: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = branch_id(source, &params.branch_id)?;
        check_cancel(cancelled)?;
        census(source, branch, cancelled)?.ranked(params, cancelled)
    }
}

impl Scan {
    pub fn state_census(
        &self,
        line: dashboard_base::prepared::state_census::Line,
    ) -> dashboard_base::prepared::state_census::Line {
        let (window_keys, window_active) = self.windows.census_keys();
        line.field("naval_episodes", self.episodes.len())
            .json("naval_episodes_json", &self.episodes)
            .field("naval_results", self.results.len())
            .field("naval_combats", self.combats.len())
            .field("naval_result_observations", self.result_observations.len())
            .field("naval_source_combats", self.source_combats.len())
            .field("naval_recorded_hours", self.recorded_hours.len())
            .field("naval_combat_hours", self.combat_hours.len())
            .field("naval_result_hours", self.result_hours.len())
            .field("naval_tag_identity", self.tag_identity.len())
            .field("naval_refusals", self.refusals.len())
            .json("naval_details_json", &self.details)
            .field("naval_pages", self.pages.len())
            .field("naval_window_keys", window_keys)
            .field("naval_window_active", window_active)
            .json("naval_windows_json", &self.windows)
    }
}

#[cfg(test)]
mod tests;
