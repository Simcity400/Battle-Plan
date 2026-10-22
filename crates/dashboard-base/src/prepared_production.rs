//! Dated production headers joined to shared changed-leaf events.

use crate::metadata::{TaggedCountry, slot_read};
use crate::products::equipment::production::{
    CostTotal, Entry, Identity, Named, ProductionSpendingUnknown, Purpose, RocketProduced,
    TagHistory, Total, variant_types_for,
};
use crate::products::equipment::{
    self, Domain, StockVariantName, Texts, VariantCatalogue, VariantHour, VariantReads,
    VariantVersion,
};
use crate::products::equipment_produced::{
    ProducedCount, ProducedGrouping, ProducedKey, ProducedMeasure, ProducedParams, ProducedType,
    ScannedProduced, month_before,
};
use replay_data::{format::FormatVersion, rows::Recorded, rows::TableId, rows::tables};
use replay_frame::{frame::DecodedFrame, frame::TableView};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::atomic::{AtomicBool, Ordering},
};

/// Owning content handles keep the ordinary recorded-stack naming loader alive.
pub struct ProductionNaming {
    catalogue: Option<std::sync::Arc<crate::catalogue::Catalogue>>,
    units: Option<std::sync::Arc<installed_content_core::equipment::Database>>,
    pub warnings: Vec<String>,
}
impl ProductionNaming {
    pub fn load(source: &crate::Source, context: &crate::products::LoadContext) -> Self {
        let mut warnings = Vec::new();
        let (catalogue, units) = equipment::production::content(source, context, &mut warnings);
        Self {
            catalogue,
            units,
            warnings,
        }
    }
    pub fn named(&self) -> Named<'_> {
        Named {
            units: self.units.as_deref(),
            texts: self
                .catalogue
                .as_ref()
                .map(|catalogue| &**catalogue as &dyn Texts),
        }
    }
}

pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::LexicalTokens,
    TableId::VariantDefinitions,
    TableId::VariantChanges,
    TableId::VariantModules,
    TableId::VariantUpgrades,
    TableId::ProductionLines,
    TableId::ProductionLineOutput,
    TableId::RocketProductionOutput,
];

pub use dashboard_core::map_pairs;

#[derive(Deserialize, Serialize)]
struct RowState {
    identity: Option<Identity>,
    entry: Entry,
    /// Its purpose was recorded.
    proved: bool,
    units: i64,
    exact: bool,
    ended: bool,
    spent_raw: i64,
}

#[derive(Default, Deserialize, Serialize)]
struct Current {
    tags: BTreeMap<String, TagHistory>,
    active: BTreeSet<String>,
    reads: Vec<i32>,
    observations: Vec<i32>,
    unattributed: usize,
    inexact: usize,
    lost: usize,
    unread: usize,
    unread_reason: Option<String>,
    unnamed: usize,
    missed: usize,
    tainted: BTreeMap<u32, i32>,
    spending_tainted: BTreeMap<u32, (i32, ProductionSpendingUnknown)>,
    rocket: RocketProduced,
}
impl Current {
    fn taint(&mut self, observation: u32, hour: i32) {
        self.missed += 1;
        self.tainted.entry(observation).or_insert(hour);
        self.spending_taint(
            observation,
            hour,
            ProductionSpendingUnknown::MissedPublication,
        );
    }
    fn spending_taint(&mut self, observation: u32, hour: i32, reason: ProductionSpendingUnknown) {
        self.spending_tainted
            .entry(observation)
            .and_modify(|first| {
                if hour < first.0 {
                    *first = (hour, reason);
                }
            })
            .or_insert((hour, reason));
    }
}

#[derive(Default, Deserialize, Serialize)]
pub struct ProductionAccumulator {
    pub schema: u32,
    branch: u64,
    result: Current,
    #[serde(with = "map_pairs")]
    rows: HashMap<(u64, u32), RowState>,
    #[serde(serialize_with = "crate::prepared_production::map_pairs::ordered_set")]
    live: HashSet<(u64, u32)>,
    #[serde(with = "versioned_pairs")]
    totals: HashMap<Identity, (u64, BTreeMap<Entry, Total>)>,
    #[serde(with = "versioned_pairs")]
    cost_totals: HashMap<Identity, (u64, BTreeMap<Entry, CostTotal>)>,
    #[serde(serialize_with = "crate::prepared_production::map_pairs::ordered_set")]
    seen_production: HashSet<Identity>,
    #[serde(serialize_with = "crate::prepared_production::map_pairs::ordered_set")]
    unlisted: HashSet<u64>,
    version: u64,
    cost_version: u64,
    observation: u32,
    spending_recorded: bool,
    last_frame: Option<i32>,
    classifications: BTreeMap<u64, Classification>,
    pending: Option<EndpointPage>,
    segments: BTreeMap<String, u64>,
    rocket_segments: BTreeMap<String, u64>,
    warnings: Vec<String>,
    joins: RocketJoins,
    pages_written: u64,
    spent: crate::prepared::series::Accumulator,
    spent_keys: BTreeMap<String, u64>,
    /// Keys of each equipment entry's cumulative input from production ticks
    /// whose purpose was not recorded, by (tag, variant, purpose); only
    /// entries that ever had such input.
    unproven_keys: BTreeMap<String, u64>,
    /// Keys of each equipment design's cumulative input of each
    /// [`SERIES_PURPOSES`] purpose, by (tag, variant, purpose); only designs
    /// that ever had input of that purpose. The key also holds that amount's
    /// endpoint events.
    purpose_keys: BTreeMap<String, u64>,
    spent_active: BTreeSet<u64>,
    #[serde(skip)]
    spent_holders: [BTreeMap<String, SpentHolder>; 2],
    #[serde(skip)]
    spent_holders_ready: bool,
    #[cfg(test)]
    #[serde(skip)]
    spent_visits: usize,
    classification_keys: BTreeMap<u64, u64>,
    /// Last published payload fingerprints, never a second copy of quantities.
    endpoint_hashes: BTreeMap<u64, [u8; 32]>,
    endpoint_active: BTreeSet<u64>,
    #[serde(skip)]
    country_reads: Option<crate::products::territory::CountryReads>,
    #[serde(skip)]
    country_unnamed: usize,
    #[serde(skip)]
    variants: ProductionVariants,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    pub work: crate::prepared::observer_tests::Work,
    first_equipment_holders: BTreeMap<String, (u32, u32)>,
    first_rocket_holders: BTreeMap<String, (u32, u32)>,
    equipment_origin: Option<i32>,
    equipment_restart: Option<i32>,
    rocket_origin: Option<i32>,
    rocket_restart: Option<i32>,
    variant_refusals: crate::catalogue::Refusals,
}

/// A holder's numeric spending projection. Track versions change whenever
/// either retained inventory changes; scope refusals and continuity are separate.
#[derive(Clone, Copy, PartialEq, Eq)]
struct SpentInputs {
    identity: Identity,
    units: Option<(Identity, u64)>,
    costs: Option<(Identity, u64)>,
    reason: Option<ProductionSpendingUnknown>,
}
struct SpentHolder {
    inputs: SpentInputs,
    hour: i32,
    active: BTreeSet<u64>,
}

/// The latest complete variant read, used only at another complete read.
/// Production takes names from one frame, rather than a catalogue's history: equal
/// operands keep their strings but every selected name takes this read's date.
#[derive(Default)]
struct ProductionVariants {
    reads: VariantReads,
    catalogue: VariantCatalogue,
    classified: HashSet<u64>,
    inventory: [BTreeMap<String, VariantInventory>; 2],
    members: BTreeMap<u64, VariantMembers>,
    #[cfg(test)]
    inventory_visits: usize,
    #[cfg(test)]
    classifications: usize,
}

/// Inventory versions authenticate the keys used by presentation independently
/// of quantities, supplying dates and country names.
struct VariantInventory {
    versions: [Option<(Identity, u64)>; 2],
    members: BTreeMap<u64, [bool; SEEN]>,
}
#[derive(Default)]
struct VariantMembers {
    seen: [usize; SEEN],
    selected: usize,
}

impl ProductionVariants {
    fn inventory(
        &mut self,
        countries: Option<&crate::products::territory::CountryReads>,
        tracks: [&BTreeMap<String, TagHistory>; 2],
    ) {
        let _cpu = ::replay_frame::cpu::scope_phase("production:inventory_changes");
        for (source, tracks) in tracks.into_iter().enumerate() {
            let held = &mut self.inventory[source];
            let removed: Vec<_> = held
                .keys()
                .filter(|tag| {
                    !countries.is_some_and(|countries| countries.holders().contains_key(*tag))
                        || !tracks.contains_key(*tag)
                })
                .cloned()
                .collect();
            for tag in removed {
                let old = held.remove(&tag).expect("removed presentation holder");
                change_members(&mut self.members, &old.members, source, false);
            }
            for tag in countries
                .into_iter()
                .flat_map(|countries| countries.holders().keys())
            {
                let Some(track) = tracks.get(tag) else {
                    continue;
                };
                let versions = [track.taken, track.cost_taken];
                if versions.iter().all(Option::is_some)
                    && held.get(tag).is_some_and(|held| held.versions == versions)
                {
                    continue;
                }
                if let Some(old) = held.remove(tag) {
                    change_members(&mut self.members, &old.members, source, false);
                }
                let mut members: BTreeMap<u64, [bool; SEEN]> = BTreeMap::new();
                for (measure, &(variant, purpose)) in track
                    .current
                    .keys()
                    .map(|entry| (UNITS_SEEN, entry))
                    .chain(track.cost_current.keys().map(|entry| (IC_SEEN, entry)))
                {
                    #[cfg(test)]
                    {
                        self.inventory_visits += 1;
                    }
                    members.entry(variant).or_default()[measure + purpose.index()] = true;
                }
                change_members(&mut self.members, &members, source, true);
                held.insert(tag.clone(), VariantInventory { versions, members });
            }
        }
    }

    fn read(&mut self, frame: &DecodedFrame, refusals: &mut crate::catalogue::Refusals) {
        for (id, raw) in self.reads.build(frame, false) {
            for (_, reason) in &raw.refusals {
                refusals.note(reason, id.to_string());
            }
            self.classified.remove(&id);
            self.catalogue.versions.insert(
                id,
                vec![VariantVersion {
                    first_hour: frame.hour,
                    raw: Some(raw),
                }],
            );
        }
        self.catalogue
            .versions
            .retain(|id, _| self.reads.contains(*id));
        self.classified.retain(|id| self.reads.contains(*id));
    }
}

