//! Land army totals and the Army size race. Country attribution is shared with
//! Scoreboard through the prepared army series and the native per-frame step.
//! Missing reads and country identities end runs; they never produce zeros.
//!
//! An expeditionary transfer retains the source owner while the target receives
//! the roster/logical links. Owner therefore means positive signed expeditionary
//! owner, otherwise roster owner; host is roster. The hourly census uses that
//! same owner rule, adding grouped and ungrouped divisions. Manpower wraps the
//! signed i32 contributor sum per division, then sums countries exactly.
//! Contributor origins never replace the division owner/host.
//! Combined strength is current HP, not a percentage or template maximum:
//! the game clamps it to 0..maximum and formats it as fixed5.
//!
//! The counter representative uses signed wrapping priority * count, selecting
//! the largest positive score and the first tie, with brigade (0,0)/null fallback.
//! The AI representative instead uses ai_priority * count. Division categories
//! use that AI representative, counter fallback, a template override and category
//! predicates. These differ from the battalion view: grouped template sub-units
//! (regiments and support companies) use their authored content group, never
//! either representative. No per-battalion HP is recorded, so HP stays unsplit.

use dashboard_base::prepared::frame_changes::{FrameChanges, field_mask};
use dashboard_base::{
    products::{
        Product,
        country_groups::HourSpan,
        country_history::{Identity, observe_identity},
    },
    source::Source,
};
use dashboard_core::{config::LoadContext, protocol::validate_tag};
use dashboard_text::metadata::{
    IdentityTrack, TaggedCountry, link_slot, slot_read, tagged_countries,
};
use replay::container::Replay;
use replay_data::{
    rows::CountryLinkRow, rows::DeltaRow, rows::DivisionCensusRow, rows::DivisionRow,
    rows::TableId, rows::tables,
};
use replay_frame::{frame::DecodedFrame, frame::TableUpdate};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::{Arc, LazyLock, atomic::AtomicBool},
};
use ts_rs::TS;
#[cfg(any(test, feature = "test-support"))]
use {
    dashboard_text::metadata::validate_campaign, replay_frame::frame::FrameDecoder,
    std::collections::VecDeque,
};

