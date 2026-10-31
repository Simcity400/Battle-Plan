//! Production's preparation: one observer of every country's production
//! output, whose dated endpoints are published as integer facts ([`facts`])
//! and read through indexes decoded once per publication ([`read`]).

pub(crate) mod facts;
pub(crate) mod read;
pub(crate) mod select;

pub use facts::FACTS;
pub use select::{SelectedCountry, SelectedValue, SelectionPage, empty_selection, select_at};

use crate::metadata::{TaggedCountry, slot_read};
use crate::products::equipment::production::{
    CostTotal, Entry, Identity, Named, ProductionSpendingUnknown, Purpose, RocketProduced,
    TagHistory, Total,
};
use crate::products::equipment::{
    self, RawVariant, Texts, VariantCatalogue, VariantHour, VariantReads, VariantVersion,
};
use crate::products::equipment_produced::{
    ProducedMeasure, RecordingCounts, month_before, recording_warnings_of,
};
use facts::{Endpoint, Holder, Leaf, NameCode, Out, Preamble};
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

/// Prepared state's layout; another is refused rather than read.
pub const SCHEMA: u32 = 2;

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
    pending: Option<Pending>,
    /// The latest complete variant read: names it dates take its hour.
    names_hour: Option<i32>,
    segments: BTreeMap<String, u64>,
    rocket_segments: BTreeMap<String, u64>,
    /// What the latest endpoint's warnings were rendered from, and its
    /// unnamed country-hours when last sealed.
    final_counts: PageCounts,
    final_unnamed: usize,
    joins: RocketJoins,
    /// Endpoints completed in this branch.
    pages_written: u64,
    spent: crate::prepared::series::Accumulator,
    /// Each (holder, Rocket, refits, design)'s spent series key.
    #[serde(with = "map_pairs")]
    spent_keys: BTreeMap<SpentKey, u64>,
    /// Keys of each equipment entry's cumulative input from production ticks
    /// whose purpose was not recorded, by (holder, variant, purpose); only
    /// entries that ever had such input.
    #[serde(with = "map_pairs")]
    unproven_keys: BTreeMap<PurposeKey, u64>,
    /// Keys of each equipment design's cumulative input of each
    /// [`SERIES_PURPOSES`] purpose, by (holder, variant, purpose); only designs
    /// that ever had input of that purpose.
    #[serde(with = "map_pairs")]
    purpose_keys: BTreeMap<PurposeKey, u64>,
    /// Keys below it were published.
    keys_written: u64,
    spent_active: BTreeSet<u64>,
    #[serde(skip)]
    spent_holders: [BTreeMap<String, SpentHolder>; 2],
    #[serde(skip)]
    spent_holders_ready: bool,
    #[cfg(test)]
    #[serde(skip)]
    spent_visits: usize,
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
    /// Variants whose recorded operands were refused, by reason.
    variant_refusals: BTreeMap<String, BTreeSet<u64>>,
    /// Every equipment observation's counting start, as history lists them;
    /// `result` keeps only the latest between batches.
    observation_starts: Vec<i32>,
    tables: Tables,
    /// Holders present at the last completed endpoint.
    present_holders: BTreeSet<u32>,
    #[serde(skip)]
    batch: Batch,
    /// Every endpoint completed, with its names hour, for tests' oracles.
    #[cfg(test)]
    #[serde(skip)]
    pub(crate) completed: Vec<(EndpointPage, Option<i32>)>,
    /// Every Rocket join withdrawn, for tests' oracles.
    #[cfg(test)]
    #[serde(skip)]
    revoked: Vec<CoverageCorrection>,
}

/// A spent series' (holder, Rocket, refits, design).
type SpentKey = (u32, bool, bool, u64);
/// A purpose series' (holder, design, purpose).
type PurposeKey = (u32, u64, Purpose);

/// The ids facts name holders, designs, tokens and raw operands by, each
/// assigned once in the order met; those past the written counts are
/// published at the next flush.
#[derive(Default, Deserialize, Serialize)]
struct Tables {
    holders: Vec<String>,
    holder_ids: BTreeMap<String, u32>,
    variants: Vec<u64>,
    variant_ids: BTreeMap<u64, u32>,
    /// Token 0 is no token.
    tokens: Vec<String>,
    token_ids: BTreeMap<String, u32>,
    raws: Vec<RawVariant>,
    #[serde(skip)]
    raw_ids: HashMap<RawVariant, u32>,
    written: [usize; 4],
}

impl Tables {
    fn holder(&mut self, tag: &str) -> u32 {
        if let Some(&id) = self.holder_ids.get(tag) {
            return id;
        }
        let id = self.holders.len() as u32;
        self.holders.push(tag.to_owned());
        self.holder_ids.insert(tag.to_owned(), id);
        id
    }