fn change_members(
    counts: &mut BTreeMap<u64, VariantMembers>,
    members: &BTreeMap<u64, [bool; SEEN]>,
    source: usize,
    add: bool,
) {
    for (&variant, seen) in members {
        let count = counts.entry(variant).or_default();
        for (count, seen) in count.seen.iter_mut().zip(seen) {
            if *seen {
                if add {
                    *count += 1;
                } else {
                    *count -= 1;
                }
            }
        }
        if source == 0 || variant != 0 {
            if add {
                count.selected += 1;
            } else {
                count.selected -= 1;
            }
        }
        if count.seen == [0; SEEN] {
            counts.remove(&variant);
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Classification {
    pub key: String,
    pub ordinal: Option<usize>,
    pub domain: Option<Domain>,
    pub name: Option<String>,
    pub names: Vec<StockVariantName>,
    /// Units, then IC, inventory, independently for each purpose: the flag
    /// at the measure's first index plus [`Purpose::index`].
    pub seen: [bool; SEEN],
    pub reason: Option<String>,
}

/// [`Classification::seen`]'s length and each measure's first index.
pub const SEEN: usize = 2 * Purpose::ALL.len();
pub const UNITS_SEEN: usize = 0;
pub const IC_SEEN: usize = Purpose::ALL.len();

impl Classification {
    /// Whether the measure starting at `inventory` saw the design, ship
    /// refits counted only with `refits`.
    fn seen_in(&self, inventory: usize, refits: bool) -> bool {
        Purpose::ALL.into_iter().any(|purpose| {
            (refits || purpose != Purpose::Refit) && self.seen[inventory + purpose.index()]
        })
    }
}

#[derive(Default, Deserialize, Serialize)]
struct RocketJoins {
    bounds: BTreeMap<String, RocketBound>,
    corrections: Vec<CoverageCorrection>,
}

#[derive(Deserialize, Serialize)]
struct RocketBound {
    slot: u32,
    country_id: u32,
    observation: Option<i32>,
    equipment_read: i32,
    rocket: Option<RocketBinding>,
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
struct RocketBinding {
    slot: u32,
    country_id: u32,
    observation: Option<i32>,
    read_hour: i32,
    segment: u64,
}
impl From<&SourceEndpoint> for RocketBinding {
    fn from(source: &SourceEndpoint) -> Self {
        Self {
            slot: source.slot,
            country_id: source.country_id,
            observation: source.observation,
            read_hour: source.read_hour,
            segment: source.segment,
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
struct CoverageCorrection {
    tag: String,
    slot: u32,
    country_id: u32,
    observation: Option<i32>,
    read_hour: i32,
    first_hour: i32,
    last_hour: i32,
}
impl CoverageCorrection {
    fn matches(&self, row: &CountryEndpoint) -> bool {
        row.tag == self.tag
            && row.equipment.as_ref().is_some_and(|source| {
                (
                    source.slot,
                    source.country_id,
                    source.observation,
                    source.read_hour,
                ) == (self.slot, self.country_id, self.observation, self.read_hour)
            })
    }
}

impl ProductionAccumulator {
    #[cfg(any(test, feature = "test-support"))]
    pub fn work(&self) -> crate::prepared::observer_tests::Work {
        let mut work = self.work;
        work.row_visits += self.result.rocket.work.row_visits;
        work.history_visits += self.result.rocket.work.history_visits;
        work
    }

    fn spending_scan(&self) -> ScannedProduced {
        let mut rocket = RocketProduced::default();
        rocket.reads = self.result.rocket.reads.clone();
        rocket.observations = [
            self.rocket_origin,
            self.rocket_restart,
            self.result.rocket.observations.last().copied(),
        ]
        .into_iter()
        .flatten()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
        rocket.unread = self.result.rocket.unread;
        rocket.unattributed = self.result.rocket.unattributed;
        rocket.inconsistent = self.result.rocket.inconsistent;
        ScannedProduced {
            reads: self.result.reads.clone(),
            observations: [
                self.equipment_origin,
                self.equipment_restart,
                self.result.observations.last().copied(),
            ]
            .into_iter()
            .flatten()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
            unlisted: self.unlisted.len(),
            unread_reason: self.result.unread_reason.clone(),
            spending_tainted: self.result.spending_tainted.clone(),
            rocket,
            ..Default::default()
        }
    }

    pub fn buffered_bytes(&self) -> usize {
        self.spent.bytes()
    }

    /// Carry only proofs current at the old frontier across certified unchanged
    /// hours. An already stale holder must still project at its next read.
    fn advance_spent_holders(&mut self, hour: i32) {
        for held in self
            .spent_holders
            .iter_mut()
            .flat_map(|holders| holders.values_mut())
        {
            if Some(held.hour) == self.last_frame {
                held.hour = hour;
            }
        }
    }

    fn observe_spent(&mut self, hour: i32) -> io::Result<()> {
        self.spent.advance(hour)?;
        let mut opened = BTreeSet::new();
        let mut closed = BTreeSet::new();
        for (rocket, tracks, holders, restart) in [
            (
                false,
                &self.result.tags,
                &mut self.first_equipment_holders,
                self.equipment_restart,
            ),
            (
                true,
                &self.result.rocket.tags,
                &mut self.first_rocket_holders,
                self.rocket_restart,
            ),
        ] {
            for tag in self
                .country_reads
                .as_ref()
                .into_iter()
                .flat_map(|reads| reads.holders().keys())
            {
                let Some(track) = tracks.get(tag) else {
                    continue;
                };
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.work.history_visits += 1;
                }
                let Some((identity, at)) = track.last.filter(|(_, at)| *at == hour) else {
                    continue;
                };
                holders
                    .entry(tag.clone())
                    .or_insert((identity.0, identity.1));
                let scope_reason = spending_reason(
                    &self.result,
                    holders,
                    tag,
                    (identity.0, identity.1),
                    restart,
                    rocket,
                    hour,
                );
                let starts = track.starts.last() == Some(&true) && track.hours.last() == Some(&at);
                let inputs = SpentInputs {
                    identity,
                    units: track.taken,
                    costs: track.cost_taken,
                    reason: scope_reason,
                };
                if !starts
                    && inputs.units.is_some()
                    && inputs.costs.is_some()
                    && let Some(held) = self.spent_holders[usize::from(rocket)].get_mut(tag)
                    && held.hour.checked_add(1) == Some(hour)
                    && held.inputs == inputs
                {
                    held.hour = hour;
                    continue;
                }
                let mut observed = BTreeSet::new();
                let mut variants: BTreeMap<u64, [Option<CostTotal>; Purpose::ALL.len()]> =
                    BTreeMap::new();
                for &(variant, purpose) in track.current.keys().chain(track.cost_current.keys()) {
                    #[cfg(test)]
                    {
                        self.spent_visits += 1;
                    }
                    variants.entry(variant).or_default()[purpose.index()] = Some(
                        track
                            .cost_current
                            .get(&(variant, purpose))
                            .copied()
                            .unwrap_or_default(),
                    );
                }
                for (variant, amounts) in variants {
                    for refits in if rocket {
                        &[true][..]
                    } else {
                        &[false, true][..]
                    } {
                        // Without refits: designs with input of another purpose.
                        if !*refits
                            && Purpose::ALL
                                .into_iter()
                                .filter(|&purpose| purpose != Purpose::Refit)
                                .all(|purpose| amounts[purpose.index()].is_none())
                        {
                            continue;
                        }
                        let token = serde_json::to_string(&(tag, rocket, refits, variant))?;
                        let next = next_key(
                            &self.spent_keys,
                            &self.classification_keys,
                            &self.unproven_keys,
                            &self.purpose_keys,
                        );
                        let key = *self.spent_keys.entry(token).or_insert(next);
                        let counted = Purpose::ALL
                            .into_iter()
                            .filter(|purpose| *refits || *purpose != Purpose::Refit)
                            .filter_map(|purpose| amounts[purpose.index()].as_ref());
                        let value = known_sum(scope_reason, counted)?;
                        if observe_known(
                            &mut self.spent,
                            &self.spent_active,
                            &mut observed,
                            (key, hour, value, starts),
                        ) {
                            #[cfg(any(test, feature = "test-support"))]
                            {
                                self.work.emitted += 1;
                            }
                        }
                    }
                    if rocket {
                        continue;
                    }
                    // Converted and unrecorded-purpose input, for designs
                    // that ever had it: months read each apart from new
                    // production in the series without refits.
                    for purpose in SERIES_PURPOSES {
                        let Some(total) = &amounts[purpose.index()] else {
                            continue;
                        };
                        let token = serde_json::to_string(&(tag, variant, purpose))?;
                        let next = next_key(
                            &self.spent_keys,
                            &self.classification_keys,
                            &self.unproven_keys,
                            &self.purpose_keys,
                        );
                        let key = *self.purpose_keys.entry(token).or_insert(next);
                        let value = known_sum(scope_reason, [total])?;
                        if observe_known(
                            &mut self.spent,
                            &self.spent_active,
                            &mut observed,
                            (key, hour, value, starts),
                        ) {
                            #[cfg(any(test, feature = "test-support"))]
                            {
                                self.work.emitted += 1;
                            }
                        }
                    }
                    // Input whose purpose is not recorded, per purpose, for
                    // entries that ever had it: ranges and months read its
                    // change over their own interval.
                    for (purpose, total) in Purpose::ALL.into_iter().filter_map(|purpose| {
                        amounts[purpose.index()].map(|total| (purpose, total))
                    }) {
                        let token = serde_json::to_string(&(tag, variant, purpose))?;
                        if total.unproven_raw == 0 && !self.unproven_keys.contains_key(&token) {
                            continue;
                        }
                        let next = next_key(
                            &self.spent_keys,
                            &self.classification_keys,
                            &self.unproven_keys,
                            &self.purpose_keys,
                        );
                        let key = *self.unproven_keys.entry(token).or_insert(next);
                        let value = scope_reason
                            .or(total.reason)
                            .is_none()
                            .then_some(total.unproven_raw);
                        if observe_known(
                            &mut self.spent,
                            &self.spent_active,
                            &mut observed,
                            (key, hour, value, starts),
                        ) {
                            #[cfg(any(test, feature = "test-support"))]
                            {
                                self.work.emitted += 1;
                            }
                        }
                    }
                }
                let held = SpentHolder {
                    inputs,
                    hour,
                    active: observed,
                };
                if let Some(previous) =
                    self.spent_holders[usize::from(rocket)].insert(tag.clone(), held)
                {
                    let active = &self.spent_holders[usize::from(rocket)][tag].active;
                    closed.extend(previous.active.difference(active).copied());
                }
                opened.extend(&self.spent_holders[usize::from(rocket)][tag].active);
            }
        }
        for holders in &mut self.spent_holders {
            let removed: Vec<_> = holders
                .iter()
                .filter(|(_, held)| held.hour != hour)
                .map(|(tag, _)| tag.clone())
                .collect();
            for tag in removed {
                closed.extend(holders.remove(&tag).unwrap().active);
            }
        }
        if !self.spent_holders_ready {
            // The saved active keys precede this runtime index. Reconcile them
            // once, including keys belonging to a holder gone at the first read.
            let observed: BTreeSet<_> = self
                .spent_holders
                .iter()
                .flat_map(|holders| holders.values())
                .flat_map(|held| held.active.iter().copied())
                .collect();
            closed.extend(self.spent_active.difference(&observed).copied());
            opened = observed;
            self.spent_holders_ready = true;
        }
        // Keep the full projection's call order: values first, then ending
        // keys in numeric order. Failed projections publish no endings.
        for key in closed {
            self.spent.observe(key, hour, None, false);
            self.spent_active.remove(&key);
            #[cfg(any(test, feature = "test-support"))]
            {
                self.work.emitted += 1;
            }
        }
        self.spent_active.extend(opened);
        Ok(())
    }

    pub fn new(format_version: u16, branch: u64, first_hour: i32) -> Self {
        Self {
            schema: 1,
            branch,
            spending_recorded: TableId::ProductionLineOutput
                .last_change()
                .appended
                .iter()
                .any(|field| field.name == "spent_raw" && format_version >= field.introduced),
            last_frame: first_hour.checked_sub(1),
            ..Self::default()
        }
    }
    pub fn gap(&mut self, first_hour: i32) {
        self.seal_holders();
        self.forget_names(first_hour);
        self.result.taint(self.observation, first_hour);
        self.result.spending_taint(
            self.observation,
            first_hour,
            ProductionSpendingUnknown::RecordingGap,
        );
        for tag in &self.result.active {
            self.result
                .tags
                .get_mut(tag)
                .expect("active equipment holder")
                .last = None;
        }
        self.result.active.clear();
        self.result.rocket.end_reads();
    }
    fn forget_names(&mut self, hour: i32) {
        self.variants.classified.clear();
        for kind in self.classifications.values_mut() {
            if kind.names.last().is_some_and(|name| !name.known) {
                continue;
            }
            kind.names = vec![StockVariantName {
                first_hour: hour,
                name: None,
                known: false,
                design: None,
            }];
        }
    }
    fn observe(
        &mut self,
        format: FormatVersion,
        decoded: &DecodedFrame,
        equipment_changed: bool,
        rocket_changed: bool,
    ) -> io::Result<()> {
        let Self {
            result,
            rows,
            live,
            totals,
            cost_totals,
            seen_production,
            unlisted,
            version,
            cost_version,
            observation,
            spending_recorded,
            last_frame,
            pending,
            country_reads,
            country_unnamed,
            #[cfg(any(test, feature = "test-support"))]
            work,
            ..
        } = self;
        let hour = decoded.hour;
        // Rocket rows retain every country/design total; they do not depend
        // on a registered line or on the equipment census being readable.
        if rocket_changed {
            result
                .rocket
                .observe_resolved(format, decoded, country_reads.as_ref())?;
        } else {
            result.rocket.advance(hour);
        }
        if let Some(last) = *last_frame
            && last.checked_add(1) != Some(hour)
        {
            result.spending_taint(
                *observation,
                last.saturating_add(1),
                ProductionSpendingUnknown::RecordingGap,
            );
        }
        *last_frame = Some(hour);

        if !equipment_changed {
            result.unnamed += pending.as_ref().map_or(0, |page| page.unnamed_per_hour);
            for tag in &result.active {
                let track = result.tags.get_mut(tag).expect("active equipment holder");
                if let Some((_, at)) = &mut track.last {
                    *at = hour;
                }
                if let Some(at) = track.last_hours.last_mut() {
                    *at = hour;
                }
            }
            return Ok(());
        }

        let read = slot_read(
            format,
            &decoded.metrics,
            TableId::ProductionLineOutput,
            decoded.hour,
        );
        // An hour the tables cannot tell extends nothing, so nothing continues across it.
        let (Ok(_), Ok(_), Ok(output)) = (
            decoded.complete::<tables::Countries>(),
            decoded.complete::<tables::CountryLinks>(),
            decoded.complete::<tables::ProductionLineOutput>(),
        ) else {
            if read {
                // The publication this read held, if any, is not ingested: whatever
                // lines ended in it are lost, for a producer that cannot be told.
                result.taint(*observation, hour);
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
            for tag in &result.active {
                result
                    .tags
                    .get_mut(tag)
                    .expect("active equipment holder")
                    .last = None;
            }
            result.active.clear();
            return Ok(());
        };
        let reads = country_reads
            .as_ref()
            .expect("resolved production countries");
        result.unnamed += *country_unnamed;
        let tagged: Vec<(String, Identity)> = reads
            .holders()
            .iter()
            .filter_map(|(tag, holder)| {
                holder.map(|(slot, id)| (tag.clone(), (slot, id, *observation)))
            })
            .collect();
        if !read {
            // Carried: the last read still holds, only for the same identity.
            let present: HashMap<&str, Identity> = tagged
                .iter()
                .map(|(tag, identity)| (tag.as_str(), *identity))
                .collect();
            result.active.retain(|tag| {
                let track = result.tags.get_mut(tag).expect("active equipment holder");
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
                continues.is_some()
            });
            return Ok(());
        }
        result.reads.push(hour);
        // A new observation restarts every count; totals never cross it.
        if let Some(since) = output.iter().map(|row| row.since_hour).min() {
            match result.observations.last() {
                None => result.observations.push(since),
                Some(current) if *current != since => {
                    result.observations.push(since);
                    *observation += 1;
                    rows.clear();
                    live.clear();
                    totals.clear();
                    cost_totals.clear();
                    seen_production.clear();
                }
                Some(_) => {}
            }
        }
        // Explicit suffix values also authenticate provisional source
        // fixtures; empty publications require the introduction ledger.
        *spending_recorded |= output
            .iter()
            .any(|row| !matches!(row.spent_raw, Recorded::NotRecorded));
        if !*spending_recorded {
            result.spending_taint(*observation, hour, ProductionSpendingUnknown::NotRecorded);
        }
        let observed: Vec<(String, Identity)> = tagged
            .into_iter()
            .map(|(tag, (slot, id, _))| (tag, (slot, id, *observation)))
            .collect();
        let by_slot: HashMap<u32, Identity> = observed
            .iter()
            .map(|(_, identity)| (identity.0, *identity))
            .collect();
        let mut presence_unknown = false;
        let census_complete = if let Ok(lines) = decoded.complete::<tables::ProductionLines>() {
            for line in lines {
                #[cfg(any(test, feature = "test-support"))]
                {
                    work.row_visits += 1;
                }
                if let Some(identity) = reads
                    .slots()
                    .get(&line.country)
                    .and_then(|slot| by_slot.get(slot))
                {
                    seen_production.insert(*identity);
                } else {
                    presence_unknown = true;
                }
            }
            true
        } else {
            false
        };
        let mut seen = HashSet::with_capacity(output.len());
        for row in output {
            #[cfg(any(test, feature = "test-support"))]
            {
                work.row_visits += 1;
            }
            let key = (row.line, row.ordinal);
            seen.insert(key);
            let purpose = Purpose::of(row);
            let proved = purpose != Purpose::Unknown;
            if !proved {
                unlisted.insert(row.line);
            }
            let producer = reads
                .slots()
                .get(&row.country)
                .and_then(|slot| by_slot.get(slot))
                .copied();
            let state = rows.entry(key).or_insert_with(|| {
                if producer.is_none() {
                    result.unattributed += 1;
                }
                RowState {
                    identity: producer,
                    entry: (row.variant, purpose),
                    proved,
                    units: 0,
                    exact: true,
                    ended: false,
                    spent_raw: 0,
                }
            });
            // A producer the links could not tell before may be told now, while
            // nothing was delivered yet; units of a producer never told are lost to
            // some country that cannot be named, so no total is known after them.
            if state.identity.is_none() && state.units == 0 && state.exact {
                state.identity = producer;
            }
            if let Some(identity) = state.identity {
                seen_production.insert(identity);
            } else {
                presence_unknown = true;
            }
            if state.identity.is_none() && (row.units != 0 || !row.exact) {
                result.taint(*observation, hour);
            }
            if state.identity.is_none() && !matches!(row.spent_raw, Recorded::Value(Some(0))) {
                result.spending_taint(
                    *observation,
                    hour,
                    ProductionSpendingUnknown::SpendingUnavailable,
                );
            }
            // Not exact once: a tick is left out of every later count.
            let exact = state.exact && row.exact;
            if state.exact && !exact {
                result.inexact += 1;
            }
            // The observer authenticates the applied input's tuple. Census
            // cost, delivered units and their exactness do not reprice it.
            let (delta, spending_reason) = match row.spent_raw {
                Recorded::Value(Some(value)) => {
                    (i128::from(value) - i128::from(state.spent_raw), None)
                }
                Recorded::Value(None) => (0, Some(ProductionSpendingUnknown::SpendingUnavailable)),
                Recorded::NotRecorded => (0, Some(ProductionSpendingUnknown::NotRecorded)),
            };
            apply_cost(cost_totals, cost_version, state, delta, spending_reason)?;
            if let Recorded::Value(Some(value)) = row.spent_raw {
                state.spent_raw = value;
            }
            apply(totals, version, state, row.units, exact);
            state.units = row.units;
            state.exact = exact;
            state.ended |= row.ended;
        }
        // A row gone without its ended publication lost its final count.
        for key in live.iter().filter(|key| !seen.contains(*key)) {
            if let Some(state) = rows.get_mut(key) {
                result.lost += usize::from(state.exact);
                let units = state.units;
                apply_cost(
                    cost_totals,
                    cost_version,
                    state,
                    0,
                    Some(ProductionSpendingUnknown::LostOutput),
                )?;
                apply(totals, version, state, units, false);
                state.exact = false;
            }
        }
        *live = seen
            .into_iter()
            .filter(|key| rows.get(key).is_some_and(|state| !state.ended))
            .collect();
        // Each tag's sample: its country's totals at this read.
        let present: HashSet<&str> = observed.iter().map(|(tag, _)| tag.as_str()).collect();
        for tag in result
            .active
            .iter()
            .filter(|tag| !present.contains(tag.as_str()))
        {
            result
                .tags
                .get_mut(tag)
                .expect("active equipment holder")
                .last = None;
        }
        result.active = observed.iter().map(|(tag, _)| tag.clone()).collect();
        for (tag, identity) in observed {
            let track = result.tags.entry(tag).or_default();
            let starts = !track
                .last
                .is_some_and(|(seen, at)| seen == identity && at.checked_add(1) == Some(hour));
            let sample = u32::try_from(track.hours.len()).map_err(|_| {
                io::Error::other("The production sample index exceeds its integer range")
            })?;
            track.hours.push(hour);
            track.last_hours.push(hour);
            track.starts.push(starts);
            track.identities.push(identity);
            track.presence.push(if seen_production.contains(&identity) {
                Some(true)
            } else if census_complete && !presence_unknown {
                Some(false)
            } else {
                None
            });
            track.last = Some((identity, hour));
            let (cost_changed, cost_entries) = cost_totals
                .get(&identity)
                .map_or((0, None), |(changed, entries)| (*changed, Some(entries)));
            if track.cost_taken != Some((identity, cost_changed)) {
                track.cost_taken = Some((identity, cost_changed));
                let empty = BTreeMap::new();
                let entries = cost_entries.unwrap_or(&empty);
                let keys: BTreeSet<Entry> = entries
                    .keys()
                    .chain(track.cost_current.keys())
                    .copied()
                    .collect();
                for entry in keys {
                    let now = entries.get(&entry).copied().unwrap_or_default();
                    if track.cost_current.get(&entry).copied().unwrap_or_default() != now {
                        track
                            .cost_changes
                            .entry(entry)
                            .or_default()
                            .push((sample, now));
                    }
                }
                track.cost_current = entries.clone();
            }
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
                    track.changes.entry(entry).or_default().push((sample, now));
                }
            }
            track.current = entries.clone();
        }
        Ok(())
    }
}

/// Purposes with their own series beside the spent series: new production is
/// the series without refits less theirs.
pub(crate) const SERIES_PURPOSES: [Purpose; 2] = [Purpose::Converted, Purpose::Unknown];

/// The next series key: spent, classification, unproven and purpose keys
/// share one numbering.
fn next_key(
    spent: &BTreeMap<String, u64>,
    classifications: &BTreeMap<u64, u64>,
    unproven: &BTreeMap<String, u64>,
    purposes: &BTreeMap<String, u64>,
) -> u64 {
    (spent.len() + classifications.len() + unproven.len() + purposes.len()) as u64
}

/// The exact sum of `totals`, unknown when the scope or any of them has a
/// reason.
fn known_sum<'a>(
    scope: Option<ProductionSpendingUnknown>,
    totals: impl IntoIterator<Item = &'a CostTotal>,
) -> io::Result<Option<i128>> {
    let mut amount = 0i128;
    let mut reason = scope;
    for total in totals {
        amount = amount
            .checked_add(total.raw)
            .ok_or_else(|| io::Error::other("Applied IC exceeds its exact integer range"))?;
        reason = reason.or(total.reason);
    }
    Ok(reason.is_none().then_some(amount))
}

/// Observes a known value of a holder's key when it is new or changed;
/// true when it was observed. A known value keeps the key active.
fn observe_known(
    spent: &mut crate::prepared::series::Accumulator,
    active: &BTreeSet<u64>,
    observed: &mut BTreeSet<u64>,
    (key, hour, value, starts): (u64, i32, Option<i128>, bool),
) -> bool {
    if value.is_none() {
        return false;
    }
    observed.insert(key);
    if starts
        || !active.contains(&key)
        || spent
            .state(key)
            .and_then(|state| state.last)
            .is_none_or(|run| Some(run.value) != value)
    {
        spent.observe(key, hour, value, starts);
        return true;
    }
    false
}

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
    total.units += i128::from(units) - i128::from(state.units);
    total.inexact = total.inexact + u32::from(!exact) - u32::from(!state.exact);
}

fn apply_cost(
    totals: &mut HashMap<Identity, (u64, BTreeMap<Entry, CostTotal>)>,
    version: &mut u64,
    state: &RowState,
    delta: i128,
    reason: Option<ProductionSpendingUnknown>,
) -> io::Result<()> {
    let Some(identity) = state.identity else {
        return Ok(());
    };
    if delta == 0 && reason.is_none() {
        return Ok(());
    }
    *version += 1;
    let (changed, entries) = totals.entry(identity).or_default();
    *changed = *version;
    let total = entries.entry(state.entry).or_default();
    total.reason = total.reason.or(reason);
    if total.reason.is_none() {
        total.raw = total
            .raw
            .checked_add(delta)
            .ok_or_else(|| io::Error::other("Applied IC exceeds its exact integer range"))?;
        // Magnitudes, so corrections between lines never cancel the evidence.
        if !state.proved {
            total.unproven_raw = delta
                .checked_abs()
                .and_then(|delta| total.unproven_raw.checked_add(delta))
                .ok_or_else(|| io::Error::other("Applied IC exceeds its exact integer range"))?;
        }
    }
    Ok(())
}

mod versioned_pairs {
    use super::*;
    type VersionedPairs<V> = HashMap<Identity, (u64, BTreeMap<Entry, V>)>;
    pub fn serialize<S: serde::Serializer, V: Serialize>(
        map: &VersionedPairs<V>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(map_pairs::ordered_entries(map).into_iter().map(
            |(identity, (version, values))| (identity, version, values.iter().collect::<Vec<_>>()),
        ))
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>, V: Deserialize<'de>>(
        deserializer: D,
    ) -> Result<VersionedPairs<V>, D::Error> {
        Ok(
            Vec::<(Identity, u64, Vec<(Entry, V)>)>::deserialize(deserializer)?
                .into_iter()
                .map(|(identity, version, values)| {
                    (identity, (version, values.into_iter().collect()))
                })
                .collect(),
        )
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Amount {
    pub variant: String,
    pub purpose: Purpose,
    /// Exact signed cumulative accounting quantity, with no frontend ceiling.
    pub units: String,
    pub units_exact: bool,
    pub spent: String,
    pub spending_reason: Option<ProductionSpendingUnknown>,
    /// The magnitudes of the input of production ticks whose purpose was not
    /// recorded, summed: dated evidence that grows and never cancels.
    pub unproven: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SourceEndpoint {
    pub slot: u32,
    pub country_id: u32,
    pub observation: Option<i32>,
    /// Continuity across table/read gaps and holder replacements.
    pub segment: u64,
    pub read_hour: i32,
    pub units_known: bool,
    pub spending_known: bool,
    /// Complete source census absence stays distinct from unavailable presence.
    pub present: Option<bool>,
    /// Campaign spending coverage, independent of delivery's restart semantics.
    pub campaign_spending_reason: Option<ProductionSpendingUnknown>,
    pub amounts: Vec<Amount>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct CountryEndpoint {
    pub tag: String,
    pub equipment: Option<SourceEndpoint>,
    pub rocket: Option<SourceEndpoint>,
    /// Whether the equipment supplying read still has its independent Rocket read.
    pub rocket_joined: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct EndpointPage {
    pub schema: u32,
    pub branch_id: String,
    pub first_hour: i32,
    pub last_hour: i32,
    pub equipment_read: Option<i32>,
    pub rocket_read: Option<i32>,
    pub equipment_origin: Option<i32>,
    pub equipment_restart: Option<i32>,
    pub rocket_origin: Option<i32>,
    pub rocket_restart: Option<i32>,
    pub countries: Vec<CountryEndpoint>,
    pub classifications: BTreeMap<u64, Classification>,
    pub warnings: Vec<String>,
    /// Accepted country-hours before this page; unavailable hours never accrue.
    pub unnamed_before: usize,
    pub unnamed_per_hour: usize,
    /// Position in the existing recording-warning order, before Rocket warnings.
    pub unnamed_warning_index: usize,
}

impl EndpointPage {
    fn unnamed_at(&self, hour: i32) -> usize {
        let hours = (i64::from(hour.min(self.last_hour)) - i64::from(self.first_hour) + 1).max(0);
        self.unnamed_before
            + usize::try_from(hours).expect("accepted production span") * self.unnamed_per_hour
    }

    fn warnings_at(&self, hour: i32) -> Vec<String> {
        let mut warnings = self.warnings.clone();
        if let Some(warning) =
            crate::products::equipment_produced::unnamed_recording_warning(self.unnamed_at(hour))
        {
            warnings.insert(self.unnamed_warning_index, warning);
        }
        warnings
    }

    fn render_warnings(&mut self, hour: i32) {
        self.warnings = self.warnings_at(hour);
        self.unnamed_before = 0;
        self.unnamed_per_hour = 0;
    }
}

fn compact(track: &mut TagHistory) {
    for values in [&mut track.hours, &mut track.last_hours] {
        if let Some(last) = values.last().copied() {
            values.clear();
            values.push(last);
        }
    }
    if let Some(last) = track.starts.last().copied() {
        track.starts.clear();
        track.starts.push(last);
    }
    if let Some(last) = track.identities.last().copied() {
        track.identities.clear();
        track.identities.push(last);
    }
    if let Some(last) = track.presence.last().copied() {
        track.presence.clear();
        track.presence.push(last);
    }
    track.changes.clear();
    track.cost_changes.clear();
}

fn spending_reason(
    current: &Current,
    first: &BTreeMap<String, (u32, u32)>,
    tag: &str,
    identity: (u32, u32),
    restart: Option<i32>,
    rocket: bool,
    hour: i32,
) -> Option<ProductionSpendingUnknown> {
    (if rocket {
        None
    } else {
        current
            .spending_tainted
            .values()
            .min_by_key(|(at, _)| *at)
            .filter(|(at, _)| hour >= *at)
            .map(|(_, reason)| *reason)
    })
    .or_else(|| {
        restart
            .filter(|at| hour >= *at)
            .map(|_| ProductionSpendingUnknown::ObservationRestarted)
    })
    .or_else(|| {
        first
            .get(tag)
            .filter(|first| **first != identity)
            .map(|_| ProductionSpendingUnknown::CountryChanged)
    })
}

fn snapshot(
    track: &TagHistory,
    hour: i32,
    origin: Option<i32>,
    segments: &mut BTreeMap<String, u64>,
    tag: &str,
    units_known: bool,
    spending_known: bool,
) -> io::Result<Option<SourceEndpoint>> {
    let Some((identity, last)) = track.last else {
        return Ok(None);
    };
    if last != hour {
        return Ok(None);
    }
    let read_hour = *track.hours.last().expect("live production read");
    let segment = segments.entry(tag.to_owned()).or_default();
    if read_hour == hour && track.starts.last() == Some(&true) {
        *segment = segment.checked_add(1).ok_or_else(|| {
            io::Error::other("Production continuity count exceeds its integer range")
        })?;
    }
    let entries: BTreeSet<Entry> = track
        .current
        .keys()
        .chain(track.cost_current.keys())
        .copied()
        .collect();
    let amounts = entries
        .into_iter()
        .map(|entry| {
            let units = track.current.get(&entry).copied().unwrap_or_default();
            let spending = track.cost_current.get(&entry).copied().unwrap_or_default();
            Amount {
                variant: entry.0.to_string(),
                purpose: entry.1,
                units: units.units.to_string(),
                units_exact: units.inexact == 0,
                spent: spending.raw.to_string(),
                spending_reason: spending.reason,
                unproven: spending.unproven_raw.to_string(),
            }
        })
        .collect();
    Ok(Some(SourceEndpoint {
        slot: identity.0,
        country_id: identity.1,
        observation: origin,
        segment: *segment,
        read_hour,
        units_known,
        spending_known,
        present: track.presence.last().copied().flatten(),
        campaign_spending_reason: None,
        amounts,
    }))
}

impl ProductionAccumulator {
    pub fn ingest_changed(
        &mut self,
        format: FormatVersion,
        frame: &DecodedFrame,
        named: Named<'_>,
        cancelled: &AtomicBool,
        changes: &crate::prepared::frame_changes::FrameChanges,
    ) -> io::Result<Option<EndpointPage>> {
        let _cpu = ::replay_frame::cpu::scope_phase("production:observe_apply");
        check_cancel(cancelled)?;
        self.spent.advance(frame.hour)?;
        let consecutive = self.last_frame.and_then(|hour| hour.checked_add(1)) == Some(frame.hour);
        let countries_changed = if changes.rows_changed[TableId::Countries.index()]
            || changes.rows_changed[TableId::CountryLinks.index()]
            || !consecutive
        {
            let rows = frame.complete::<tables::Countries>().ok();
            let same = rows.is_some_and(|rows| {
                self.country_reads
                    .as_ref()
                    .is_some_and(|seen| seen.matches_changes(frame, rows, Some(changes)))
            });
            if !same {
                self.country_reads = rows.map(|rows| {
                    if !changes.rows_changed[TableId::CountryLinks.index()]
                        && let Some(reads) = &self.country_reads
                    {
                        crate::products::territory::CountryReads::from_slots(
                            frame,
                            rows,
                            std::sync::Arc::clone(reads.slots()),
                        )
                    } else {
                        crate::products::territory::CountryReads::new(frame, rows)
                    }
                });
                self.country_unnamed =
                    rows.zip(self.country_reads.as_ref())
                        .map_or(0, |(rows, reads)| {
                            crate::metadata::tagged_countries_indexed(rows, reads.slots())
                                .into_iter()
                                .filter(|(tag, holder)| {
                                    matches!(holder, TaggedCountry::Found(..))
                                        && std::str::from_utf8(tag).is_err()
                                })
                                .count()
                        });
            }
            !same
        } else {
            false
        };
        let changed = self.schema != 1
            || !consecutive
            || countries_changed
            || TABLES
                .iter()
                .filter(|&&table| {
                    !matches!(
                        table,
                        TableId::Campaign | TableId::Countries | TableId::CountryLinks
                    )
                })
                .any(|table| changes.rows_changed[table.index()])
            || [
                TableId::ProductionLineOutput,
                TableId::RocketProductionOutput,
                TableId::VariantDefinitions,
            ]
            .iter()
            .any(|table| changes.slot_read[table.index()]);
        if !changed && let Some(pending) = &mut self.pending {
            pending.last_hour = frame.hour;
            self.advance_spent_holders(frame.hour);
            self.last_frame = Some(frame.hour);
            return Ok(None);
        }
        let equipment_changed = !consecutive
            || countries_changed
            || self.pending.is_none()
            || changes.rows_changed[TableId::ProductionLineOutput.index()]
            || changes.slot_read[TableId::ProductionLineOutput.index()];
        let rocket_changed = !consecutive
            || countries_changed
            || changes.rows_changed[TableId::RocketProductionOutput.index()]
            || changes.slot_read[TableId::RocketProductionOutput.index()];
        self.ingest_frame(
            format,
            frame,
            named,
            cancelled,
            equipment_changed,
            rocket_changed,
        )
    }
    /// Hours `first..=last` that follow the last frame, at which no table
    /// production reads changed (country keys and links included) and no
    /// output or variant read is dated: ingesting each of them extends the
    /// open endpoint page and the spent series to it, carrying holder cache
    /// proofs without visiting inventories. False without an open page, whose
    /// hours only ingestion opens.
    pub fn advance_quiet(&mut self, first: i32, last: i32) -> io::Result<bool> {
        if self.schema != 1
            || self.pending.is_none()
            || self.last_frame.and_then(|hour| hour.checked_add(1)) != Some(first)
        {
            return Ok(false);
        }
        self.spent.advance_span(first, last)?;
        if let Some(pending) = &mut self.pending {
            pending.last_hour = last;
        }
        self.advance_spent_holders(last);
        self.last_frame = Some(last);
        Ok(true)
    }
    fn seal_holders(&mut self) {
        let Some(hour) = self.last_frame else {
            return;
        };
        let Some(page) = &self.pending else {
            return;
        };
        self.result.unnamed = page.unnamed_at(hour);
        self.warnings = page.warnings_at(hour);
        for row in &page.countries {
            for (source, tracks) in [
                (row.equipment.as_ref(), &mut self.result.tags),
                (row.rocket.as_ref(), &mut self.result.rocket.tags),
            ] {
                if source.is_some()
                    && let Some(track) = tracks.get_mut(&row.tag)
                    && let Some((_, at)) = &mut track.last
                {
                    *at = hour;
                    if let Some(last) = track.last_hours.last_mut() {
                        *last = hour;
                    }
                    #[cfg(any(test, feature = "test-support"))]
                    {
                        self.work.history_visits += 1;
                    }
                }
            }
        }
    }
    /// The caller validates campaign membership and supplies frames in recorded order.
    #[cfg(any(test, feature = "test-support"))]
    pub fn ingest(
        &mut self,
        format: FormatVersion,
        frame: &DecodedFrame,
        named: Named<'_>,
        cancelled: &AtomicBool,
    ) -> io::Result<Option<EndpointPage>> {
        let changes = crate::prepared::frame_changes::FrameChanges {
            rows_changed: [true; replay_data::rows::TABLE_COUNT],
            table_read: [true; replay_data::rows::TABLE_COUNT],
            slot_read: [true; replay_data::rows::TABLE_COUNT],
            updates: None,
        };
        self.ingest_changed(format, frame, named, cancelled, &changes)
    }

    fn ingest_frame(
        &mut self,
        format: FormatVersion,
        frame: &DecodedFrame,
        named: Named<'_>,
        cancelled: &AtomicBool,
        equipment_changed: bool,
        rocket_changed: bool,
    ) -> io::Result<Option<EndpointPage>> {
        let cpu = ::replay_frame::cpu::scope_phase("production:settle_reconcile");
        check_cancel(cancelled)?;
        if self.schema != 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Prepared production state schema mismatch",
            ));
        }
        self.seal_holders();
        if let Some(last) = self.last_frame
            && last.checked_add(1) != Some(frame.hour)
        {
            self.forget_names(last.saturating_add(1));
        }
        let names_read = equipment::design_changed(frame)
            || slot_read(
                format,
                &frame.metrics,
                TableId::VariantDefinitions,
                frame.hour,
            );
        if frame.complete::<tables::VariantDefinitions>().is_err()
            || names_read
                && (frame.complete::<tables::Countries>().is_err()
                    || frame.complete::<tables::CountryLinks>().is_err()
                    || frame.complete::<tables::LexicalTokens>().is_err())
        {
            self.forget_names(frame.hour);
        }
        let diagnostics_before = self.diagnostic_counts();
        let consecutive = self.last_frame.and_then(|last| last.checked_add(1)) == Some(frame.hour);
        drop(cpu);
        let cpu = ::replay_frame::cpu::scope_phase("production:observe_apply");
        self.observe(format, frame, equipment_changed, rocket_changed)?;
        for (values, origin, restart) in [
            (
                &self.result.observations,
                &mut self.equipment_origin,
                &mut self.equipment_restart,
            ),
            (
                &self.result.rocket.observations,
                &mut self.rocket_origin,
                &mut self.rocket_restart,
            ),
        ] {
            if let Some(&read) = values.last() {
                if let Some(first) = *origin {
                    if first != read && restart.is_none() {
                        *restart = Some(read);
                    }
                } else {
                    *origin = Some(read);
                }
            }
        }
        drop(cpu);
        let cpu = ::replay_frame::cpu::scope_phase("production:spending_projection_series");
        self.observe_spent(frame.hour)?;
        drop(cpu);
        let numeric_read = slot_read(
            format,
            &frame.metrics,
            TableId::ProductionLineOutput,
            frame.hour,
        ) || slot_read(
            format,
            &frame.metrics,
            TableId::RocketProductionOutput,
            frame.hour,
        );
        if consecutive
            && !numeric_read
            && !names_read
            && diagnostics_before == self.diagnostic_counts()
            && self.pending.as_ref().is_some_and(|page| {
                page.unnamed_per_hour == 0 && self.same_holders(page, frame.hour)
            })
        {
            self.pending.as_mut().expect("unchanged endpoint").last_hour = frame.hour;
            check_cancel(cancelled)?;
            return Ok(None);
        }
        let cpu = ::replay_frame::cpu::scope_phase("production:presentation");
        // A fresh hourly catalogue is empty outside a complete variant read.
        // At a complete read only changed operands need building again.
        let complete_names = VariantCatalogue::hour_of(format, frame) == VariantHour::Read;
        if complete_names {
            self.variants.read(frame, &mut self.variant_refusals);
        }
        self.variants.inventory(
            self.country_reads.as_ref(),
            [&self.result.tags, &self.result.rocket.tags],
        );
        let empty = VariantCatalogue::default();
        let catalogue = if complete_names {
            &self.variants.catalogue
        } else {
            &empty
        };
        for (&variant, members) in &self.variants.members {
            if members.selected == 0 {
                continue;
            }
            let token = catalogue.token(variant);
            if let Some(kind) = self.classifications.get_mut(&variant) {
                if token.is_empty() {
                    continue;
                }
                if self.variants.classified.contains(&variant) {
                    for name in &mut kind.names {
                        name.first_hour = frame.hour;
                    }
                    continue;
                }
            }
            #[cfg(test)]
            {
                self.variants.classifications += 1;
            }
            let (mut types, reasons, _) = variant_types_for(catalogue, [variant], named);
            let kind = types
                .remove(&variant)
                .expect("classified production variant");
            let mut names = catalogue.names(variant, named.texts);
            for name in &mut names {
                name.first_hour = frame.hour;
            }
            self.classifications.insert(
                variant,
                Classification {
                    reason: reasons.first().cloned(),
                    name: named.type_name(&kind.key),
                    names,
                    seen: self
                        .classifications
                        .get(&variant)
                        .map_or([false; SEEN], |kind| kind.seen),
                    key: kind.key,
                    ordinal: kind.ordinal,
                    domain: kind.domain,
                },
            );
            if !token.is_empty() {
                self.variants.classified.insert(variant);
            }
        }
        for (&variant, members) in &self.variants.members {
            if let Some(kind) = self.classifications.get_mut(&variant) {
                for (seen, count) in kind.seen.iter_mut().zip(members.seen) {
                    *seen |= count != 0;
                }
            }
        }
        if names_read {
            for (&variant, kind) in &mut self.classifications {
                if !catalogue.versions.contains_key(&variant) {
                    kind.names = vec![StockVariantName {
                        first_hour: frame.hour,
                        name: None,
                        known: false,
                        design: None,
                    }];
                }
            }
        }
        drop(cpu);
        let _cpu = ::replay_frame::cpu::scope_phase("production:endpoint_projection");
        let mut countries = Vec::new();
        let tags: BTreeSet<_> = self
            .country_reads
            .as_ref()
            .into_iter()
            .flat_map(|reads| reads.holders().keys())
            .cloned()
            .collect();
        for tag in tags {
            check_cancel(cancelled)?;
            let track = self.result.tags.get(&tag);
            let identity = track.and_then(|track| track.identities.last().copied());
            let units_known = identity.is_some_and(|identity| {
                self.result
                    .tainted
                    .get(&identity.2)
                    .is_none_or(|from| frame.hour < *from)
            });
            let spending_known = identity.is_some_and(|identity| {
                self.result
                    .spending_tainted
                    .get(&identity.2)
                    .is_none_or(|(from, _)| frame.hour < *from)
            });
            let mut equipment = track
                .map(|track| {
                    snapshot(
                        track,
                        frame.hour,
                        self.result.observations.last().copied(),
                        &mut self.segments,
                        &tag,
                        units_known,
                        spending_known,
                    )
                })
                .transpose()?
                .flatten();
            if let Some(source) = &mut equipment {
                source.campaign_spending_reason = spending_reason(
                    &self.result,
                    &self.first_equipment_holders,
                    &tag,
                    (source.slot, source.country_id),
                    self.equipment_restart,
                    false,
                    frame.hour,
                );
            }
            let mut rocket = self
                .result
                .rocket
                .tags
                .get(&tag)
                .map(|track| {
                    snapshot(
                        track,
                        frame.hour,
                        self.result.rocket.observations.last().copied(),
                        &mut self.rocket_segments,
                        &tag,
                        true,
                        true,
                    )
                })
                .transpose()?
                .flatten();
            if let Some(source) = &mut rocket {
                source.campaign_spending_reason = spending_reason(
                    &self.result,
                    &self.first_rocket_holders,
                    &tag,
                    (source.slot, source.country_id),
                    self.rocket_restart,
                    true,
                    frame.hour,
                );
            }
            let rocket_joined = if let Some(equipment_source) = equipment.as_ref() {
                self.joins.bind_rocket(
                    &tag,
                    equipment_source,
                    rocket.as_ref(),
                    frame.hour,
                    &mut self.pending,
                )
            } else {
                false
            };
            if equipment.is_some() || rocket.is_some() {
                countries.push(CountryEndpoint {
                    tag,
                    equipment,
                    rocket,
                    rocket_joined,
                });
            }
        }
        let present: BTreeSet<&str> = countries.iter().map(|row| row.tag.as_str()).collect();
        self.joins
            .bounds
            .retain(|tag, _| present.contains(tag.as_str()));
        let mut rocket_diagnostics = RocketProduced::default();
        rocket_diagnostics.reads = self.result.rocket.reads.clone();
        rocket_diagnostics.unread = self.result.rocket.unread;
        rocket_diagnostics.unread_reason = self.result.rocket.unread_reason.clone();
        rocket_diagnostics.unattributed = self.result.rocket.unattributed;
        rocket_diagnostics.inconsistent = self.result.rocket.inconsistent;
        let diagnostics = ScannedProduced {
            reads: self.result.reads.clone(),
            observations: self.result.observations.clone(),
            unattributed: self.result.unattributed,
            inexact: self.result.inexact,
            lost: self.result.lost,
            unlisted: self.unlisted.len(),
            unread: self.result.unread,
            unread_reason: self.result.unread_reason.clone(),
            unnamed: 0,
            missed: self.result.missed,
            tainted: self.result.tainted.clone(),
            rocket: rocket_diagnostics,
            ..Default::default()
        };
        let (mut warnings, unnamed_warning_index) =
            crate::products::equipment_produced::recording_warnings_with_unnamed(&diagnostics);
        let variant_refusals = self
            .variant_refusals
            .diagnostic_lines("equipment variants with unknown text/design operands");
        let unnamed_warning_index = unnamed_warning_index + variant_refusals.len();
        warnings.splice(..0, variant_refusals);
        if self.observation > 0 {
            warnings.push(format!("The production output observation restarted {} times in this branch; running totals start again at each.", self.observation));
        }
        let mut page = EndpointPage {
            schema: 1,
            branch_id: self.branch.to_string(),
            first_hour: frame.hour,
            last_hour: frame.hour,
            equipment_read: self.result.reads.last().copied(),
            rocket_read: self.result.rocket.reads.last().copied(),
            equipment_origin: self.equipment_origin,
            equipment_restart: self.equipment_restart,
            rocket_origin: self.rocket_origin,
            rocket_restart: self.rocket_restart,
            countries,
            classifications: self.classifications.clone(),
            warnings,
            unnamed_before: diagnostics_before[4],
            unnamed_per_hour: self.result.unnamed - diagnostics_before[4],
            unnamed_warning_index,
        };
        let completed = match self.pending.take() {
            Some(mut previous) => {
                page.first_hour = previous.first_hour;
                let last = previous.last_hour;
                if page.unnamed_per_hour == previous.unnamed_per_hour
                    && page.unnamed_before == previous.unnamed_at(last)
                {
                    page.unnamed_before = previous.unnamed_before;
                }
                previous.last_hour = frame.hour;
                if last.checked_add(1) == Some(frame.hour) && previous == page {
                    self.pending = Some(page);
                    None
                } else {
                    previous.last_hour = last;
                    page.first_hour = frame.hour;
                    page.unnamed_before = diagnostics_before[4];
                    self.pending = Some(page);
                    Some(previous)
                }
            }
            None => {
                self.pending = Some(page);
                None
            }
        };
        let _pruning = ::replay_frame::cpu::scope_phase("production:pruning");
        // Only current supplying reads and retained row counters survive a frame.
        for tag in &self.result.active {
            compact(
                self.result
                    .tags
                    .get_mut(tag)
                    .expect("active equipment holder"),
            );
        }
        for tag in &self.result.rocket.active {
            compact(
                self.result
                    .rocket
                    .tags
                    .get_mut(tag)
                    .expect("active Rocket holder"),
            );
        }
        retain_last(&mut self.result.reads);
        retain_last(&mut self.result.rocket.reads);
        retain_last(&mut self.result.observations);
        retain_last(&mut self.result.rocket.observations);
        self.seal_holders();
        check_cancel(cancelled)?;
        Ok(completed)
    }
    /// Publish the accepted prefix before saving this accumulator. Appends begin a new page.
    pub fn flush(&mut self) -> Option<EndpointPage> {
        self.seal_holders();
        self.pending.take()
    }

    fn diagnostic_counts(&self) -> [usize; 9] {
        [
            self.result.unattributed,
            self.result.inexact,
            self.result.lost,
            self.result.unread,
            self.result.unnamed,
            self.result.missed,
            self.result.rocket.unread,
            self.result.rocket.unattributed,
            self.result.rocket.inconsistent,
        ]
    }
    fn same_holders(&self, page: &EndpointPage, hour: i32) -> bool {
        if page.classifications != self.classifications {
            return false;
        }
        let present = self
            .result
            .active
            .iter()
            .chain(&self.result.rocket.active)
            .collect::<BTreeSet<_>>()
            .len();
        page.countries.len() == present
            && page.countries.iter().all(|row| {
                source_holds(row.equipment.as_ref(), self.result.tags.get(&row.tag), hour)
                    && source_holds(
                        row.rocket.as_ref(),
                        self.result.rocket.tags.get(&row.tag),
                        hour,
                    )
            })
    }
}

impl RocketJoins {
    fn bind_rocket(
        &mut self,
        tag: &str,
        equipment: &SourceEndpoint,
        observed: Option<&SourceEndpoint>,
        hour: i32,
        pending: &mut Option<EndpointPage>,
    ) -> bool {
        let observed = observed.map(RocketBinding::from);
        let bound = self
            .bounds
            .entry(tag.to_owned())
            .or_insert_with(|| RocketBound {
                slot: equipment.slot,
                country_id: equipment.country_id,
                observation: equipment.observation,
                equipment_read: equipment.read_hour,
                rocket: observed.clone(),
            });
        let identity = (
            equipment.slot,
            equipment.country_id,
            equipment.observation,
            equipment.read_hour,
        );
        if (
            bound.slot,
            bound.country_id,
            bound.observation,
            bound.equipment_read,
        ) != identity
        {
            *bound = RocketBound {
                slot: equipment.slot,
                country_id: equipment.country_id,
                observation: equipment.observation,
                equipment_read: equipment.read_hour,
                rocket: observed,
            };
        } else if bound.rocket.as_ref().is_some_and(|supplied| {
            observed.as_ref().is_none_or(|current| {
                (
                    current.slot,
                    current.country_id,
                    current.observation,
                    current.read_hour,
                    current.segment,
                ) != (
                    supplied.slot,
                    supplied.country_id,
                    supplied.observation,
                    supplied.read_hour,
                    supplied.segment,
                )
            })
        }) {
            bound.rocket = None;
            let correction = CoverageCorrection {
                tag: tag.to_owned(),
                slot: equipment.slot,
                country_id: equipment.country_id,
                observation: equipment.observation,
                read_hour: equipment.read_hour,
                first_hour: equipment.read_hour,
                last_hour: hour,
            };
            if let Some(page) = pending {
                for row in &mut page.countries {
                    if correction.matches(row) {
                        row.rocket_joined = false;
                    }
                }
            }
            self.corrections.push(correction);
        }
        bound.rocket.is_some()
    }
}

fn source_holds(source: Option<&SourceEndpoint>, track: Option<&TagHistory>, hour: i32) -> bool {
    let live = track.and_then(|track| {
        track
            .last
            .filter(|(_, at)| *at == hour)
            .map(|(identity, _)| (track, identity))
    });
    match (source, live) {
        (None, None) => true,
        (Some(source), Some((track, identity))) => {
            source.slot == identity.0
                && source.country_id == identity.1
                && track.hours.last() == Some(&source.read_hour)
        }
        _ => false,
    }
}

fn retain_last<T: Copy>(values: &mut Vec<T>) {
    if let Some(last) = values.last().copied() {
        values.clear();
        values.push(last);
    }
}
fn check_cancel(cancelled: &AtomicBool) -> io::Result<()> {
    if cancelled.load(Ordering::Acquire) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Production preparation/query cancelled",
        ))
    } else {
        Ok(())
    }
}

/// The exact preceding supplying read, not the displayed carried hour.
pub fn predecessor(hour: i32, measure: ProducedMeasure) -> Option<i32> {
    match measure {
        ProducedMeasure::Total => None,
        ProducedMeasure::Day => hour.checked_sub(24),
        ProducedMeasure::Week => hour.checked_sub(168),
        ProducedMeasure::Month => month_before(hour),
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PreparedProductionValue")]
pub struct SelectedValue {
    pub key: String,
    pub amount: Option<String>,
    pub equipment_amount: Option<String>,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PreparedProductionCountry")]
pub struct SelectedCountry {
    pub tag: String,
    pub slot: u32,
    pub country_id: u32,
    pub segment: String,
    pub total: Option<String>,
    pub equipment_total: Option<String>,
    pub read_hour: i32,
    pub observation: Option<i32>,
    pub values: Vec<SelectedValue>,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PreparedProductionSelection")]
pub struct SelectionPage {
    pub schema: u32,
    pub branch_id: String,
    pub first_hour: i32,
    pub last_hour: i32,
    pub known: bool,
    pub keys: Vec<ProducedKey>,
    pub types: Vec<ProducedType>,
    pub total_countries: usize,
    /// Logical first rank in this page; quantities remain exact until geometry.
    pub offset: String,
    pub value_min: String,
    pub value_max: String,
    pub countries: Vec<SelectedCountry>,
    /// Totals by type before paging, for the automatic units choice. Only a
    /// total-by-type units selection supplies them; unknown amounts add nothing.
    pub type_totals: Option<Vec<(String, String)>>,
    pub warnings: Vec<String>,
}

fn country<'a>(page: Option<&'a EndpointPage>, tag: &str) -> Option<&'a CountryEndpoint> {
    let page = page?;
    page.countries
        .binary_search_by(|country| country.tag.as_str().cmp(tag))
        .ok()
        .map(|index| &page.countries[index])
}
fn compatible(
    now: &SourceEndpoint,
    before: Option<&SourceEndpoint>,
    measure: ProducedMeasure,
) -> bool {
    if measure == ProducedMeasure::Total {
        return true;
    }
    let Some(target) = predecessor(now.read_hour, measure) else {
        return false;
    };
    before.is_some_and(|before| {
        before.read_hour == target
            && (now.slot, now.country_id, now.observation, now.segment)
                == (
                    before.slot,
                    before.country_id,
                    before.observation,
                    before.segment,
                )
    })
}
fn parse(value: &str) -> io::Result<i128> {
    value.parse().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid exact production amount",
        )
    })
}

fn aggregate(
    source: Option<&SourceEndpoint>,
    before: Option<&SourceEndpoint>,
    params: &ProducedParams,
    counted: impl Fn(u64, Purpose) -> bool,
) -> io::Result<Option<i128>> {
    let Some(source) = source else {
        return Ok(None);
    };
    if !compatible(source, before, params.measure)
        || match params.count {
            ProducedCount::Units => !source.units_known,
            ProducedCount::Ic => !source.spending_known,
        }
    {
        return Ok(None);
    }
    let sum = |endpoint: &SourceEndpoint, check_exact: bool| -> io::Result<Option<i128>> {
        let mut sum = 0i128;
        for amount in &endpoint.amounts {
            let variant = amount.variant.parse::<u64>().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Invalid recorded variant identity",
                )
            })?;
            if !counted(variant, amount.purpose) {
                continue;
            }
            if check_exact
                && match params.count {
                    ProducedCount::Units => !amount.units_exact,
                    ProducedCount::Ic => amount.spending_reason.is_some(),
                }
            {
                return Ok(None);
            }
            let raw = parse(match params.count {
                ProducedCount::Units => &amount.units,
                ProducedCount::Ic => &amount.spent,
            })?;
            sum = sum.checked_add(raw).ok_or_else(|| {
                io::Error::other("Produced equipment group exceeds its exact integer range")
            })?;
        }
        Ok(Some(sum))
    };
    let Some(current) = sum(source, true)? else {
        return Ok(None);
    };
    if params.measure == ProducedMeasure::Total {
        return Ok(Some(current));
    }
    let previous =
        sum(before.expect("compatible predecessor"), false)?.expect("unchecked exactness");
    Ok(Some(current.checked_sub(previous).ok_or_else(|| {
        io::Error::other("Produced equipment range exceeds its exact integer range")
    })?))
}