pub struct ArmySize;
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum ArmyMeasure {
    Manpower,
    Divisions,
    Strength,
    Battalions,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum ArmyAttribution {
    Owner,
    Host,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArmySizeParams {
    // Reached country and hour for plain template/configuration sections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub template_at: Option<(String, i32)>,
    pub branch_id: String,
    pub tags: Vec<String>,
    pub top: Option<usize>,
    pub measure: ArmyMeasure,
    pub count_for: ArmyAttribution,
    pub by_type: bool,
    /// None is the capability/window manifest; histories use a replay-block index.
    #[serde(default)]
    pub window: Option<usize>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ArmyType {
    pub key: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ArmyRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub total: Option<f64>,
    pub raw: String,
}

// Compact hourly totals: [first hour, geometry total, last hour or null, exact raw].
impl Serialize for ArmyRun {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        (
            self.first_hour,
            self.total,
            (self.last_hour != self.first_hour).then_some(self.last_hour),
            &self.raw,
        )
            .serialize(serializer)
    }
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ArmyMixRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub values: Vec<Option<f64>>,
    /// Decimal counts retain exact labels; values are chart geometry only.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub raw_values: Option<Vec<Option<String>>>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ArmySeries {
    pub tag: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub identities: Option<Vec<dashboard_base::products::country_history::CountryIdentityRun>>,
    #[ts(type = "Array<ArmyRun>")]
    pub runs: Vec<ArmyRun>,
    pub segments: Vec<HourSpan>,
    pub membership: Option<Vec<HourSpan>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub mix: Option<Vec<ArmyMixRun>>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ArmySizeReply {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub template_sections: Option<Vec<String>>,
    pub branch_id: String,
    pub windows: Vec<HourSpan>,
    pub window: Option<usize>,
    pub interval: Option<HourSpan>,
    pub available: bool,
    pub measures: Vec<ArmyMeasure>,
    pub types: Vec<ArmyType>,
    pub series: Vec<ArmySeries>,
    pub warnings: Vec<String>,
}

/// Value and supplying read for an hour. None from `army_value_at` is unknown or
/// absent, never zero. Strength is current hit points, in ordinary HP units.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg(test)]
pub struct ArmyValue {
    pub value: f64,
    pub raw: i128,
    pub read_hour: i32,
}

pub fn channel(measure: ArmyMeasure, attribution: ArmyAttribution) -> usize {
    match (measure, attribution) {
        (ArmyMeasure::Divisions, ArmyAttribution::Owner) => 0,
        (ArmyMeasure::Strength, ArmyAttribution::Owner) => 1,
        (ArmyMeasure::Divisions, ArmyAttribution::Host) => 2,
        (ArmyMeasure::Strength, ArmyAttribution::Host) => 3,
        (ArmyMeasure::Battalions, ArmyAttribution::Owner) => 4,
        (ArmyMeasure::Battalions, ArmyAttribution::Host) => 5,
        (ArmyMeasure::Manpower, ArmyAttribution::Owner) => 6,
        (ArmyMeasure::Manpower, ArmyAttribution::Host) => 7,
    }
}

fn amount(raw: Option<i128>, measure: ArmyMeasure) -> Option<f64> {
    raw.map(|raw| {
        raw as f64
            / if measure == ArmyMeasure::Strength {
                100_000.0
            } else {
                1.0
            }
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Totals {
    // Owner/host divisions, HP, battalions, then native division manpower totals.
    values: [Option<i128>; 8],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RawRun {
    first: i32,
    last: i32,
    starts: bool,
    totals: Totals,
}

#[derive(Default, Serialize, Deserialize)]
struct Track {
    runs: Vec<RawRun>,
    last: Option<(Identity, i32)>,
    segments: Vec<HourSpan>,
    last_identity: IdentityTrack,
    mix: Vec<RawMixRun>,
    /// The composition version of the in-memory inventory that `mix`'s last
    /// run was read at. It names that inventory's counters, which a restored
    /// observer starts afresh, so it is never saved: a restored track compares
    /// its first composition by parts.
    #[serde(skip)]
    mix_version: Option<(u64, u64, [bool; 2])>,
}
impl Track {
    fn advance(&mut self, hour: i32) {
        let Some(segment) = self.segments.last_mut() else {
            return;
        };
        let before = segment.last_hour;
        segment.last_hour = hour;
        self.last_identity.advance(hour);
        if let Some((_, at)) = &mut self.last
            && *at == before
        {
            *at = hour;
            if let Some(run) = self.runs.last_mut() {
                run.last = hour;
            }
            if let Some(mix) = self.mix.last_mut()
                && mix.last == before
            {
                mix.last = hour;
            }
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Scan {
    tracks: BTreeMap<String, Track>,
    divisions: bool,
    strength: bool,
    manpower: bool,
    inventory_fallback: bool,
    unreadable: usize,
    battalions: bool,
    composition_unreadable: usize,
    invalid_table_reads: BTreeMap<String, usize>,
    frontier: Option<i32>,
    live: std::collections::BTreeSet<String>,
    #[serde(skip)]
    removed: Vec<String>,
    /// The identities observed at the last hour, by their index among the
    /// scan state's identities.
    #[serde(skip)]
    observed: Vec<usize>,
}
impl Scan {
    fn seal(&mut self) {
        if let Some(hour) = self.frontier {
            for tag in &self.live {
                self.tracks
                    .get_mut(tag)
                    .expect("live army holder")
                    .advance(hour);
            }
        }
    }
}

/// The Divisions fields the totals and composition read; the id is the key.
static DIVISION_FIELDS: LazyLock<u64> = LazyLock::new(|| {
    field_mask(
        TableId::Divisions,
        &[
            "logical_owner",
            "roster_owner",
            "expeditionary_owner",
            "template",
            "hit_points",
        ],
    )
});

/// The DivisionTemplateIcons fields a composition reads; the template is the key.
static TEMPLATE_FIELDS: LazyLock<u64> = LazyLock::new(|| {
    field_mask(
        TableId::DivisionTemplateIcons,
        &["subunits", "model_unreadable"],
    )
});

/// The DivisionLandInputs fields the manpower total reads; the division is the key.
static LAND_FIELDS: LazyLock<u64> =
    LazyLock::new(|| field_mask(TableId::DivisionLandInputs, &["manpower"]));

/// The Countries fields a country's identity is read from; its id is the key.
static HOLDER_FIELDS: LazyLock<u64> =
    LazyLock::new(|| field_mask(TableId::Countries, &["canonical_slot", "tag"]));

/// Census owner rule: positive signed expeditionary owner, otherwise roster owner.
pub fn owner(row: &DivisionRow) -> u32 {
    replay_data::military_aid::owner(row)
}

/// The raw country a division counts for under the chosen attribution: its
/// census owner, or the roster that hosts it.
pub fn attributed(row: &DivisionRow, attribution: ArmyAttribution) -> u32 {
    match attribution {
        ArmyAttribution::Owner => owner(row),
        ArmyAttribution::Host => row.roster_owner,
    }
}

fn add(target: &mut Option<i64>, value: i64) {
    *target = target.and_then(|current| current.checked_add(value));
}

/// Detailed tables use native sparse deltas; only country/census tables need
/// full decoder reconstruction. Sparse tables validate keys and native deltas
/// before updating their own baselines; failed reads stay gaps.
#[cfg(any(test, feature = "test-support"))]
fn scan_tables(detailed: bool) -> Vec<TableId> {
    let mut tables = vec![
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::DivisionCensus,
    ];
    if detailed {
        tables.push(TableId::LexicalTokens);
    }
    tables
}

#[cfg(any(test, feature = "test-support"))]
fn scan(replay: &Replay, branch: u64, detailed: bool, cancelled: &AtomicBool) -> io::Result<Scan> {
    let selected = scan_tables(detailed);
    let mut wire_selected = selected.clone();
    wire_selected.extend([
        TableId::Divisions,
        TableId::DivisionTemplateIcons,
        TableId::DivisionLandInputs,
    ]);
    let mut result = Scan::default();
    let mut state = ScanState::new(detailed);
    dashboard_core::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &wire_selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                dashboard_text::metadata::check_cancel(cancelled)?;
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
                    continue;
                };
                for frame in frames {
                    dashboard_text::metadata::check_cancel(cancelled)?;
                    let Ok(decoded) = decoder.advance(&frame) else {
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        break;
                    }
                    state.step(&mut result, &frame, decoded, replay);
                }
            }
            Ok(())
        },
    )?;
    Ok(result)
}

#[derive(Default)]
struct ScanState {
    inventory: Option<Inventory>,
    identities: Vec<(String, Identity)>,
    countries: Option<replay_frame::frame::ViewIdentity>,
    links: Option<replay_frame::frame::ViewIdentity>,
    unreadable_tags: usize,
    keys: Option<dashboard_text::metadata::CountryKeys>,
    /// How many Countries rows `keys` were read from.
    country_rows: Option<usize>,
    identity_changed: bool,
    applied: Option<i32>,
    reads: Option<[bool; 4]>,
    rates: (usize, usize, BTreeMap<String, usize>),
    slots: HashMap<u32, Vec<usize>>,
    census_counts: HashMap<u32, Option<i64>>,
    census_invalid: bool,
    census_unreadable: usize,
    census_dirty: std::collections::BTreeSet<u32>,
    census_ready: Option<bool>,
    /// Each census row's slot and division count, by country.
    census_rows: HashMap<u32, (Option<u32>, i64)>,
    /// Each slot's census rows and their summed division count.
    census_sums: HashMap<u32, (usize, i128)>,
    /// The hour whose census `census_rows` hold, while they hold one.
    census_hour: Option<i32>,
    restriction: Option<bool>,
    composition_rates: HashMap<String, usize>,
    composition_rate: usize,

    #[cfg(test)]
    work: dashboard_base::prepared::observer_tests::Work,
}

impl ScanState {
    #[cfg(any(test, feature = "test-support"))]
    fn step(
        &mut self,
        result: &mut Scan,
        frame: &(impl replay_frame::frame::FrameSource + ?Sized),
        decoded: &DecodedFrame,
        replay: &Replay,
    ) {
        self.step_changes(result, frame, decoded, replay, None);
        result.seal();
    }
    #[cfg(any(test, feature = "test-support"))]
    fn step_changes(
        &mut self,
        result: &mut Scan,
        frame: &(impl replay_frame::frame::FrameSource + ?Sized),
        decoded: &DecodedFrame,
        replay: &Replay,
        changes: Option<&FrameChanges>,
    ) -> bool {
        self.step_updates(result, Updates::recorded(frame), decoded, replay, changes)
    }
    /// Observe one hour whose inventory tables reached it by `updates`.
    /// False when nothing the army reads changed since the previous hour.
    fn step_updates(
        &mut self,
        result: &mut Scan,
        updates: Updates<'_>,
        decoded: &DecodedFrame,
        replay: &Replay,
        changes: Option<&FrameChanges>,
    ) -> bool {
        result.observed.clear();
        result.removed.clear();
        let continuous = result
            .frontier
            .is_some_and(|at| at.checked_add(1) == Some(decoded.hour));
        // Recorded changes to no country's key, slot or tag keep the
        // identities read at the previous hour.
        let kept = continuous
            && changes
                .and_then(|changes| changes.rows(TableId::Countries))
                .is_some_and(|(changed, deleted)| {
                    deleted.is_empty() && changed.iter().all(|row| row.mask & *HOLDER_FIELDS == 0)
                });
        let valid = self.identities_kept(decoded, kept);
        let identity_changed = self.identity_changed;
        let reads = [
            TableId::DivisionCensus,
            TableId::Divisions,
            TableId::DivisionTemplateIcons,
            TableId::DivisionLandInputs,
        ]
        .map(|id| slot_read(replay.format, &decoded.metrics, id, decoded.hour));
        let unchanged = continuous
            && valid
            && !self.identity_changed
            && self.reads == Some(reads)
            && changes.is_some_and(|changes| {
                [
                    TableId::DivisionCensus,
                    TableId::Divisions,
                    TableId::DivisionTemplateIcons,
                    TableId::DivisionLandInputs,
                    TableId::LexicalTokens,
                ]
                .iter()
                .all(|id| !changes.rows_changed[id.index()])
            });
        if unchanged {
            result.unreadable += self.rates.0;
            result.composition_unreadable += self.rates.1;
            for (key, &count) in &self.rates.2 {
                *result.invalid_table_reads.entry(key.clone()).or_default() += count;
            }
            // The census held is still this hour's.
            if self
                .census_hour
                .is_some_and(|at| at.checked_add(1) == Some(decoded.hour))
            {
                self.census_hour = Some(decoded.hour);
            }
            result.frontier = Some(decoded.hour);
            return false;
        }
        if !continuous || identity_changed {
            result.seal();
        }
        let before_inventory = self.inventory.as_ref().map(|inventory| {
            (
                inventory.known(),
                inventory.composition_known(),
                inventory.unreadable() != 0,
            )
        });
        let unreadable = result.unreadable;
        let composition = result.composition_unreadable;
        self.apply_updates(updates, decoded);
        self.compose(decoded);
        let gap = replay.gaps.iter().any(|gap| {
            gap.branch == decoded.branch
                && gap.first_hour <= decoded.hour
                && decoded.hour <= gap.last_hour
        });
        if identity_changed {
            let present: std::collections::BTreeSet<_> = self
                .identities
                .iter()
                .map(|(tag, _)| tag.as_str())
                .collect();
            self.composition_rates
                .retain(|tag, _| present.contains(tag.as_str()));
            self.composition_rate = self.composition_rates.values().sum();
        }
        #[cfg(test)]
        {
            self.work.row_visits += updates.row_visits(decoded);
        }
        let after_inventory = self.inventory.as_ref().map(|inventory| {
            (
                inventory.known(),
                inventory.composition_known(),
                inventory.unreadable() != 0,
            )
        });
        self.restriction = Some(
            changes.is_some()
                && continuous
                && !identity_changed
                && self.reads == Some(reads)
                && before_inventory == after_inventory,
        );
        observe_updates(result, decoded, updates, replay.format, gap, self, changes);
        // The observer's cached identity reread must not clear this hour's change.
        self.identity_changed = identity_changed;
        self.rates = (
            result.unreadable - unreadable,
            result.composition_unreadable - composition,
            self.inventory
                .as_ref()
                .into_iter()
                .flat_map(|inventory| inventory.errors())
                .map(|(table, reason)| (format!("{table:?}: {reason}"), 1))
                .collect(),
        );
        result.frontier = Some(decoded.hour);
        if gap || !valid || identity_changed || !continuous || result.live.is_empty() {
            let live = if gap || !valid {
                Default::default()
            } else {
                self.identities.iter().map(|(tag, _)| tag.clone()).collect()
            };
            result.removed = result.live.difference(&live).cloned().collect();
            result.live = live;
        }
        self.applied = Some(decoded.hour);
        self.reads = Some(reads);
        true
    }
    fn new(detailed: bool) -> Self {
        Self {
            inventory: detailed.then(Inventory::default),
            ..Default::default()
        }
    }
    fn apply_changes(
        &mut self,
        frame: &(impl replay_frame::frame::FrameSource + ?Sized),
        decoded: &DecodedFrame,
    ) {
        self.apply_updates(Updates::recorded(frame), decoded);
    }
    fn apply_updates(&mut self, updates: Updates<'_>, decoded: &DecodedFrame) {
        if let Some(inventory) = &mut self.inventory {
            inventory.apply_updates(updates, decoded);
        }
    }
    fn compose(&mut self, decoded: &DecodedFrame) {
        if let Some(inventory) = &mut self.inventory {
            inventory.compose(decoded);
        }
    }
    fn identities(&mut self, decoded: &DecodedFrame) -> bool {
        self.identities_kept(decoded, false)
    }
    /// Read the hour's country identities. `kept`: the Countries rows changed
    /// from the previous hour's, which the held keys were read from, by
    /// recorded rows that change no key, slot or tag.
    fn identities_kept(&mut self, decoded: &DecodedFrame, kept: bool) -> bool {
        self.identity_changed = false;
        let (Ok(countries), Ok(links)) = (
            decoded.complete::<tables::Countries>(),
            decoded.complete::<tables::CountryLinks>(),
        ) else {
            self.countries = None;
            self.links = None;
            self.identity_changed = true;
            self.identities.clear();
            self.keys = None;
            self.country_rows = None;
            return false;
        };
        let country_view = &decoded.tables[TableId::Countries.index()];
        let link_view = &decoded.tables[TableId::CountryLinks.index()];
        if self
            .countries
            .as_ref()
            .is_some_and(|old| old.is(country_view))
            && self.links.as_ref().is_some_and(|old| old.is(link_view))
        {
            return true;
        }
        // Without deletions, the same row count means no country was added.
        if self.links.as_ref().is_some_and(|old| old.is(link_view))
            && self.keys.as_ref().is_some_and(|keys| {
                (kept && self.country_rows == Some(countries.len())) || keys.same(countries)
            })
        {
            self.countries = Some(country_view.identity());
            return true;
        }
        self.identity_changed = true;
        let tagged = tagged_countries(countries, links);
        self.unreadable_tags = tagged
            .iter()
            .filter(|(tag, lookup)| {
                matches!(lookup, TaggedCountry::Found(_, _)) && std::str::from_utf8(tag).is_err()
            })
            .count();
        self.identities = tagged
            .into_iter()
            .filter_map(|(tag, lookup)| {
                let TaggedCountry::Found(country, slot) = lookup else {
                    return None;
                };
                std::str::from_utf8(tag)
                    .ok()
                    .map(|tag| (tag.to_owned(), (slot, country.id)))
            })
            .collect();
        self.slots.clear();
        for (index, (_, identity)) in self.identities.iter().enumerate() {
            self.slots.entry(identity.0).or_default().push(index);
        }
        self.countries = Some(country_view.identity());
        self.links = Some(link_view.identity());
        self.keys = Some(dashboard_text::metadata::CountryKeys::read(countries));
        self.country_rows = Some(countries.len());
        true
    }
    /// Read the census whole: each slot's division count, the slots whose
    /// count changed, and the rows no link resolves.
    fn read_census(
        &mut self,
        census: Option<&[DivisionCensusRow]>,
        links: &[CountryLinkRow],
        hour: i32,
    ) {
        let mut counts = HashMap::<u32, Option<i64>>::new();
        let mut invalid = false;
        let mut unreadable = 0;
        self.census_rows.clear();
        self.census_sums.clear();
        for row in census.into_iter().flatten() {
            #[cfg(test)]
            {
                self.work.row_visits += 1;
            }
            let slot = link_slot(links, row.country);
            let value = i64::from(row.grouped) + i64::from(row.ungrouped);
            self.census_rows.insert(row.country, (slot, value));
            if let Some(slot) = slot {
                add(counts.entry(slot).or_insert(Some(0)), value);
                let sum = self.census_sums.entry(slot).or_default();
                sum.0 += 1;
                sum.1 += i128::from(value);
            } else if row.grouped != 0 || row.ungrouped != 0 {
                invalid = true;
                unreadable += 1;
            }
        }
        self.census_dirty.extend(
            self.census_counts
                .keys()
                .chain(counts.keys())
                .filter(|slot| self.census_counts.get(slot) != counts.get(slot))
                .copied(),
        );
        if self.census_invalid != invalid {
            self.restriction = Some(false);
        }
        self.census_counts = counts;
        self.census_invalid = invalid;
        self.census_unreadable = unreadable;
        // The held rows answer a later hour's changed rows only when each
        // country had one row.
        self.census_hour = census
            .filter(|census| census.len() == self.census_rows.len())
            .map(|_| hour);
    }
    /// Apply the census rows that changed from the previous hour's, which
    /// `census_rows` hold, through the same links: only their slots' counts
    /// can change. False, changing nothing, when a changed row is not in the
    /// census or a deleted key is not held: the census is then read whole.
    fn apply_census_rows(
        &mut self,
        census: &[DivisionCensusRow],
        links: &[CountryLinkRow],
        (changed, deleted): (&[DeltaRow], &[u128]),
        hour: i32,
    ) -> bool {
        let mut read = Vec::with_capacity(changed.len());
        let mut from = 0;
        for change in changed {
            let Some(at) = inventory::find_from(census, from, change.key) else {
                return false;
            };
            from = at;
            read.push(&census[from]);
        }
        let Some(deleted) = deleted
            .iter()
            .map(|&key| {
                u32::try_from(key)
                    .ok()
                    .filter(|key| self.census_rows.contains_key(key))
            })
            .collect::<Option<Vec<_>>>()
        else {
            return false;
        };
        #[cfg(test)]
        {
            self.work.row_visits += read.len() + deleted.len();
        }
        let mut slots = Vec::new();
        let mut unlinked = self.census_unreadable;
        for country in deleted {
            let held = self.census_rows.remove(&country).expect("held census row");
            census_edit(
                &mut self.census_sums,
                &mut unlinked,
                &mut slots,
                held,
                false,
            );
        }
        for row in read {
            let value = i64::from(row.grouped) + i64::from(row.ungrouped);
            let slot = link_slot(links, row.country);
            if let Some(held) = self.census_rows.insert(row.country, (slot, value)) {
                census_edit(
                    &mut self.census_sums,
                    &mut unlinked,
                    &mut slots,
                    held,
                    false,
                );
            }
            census_edit(
                &mut self.census_sums,
                &mut unlinked,
                &mut slots,
                (slot, value),
                true,
            );
        }
        for slot in slots {
            // Counts are not negative, so a running sum overflows exactly
            // when the slot's whole sum does.
            let count = self
                .census_sums
                .get(&slot)
                .map(|&(_, sum)| i64::try_from(sum).ok());
            if self.census_counts.get(&slot) != count.as_ref() {
                self.census_dirty.insert(slot);
                match count {
                    Some(count) => self.census_counts.insert(slot, count),
                    None => self.census_counts.remove(&slot),
                };
            }
        }
        let invalid = unlinked > 0;
        if self.census_invalid != invalid {
            self.restriction = Some(false);
        }
        self.census_invalid = invalid;
        self.census_unreadable = unlinked;
        self.census_hour = Some(hour);
        true
    }
}

/// Count one held census row in (`adding`) or out: in its slot's sum, or
/// among the unresolved rows when no link names it and it counts divisions.
fn census_edit(
    sums: &mut HashMap<u32, (usize, i128)>,
    unlinked: &mut usize,
    slots: &mut Vec<u32>,
    (slot, value): (Option<u32>, i64),
    adding: bool,
) {
    match slot {
        Some(slot) => {
            let sum = sums.entry(slot).or_default();
            if adding {
                sum.0 += 1;
                sum.1 += i128::from(value);
            } else {
                sum.0 -= 1;
                sum.1 -= i128::from(value);
            }
            if sum.0 == 0 {
                sums.remove(&slot);
            }
            slots.push(slot);
        }
        None if value != 0 => {
            if adding {
                *unlinked += 1;
            } else {
                *unlinked -= 1;
            }
        }
        None => {}
    }
}

fn observe_totals(
    result: &mut Scan,
    decoded: &DecodedFrame,
    frame: &(impl replay_frame::frame::FrameSource + ?Sized),
    format: replay_data::format::FormatVersion,
    gap: bool,
    state: &mut ScanState,
    changes: Option<&FrameChanges>,
) {
    observe_updates(
        result,
        decoded,
        Updates::recorded(frame),
        format,
        gap,
        state,
        changes,
    );
}

fn observe_updates(
    result: &mut Scan,
    decoded: &DecodedFrame,
    updates: Updates<'_>,
    format: replay_data::format::FormatVersion,
    gap: bool,
    state: &mut ScanState,
    changes: Option<&FrameChanges>,
) {
    let hour = decoded.hour;
    if let Some(inventory) = &state.inventory {
        for (table, reason) in inventory.errors() {
            *result
                .invalid_table_reads
                .entry(format!("{table:?}: {reason}"))
                .or_default() += 1;
        }
    }
    let division_read = updates.read(TableId::Divisions, decoded)
        && slot_read(format, &decoded.metrics, TableId::Divisions, hour);
    let template_read = updates.read(TableId::DivisionTemplateIcons, decoded)
        && slot_read(
            format,
            &decoded.metrics,
            TableId::DivisionTemplateIcons,
            hour,
        );
    if division_read {
        result.strength = true;
        result.divisions = true;
        result.manpower |= slot_read(format, &decoded.metrics, TableId::DivisionLandInputs, hour)
            && updates.read(TableId::DivisionLandInputs, decoded);
        result.battalions |= template_read;
    }
    if gap || !state.identities(decoded) {
        return;
    }
    result.unreadable += state.unreadable_tags;
    let Ok(links) = decoded.complete::<tables::CountryLinks>() else {
        return;
    };
    let census = decoded
        .complete::<tables::DivisionCensus>()
        .ok()
        .filter(|_| slot_read(format, &decoded.metrics, TableId::DivisionCensus, hour));
    result.inventory_fallback |= division_read && census.is_none();
    state.census_dirty.clear();
    let census_changed = state.census_ready != Some(census.is_some())
        || changes.is_none_or(|changes| {
            changes.rows_changed[TableId::DivisionCensus.index()]
                || changes.rows_changed[TableId::CountryLinks.index()]
        });
    if census_changed {
        // The previous hour's census, held, changed by recorded rows
        // through the same links.
        let rows = census
            .filter(|_| {
                state.census_ready == Some(true)
                    && state
                        .census_hour
                        .is_some_and(|at| at.checked_add(1) == Some(hour))
            })
            .zip(
                changes
                    .filter(|changes| !changes.rows_changed[TableId::CountryLinks.index()])
                    .and_then(|changes| changes.rows(TableId::DivisionCensus)),
            );
        if !rows
            .is_some_and(|(census, changed)| state.apply_census_rows(census, links, changed, hour))
        {
            state.read_census(census, links, hour);
        }
        state.census_ready = Some(census.is_some());
    } else if state
        .census_hour
        .is_some_and(|at| at.checked_add(1) == Some(hour))
    {
        state.census_hour = Some(hour);
    }
    result.divisions |= census.is_some();
    result.unreadable += state.census_unreadable;
    let inventory = state
        .inventory
        .as_ref()
        .filter(|inventory| division_read && inventory.known());
    let composition = inventory.filter(|inventory| template_read && inventory.composition_known());
    if let Some(inventory) = inventory {
        result.unreadable += inventory.unreadable();
    }
    let indices: Vec<_> = if state.restriction == Some(true) {
        state
            .inventory
            .as_ref()
            .into_iter()
            .flat_map(|inventory| inventory.dirty.iter())
            .chain(&state.census_dirty)
            .filter_map(|slot| state.slots.get(slot))
            .flatten()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    } else {
        (0..state.identities.len()).collect()
    };
    result.composition_unreadable += state.composition_rate;
    for index in indices {
        let (tag, identity) = &state.identities[index];
        let slot = identity.0;
        #[cfg(test)]
        {
            state.work.history_visits += 1;
        }
        let mut total = inventory.map_or_else(Totals::default, |inventory| {
            inventory.totals(slot, composition.is_some())
        });
        if !slot_read(format, &decoded.metrics, TableId::DivisionLandInputs, hour) {
            total.values[6] = None;
            total.values[7] = None;
        }
        if census.is_some() {
            total.values[0] = (!state.census_invalid)
                .then(|| {
                    state
                        .census_counts
                        .get(&slot)
                        .copied()
                        .flatten()
                        .map(i128::from)
                })
                .flatten();
        }
        // Identity coverage is independent of measure availability.
        if !result.tracks.contains_key(tag) {
            result.tracks.insert(tag.clone(), Track::default());
        }
        let Some(track) = result.tracks.get_mut(tag) else {
            continue;
        };
        if result.live.contains(tag)
            && let Some(frontier) = result.frontier
        {
            track.advance(frontier);
        }
        observe_identity(
            &mut track.segments,
            &mut track.last_identity,
            *identity,
            hour,
        );
        result.observed.push(index);
        let composition_notes = composition.map_or(0, |inventory| {
            inventory
                .parts_version(slot)
                .2
                .iter()
                .filter(|known| !**known)
                .count()
        });
        let before_notes = match state.composition_rates.get_mut(tag) {
            Some(notes) => std::mem::replace(notes, composition_notes),
            None => {
                state
                    .composition_rates
                    .insert(tag.clone(), composition_notes);
                0
            }
        };
        state.composition_rate = state.composition_rate + composition_notes - before_notes;
        result.composition_unreadable =
            result.composition_unreadable + composition_notes - before_notes;
        if census.is_none() && inventory.is_none() {
            continue;
        }
        let continues = track
            .last
            .is_some_and(|(last, at)| last == *identity && at.checked_add(1) == Some(hour));
        match track.runs.last_mut() {
            Some(run) if continues && run.totals == total => run.last = hour,
            _ => track.runs.push(RawRun {
                first: hour,
                last: hour,
                starts: !continues,
                totals: total,
            }),
        }
        if let Some(inventory) = composition {
            let version = inventory.parts_version(slot);
            if let Some(run) = track.mix.last_mut()
                && continues
                && track.mix_version == Some(version)
                && run.last.checked_add(1) == Some(hour)
            {
                run.last = hour;
            } else {
                let parts = inventory.parts_at(slot);
                match track.mix.last_mut() {
                    Some(run)
                        if continues
                            && run.last.checked_add(1) == Some(hour)
                            && run.parts == parts =>
                    {
                        run.last = hour
                    }
                    _ => track.mix.push(RawMixRun {
                        first: hour,
                        last: hour,
                        parts,
                    }),
                }
            }
            track.mix_version = Some(version);
        }
        track.last = Some((*identity, hour));
    }
}

#[derive(Default)]
#[cfg(any(test, feature = "test-support"))]
struct Scans {
    entries: VecDeque<(u64, bool, Arc<Scan>)>,
    runs: usize,
}

#[cfg(any(test, feature = "test-support"))]
fn scanned(
    source: &Source,
    branch: u64,
    detailed: bool,
    cancelled: &AtomicBool,
) -> io::Result<Arc<Scan>> {
    dashboard_text::metadata::check_cancel(cancelled)?;
    let state = source.state::<Scans>();
    let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
    if !source
        .replay_ref()
        .branches
        .iter()
        .any(|known| known.id == branch)
    {
        return Err(dashboard_core::invalid("Unknown branch"));
    }
    if let Some((_, detail, scan)) = cache.entries.iter().find(|(id, _, _)| *id == branch)
        && (*detail || !detailed)
    {
        return Ok(Arc::clone(scan));
    }
    cache.runs += 1;
    let mut result = scan(source.replay_ref(), branch, detailed, cancelled)?;
    let fallback = !detailed && result.inventory_fallback;
    if fallback {
        cache.runs += 1;
        result = scan(source.replay_ref(), branch, true, cancelled)?;
    }
    let scan = Arc::new(result);
    cache.entries.retain(|(id, _, _)| *id != branch);
    cache
        .entries
        .push_back((branch, detailed || fallback, Arc::clone(&scan)));
    while cache.entries.len() > 2 {
        cache.entries.pop_front();
    }
    Ok(scan)
}

/// Scoreboard's army column: exact per-country value at one recorded hour,
/// using the same cached scan and owner/host rules as the widget. Unavailable
/// tables, missing country rows, invalid HP and gaps return None.
#[cfg(test)]
pub fn army_value_at(
    source: &Source,
    branch: u64,
    tag: &str,
    hour: i32,
    measure: ArmyMeasure,
    count_for: ArmyAttribution,
    cancelled: &AtomicBool,
) -> io::Result<Option<ArmyValue>> {
    let scan = scanned(
        source,
        branch,
        measure != ArmyMeasure::Divisions || count_for == ArmyAttribution::Host,
        cancelled,
    )?;
    let value = scan.tracks.get(tag).and_then(|track| {
        let index = track
            .runs
            .partition_point(|run| run.first <= hour)
            .checked_sub(1)?;
        let run = &track.runs[index];
        (hour <= run.last)
            .then(|| run.totals.values[channel(measure, count_for)])
            .flatten()
    });
    Ok(value.map(|raw| ArmyValue {
        value: amount(Some(raw), measure).expect("recorded total"),
        raw,
        read_hour: hour,
    }))
}

/// Exact cached runs for the requested tags; an empty selection visits all tags.
/// Inclusive hours are hourly supplying reads. None stays unknown, and starts
/// preserve native identity/gap boundaries without expanding campaign hours.
#[cfg(any(test, feature = "test-support"))]
pub fn visit_army_runs(
    source: &Source,
    branch: u64,
    tags: &[String],
    measure: ArmyMeasure,
    attribution: ArmyAttribution,
    cancelled: &AtomicBool,
    mut visitor: impl FnMut(&str, i32, i32, bool, Option<i128>) -> io::Result<()>,
) -> io::Result<()> {
    let scan = scanned(
        source,
        branch,
        measure != ArmyMeasure::Divisions || attribution == ArmyAttribution::Host,
        cancelled,
    )?;
    let index = channel(measure, attribution);
    let mut visit = |tag: &str, track: &Track| -> io::Result<()> {
        let mut pending: Option<(i32, i32, bool, Option<i128>)> = None;
        for run in &track.runs {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let raw = run.totals.values[index];
            if let Some(previous) = pending.as_mut()
                && !run.starts
                && previous.1.checked_add(1) == Some(run.first)
                && previous.3 == raw
            {
                previous.1 = run.last;
            } else {
                if let Some((first, last, starts, raw)) = pending.take() {
                    visitor(tag, first, last, starts, raw)?;
                }
                pending = Some((run.first, run.last, run.starts, raw));
            }
        }
        if let Some((first, last, starts, raw)) = pending {
            visitor(tag, first, last, starts, raw)?;
        }
        Ok(())
    };
    if tags.is_empty() {
        for (tag, track) in &scan.tracks {
            visit(tag, track)?;
        }
    } else {
        for tag in tags.iter().collect::<std::collections::BTreeSet<_>>() {
            if let Some(track) = scan.tracks.get(tag) {
                visit(tag, track)?;
            }
        }
    }
    Ok(())
}

fn for_each_reply_run(
    track: &Track,
    params: &ArmySizeParams,
    range: &HourSpan,
    spans: Option<&[HourSpan]>,
    cancelled: &AtomicBool,
    mut output: impl FnMut(ArmyRun) -> io::Result<()>,
) -> io::Result<()> {
    let index = channel(params.measure, params.count_for);
    let mut pending: Option<ArmyRun> = None;
    let mut visit = |raw: &RawRun, first, last| -> io::Result<()> {
        let Some(exact) = raw.totals.values[index] else {
            return Ok(());
        };
        let starts_segment = (raw.starts && first == raw.first)
            || pending
                .as_ref()
                .is_none_or(|run| run.last_hour.checked_add(1) != Some(first));
        if let Some(previous) = pending.as_mut()
            && !starts_segment
            && previous.raw == exact.to_string()
        {
            previous.last_hour = last;
        } else {
            if let Some(previous) = pending.take() {
                output(previous)?;
            }
            pending = Some(ArmyRun {
                first_hour: first,
                last_hour: last,
                total: amount(Some(exact), params.measure),
                raw: exact.to_string(),
            });
        }
        Ok(())
    };
    let runs = window_runs(track, range);
    let whole = [*range];
    dashboard_base::ranking::for_each_piece(
        runs,
        spans.unwrap_or(&whole),
        |run| (run.first, run.last),
        cancelled,
        &mut visit,
    )?;
    if let Some(last) = pending {
        output(last)?;
    }
    Ok(())
}

fn reply_bound() -> io::Error {
    io::Error::other("Army size reply exceeds the transport reply budget; choose fewer countries")
}

fn window_runs<'a>(track: &'a Track, range: &HourSpan) -> &'a [RawRun] {
    let first = track
        .runs
        .partition_point(|run| run.last < range.first_hour);
    let last = track
        .runs
        .partition_point(|run| run.first <= range.last_hour);
    &track.runs[first..last]
}

fn window_mix<'a>(track: &'a Track, range: &HourSpan) -> &'a [RawMixRun] {
    let first = track.mix.partition_point(|run| run.last < range.first_hour);
    let last = track
        .mix
        .partition_point(|run| run.first <= range.last_hour);
    &track.mix[first..last]
}

fn window_segments(track: &Track, range: &HourSpan) -> Vec<HourSpan> {
    // Keep the original identity start for country looks; omit other windows.
    let first = track
        .segments
        .partition_point(|span| span.last_hour < range.first_hour);
    let last = track
        .segments
        .partition_point(|span| span.first_hour <= range.last_hour);
    track.segments[first..last]
        .iter()
        .map(|span| HourSpan {
            first_hour: span.first_hour,
            last_hour: span.last_hour.min(range.last_hour),
        })
        .collect()
}

fn window_identities(
    country: &dashboard_base::products::country_history::RecordedCountry,
    range: &HourSpan,
) -> Vec<dashboard_base::products::country_history::CountryIdentityRun> {
    let first = country
        .identities
        .partition_point(|identity| identity.last_hour < range.first_hour);
    let last = country
        .identities
        .partition_point(|identity| identity.first_hour <= range.last_hour);
    country.identities[first..last]
        .iter()
        .map(
            |identity| dashboard_base::products::country_history::CountryIdentityRun {
                first_hour: identity.first_hour.max(range.first_hour),
                last_hour: identity.last_hour.min(range.last_hour),
                slot: identity.slot,
                country_id: identity.country_id,
            },
        )
        .collect()
}

fn reply_series(
    scan: &Scan,
    params: &ArmySizeParams,
    range: &HourSpan,
    selected: Option<
        &BTreeMap<String, Vec<dashboard_base::products::country_history::CountryIdentityRun>>,
    >,
    cancelled: &AtomicBool,
    prepared_membership: Option<BTreeMap<String, Vec<HourSpan>>>,
) -> io::Result<(Vec<ArmySeries>, usize)> {
    let transport_budget = dashboard_base::products::MAX_PRODUCT_REPLY_BYTES;
    let index = channel(params.measure, params.count_for);
    let membership = if prepared_membership.is_some() {
        prepared_membership
    } else if let Some(top) = params.top {
        let runs: BTreeMap<_, _> = scan
            .tracks
            .iter()
            .map(|(tag, track)| {
                (
                    tag.clone(),
                    window_runs(track, range)
                        .iter()
                        .map(|run| RawRun {
                            first: run.first.max(range.first_hour),
                            last: run.last.min(range.last_hour),
                            starts: run.starts,
                            totals: run.totals,
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        Some(dashboard_base::ranking::top_membership(
            &runs,
            top,
            |run| (run.first, run.last),
            |run| run.totals.values[index],
            cancelled,
        )?)
    } else {
        None
    };
    let mut seen = std::collections::BTreeSet::new();
    let wanted: Vec<_> = membership.as_ref().map_or_else(
        || {
            params
                .tags
                .iter()
                .filter(|tag| seen.insert(*tag))
                .map(|tag| (tag.clone(), None))
                .collect()
        },
        |members| {
            members
                .iter()
                .map(|(tag, spans)| (tag.clone(), Some(spans.clone())))
                .collect()
        },
    );
    let wanted: Vec<_> = wanted
        .into_iter()
        .filter_map(|(tag, membership)| {
            let track = scan.tracks.get(&tag)?;
            let identities = selected.map(|countries| countries[&tag].clone());
            let spans = identities.as_ref().map(|identities| {
                identities
                    .iter()
                    .map(|identity| HourSpan {
                        first_hour: identity.first_hour,
                        last_hour: identity.last_hour,
                    })
                    .collect::<Vec<_>>()
            });
            let mut segments = window_segments(track, range);
            if let Some(spans) = &spans {
                segments.retain(|segment| {
                    let first = spans.partition_point(|span| span.last_hour < segment.first_hour);
                    spans
                        .get(first)
                        .is_some_and(|span| span.first_hour <= segment.last_hour)
                });
            }
            Some((
                ArmySeries {
                    tag,
                    identities,
                    runs: Vec::new(),
                    segments,
                    membership,
                    mix: None,
                },
                track,
                spans,
            ))
        })
        .collect();
    // Count the selected serialized series before copying its history.
    let mut bytes = 2usize; // Array brackets.
    let mut series_comma = 0;
    for (series, track, spans) in &wanted {
        dashboard_text::metadata::check_cancel(cancelled)?;
        bytes += series_comma
            + dashboard_core::transport::json_len(series, transport_budget)
                .ok_or_else(reply_bound)?;
        series_comma = 1;
        if bytes > transport_budget {
            return Err(reply_bound());
        }
        let mut run_comma = 0;
        for_each_reply_run(
            track,
            params,
            range,
            spans.as_deref().or(series.membership.as_deref()),
            cancelled,
            |run| {
                bytes += run_comma
                    + dashboard_core::transport::json_len(&run, transport_budget)
                        .ok_or_else(reply_bound)?;
                run_comma = 1;
                if bytes > transport_budget {
                    return Err(reply_bound());
                }
                Ok(())
            },
        )?;
    }
    let series = wanted
        .into_iter()
        .map(|(mut series, track, spans)| {
            for_each_reply_run(
                track,
                params,
                range,
                spans.as_deref().or(series.membership.as_deref()),
                cancelled,
                |run| {
                    series.runs.push(run);
                    Ok(())
                },
            )?;
            Ok(series)
        })
        .collect::<io::Result<Vec<_>>>()?;
    Ok((series, bytes))
}

impl Product for ArmySize {
    const KEY: &'static str = "army-size";
    const CONTENT_DEPENDENT: bool = true;
    type Params = ArmySizeParams;
    type Reply = ArmySizeReply;
    fn types() -> Vec<String> {
        vec![
            ArmyMeasure::decl(),
            ArmyAttribution::decl(),
            ArmySizeParams::decl(),
            ArmyType::decl(),
            "type ArmyRun = [number, number | null, number | null, string?];".into(),
            ArmyMixRun::decl(),
            ArmySeries::decl(),
            ArmySizeReply::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if let Some((tag, _)) = &params.template_at {
            validate_tag(tag)?;
        }
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        if params
            .top
            .is_some_and(|top| top == 0 || !params.tags.is_empty())
        {
            return Err("Invalid country selection");
        }
        for tag in &params.tags {
            validate_tag(tag)?;
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
        if params.template_at.is_some() {
            return template_roster::query(source, params, context, cancelled);
        }
        let branch = params
            .branch_id
            .parse()
            .map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let detailed =
            params.measure != ArmyMeasure::Divisions || params.count_for == ArmyAttribution::Host;
        let scan = scanned(source, branch, detailed, cancelled)?;
        build_reply(source, params, context, cancelled, &scan, None, None)
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "Oracle and prepared windows share one reply builder"
)]
fn build_reply(
    source: &Source,
    params: &ArmySizeParams,
    context: &LoadContext,
    cancelled: &AtomicBool,
    scan: &Scan,
    histories: Option<Arc<dashboard_base::products::country_history::RecordedHistories>>,
    prepared_membership: Option<BTreeMap<String, Vec<HourSpan>>>,
) -> io::Result<ArmySizeReply> {
    let branch = params
        .branch_id
        .parse()
        .map_err(|_| dashboard_core::invalid("Invalid branch"))?;
    if !source
        .replay_ref()
        .branches
        .iter()
        .any(|known| known.id == branch)
    {
        return Err(dashboard_core::invalid("Unknown branch"));
    }
    let windows = source.block_windows(branch);
    let interval = params
        .window
        .map(|index| {
            windows
                .get(index)
                .cloned()
                .ok_or_else(|| dashboard_core::invalid("Unknown army history window"))
        })
        .transpose()?;
    let mut warnings = Vec::new();
    warnings.extend(scan.invalid_table_reads.iter().map(|(reason, count)| {
        format!("{count} army table reads rejected ({reason}); affected values stay gaps.")
    }));
    if scan.unreadable > 0 {
        warnings.push(format!("{} division/country attribution reads could not be resolved; affected totals remain unknown.", scan.unreadable));
    }
    if scan.composition_unreadable > 0 && params.measure == ArmyMeasure::Battalions {
        warnings.push(format!(
                "{} country attribution-hours have unresolved division ownership or missing/invalid template composition; battalion totals stay unknown.",
                scan.composition_unreadable));
    }
    let measures = [
        ArmyMeasure::Manpower,
        ArmyMeasure::Divisions,
        ArmyMeasure::Strength,
        ArmyMeasure::Battalions,
    ]
    .into_iter()
    .filter(|measure| match measure {
        ArmyMeasure::Divisions => scan.divisions,
        ArmyMeasure::Strength => scan.strength,
        ArmyMeasure::Manpower => scan.manpower,
        ArmyMeasure::Battalions => scan.battalions,
    })
    .collect();
    let mut projection = params.clone();
    let identities = if params.top.is_none() && !params.tags.is_empty() {
        let identities = if let Some(histories) = histories {
            histories
        } else {
            dashboard_base::products::country_history::branch_histories(
                source,
                branch,
                false,
                &[],
                cancelled,
            )?
        };
        let selected = dashboard_base::products::country_history::selected_identities(
            &identities,
            &params.tags,
        );
        projection.tags =
            dashboard_base::products::country_history::selected_aliases(&identities, &params.tags);
        interval.as_ref().map(|range| {
            projection
                .tags
                .iter()
                .map(|tag| {
                    let runs = identities
                        .countries
                        .get(tag)
                        .map_or_else(Vec::new, |country| {
                            window_identities(country, range)
                                .into_iter()
                                .filter(|identity| {
                                    selected.contains(&(identity.slot, identity.country_id))
                                })
                                .collect()
                        });
                    (tag.clone(), runs)
                })
                .collect::<BTreeMap<_, _>>()
        })
    } else {
        None
    };
    let (mut series, series_bytes) = interval.as_ref().map_or_else(
        || Ok((Vec::new(), 2)),
        |range| {
            reply_series(
                scan,
                &projection,
                range,
                identities.as_ref(),
                cancelled,
                prepared_membership,
            )
        },
    )?;
    let types = if let Some(range) = interval.as_ref()
        && params.by_type
        && params.measure == ArmyMeasure::Battalions
    {
        composition_reply(
            source,
            context,
            scan,
            params,
            range,
            &mut series,
            &mut warnings,
            cancelled,
        )?
    } else {
        Vec::new()
    };
    let mut reply = ArmySizeReply {
        template_sections: None,
        branch_id: params.branch_id.clone(),
        windows: if params.window.is_none() {
            windows.to_vec()
        } else {
            Vec::new()
        },
        window: params.window,
        interval,
        available: match params.measure {
            ArmyMeasure::Divisions if params.count_for == ArmyAttribution::Host => scan.strength,
            ArmyMeasure::Divisions => scan.divisions,
            ArmyMeasure::Strength => scan.strength,
            ArmyMeasure::Manpower => scan.manpower,
            ArmyMeasure::Battalions => scan.battalions,
        },
        measures,
        types,
        series: Vec::new(),
        warnings,
    };
    let envelope_bytes = dashboard_core::transport::json_len(
        &reply,
        dashboard_base::products::MAX_PRODUCT_REPLY_BYTES,
    )
    .ok_or_else(reply_bound)?;
    let bytes = if series
        .iter()
        .any(|series| series.mix.is_some() || series.identities.is_some())
    {
        dashboard_core::transport::json_len(
            &series,
            dashboard_base::products::MAX_PRODUCT_REPLY_BYTES,
        )
        .ok_or_else(reply_bound)?
    } else {
        series_bytes
    };
    if envelope_bytes + bytes - 2 > dashboard_base::products::MAX_PRODUCT_REPLY_BYTES {
        return Err(reply_bound());
    }
    reply.series = series;
    Ok(reply)
}

mod composition;
pub mod prepared;
mod template_roster;
pub use composition::check_content;
use composition::{RawMixRun, composition_reply};
pub(crate) mod aid;
mod inventory;
use inventory::{Inventory, Updates};

#[cfg(any(test, feature = "test-support"))]
pub mod test_fixtures;
#[cfg(test)]
mod tests;
