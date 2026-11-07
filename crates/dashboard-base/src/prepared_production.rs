//! Production's preparation: one observer of every country's production
//! output, whose dated endpoints are published as integer facts ([`facts`])
//! and read through indexes decoded once per publication ([`read`]).

pub mod columnar;
pub(crate) mod facts;
pub(crate) mod fold;
mod holders;
pub(crate) mod input;
pub(crate) mod read;
pub(crate) mod select;
pub(crate) mod served;
mod shards;

pub use facts::FACTS;
pub use select::{SelectedCountry, SelectedValue, SelectionPage, empty_selection, select_at};

use crate::metadata::{TaggedCountry, slot_read};
use crate::products::equipment::production::{
    CostTotal, CountryHolders, Entry, Identity, Named, ProductionSpendingUnknown, Purpose,
    ROWS_READER, RocketFold, RocketProduced, RocketRead, Rows,
};
use crate::products::equipment::{
    self, RawVariant, Texts, VariantCatalogue, VariantReads, VariantVersion,
};
use crate::products::equipment_produced::{
    ProducedMeasure, RecordingCounts, month_before, recording_warnings_of,
};
use facts::{Endpoint, Holder, Leaf, NameCode, Out, Preamble};
use fold::{Fold, OutputChanges, Versioned};
use holders::{Track, Visits};
use input::{Changed, Complete, Hour, Lines, Reads, Variants};
use replay_data::{format::FormatVersion, rows::Recorded, rows::TableId, rows::tables};
use replay_frame::{frame::DecodedFrame, frame::TableView};
use serde::{Deserialize, Serialize};
use store::{Piece, Spill};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io::{self, Read},
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

/// The tables design names are read from; VariantChanges only witnesses a naming read.
pub(crate) const NAMING: [TableId; 7] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::LexicalTokens,
    TableId::VariantDefinitions,
    TableId::VariantModules,
    TableId::VariantUpgrades,
];

pub use dashboard_core::map_pairs;

/// What the ordered work keeps between hours so that no hour allocates it again.
#[derive(Default)]
struct Scratch {
    /// The output reads folded; totals note their changes by it.
    read: u64,
    rows: Vec<RowEdit>,
    amount_edits: Vec<(Entry, Option<Amount>)>,
    /// An endpoint's holder ids, ascending.
    holders: Vec<u32>,
    /// Presentation's inventories were built in this session.
    inventoried: bool,
    /// The variants whose members an inventory changed.
    members: Vec<u64>,
    created: Vec<u64>,
    unclassified: Vec<u64>,
    redated: Vec<u64>,
    row_buffers: RowBuffers,
}

#[derive(Default, Deserialize, Serialize)]
struct Current {
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
    /// The Rocket output's reads; its holders' tracks are the accumulator's.
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
pub const SCHEMA: u32 = 10;

#[derive(Default, Deserialize, Serialize)]
pub struct ProductionAccumulator {
    pub schema: u32,
    branch: u64,
    result: Current,
    /// Every tag the countries named, by tag.
    holders: BTreeMap<String, holders::Holder>,
    /// The line output's rows and each producer's totals.
    fold: Fold,
    observation: u32,
    spending_recorded: bool,
    last_frame: Option<i32>,
    classifications: BTreeMap<u64, Classification>,
    /// A classification changed since the open endpoint's page was last made or held.
    #[serde(default)]
    classes_moved: bool,
    /// What names designs moved since a complete naming read last took it.
    #[serde(default)]
    names_dirty: bool,
    /// The variants whose classification changed since the open endpoint's names were taken
    /// ([`name_events`]); a variant may be listed more than once.
    #[serde(default)]
    class_changed: Vec<u64>,
    pending: Option<Pending>,
    /// The latest complete variant read: names it dates take its hour.
    names_hour: Option<i32>,
    /// What the latest endpoint's warnings were rendered from, and its
    /// unnamed country-hours when last sealed.
    final_counts: PageCounts,
    final_unnamed: usize,
    /// Rocket joins withdrawn since the last flush.
    corrections: Vec<CoverageCorrection>,
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
    /// The hour every holder's spending projection was taken or carried to.
    #[serde(skip)]
    spent_hour: Option<i32>,
    /// The holders' spending projections held.
    #[serde(skip)]
    spent_held: usize,
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
    /// Each raw country id's producer under the hour's countries: its canonical slot and id.
    #[serde(skip)]
    producers: HashMap<u32, (u32, u32)>,
    variants: ProductionVariants,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    pub work: crate::prepared::observer_tests::Work,
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
    #[serde(skip)]
    spill: Option<Spill>,
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
    /// It goes on from the endpoint a flush completed ([`facts::Endpoint::continues`]).
    continues: bool,
}

impl Pending {
    /// Whether it holds an hour: a continuation holds none until an hour extends it.
    fn holds_hours(&self) -> bool {
        self.page.first_hour <= self.page.last_hour
    }
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
    spending_since: i32,
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
            spending_since: source.spending_since,
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
            spending_since: self.spending_since,
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

type SpendingIdentity = (
    u32,
    u32,
    Option<i32>,
    u64,
    bool,
    Option<bool>,
    Option<ProductionSpendingUnknown>,
    Option<i32>,
);

fn spending_identity(head: SourceHead) -> SpendingIdentity {
    (
        head.slot,
        head.country_id,
        head.observation,
        head.segment,
        head.spending_known,
        head.present,
        head.campaign_spending_reason,
        (head.present == Some(false)).then_some(head.read_hour),
    )
}

#[derive(Deserialize, Serialize)]
struct SpendingSource {
    identity: Option<SpendingIdentity>,
    since: i32,
    #[serde(with = "map_pairs")]
    amounts: BTreeMap<Entry, (i128, Option<ProductionSpendingUnknown>, i128)>,
}

impl SpendingSource {
    fn absent(saved: &mut Option<Self>, hour: i32, consecutive: bool) {
        let since = saved
            .as_ref()
            .filter(|saved| consecutive && saved.identity.is_none())
            .map_or(hour, |saved| saved.since);
        *saved = Some(Self {
            identity: None,
            since,
            amounts: BTreeMap::new(),
        });
    }

    fn value(amount: &Amount) -> (i128, Option<ProductionSpendingUnknown>, i128) {
        (amount.spent, amount.spending_reason, amount.unproven)
    }