fn combined(left: Option<i128>, right: Option<i128>) -> io::Result<Option<i128>> {
    match (left, right) {
        (Some(left), Some(right)) => left.checked_add(right).map(Some).ok_or_else(|| {
            io::Error::other("Combined delivered amount exceeds its exact integer range")
        }),
        _ => Ok(None),
    }
}

/// Rank an arbitrary included subset at this endpoint, then page in complete order.
/// Work depends on the endpoint's countries/designs, independent of campaign length.
pub fn select(
    current: &EndpointPage,
    equipment_before: Option<&EndpointPage>,
    rocket_before: Option<&EndpointPage>,
    params: &ProducedParams,
    offset: usize,
    limit: usize,
    cancelled: &AtomicBool,
) -> io::Result<SelectionPage> {
    select_ordered(
        current,
        equipment_before,
        rocket_before,
        params,
        offset,
        limit,
        false,
        cancelled,
    )
}

#[allow(clippy::too_many_arguments)]
fn select_ordered(
    current: &EndpointPage,
    equipment_before: Option<&EndpointPage>,
    rocket_before: Option<&EndpointPage>,
    params: &ProducedParams,
    offset: usize,
    limit: usize,
    ranked: bool,
    cancelled: &AtomicBool,
) -> io::Result<SelectionPage> {
    check_cancel(cancelled)?;
    if params.top.is_some() && !params.tags.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Choose countries or a top count, never both",
        ));
    }
    if current.schema != 1
        || current.branch_id.parse::<u64>().ok() != params.branch_id.parse::<u64>().ok()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Prepared production schema or branch mismatch",
        ));
    }
    <crate::products::equipment_produced::EquipmentProduced as crate::products::Product>::validate(
        params,
    )
    .map_err(|reason| io::Error::new(io::ErrorKind::InvalidInput, reason))?;
    let included: BTreeSet<&str> = params.included.iter().map(String::as_str).collect();
    let classes = &current.classifications;
    let includes_resolved = classes
        .values()
        .any(|kind| kind.ordinal.is_some() && included.contains(kind.key.as_str()));
    let counted = |variant: u64, purpose: Purpose| {
        (params.refits || purpose != Purpose::Refit)
            && classes
                .get(&variant)
                .is_some_and(|kind| included.is_empty() || included.contains(kind.key.as_str()))
    };
    let unresolved = |variant: u64| {
        classes
            .get(&variant)
            .is_none_or(|kind| kind.ordinal.is_none())
    };
    let outside = |variant: u64, purpose: Purpose| {
        includes_resolved
            && unresolved(variant)
            && (params.refits || purpose != Purpose::Refit)
            && !counted(variant, purpose)
    };
    type GroupOrder = (usize, String, u64);
    type GroupMembers = (String, bool, BTreeSet<u64>);
    let mut groups: BTreeMap<GroupOrder, GroupMembers> = BTreeMap::new();
    let inventory = match params.count {
        ProducedCount::Units => UNITS_SEEN,
        ProducedCount::Ic => IC_SEEN,
    };
    for (&variant, kind) in classes {
        if !kind.seen_in(inventory, params.refits) {
            continue;
        }
        if !counted(variant, Purpose::New) {
            continue;
        }
        let (order, key, ambiguous) = match params.grouping {
            ProducedGrouping::Type => (
                (kind.ordinal.unwrap_or(usize::MAX), kind.key.clone(), 0),
                kind.key.clone(),
                kind.ordinal.is_some(),
            ),
            ProducedGrouping::Variant => (
                (
                    kind.ordinal.unwrap_or(usize::MAX),
                    kind.key.clone(),
                    variant,
                ),
                variant.to_string(),
                false,
            ),
            ProducedGrouping::Domain => {
                let (rank, key) = match kind.domain {
                    Some(Domain::Land) => (0, "land"),
                    Some(Domain::Air) => (1, "air"),
                    Some(Domain::Naval) => (2, "naval"),
                    None => (3, equipment::UNRESOLVED_ARM),
                };
                (
                    (rank, String::new(), 0),
                    key.to_owned(),
                    kind.domain.is_some(),
                )
            }
        };
        groups
            .entry(order)
            .or_insert_with(|| (key, ambiguous, BTreeSet::new()))
            .2
            .insert(variant);
    }
    let mut rows = Vec::new();
    let mut has_known = false;
    let mut has_unknown = false;
    let mut seen = HashSet::new();
    let selected: Vec<&CountryEndpoint> = if params.top.is_some() {
        current.countries.iter().collect()
    } else {
        params
            .tags
            .iter()
            .filter(|tag| seen.insert(tag.as_str()))
            .filter_map(|tag| country(Some(current), tag))
            .collect()
    };
    let evaluate = |row: &CountryEndpoint,
                    hydrate: bool|
     -> io::Result<(Option<i128>, SelectedCountry, bool)> {
        check_cancel(cancelled)?;
        let equipment = row.equipment.as_ref().expect("covered equipment endpoint");
        let e_before = country(equipment_before, &row.tag).and_then(|row| row.equipment.as_ref());
        let r_before = country(rocket_before, &row.tag).and_then(|row| row.rocket.as_ref());
        let rocket = row.rocket.as_ref().filter(|rocket| {
            row.rocket_joined
                && (rocket.slot, rocket.country_id) == (equipment.slot, equipment.country_id)
                && match (equipment.observation, rocket.observation) {
                    (Some(left), Some(right)) => left == right,
                    _ => true,
                }
        });
        let e = |filter: &dyn Fn(u64, Purpose) -> bool| {
            aggregate(Some(equipment), e_before, params, filter)
        };
        let r = |filter: &dyn Fn(u64, Purpose) -> bool| aggregate(rocket, r_before, params, filter);
        let scope_known = r(&|variant, _| variant == 0)?.is_some();
        let rocket_value = |value| if scope_known { value } else { None };
        let e_outside = e(&outside)?;
        let e_total = e(&counted)?.filter(|_| e_outside == Some(0));
        let r_outside = r(&outside)?;
        let r_total = rocket_value(r(&counted)?.filter(|_| r_outside == Some(0)));
        let total = combined(e_total, r_total)?;
        let covers = compatible(equipment, e_before, params.measure)
            && match params.count {
                ProducedCount::Units => equipment.units_known,
                ProducedCount::Ic => equipment.spending_known,
            };
        let mut values = Vec::new();
        for (key, ambiguous, variants) in groups.values().filter(|_| hydrate) {
            let group = |variant: u64, purpose: Purpose| {
                (params.refits || purpose != Purpose::Refit) && variants.contains(&variant)
            };
            let unsure = |variant: u64, purpose: Purpose| {
                unresolved(variant)
                    && (params.refits || purpose != Purpose::Refit)
                    && match params.grouping {
                        ProducedGrouping::Type => true,
                        ProducedGrouping::Domain => counted(variant, purpose),
                        ProducedGrouping::Variant => false,
                    }
            };
            let e_unsure = e(&unsure)?;
            let e_amount = e(&group)?.filter(|_| !ambiguous || e_unsure == Some(0));
            let r_unsure = r(&unsure)?;
            let r_amount = rocket_value(r(&group)?.filter(|_| !ambiguous || r_unsure == Some(0)));
            values.push(SelectedValue {
                key: key.clone(),
                amount: combined(e_amount, r_amount)?.map(|value| value.to_string()),
                equipment_amount: e_amount.map(|value| value.to_string()),
            });
        }
        Ok((
            total,
            SelectedCountry {
                tag: row.tag.clone(),
                slot: equipment.slot,
                country_id: equipment.country_id,
                segment: equipment.segment.to_string(),
                total: total.map(|value| value.to_string()),
                equipment_total: e_total.map(|value| value.to_string()),
                read_hour: equipment.read_hour,
                observation: equipment.observation,
                values,
            },
            covers,
        ))
    };
    for row in selected {
        if row.equipment.is_none() {
            continue;
        }
        let (total, country, covers) = evaluate(row, false)?;
        if covers {
            has_known |= total.is_some();
            has_unknown |= total.is_none();
        }
        rows.push((total, country));
    }
    if ranked || params.top.is_some() {
        rows.retain(|(value, _)| value.is_some_and(|value| params.top.is_none() || value != 0));
        rows.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| left.1.tag.cmp(&right.1.tag))
        });
        if let Some(top) = params.top {
            rows.truncate(top as usize);
        }
    }
    let total_countries = rows.len();
    let type_totals = if params.count == ProducedCount::Units
        && params.measure == ProducedMeasure::Total
        && params.grouping == ProducedGrouping::Type
        && params.included.is_empty()
    {
        let mut sums: BTreeMap<String, i128> = BTreeMap::new();
        for (_, row) in &rows {
            let (_, country, _) = evaluate(
                country(Some(current), &row.tag).expect("selected endpoint"),
                true,
            )?;
            for value in country.values {
                if let Some(amount) = value.amount {
                    let total = sums.entry(value.key).or_default();
                    *total = total.checked_add(parse(&amount)?).ok_or_else(|| {
                        io::Error::other("Produced type total exceeds its exact integer range")
                    })?;
                }
            }
        }
        Some(
            groups
                .values()
                .filter_map(|(key, _, _)| sums.get(key).map(|sum| (key.clone(), sum.to_string())))
                .collect(),
        )
    } else {
        None
    };
    let value_min = rows
        .iter()
        .filter_map(|(value, _)| *value)
        .min()
        .unwrap_or(0)
        .min(0);
    let value_max = rows
        .iter()
        .filter_map(|(value, _)| *value)
        .max()
        .unwrap_or(0)
        .max(0);
    let countries = rows
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|(_, row)| {
            evaluate(
                country(Some(current), &row.tag).expect("selected endpoint"),
                true,
            )
            .map(|(_, row, _)| row)
        })
        .collect::<io::Result<_>>()?;
    Ok(SelectionPage {
        type_totals,
        schema: 1,
        branch_id: params.branch_id.clone(),
        first_hour: current.first_hour,
        last_hour: current.last_hour,
        known: has_known && !has_unknown,
        keys: groups
            .values()
            .map(|(key, _, variants)| {
                let variant = *variants.first().expect("nonempty produced group");
                let kind = &classes[&variant];
                ProducedKey {
                    key: key.clone(),
                    name: if params.grouping == ProducedGrouping::Type {
                        kind.name.clone()
                    } else {
                        None
                    },
                    type_key: (params.grouping == ProducedGrouping::Variant)
                        .then(|| kind.key.clone()),
                    domain: if params.grouping == ProducedGrouping::Domain {
                        match key.as_str() {
                            "land" => Some(Domain::Land),
                            "air" => Some(Domain::Air),
                            "naval" => Some(Domain::Naval),
                            _ => None,
                        }
                    } else {
                        kind.domain
                    },
                    names: if params.grouping == ProducedGrouping::Variant {
                        kind.names.clone()
                    } else {
                        Vec::new()
                    },
                }
            })
            .collect(),
        types: {
            let mut types = BTreeMap::new();
            for kind in classes
                .values()
                .filter(|kind| kind.seen_in(inventory, true))
            {
                types
                    .entry((kind.ordinal.unwrap_or(usize::MAX), kind.key.clone()))
                    .or_insert_with(|| ProducedType {
                        key: kind.key.clone(),
                        name: kind.name.clone(),
                        domain: kind.domain,
                    });
            }
            types.into_values().collect()
        },
        total_countries,
        offset: offset.to_string(),
        value_min: value_min.to_string(),
        value_max: value_max.to_string(),
        countries,
        warnings: current.warnings_at(current.last_hour),
    })
}

