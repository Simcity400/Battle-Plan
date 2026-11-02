//! Production's preparation: one observer of every country's production
//! output, whose dated endpoints are published as integer facts ([`facts`])
//! and read through indexes decoded once per publication ([`read`]).

pub mod columnar;
pub(crate) mod facts;
pub(crate) mod input;
pub(crate) mod read;
pub(crate) mod select;
pub(crate) mod served;

pub use facts::FACTS;
pub use select::{SelectedCountry, SelectedValue, SelectionPage, empty_selection, select_at};

use crate::metadata::{TaggedCountry, slot_read};
use crate::products::equipment::production::{
    CostTotal, CountryHolders, Edits, Entry, INVENTORY_READER, Identity, Named,
    ProductionSpendingUnknown, Purpose, ROWS_READER, RocketProduced, RocketRead, Rows,
    SPENT_READER, TagHistory, Total, each_track,
};
use crate::products::equipment::{
    self, RawVariant, Texts, VariantCatalogue, VariantReads, VariantVersion,
};
use crate::products::equipment_produced::{
    ProducedMeasure, RecordingCounts, month_before, recording_warnings_of,
};
use facts::{Endpoint, Holder, Leaf, NameCode, Out, Preamble};
use input::{Changed, Complete, Hour, Lines, Reads, Variants};
use replay_data::{format::FormatVersion, rows::Recorded, rows::TableId, rows::tables};
use replay_frame::{frame::DecodedFrame, frame::TableView};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
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
    /// The read that last saw it ([`Scratch::read`]).
    #[serde(skip)]
    seen: u64,
}

/// The totals entries each identity changed at in one read, with the version its totals had
/// before the read changed them.
#[derive(Default)]
struct Noted {
    read: u64,
    by: HashMap<Identity, (u64, u64, Vec<Entry>)>,
}

impl Noted {
    fn note(&mut self, identity: Identity, before: u64, entry: Entry) {
        let read = self.read;
        let held = self
            .by
            .entry(identity)
            .or_insert_with(|| (read, before, Vec::new()));
        if held.0 != read {
            held.0 = read;
            held.1 = before;
            held.2.clear();
        }
        held.2.push(entry);
    }

    /// The entries `identity`'s totals changed at in this read, where they had version
    /// `before` until it.
    fn since(&self, identity: Identity, before: u64) -> Option<&[Entry]> {
        self.by
            .get(&identity)
            .filter(|held| held.0 == self.read && held.1 == before)
            .map(|held| &held.2[..])
    }
}

/// What the serial lane keeps between hours so that no hour allocates it again.
#[derive(Default)]
struct Scratch {
    /// The output reads ingested; rows a read saw carry its number.
    read: u64,
    units: Noted,
    costs: Noted,
    /// Each canonical slot's identity at the current read.
    by_slot: HashMap<u32, Identity>,
    entries: Vec<Entry>,
    variants: Vec<u64>,
    members: Vec<u64>,
    created: Vec<u64>,
    keys: Vec<u64>,
    old_keys: Vec<u64>,
    amounts: Vec<(u64, [Option<CostTotal>; Purpose::ALL.len()])>,
    rows: Vec<RowEdit>,
    amount_edits: Vec<(Entry, Option<Amount>)>,
    holders: Vec<u32>,
    changes: Vec<HolderChange>,
    leaf_edits: Vec<((u32, usize), Option<Leaf>)>,
    patches: Vec<u8>,
    /// The hour the open page's holders were last extended to.
    sealed: Option<i32>,
    /// Presentation's inventories were built in this session.
    inventoried: bool,
    unclassified: Vec<u64>,
    redated: Vec<u64>,
    row_buffers: RowBuffers,
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
    /// A classification changed since the open endpoint's page was last made or held.
    #[serde(default)]
    classes_moved: bool,
    /// The variants whose classification changed since the open endpoint's names were taken
    /// ([`name_events`]); a variant may be listed more than once.
    #[serde(default)]
    class_changed: Vec<u64>,
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
    /// Each (holder, Rocket, design)'s keys in the key maps, read from them at its first need.
    #[serde(skip)]
    design_keys: HashMap<(u32, bool, u64), DesignKeys>,
    #[serde(skip)]
    spent_holders: [BTreeMap<String, SpentHolder>; 2],
    #[serde(skip)]
    spent_holders_ready: bool,
    #[cfg(test)]
    #[serde(skip)]
    spent_visits: usize,
    /// What the row frames' countries were last compared with.
    #[serde(skip)]
    country_reads: Option<crate::products::territory::CountryReads>,
    /// The hour's countries: none while Countries is not complete.
    #[serde(skip)]
    countries: Option<Arc<CountryHolders>>,
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
    #[serde(skip)]
    scratch: Box<Scratch>,
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

/// The open endpoint: what it publishes, with what is kept beside it. Its page holds no
/// classifications outside tests: they are the accumulator's while it is open.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Pending {
    page: EndpointPage,
    /// The latest complete variant read when it began.
    names_hour: Option<i32>,
    /// What its warnings were rendered from.
    counts: PageCounts,
    /// What changed in each row since the endpoint before it, and the entries those changes
    /// name; none where its rows were not made from that endpoint's.
    #[serde(skip)]
    edited: Option<(Vec<Edited>, Vec<Entry>)>,
    /// The names its completion publishes, taken when it began; none where every name is
    /// compared at its completion.
    #[serde(skip)]
    named: Option<Vec<(u64, Vec<NameCode>)>>,
}

/// What changed in one source of a row since the endpoint before: the whole source, or the
/// entries `entries` names (a range of the page's edited entries).
#[derive(Clone, Copy, Debug, Default)]
struct Edited {
    whole: [bool; 2],
    entries: [(u32, u32); 2],
}

/// A source of a row but its amounts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SourceHead {
    slot: u32,
    country_id: u32,
    observation: Option<i32>,
    segment: u64,
    read_hour: i32,
    units_known: bool,
    spending_known: bool,
    present: Option<bool>,
    campaign_spending_reason: Option<ProductionSpendingUnknown>,
}

impl SourceHead {
    fn of(source: &SourceEndpoint) -> Self {
        Self {
            slot: source.slot,
            country_id: source.country_id,
            observation: source.observation,
            segment: source.segment,
            read_hour: source.read_hour,
            units_known: source.units_known,
            spending_known: source.spending_known,
            present: source.present,
            campaign_spending_reason: source.campaign_spending_reason,
        }
    }

    fn source(self, amounts: Vec<Amount>) -> SourceEndpoint {
        SourceEndpoint {
            slot: self.slot,
            country_id: self.country_id,
            observation: self.observation,
            segment: self.segment,
            read_hour: self.read_hour,
            units_known: self.units_known,
            spending_known: self.spending_known,
            present: self.present,
            campaign_spending_reason: self.campaign_spending_reason,
            amounts,
        }
    }

    fn set(self, source: &mut SourceEndpoint) {
        let amounts = std::mem::take(&mut source.amounts);
        *source = self.source(amounts);
    }
}

/// How one source of a row changes at an hour.
#[derive(Debug)]
enum SourceEdit {
    Same,
    Gone,
    /// A source read whole: new, or of another identity.
    Whole(SourceEndpoint),
    /// Its head, and the amounts a range of the hour's amount edits changes.
    Head(SourceHead, (u32, u32)),
}

/// How the open page's rows change at an hour, in tag order.
#[derive(Debug)]
enum RowEdit {
    /// The open page's row at this place is gone.
    Remove(usize),
    /// A row the open page lacks, before its row at this place.
    Insert(usize, CountryEndpoint),
    /// The open page's row at this place takes these sources and Rocket join.
    Modify {
        held: usize,
        sources: [SourceEdit; 2],
        joined: bool,
    },
}

/// How a holder's facts change at an endpoint.
struct HolderChange {
    holder: u32,
    /// None where the holder is gone.
    now: Option<HolderNow>,
    /// Its patch against the endpoint before, in the scratch patch bytes.
    patch: (usize, usize),
}

struct HolderNow {
    sources: [SourceChange; 2],
    joined: bool,
}

enum SourceChange {
    Absent,
    Whole(facts::Source),
    /// Its head, and the leaves a range of the endpoint's leaf edits changes.
    Leaves(facts::Head, (usize, usize)),
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
    /// Brings each holder's inventory keys to its tracks, visiting only the designs whose
    /// entries changed where the holder's inventory was kept; `changed` gets every variant
    /// whose counts it added to.
    fn inventory(
        &mut self,
        countries: Option<&CountryHolders>,
        tracks: [&mut BTreeMap<String, TagHistory>; 2],
        entries: &mut Vec<Entry>,
        changed: &mut Vec<u64>,
    ) {
        let _cpu = ::replay_frame::cpu::scope_phase("production:inventory_changes");
        changed.clear();
        let Self {
            inventory,
            members,
            #[cfg(test)]
            inventory_visits,
            ..
        } = self;
        for (source, tracks) in tracks.into_iter().enumerate() {
            let held = &mut inventory[source];
            held.retain(|tag, old| {
                let kept = countries.is_some_and(|countries| countries.holders.contains_key(tag))
                    && tracks.contains_key(tag);
                if !kept {
                    change_members(members, &old.members, source, false);
                }
                kept
            });
            for tag in countries
                .into_iter()
                .flat_map(|countries| countries.holders.keys())
            {
                let Some(track) = tracks.get_mut(tag) else {
                    continue;
                };
                let versions = [track.taken, track.cost_taken];
                let whole = track.edits.take(INVENTORY_READER, entries);
                if versions.iter().all(Option::is_some)
                    && held.get(tag).is_some_and(|held| held.versions == versions)
                {
                    continue;
                }
                if !whole
                    && !entries.is_empty()
                    && let Some(kept) = held.get_mut(tag)
                {
                    let mut at = 0;
                    while at < entries.len() {
                        let variant = entries[at].0;
                        while entries.get(at).is_some_and(|entry| entry.0 == variant) {
                            at += 1;
                        }
                        let mut seen = [false; SEEN];
                        let design = (variant, Purpose::New)..=(variant, Purpose::Unknown);
                        let units = track.current.range(design.clone()).map(|(entry, _)| entry);
                        let costs = track.cost_current.range(design).map(|(entry, _)| entry);
                        for (measure, &(_, purpose)) in units
                            .map(|entry| (UNITS_SEEN, entry))
                            .chain(costs.map(|entry| (IC_SEEN, entry)))
                        {
                            #[cfg(test)]
                            {
                                *inventory_visits += 1;
                            }
                            seen[measure + purpose.index()] = true;
                        }
                        let before = kept.members.get(&variant).copied();
                        if before != seen.contains(&true).then_some(seen) {
                            if let Some(before) = before {
                                change_member(members, variant, &before, source, false);
                                kept.members.remove(&variant);
                            }
                            if seen.contains(&true) {
                                change_member(members, variant, &seen, source, true);
                                kept.members.insert(variant, seen);
                            }
                            changed.push(variant);
                        }
                    }
                    kept.versions = versions;
                    continue;
                }
                if let Some(old) = held.get(tag) {
                    change_members(members, &old.members, source, false);
                }
                let mut kept: BTreeMap<u64, [bool; SEEN]> = BTreeMap::new();
                for (measure, &(variant, purpose)) in track
                    .current
                    .keys()
                    .map(|entry| (UNITS_SEEN, entry))
                    .chain(track.cost_current.keys().map(|entry| (IC_SEEN, entry)))
                {
                    #[cfg(test)]
                    {
                        *inventory_visits += 1;
                    }
                    kept.entry(variant).or_default()[measure + purpose.index()] = true;
                }
                change_members(members, &kept, source, true);
                changed.extend(kept.keys().copied());
                let rebuilt = VariantInventory {
                    versions,
                    members: kept,
                };
                match held.get_mut(tag) {
                    Some(held) => *held = rebuilt,
                    None => {
                        held.insert(tag.clone(), rebuilt);
                    }
                }
            }
        }
        changed.sort_unstable();
        changed.dedup();
    }