    fn whole(
        saved: &mut Option<Self>,
        head: &mut SourceHead,
        amounts: &[Amount],
        consecutive: bool,
    ) {
        let identity = spending_identity(*head);
        let amounts = amounts
            .iter()
            .map(|amount| ((amount.variant, amount.purpose), Self::value(amount)))
            .collect();
        let since = saved
            .as_ref()
            .filter(|saved| {
                consecutive && saved.identity == Some(identity) && saved.amounts == amounts
            })
            .map_or(head.spending_since, |saved| saved.since);
        head.spending_since = since;
        *saved = Some(Self {
            identity: Some(identity),
            since,
            amounts,
        });
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
        absent_since: [Option<i32>; 2],
    },
}

/// How a holder's facts change at an endpoint.
struct HolderChange {
    holder: u32,
    /// None where the holder is gone.
    now: Option<HolderNow>,
    /// Its patch against the endpoint before.
    patch: Vec<u8>,
}

struct HolderNow {
    sources: [SourceChange; 2],
    joined: bool,
    absent_since: [Option<i32>; 2],
}

enum SourceChange {
    Absent,
    Whole(facts::Source),
    /// Its head, and the leaves it changes, by key.
    Leaves(facts::Head, Vec<((u32, usize), Option<Leaf>)>),
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

/// Current naming operands; a definitions census dates every selected name anew.
#[derive(Default, Deserialize, Serialize)]
struct ProductionVariants {
    #[serde(skip)]
    reads: VariantReads,
    catalogue: VariantCatalogue,
    classified: BTreeSet<u64>,
    invalidated: BTreeSet<u64>,
    /// The first complete naming read since names were last forgotten, from which the held
    /// catalogue names every design; none while forgotten.
    known: Option<i32>,
    #[serde(skip)]
    changed: Vec<u64>,
    members: BTreeMap<u64, VariantMembers>,
    #[cfg(test)]
    #[serde(skip)]
    inventory_visits: usize,
    #[cfg(test)]
    #[serde(skip)]
    classifications: usize,
}

/// Inventory versions authenticate the keys used by presentation independently
/// of quantities, supplying dates and country names.
#[derive(Deserialize, Serialize)]
struct VariantInventory {
    versions: [Option<(Identity, u64)>; 2],
    members: BTreeMap<u64, [bool; SEEN]>,
}
#[derive(Default, Deserialize, Serialize)]
struct VariantMembers {
    seen: [usize; SEEN],
    selected: usize,
}

impl ProductionVariants {
    /// Applies what the holders' inventories added and took of the variants' members; `changed`
    /// gets every variant whose counts they changed.
    fn members(&mut self, presented: Vec<holders::Presented>, changed: &mut Vec<u64>) {
        let _cpu = ::replay_frame::cpu::scope_phase("production:inventory_changes");
        changed.clear();
        for presented in presented {
            for (variant, seen, source, add) in presented.members {
                change_member(&mut self.members, variant, &seen, source, add);
            }
            changed.extend(presented.changed);
            #[cfg(test)]
            {
                self.inventory_visits += presented.visits;
            }
            #[cfg(not(test))]
            let _ = presented.visits;
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
            changed,
            ..
        } = self;
        changed.clear();
        match variants {
            Variants::Frame(frame) => {
                let built = reads.build(frame, false);
                let live = |id| reads.contains(id);
                name_variants(catalogue, classified, changed, hour, built, live, refusals);
            }
            Variants::Read(pending) => {
                let built = std::mem::take(&mut pending.built);
                let present = &pending.present;
                name_variants(
                    catalogue,
                    classified,
                    changed,
                    hour,
                    built,
                    |id| present.contains(&id),
                    refusals,
                );
            }
        }
        changed.extend(std::mem::take(&mut self.invalidated));
        changed.sort_unstable();
        changed.dedup();
    }
}

/// Names `built`'s definitions as read at `hour`, and forgets those `live` no longer holds.
fn name_variants(
    catalogue: &mut VariantCatalogue,
    classified: &mut BTreeSet<u64>,
    changed: &mut Vec<u64>,
    hour: i32,
    built: impl IntoIterator<Item = (u64, RawVariant)>,
    live: impl Fn(u64) -> bool,
    refusals: &mut BTreeMap<String, BTreeSet<u64>>,
) {
    for (id, mut raw) in built {
        if let Some(previous) = catalogue
            .versions
            .get(&id)
            .and_then(|versions| versions.last())
            && let Some(previous) = &previous.raw
        {
            raw.first_hour = previous.first_hour;
            if &raw == previous {
                continue;
            }
        }
        raw.first_hour = hour;
        changed.push(id);
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
    catalogue.versions.retain(|id, _| {
        if live(*id) {
            true
        } else {
            changed.push(*id);
            classified.remove(id);
            false
        }
    });
    changed.sort_unstable();
    changed.dedup();
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

/// A tag's equipment read and the Rocket read joined to it.
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

/// What an hour's equipment read gives every holder to sample.
#[derive(Clone, Copy)]
struct EquipmentPass {
    /// The fold's number for the read.
    number: u64,
    census_complete: bool,
    presence_unknown: bool,
}

/// What the hour's reads give the holders to sample.
#[derive(Default)]
struct Samples {
    equipment: Option<EquipmentPass>,
    rocket: Option<crate::products::equipment::production::RocketTotals>,
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
        let Self {
            result,
            holders,
            tables,
            fold,
            equipment_origin,
            equipment_restart,
            rocket_origin,
            rocket_restart,
            ..
        } = self;
        let mut firsts = |source: usize| -> Vec<u32> {
            holders
                .iter()
                .filter(|(_, holder)| holder.first[source].is_some())
                .map(|(tag, _)| tables.holder(tag))
                .collect()
        };
        let equipment_holders = firsts(0);
        let rocket_holders = firsts(1);
        read::SpendingFacts {
            reads: result.reads.clone(),
            observations: observations(
                *equipment_origin,
                *equipment_restart,
                result.observations.last().copied(),
            ),
            unlisted: fold.unlisted(),
            unread_reason: result.unread_reason.clone(),
            tainted: result
                .spending_tainted
                .iter()
                .map(|(&observation, &(hour, reason))| (observation, hour, reason))
                .collect(),
            rocket_reads: result.rocket.reads.clone(),
            rocket_observations: observations(
                *rocket_origin,
                *rocket_restart,
                result.rocket.observations.last().copied(),
            ),
            rocket_unread: result.rocket.unread,
            rocket_unattributed: result.rocket.unattributed,
            rocket_inconsistent: result.rocket.inconsistent,
            equipment_holders,
            rocket_holders,
        }
    }

    pub fn buffered_bytes(&self) -> usize {
        self.spent.bytes() + self.batch.bytes
    }

    /// Carries only projections current at the old frontier across certified unchanged hours:
    /// a stale one must still project at its next read.
    fn advance_spent(&mut self, hour: i32) {
        if self.spent_hour == self.last_frame {
            self.spent_hour = Some(hour);
        }
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
        for holder in self.holders.values_mut() {
            holder.end(0);
            holder.end(1);
        }
    }

    fn forget_names(&mut self, hour: i32) {
        self.variants.classified.clear();
        self.variants.known = None;
        for (&variant, kind) in &mut self.classifications {
            self.variants.invalidated.insert(variant);
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

    /// Takes the hour's countries: whether each tag is named and how it resolves, and each raw
    /// country id's producer, a slot two tags name being the later tag's.
    fn set_countries(&mut self, countries: Option<Arc<CountryHolders>>) {
        self.producers.clear();
        match &countries {
            None => {
                for holder in self.holders.values_mut() {
                    holder.list(None);
                    holder.first_of_country = false;
                }
            }
            Some(read) => {
                // Both are by tag: the countries' tags are walked in step with the holders.
                let mut held = self.holders.iter().peekable();
                let new: Vec<String> = read
                    .holders
                    .keys()
                    .filter(|tag| walked(&mut held, tag).is_none())
                    .cloned()
                    .collect();
                self.holders
                    .extend(new.into_iter().map(|tag| (tag, holders::Holder::default())));
                let mut named = read.holders.iter().peekable();
                let mut met = std::collections::HashSet::new();
                for (tag, holder) in &mut self.holders {
                    let country = named
                        .next_if(|(held, _)| *held == tag)
                        .map(|(_, country)| *country);
                    holder.list(country);
                    holder.first_of_country =
                        holder.country.is_some_and(|country| met.insert(country));
                }
                let mut by_slot: HashMap<u32, u32> = HashMap::new();
                for &(slot, id) in read.holders.values().flatten() {
                    by_slot.insert(slot, id);
                }
                self.producers
                    .extend(read.slots.iter().filter_map(|(&country, &slot)| {
                        by_slot.get(&slot).map(|&id| (country, (slot, id)))
                    }));
            }
        }
        self.countries = countries;
    }

    /// The hour's reads of the line and Rocket output but the holders' samples, which
    /// [`Self::observe_holders`] takes.
    fn observe_rows(
        &mut self,
        input: &Hour<'_>,
        equipment_changed: bool,
        rocket_changed: bool,
        consecutive: bool,
    ) -> io::Result<Samples> {
        let hour = input.hour;
        let complete = input.complete;
        let mut samples = Samples::default();
        // Rocket rows retain every country/design total; they do not depend
        // on a registered line or on the equipment census being readable.
        if rocket_changed {
            let read = RocketRead {
                hour,
                read: input.reads.rocket,
                complete: complete.countries && complete.links && complete.rocket,
                rows: input.rocket,
            };
            match self.result.rocket.fold(&read, self.countries.as_deref())? {
                RocketFold::Ended => {
                    for holder in self.holders.values_mut() {
                        holder.end(1);
                    }
                }
                RocketFold::Carried => {
                    let observation = self.result.rocket.observation();
                    for holder in self.holders.values_mut() {
                        holder.carry(1, observation, consecutive);
                    }
                }
                RocketFold::Read(totals) => samples.rocket = Some(totals),
            }
        }
        if let Some(last) = self.last_frame
            && last.checked_add(1) != Some(hour)
        {
            self.result.spending_taint(
                self.observation,
                last.saturating_add(1),
                ProductionSpendingUnknown::RecordingGap,
            );
        }
        self.last_frame = Some(hour);
        if !equipment_changed {
            self.result.unnamed += self
                .pending
                .as_ref()
                .map_or(0, |pending| pending.page.unnamed_per_hour);
            return Ok(samples);
        }
        let read = input.reads.output;
        // An hour the tables cannot tell extends nothing, so nothing continues across it.
        if !(complete.countries && complete.links && complete.output) {
            if read {
                // The publication this read held, if any, is not ingested: whatever
                // lines ended in it are lost, for a producer that cannot be told.
                self.result.taint(self.observation, hour);
                if complete.lines {
                    self.result.unread += 1;
                }
                if self.result.unread_reason.is_none()
                    && let Some(reason) = input.output_reason
                {
                    self.result.unread_reason = Some(reason.to_owned());
                }
            }
            for holder in self.holders.values_mut() {
                holder.end(0);
            }
            return Ok(samples);
        }
        let unnamed = self
            .countries
            .as_deref()
            .expect("resolved production countries")
            .unnamed;
        self.result.unnamed += unnamed;
        if !read {
            // Carried: the last read still holds, only for the same identity.
            let observation = self.observation;
            for holder in self.holders.values_mut() {
                holder.carry(0, observation, consecutive);
            }
            return Ok(samples);
        }
        self.result.reads.push(hour);
        // A new observation restarts every count; totals never cross it.
        let mut restart = false;
        if let Some(since) = self.fold.since() {
            match self.result.observations.last() {
                None => self.result.observations.push(since),
                Some(current) if *current != since => {
                    self.result.observations.push(since);
                    self.observation += 1;
                    restart = true;
                }
                Some(_) => {}
            }
        }
        // Explicit suffix values also authenticate provisional source
        // fixtures; empty publications require the introduction ledger.
        if !self.spending_recorded {
            self.spending_recorded = self.fold.recorded();
        }
        if !self.spending_recorded {
            self.result.spending_taint(
                self.observation,
                hour,
                ProductionSpendingUnknown::NotRecorded,
            );
        }
        let observation = self.observation;
        self.scratch.read += 1;
        let number = self.scratch.read;
        let mut presence_unknown = false;
        let mut census = Vec::new();
        if complete.lines {
            for country in input.lines.countries() {
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.work.row_visits += 1;
                }
                match self.producers.get(&country) {
                    Some(&(slot, id)) => census.push((slot, id, observation)),
                    None => presence_unknown = true,
                }
            }
        }
        let folded = self.fold.read(&fold::Read {
            observation,
            number,
            restart,
            producers: &self.producers,
            census: &census,
        })?;
        #[cfg(any(test, feature = "test-support"))]
        {
            self.work.row_visits += folded.visits;
        }
        #[cfg(not(any(test, feature = "test-support")))]
        let _ = folded.visits;
        self.result.unattributed += folded.unattributed;
        self.result.inexact += folded.inexact;
        self.result.lost += folded.lost;
        // The first row whose spending cannot be told dates the reason; the taints after it
        // keep it.
        if let Some(reason) = folded.spending {
            self.result.spending_taint(observation, hour, reason);
        }
        if folded.tainted > 0 {
            self.result.missed += folded.tainted;
            self.result.tainted.entry(observation).or_insert(hour);
        }
        samples.equipment = Some(EquipmentPass {
            number,
            census_complete: complete.lines,
            presence_unknown: presence_unknown || folded.presence_unknown,
        });
        Ok(samples)
    }

    /// Each holder's samples of the hour's reads, and each live holder's spent, purpose and
    /// unproven series at `hour`. A holder whose identity, scope and continuity held since its
    /// last projection projects only the designs its entries changed at; the others it holds
    /// observe nothing a whole projection would. The holders take their steps here and on the
    /// run's idle workers; the series observe and make keys here, in the order a pass over
    /// every holder would.
    fn observe_holders(
        &mut self,
        hour: i32,
        samples: Samples,
        moved: bool,
        consecutive: bool,
    ) -> io::Result<()> {
        self.spent.advance(hour)?;
        let carried = self.spent_hour.and_then(|at| at.checked_add(1)) == Some(hour);
        // Without a moved track, country, taint or restart every live holder's inputs held
        // from the hour before: each projection goes on, as a visit would carry it.
        let projecting = moved || !self.spent_holders_ready || !(carried || self.spent_held == 0);
        if samples.equipment.is_none() && samples.rocket.is_none() && !projecting {
            self.spent_hour = Some(hour);
            return Ok(());
        }
        let tainted_from = earliest_taint(&self.result.spending_tainted);
        let Self {
            holders,
            fold,
            result,
            spent,
            spent_active,
            spent_keys,
            unproven_keys,
            purpose_keys,
            tables,
            observation,
            equipment_restart,
            rocket_restart,
            spent_holders_ready,
            spent_held,
            #[cfg(test)]
            spent_visits,
            #[cfg(any(test, feature = "test-support"))]
            work,
            ..
        } = self;
        let equipment = match samples.equipment {
            Some(pass) => Some(holders::EquipmentRead {
                fold: &*fold,
                observation: *observation,
                number: pass.number,
                census_complete: pass.census_complete,
                presence_unknown: pass.presence_unknown,
            }),
            None => None,
        };
        let rocket = match &samples.rocket {
            Some(totals) => Some(holders::RocketSample {
                totals,
                observation: result.rocket.observation(),
            }),
            None => None,
        };
        let spending = if projecting {
            Some(holders::SpendingRead {
                tainted_from,
                restarts: [*equipment_restart, *rocket_restart],
                consecutive: carried,
                spent: &*spent,
                active: &*spent_active,
                keys: holders::Held {
                    spent: &*spent_keys,
                    unproven: &*unproven_keys,
                    purposes: &*purpose_keys,
                },
                ids: &tables.holder_ids,
            })
        } else {
            None
        };
        let shared = holders::Shared {
            hour,
            consecutive,
            equipment,
            rocket,
            spending,
        };
        let mut entries: Vec<(&String, &mut holders::Holder)> = holders.iter_mut().collect();
        let count = entries.len();
        let given = shards::chunks(&mut entries, |_, chunk| {
            let mut visits = Visits::default();
            let observed = chunk
                .iter_mut()
                .map(|(tag, holder)| holder.observe(tag.as_str(), &shared, &mut visits))
                .collect::<io::Result<Vec<_>>>()?;
            Ok((observed, visits))
        })?;
        let mut visits = Visits::default();
        let mut observed = Vec::with_capacity(count);
        for (chunk, chunk_visits) in given {
            observed.extend(chunk);
            visits.add(chunk_visits);
        }
        if projecting {
            let mut opened = BTreeSet::new();
            let mut closed = BTreeSet::new();
            // Equipment's projections first, then Rocket's, each in tag order.
            for source in 0..2 {
                for ((tag, holder), observed) in entries.iter_mut().zip(observed.iter_mut()) {
                    let Some(observed) = observed else {
                        continue;
                    };
                    let spending =
                        std::mem::replace(&mut observed.spending[source], holders::Spending::Held);
                    let projection = match spending {
                        holders::Spending::Closed(keys) => {
                            closed.extend(keys);
                            continue;
                        }
                        holders::Spending::Held => continue,
                        holders::Spending::Projected(projection) => projection,
                        holders::Spending::Ordered(gathered) => {
                            let id = tables.holder(tag.as_str());
                            holder.project_ordered(
                                id,
                                source,
                                gathered,
                                holders::Maps::Making {
                                    spent: &mut *spent_keys,
                                    unproven: &mut *unproven_keys,
                                    purposes: &mut *purpose_keys,
                                },
                                spent,
                                spent_active,
                                &mut visits,
                            )?
                        }
                    };
                    for (key, value, starts) in projection.observed {
                        spent.observe(key, hour, value, starts);
                    }
                    opened.extend(projection.opened);
                    closed.extend(projection.closed);
                }
            }
            if !*spent_holders_ready {
                // The saved active keys precede this runtime index. Reconcile them
                // once, including keys belonging to a holder gone at the first read.
                let held: BTreeSet<u64> = entries
                    .iter()
                    .flat_map(|(_, holder)| holder.spent.iter().flatten())
                    .flat_map(|held| held.active.iter().copied())
                    .collect();
                closed.extend(spent_active.difference(&held).copied());
                opened = held;
                *spent_holders_ready = true;
            }
            // Keep the full projection's call order: values first, then ending
            // keys in numeric order. Failed projections publish no endings.
            for key in closed {
                spent.observe(key, hour, None, false);
                spent_active.remove(&key);
                visits.emitted += 1;
            }
            spent_active.extend(opened);
            *spent_held = entries
                .iter()
                .map(|(_, holder)| holder.spent.iter().flatten().count())
                .sum();
        }
        #[cfg(test)]
        {
            *spent_visits += visits.spent;
        }
        #[cfg(any(test, feature = "test-support"))]
        {
            work.history_visits += visits.history;
            work.emitted += visits.emitted;
        }
        #[cfg(not(any(test, feature = "test-support")))]
        let _ = visits;
        self.spent_hour = Some(hour);
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

/// A design's spending of each purpose in `track`, for the purposes either inventory holds;
/// none where neither holds the design.
fn design_amounts(
    track: &Track,
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
    track: &Track,
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

/// Brings `current`, a track's inventory taken at `taken`, to `totals`, `identity`'s: at only
/// the entries read `read` changed where it took them just before, else whole.
fn take_totals<V: Copy>(
    current: &mut BTreeMap<Entry, V>,
    edits: &mut crate::products::equipment::production::Edits,
    taken: Option<(Identity, u64)>,
    identity: Identity,
    totals: Option<&Versioned<V>>,
    read: u64,
) {
    if let (Some((held, version)), Some(totals)) = (taken, totals)
        && held == identity
        && let Some(changed) = totals.since(read, version)
    {
        for &entry in changed {
            match totals.entries.get(&entry) {
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
    match totals {
        Some(totals) => current.clone_from(&totals.entries),
        None => current.clear(),
    }
    edits.replaced();
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
    /// First hour of this source's unchanged spending, presence and coverage.
    pub spending_since: i32,
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
    pub absent_since: [Option<i32>; 2],
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

/// The earliest campaign spending taint: its hour and reason.
fn earliest_taint(
    spending_tainted: &BTreeMap<u32, (i32, ProductionSpendingUnknown)>,
) -> Option<(i32, ProductionSpendingUnknown)> {
    spending_tainted.values().min_by_key(|(at, _)| *at).copied()
}

/// A source's campaign spending reason at `hour`: from the earliest spending taint
/// ([`earliest_taint`]) on, the observation's restart on, or where its tag was first held by
/// another country.
fn spending_reason_from(
    earliest: Option<(i32, ProductionSpendingUnknown)>,
    first: Option<(u32, u32)>,
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
            .filter(|first| *first != identity)
            .map(|_| ProductionSpendingUnknown::CountryChanged)
    })
}

/// A live source's row at `hour` but its amounts and campaign spending reason: none where the
/// track does not hold through `hour`. A read that starts a line opens another continuity
/// segment, counted in `segment`.
fn source_head(
    track: &Track,
    hour: i32,
    origin: Option<i32>,
    segment: &mut Option<u64>,
    units_known: bool,
    spending_known: bool,
) -> io::Result<Option<SourceHead>> {
    let Some(identity) = track.live else {
        return Ok(None);
    };
    let sample = track.sample.expect("live production read");
    let read_hour = sample.hour;
    let segment = segment.get_or_insert(0);
    if read_hour == hour && sample.starts {
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
        spending_since: hour,
        units_known,
        spending_known,
        present: sample.presence,
        campaign_spending_reason: None,
    }))
}

/// `entry`'s amount in `track`: none where neither inventory holds it.
fn amount_of(track: &Track, entry: Entry) -> Option<Amount> {
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
fn amounts_of(track: &Track) -> Vec<Amount> {
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

/// What a source's campaign spending reason is read from: the earliest spending taint, the
/// slot and id its tag was first projected with, the observation restart, and whether it is
/// the Rocket source.
type Scope = (
    Option<(i32, ProductionSpendingUnknown)>,
    Option<(u32, u32)>,
    Option<i32>,
    bool,
);

/// A row's source at `hour` against the open page's, `held`: its head, and how it changed.
/// The amounts are read at only the entries the track changed since the rows last read it,
/// unless it was replaced or the open page lacks the source.
#[allow(clippy::too_many_arguments)]
fn source_edit(
    track: Option<&mut Track>,
    hour: i32,
    origin: Option<i32>,
    segment: &mut Option<u64>,
    (units_known, spending_known): (bool, bool),
    (tainted, first, restart, rocket): Scope,
    held: Option<&SourceEndpoint>,
    continuity: &mut Option<SpendingSource>,
    consecutive: bool,
    entries: &mut Vec<Entry>,
    edits: &mut Vec<(Entry, Option<Amount>)>,
) -> io::Result<(Option<SourceHead>, SourceEdit)> {
    let gone = if held.is_some() {
        SourceEdit::Gone
    } else {
        SourceEdit::Same
    };
    let Some(track) = track else {
        SpendingSource::absent(continuity, hour, consecutive);
        return Ok((None, gone));
    };
    let whole = track.edits.take(ROWS_READER, entries);
    let Some(mut head) = source_head(track, hour, origin, segment, units_known, spending_known)?
    else {
        SpendingSource::absent(continuity, hour, consecutive);
        return Ok((None, gone));
    };
    head.campaign_spending_reason = spending_reason_from(
        tainted,
        first,
        (head.slot, head.country_id),
        restart,
        rocket,
        hour,
    );
    if whole || held.is_none() || continuity.is_none() {
        let amounts = amounts_of(track);
        SpendingSource::whole(continuity, &mut head, &amounts, consecutive);
        let Some(held) = held else {
            return Ok((Some(head), SourceEdit::Whole(head.source(amounts))));
        };
        let source = head.source(amounts);
        let edit = if source == *held {
            SourceEdit::Same
        } else {
            SourceEdit::Whole(source)
        };
        return Ok((Some(head), edit));
    }
    let held = held.expect("an incremental source's held page");
    let saved = continuity
        .as_mut()
        .expect("an incremental source's spending record");
    let identity = spending_identity(head);
    let first = edits.len();
    let mut same_spending = consecutive && saved.identity == Some(identity);
    for &entry in entries.iter() {
        let now = amount_of(track, entry);
        let before = held_amount(&held.amounts, entry)
            .ok()
            .map(|at| &held.amounts[at]);
        let value = now.as_ref().map(SpendingSource::value);
        same_spending &= value == saved.amounts.get(&entry).copied();
        if let Some(value) = value {
            saved.amounts.insert(entry, value);
        } else {
            saved.amounts.remove(&entry);
        }
        if now.as_ref() != before {
            edits.push((entry, now));
        }
    }
    if !same_spending {
        saved.since = head.spending_since;
    }
    saved.identity = Some(identity);
    head.spending_since = saved.since;
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
                absent_since,
            } = edit
            else {
                unreachable!("only changes");
            };
            edited[held] = modify(
                &mut rows[held],
                sources,
                joined,
                absent_since,
                amounts,
                entries,
            );
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
                    sources,
                    joined,
                    absent_since,
                    ..
                }) = edits.next()
                else {
                    unreachable!("a peeked change");
                };
                edited.push(modify(
                    &mut row,
                    sources,
                    joined,
                    absent_since,
                    amounts,
                    entries,
                ));
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
    absent_since: [Option<i32>; 2],
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
    row.absent_since = absent_since;
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
        spending_since: source.spending_since,
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
    placed_source(source, read, &mut |variant| Some(tables.variant(variant)))
        .map(|placed| placed.expect("every variant placed"))
}

/// A source's facts, read whole, its variants at the places `place` gives in entry order; none
/// where it gives none.
fn placed_source(
    source: &SourceEndpoint,
    read: Option<i32>,
    place: &mut impl FnMut(u64) -> Option<u32>,
) -> io::Result<Option<facts::Source>> {
    let mut leaves = Vec::with_capacity(source.amounts.len());
    for amount in &source.amounts {
        let Some(variant) = place(amount.variant) else {
            return Ok(None);
        };
        leaves.push(leaf_of(variant, amount)?);
    }
    leaves.sort_unstable_by_key(Leaf::key);
    Ok(Some(facts::Source {
        head: facts_head(source, read),
        leaves,
    }))
}

/// How a row of an endpoint changed its holder since the batch's last endpoint, `previous`,
/// its variants at the places `place` gives: none where it gives none; held where nothing
/// changed. Only the entries `edit` names are read where the source was not read whole.
fn row_change(
    row: &CountryEndpoint,
    holder: u32,
    (edit, entries): (&Edited, &[Entry]),
    reads: [Option<i32>; 2],
    previous: Option<&Holder>,
    place: &mut impl FnMut(u64) -> Option<u32>,
) -> io::Result<Option<Option<HolderChange>>> {
    let mut moved = previous.is_none_or(|previous| {
        previous.rocket_joined != row.rocket_joined || previous.absent_since != row.absent_since
    });
    let mut sources = [SourceChange::Absent, SourceChange::Absent];
    for (slot, (source, read)) in [
        (row.equipment.as_ref(), reads[0]),
        (row.rocket.as_ref(), reads[1]),
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
                let mut leaves = Vec::new();
                let (first, last) = edit.entries[slot];
                for &entry in &entries[first as usize..last as usize] {
                    let Some(variant) = place(entry.0) else {
                        return Ok(None);
                    };
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
                        leaves.push((key, leaf));
                    }
                }
                leaves.sort_unstable_by_key(|(key, _)| *key);
                moved |= head != before.head || !leaves.is_empty();
                sources[slot] = SourceChange::Leaves(head, leaves);
            }
            _ => {
                let Some(whole) = placed_source(source, read, place)? else {
                    return Ok(None);
                };
                moved |= before != Some(&whole);
                sources[slot] = SourceChange::Whole(whole);
            }
        }
    }
    if !moved {
        return Ok(Some(None));
    }
    let now = HolderNow {
        sources,
        joined: row.rocket_joined,
        absent_since: row.absent_since,
    };
    let mut patch = Out::default();
    write_change(holder, previous, Some(&now), &mut patch)?;
    Ok(Some(Some(HolderChange {
        holder,
        now: Some(now),
        patch: patch.0,
    })))
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
                || held
                    .iter()
                    .zip(&kind.names)
                    .any(|(held, name)| *held != code(name))
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
    /// The last definitions census, which read every held name.
    census: Option<i32>,
    /// [`ProductionVariants::known`].
    known: Option<i32>,
}

impl ClassEdits<'_> {
    fn mark(&mut self, variant: u64) {
        *self.moved = true;
        self.changed.push(variant);
    }

    /// The read a held catalogue name changed at `changed` came from: that change, or the
    /// census or restoring read after it.
    fn read_at(&self, changed: i32) -> i32 {
        changed.max(self.census.max(self.known).unwrap_or(changed))
    }

    /// Dates every name of `variant` at the read it came from by `at`: its own date or the
    /// census after it. A design-edit or output read does not read names it did not change.
    fn redate(&mut self, variant: u64, at: i32) {
        let Some(kind) = self.held.get_mut(&variant) else {
            return;
        };
        let own = kind.names.iter().map(|name| name.first_hour).max();
        let hour = own.max(self.census).map_or(at, |hour| hour.min(at));
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
    classified: &mut BTreeSet<u64>,
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
            first_hour: classes.read_at(version.first_hour),
            raw: version.raw.as_ref().map(|raw| tables.raw(raw)),
        };
        if names.last().is_none_or(|last| last.raw != name.raw) {
            names.push(name);
        }
    }
    if names.is_empty() {
        names.push(VariantName {
            first_hour: at,
            raw: None,
        });
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
        // A row frame holds every row: each hour's output is read whole.
        let output = frame
            .complete::<tables::ProductionLineOutput>()
            .ok()
            .map(|rows| {
                OutputChanges::new(
                    true,
                    rows.iter().map(columnar::output_row).collect(),
                    Vec::new(),
                    rows.iter().map(|row| row.since_hour).min(),
                    rows.iter()
                        .any(|row| !matches!(row.spent_raw, Recorded::NotRecorded)),
                )
            });
        let hour = Hour {
            hour: frame.hour,
            countries,
            changed: Changed {
                others: TABLES
                    .iter()
                    .filter(|&&table| {
                        !matches!(
                            table,
                            TableId::Campaign
                                | TableId::Countries
                                | TableId::CountryLinks
                                | TableId::VariantChanges
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
            naming_moved: !consecutive || NAMING.iter().any(|&table| changed(table)),
            output,
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
        if let Some(output) = hour.output.take() {
            self.fold.push(output)?;
        }
        self.names_dirty |= hour.naming_moved;
        self.spent.advance(hour.hour)?;
        let consecutive = self.last_frame.and_then(|last| last.checked_add(1)) == Some(hour.hour);
        let countries_changed = match hour.countries.take() {
            Some(countries) => {
                self.set_countries(countries);
                true
            }
            None => false,
        };
        let changed = self.schema != SCHEMA
            || !consecutive
            || countries_changed
            || hour.changed.others
            || hour.dated.any()
            || hour.design_changed && self.ledger_moves(hour.complete);
        if !changed && let Some(pending) = &mut self.pending {
            pending.page.last_hour = hour.hour;
            self.advance_spent(hour.hour);
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

    /// Whether a ledger hour's naming read at `complete` would change what names hold.
    pub(crate) fn ledger_moves(&self, complete: Complete) -> bool {
        let variants = &self.variants;
        if complete.countries && complete.links && complete.lexical && complete.definitions {
            self.names_dirty || variants.known.is_none() || !variants.invalidated.is_empty()
        } else {
            variants.known.is_some() || !variants.classified.is_empty()
        }
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
    /// open endpoint page and the spent series to it, carrying the holders'
    /// projections without visiting them. False without an open page, whose
    /// hours only ingestion opens.
    pub fn advance_quiet(&mut self, first: i32, last: i32) -> io::Result<bool> {
        if !self.quiet_from(first) {
            return Ok(false);
        }
        self.spent.advance_span(first, last)?;
        if let Some(pending) = &mut self.pending {
            pending.page.last_hour = last;
        }
        self.advance_spent(last);
        self.last_frame = Some(last);
        Ok(true)
    }

    /// The unnamed country-hours through the last hour taken. Every live track holds through
    /// that hour, which no holder is visited to say.
    fn seal_holders(&mut self) {
        let Some(hour) = self.last_frame else {
            return;
        };
        let Some(Pending { page, .. }) = &self.pending else {
            return;
        };
        self.result.unnamed = page.unnamed_at(hour);
        self.final_unnamed = self.result.unnamed;
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
        let samples = self.observe_rows(&hour, equipment_changed, rocket_changed, consecutive)?;
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
        self.observe_holders(at, samples, moved, consecutive)?;
        drop(cpu);
        let numeric_read = hour.reads.output || hour.reads.rocket;
        let names_moved =
            hour.reads.variants || hour.design_changed && self.ledger_moves(hour.complete);
        if consecutive
            && !numeric_read
            && !names_moved
            && diagnostics_before == self.diagnostic_counts()
            && self.pending.as_ref().is_some_and(|pending| {
                pending.page.unnamed_per_hour == 0
                    && self.same_holders(&pending.page, equipment_changed || rocket_changed)
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
        // Mutations refresh operands; only a definitions census dates every selected name.
        let complete_names = names_read && named && complete.definitions;
        if complete_names {
            self.variants
                .read(at, hour.variants, &mut self.variant_refusals);
            self.names_dirty = false;
            self.variants.known.get_or_insert(at);
            if hour.reads.variants {
                self.names_hour = Some(at);
            }
        }
        let rows_moved = self.present(
            at,
            (complete_names, hour.reads.variants),
            moved,
            consecutive,
            cancelled,
        )?;
        drop(cpu);
        let cpu = ::replay_frame::cpu::scope_phase("production:endpoint_projection");
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
            self.pending
                .as_mut()
                .expect("a held endpoint")
                .page
                .last_hour = at;
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
                    if previous.holds_hours() {
                        self.complete(&previous)?;
                    }
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
                continues: false,
            });
        }
        self.classes_moved = false;
        drop(cpu);
        let _pruning = ::replay_frame::cpu::scope_phase("production:pruning");
        // Only current supplying reads survive a frame.
        retain_last(&mut self.result.reads);
        retain_last(&mut self.result.rocket.reads);
        for since in &self.result.observations {
            if self.observation_starts.last() != Some(since) {
                self.observation_starts.push(*since);
            }
        }
        retain_last(&mut self.result.observations);
        retain_last(&mut self.result.rocket.observations);
        self.seal_holders();
        check_cancel(cancelled)?;
        Ok(())
    }

    /// Each holder's inventories, where `inventory`, and its row of the open page, where
    /// `rows`, stepped here and on idle workers: what each run of holders gave, in tag order.
    fn pass_holders(
        &mut self,
        at: i32,
        (inventory, rows): (bool, bool),
        consecutive: bool,
        cancelled: &AtomicBool,
    ) -> io::Result<(Vec<holders::Presented>, Vec<holders::RowsOut>)> {
        if !inventory && !rows {
            return Ok((Vec::new(), Vec::new()));
        }
        let Self {
            holders,
            result,
            pending,
            equipment_restart,
            rocket_restart,
            ..
        } = self;
        let held: &[CountryEndpoint] = match pending {
            Some(pending) => &pending.page.countries,
            None => &[],
        };
        let shared = holders::RowsShared {
            at,
            consecutive,
            origins: [
                result.observations.last().copied(),
                result.rocket.observations.last().copied(),
            ],
            tainted: &result.tainted,
            spending_tainted: &result.spending_tainted,
            tainted_from: earliest_taint(&result.spending_tainted),
            restarts: [*equipment_restart, *rocket_restart],
            held,
        };
        let mut entries: Vec<(&String, &mut holders::Holder)> = holders.iter_mut().collect();
        let given = shards::chunks(&mut entries, |_, chunk| {
            check_cancel(cancelled)?;
            let mut presented = holders::Presented::default();
            let mut out = holders::RowsOut::default();
            let mut next = chunk.first().map_or(held.len(), |(tag, _)| {
                held.partition_point(|row| row.tag.as_str() < tag.as_str())
            });
            for (tag, holder) in chunk.iter_mut() {
                if inventory {
                    holder.inventory(&mut presented);
                }
                if rows {
                    holder.rows(tag.as_str(), &mut next, &shared, &mut out)?;
                }
            }
            Ok((presented, out))
        })?;
        Ok(given.into_iter().unzip())
    }

    /// Brings the holders' inventories and rows to `at`, as `moved` asks, then updates the
    /// classifications from inventory edits, naming edits or a definitions census: true where
    /// the open page's rows change, which [`Scratch::rows`] holds with the amount edits they
    /// name.
    fn present(
        &mut self,
        at: i32,
        (complete_names, census): (bool, bool),
        moved: bool,
        consecutive: bool,
        cancelled: &AtomicBool,
    ) -> io::Result<bool> {
        let inventory = moved || !self.scratch.inventoried;
        let rows = moved || self.pending.is_none();
        let (presented, rows_out) =
            self.pass_holders(at, (inventory, rows), consecutive, cancelled)?;
        if inventory {
            self.variants.members(presented, &mut self.scratch.members);
            self.scratch.inventoried = true;
        } else {
            self.scratch.members.clear();
        }
        self.classify_present(at, complete_names, census);
        let scratch = &mut *self.scratch;
        scratch.rows.clear();
        scratch.amount_edits.clear();
        for out in rows_out {
            let base = scratch.amount_edits.len() as u32;
            scratch.amount_edits.extend(out.amounts);
            for mut edit in out.edits {
                if let RowEdit::Modify { sources, .. } = &mut edit {
                    for source in sources {
                        if let SourceEdit::Head(_, (first, last)) = source {
                            *first += base;
                            *last += base;
                        }
                    }
                }
                scratch.rows.push(edit);
            }
            self.corrections.extend(out.corrections);
            if let Some(pending) = &mut self.pending {
                for row in out.unjoined {
                    pending.page.countries[row].rocket_joined = false;
                }
            }
        }
        Ok(!scratch.rows.is_empty())
    }

    /// Updates selected classifications from the inventories' member changes, naming edits or
    /// a definitions census.
    fn classify_present(&mut self, at: i32, complete_names: bool, census: bool) {
        let Self {
            variants,
            scratch,
            classifications,
            classes_moved,
            class_changed,
            tables,
            names_hour,
            ..
        } = self;
        let Scratch {
            members: changed,
            created,
            unclassified,
            redated,
            ..
        } = &mut **scratch;
        let ProductionVariants {
            catalogue: read,
            classified,
            invalidated,
            known,
            members,
            changed: naming_changed,
            #[cfg(test)]
                classifications: built,
            ..
        } = variants;
        // The held catalogue names a design at any hour, unless names were forgotten since.
        let empty = VariantCatalogue::default();
        let catalogue = if known.is_some() { &*read } else { &empty };
        let mut classes = ClassEdits {
            held: classifications,
            moved: classes_moved,
            changed: class_changed,
            census: *names_hour,
            known: *known,
        };
        created.clear();
        let mut builds = 0usize;
        if complete_names && census {
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
            for &variant in changed
                .iter()
                .chain(naming_changed.iter().filter(|_| complete_names))
            {
                // A renamed design left `classified` in `name_variants`; an inventory change still redates.
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
        if known.is_none() {
            // Named unknown while forgotten: the read that restores names names them too.
            invalidated.extend(created.iter().copied());
        }
        if complete_names && !census {
            for &variant in naming_changed.iter() {
                if !catalogue.versions.contains_key(&variant)
                    && let Some(kind) = classes.held.get_mut(&variant)
                {
                    kind.names = vec![VariantName {
                        first_hour: at,
                        raw: None,
                    }];
                    classes.mark(variant);
                }
            }
        }
        if census {
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
            && recorded.unlisted == self.fold.unlisted()
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
                unlisted: self.fold.unlisted(),
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

    /// Whether `page`'s rows still hold every live source as it was read. Where no track moved
    /// since the page was made or held from them, they do.
    fn same_holders(&self, page: &EndpointPage, tracks_moved: bool) -> bool {
        if self.classes_moved {
            return false;
        }
        if !tracks_moved {
            return true;
        }
        let present = self
            .holders
            .values()
            .filter(|holder| {
                holder
                    .tracks
                    .iter()
                    .flatten()
                    .any(|track| track.live.is_some())
            })
            .count();
        // The page's rows are in tag order, as the holders are.
        let mut held = self.holders.iter().peekable();
        page.countries.len() == present
            && page.countries.iter().all(|row| {
                let holder = walked(&mut held, &row.tag);
                let track =
                    |source: usize| holder.and_then(|holder| holder.tracks[source].as_ref());
                source_holds(row.equipment.as_ref(), track(0))
                    && source_holds(row.rocket.as_ref(), track(1))
            })
    }
}

/// Binds a tag's Rocket read `observed` to its equipment read `equipment` (slot, country,
/// observation and read hour): true while the join holds. A join whose Rocket read ended is
/// withdrawn, its correction given.
fn bind_rocket(
    bound: &mut Option<RocketBound>,
    equipment: (u32, u32, Option<i32>, i32),
    observed: Option<RocketBinding>,
    tag: &str,
    hour: i32,
) -> (bool, Option<CoverageCorrection>) {
    let (slot, country_id, observation, read_hour) = equipment;
    let bound = bound.get_or_insert_with(|| RocketBound {
        slot,
        country_id,
        observation,
        equipment_read: read_hour,
        rocket: observed.clone(),
    });
    let mut withdrawn = None;
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
        withdrawn = Some(CoverageCorrection {
            tag: tag.to_owned(),
            slot,
            country_id,
            observation,
            read_hour,
            first_hour: read_hour,
            last_hour: hour,
        });
    }
    (bound.rocket.is_some(), withdrawn)
}

#[cfg(test)]
impl ProductionAccumulator {
    /// Drops every holder's spending projection, as a resumed accumulator holds none.
    pub(crate) fn forget_projections(&mut self) {
        for holder in self.holders.values_mut() {
            holder.spent = Default::default();
        }
        self.spent_held = 0;
        self.spent_holders_ready = false;
    }

    /// `tag`'s track of `source`, equipment's 0 and Rocket's 1.
    fn track(&mut self, tag: &str, source: usize) -> &mut Track {
        self.holders
            .get_mut(tag)
            .and_then(|holder| holder.tracks[source].as_mut())
            .expect("a held track")
    }

    /// Binds `tag`'s Rocket read as [`bind_rocket`] does, withdrawing the join it ends from
    /// `pending`.
    pub(crate) fn bind_rocket(
        &mut self,
        tag: &str,
        equipment: &SourceEndpoint,
        observed: Option<&SourceEndpoint>,
        hour: i32,
        pending: Option<&mut EndpointPage>,
    ) -> bool {
        let holder = self.holders.entry(tag.to_owned()).or_default();
        let (joined, withdrawn) = bind_rocket(
            &mut holder.bound,
            (
                equipment.slot,
                equipment.country_id,
                equipment.observation,
                equipment.read_hour,
            ),
            observed.map(RocketBinding::from),
            tag,
            hour,
        );
        if let Some(correction) = withdrawn {
            if let Some(page) = pending {
                for row in &mut page.countries {
                    if correction.matches(row) {
                        row.rocket_joined = false;
                    }
                }
            }
            self.corrections.push(correction);
        }
        joined
    }
}

fn source_holds(source: Option<&SourceEndpoint>, track: Option<&Track>) -> bool {
    let live = track.and_then(|track| track.live.map(|identity| (track, identity)));
    match (source, live) {
        (None, None) => true,
        (Some(source), Some((track, identity))) => {
            source.slot == identity.0
                && source.country_id == identity.1
                && track.sample.map(|sample| sample.hour) == Some(source.read_hour)
        }
        _ => false,
    }
}

/// `tag`'s value among `held`, which is walked in step with ascending tags.
fn walked<'a, V>(
    held: &mut std::iter::Peekable<impl Iterator<Item = (&'a String, V)>>,
    tag: &str,
) -> Option<V> {
    while held.next_if(|(at, _)| at.as_str() < tag).is_some() {}
    held.next_if(|(at, _)| at.as_str() == tag)
        .map(|(_, value)| value)
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
pub const STATE: &str = "production-state-v9";

/// One holder's leaf events in a batch (P3), in groups that each begin with its whole state;
/// the batch's write compresses each group.
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
    bytes: Block,
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
        if self.events as usize == facts::HOLDER_GROUP {
            self.close();
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
        Ok(self.open.0.len() - start)
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

    fn close(&mut self) {
        if self.events == 0 {
            return;
        }
        self.groups.push(HolderGroup {
            first: self.first,
            last: self.last,
            events: self.events,
            bytes: Block::Raw(std::mem::take(&mut self.open.0)),
        });
        self.events = 0;
    }
}

/// The facts of the endpoints completed since the last flush.
#[derive(Default)]
struct Batch {
    first_endpoint: Option<u64>,
    /// Each endpoint completed, and its ranking block.
    endpoints: Vec<(Endpoint, Block)>,
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
    packed: usize,
}

/// Raw bytes a batch adds between packings of its finished blocks.
const PACK_BYTES: usize = 8 << 20;

fn spill_here() -> Option<Spill> {
    engine_allocation::spill::room().map(|room| Spill::new(room.folder()))
}

/// A ranking block or closed holder group, spilled once final.
enum Block {
    Raw(Vec<u8>),
    Packed(Vec<u8>),
    Spilled(Piece),
}

impl Block {
    fn len(&self) -> io::Result<u64> {
        match self {
            Self::Packed(bytes) => Ok(bytes.len() as u64),
            Self::Spilled(piece) => Ok(piece.len),
            Self::Raw(_) => Err(io::Error::other("A production block was never compressed")),
        }
    }

    fn place(self, held: &mut Vec<u8>, pieces: &mut Vec<Piece>) {
        match self {
            Self::Packed(bytes) => held.extend_from_slice(&bytes),
            Self::Spilled(piece) => pieces.push(piece),
            Self::Raw(_) => {}
        }
    }
}

impl Batch {
    /// Compresses every raw finished block on idle workers, spilling it where there is a spill.
    fn pack(&mut self, spill: Option<&mut Spill>) -> io::Result<()> {
        let mut blocks: Vec<&mut Block> = self
            .streams
            .values_mut()
            .flat_map(|stream| stream.groups.iter_mut().map(|group| &mut group.bytes))
            .chain(self.endpoints.iter_mut().map(|(_, block)| block))
            .filter(|block| matches!(block, Block::Raw(_)))
            .collect();
        shards::chunks(&mut blocks, |_, chunk| {
            for block in chunk {
                if let Block::Raw(bytes) = &**block {
                    **block = Block::Packed(crate::prepared::series::compress(bytes, 3)?);
                }
            }
            Ok(())
        })?;
        if let Some(spill) = spill {
            for block in blocks {
                if let Block::Packed(bytes) = &*block {
                    *block = Block::Spilled(spill.append(bytes)?);
                }
            }
            spill.seal()?;
        }
        self.packed = self.bytes;
        Ok(())
    }

    /// Whether the next endpoint's ranking block states every holder.
    fn keyframe(&self) -> bool {
        self.endpoints.len() % facts::RANKING_KEYFRAME == 0
    }

    /// An endpoint's record and ranking block. Returns the bytes they added.
    fn endpoint(
        &mut self,
        page: &EndpointPage,
        names_hour: Option<i32>,
        keyframe: bool,
        block: Vec<u8>,
    ) -> usize {
        let added = block.len();
        self.endpoints.push((
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
                ranking_offset: 0,
                ranking_len: 0,
                keyframe,
                continues: false,
            },
            Block::Raw(block),
        ));
        added
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
            continues,
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
                self.complete_changed(page, (&edits[..], &entries[..]), endpoint, names_hour)?
            }
            None => self.complete_whole(page, endpoint, names_hour)?,
        };
        if let Some((record, _)) = self.batch.endpoints.last_mut() {
            record.continues = *continues;
        }
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
        if self.batch.bytes - self.batch.packed >= PACK_BYTES {
            if self.spill.is_none() {
                self.spill = spill_here();
            }
            if let Some(spill) = &mut self.spill {
                self.batch.pack(Some(spill))?;
            }
        }
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
                    absent_since: row.absent_since,
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
        let keyframe = batch.keyframe();
        let mut block = Out::default();
        for &holder in &holders {
            let before = if keyframe {
                None
            } else {
                previous.get(&holder)
            };
            facts::write_patch(holder, before, now.get(&holder), &mut block)?;
        }
        let mut added = batch.endpoint(page, names_hour, keyframe, block.0);
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

    /// Each row's holder id, as a pass over `page` in order assigns them.
    fn page_ids(&mut self, page: &EndpointPage) -> Vec<u32> {
        let Self {
            holders, tables, ..
        } = self;
        let mut held = holders.iter_mut().peekable();
        let mut ids = Vec::with_capacity(page.countries.len());
        for row in &page.countries {
            while held
                .peek()
                .is_some_and(|(tag, _)| tag.as_str() < row.tag.as_str())
            {
                held.next();
            }
            let id = match held.peek_mut() {
                Some((tag, holder)) if tag.as_str() == row.tag.as_str() => {
                    *holder.id.get_or_insert_with(|| tables.holder(&row.tag))
                }
                _ => tables.holder(&row.tag),
            };
            ids.push(id);
        }
        ids
    }

    /// The holders of `page` whose rows `edits` changed since the batch's last endpoint, or
    /// whose heads read differently now, patched against that endpoint's; the rest hold.
    /// Each row is diffed here and on the run's idle workers where its variants hold table
    /// places already, else in page order, which places them as a whole read meets them.
    /// Returns the bytes it added.
    fn complete_changed(
        &mut self,
        page: &EndpointPage,
        (edits, entries): (&[Edited], &[Entry]),
        endpoint: u64,
        names_hour: Option<i32>,
    ) -> io::Result<usize> {
        let ids = self.page_ids(page);
        let Self {
            tables,
            batch,
            present_holders,
            scratch,
            ..
        } = self;
        let reads = [page.equipment_read, page.rocket_read];
        let (placed, previous) = (&tables.variant_ids, &batch.holders);
        let rows = &page.countries[..];
        let mut order: Vec<usize> = (0..rows.len()).collect();
        let given = shards::chunks(&mut order, |_, chunk| {
            chunk
                .iter()
                .map(|&at| {
                    row_change(
                        &rows[at],
                        ids[at],
                        (&edits[at], entries),
                        reads,
                        previous.get(&ids[at]),
                        &mut |variant| placed.get(&variant).copied(),
                    )
                })
                .collect::<io::Result<Vec<_>>>()
        })?;
        let mut changes = Vec::new();
        for (at, change) in given.into_iter().flatten().enumerate() {
            let change = match change {
                Some(change) => change,
                None => row_change(
                    &rows[at],
                    ids[at],
                    (&edits[at], entries),
                    reads,
                    batch.holders.get(&ids[at]),
                    &mut |variant| Some(tables.variant(variant)),
                )?
                .expect("every variant placed"),
            };
            changes.extend(change);
        }
        let sorted = &mut scratch.holders;
        sorted.clear();
        sorted.extend_from_slice(&ids);
        sorted.sort_unstable();
        for &holder in batch.holders.keys() {
            if sorted.binary_search(&holder).is_err() {
                let mut patch = Out::default();
                write_change(holder, None, None, &mut patch)?;
                changes.push(HolderChange {
                    holder,
                    now: None,
                    patch: patch.0,
                });
            }
        }
        changes.sort_unstable_by_key(|change| change.holder);
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
                    absent_since: [None; 2],
                }
            });
            state.rocket_joined = now.joined;
            state.absent_since = now.absent_since;
            for (slot, source) in now.sources.into_iter().enumerate() {
                let held = state.source_mut(slot);
                match source {
                    SourceChange::Absent => *held = None,
                    SourceChange::Whole(source) => *held = Some(source),
                    SourceChange::Leaves(head, leaves) => {
                        let held = held.as_mut().expect("a patched source's leaves");
                        held.head = head;
                        for (key, leaf) in leaves {
                            match (held.leaves.binary_search_by_key(&key, Leaf::key), leaf) {
                                (Ok(at), Some(leaf)) => held.leaves[at] = leaf,
                                (Ok(at), None) => {
                                    held.leaves.remove(at);
                                }
                                (Err(at), Some(leaf)) => held.leaves.insert(at, leaf),
                                (Err(_), None) => {}
                            }
                        }
                    }
                }
            }
        }
        // Ranking (P2): every holder at a keyframe, else what changed.
        let keyframe = batch.keyframe();
        let mut block = Out::default();
        if keyframe {
            for (&holder, state) in &batch.holders {
                facts::write_patch(holder, None, Some(state), &mut block)?;
            }
        } else {
            for change in &changes {
                block.0.extend_from_slice(&change.patch);
            }
        }
        let mut added = batch.endpoint(page, names_hour, keyframe, block.0);
        // Each holder's leaves (P3), wherever they changed.
        for change in &changes {
            added += batch.streams.entry(change.holder).or_default().push_patch(
                change.holder,
                endpoint,
                &change.patch,
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
    out: &mut Out,
) -> io::Result<()> {
    out.u32(holder);
    let Some(now) = now else {
        out.u8(0);
        return Ok(());
    };
    let present = |slot: usize| !matches!(now.sources[slot], SourceChange::Absent);
    out.u8(1 | u8::from(present(0)) << 1 | u8::from(present(1)) << 2 | u8::from(now.joined) << 3);
    for since in now.absent_since {
        out.opt_i32(since);
    }
    for (slot, source) in now.sources.iter().enumerate() {
        let previous = before.and_then(|before| before.source(slot));
        match source {
            SourceChange::Absent => {}
            SourceChange::Whole(source) => facts::write_source(previous, source, out)?,
            SourceChange::Leaves(head, leaves) => facts::write_leaves(
                previous.expect("a patched source's leaves"),
                head,
                leaves,
                out,
            )?,
        }
    }
    Ok(())
}

impl ProductionAccumulator {
    /// Completes the open endpoint and appends the batch's facts and spent
    /// series. The endpoint goes on past the flush: the hours after it that hold
    /// it extend it, published as its continuation, so where a flush falls
    /// changes no endpoint's span.
    pub fn flush(&mut self, writer: &mut store::Writer, cancelled: &AtomicBool) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope_phase("production:series_flush");
        self.seal_holders();
        if let Some(mut pending) = self.pending.take() {
            if pending.holds_hours() {
                self.complete(&pending)?;
            }
            self.classes_moved = false;
            let page = &mut pending.page;
            page.unnamed_before = page.unnamed_at(page.last_hour);
            page.first_hour = page
                .last_hour
                .checked_add(1)
                .ok_or_else(|| io::Error::other("Production hours exceed their range"))?;
            pending.edited = None;
            // It names nothing: the names its readers take are the endpoint's it continues.
            pending.named = Some(Vec::new());
            pending.continues = true;
            self.pending = Some(pending);
        }
        self.spent
            .flush(writer, &spent_section(self.branch), 0, cancelled)?;
        let (head, pieces) = self.facts_range()?;
        let section = section(FACTS, self.branch);
        match (&mut self.spill, pieces) {
            (Some(spill), Some(pieces)) => {
                writer.write(&section, io::Cursor::new(head).chain(spill.read(pieces)))?;
                spill.clear();
            }
            _ => {
                writer.write(&section, head.as_slice())?;
            }
        }
        Ok(())
    }

    /// The batch's range, or its preamble and head and the spilled blocks that follow them.
    fn facts_range(&mut self) -> io::Result<(Vec<u8>, Option<Vec<Piece>>)> {
        let mut batch = std::mem::take(&mut self.batch);
        let first_endpoint = batch.first_endpoint.unwrap_or(self.pages_written);
        for stream in batch.streams.values_mut() {
            stream.close();
        }
        if self.spill.is_none() {
            self.spill = spill_here();
        }
        batch.pack(self.spill.as_mut())?;
        let spilled = self.spill.is_some();
        let mut pieces = Vec::new();
        let endpoint_count = u32::try_from(batch.endpoints.len())
            .map_err(|_| io::Error::other("Production endpoints exceed their integer range"))?;
        let mut endpoints = Out::default();
        let mut ranking = Vec::new();
        let mut ranking_len = 0;
        for (mut record, block) in std::mem::take(&mut batch.endpoints) {
            let len = block.len()?;
            record.ranking_offset = ranking_len;
            record.ranking_len = u32::try_from(len)
                .map_err(|_| io::Error::other("A production ranking block exceeds 4 GiB"))?;
            ranking_len += len;
            block.place(&mut ranking, &mut pieces);
            record.write(&mut endpoints);
        }
        let mut directory = Out::default();
        let mut groups = Vec::new();
        let mut holders_len = 0;
        directory.count(batch.streams.len())?;
        for (holder, stream) in std::mem::take(&mut batch.streams) {
            directory.u32(holder);
            directory.count(stream.groups.len())?;
            for group in stream.groups {
                let len = group.bytes.len()?;
                directory.u64(group.first);
                directory.u64(group.last);
                directory.u64(holders_len);
                directory.count(usize::try_from(len).map_err(io::Error::other)?)?;
                directory.count(group.events as usize)?;
                holders_len += len;
                group.bytes.place(&mut groups, &mut pieces);
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
        let corrections = std::mem::take(&mut self.corrections);
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
        head.u32(endpoint_count);
        head.region(&endpoints.0);
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
            ranking_len,
            holders_len,
        }
        .write(&mut out);
        out.0.extend_from_slice(&head);
        if spilled {
            return Ok((out.0, Some(pieces)));
        }
        out.0.extend_from_slice(&ranking);
        out.0.extend_from_slice(&groups);
        Ok((out.0, None))
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
            continues: false,
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
mod parity_tests;
#[cfg(test)]
#[path = "prepared_production/tests.rs"]
mod tests;