fn section(name: &str, branch: u64) -> String {
    format!("{name}/{branch}")
}

const ENDPOINTS: &str = "production-endpoints-v1";
const TIMES: &str = "production-times-v1";
const STATE: &str = "production-state-v1";
const CATALOGUE: &str = "production-catalogue-v1";
#[derive(Deserialize, Serialize)]
pub(crate) struct ProductionCatalogue {
    schema: u32,
    branch: u64,
    pub(crate) classifications: BTreeMap<u64, Classification>,
    pub(crate) warnings: Vec<String>,
    pub(crate) spent_keys: BTreeMap<String, u64>,
    classification_keys: BTreeMap<u64, u64>,
    pub(crate) unproven_keys: BTreeMap<String, u64>,
    pub(crate) purpose_keys: BTreeMap<String, u64>,
    pub(crate) spending: SpendingDiagnostics,
    pub(crate) variant_warnings: Vec<String>,
}

/// Only refusal/read provenance; no production quantities or retained rows.
#[derive(Deserialize, Serialize)]
pub(crate) struct SpendingDiagnostics {
    equipment_tags: BTreeSet<String>,
    rocket_tags: BTreeSet<String>,
    reads: Vec<i32>,
    observations: Vec<i32>,
    unlisted: usize,
    unread_reason: Option<String>,
    tainted: BTreeMap<u32, (i32, ProductionSpendingUnknown)>,
    rocket_reads: Vec<i32>,
    rocket_observations: Vec<i32>,
    rocket_unread: usize,
    rocket_unattributed: usize,
    rocket_inconsistent: usize,
}
impl From<ScannedProduced> for SpendingDiagnostics {
    fn from(scanned: ScannedProduced) -> Self {
        Self {
            equipment_tags: BTreeSet::new(),
            rocket_tags: BTreeSet::new(),
            reads: scanned.reads,
            observations: scanned.observations,
            unlisted: scanned.unlisted,
            unread_reason: scanned.unread_reason,
            tainted: scanned.spending_tainted,
            rocket_reads: scanned.rocket.reads,
            rocket_observations: scanned.rocket.observations,
            rocket_unread: scanned.rocket.unread,
            rocket_unattributed: scanned.rocket.unattributed,
            rocket_inconsistent: scanned.rocket.inconsistent,
        }
    }
}
impl SpendingDiagnostics {
    pub fn scanned(&self, tag: &str) -> ScannedProduced {
        let mut rocket = RocketProduced::default();
        rocket.reads = self.rocket_reads.clone();
        rocket.observations = self.rocket_observations.clone();
        rocket.unread = self.rocket_unread;
        rocket.unattributed = self.rocket_unattributed;
        rocket.inconsistent = self.rocket_inconsistent;
        if self.rocket_tags.contains(tag) {
            rocket.tags.insert(tag.to_owned(), TagHistory::default());
        }
        ScannedProduced {
            tags: if self.equipment_tags.contains(tag) {
                BTreeMap::from([(tag.to_owned(), TagHistory::default())])
            } else {
                BTreeMap::new()
            },
            reads: self.reads.clone(),
            observations: self.observations.clone(),
            unlisted: self.unlisted,
            unread_reason: self.unread_reason.clone(),
            spending_tainted: self.tainted.clone(),
            rocket,
            ..Default::default()
        }
    }
}
#[derive(Deserialize, Serialize)]
struct TimeEntry {
    first: i32,
    last: i32,
    endpoint: u64,
}