    /// The names a complete read at `hour` gives: built from `variants` since the read before.
    fn read(
        &mut self,
        hour: i32,
        variants: Variants<'_>,
        refusals: &mut BTreeMap<String, BTreeSet<u64>>,
    ) {
        let Self {
            reads,
            catalogue,
            classified,
            ..
        } = self;
        match variants {
            Variants::Frame(frame) => {
                let built = reads.build(frame, false);
                let live = |id| reads.contains(id);
                name_variants(catalogue, classified, hour, built, live, refusals);
            }
            Variants::Read(pending) => {
                let built = std::mem::take(&mut pending.built);
                let present = &pending.present;
                name_variants(
                    catalogue,
                    classified,
                    hour,
                    built,
                    |id| present.contains(&id),
                    refusals,
                );
            }
        }
    }
}

/// Names `built`'s definitions as read at `hour`, and forgets those `live` no longer holds.
fn name_variants(
    catalogue: &mut VariantCatalogue,
    classified: &mut HashSet<u64>,
    hour: i32,
    built: impl IntoIterator<Item = (u64, RawVariant)>,
    live: impl Fn(u64) -> bool,
    refusals: &mut BTreeMap<String, BTreeSet<u64>>,
) {
    for (id, raw) in built {
        for (_, reason) in &raw.refusals {
            refusals.entry(reason.clone()).or_default().insert(id);
        }
        classified.remove(&id);
        catalogue.versions.insert(
            id,
            vec![VariantVersion {
                first_hour: hour,
                raw: Some(raw),
            }],
        );
    }
    catalogue.versions.retain(|id, _| live(*id));
    classified.retain(|id| live(*id));
}

fn change_members(
    counts: &mut BTreeMap<u64, VariantMembers>,
    members: &BTreeMap<u64, [bool; SEEN]>,
    source: usize,
    add: bool,
) {
    for (&variant, seen) in members {
        change_member(counts, variant, seen, source, add);
    }
}