    fn variant(&mut self, handle: u64) -> u32 {
        if let Some(&id) = self.variant_ids.get(&handle) {
            return id;
        }
        let id = self.variants.len() as u32;
        self.variants.push(handle);
        self.variant_ids.insert(handle, id);
        id
    }

    fn token(&mut self, token: &str) -> u32 {
        if self.tokens.is_empty() {
            self.tokens.push(String::new());
            self.token_ids.insert(String::new(), 0);
        }
        if let Some(&id) = self.token_ids.get(token) {
            return id;
        }
        let id = self.tokens.len() as u32;
        self.tokens.push(token.to_owned());
        self.token_ids.insert(token.to_owned(), id);
        id
    }

    /// A read's operands, one id however many reads held them.
    fn raw(&mut self, raw: &RawVariant) -> u32 {
        if self.raw_ids.len() != self.raws.len() {
            self.raw_ids = self
                .raws
                .iter()
                .enumerate()
                .map(|(id, raw)| (raw.clone(), id as u32))
                .collect();
        }
        let key = RawVariant {
            first_hour: 0,
            ..raw.clone()
        };
        if let Some(&id) = self.raw_ids.get(&key) {
            return id;
        }
        let id = self.raws.len() as u32;
        self.raws.push(key.clone());
        self.raw_ids.insert(key, id);
        id
    }

    /// The entries assigned since the last flush, which it publishes.
    fn write_new(&mut self, out: &mut Out) -> io::Result<()> {
        self.token("");
        let [holders, variants, tokens, raws] = self.written;
        out.count(holders)?;
        out.count(self.holders.len() - holders)?;
        for tag in &self.holders[holders..] {
            out.str(tag)?;
        }
        out.count(variants)?;
        out.count(self.variants.len() - variants)?;
        for handle in &self.variants[variants..] {
            out.u64(*handle);
        }
        out.count(tokens)?;
        out.count(self.tokens.len() - tokens)?;
        for token in &self.tokens[tokens..] {
            out.str(token)?;
        }
        out.count(raws)?;
        out.count(self.raws.len() - raws)?;
        for raw in &self.raws[raws..] {
            read::write_raw(raw, out)?;
        }
        self.written = [
            self.holders.len(),
            self.variants.len(),
            self.tokens.len(),
            self.raws.len(),
        ];
        Ok(())
    }
}

/// The open endpoint: what it publishes, with what is kept beside it.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Pending {
    page: EndpointPage,
    /// The latest complete variant read when it began.
    names_hour: Option<i32>,
    /// What its warnings were rendered from.
    counts: PageCounts,
}

/// What an endpoint's warnings are rendered from: counted recording
/// limitations, refused variant operands and observation restarts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct PageCounts {
    pub recording: SavedCounts,
    pub refusals: BTreeMap<String, BTreeSet<u64>>,
    pub restarts: u32,
}

/// [`RecordingCounts`] as kept: without unnamed holders, which an endpoint
/// counts by hour.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct SavedCounts {
    pub reads: bool,
    pub unread: usize,
    pub unread_reason: Option<String>,
    pub tainted_from: Option<i32>,
    pub missed: usize,
    pub inexact: usize,
    pub lost: usize,
    pub unattributed: usize,
    pub unlisted: usize,
    pub observations: usize,
    pub rocket_reads: bool,
    pub rocket_unread: usize,
    pub rocket_unattributed: usize,
    pub rocket_inconsistent: usize,
}

/// One line per refusal reason, naming every variant it refused.
pub(crate) fn refusal_lines(refusals: &BTreeMap<String, BTreeSet<u64>>) -> Vec<String> {
    let mut lines = crate::catalogue::Refusals::default();
    for (reason, variants) in refusals {
        for variant in variants {
            lines.note(reason.as_str(), variant.to_string());
        }
    }
    lines.diagnostic_lines("equipment variants with unknown text/design operands")
}

impl PageCounts {
    /// Its warnings, and where the unnamed holders' warning goes.
    pub(crate) fn warnings(&self) -> (Vec<String>, usize) {
        let counts = &self.recording;
        let (mut warnings, unnamed) = recording_warnings_of(
            Vec::new(),
            &RecordingCounts {
                reads: counts.reads,
                unread: counts.unread,
                unread_reason: counts.unread_reason.clone(),
                tainted_from: counts.tainted_from,
                missed: counts.missed,
                inexact: counts.inexact,
                lost: counts.lost,
                unattributed: counts.unattributed,
                unlisted: counts.unlisted,
                observations: counts.observations,
                unnamed: 0,
                rocket_reads: counts.rocket_reads,
                rocket_unread: counts.rocket_unread,
                rocket_unattributed: counts.rocket_unattributed,
                rocket_inconsistent: counts.rocket_inconsistent,
            },
        );
        let refusals = refusal_lines(&self.refusals);
        let unnamed = unnamed + refusals.len();
        warnings.splice(..0, refusals);
        if self.restarts > 0 {
            warnings.push(format!("The production output observation restarted {} times in this branch; running totals start again at each.", self.restarts));
        }
        (warnings, unnamed)
    }