#[derive(Deserialize, Serialize)]
struct LeafEndpoint {
    holder: (u32, u32, Option<i32>, u64),
    amount: Amount,
}

impl SourceEndpoint {
    fn holder(&self) -> (u32, u32, Option<i32>, u64) {
        (self.slot, self.country_id, self.observation, self.segment)
    }
}

impl ProductionAccumulator {
    fn endpoint_event(&mut self, key: u64, hour: i32, value: &impl Serialize) -> io::Result<()> {
        let bytes = serde_json::to_vec(value)?;
        let hash = *blake3::hash(&bytes).as_bytes();
        if self.endpoint_hashes.get(&key) != Some(&hash) {
            self.spent.event_raw(key, hour, bytes)?;
            self.endpoint_hashes.insert(key, hash);
        }
        Ok(())
    }
}

/// Where endpoint pages are written: the store writer, or [`HeldPages`].
pub trait PageWrites {
    /// Write `bytes` as range `ordinal` of `section`.
    fn write_range(&mut self, section: &str, ordinal: u64, bytes: Vec<u8>) -> io::Result<()>;
    /// Append `bytes` to `section`, whose next range is `next`; the range written.
    fn append(&mut self, section: &str, next: u64, bytes: Vec<u8>) -> io::Result<u64>;
}