fn change_member(
    counts: &mut BTreeMap<u64, VariantMembers>,
    variant: u64,
    seen: &[bool; SEEN],
    source: usize,
    add: bool,
) {
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

    /// Each live holder's spent, purpose and unproven series at `hour`. A holder whose identity,
    /// scope and continuity held since its last projection projects only the designs its
    /// entries changed at; the others it holds observe nothing a whole projection would.
    fn observe_spent(&mut self, hour: i32, moved: bool) -> io::Result<()> {
        self.spent.advance(hour)?;
        // Without a moved track, country, taint or restart every live holder's inputs held
        // from the hour before: each projection goes on, as a visit would carry it.
        if !moved
            && self.spent_holders_ready
            && self
                .spent_holders
                .iter()
                .flat_map(|holders| holders.values())
                .all(|held| held.hour.checked_add(1) == Some(hour))
        {
            for held in self
                .spent_holders
                .iter_mut()
                .flat_map(|holders| holders.values_mut())
            {
                held.hour = hour;
            }
            return Ok(());
        }
        let mut opened = BTreeSet::new();
        let mut closed = BTreeSet::new();
        let mut visits = 0usize;
        let Self {
            result,
            first_equipment_holders,
            first_rocket_holders,
            equipment_restart,
            rocket_restart,
            countries,
            spent_holders,
            tables,
            spent_keys,
            unproven_keys,
            purpose_keys,
            design_keys,
            spent,
            spent_active,
            scratch,
            #[cfg(test)]
            spent_visits,
            #[cfg(any(test, feature = "test-support"))]
            work,
            ..
        } = &mut *self;
        let Current {
            tags,
            rocket,
            spending_tainted,
            ..
        } = result;
        let Scratch {
            entries,
            variants,
            keys,
            old_keys,
            amounts,
            ..
        } = &mut **scratch;
        let mut series = SeriesKeys {
            spent: spent_keys,
            unproven: unproven_keys,
            purposes: purpose_keys,
            designs: design_keys,
        };
        let tainted_from = earliest_taint(spending_tainted);
        let mut emitted = 0usize;
        for (rocket_source, tracks, holders, restart) in [
            (false, tags, first_equipment_holders, *equipment_restart),
            (true, &mut rocket.tags, first_rocket_holders, *rocket_restart),
        ] {
            let projected = &mut spent_holders[usize::from(rocket_source)];
            for tag in countries
                .as_ref()
                .into_iter()
                .flat_map(|countries| countries.holders.keys())
            {
                let Some(track) = tracks.get_mut(tag) else {
                    continue;
                };
                #[cfg(any(test, feature = "test-support"))]
                {
                    work.history_visits += 1;
                }
                let Some((identity, at)) = track.last.filter(|(_, at)| *at == hour) else {
                    continue;
                };
                if !holders.contains_key(tag) {
                    holders.insert(tag.clone(), (identity.0, identity.1));
                }
                let scope_reason = spending_reason_from(
                    tainted_from,
                    holders,
                    tag,
                    (identity.0, identity.1),
                    restart,
                    rocket_source,
                    hour,
                );
                let starts = track.starts.last() == Some(&true) && track.hours.last() == Some(&at);
                let inputs = SpentInputs {
                    identity,
                    units: track.taken,
                    costs: track.cost_taken,
                    reason: scope_reason,
                };
                let whole = track.edits.take(SPENT_READER, entries);
                let mut partial = false;
                if !starts
                    && inputs.units.is_some()
                    && inputs.costs.is_some()
                    && let Some(held) = projected.get_mut(tag)
                    && held.hour.checked_add(1) == Some(hour)
                {
                    if held.inputs == inputs {
                        held.hour = hour;
                        continue;
                    }
                    partial = !whole
                        && !entries.is_empty()
                        && held.inputs.identity == inputs.identity
                        && held.inputs.reason == inputs.reason;
                }
                let holder = tables.holder(tag);
                if partial {
                    let held = projected.get_mut(tag).expect("a projected holder");
                    variants.clear();
                    variants.extend(entries.iter().map(|entry| entry.0));
                    variants.dedup();
                    for &variant in variants.iter() {
                        old_keys.clear();
                        for key in series.of(holder, rocket_source, variant) {
                            if held.active.remove(&key) {
                                old_keys.push(key);
                            }
                        }
                        keys.clear();
                        if let Some(amounts) = design_amounts(track, variant, &mut visits) {
                            emitted += project_design(
                                &mut series,
                                spent,
                                spent_active,
                                keys,
                                (holder, rocket_source, variant),
                                &amounts,
                                (scope_reason, starts, hour),
                            )?;
                        }
                        for &key in keys.iter() {
                            held.active.insert(key);
                            opened.insert(key);
                        }
                        closed.extend(old_keys.iter().copied().filter(|key| !keys.contains(key)));
                    }
                    held.inputs = inputs;
                    held.hour = hour;
                    continue;
                }
                keys.clear();
                design_inventory(track, amounts, &mut visits);
                for (variant, amounts) in amounts.iter() {
                    emitted += project_design(
                        &mut series,
                        spent,
                        spent_active,
                        keys,
                        (holder, rocket_source, *variant),
                        amounts,
                        (scope_reason, starts, hour),
                    )?;
                }
                let active: BTreeSet<u64> = keys.iter().copied().collect();
                opened.extend(active.iter().copied());
                let held = SpentHolder {
                    inputs,
                    hour,
                    active,
                };
                match projected.get_mut(tag) {
                    Some(previous) => {
                        closed.extend(previous.active.difference(&held.active).copied());
                        *previous = held;
                    }
                    None => {
                        projected.insert(tag.clone(), held);
                    }
                }
            }
        }
        #[cfg(test)]
        {
            *spent_visits += visits;
        }
        #[cfg(not(test))]
        let _ = visits;
        #[cfg(any(test, feature = "test-support"))]
        {
            work.emitted += emitted;
        }
        #[cfg(not(any(test, feature = "test-support")))]
        let _ = emitted;
        for holders in spent_holders.iter_mut() {
            holders.retain(|_, held| {
                if held.hour == hour {
                    return true;
                }
                closed.extend(held.active.iter().copied());
                false
            });
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
        self.scratch.sealed = None;
    }
    fn forget_names(&mut self, hour: i32) {
        self.variants.classified.clear();
        for (&variant, kind) in &mut self.classifications {
            if kind.names.last().is_some_and(|name| name.raw.is_none()) {
                continue;
            }
            kind.names = vec![VariantName {
                first_hour: hour,
                raw: None,
            }];
            self.classes_moved = true;
            self.class_changed.push(variant);
        }
    }
    fn observe(
        &mut self,
        input: &Hour<'_>,
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
            countries,
            scratch,
            #[cfg(any(test, feature = "test-support"))]
            work,
            ..
        } = self;
        let hour = input.hour;
        let complete = input.complete;
        // Rocket rows retain every country/design total; they do not depend
        // on a registered line or on the equipment census being readable.
        if rocket_changed {
            let read = RocketRead {
                hour,
                read: input.reads.rocket,
                complete: complete.countries && complete.links && complete.rocket,
                rows: input.rocket,
            };
            result
                .rocket
                .observe_resolved(&read, countries.as_deref(), true)?;
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
            each_track(
                &mut result.tags,
                result.active.iter().map(String::as_str),
                |track| {
                    if let Some((_, at)) = &mut track.last {
                        *at = hour;
                    }
                    if let Some(at) = track.last_hours.last_mut() {
                        *at = hour;
                    }
                },
            );
            return Ok(());
        }

        let read = input.reads.output;
        let output = input.output;
        // An hour the tables cannot tell extends nothing, so nothing continues across it.
        if !(complete.countries && complete.links && complete.output) {
            if read {
                // The publication this read held, if any, is not ingested: whatever
                // lines ended in it are lost, for a producer that cannot be told.
                result.taint(*observation, hour);
                if complete.lines {
                    result.unread += 1;
                }
                if result.unread_reason.is_none()
                    && let Some(reason) = input.output_reason
                {
                    result.unread_reason = Some(reason.to_owned());
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
        }
        let reads = countries.as_deref().expect("resolved production countries");
        result.unnamed += reads.unnamed;
        if !read {
            // Carried: the last read still holds, only for the same identity.
            let observation = *observation;
            result.active.retain(|tag| {
                let track = result.tags.get_mut(tag).expect("active equipment holder");
                let continues = reads
                    .holders
                    .get(tag)
                    .copied()
                    .flatten()
                    .map(|(slot, id)| (slot, id, observation))
                    .filter(|identity| {
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
        if !*spending_recorded {
            *spending_recorded = output
                .iter()
                .any(|row| !matches!(row.spent_raw, Recorded::NotRecorded));
        }
        if !*spending_recorded {
            result.spending_taint(*observation, hour, ProductionSpendingUnknown::NotRecorded);
        }
        let observation = *observation;
        let Scratch {
            read: reads_ingested,
            units: units_changed,
            costs: costs_changed,
            by_slot,
            ..
        } = &mut **scratch;
        *reads_ingested += 1;
        let seen = *reads_ingested;
        units_changed.read = seen;
        costs_changed.read = seen;
        // A slot two tags name is the later tag's, as the tags are met in order.
        by_slot.clear();
        for holder in reads.holders.values() {
            if let Some((slot, id)) = *holder {
                by_slot.insert(slot, (slot, id, observation));
            }
        }
        let mut presence_unknown = false;
        let census_complete = if complete.lines {
            for country in input.lines.countries() {
                #[cfg(any(test, feature = "test-support"))]
                {
                    work.row_visits += 1;
                }
                if let Some(identity) = reads.slots.get(&country).and_then(|slot| by_slot.get(slot))
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
        for row in output.iter() {
            #[cfg(any(test, feature = "test-support"))]
            {
                work.row_visits += 1;
            }
            let key = (row.line, row.ordinal);
            let purpose = Purpose::of(row);
            let proved = purpose != Purpose::Unknown;
            if !proved {
                unlisted.insert(row.line);
            }
            let producer = reads
                .slots
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
                    seen: 0,
                }
            });
            state.seen = seen;
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
                result.taint(observation, hour);
            }
            if state.identity.is_none() && !matches!(row.spent_raw, Recorded::Value(Some(0))) {
                result.spending_taint(
                    observation,
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
            apply_cost(
                cost_totals,
                cost_version,
                state,
                delta,
                spending_reason,
                costs_changed,
            )?;
            if let Recorded::Value(Some(value)) = row.spent_raw {
                state.spent_raw = value;
            }
            apply(totals, version, state, row.units, exact, units_changed);
            state.units = row.units;
            state.exact = exact;
            state.ended |= row.ended;
            // Rows still expected at the next read: those it saw that did not end.
            if state.ended {
                live.remove(&key);
            } else {
                live.insert(key);
            }
        }
        // A row gone without its ended publication lost its final count.
        let mut failed = None;
        live.retain(|key| {
            let Some(state) = rows.get_mut(key) else {
                return false;
            };
            if state.seen == seen || failed.is_some() {
                return state.seen == seen;
            }
            result.lost += usize::from(state.exact);
            let units = state.units;
            if let Err(error) = apply_cost(
                cost_totals,
                cost_version,
                state,
                0,
                Some(ProductionSpendingUnknown::LostOutput),
                costs_changed,
            ) {
                failed = Some(error);
                return false;
            }
            apply(totals, version, state, units, false, units_changed);
            state.exact = false;
            false
        });
        if let Some(error) = failed {
            return Err(error);
        }
        // Each tag's sample: its country's totals at this read.
        result.active.retain(|tag| {
            let present = matches!(reads.holders.get(tag), Some(Some(_)));
            if !present {
                result
                    .tags
                    .get_mut(tag)
                    .expect("active equipment holder")
                    .last = None;
            }
            present
        });
        for (tag, holder) in &reads.holders {
            let Some((slot, id)) = *holder else {
                continue;
            };
            let identity = (slot, id, observation);
            if !result.active.contains(tag) {
                result.active.insert(tag.clone());
            }
            if !result.tags.contains_key(tag) {
                result.tags.insert(tag.clone(), TagHistory::default());
            }
            let track = result.tags.get_mut(tag).expect("an observed tag's track");
            let starts = !track
                .last
                .is_some_and(|(seen, at)| seen == identity && at.checked_add(1) == Some(hour));
            u32::try_from(track.hours.len()).map_err(|_| {
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
                take_totals(
                    &mut track.cost_current,
                    &mut track.edits,
                    track.cost_taken,
                    identity,
                    cost_entries,
                    costs_changed,
                );
                track.cost_taken = Some((identity, cost_changed));
            }
            let (changed, entries) = totals
                .get(&identity)
                .map_or((0, None), |(changed, entries)| (*changed, Some(entries)));
            if track.taken != Some((identity, changed)) {
                take_totals(
                    &mut track.current,
                    &mut track.edits,
                    track.taken,
                    identity,
                    entries,
                    units_changed,
                );
                track.taken = Some((identity, changed));
            }
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

/// One (holder, Rocket, design)'s keys in the key maps: spent without and with refits, each
/// [`SERIES_PURPOSES`] purpose's and each purpose's unproven key; none of the last two for
/// Rocket.
#[derive(Clone, Copy, Default)]
struct DesignKeys {
    spent: [Option<u64>; 2],
    purposes: [Option<u64>; SERIES_PURPOSES.len()],
    unproven: [Option<u64>; Purpose::ALL.len()],
}

/// The spent, purpose and unproven series keys, which share one numbering, and each design's
/// keys among them. Keys are only ever added, through [`Self::spent_key`],
/// [`Self::purpose_key`] and [`Self::unproven_key`], so a design's keys stay what the maps hold.
struct SeriesKeys<'a> {
    spent: &'a mut BTreeMap<SpentKey, u64>,
    unproven: &'a mut BTreeMap<PurposeKey, u64>,
    purposes: &'a mut BTreeMap<PurposeKey, u64>,
    designs: &'a mut HashMap<(u32, bool, u64), DesignKeys>,
}

impl SeriesKeys<'_> {
    fn next(&self) -> u64 {
        next_key(self.spent, self.unproven, self.purposes)
    }

    /// `variant`'s keys for `holder`, read from the maps at its first need.
    fn design(&mut self, holder: u32, rocket: bool, variant: u64) -> &mut DesignKeys {
        let Self {
            spent,
            unproven,
            purposes,
            designs,
        } = self;
        designs
            .entry((holder, rocket, variant))
            .or_insert_with(|| DesignKeys {
                spent: [false, true]
                    .map(|refits| spent.get(&(holder, rocket, refits, variant)).copied()),
                purposes: SERIES_PURPOSES.map(|purpose| {
                    (!rocket)
                        .then(|| purposes.get(&(holder, variant, purpose)).copied())
                        .flatten()
                }),
                unproven: Purpose::ALL.map(|purpose| {
                    (!rocket)
                        .then(|| unproven.get(&(holder, variant, purpose)).copied())
                        .flatten()
                }),
            })
    }

    /// Every key a holder's projection of `variant` may hold.
    fn of(&mut self, holder: u32, rocket: bool, variant: u64) -> impl Iterator<Item = u64> + use<> {
        let design = *self.design(holder, rocket, variant);
        design
            .spent
            .into_iter()
            .chain(design.purposes)
            .chain(design.unproven)
            .flatten()
    }

    /// The spent key of (`holder`, `rocket`, `refits`, `variant`), the next one where it has none.
    fn spent_key(&mut self, holder: u32, rocket: bool, refits: bool, variant: u64) -> u64 {
        if let Some(key) = self.design(holder, rocket, variant).spent[usize::from(refits)] {
            return key;
        }
        let next = self.next();
        let key = *self
            .spent
            .entry((holder, rocket, refits, variant))
            .or_insert(next);
        self.design(holder, rocket, variant).spent[usize::from(refits)] = Some(key);
        key
    }

    /// The key of equipment design `variant`'s input of `purpose`, one of [`SERIES_PURPOSES`],
    /// the next one where it has none.
    fn purpose_key(&mut self, holder: u32, variant: u64, purpose: Purpose) -> io::Result<u64> {
        let at = SERIES_PURPOSES
            .iter()
            .position(|&series| series == purpose)
            .ok_or_else(|| io::Error::other("A purpose without its own series"))?;
        if let Some(key) = self.design(holder, false, variant).purposes[at] {
            return Ok(key);
        }
        let next = self.next();
        let key = *self
            .purposes
            .entry((holder, variant, purpose))
            .or_insert(next);
        self.design(holder, false, variant).purposes[at] = Some(key);
        Ok(key)
    }

    /// Equipment design `variant`'s unproven key of `purpose`, if it has one.
    fn unproven(&mut self, holder: u32, variant: u64, purpose: Purpose) -> Option<u64> {
        self.design(holder, false, variant).unproven[purpose.index()]
    }

    /// Equipment design `variant`'s unproven key of `purpose`, the next one where it has none.
    fn unproven_key(&mut self, holder: u32, variant: u64, purpose: Purpose) -> u64 {
        if let Some(key) = self.unproven(holder, variant, purpose) {
            return key;
        }
        let next = self.next();
        let key = *self
            .unproven
            .entry((holder, variant, purpose))
            .or_insert(next);
        self.design(holder, false, variant).unproven[purpose.index()] = Some(key);
        key
    }
}

/// A design's spending of each purpose in `track`, for the purposes either inventory holds;
/// none where neither holds the design.
fn design_amounts(
    track: &TagHistory,
    variant: u64,
    visits: &mut usize,
) -> Option<[Option<CostTotal>; Purpose::ALL.len()]> {
    let mut amounts = [None; Purpose::ALL.len()];
    let design = (variant, Purpose::New)..=(variant, Purpose::Unknown);
    for (&(_, purpose), _) in track.current.range(design.clone()) {
        *visits += 1;
        amounts[purpose.index()] = Some(CostTotal::default());
    }
    for (&(_, purpose), total) in track.cost_current.range(design) {
        *visits += 1;
        amounts[purpose.index()] = Some(*total);
    }
    amounts.iter().any(Option::is_some).then_some(amounts)
}

/// Every design either inventory of `track` holds, ascending, with [`design_amounts`].
fn design_inventory(
    track: &TagHistory,
    out: &mut Vec<(u64, [Option<CostTotal>; Purpose::ALL.len()])>,
    visits: &mut usize,
) {
    out.clear();
    for &(variant, purpose) in track.current.keys().chain(track.cost_current.keys()) {
        *visits += 1;
        let at = match out.binary_search_by_key(&variant, |(held, _)| *held) {
            Ok(at) => at,
            Err(at) => {
                out.insert(at, (variant, [None; Purpose::ALL.len()]));
                at
            }
        };
        out[at].1[purpose.index()] = Some(
            track
                .cost_current
                .get(&(variant, purpose))
                .copied()
                .unwrap_or_default(),
        );
    }
}

/// Projects one design of a holder at an hour: its spent series with and without refits,
/// and for equipment its purpose and unproven series, noting in `known` each key it holds a
/// known value of. Returns how many observations it made.
fn project_design(
    series: &mut SeriesKeys<'_>,
    spent: &mut crate::prepared::series::Accumulator,
    active: &BTreeSet<u64>,
    known: &mut Vec<u64>,
    (holder, rocket, variant): (u32, bool, u64),
    amounts: &[Option<CostTotal>; Purpose::ALL.len()],
    (scope_reason, starts, hour): (Option<ProductionSpendingUnknown>, bool, i32),
) -> io::Result<usize> {
    let mut emitted = 0;
    for refits in if rocket { &[true][..] } else { &[false, true][..] } {
        // Without refits: designs with input of another purpose.
        if !*refits
            && Purpose::ALL
                .into_iter()
                .filter(|&purpose| purpose != Purpose::Refit)
                .all(|purpose| amounts[purpose.index()].is_none())
        {
            continue;
        }
        let key = series.spent_key(holder, rocket, *refits, variant);
        let counted = Purpose::ALL
            .into_iter()
            .filter(|purpose| *refits || *purpose != Purpose::Refit)
            .filter_map(|purpose| amounts[purpose.index()].as_ref());
        let value = known_sum(scope_reason, counted)?;
        emitted += usize::from(observe_known(
            spent,
            active,
            known,
            (key, hour, value, starts),
        ));
    }
    if rocket {
        return Ok(emitted);
    }
    // Converted and unrecorded-purpose input, for designs that ever had it:
    // months read each apart from new production in the series without refits.
    for purpose in SERIES_PURPOSES {
        let Some(total) = &amounts[purpose.index()] else {
            continue;
        };
        let key = series.purpose_key(holder, variant, purpose)?;
        let value = known_sum(scope_reason, [total])?;
        emitted += usize::from(observe_known(
            spent,
            active,
            known,
            (key, hour, value, starts),
        ));
    }
    // Input whose purpose is not recorded, per purpose, for entries that ever
    // had it: ranges and months read its change over their own interval.
    for (purpose, total) in Purpose::ALL
        .into_iter()
        .filter_map(|purpose| amounts[purpose.index()].map(|total| (purpose, total)))
    {
        if total.unproven_raw == 0 && series.unproven(holder, variant, purpose).is_none() {
            continue;
        }
        let key = series.unproven_key(holder, variant, purpose);
        let value = scope_reason
            .or(total.reason)
            .is_none()
            .then_some(total.unproven_raw);
        emitted += usize::from(observe_known(
            spent,
            active,
            known,
            (key, hour, value, starts),
        ));
    }
    Ok(emitted)
}

/// Observes a known value of a holder's key when it is new or changed;
/// true when it was observed. A known value keeps the key active.
fn observe_known(
    spent: &mut crate::prepared::series::Accumulator,
    active: &BTreeSet<u64>,
    observed: &mut Vec<u64>,
    (key, hour, value, starts): (u64, i32, Option<i128>, bool),
) -> bool {
    if value.is_none() {
        return false;
    }
    observed.push(key);
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
    noted: &mut Noted,
) {
    let Some(identity) = state.identity else {
        return;
    };
    if units == state.units && exact == state.exact {
        return;
    }
    *version += 1;
    let (changed, entries) = totals.entry(identity).or_default();
    noted.note(identity, *changed, state.entry);
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
    noted: &mut Noted,
) -> io::Result<()> {
    let Some(identity) = state.identity else {
        return Ok(());
    };
    if delta == 0 && reason.is_none() {
        return Ok(());
    }
    *version += 1;
    let (changed, entries) = totals.entry(identity).or_default();
    noted.note(identity, *changed, state.entry);
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

/// Brings `current`, a track's inventory taken at `taken`, to `entries`, `identity`'s totals:
/// at only the entries this read changed where it took them just before, else whole.
fn take_totals<V: Copy>(
    current: &mut BTreeMap<Entry, V>,
    edits: &mut Edits,
    taken: Option<(Identity, u64)>,
    identity: Identity,
    entries: Option<&BTreeMap<Entry, V>>,
    changed: &Noted,
) {
    if let (Some((held, version)), Some(entries)) = (taken, entries)
        && held == identity
        && let Some(changed) = changed.since(identity, version)
    {
        for &entry in changed {
            match entries.get(&entry) {
                Some(value) => {
                    current.insert(entry, *value);
                }
                None => {
                    current.remove(&entry);
                }
            }
            edits.note(entry);
        }
        return;
    }
    match entries {
        Some(entries) => current.clone_from(entries),
        None => current.clear(),
    }
    edits.replaced();
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
    spending_tainted: &BTreeMap<u32, (i32, ProductionSpendingUnknown)>,
    first: &BTreeMap<String, (u32, u32)>,
    tag: &str,
    identity: (u32, u32),
    restart: Option<i32>,
    rocket: bool,
    hour: i32,
) -> Option<ProductionSpendingUnknown> {
    spending_reason_from(
        earliest_taint(spending_tainted),
        first,
        tag,
        identity,
        restart,
        rocket,
        hour,
    )
}

/// The earliest campaign spending taint: its hour and reason.
fn earliest_taint(
    spending_tainted: &BTreeMap<u32, (i32, ProductionSpendingUnknown)>,
) -> Option<(i32, ProductionSpendingUnknown)> {
    spending_tainted
        .values()
        .min_by_key(|(at, _)| *at)
        .copied()
}

/// [`spending_reason`] from the earliest taint ([`earliest_taint`]).
fn spending_reason_from(
    earliest: Option<(i32, ProductionSpendingUnknown)>,
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
        earliest
            .filter(|(at, _)| hour >= *at)
            .map(|(_, reason)| reason)
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
/// A live source's row at `hour` but its amounts and campaign spending reason: none where the
/// track does not hold through `hour`. A read that starts a line opens another continuity
/// segment.
fn source_head(
    track: &TagHistory,
    hour: i32,
    origin: Option<i32>,
    segments: &mut BTreeMap<String, u64>,
    tag: &str,
    units_known: bool,
    spending_known: bool,
) -> io::Result<Option<SourceHead>> {
    let Some((identity, last)) = track.last else {
        return Ok(None);
    };
    if last != hour {
        return Ok(None);
    }
    let read_hour = *track.hours.last().expect("live production read");
    if !segments.contains_key(tag) {
        segments.insert(tag.to_owned(), 0);
    }
    let segment = segments.get_mut(tag).expect("a holder's continuity count");
    if read_hour == hour && track.starts.last() == Some(&true) {
        *segment = segment.checked_add(1).ok_or_else(|| {
            io::Error::other("Production continuity count exceeds its integer range")
        })?;
    }
    Ok(Some(SourceHead {
        slot: identity.0,
        country_id: identity.1,
        observation: origin,
        segment: *segment,
        read_hour,
        units_known,
        spending_known,
        present: track.presence.last().copied().flatten(),
        campaign_spending_reason: None,
    }))
}

/// `entry`'s amount in `track`: none where neither inventory holds it.
fn amount_of(track: &TagHistory, entry: Entry) -> Option<Amount> {
    let (units, spending) = (track.current.get(&entry), track.cost_current.get(&entry));
    if units.is_none() && spending.is_none() {
        return None;
    }
    let units = units.copied().unwrap_or_default();
    let spending = spending.copied().unwrap_or_default();
    Some(Amount {
        variant: entry.0,
        purpose: entry.1,
        units: units.units,
        units_exact: units.inexact == 0,
        spent: spending.raw,
        spending_reason: spending.reason,
        unproven: spending.unproven_raw,
    })
}

/// Every amount of `track`, in entry order.
fn amounts_of(track: &TagHistory) -> Vec<Amount> {
    let entries: BTreeSet<Entry> = track
        .current
        .keys()
        .chain(track.cost_current.keys())
        .copied()
        .collect();
    entries
        .into_iter()
        .filter_map(|entry| amount_of(track, entry))
        .collect()
}

/// The amount of `entry` in `amounts`, which are in entry order.
fn held_amount(amounts: &[Amount], entry: Entry) -> Result<usize, usize> {
    amounts.binary_search_by_key(&entry, |amount| (amount.variant, amount.purpose))
}

/// What a source's campaign spending reason is read from: the spending taints, the first
/// holder of each tag, the observation restart, and whether it is the Rocket source.
type Scope<'a> = (
    &'a BTreeMap<u32, (i32, ProductionSpendingUnknown)>,
    &'a BTreeMap<String, (u32, u32)>,
    Option<i32>,
    bool,
);

/// A row's source at `hour` against the open page's, `held`: its head, and how it changed.
/// The amounts are read at only the entries the track changed since the rows last read it,
/// unless it was replaced or the open page lacks the source.
#[allow(clippy::too_many_arguments)]
fn source_edit(
    track: Option<&mut TagHistory>,
    hour: i32,
    origin: Option<i32>,
    segments: &mut BTreeMap<String, u64>,
    tag: &str,
    (units_known, spending_known): (bool, bool),
    (tainted, first, restart, rocket): Scope<'_>,
    held: Option<&SourceEndpoint>,
    entries: &mut Vec<Entry>,
    edits: &mut Vec<(Entry, Option<Amount>)>,
) -> io::Result<(Option<SourceHead>, SourceEdit)> {
    let gone = if held.is_some() {
        SourceEdit::Gone
    } else {
        SourceEdit::Same
    };
    let Some(track) = track else {
        return Ok((None, gone));
    };
    let whole = track.edits.take(ROWS_READER, entries);
    let Some(mut head) = source_head(
        track,
        hour,
        origin,
        segments,
        tag,
        units_known,
        spending_known,
    )?
    else {
        return Ok((None, gone));
    };
    head.campaign_spending_reason = spending_reason(
        tainted,
        first,
        tag,
        (head.slot, head.country_id),
        restart,
        rocket,
        hour,
    );
    let Some(held) = held else {
        return Ok((Some(head), SourceEdit::Whole(head.source(amounts_of(track)))));
    };
    if whole {
        let source = head.source(amounts_of(track));
        let edit = if source == *held {
            SourceEdit::Same
        } else {
            SourceEdit::Whole(source)
        };
        return Ok((Some(head), edit));
    }
    let first = edits.len();
    for &entry in entries.iter() {
        let now = amount_of(track, entry);
        let before = held_amount(&held.amounts, entry)
            .ok()
            .map(|at| &held.amounts[at]);
        if now.as_ref() != before {
            edits.push((entry, now));
        }
    }
    let edit = if first == edits.len() && head == SourceHead::of(held) {
        SourceEdit::Same
    } else {
        SourceEdit::Head(head, (first as u32, edits.len() as u32))
    };
    Ok((Some(head), edit))
}

/// A source the open page lacks, read whole by [`source_edit`].
fn whole_source(edit: SourceEdit) -> Option<SourceEndpoint> {
    match edit {
        SourceEdit::Whole(source) => Some(source),
        _ => None,
    }
}

/// The buffers a page's row edits are applied with, kept between endpoints.
#[derive(Default)]
struct RowBuffers {
    /// The rows a page that gains or loses rows is rebuilt into.
    spare: Vec<CountryEndpoint>,
    edited: Vec<Edited>,
    entries: Vec<Entry>,
}

/// Changes `rows`, the open page's in tag order, by `edits`; `edited` gets what changed in
/// each row and `entries` the entries those changes name. Rows are edited in place unless a
/// row is added or removed, when they move once into the spare buffer, which takes the old.
fn apply_rows(
    rows: &mut Vec<CountryEndpoint>,
    buffers: &mut RowBuffers,
    edits: &mut Vec<RowEdit>,
    amounts: &[(Entry, Option<Amount>)],
) {
    let RowBuffers {
        spare,
        edited,
        entries,
    } = buffers;
    edited.clear();
    entries.clear();
    let inserted = |row: &CountryEndpoint| Edited {
        whole: [row.equipment.is_some(), row.rocket.is_some()],
        entries: [(0, 0); 2],
    };
    if edits
        .iter()
        .all(|edit| matches!(edit, RowEdit::Modify { .. }))
    {
        edited.resize(rows.len(), Edited::default());
        for edit in edits.drain(..) {
            let RowEdit::Modify {
                held,
                sources,
                joined,
            } = edit
            else {
                unreachable!("only changes");
            };
            edited[held] = modify(&mut rows[held], sources, joined, amounts, entries);
        }
        return;
    }
    spare.clear();
    spare.reserve(rows.len() + edits.len());
    let mut edits = edits.drain(..).peekable();
    for (at, mut row) in rows.drain(..).enumerate() {
        while let Some(RowEdit::Insert(before, _)) = edits.peek()
            && *before == at
        {
            let Some(RowEdit::Insert(_, new)) = edits.next() else {
                unreachable!("a peeked insertion");
            };
            edited.push(inserted(&new));
            spare.push(new);
        }
        match edits.peek() {
            Some(RowEdit::Remove(gone)) if *gone == at => {
                edits.next();
            }
            Some(RowEdit::Modify { held, .. }) if *held == at => {
                let Some(RowEdit::Modify {
                    sources, joined, ..
                }) = edits.next()
                else {
                    unreachable!("a peeked change");
                };
                edited.push(modify(&mut row, sources, joined, amounts, entries));
                spare.push(row);
            }
            _ => {
                edited.push(Edited::default());
                spare.push(row);
            }
        }
    }
    for edit in edits {
        if let RowEdit::Insert(_, new) = edit {
            edited.push(inserted(&new));
            spare.push(new);
        }
    }
    std::mem::swap(rows, spare);
}

/// Changes one row by its sources' edits and Rocket join; what changed in it, its entries
/// appended to `entries`.
fn modify(
    row: &mut CountryEndpoint,
    sources: [SourceEdit; 2],
    joined: bool,
    amounts: &[(Entry, Option<Amount>)],
    entries: &mut Vec<Entry>,
) -> Edited {
    let mut edit = Edited::default();
    for (slot, (source, change)) in [&mut row.equipment, &mut row.rocket]
        .into_iter()
        .zip(sources)
        .enumerate()
    {
        match change {
            SourceEdit::Same => {}
            SourceEdit::Gone => *source = None,
            SourceEdit::Whole(whole) => {
                *source = Some(whole);
                edit.whole[slot] = true;
            }
            SourceEdit::Head(head, (first, last)) => {
                let source = source.as_mut().expect("a held source's head");
                head.set(source);
                let start = entries.len() as u32;
                for (entry, amount) in &amounts[first as usize..last as usize] {
                    entries.push(*entry);
                    match (held_amount(&source.amounts, *entry), amount) {
                        (Ok(at), Some(amount)) => source.amounts[at] = amount.clone(),
                        (Ok(at), None) => {
                            source.amounts.remove(at);
                        }
                        (Err(at), Some(amount)) => source.amounts.insert(at, amount.clone()),
                        (Err(_), None) => {}
                    }
                }
                edit.entries[slot] = (start, entries.len() as u32);
            }
        }
    }
    row.rocket_joined = joined;
    edit
}

/// A source's facts head at an endpoint whose read of it is `read`.
fn facts_head(source: &SourceEndpoint, read: Option<i32>) -> facts::Head {
    facts::Head {
        slot: source.slot,
        country_id: source.country_id,
        observation: source.observation,
        segment: source.segment,
        read_hour: (Some(source.read_hour) != read).then_some(source.read_hour),
        units_known: source.units_known,
        spending_known: source.spending_known,
        present: source.present,
        campaign_reason: source.campaign_spending_reason,
    }
}

/// An amount's leaf, its variant `variant` in the variant table.
fn leaf_of(variant: u32, amount: &Amount) -> io::Result<Leaf> {
    Ok(Leaf {
        variant,
        purpose: amount.purpose,
        units: facts::at_rest(amount.units)?,
        units_exact: amount.units_exact,
        spent: facts::at_rest(amount.spent)?,
        reason: amount.spending_reason,
        unproven: facts::at_rest(amount.unproven)?,
    })
}

/// A source's facts, read whole: each amount's variant takes its table place in entry order.
fn facts_source(
    tables: &mut Tables,
    source: &SourceEndpoint,
    read: Option<i32>,
) -> io::Result<facts::Source> {
    let mut leaves = source
        .amounts
        .iter()
        .map(|amount| leaf_of(tables.variant(amount.variant), amount))
        .collect::<io::Result<Vec<_>>>()?;
    leaves.sort_unstable_by_key(Leaf::key);
    Ok(facts::Source {
        head: facts_head(source, read),
        leaves,
    })
}

/// The names an endpoint dated by `names_hour` publishes of `kinds`: each variant's whose codes
/// differ from those its batch last published. Where `changed` lists every variant changed
/// since the batch's last endpoint, of the same names hour, only those are compared; it is
/// emptied.
fn name_events(
    kinds: &BTreeMap<u64, Classification>,
    changed: Option<&mut Vec<u64>>,
    names_hour: Option<i32>,
    batch: &Batch,
) -> Vec<(u64, Vec<NameCode>)> {
    let code = |name: &VariantName| NameCode {
        hour: (Some(name.first_hour) != names_hour).then_some(name.first_hour),
        raw: name.raw,
    };
    let mut events = Vec::new();
    let mut compare = |variant: u64, kind: &Classification, held: Option<&Vec<NameCode>>| {
        let differs = held.is_none_or(|held| {
            held.len() != kind.names.len()
                || held.iter().zip(&kind.names).any(|(held, name)| *held != code(name))
        });
        if differs {
            events.push((variant, kind.names.iter().map(code).collect()));
        }
    };
    let since = batch.started && batch.names_hour == Some(names_hour);
    match changed {
        Some(changed) if since => {
            changed.sort_unstable();
            changed.dedup();
            for &variant in changed.iter() {
                if let Some(kind) = kinds.get(&variant) {
                    compare(variant, kind, batch.named.get(&variant));
                }
            }
            changed.clear();
        }
        changed => {
            let mut named = batch.named.iter().peekable();
            for (&variant, kind) in kinds {
                while named.peek().is_some_and(|(held, _)| **held < variant) {
                    named.next();
                }
                let held = named
                    .peek()
                    .filter(|(held, _)| **held == variant)
                    .map(|(_, codes)| *codes);
                compare(variant, kind, held);
            }
            if let Some(changed) = changed {
                changed.clear();
            }
        }
    }
    events
}

/// The accumulator's classifications as presentation changes them: each real change is noted
/// for the open page and for the names endpoints publish.
struct ClassEdits<'a> {
    held: &'a mut BTreeMap<u64, Classification>,
    moved: &'a mut bool,
    changed: &'a mut Vec<u64>,
}

impl ClassEdits<'_> {
    fn mark(&mut self, variant: u64) {
        *self.moved = true;
        self.changed.push(variant);
    }

    /// Dates every name of `variant` at `hour`.
    fn redate(&mut self, variant: u64, hour: i32) {
        let Some(kind) = self.held.get_mut(&variant) else {
            return;
        };
        if kind.names.iter().all(|name| name.first_hour == hour) {
            return;
        }
        for name in &mut kind.names {
            name.first_hour = hour;
        }
        self.mark(variant);
    }

    fn set(&mut self, variant: u64, kind: Classification) {
        if self.held.get(&variant) == Some(&kind) {
            return;
        }
        self.held.insert(variant, kind);
        self.mark(variant);
    }

    /// Marks `variant` seen by each inventory `counts` holds it in.
    fn see(&mut self, variant: u64, counts: &[usize; SEEN]) {
        let Some(kind) = self.held.get_mut(&variant) else {
            return;
        };
        let mut seen = kind.seen;
        for (seen, count) in seen.iter_mut().zip(counts) {
            *seen |= *count != 0;
        }
        if seen == kind.seen {
            return;
        }
        kind.seen = seen;
        self.mark(variant);
    }

    /// Every variant `read` does not define has no names from `hour`.
    fn forget_unread(&mut self, hour: i32, read: &BTreeMap<u64, Vec<VariantVersion>>) {
        let unread = VariantName {
            first_hour: hour,
            raw: None,
        };
        let mut defined = read.keys().peekable();
        for (&variant, kind) in self.held.iter_mut() {
            while defined.peek().is_some_and(|held| **held < variant) {
                defined.next();
            }
            if defined.peek() == Some(&&variant) || kind.names.as_slice() == [unread] {
                continue;
            }
            kind.names.clear();
            kind.names.push(unread);
            *self.moved = true;
            self.changed.push(variant);
        }
    }
}

/// Classifies `variant`, which an inventory selects or none does, as `catalogue` names it at
/// `at`; true where it built the classification. `created` gets each variant it classified
/// first.
#[allow(clippy::too_many_arguments)]
fn classify(
    classes: &mut ClassEdits<'_>,
    classified: &mut HashSet<u64>,
    tables: &mut Tables,
    catalogue: &VariantCatalogue,
    created: &mut Vec<u64>,
    at: i32,
    variant: u64,
    count: &VariantMembers,
) -> bool {
    if count.selected == 0 {
        return false;
    }
    let token = catalogue.token(variant);
    if classes.held.contains_key(&variant) {
        if token.is_empty() {
            return false;
        }
        if classified.contains(&variant) {
            classes.redate(variant, at);
            return false;
        }
    }
    let mut names: Vec<VariantName> = Vec::new();
    for version in catalogue.versions.get(&variant).into_iter().flatten() {
        let name = VariantName {
            first_hour: at,
            raw: version.raw.as_ref().map(|raw| tables.raw(raw)),
        };
        if names.last().is_none_or(|last| last.raw != name.raw) {
            names.push(name);
        }
    }
    let seen = match classes.held.get(&variant) {
        Some(kind) => kind.seen,
        None => {
            created.push(variant);
            [false; SEEN]
        }
    };
    classes.set(
        variant,
        Classification {
            token: token.to_owned(),
            names,
            seen,
        },
    );
    if !token.is_empty() {
        classified.insert(variant);
    }
    true
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
        let consecutive = self.last_frame.and_then(|hour| hour.checked_add(1)) == Some(frame.hour);
        let countries = if changes.rows_changed[TableId::Countries.index()]
            || changes.rows_changed[TableId::CountryLinks.index()]
            || !consecutive
        {
            let rows = frame.complete::<tables::Countries>().ok();
            let same = rows.is_some_and(|rows| {
                self.country_reads
                    .as_ref()
                    .is_some_and(|seen| seen.matches_changes(frame, rows, Some(changes)))
            });
            (!same).then(|| {
                self.country_reads = rows.map(|rows| {
                    if !changes.rows_changed[TableId::CountryLinks.index()]
                        && let Some(reads) = &self.country_reads
                    {
                        crate::products::territory::CountryReads::from_slots(
                            frame,
                            rows,
                            Arc::clone(reads.slots()),
                        )
                    } else {
                        crate::products::territory::CountryReads::new(frame, rows)
                    }
                });
                rows.zip(self.country_reads.as_ref()).map(|(rows, reads)| {
                    let unnamed = crate::metadata::tagged_countries_indexed(rows, reads.slots())
                        .into_iter()
                        .filter(|(tag, holder)| {
                            matches!(holder, TaggedCountry::Found(..))
                                && std::str::from_utf8(tag).is_err()
                        })
                        .count();
                    Arc::new(CountryHolders::read(reads, unnamed))
                })
            })
        } else {
            None
        };
        let changed = |table: TableId| changes.rows_changed[table.index()];
        let dated = |table: TableId| changes.slot_read[table.index()];
        let read = |table: TableId| slot_read(format, &frame.metrics, table, frame.hour);
        let complete = Complete {
            countries: frame.complete::<tables::Countries>().is_ok(),
            links: frame.complete::<tables::CountryLinks>().is_ok(),
            output: frame.complete::<tables::ProductionLineOutput>().is_ok(),
            lines: frame.complete::<tables::ProductionLines>().is_ok(),
            rocket: frame.complete::<tables::RocketProductionOutput>().is_ok(),
            definitions: frame.complete::<tables::VariantDefinitions>().is_ok(),
            lexical: frame.complete::<tables::LexicalTokens>().is_ok(),
        };
        let hour = Hour {
            hour: frame.hour,
            countries,
            changed: Changed {
                others: TABLES
                    .iter()
                    .filter(|&&table| {
                        !matches!(
                            table,
                            TableId::Campaign | TableId::Countries | TableId::CountryLinks
                        )
                    })
                    .any(|&table| changed(table)),
                output: changed(TableId::ProductionLineOutput),
                rocket: changed(TableId::RocketProductionOutput),
            },
            dated: Reads {
                output: dated(TableId::ProductionLineOutput),
                rocket: dated(TableId::RocketProductionOutput),
                variants: dated(TableId::VariantDefinitions),
            },
            reads: Reads {
                output: read(TableId::ProductionLineOutput),
                rocket: read(TableId::RocketProductionOutput),
                variants: read(TableId::VariantDefinitions),
            },
            complete,
            design_changed: equipment::design_changed(frame),
            output: Rows::Slice(
                frame
                    .complete::<tables::ProductionLineOutput>()
                    .unwrap_or(&[]),
            ),
            output_reason: match &*frame.tables[TableId::ProductionLineOutput.index()] {
                TableView::Unavailable { reason } => Some(reason.as_str()),
                _ => None,
            },
            lines: Lines::Rows(frame.complete::<tables::ProductionLines>().unwrap_or(&[])),
            rocket: Rows::Slice(
                frame
                    .complete::<tables::RocketProductionOutput>()
                    .unwrap_or(&[]),
            ),
            variants: Variants::Frame(frame),
        };
        self.ingest_hour(hour, cancelled)
    }

    /// Ingests one hour, as [`Self::ingest_changed`] ingests a frame.
    pub(crate) fn ingest_hour(&mut self, hour: Hour<'_>, cancelled: &AtomicBool) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope_phase("production:observe_apply");
        check_cancel(cancelled)?;
        let mut hour = hour;
        self.spent.advance(hour.hour)?;
        let consecutive = self.last_frame.and_then(|last| last.checked_add(1)) == Some(hour.hour);
        let countries_changed = match hour.countries.take() {
            Some(countries) => {
                self.countries = countries;
                true
            }
            None => false,
        };
        let changed = self.schema != SCHEMA
            || !consecutive
            || countries_changed
            || hour.changed.others
            || hour.dated.any();
        if !changed && let Some(pending) = &mut self.pending {
            pending.page.last_hour = hour.hour;
            self.advance_spent_holders(hour.hour);
            self.last_frame = Some(hour.hour);
            return Ok(());
        }
        let equipment_changed = !consecutive
            || countries_changed
            || self.pending.is_none()
            || hour.changed.output
            || hour.dated.output;
        let rocket_changed =
            !consecutive || countries_changed || hour.changed.rocket || hour.dated.rocket;
        self.ingest_frame(hour, cancelled, equipment_changed, rocket_changed)
    }

    /// Whether hours from `first` on that change nothing production reads and date none of
    /// its reads can be advanced over without ingesting them ([`Self::advance_quiet`]).
    pub(crate) fn quiet_from(&self, first: i32) -> bool {
        self.schema == SCHEMA
            && self.pending.is_some()
            && self.last_frame.and_then(|hour| hour.checked_add(1)) == Some(first)
    }

    /// Hours `first..=last` that follow the last frame, at which no table
    /// production reads changed (country keys and links included) and no
    /// output or variant read is dated: ingesting each of them extends the
    /// open endpoint page and the spent series to it, carrying holder cache
    /// proofs without visiting inventories. False without an open page, whose
    /// hours only ingestion opens.
    pub fn advance_quiet(&mut self, first: i32, last: i32) -> io::Result<bool> {
        if !self.quiet_from(first) {
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
        // The open page's holders were extended to this hour, or read there.
        if self.scratch.sealed == Some(hour) {
            return;
        }
        let mut sealed = 0usize;
        for (slot, tracks) in [&mut self.result.tags, &mut self.result.rocket.tags]
            .into_iter()
            .enumerate()
        {
            let live = page.countries.iter().filter(|row| {
                [&row.equipment, &row.rocket][slot].is_some()
            });
            each_track(tracks, live.map(|row| row.tag.as_str()), |track| {
                if let Some((_, at)) = &mut track.last {
                    *at = hour;
                    if let Some(last) = track.last_hours.last_mut() {
                        *last = hour;
                    }
                    sealed += 1;
                }
            });
        }
        #[cfg(any(test, feature = "test-support"))]
        {
            self.work.history_visits += sealed;
        }
        #[cfg(not(any(test, feature = "test-support")))]
        let _ = sealed;
        self.scratch.sealed = Some(hour);
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
        hour: Hour<'_>,
        cancelled: &AtomicBool,
        equipment_changed: bool,
        rocket_changed: bool,
    ) -> io::Result<()> {
        let cpu = ::replay_frame::cpu::scope_phase("production:settle_reconcile");
        let at = hour.hour;
        let complete = hour.complete;
        check_cancel(cancelled)?;
        if self.schema != SCHEMA {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Prepared production state schema mismatch",
            ));
        }
        self.seal_holders();
        if let Some(last) = self.last_frame
            && last.checked_add(1) != Some(at)
        {
            self.forget_names(last.saturating_add(1));
        }
        let names_read = hour.design_changed || hour.reads.variants;
        let named = complete.countries && complete.links && complete.lexical;
        if !complete.definitions || names_read && !named {
            self.forget_names(at);
        }
        let diagnostics_before = self.diagnostic_counts();
        let consecutive = self.last_frame.and_then(|last| last.checked_add(1)) == Some(at);
        drop(cpu);
        let cpu = ::replay_frame::cpu::scope_phase("production:observe_apply");
        let reads_before = (self.result.reads.len(), self.result.rocket.reads.len());
        self.observe(&hour, equipment_changed, rocket_changed)?;
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
        // Where no track or country moved and no reason is dated at this hour, every
        // holder's spending, inventory and row holds as it was.
        let moved = equipment_changed || rocket_changed || !self.steady_at(at);
        self.observe_spent(at, moved)?;
        drop(cpu);
        let numeric_read = hour.reads.output || hour.reads.rocket;
        if consecutive
            && !numeric_read
            && !names_read
            && diagnostics_before == self.diagnostic_counts()
            && self.pending.as_ref().is_some_and(|pending| {
                pending.page.unnamed_per_hour == 0 && self.same_holders(&pending.page, at)
            })
        {
            self.pending
                .as_mut()
                .expect("unchanged endpoint")
                .page
                .last_hour = at;
            check_cancel(cancelled)?;
            return Ok(());
        }
        let cpu = ::replay_frame::cpu::scope_phase("production:presentation");
        // A fresh hourly catalogue is empty outside a complete variant read.
        // At a complete read only changed operands need building again.
        let complete_names = names_read && named && complete.definitions;
        if complete_names {
            self.variants
                .read(at, hour.variants, &mut self.variant_refusals);
            self.names_hour = Some(at);
        }
        self.present(at, complete_names, names_read, moved);
        drop(cpu);
        let cpu = ::replay_frame::cpu::scope_phase("production:endpoint_projection");
        let rows_moved = if moved || self.pending.is_none() {
            self.project_rows(at, cancelled)?
        } else {
            self.scratch.rows.clear();
            self.scratch.amount_edits.clear();
            false
        };
        let counts_held = self
            .pending
            .as_ref()
            .is_some_and(|pending| self.counts_match(&pending.counts));
        let rendered = (!counts_held).then(|| {
            let counts = self.page_counts();
            let (warnings, index) = counts.warnings();
            (counts, warnings, index)
        });
        let unnamed_per_hour = self.result.unnamed - diagnostics_before[4];
        let equipment_read = self.result.reads.last().copied();
        let rocket_read = self.result.rocket.reads.last().copied();
        let holds = !rows_moved
            && !self.classes_moved
            && self.pending.as_ref().is_some_and(|pending| {
                let page = &pending.page;
                let mut unnamed_before = diagnostics_before[4];
                if unnamed_per_hour == page.unnamed_per_hour
                    && unnamed_before == page.unnamed_at(page.last_hour)
                {
                    unnamed_before = page.unnamed_before;
                }
                page.last_hour.checked_add(1) == Some(at)
                    && rendered.as_ref().is_none_or(|(_, warnings, index)| {
                        page.warnings == *warnings && page.unnamed_warning_index == *index
                    })
                    && page.equipment_read == equipment_read
                    && page.rocket_read == rocket_read
                    && page.equipment_origin == self.equipment_origin
                    && page.equipment_restart == self.equipment_restart
                    && page.rocket_origin == self.rocket_origin
                    && page.rocket_restart == self.rocket_restart
                    && page.unnamed_per_hour == unnamed_per_hour
                    && page.unnamed_before == unnamed_before
            });
        if holds {
            // The page holds: what it began with stays.
            self.pending.as_mut().expect("a held endpoint").page.last_hour = at;
        } else {
            let (counts, warnings, unnamed_warning_index) = match rendered {
                Some(rendered) => rendered,
                None => {
                    let pending = self.pending.as_ref().expect("an endpoint of equal counts");
                    (
                        pending.counts.clone(),
                        pending.page.warnings.clone(),
                        pending.page.unnamed_warning_index,
                    )
                }
            };
            let (countries, edited) = match self.pending.take() {
                Some(mut previous) => {
                    self.complete(&previous)?;
                    let scratch = &mut *self.scratch;
                    // The completed endpoint's edit lists are this one's buffers.
                    if let Some((edited, entries)) = previous.edited.take() {
                        scratch.row_buffers.edited = edited;
                        scratch.row_buffers.entries = entries;
                    }
                    let mut rows = std::mem::take(&mut previous.page.countries);
                    apply_rows(
                        &mut rows,
                        &mut scratch.row_buffers,
                        &mut scratch.rows,
                        &scratch.amount_edits,
                    );
                    let buffers = &mut scratch.row_buffers;
                    let edited = (
                        std::mem::take(&mut buffers.edited),
                        std::mem::take(&mut buffers.entries),
                    );
                    (rows, Some(edited))
                }
                None => {
                    let scratch = &mut *self.scratch;
                    let mut rows = Vec::new();
                    apply_rows(
                        &mut rows,
                        &mut scratch.row_buffers,
                        &mut scratch.rows,
                        &scratch.amount_edits,
                    );
                    (rows, None)
                }
            };
            #[cfg(test)]
            let classifications = self.classifications.clone();
            #[cfg(not(test))]
            let classifications = BTreeMap::new();
            let page = EndpointPage {
                first_hour: at,
                last_hour: at,
                equipment_read,
                rocket_read,
                equipment_origin: self.equipment_origin,
                equipment_restart: self.equipment_restart,
                rocket_origin: self.rocket_origin,
                rocket_restart: self.rocket_restart,
                countries,
                classifications,
                warnings,
                unnamed_before: diagnostics_before[4],
                unnamed_per_hour,
                unnamed_warning_index,
            };
            let named = name_events(
                &self.classifications,
                Some(&mut self.class_changed),
                self.names_hour,
                &self.batch,
            );
            self.final_counts = counts.clone();
            self.pending = Some(Pending {
                page,
                names_hour: self.names_hour,
                counts,
                edited,
                named: Some(named),
            });
        }
        self.classes_moved = false;
        drop(cpu);
        let _pruning = ::replay_frame::cpu::scope_phase("production:pruning");
        // Only current supplying reads and retained row counters survive a frame; only a
        // read adds to them.
        if self.result.reads.len() > reads_before.0 {
            each_track(
                &mut self.result.tags,
                self.result.active.iter().map(String::as_str),
                compact,
            );
        }
        if self.result.rocket.reads.len() > reads_before.1 {
            let rocket = &mut self.result.rocket;
            each_track(
                &mut rocket.tags,
                rocket.active.iter().map(String::as_str),
                compact,
            );
        }
        retain_last(&mut self.result.reads);
        retain_last(&mut self.result.rocket.reads);
        for since in &self.result.observations {
            if self.observation_starts.last() != Some(since) {
                self.observation_starts.push(*since);
            }
        }
        retain_last(&mut self.result.observations);
        retain_last(&mut self.result.rocket.observations);
        // Every source the open page holds was read live at this hour.
        self.scratch.sealed = Some(at);
        self.seal_holders();
        check_cancel(cancelled)?;
        Ok(())
    }

    /// Presentation at `at`: each holder's inventory keys, then the classifications of the
    /// variants they select. Outside a complete variant read only variants whose inventories
    /// changed can need classifying.
    fn present(&mut self, at: i32, complete_names: bool, names_read: bool, moved: bool) {
        let Self {
            variants,
            result,
            countries,
            scratch,
            classifications,
            classes_moved,
            class_changed,
            tables,
            ..
        } = self;
        let Current { tags, rocket, .. } = result;
        let Scratch {
            entries,
            members: changed,
            created,
            inventoried,
            unclassified,
            redated,
            ..
        } = &mut **scratch;
        if moved || !*inventoried {
            variants.inventory(
                countries.as_deref(),
                [tags, &mut rocket.tags],
                entries,
                changed,
            );
            *inventoried = true;
        } else {
            changed.clear();
        }
        let ProductionVariants {
            catalogue: read,
            classified,
            members,
            #[cfg(test)]
                classifications: built,
            ..
        } = variants;
        let empty = VariantCatalogue::default();
        let catalogue = if complete_names { &*read } else { &empty };
        let mut classes = ClassEdits {
            held: classifications,
            moved: classes_moved,
            changed: class_changed,
        };
        created.clear();
        let mut builds = 0usize;
        if complete_names {
            // A classified design keeps its token, as its definition was not read anew: it
            // only takes this read's date. The rest are classified after, in order, so their
            // raw operands take table places as a single pass gives them.
            unclassified.clear();
            redated.clear();
            let mut kinds = classes.held.iter_mut().peekable();
            for (&variant, count) in members.iter() {
                if count.selected == 0 {
                    continue;
                }
                while kinds.peek().is_some_and(|(held, _)| **held < variant) {
                    kinds.next();
                }
                if classified.contains(&variant)
                    && let Some((held, kind)) = kinds.peek_mut()
                    && **held == variant
                {
                    if kind.names.iter().any(|name| name.first_hour != at) {
                        for name in &mut kind.names {
                            name.first_hour = at;
                        }
                        redated.push(variant);
                    }
                    continue;
                }
                unclassified.push(variant);
            }
            for &variant in redated.iter() {
                classes.mark(variant);
            }
            for &variant in unclassified.iter() {
                builds += usize::from(classify(
                    &mut classes,
                    classified,
                    tables,
                    catalogue,
                    created,
                    at,
                    variant,
                    &members[&variant],
                ));
            }
        } else {
            for &variant in changed.iter() {
                if let Some(count) = members.get(&variant) {
                    builds += usize::from(classify(
                        &mut classes,
                        classified,
                        tables,
                        catalogue,
                        created,
                        at,
                        variant,
                        count,
                    ));
                }
            }
        }
        #[cfg(test)]
        {
            *built += builds;
        }
        #[cfg(not(test))]
        let _ = builds;
        // A count only grows where an inventory changed, or a classification is new.
        for &variant in changed.iter().chain(created.iter()) {
            if let Some(count) = members.get(&variant) {
                classes.see(variant, &count.seen);
            }
        }
        if names_read {
            classes.forget_unread(at, &catalogue.versions);
        }
    }

    /// No taint or observation restart is dated at `hour`: every known flag and spending
    /// reason that held the hour before holds.
    fn steady_at(&self, hour: i32) -> bool {
        self.result.tainted.values().all(|from| *from != hour)
            && self
                .result
                .spending_tainted
                .values()
                .all(|(from, _)| *from != hour)
            && self.equipment_restart != Some(hour)
            && self.rocket_restart != Some(hour)
    }

    /// Whether `counts` are what the open page would render its warnings from now.
    fn counts_match(&self, counts: &PageCounts) -> bool {
        let (recorded, result) = (&counts.recording, &self.result);
        recorded.reads == !result.reads.is_empty()
            && recorded.unread == result.unread
            && recorded.unread_reason == result.unread_reason
            && recorded.tainted_from == result.tainted.iter().next().map(|(_, from)| *from)
            && recorded.missed == result.missed
            && recorded.inexact == result.inexact
            && recorded.lost == result.lost
            && recorded.unattributed == result.unattributed
            && recorded.unlisted == self.unlisted.len()
            && recorded.observations == result.observations.len()
            && recorded.rocket_reads == !result.rocket.reads.is_empty()
            && recorded.rocket_unread == result.rocket.unread
            && recorded.rocket_unattributed == result.rocket.unattributed
            && recorded.rocket_inconsistent == result.rocket.inconsistent
            && counts.refusals == self.variant_refusals
            && counts.restarts == self.observation
    }

    fn page_counts(&self) -> PageCounts {
        PageCounts {
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
        }
    }

    /// How the open page's rows change at `at`, in [`Scratch::rows`] with the amount edits
    /// they name; true where any does. Binds each equipment read's Rocket read as a whole
    /// projection does, withdrawing the open page's joins it ends.
    fn project_rows(&mut self, at: i32, cancelled: &AtomicBool) -> io::Result<bool> {
        let countries = self.countries.clone();
        let Self {
            result,
            segments,
            rocket_segments,
            first_equipment_holders,
            first_rocket_holders,
            equipment_restart,
            rocket_restart,
            joins,
            pending,
            scratch,
            ..
        } = self;
        let Scratch {
            rows: edits,
            amount_edits,
            entries,
            ..
        } = &mut **scratch;
        edits.clear();
        amount_edits.clear();
        let held: &mut [CountryEndpoint] = match pending {
            Some(pending) => &mut pending.page.countries,
            None => &mut [],
        };
        let Current {
            tags,
            rocket,
            tainted,
            spending_tainted,
            observations,
            ..
        } = result;
        let origin = observations.last().copied();
        let rocket_origin = rocket.observations.last().copied();
        let mut present: Vec<&str> = Vec::new();
        let mut next = 0;
        for tag in countries
            .iter()
            .flat_map(|countries| countries.holders.keys())
        {
            check_cancel(cancelled)?;
            while next < held.len() && held[next].tag.as_str() < tag.as_str() {
                edits.push(RowEdit::Remove(next));
                next += 1;
            }
            let row = (next < held.len() && held[next].tag == *tag).then_some(next);
            let track = tags.get_mut(tag);
            let identity = track
                .as_ref()
                .and_then(|track| track.identities.last().copied());
            let units_known = identity
                .is_some_and(|identity| tainted.get(&identity.2).is_none_or(|from| at < *from));
            let spending_known = identity.is_some_and(|identity| {
                spending_tainted
                    .get(&identity.2)
                    .is_none_or(|(from, _)| at < *from)
            });
            let equipment = source_edit(
                track,
                at,
                origin,
                segments,
                tag,
                (units_known, spending_known),
                (&*spending_tainted, &*first_equipment_holders, *equipment_restart, false),
                row.and_then(|row| held[row].equipment.as_ref()),
                entries,
                amount_edits,
            )?;
            let rocket_source = source_edit(
                rocket.tags.get_mut(tag),
                at,
                rocket_origin,
                rocket_segments,
                tag,
                (true, true),
                (&*spending_tainted, &*first_rocket_holders, *rocket_restart, true),
                row.and_then(|row| held[row].rocket.as_ref()),
                entries,
                amount_edits,
            )?;
            let joined = match equipment.0 {
                Some(head) => {
                    let (joined, withdrawn) = joins.bind(
                        tag,
                        (head.slot, head.country_id, head.observation, head.read_hour),
                        rocket_source.0.map(|rocket| RocketBinding {
                            slot: rocket.slot,
                            country_id: rocket.country_id,
                            observation: rocket.observation,
                            read_hour: rocket.read_hour,
                            segment: rocket.segment,
                        }),
                        at,
                    );
                    if withdrawn {
                        let correction = joins.corrections.last().expect("a withdrawn join");
                        for row in held.iter_mut() {
                            if correction.matches(row) {
                                row.rocket_joined = false;
                            }
                        }
                    }
                    joined
                }
                None => false,
            };
            let live = equipment.0.is_some() || rocket_source.0.is_some();
            match row {
                None if live => edits.push(RowEdit::Insert(
                    next,
                    CountryEndpoint {
                        tag: tag.clone(),
                        equipment: whole_source(equipment.1),
                        rocket: whole_source(rocket_source.1),
                        rocket_joined: joined,
                    },
                )),
                None => {}
                Some(row) if !live => edits.push(RowEdit::Remove(row)),
                Some(row) => {
                    if !matches!(equipment.1, SourceEdit::Same)
                        || !matches!(rocket_source.1, SourceEdit::Same)
                        || held[row].rocket_joined != joined
                    {
                        edits.push(RowEdit::Modify {
                            held: row,
                            sources: [equipment.1, rocket_source.1],
                            joined,
                        });
                    }
                }
            }
            if row.is_some() {
                next += 1;
            }
            if live {
                present.push(tag);
            }
        }
        while next < held.len() {
            edits.push(RowEdit::Remove(next));
            next += 1;
        }
        joins
            .bounds
            .retain(|tag, _| present.binary_search(&tag.as_str()).is_ok());
        Ok(!edits.is_empty())
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
        if self.classes_moved {
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
    /// Binds `tag`'s Rocket read `observed` to its equipment read `equipment` (slot, country,
    /// observation and read hour): true while the join holds. A join whose Rocket read ended
    /// is withdrawn, true in the second place, its correction the last kept.
    fn bind(
        &mut self,
        tag: &str,
        equipment: (u32, u32, Option<i32>, i32),
        observed: Option<RocketBinding>,
        hour: i32,
    ) -> (bool, bool) {
        let (slot, country_id, observation, read_hour) = equipment;
        if !self.bounds.contains_key(tag) {
            self.bounds.insert(
                tag.to_owned(),
                RocketBound {
                    slot,
                    country_id,
                    observation,
                    equipment_read: read_hour,
                    rocket: observed.clone(),
                },
            );
        }
        let bound = self.bounds.get_mut(tag).expect("a bound tag");
        let mut withdrawn = false;
        if (
            bound.slot,
            bound.country_id,
            bound.observation,
            bound.equipment_read,
        ) != equipment
        {
            *bound = RocketBound {
                slot,
                country_id,
                observation,
                equipment_read: read_hour,
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
            self.corrections.push(CoverageCorrection {
                tag: tag.to_owned(),
                slot,
                country_id,
                observation,
                read_hour,
                first_hour: read_hour,
                last_hour: hour,
            });
            withdrawn = true;
        }
        (bound.rocket.is_some(), withdrawn)
    }

    #[cfg(test)]
    fn bind_rocket(
        &mut self,
        tag: &str,
        equipment: &SourceEndpoint,
        observed: Option<&SourceEndpoint>,
        hour: i32,
        pending: Option<&mut EndpointPage>,
    ) -> bool {
        let (joined, withdrawn) = self.bind(
            tag,
            (
                equipment.slot,
                equipment.country_id,
                equipment.observation,
                equipment.read_hour,
            ),
            observed.map(RocketBinding::from),
            hour,
        );
        if withdrawn && let Some(page) = pending {
            let correction = self.corrections.last().expect("a withdrawn join");
            for row in &mut page.countries {
                if correction.matches(row) {
                    row.rocket_joined = false;
                }
            }
        }
        joined
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

    /// [`Self::push`] of a changed holder, whose patch from the endpoint before is `patch`
    /// and whose state is now `now`.
    fn push_patch(
        &mut self,
        holder: u32,
        endpoint: u64,
        patch: &[u8],
        now: Option<&Holder>,
    ) -> io::Result<usize> {
        if self.events != 0 && (self.events as usize) < facts::HOLDER_GROUP {
            let start = self.open.0.len();
            self.open.u64(endpoint);
            self.open.0.extend_from_slice(patch);
            self.last = endpoint;
            self.events += 1;
            return Ok(self.open.0.len() - start);
        }
        self.push(holder, endpoint, None, now)
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
    /// An endpoint was completed in it.
    started: bool,
    /// Every holder at the batch's last endpoint, by id.
    holders: BTreeMap<u32, Holder>,
    /// The names hour of the batch's last endpoint.
    names_hour: Option<Option<i32>>,
    streams: BTreeMap<u32, HolderStream>,
    naming: Out,
    naming_count: u32,
    /// Each variant's names as last published in this batch.
    named: BTreeMap<u64, Vec<NameCode>>,
    bytes: usize,
}

impl Batch {
    /// An endpoint's record and ranking block, `block` before compression. Returns the bytes
    /// they added.
    fn endpoint(
        &mut self,
        page: &EndpointPage,
        names_hour: Option<i32>,
        keyframe: bool,
        block: &[u8],
    ) -> io::Result<usize> {
        let block = crate::prepared::series::compress(block, 3)?;
        let ranking_offset = self.ranking.len() as u64;
        let ranking_len = u32::try_from(block.len())
            .map_err(|_| io::Error::other("A production ranking block exceeds 4 GiB"))?;
        self.ranking.extend_from_slice(&block);
        let before = self.endpoints.0.len();
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
        .write(&mut self.endpoints);
        self.endpoint_count += 1;
        Ok(block.len() + self.endpoints.0.len() - before)
    }
}

impl ProductionAccumulator {
    /// A completed endpoint's facts, joined to the batch's. Where its rows were made from the
    /// batch's last endpoint's, only the rows that changed are read again.
    fn complete(&mut self, completed: &Pending) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope_phase("production:facts");
        let Pending {
            page,
            names_hour,
            edited,
            named,
            ..
        } = completed;
        let names_hour = *names_hour;
        #[cfg(test)]
        self.completed.push((page.clone(), names_hour));
        let endpoint = self.pages_written;
        self.pages_written = endpoint
            .checked_add(1)
            .ok_or_else(|| io::Error::other("Production page count exceeds its integer range"))?;
        let mut added = match edited.as_ref().filter(|_| self.batch.started) {
            Some((edits, entries)) => {
                self.complete_changed(page, edits, entries, endpoint, names_hour)?
            }
            None => self.complete_whole(page, endpoint, names_hour)?,
        };
        // Names (P5), wherever a variant's changed.
        let compared;
        let events = match named {
            Some(events) => events,
            None => {
                compared = name_events(&self.classifications, None, names_hour, &self.batch);
                &compared
            }
        };
        let before = self.batch.naming.0.len();
        for (variant, code) in events {
            let index = self.tables.variant(*variant);
            let batch = &mut self.batch;
            batch.naming.u64(endpoint);
            batch.naming.u32(index);
            batch.naming.count(code.len())?;
            for name in code {
                name.write(&mut batch.naming);
            }
            batch.naming_count += 1;
            batch.named.insert(*variant, code.clone());
        }
        added += self.batch.naming.0.len() - before;
        self.batch.bytes += added;
        self.batch.started = true;
        self.batch.names_hour = Some(names_hour);
        Ok(())
    }

    /// Every holder of `page` read whole and diffed against the batch's last endpoint's (none
    /// at the batch's first). Returns the bytes it added.
    fn complete_whole(
        &mut self,
        page: &EndpointPage,
        endpoint: u64,
        names_hour: Option<i32>,
    ) -> io::Result<usize> {
        let mut now = BTreeMap::new();
        for row in &page.countries {
            let holder = self.tables.holder(&row.tag);
            let equipment = row
                .equipment
                .as_ref()
                .map(|source| facts_source(&mut self.tables, source, page.equipment_read))
                .transpose()?;
            let rocket = row
                .rocket
                .as_ref()
                .map(|source| facts_source(&mut self.tables, source, page.rocket_read))
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
        let batch = &mut self.batch;
        let start = !batch.started;
        if start {
            batch.first_endpoint = Some(endpoint);
        }
        let previous = if start {
            BTreeMap::new()
        } else {
            std::mem::take(&mut batch.holders)
        };
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
        let mut added = batch.endpoint(page, names_hour, keyframe, &block.0)?;
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
        self.present_holders = now.keys().copied().collect();
        batch.holders = now;
        Ok(added)
    }

    /// The holders of `page` whose rows `edits` changed since the batch's last endpoint, or
    /// whose heads read differently now, patched against that endpoint's; the rest hold.
    /// Variants take their table places in the order a whole read meets them. Returns the
    /// bytes it added.
    fn complete_changed(
        &mut self,
        page: &EndpointPage,
        edits: &[Edited],
        entries: &[Entry],
        endpoint: u64,
        names_hour: Option<i32>,
    ) -> io::Result<usize> {
        let Self {
            tables,
            batch,
            present_holders,
            scratch,
            ..
        } = self;
        let Scratch {
            holders: ids,
            changes,
            leaf_edits,
            patches,
            ..
        } = &mut **scratch;
        ids.clear();
        changes.clear();
        leaf_edits.clear();
        patches.clear();
        for (row, edit) in page.countries.iter().zip(edits) {
            let holder = tables.holder(&row.tag);
            ids.push(holder);
            let previous = batch.holders.get(&holder);
            let mut moved = previous.is_none_or(|previous| previous.rocket_joined != row.rocket_joined);
            let mut sources = [SourceChange::Absent, SourceChange::Absent];
            for (slot, (source, read)) in [
                (row.equipment.as_ref(), page.equipment_read),
                (row.rocket.as_ref(), page.rocket_read),
            ]
            .into_iter()
            .enumerate()
            {
                let before = previous.and_then(|previous| previous.source(slot));
                let Some(source) = source else {
                    moved |= before.is_some();
                    continue;
                };
                match before {
                    Some(before) if !edit.whole[slot] => {
                        let head = facts_head(source, read);
                        let start = leaf_edits.len();
                        let (first, last) = edit.entries[slot];
                        for &entry in &entries[first as usize..last as usize] {
                            let variant = tables.variant(entry.0);
                            let key = (variant, entry.1.index());
                            let leaf = held_amount(&source.amounts, entry)
                                .ok()
                                .map(|at| leaf_of(variant, &source.amounts[at]))
                                .transpose()?;
                            let held = before
                                .leaves
                                .binary_search_by_key(&key, Leaf::key)
                                .ok()
                                .map(|at| before.leaves[at]);
                            if leaf != held {
                                leaf_edits.push((key, leaf));
                            }
                        }
                        leaf_edits[start..].sort_unstable_by_key(|(key, _)| *key);
                        moved |= head != before.head || leaf_edits.len() > start;
                        sources[slot] = SourceChange::Leaves(head, (start, leaf_edits.len()));
                    }
                    _ => {
                        let whole = facts_source(tables, source, read)?;
                        moved |= before != Some(&whole);
                        sources[slot] = SourceChange::Whole(whole);
                    }
                }
            }
            if moved {
                changes.push(HolderChange {
                    holder,
                    now: Some(HolderNow {
                        sources,
                        joined: row.rocket_joined,
                    }),
                    patch: (0, 0),
                });
            }
        }
        ids.sort_unstable();
        for &holder in batch.holders.keys() {
            if ids.binary_search(&holder).is_err() {
                changes.push(HolderChange {
                    holder,
                    now: None,
                    patch: (0, 0),
                });
            }
        }
        changes.sort_unstable_by_key(|change| change.holder);
        // Each change's patch from the batch's last endpoint, before it applies.
        let mut out = Out(std::mem::take(patches));
        for change in changes.iter_mut() {
            let start = out.0.len();
            write_change(
                change.holder,
                batch.holders.get(&change.holder),
                change.now.as_ref(),
                leaf_edits,
                &mut out,
            )?;
            change.patch = (start, out.0.len());
        }
        *patches = out.0;
        for change in changes.iter_mut() {
            let Some(now) = change.now.take() else {
                batch.holders.remove(&change.holder);
                present_holders.remove(&change.holder);
                continue;
            };
            let state = batch.holders.entry(change.holder).or_insert_with(|| {
                present_holders.insert(change.holder);
                Holder {
                    equipment: None,
                    rocket: None,
                    rocket_joined: false,
                }
            });
            state.rocket_joined = now.joined;
            for (slot, source) in now.sources.into_iter().enumerate() {
                let held = state.source_mut(slot);
                match source {
                    SourceChange::Absent => *held = None,
                    SourceChange::Whole(source) => *held = Some(source),
                    SourceChange::Leaves(head, (first, last)) => {
                        let held = held.as_mut().expect("a patched source's leaves");
                        held.head = head;
                        for (key, leaf) in &leaf_edits[first..last] {
                            match (held.leaves.binary_search_by_key(key, Leaf::key), leaf) {
                                (Ok(at), Some(leaf)) => held.leaves[at] = *leaf,
                                (Ok(at), None) => {
                                    held.leaves.remove(at);
                                }
                                (Err(at), Some(leaf)) => held.leaves.insert(at, *leaf),
                                (Err(_), None) => {}
                            }
                        }
                    }
                }
            }
        }
        // Ranking (P2): every holder at a keyframe, else what changed.
        let keyframe = batch.endpoint_count as usize % facts::RANKING_KEYFRAME == 0;
        let mut block = Out::default();
        if keyframe {
            for (&holder, state) in &batch.holders {
                facts::write_patch(holder, None, Some(state), &mut block)?;
            }
        } else {
            for change in changes.iter() {
                block.0.extend_from_slice(&patches[change.patch.0..change.patch.1]);
            }
        }
        let mut added = batch.endpoint(page, names_hour, keyframe, &block.0)?;
        // Each holder's leaves (P3), wherever they changed.
        for change in changes.iter() {
            added += batch.streams.entry(change.holder).or_default().push_patch(
                change.holder,
                endpoint,
                &patches[change.patch.0..change.patch.1],
                batch.holders.get(&change.holder),
            )?;
        }
        Ok(added)
    }
}

/// The patch [`facts::write_patch`] writes from `before` to the state `now` makes of it, or to
/// no holder where `now` is none.
fn write_change(
    holder: u32,
    before: Option<&Holder>,
    now: Option<&HolderNow>,
    leaf_edits: &[((u32, usize), Option<Leaf>)],
    out: &mut Out,
) -> io::Result<()> {
    out.u32(holder);
    let Some(now) = now else {
        out.u8(0);
        return Ok(());
    };
    let present = |slot: usize| !matches!(now.sources[slot], SourceChange::Absent);
    out.u8(1 | u8::from(present(0)) << 1 | u8::from(present(1)) << 2 | u8::from(now.joined) << 3);
    for (slot, source) in now.sources.iter().enumerate() {
        let previous = before.and_then(|before| before.source(slot));
        match source {
            SourceChange::Absent => {}
            SourceChange::Whole(source) => facts::write_source(previous, source, out)?,
            SourceChange::Leaves(head, (first, last)) => facts::write_leaves(
                previous.expect("a patched source's leaves"),
                head,
                &leaf_edits[*first..*last],
                out,
            )?,
        }
    }
    Ok(())
}

impl ProductionAccumulator {
    /// Completes the open endpoint and appends the batch's facts and spent
    /// series. Appends begin a new endpoint.
    pub fn flush(&mut self, writer: &mut store::Writer, cancelled: &AtomicBool) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope_phase("production:series_flush");
        self.seal_holders();
        if let Some(pending) = self.pending.take() {
            self.complete(&pending)?;
            self.classes_moved = false;
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
        let named = name_events(&page.classifications, None, names_hour, &self.batch);
        self.complete(&Pending {
            page,
            names_hour,
            counts: PageCounts::default(),
            edited: None,
            named: Some(named),
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
mod columnar_tests;
#[cfg(test)]
mod parity;
#[cfg(test)]
#[path = "prepared_production/tests.rs"]
mod tests;