    /// Its warnings with `unnamed` country-hours counted.
    pub(crate) fn warnings_with(&self, unnamed: usize) -> Vec<String> {
        let (mut warnings, index) = self.warnings();
        if let Some(warning) =
            crate::products::equipment_produced::unnamed_recording_warning(unnamed)
        {
            warnings.insert(index, warning);
        }
        warnings
    }
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

    fn read(&mut self, frame: &DecodedFrame, refusals: &mut BTreeMap<String, BTreeSet<u64>>) {
        for (id, raw) in self.reads.build(frame, false) {
            for (_, reason) in &raw.refusals {
                refusals.entry(reason.clone()).or_default().insert(id);
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

/// A variant's classification as preparation knows it, without content: the
/// token it was last classified with, its names as each read recorded them,
/// and which inventories saw it. Readers resolve its type and names.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Classification {
    /// Empty when no read spelled it.
    pub token: String,
    pub names: Vec<VariantName>,
    /// Units, then IC, inventory, independently for each purpose: the flag
    /// at the measure's first index plus [`Purpose::index`].
    pub seen: [bool; SEEN],
}

/// A variant's names from one read: dated by it, with the raw operands read
/// then (none from an hour its names stopped being known).
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct VariantName {
    pub first_hour: i32,
    pub raw: Option<u32>,
}

/// [`Classification::seen`]'s length and each measure's first index.
pub const SEEN: usize = 2 * Purpose::ALL.len();
pub const UNITS_SEEN: usize = 0;
pub const IC_SEEN: usize = Purpose::ALL.len();

pub(crate) fn seen_mask(seen: &[bool; SEEN]) -> u8 {
    seen.iter()
        .enumerate()
        .fold(0, |mask, (index, seen)| mask | u8::from(*seen) << index)
}

/// Whether the measure starting at `inventory` saw the design, ship refits
/// counted only with `refits`.
pub(crate) fn seen_in(mask: u8, inventory: usize, refits: bool) -> bool {
    Purpose::ALL.into_iter().any(|purpose| {
        (refits || purpose != Purpose::Refit) && mask & 1 << (inventory + purpose.index()) != 0
    })
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

#[derive(Clone, Debug, Deserialize, Serialize)]
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

    /// What spending readers count of the branch's reads, by holder id.
    fn spending_facts(&mut self) -> read::SpendingFacts {
        let observations = |origin: Option<i32>, restart: Option<i32>, last: Option<i32>| {
            [origin, restart, last]
                .into_iter()
                .flatten()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        };
        read::SpendingFacts {
            reads: self.result.reads.clone(),
            observations: observations(
                self.equipment_origin,
                self.equipment_restart,
                self.result.observations.last().copied(),
            ),
            unlisted: self.unlisted.len(),
            unread_reason: self.result.unread_reason.clone(),
            tainted: self
                .result
                .spending_tainted
                .iter()
                .map(|(&observation, &(hour, reason))| (observation, hour, reason))
                .collect(),
            rocket_reads: self.result.rocket.reads.clone(),
            rocket_observations: observations(
                self.rocket_origin,
                self.rocket_restart,
                self.result.rocket.observations.last().copied(),
            ),
            rocket_unread: self.result.rocket.unread,
            rocket_unattributed: self.result.rocket.unattributed,
            rocket_inconsistent: self.result.rocket.inconsistent,
            equipment_holders: self
                .first_equipment_holders
                .keys()
                .map(|tag| self.tables.holder(tag))
                .collect(),
            rocket_holders: self
                .first_rocket_holders
                .keys()
                .map(|tag| self.tables.holder(tag))
                .collect(),
        }
    }

    pub fn buffered_bytes(&self) -> usize {
        self.spent.bytes() + self.batch.bytes
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
                let holder = self.tables.holder(tag);
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
                        let next =
                            next_key(&self.spent_keys, &self.unproven_keys, &self.purpose_keys);
                        let key = *self
                            .spent_keys
                            .entry((holder, rocket, *refits, variant))
                            .or_insert(next);
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
                        let next =
                            next_key(&self.spent_keys, &self.unproven_keys, &self.purpose_keys);
                        let key = *self
                            .purpose_keys
                            .entry((holder, variant, purpose))
                            .or_insert(next);
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
                        let token = (holder, variant, purpose);
                        if total.unproven_raw == 0 && !self.unproven_keys.contains_key(&token) {
                            continue;
                        }
                        let next =
                            next_key(&self.spent_keys, &self.unproven_keys, &self.purpose_keys);
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
            schema: SCHEMA,
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
    fn observation_starts(&self) -> Vec<i32> {
        let mut starts = self.observation_starts.clone();
        for since in &self.result.observations {
            if starts.last() != Some(since) {
                starts.push(*since);
            }
        }
        starts
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
            if kind.names.last().is_some_and(|name| name.raw.is_none()) {
                continue;
            }
            kind.names = vec![VariantName {
                first_hour: hour,
                raw: None,
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
            result.unnamed += pending
                .as_ref()
                .map_or(0, |pending| pending.page.unnamed_per_hour);
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

/// The next series key: spent, unproven and purpose keys share one numbering.
fn next_key(
    spent: &BTreeMap<SpentKey, u64>,
    unproven: &BTreeMap<PurposeKey, u64>,
    purposes: &BTreeMap<PurposeKey, u64>,
) -> u64 {
    (spent.len() + unproven.len() + purposes.len()) as u64
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

/// One design's cumulative amounts of one purpose in a source.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Amount {
    pub variant: u64,
    pub purpose: Purpose,
    /// Exact signed cumulative accounting quantity.
    pub units: i128,
    pub units_exact: bool,
    pub spent: i128,
    pub spending_reason: Option<ProductionSpendingUnknown>,
    /// The magnitudes of the input of production ticks whose purpose was not
    /// recorded, summed: dated evidence that grows and never cancels.
    pub unproven: i128,
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

/// One endpoint as preparation holds it: another begins wherever any of it
/// would differ.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct EndpointPage {
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
                variant: entry.0,
                purpose: entry.1,
                units: units.units,
                units_exact: units.inexact == 0,
                spent: spending.raw,
                spending_reason: spending.reason,
                unproven: spending.unproven_raw,
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
    /// Ingests one frame; an endpoint it completes joins the batch's facts.
    pub fn ingest_changed(
        &mut self,
        format: FormatVersion,
        frame: &DecodedFrame,
        cancelled: &AtomicBool,
        changes: &crate::prepared::frame_changes::FrameChanges,
    ) -> io::Result<()> {
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
        let changed = self.schema != SCHEMA
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
            pending.page.last_hour = frame.hour;
            self.advance_spent_holders(frame.hour);
            self.last_frame = Some(frame.hour);
            return Ok(());
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
        if self.schema != SCHEMA
            || self.pending.is_none()
            || self.last_frame.and_then(|hour| hour.checked_add(1)) != Some(first)
        {
            return Ok(false);
        }
        self.spent.advance_span(first, last)?;
        if let Some(pending) = &mut self.pending {
            pending.page.last_hour = last;
        }
        self.advance_spent_holders(last);
        self.last_frame = Some(last);
        Ok(true)
    }
    fn seal_holders(&mut self) {
        let Some(hour) = self.last_frame else {
            return;
        };
        let Some(Pending { page, .. }) = &self.pending else {
            return;
        };
        self.result.unnamed = page.unnamed_at(hour);
        self.final_unnamed = self.result.unnamed;
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
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let changes = crate::prepared::frame_changes::FrameChanges {
            rows_changed: [true; replay_data::rows::TABLE_COUNT],
            table_read: [true; replay_data::rows::TABLE_COUNT],
            slot_read: [true; replay_data::rows::TABLE_COUNT],
            updates: None,
        };
        self.ingest_changed(format, frame, cancelled, &changes)
    }

    fn ingest_frame(
        &mut self,
        format: FormatVersion,
        frame: &DecodedFrame,
        cancelled: &AtomicBool,
        equipment_changed: bool,
        rocket_changed: bool,
    ) -> io::Result<()> {
        let cpu = ::replay_frame::cpu::scope_phase("production:settle_reconcile");
        check_cancel(cancelled)?;
        if self.schema != SCHEMA {
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
            && self.pending.as_ref().is_some_and(|pending| {
                pending.page.unnamed_per_hour == 0 && self.same_holders(&pending.page, frame.hour)
            })
        {
            self.pending.as_mut().expect("unchanged endpoint").page.last_hour = frame.hour;
            check_cancel(cancelled)?;
            return Ok(());
        }
        let cpu = ::replay_frame::cpu::scope_phase("production:presentation");
        // A fresh hourly catalogue is empty outside a complete variant read.
        // At a complete read only changed operands need building again.
        let complete_names = VariantCatalogue::hour_of(format, frame) == VariantHour::Read;
        if complete_names {
            self.variants.read(frame, &mut self.variant_refusals);
            self.names_hour = Some(frame.hour);
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
            let mut names: Vec<VariantName> = Vec::new();
            for version in catalogue.versions.get(&variant).into_iter().flatten() {
                let name = VariantName {
                    first_hour: frame.hour,
                    raw: version.raw.as_ref().map(|raw| self.tables.raw(raw)),
                };
                if names.last().is_none_or(|last| last.raw != name.raw) {
                    names.push(name);
                }
            }
            self.classifications.insert(
                variant,
                Classification {
                    token: token.to_owned(),
                    names,
                    seen: self
                        .classifications
                        .get(&variant)
                        .map_or([false; SEEN], |kind| kind.seen),
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
                    kind.names = vec![VariantName {
                        first_hour: frame.hour,
                        raw: None,
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
                    self.pending.as_mut().map(|pending| &mut pending.page),
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
        let counts = PageCounts {
            recording: SavedCounts {
                reads: !self.result.reads.is_empty(),
                unread: self.result.unread,
                unread_reason: self.result.unread_reason.clone(),
                tainted_from: self.result.tainted.iter().next().map(|(_, from)| *from),
                missed: self.result.missed,
                inexact: self.result.inexact,
                lost: self.result.lost,
                unattributed: self.result.unattributed,
                unlisted: self.unlisted.len(),
                observations: self.result.observations.len(),
                rocket_reads: !self.result.rocket.reads.is_empty(),
                rocket_unread: self.result.rocket.unread,
                rocket_unattributed: self.result.rocket.unattributed,
                rocket_inconsistent: self.result.rocket.inconsistent,
            },
            refusals: self.variant_refusals.clone(),
            restarts: self.observation,
        };
        let (warnings, unnamed_warning_index) = counts.warnings();
        let mut page = EndpointPage {
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
                page.first_hour = previous.page.first_hour;
                let last = previous.page.last_hour;
                if page.unnamed_per_hour == previous.page.unnamed_per_hour
                    && page.unnamed_before == previous.page.unnamed_at(last)
                {
                    page.unnamed_before = previous.page.unnamed_before;
                }
                previous.page.last_hour = frame.hour;
                if last.checked_add(1) == Some(frame.hour) && previous.page == page {
                    // The page holds: what it began with stays.
                    self.pending = Some(Pending { page, ..previous });
                    None
                } else {
                    previous.page.last_hour = last;
                    page.first_hour = frame.hour;
                    page.unnamed_before = diagnostics_before[4];
                    self.final_counts = counts.clone();
                    self.pending = Some(Pending {
                        page,
                        names_hour: self.names_hour,
                        counts,
                    });
                    Some(previous)
                }
            }
            None => {
                self.final_counts = counts.clone();
                self.pending = Some(Pending {
                    page,
                    names_hour: self.names_hour,
                    counts,
                });
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
        self.observation_starts = self.observation_starts();
        retain_last(&mut self.result.observations);
        retain_last(&mut self.result.rocket.observations);
        self.seal_holders();
        if let Some(completed) = completed {
            self.complete(completed)?;
        }
        check_cancel(cancelled)?;
        Ok(())
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
        pending: Option<&mut EndpointPage>,
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

pub(crate) fn section(name: &str, branch: u64) -> String {
    format!("{name}/{branch}")
}

/// A branch's spent series (P4), by integer key.
pub(crate) fn spent_section(branch: u64) -> String {
    format!("production-spent-{branch}")
}

/// The section a test's saved accumulator is kept in, as `{STATE}/{branch}`.
pub const STATE: &str = "production-state-v2";

/// One holder's leaf events in a batch (P3), in groups that each begin with
/// its whole state.
#[derive(Default)]
struct HolderStream {
    groups: Vec<HolderGroup>,
    open: Out,
    first: u64,
    last: u64,
    events: u32,
}

struct HolderGroup {
    first: u64,
    last: u64,
    events: u32,
    bytes: Vec<u8>,
}

impl HolderStream {
    /// One event: the holder's state at `endpoint`, where `before` was its
    /// state at the endpoint before. Returns the bytes it added.
    fn push(
        &mut self,
        holder: u32,
        endpoint: u64,
        before: Option<&Holder>,
        now: Option<&Holder>,
    ) -> io::Result<usize> {
        let mut added = 0;
        if self.events as usize == facts::HOLDER_GROUP {
            added += self.close()?;
        }
        if self.events == 0 {
            self.first = endpoint;
        }
        let start = self.open.0.len();
        self.open.u64(endpoint);
        let before = if self.events == 0 { None } else { before };
        if !facts::write_patch(holder, before, now, &mut self.open)? {
            // A group's first event is whole, an absent holder's too.
            self.open.u32(holder);
            self.open.u8(0);
        }
        self.last = endpoint;
        self.events += 1;
        Ok(added + self.open.0.len() - start)
    }

    fn close(&mut self) -> io::Result<usize> {
        if self.events == 0 {
            return Ok(0);
        }
        let bytes = crate::prepared::series::compress(&std::mem::take(&mut self.open.0), 3)?;
        let added = bytes.len();
        self.groups.push(HolderGroup {
            first: self.first,
            last: self.last,
            events: self.events,
            bytes,
        });
        self.events = 0;
        Ok(added)
    }
}

/// The facts of the endpoints completed since the last flush.
#[derive(Default)]
struct Batch {
    first_endpoint: Option<u64>,
    endpoints: Out,
    endpoint_count: u32,
    ranking: Vec<u8>,
    /// Every holder at the batch's last endpoint, by id.
    previous: Option<BTreeMap<u32, Holder>>,
    streams: BTreeMap<u32, HolderStream>,
    naming: Out,
    naming_count: u32,
    /// Each variant's names as last published in this batch.
    named: HashMap<u64, Vec<NameCode>>,
    bytes: usize,
}

impl ProductionAccumulator {
    /// A completed endpoint's facts, joined to the batch's.
    fn complete(&mut self, completed: Pending) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope_phase("production:facts");
        let Pending {
            page, names_hour, ..
        } = completed;
        #[cfg(test)]
        self.completed.push((page.clone(), names_hour));
        let endpoint = self.pages_written;
        self.pages_written = endpoint
            .checked_add(1)
            .ok_or_else(|| io::Error::other("Production page count exceeds its integer range"))?;
        let mut now = BTreeMap::new();
        for row in &page.countries {
            let holder = self.tables.holder(&row.tag);
            let equipment = row
                .equipment
                .as_ref()
                .map(|source| self.facts_source(source, page.equipment_read))
                .transpose()?;
            let rocket = row
                .rocket
                .as_ref()
                .map(|source| self.facts_source(source, page.rocket_read))
                .transpose()?;
            now.insert(
                holder,
                Holder {
                    equipment,
                    rocket,
                    rocket_joined: row.rocket_joined,
                },
            );
        }
        let mut named = Vec::new();
        for (&variant, kind) in &page.classifications {
            let code: Vec<NameCode> = kind
                .names
                .iter()
                .map(|name| NameCode {
                    hour: (Some(name.first_hour) != names_hour).then_some(name.first_hour),
                    raw: name.raw,
                })
                .collect();
            if self.batch.named.get(&variant) != Some(&code) {
                named.push((self.tables.variant(variant), variant, code));
            }
        }
        let batch = &mut self.batch;
        let start = batch.previous.is_none();
        if start {
            batch.first_endpoint = Some(endpoint);
        }
        let previous = batch.previous.take().unwrap_or_default();
        let holders: BTreeSet<u32> = previous
            .keys()
            .chain(now.keys())
            .chain(self.present_holders.iter().filter(|_| start))
            .copied()
            .collect();
        // Ranking (P2): every holder at a keyframe, else what changed.
        let keyframe = batch.endpoint_count as usize % facts::RANKING_KEYFRAME == 0;
        let mut block = Out::default();
        for &holder in &holders {
            let before = if keyframe { None } else { previous.get(&holder) };
            facts::write_patch(holder, before, now.get(&holder), &mut block)?;
        }
        let block = crate::prepared::series::compress(&block.0, 3)?;
        let ranking_offset = batch.ranking.len() as u64;
        let ranking_len = u32::try_from(block.len())
            .map_err(|_| io::Error::other("A production ranking block exceeds 4 GiB"))?;
        batch.ranking.extend_from_slice(&block);
        let before = batch.endpoints.0.len();
        Endpoint {
            first_hour: page.first_hour,
            last_hour: page.last_hour,
            equipment_read: page.equipment_read,
            rocket_read: page.rocket_read,
            equipment_origin: page.equipment_origin,
            equipment_restart: page.equipment_restart,
            rocket_origin: page.rocket_origin,
            rocket_restart: page.rocket_restart,
            names_hour,
            ranking_offset,
            ranking_len,
            keyframe,
        }
        .write(&mut batch.endpoints);
        batch.endpoint_count += 1;
        let mut added = block.len() + batch.endpoints.0.len() - before;
        // Each holder's leaves (P3), wherever they changed; a batch's first
        // endpoint states every holder present then or at the batch before.
        for holder in holders {
            let before = previous.get(&holder);
            let state = now.get(&holder);
            let changed = if start {
                state.is_some() || self.present_holders.contains(&holder)
            } else {
                before != state
            };
            if changed {
                added += batch
                    .streams
                    .entry(holder)
                    .or_default()
                    .push(holder, endpoint, before, state)?;
            }
        }
        // Names (P5), wherever a variant's changed.
        let before = batch.naming.0.len();
        for (index, variant, code) in named {
            batch.naming.u64(endpoint);
            batch.naming.u32(index);
            batch.naming.count(code.len())?;
            for name in &code {
                name.write(&mut batch.naming);
            }
            batch.naming_count += 1;
            batch.named.insert(variant, code);
        }
        added += batch.naming.0.len() - before;
        batch.bytes += added;
        self.present_holders = now.keys().copied().collect();
        batch.previous = Some(now);
        Ok(())
    }

    fn facts_source(
        &mut self,
        source: &SourceEndpoint,
        read: Option<i32>,
    ) -> io::Result<facts::Source> {
        let mut leaves = source
            .amounts
            .iter()
            .map(|amount| {
                Ok(Leaf {
                    variant: self.tables.variant(amount.variant),
                    purpose: amount.purpose,
                    units: facts::at_rest(amount.units)?,
                    units_exact: amount.units_exact,
                    spent: facts::at_rest(amount.spent)?,
                    reason: amount.spending_reason,
                    unproven: facts::at_rest(amount.unproven)?,
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        leaves.sort_unstable_by_key(Leaf::key);
        Ok(facts::Source {
            head: facts::Head {
                slot: source.slot,
                country_id: source.country_id,
                observation: source.observation,
                segment: source.segment,
                read_hour: (Some(source.read_hour) != read).then_some(source.read_hour),
                units_known: source.units_known,
                spending_known: source.spending_known,
                present: source.present,
                campaign_reason: source.campaign_spending_reason,
            },
            leaves,
        })
    }

    /// Completes the open endpoint and appends the batch's facts and spent
    /// series. Appends begin a new endpoint.
    pub fn flush(&mut self, writer: &mut store::Writer, cancelled: &AtomicBool) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope_phase("production:series_flush");
        self.seal_holders();
        if let Some(pending) = self.pending.take() {
            self.complete(pending)?;
        }
        self.spent
            .flush(writer, &spent_section(self.branch), 0, cancelled)?;
        let range = self.facts_range()?;
        writer.write(&section(FACTS, self.branch), range.as_slice())?;
        Ok(())
    }

    /// The batch's range: its preamble, head, ranking blocks and holder groups.
    fn facts_range(&mut self) -> io::Result<Vec<u8>> {
        let mut batch = std::mem::take(&mut self.batch);
        let first_endpoint = batch.first_endpoint.unwrap_or(self.pages_written);
        let mut directory = Out::default();
        let mut groups = Vec::new();
        directory.count(batch.streams.len())?;
        for (holder, mut stream) in std::mem::take(&mut batch.streams) {
            stream.close()?;
            directory.u32(holder);
            directory.count(stream.groups.len())?;
            for group in stream.groups {
                directory.u64(group.first);
                directory.u64(group.last);
                directory.u64(groups.len() as u64);
                directory.count(group.bytes.len())?;
                directory.count(group.events as usize)?;
                groups.extend_from_slice(&group.bytes);
            }
        }
        // Spent series keys (P4) made since the last flush.
        let mut keys = Vec::new();
        for (&(holder, rocket, refits, variant), &key) in &self.spent_keys {
            keys.push((key, 0u8, holder, variant, rocket, refits, Purpose::New));
        }
        for (kind, map) in [(1u8, &self.purpose_keys), (2u8, &self.unproven_keys)] {
            for (&(holder, variant, purpose), &key) in map {
                keys.push((key, kind, holder, variant, false, false, purpose));
            }
        }
        keys.retain(|key| key.0 >= self.keys_written);
        keys.sort_unstable_by_key(|key| key.0);
        let mut key_out = Out::default();
        key_out.count(keys.len())?;
        for (key, kind, holder, variant, rocket, refits, purpose) in keys {
            key_out.u8(kind);
            key_out.u32(holder);
            key_out.u64(variant);
            key_out.bool(rocket);
            key_out.bool(refits);
            key_out.u8(purpose.index() as u8);
            key_out.u64(key);
        }
        self.keys_written = next_key(&self.spent_keys, &self.unproven_keys, &self.purpose_keys);
        // Every variant's classification as of the flush (P5).
        let mut finals = Out::default();
        finals.count(self.classifications.len())?;
        for (&variant, kind) in &self.classifications {
            finals.u32(self.tables.variant(variant));
            finals.u32(self.tables.token(&kind.token));
            finals.u8(seen_mask(&kind.seen));
            finals.count(kind.names.len())?;
            for name in &kind.names {
                NameCode {
                    hour: Some(name.first_hour),
                    raw: name.raw,
                }
                .write(&mut finals);
            }
        }
        // Rocket joins withdrawn since the last flush (P6).
        let mut revocations = Out::default();
        let corrections = std::mem::take(&mut self.joins.corrections);
        #[cfg(test)]
        self.revoked.extend(corrections.iter().cloned());
        revocations.count(corrections.len())?;
        for correction in corrections {
            revocations.u32(self.tables.holder(&correction.tag));
            revocations.u32(correction.slot);
            revocations.u32(correction.country_id);
            revocations.opt_i32(correction.observation);
            revocations.i32(correction.read_hour);
            revocations.i32(correction.last_hour);
        }
        // Diagnostics as of the flush (P7).
        let spending = self.spending_facts();
        let mut diagnostics = Out::default();
        read::write_diagnostics(
            &mut diagnostics,
            &read::Diagnostics {
                counts: self.final_counts.clone(),
                unnamed: self.final_unnamed,
                spending,
                variant_refusals: self.variant_refusals.clone(),
                observation_starts: self.observation_starts(),
            },
        )?;
        // Tables last, as everything above can add to them.
        let mut tables = Out::default();
        self.tables.write_new(&mut tables)?;
        let mut head = Out::default();
        head.region(&tables.0);
        head.u32(batch.endpoint_count);
        head.region(&batch.endpoints.0);
        head.u32(batch.naming_count);
        head.region(&batch.naming.0);
        head.region(&key_out.0);
        head.region(&finals.0);
        head.region(&revocations.0);
        head.region(&diagnostics.0);
        head.region(&directory.0);
        let head = crate::prepared::series::compress(&head.0, 3)?;
        let mut out = Out::default();
        Preamble {
            first_endpoint,
            head_len: head.len() as u64,
            ranking_len: batch.ranking.len() as u64,
            holders_len: groups.len() as u64,
        }
        .write(&mut out);
        out.0.extend_from_slice(&head);
        out.0.extend_from_slice(&batch.ranking);
        out.0.extend_from_slice(&groups);
        Ok(out.0)
    }
}

#[cfg(test)]
impl ProductionAccumulator {
    /// Completes `page`, with its classifications, as an endpoint: for tests
    /// that build pages by hand.
    pub(crate) fn publish_page(
        &mut self,
        page: EndpointPage,
        names_hour: Option<i32>,
    ) -> io::Result<()> {
        for (&variant, kind) in &page.classifications {
            self.classifications.insert(variant, kind.clone());
        }
        self.complete(Pending {
            page,
            names_hour,
            counts: PageCounts::default(),
        })
    }

    /// The raw operands a name refers to.
    pub(crate) fn raw(&self, id: u32) -> &RawVariant {
        &self.tables.raws[id as usize]
    }

    /// Every withdrawal of a Rocket join flushed so far: (tag, slot,
    /// country, observation, supplying read, last hour).
    pub(crate) fn revocations(&self) -> Vec<(String, u32, u32, Option<i32>, i32, i32)> {
        self.revoked
            .iter()
            .map(|correction| {
                (
                    correction.tag.clone(),
                    correction.slot,
                    correction.country_id,
                    correction.observation,
                    correction.read_hour,
                    correction.last_hour,
                )
            })
            .collect()
    }

    /// The spent series keys as (tag, Rocket, refits, design) → key.
    pub(crate) fn spent_keys(&self) -> BTreeMap<(String, bool, bool, u64), u64> {
        self.spent_keys
            .iter()
            .map(|(&(holder, rocket, refits, variant), &key)| {
                (
                    (
                        self.tables.holders[holder as usize].clone(),
                        rocket,
                        refits,
                        variant,
                    ),
                    key,
                )
            })
            .collect()
    }
}

/// Flushes and saves `state` as a resumed preparation would find it.
#[cfg(any(test, feature = "test-support"))]
pub fn write_state(
    writer: &mut store::Writer,
    state: &mut ProductionAccumulator,
) -> io::Result<u64> {
    state.flush(writer, &AtomicBool::new(false))?;
    let bytes = serde_json::to_vec(state)?;
    writer.write_at(&section(STATE, state.branch), 0, bytes.as_slice())?;
    Ok(0)
}

#[cfg(any(test, feature = "test-support"))]
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
    if state.schema != SCHEMA || state.branch != branch {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Prepared production state schema or branch mismatch",
        ));
    }
    Ok(Some(state))
}

#[cfg(any(test, feature = "test-support"))]
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


#[cfg(test)]
mod parity;
#[cfg(test)]
#[path = "prepared_production/tests.rs"]
mod tests;