impl PageWrites for store::Writer {
    fn write_range(&mut self, section: &str, ordinal: u64, bytes: Vec<u8>) -> io::Result<()> {
        self.write_at(section, ordinal, bytes.as_slice())
    }

    fn append(&mut self, section: &str, _next: u64, bytes: Vec<u8>) -> io::Result<u64> {
        self.write(section, bytes.as_slice())
    }
}

/// One held write: a range at its ordinal, or an append expected at its range.
enum HeldWrite {
    Range(u64),
    Append(u64),
}

/// Endpoint pages published while production is prepared on its own, whose
/// folds have no writer: each page's writes are held in publication order and
/// written by its next flush, before the flush's own, so the store holds what
/// publishing each page at its frame writes. Only endpoint publication
/// appends to the time pages, so an append's range is the accumulator's page
/// count when it was published.
#[derive(Default)]
pub struct HeldPages {
    writes: Vec<(String, HeldWrite, Vec<u8>)>,
    bytes: usize,
}

impl HeldPages {
    /// Bytes held for the next flush.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Write every held page, in order, through `writer`.
    pub fn write(&mut self, writer: &mut store::Writer) -> io::Result<()> {
        for (section, write, bytes) in std::mem::take(&mut self.writes) {
            match write {
                HeldWrite::Range(ordinal) => {
                    writer.write_at(&section, ordinal, bytes.as_slice())?
                }
                HeldWrite::Append(expected) => {
                    if writer.write(&section, bytes.as_slice())? != expected {
                        return Err(io::Error::other(
                            "Held production pages were appended out of their order",
                        ));
                    }
                }
            }
        }
        self.bytes = 0;
        Ok(())
    }
}

impl PageWrites for HeldPages {
    fn write_range(&mut self, section: &str, ordinal: u64, bytes: Vec<u8>) -> io::Result<()> {
        self.bytes += bytes.len();
        self.writes
            .push((section.to_owned(), HeldWrite::Range(ordinal), bytes));
        Ok(())
    }

    fn append(&mut self, section: &str, next: u64, bytes: Vec<u8>) -> io::Result<u64> {
        self.bytes += bytes.len();
        self.writes
            .push((section.to_owned(), HeldWrite::Append(next), bytes));
        Ok(next)
    }
}

pub fn publish_endpoint(
    writer: &mut store::Writer,
    state: &mut ProductionAccumulator,
    page: &EndpointPage,
) -> io::Result<u64> {
    publish_endpoint_to(writer, state, page)
}

/// [`publish_endpoint`] into `writes`.
pub fn publish_endpoint_to(
    writes: &mut impl PageWrites,
    state: &mut ProductionAccumulator,
    page: &EndpointPage,
) -> io::Result<u64> {
    if page.schema != 1 || page.branch_id.parse::<u64>().ok() != Some(state.branch) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Prepared production schema or branch mismatch",
        ));
    }
    let _cpu = ::replay_frame::cpu::scope_phase("production:series_flush");
    let mut header = page.clone();
    let mut active = BTreeSet::new();
    for row in &mut header.countries {
        for (rocket, source) in [(false, &mut row.equipment), (true, &mut row.rocket)] {
            let Some(source) = source else { continue };
            let holder = source.holder();
            for amount in std::mem::take(&mut source.amounts) {
                let variant: u64 = amount
                    .variant
                    .parse()
                    .map_err(|_| io::Error::other("Invalid production variant"))?;
                // The combined-refit S key holds the native refit endpoint event
                // and a purpose key its purpose's; their numeric runs remain
                // the already-computed combined and purpose amounts.
                let next = next_key(
                    &state.spent_keys,
                    &state.classification_keys,
                    &state.unproven_keys,
                    &state.purpose_keys,
                );
                let key = if !rocket && SERIES_PURPOSES.contains(&amount.purpose) {
                    let token = serde_json::to_string(&(&row.tag, variant, amount.purpose))?;
                    *state.purpose_keys.entry(token).or_insert(next)
                } else {
                    let refits = rocket || amount.purpose == Purpose::Refit;
                    let token = serde_json::to_string(&(&row.tag, rocket, refits, variant))?;
                    *state.spent_keys.entry(token).or_insert(next)
                };
                active.insert(key);
                state.endpoint_event(key, page.first_hour, &LeafEndpoint { holder, amount })?;
            }
        }
    }
    let removed: Vec<_> = state.endpoint_active.difference(&active).copied().collect();
    for key in removed {
        state
            .spent
            .event(key, page.first_hour, &Option::<LeafEndpoint>::None)?;
        state.endpoint_hashes.remove(&key);
    }
    state.endpoint_active = active;
    for (variant, classification) in std::mem::take(&mut header.classifications) {
        let next = next_key(
            &state.spent_keys,
            &state.classification_keys,
            &state.unproven_keys,
            &state.purpose_keys,
        );
        let key = *state.classification_keys.entry(variant).or_insert(next);
        state.endpoint_event(key, page.first_hour, &classification)?;
    }
    // The completed page can precede the frame whose new numeric read closed it.
    // Appending its events must not shorten the shared numeric batch frontier.
    state.spent.through(
        state
            .last_frame
            .unwrap_or(page.last_hour)
            .max(page.last_hour),
    );
    let endpoint = state.pages_written;
    // As `series::write_reply` writes a reply.
    let header = crate::prepared::series::compress(serde_json::to_vec(&header)?.as_slice(), 3)?;
    writes.write_range(&section(ENDPOINTS, state.branch), endpoint, header)?;
    let bytes = serde_json::to_vec(&TimeEntry {
        first: page.first_hour,
        last: page.last_hour,
        endpoint,
    })?;
    let ordinal = writes.append(&section(TIMES, state.branch), state.pages_written, bytes)?;
    state.pages_written = ordinal
        .checked_add(1)
        .ok_or_else(|| io::Error::other("Production page count exceeds its integer range"))?;
    Ok(ordinal)
}
pub fn write_state(
    writer: &mut store::Writer,
    state: &mut ProductionAccumulator,
) -> io::Result<u64> {
    write_catalogue(writer, state, &AtomicBool::new(false))?;
    let bytes = serde_json::to_vec(state)?;
    writer.write_at(&section(STATE, state.branch), 0, bytes.as_slice())?;
    Ok(0)
}

/// The common runner persists its combined accumulator once; this small catalogue
/// supplies reference-wide static type facts and diagnostics to older endpoint pages.
pub fn write_catalogue(
    writer: &mut store::Writer,
    state: &mut ProductionAccumulator,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    state.seal_holders();
    state.spent.flush(
        writer,
        &format!("production-spent-{}", state.branch),
        0,
        cancelled,
    )?;
    apply_corrections(writer, state)?;
    let bytes = serde_json::to_vec(&ProductionCatalogue {
        schema: state.schema,
        branch: state.branch,
        classifications: state.classifications.clone(),
        warnings: state.warnings.clone(),
        spent_keys: state.spent_keys.clone(),
        classification_keys: state.classification_keys.clone(),
        unproven_keys: state.unproven_keys.clone(),
        purpose_keys: state.purpose_keys.clone(),
        spending: {
            let mut diagnostics: SpendingDiagnostics = state.spending_scan().into();
            diagnostics.equipment_tags = state.first_equipment_holders.keys().cloned().collect();
            diagnostics.rocket_tags = state.first_rocket_holders.keys().cloned().collect();
            diagnostics
        },
        variant_warnings: state
            .variant_refusals
            .diagnostic_lines("equipment variants with unknown text/design operands"),
    })?;
    writer.write_at(&section(CATALOGUE, state.branch), 0, bytes.as_slice())
}

fn apply_corrections(
    writer: &mut store::Writer,
    state: &mut ProductionAccumulator,
) -> io::Result<()> {
    type Holder = (String, u32, u32, Option<i32>, i32);
    let mut intervals: BTreeMap<i32, (i32, HashSet<Holder>)> = BTreeMap::new();
    for correction in std::mem::take(&mut state.joins.corrections) {
        let interval = intervals
            .entry(correction.first_hour)
            .or_insert_with(|| (correction.last_hour, HashSet::new()));
        interval.0 = interval.0.max(correction.last_hour);
        interval.1.insert((
            correction.tag,
            correction.slot,
            correction.country_id,
            correction.observation,
            correction.read_hour,
        ));
    }
    for (first, (last, holders)) in intervals {
        // Seek only the affected native supplying interval, never earlier history.
        let (mut low, mut high) = (0, state.pages_written);
        while low < high {
            let mid = low + (high - low) / 2;
            let file = writer
                .read_at(&section(TIMES, state.branch), mid)?
                .ok_or_else(|| {
                    io::Error::other("Missing production time page for coverage repair")
                })?;
            let entry: TimeEntry = serde_json::from_reader(io::BufReader::new(file))?;
            if entry.last < first {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        for ordinal in low..state.pages_written {
            let file = writer
                .read_at(&section(TIMES, state.branch), ordinal)?
                .ok_or_else(|| {
                    io::Error::other("Missing production time page for coverage repair")
                })?;
            let entry: TimeEntry = serde_json::from_reader(io::BufReader::new(file))?;
            if entry.first > last {
                break;
            }
            let file = writer
                .read_at(&section(ENDPOINTS, state.branch), entry.endpoint)?
                .ok_or_else(|| {
                    io::Error::other("Missing production endpoint for coverage repair")
                })?;
            let mut compressed = Vec::new();
            io::Read::read_to_end(&mut io::BufReader::new(file), &mut compressed)?;
            let mut page: EndpointPage =
                serde_json::from_slice(&crate::prepared::series::decompress(&compressed)?)?;
            let mut changed = false;
            for row in &mut page.countries {
                if row.rocket_joined
                    && row.equipment.as_ref().is_some_and(|source| {
                        holders.contains(&(
                            row.tag.clone(),
                            source.slot,
                            source.country_id,
                            source.observation,
                            source.read_hour,
                        ))
                    })
                {
                    row.rocket_joined = false;
                    changed = true;
                }
            }
            if changed {
                crate::prepared::series::write_reply(
                    writer,
                    &section(ENDPOINTS, state.branch),
                    entry.endpoint,
                    &page,
                )?;
            }
        }
    }
    Ok(())
}
pub fn read_state(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    ordinal: u64,
) -> io::Result<Option<ProductionAccumulator>> {
    let Some(file) = store.read(key, &section(STATE, branch), ordinal)? else {
        return Ok(None);
    };
    let state: ProductionAccumulator = serde_json::from_reader(io::BufReader::new(file))?;
    if state.schema != 1 || state.branch != branch {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Prepared production state schema or branch mismatch",
        ));
    }
    Ok(Some(state))
}
/// Binary search tiny time entries on disk; retain only the endpoint being returned.
pub fn read_at(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    count: u64,
    hour: i32,
    cancelled: &AtomicBool,
) -> io::Result<Option<EndpointPage>> {
    Endpoints::new(store, key, branch, count, cancelled).page(hour, Leaves::All)
}

/// Which leaves an endpoint read restores onto its page.
#[derive(Clone, Copy)]
pub(crate) enum Leaves<'a> {
    /// Every country's equipment and Rocket amounts, and the hour's classifications.
    All,
    /// One country's equipment and Rocket amounts, and the hour's classifications.
    Country(&'a str),
    /// These countries' Rocket amounts only. Other amounts stay empty and the
    /// page's classifications are not read.
    Rockets(&'a BTreeSet<String>),
    /// No amounts or classifications: only the page's sources and coverage.
    Sources,
}

/// One query's reads of a branch's endpoint pages. The catalogue, its parsed
/// leaf tokens and the leaf reader (with the directories and blobs it has read)
/// are shared by every hour the query reads, and each read looks up only the
/// leaves it restores, all at once.
pub(crate) struct Endpoints<'a> {
    store: &'a store::Store,
    key: &'a store::SourceKey,
    branch: u64,
    count: u64,
    cancelled: &'a AtomicBool,
    /// None until read; then the branch's catalogue, if it has one.
    catalogue: Option<Option<ProductionCatalogue>>,
    /// The catalogue's leaf tokens in catalogue order: tag, Rocket, variant, leaf key.
    tokens: Vec<(String, bool, u64, u64)>,
    reader: Option<crate::prepared::series::Reader<'a>>,
}

impl<'a> Endpoints<'a> {
    pub(crate) fn new(
        store: &'a store::Store,
        key: &'a store::SourceKey,
        branch: u64,
        count: u64,
        cancelled: &'a AtomicBool,
    ) -> Self {
        Self {
            store,
            key,
            branch,
            count,
            cancelled,
            catalogue: None,
            tokens: Vec::new(),
            reader: None,
        }
    }

    pub(crate) fn catalogue(&mut self) -> io::Result<Option<&ProductionCatalogue>> {
        if self.catalogue.is_none() {
            let catalogue = read_catalogue(self.store, self.key, self.branch)?;
            if let Some(catalogue) = &catalogue {
                let spent = catalogue.spent_keys.iter().map(|(token, &leaf)| {
                    let (tag, rocket, _, variant): (String, bool, bool, u64) =
                        serde_json::from_str(token)?;
                    Ok((tag, rocket, variant, leaf))
                });
                let purposes = catalogue.purpose_keys.iter().map(|(token, &leaf)| {
                    let (tag, variant, _): (String, u64, Purpose) = serde_json::from_str(token)?;
                    Ok((tag, false, variant, leaf))
                });
                self.tokens = spent.chain(purposes).collect::<io::Result<_>>()?;
            }
            self.catalogue = Some(catalogue);
        }
        Ok(self.catalogue.as_ref().and_then(Option::as_ref))
    }

    /// The branch's leaf reader, opened once.
    pub(crate) fn reader(&mut self) -> io::Result<&mut crate::prepared::series::Reader<'a>> {
        if self.reader.is_none() {
            self.reader = Some(crate::prepared::series::Reader::new(
                self.store,
                self.key,
                format!("production-spent-{}", self.branch),
                self.cancelled,
            )?);
        }
        Ok(self.reader.as_mut().expect("leaf reader opened above"))
    }

    /// The page holding `hour`, with the requested leaves restored at `hour`.
    pub(crate) fn page(
        &mut self,
        hour: i32,
        leaves: Leaves<'_>,
    ) -> io::Result<Option<EndpointPage>> {
        let (store, key, branch) = (self.store, self.key, self.branch);
        let (mut low, mut high) = (0, self.count);
        while low < high {
            check_cancel(self.cancelled)?;
            let mid = low + (high - low) / 2;
            let Some(file) = store.read(key, &section(TIMES, branch), mid)? else {
                return Ok(None);
            };
            let entry: TimeEntry = serde_json::from_reader(io::BufReader::new(file))?;
            if hour < entry.first {
                high = mid;
            } else if hour > entry.last {
                low = mid + 1;
            } else {
                let Some(mut page) = crate::prepared::series::read_reply::<EndpointPage>(
                    store,
                    key,
                    &section(ENDPOINTS, branch),
                    entry.endpoint,
                )?
                else {
                    return Ok(None);
                };
                if page.schema != 1 || page.branch_id != branch.to_string() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Prepared production schema or branch mismatch",
                    ));
                }
                if self.catalogue()?.is_none() {
                    return Ok(None);
                }
                self.restore(&mut page, hour, leaves)?;
                page.render_warnings(hour);
                return Ok(Some(page));
            }
        }
        Ok(None)
    }

    /// Each requested leaf's latest event at `hour`, kept when it is the
    /// page's current holder's; then the hour's classifications.
    fn restore(
        &mut self,
        page: &mut EndpointPage,
        hour: i32,
        leaves: Leaves<'_>,
    ) -> io::Result<()> {
        let mut selected = Vec::new();
        for (tag, rocket, variant, leaf) in &self.tokens {
            let wanted = match leaves {
                Leaves::All => true,
                Leaves::Country(only) => tag == only,
                Leaves::Rockets(tags) => *rocket && tags.contains(tag),
                Leaves::Sources => false,
            };
            if !wanted {
                continue;
            }
            let Ok(index) = page.countries.binary_search_by(|row| row.tag.cmp(tag)) else {
                continue;
            };
            let row = &page.countries[index];
            if (if *rocket { &row.rocket } else { &row.equipment }).is_none() {
                continue;
            }
            selected.push((index, *rocket, *variant, *leaf));
        }
        let keys: Vec<u64> = selected.iter().map(|&(.., leaf)| leaf).collect();
        let rows = self.reader()?.latest_event_rows(&keys, hour)?;
        let mut amounts: BTreeMap<(usize, bool), BTreeMap<Entry, Amount>> = BTreeMap::new();
        for ((index, rocket, variant, _), row) in selected.into_iter().zip(rows) {
            let Some((_, bytes)) = row else {
                continue;
            };
            let value: serde_json::Value = serde_json::from_slice(&bytes)?;
            let Some(leaf) = serde_json::from_value::<Option<LeafEndpoint>>(value)? else {
                continue;
            };
            let row = &page.countries[index];
            let source = if rocket { &row.rocket } else { &row.equipment };
            if source
                .as_ref()
                .is_some_and(|source| leaf.holder == source.holder())
            {
                amounts
                    .entry((index, rocket))
                    .or_default()
                    .insert((variant, leaf.amount.purpose), leaf.amount);
            }
        }
        for ((index, rocket), values) in amounts {
            let row = &mut page.countries[index];
            let source = if rocket {
                &mut row.rocket
            } else {
                &mut row.equipment
            };
            source.as_mut().expect("selected production source").amounts =
                values.into_values().collect();
        }
        if matches!(leaves, Leaves::Rockets(_) | Leaves::Sources) {
            return Ok(());
        }
        let (variants, keys): (Vec<u64>, Vec<u64>) = self
            .catalogue
            .as_ref()
            .and_then(Option::as_ref)
            .expect("catalogue read before restoring leaves")
            .classification_keys
            .iter()
            .map(|(&variant, &key)| (variant, key))
            .unzip();
        let rows = self.reader()?.latest_event_rows(&keys, hour)?;
        for (variant, row) in variants.into_iter().zip(rows) {
            if let Some((_, bytes)) = row {
                let value: serde_json::Value = serde_json::from_slice(&bytes)?;
                page.classifications
                    .insert(variant, serde_json::from_value(value)?);
            }
        }
        Ok(())
    }
}

pub fn endpoint_count(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
) -> io::Result<u64> {
    Ok(store
        .range_count(key, &section(TIMES, branch))?
        .unwrap_or(0))
}

#[cfg(test)]
pub fn latest_state(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
) -> io::Result<Option<ProductionAccumulator>> {
    let count = store
        .range_count(key, &section(STATE, branch))?
        .unwrap_or(0);
    match count.checked_sub(1) {
        Some(ordinal) => read_state(store, key, branch, ordinal),
        None => Ok(None),
    }
}

pub fn select_at(
    store: &store::Store,
    key: &store::SourceKey,
    hour: i32,
    params: &ProducedParams,
    offset: usize,
    limit: usize,
    cancelled: &AtomicBool,
) -> io::Result<Option<SelectionPage>> {
    select_at_ordered(store, key, hour, params, offset, limit, false, cancelled)
}

pub(crate) fn read_catalogue(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
) -> io::Result<Option<ProductionCatalogue>> {
    let Some(file) = store.read(key, &section(CATALOGUE, branch), 0)? else {
        return Ok(None);
    };
    let catalogue: ProductionCatalogue = serde_json::from_reader(io::BufReader::new(file))?;
    if catalogue.schema != 1 || catalogue.branch != branch {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Prepared production catalogue schema or branch mismatch",
        ));
    }
    Ok(Some(catalogue))
}

/// Amounts remain independent on disk; equipment-produced projects its Rocket
/// source to the equipment supplying read using the original dated join.
/// Each supplying read hour is read once, restoring only the Rocket amounts of
/// the countries joined at it.
fn bind_equipment_rockets(
    endpoints: &mut Endpoints<'_>,
    page: &mut EndpointPage,
) -> io::Result<()> {
    let mut joined: BTreeMap<i32, BTreeSet<String>> = BTreeMap::new();
    for row in &mut page.countries {
        let Some(equipment) = row.equipment.as_ref().filter(|_| row.rocket_joined) else {
            row.rocket = None;
            continue;
        };
        joined
            .entry(equipment.read_hour)
            .or_default()
            .insert(row.tag.clone());
    }
    let mut reads = BTreeMap::new();
    for (&hour, tags) in &joined {
        reads.insert(hour, endpoints.page(hour, Leaves::Rockets(tags))?);
    }
    for row in &mut page.countries {
        let Some(equipment) = row.equipment.as_ref().filter(|_| row.rocket_joined) else {
            continue;
        };
        let hour = equipment.read_hour;
        row.rocket = country(reads[&hour].as_ref(), &row.tag).and_then(|row| row.rocket.clone());
    }
    Ok(())
}

/// An unobserved endpoint supplies no country amounts, ranks or classifications.
pub fn empty_selection(
    hour: i32,
    params: &ProducedParams,
    offset: usize,
    limit: usize,
    ranked: bool,
    cancelled: &AtomicBool,
) -> io::Result<SelectionPage> {
    let current = EndpointPage {
        schema: 1,
        branch_id: params.branch_id.clone(),
        first_hour: hour,
        last_hour: hour,
        equipment_read: None,
        rocket_read: None,
        equipment_origin: None,
        equipment_restart: None,
        rocket_origin: None,
        rocket_restart: None,
        countries: Vec::new(),
        classifications: BTreeMap::new(),
        warnings: Vec::new(),
        unnamed_before: 0,
        unnamed_per_hour: 0,
        unnamed_warning_index: 0,
    };
    select_ordered(
        &current, None, None, params, offset, limit, ranked, cancelled,
    )
}

pub fn select_ranked_at(
    store: &store::Store,
    key: &store::SourceKey,
    hour: i32,
    params: &ProducedParams,
    offset: usize,
    limit: usize,
    cancelled: &AtomicBool,
) -> io::Result<Option<SelectionPage>> {
    select_at_ordered(store, key, hour, params, offset, limit, true, cancelled)
}

#[allow(clippy::too_many_arguments)]
fn select_at_ordered(
    store: &store::Store,
    key: &store::SourceKey,
    hour: i32,
    params: &ProducedParams,
    offset: usize,
    limit: usize,
    ranked: bool,
    cancelled: &AtomicBool,
) -> io::Result<Option<SelectionPage>> {
    let branch = params
        .branch_id
        .parse::<u64>()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid branch"))?;
    check_cancel(cancelled)?;
    if params.tags.is_empty() && params.top.is_none() {
        let Some(catalogue) = read_catalogue(store, key, branch)? else {
            return Ok(None);
        };
        let current = EndpointPage {
            schema: 1,
            branch_id: branch.to_string(),
            first_hour: hour,
            last_hour: hour,
            equipment_read: None,
            rocket_read: None,
            equipment_origin: None,
            equipment_restart: None,
            rocket_origin: None,
            rocket_restart: None,
            countries: Vec::new(),
            classifications: catalogue.classifications,
            warnings: catalogue.warnings,
            unnamed_before: 0,
            unnamed_per_hour: 0,
            unnamed_warning_index: 0,
        };
        let mut selection = select_ordered(
            &current, None, None, params, offset, limit, ranked, cancelled,
        )?;
        selection.keys.clear();
        return Ok(Some(selection));
    }
    let count = endpoint_count(store, key, branch)?;
    let mut endpoints = Endpoints::new(store, key, branch, count, cancelled);
    let Some(mut current) = endpoints.page(hour, Leaves::All)? else {
        return Ok(None);
    };
    bind_equipment_rockets(&mut endpoints, &mut current)?;
    if let Some(catalogue) = endpoints.catalogue()? {
        for (&variant, kind) in &catalogue.classifications {
            let mut kind = kind.clone();
            // Type identity is stable; recorded naming stays at this endpoint's read.
            if let Some(at_hour) = current.classifications.get(&variant) {
                kind.names = at_hour.names.clone();
            }
            current.classifications.insert(variant, kind);
        }
        current.warnings = catalogue.warnings.clone();
    }
    let equipment_before = current
        .equipment_read
        .and_then(|hour| predecessor(hour, params.measure))
        .map(|hour| endpoints.page(hour, Leaves::All))
        .transpose()?
        .flatten();
    let mut rocket_before = current
        .rocket_read
        .and_then(|hour| predecessor(hour, params.measure))
        .map(|hour| endpoints.page(hour, Leaves::All))
        .transpose()?
        .flatten();
    if let Some(page) = &mut rocket_before {
        bind_equipment_rockets(&mut endpoints, page)?;
    }
    select_ordered(
        &current,
        equipment_before.as_ref(),
        rocket_before.as_ref(),
        params,
        offset,
        limit,
        ranked,
        cancelled,
    )
    .map(Some)
}

#[cfg(test)]
#[path = "prepared_production/tests.rs"]
mod tests;
