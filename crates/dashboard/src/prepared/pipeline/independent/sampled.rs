//! The sampled families prepared on their own: air force size, navy size,
//! stability's parts, equipment losses, the party history and trade flows.
//! Their widgets answer at open through samplers; their exact history is this
//! preparation. (Fuel, sampled too, is
//! read daily and prepared with the economy ledgers.)
//!
//! Each observer reads tables that move nearly every hour while countries
//! fight or produce (wing equipment, ship strength, sliced stability and
//! command-power reads, combat-log losses, resource
//! exchanges), and carries state from one hour to the next that
//! decides what the next adds (open runs and their parts, composition
//! versions, sliced read dates, open loss days, open deals). An hour's observation already does nothing where nothing it
//! reads changed. So no block is read by its changes alone, and no block can
//! be observed apart from the block before. The family's own thread observes
//! each block in order, as the hourly walk observes it: on the block read
//! alone with the tables the observer reads, from the shared read of every
//! block ([`super::shared`]), with the change facts and recorded updates of
//! the block alone, which are the walk's at every hour (every block starts
//! with no change facts of the block before). Its pre-pass extract keeps
//! nothing.
//!
//! Air force and navy size fold their blocks from pre-pass workers instead
//! (`parts_race_prepared::block`): a worker steps a census of its own through
//! the block read alone, keeping the census right after the block's first
//! hour and each later hour's changed rows. The family's thread observes the
//! first hour on the carried state; when the carried census then decides the
//! block's later hours as the worker's did (the same last hour and read
//! evidence, country join, census readiness, live tracks' last runs,
//! identities and segments as the next hour extends them, and counter rates;
//! air's variant names and current versions; navy's per-country notes), it
//! renumbers the ship classes a navy worker numbered in its own order as the
//! carried census numbers them, projects each later hour's rows through the
//! content (hours apart, on several threads), publishes them on the carried
//! series and carries the census's tracks and counters to the block's end.
//! Otherwise it observes the rest of the block hour by hour.
//!
//! What hourly preparation does around a block's hours is done at the same
//! points: it binds an observer's content at every block (air and navy part
//! content), and resumes the air and navy censuses at every
//! block from the frame its stream last accepted (the last frame of the
//! recording's block before, when that block is on the same branch). The
//! family keeps that frame from its own read of the block before, or reads
//! that block alone again where it did not observe it in this run. It ends
//! each block of equipment losses at the block's last hour (the branch's last
//! hour at its last block), as the walk does. Trade flows name their resources
//! at each flush from the source's content catalogue and resource cache, which
//! the family takes from the source at each block it observes.
//!
//! Each saves its state, which hourly preparation keeps in its branch state,
//! as `<family>-continuation-<branch>`, except the party history, whose saved
//! page (`party-history-<branch>`) is its state in both, and territory, whose
//! own read of the identity history is saved where the walk kept it
//! (`territory-identities-<branch>`). Each publishes and
//! flushes what hourly preparation does, where it does. No other family reads
//! their state in the same hour, so the hourly walk skips them.

use super::{Block, Branch, Load, Observed, Own, census};
use crate::products::parts_race_prepared::{
    self,
    block::{BlockExtract, BlockFacts},
};
use crate::{
    prepared::{
        frame_changes::{FrameChangeTracker, FrameChanges},
        prepass::{self, Extract, Folded},
    },
    products::{
        air_force, convoy_health, country_history::HistoryScan, equipment_losses, fuel_branches,
        lend_lease_balance, market, navy, politics_state, production_catalogue,
        parts_race_prepared::Family as _, party_history, stability_parts, territory, trade_flows,
    },
};
use dashboard_base::prepared::timing::FamilyTimings;
use dashboard_base::products::resources::ResourceNaming;
use replay::container::Replay;
use replay_data::{format::FormatVersion, rows::TableId};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    any::Any,
    collections::BTreeMap,
    io,
    marker::PhantomData,
    sync::{LazyLock, atomic::AtomicBool},
    time::Duration,
};

/// Measured timings of an observer's parts, by timing name.
type Timings = Option<BTreeMap<&'static str, FamilyTimings>>;

#[test]
fn equipment_loss_preparation_reads_the_division_census_without_division_rows() {
    let tables = <equipment_losses::prepared::Prepared as Sampled>::tables();
    assert!(tables.contains(&TableId::DivisionCensus));
    assert!(!tables.contains(&TableId::Divisions));
}

/// A sampled family's branch state, observing as hourly preparation does.
trait Sampled: Serialize + DeserializeOwned + Send + 'static {
    /// Its pre-pass family: its name is the hourly walk's.
    fn family() -> &'static prepass::Family;
    /// The timing row of its whole preparation.
    const TIMING: &'static str;
    /// Each hour its own thread observes, counted in tests.
    #[cfg(test)]
    const OBSERVED: &'static str;
    /// Whether hourly preparation resumes it at each block from the frame its
    /// stream last accepted.
    const RESUMES: bool = false;
    /// Whether its blocks fold from a pre-pass worker's facts
    /// ([`Self::fold_block`]) rather than being observed hour by hour on the
    /// shared block read.
    const FOLDS: bool = false;

    /// Every table its observation (and resumption) reads.
    fn tables() -> &'static [TableId];

    /// A new state, as hourly preparation's new branch state holds it when
    /// it loads the branch at `block`.
    fn new(block: &Block<'_>) -> Self;

    /// Before the block's hours: what hourly preparation does for it at
    /// every block. An error stops preparation, as the walk's does.
    fn configure(&mut self, block: &Block<'_>) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("fold binds", None);
        let _ = block;
        Ok(())
    }

    /// Resume from `frame`, the last frame of the recording's block before,
    /// on the same branch. An error stops preparation, as the walk's does.
    fn resume(&mut self, frame: &DecodedFrame, replay: &Replay) -> io::Result<()> {
        let _ = (frame, replay);
        Ok(())
    }

    /// Fold a block from a pre-pass worker's `facts`, after
    /// [`Self::configure`] and [`Self::resume`]: how it folded, the last hour
    /// reached and the block's last frame. Only a family that folds.
    fn fold_block(
        &mut self,
        facts: Box<dyn Any + Send>,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>, Option<DecodedFrame>)> {
        let _ = (facts, block, observed, check);
        Err(io::Error::other(
            "This family observes its blocks hour by hour",
        ))
    }

    /// One hour, as hourly preparation observes it.
    fn observe(
        &mut self,
        frame: &DecodedFrame,
        block: &Block<'_>,
        changes: &FrameChanges,
        running: &AtomicBool,
        timings: &mut Timings,
    ) -> io::Result<()>;

    /// After a block it went on through, as hourly preparation ends it.
    fn finish_block(&mut self, block: &Block<'_>) {
        let _ = block;
    }

    /// Bytes held for its next flush.
    fn bytes(&self) -> usize;

    /// Its flush, where hourly preparation flushes it.
    fn flush(
        &mut self,
        writer: &mut store::Writer,
        format: FormatVersion,
        branch: u64,
        cancelled: &AtomicBool,
        timings: &mut Timings,
    ) -> io::Result<()>;

    /// Pages hourly preparation writes after its flush: the outer error stops
    /// preparation, the inner one refuses the branch.
    fn save_pages(&self, writer: &mut store::Writer, branch: u64) -> io::Result<io::Result<()>> {
        let _ = (writer, branch);
        Ok(Ok(()))
    }

    /// The state its last commit saved, if any.
    fn load(load: &Load<'_>, branch: u64) -> io::Result<Option<Self>> {
        census::read(load, &census::continuation(Self::family().name, branch))
    }

    /// Save its state where it loads it from.
    fn save_state(&self, writer: &mut store::Writer, branch: u64) -> io::Result<()> {
        census::save(
            writer,
            &census::continuation(Self::family().name, branch),
            self,
        )
    }
}

/// The tables `parts` read, once each, sorted.
fn union(parts: &[&[TableId]]) -> Vec<TableId> {
    let mut tables: Vec<TableId> = parts.iter().flat_map(|part| part.iter().copied()).collect();
    tables.push(TableId::Campaign);
    tables.sort_unstable();
    tables.dedup();
    tables
}

// --- Air force and navy size ----------------------------------------------

impl Sampled for air_force::prepared::Prepared {
    fn family() -> &'static prepass::Family {
        &AIR_FAMILY
    }

    const TIMING: &'static str = "independent.air";
    #[cfg(test)]
    const OBSERVED: &'static str = "air.observed";
    const RESUMES: bool = true;
    const FOLDS: bool = true;

    fn fold_block(
        &mut self,
        facts: Box<dyn Any + Send>,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>, Option<DecodedFrame>)> {
        fold_forces(
            self,
            Self::tables(),
            facts,
            block,
            observed,
            check,
            #[cfg(test)]
            (Self::OBSERVED, AIR_APPLIED),
        )
    }

    /// The census's tables and the variant catalogue naming its equipment.
    fn tables() -> &'static [TableId] {
        static TABLES: LazyLock<Vec<TableId>> = LazyLock::new(|| {
            union(&[
                air_force::prepared::Air::SAMPLE_TABLES,
                &dashboard_base::products::equipment::VariantCatalogue::TABLES,
            ])
        });
        &TABLES
    }

    fn new(_: &Block<'_>) -> Self {
        Self::default()
    }

    fn configure(&mut self, block: &Block<'_>) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("fold binds", None);
        Self::configure(self, block.source, block.context);
        Ok(())
    }

    fn resume(&mut self, frame: &DecodedFrame, replay: &Replay) -> io::Result<()> {
        // Cancellation is checked between blocks.
        Self::resume(self, frame, replay, &AtomicBool::new(false))
    }

    fn observe(
        &mut self,
        frame: &DecodedFrame,
        block: &Block<'_>,
        changes: &FrameChanges,
        running: &AtomicBool,
        _: &mut Timings,
    ) -> io::Result<()> {
        Self::observe(
            self,
            frame,
            block.replay,
            block.source,
            block.context,
            running,
            changes,
        )
    }

    fn bytes(&self) -> usize {
        Self::bytes(self)
    }

    fn flush(
        &mut self,
        writer: &mut store::Writer,
        _: FormatVersion,
        branch: u64,
        cancelled: &AtomicBool,
        _: &mut Timings,
    ) -> io::Result<()> {
        Self::flush(self, writer, branch, cancelled)
    }
}

impl Sampled for navy::prepared::Prepared {
    fn family() -> &'static prepass::Family {
        &NAVY_FAMILY
    }

    const TIMING: &'static str = "independent.navy";
    #[cfg(test)]
    const OBSERVED: &'static str = "navy.observed";
    const RESUMES: bool = true;
    const FOLDS: bool = true;

    fn fold_block(
        &mut self,
        facts: Box<dyn Any + Send>,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>, Option<DecodedFrame>)> {
        fold_forces(
            self,
            Self::tables(),
            facts,
            block,
            observed,
            check,
            #[cfg(test)]
            (Self::OBSERVED, NAVY_APPLIED),
        )
    }

    fn tables() -> &'static [TableId] {
        static TABLES: LazyLock<Vec<TableId>> =
            LazyLock::new(|| union(&[navy::prepared::Navy::SAMPLE_TABLES]));
        &TABLES
    }

    fn new(_: &Block<'_>) -> Self {
        Self::default()
    }

    fn configure(&mut self, block: &Block<'_>) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("fold binds", None);
        Self::configure(self, block.source, block.context);
        Ok(())
    }

    fn resume(&mut self, frame: &DecodedFrame, replay: &Replay) -> io::Result<()> {
        // Cancellation is checked between blocks.
        Self::resume(self, frame, replay, &AtomicBool::new(false))
    }

    fn observe(
        &mut self,
        frame: &DecodedFrame,
        block: &Block<'_>,
        changes: &FrameChanges,
        running: &AtomicBool,
        _: &mut Timings,
    ) -> io::Result<()> {
        Self::observe(
            self,
            frame,
            block.replay,
            block.source,
            block.context,
            running,
            changes,
        )
    }

    fn bytes(&self) -> usize {
        Self::bytes(self)
    }

    fn flush(
        &mut self,
        writer: &mut store::Writer,
        _: FormatVersion,
        branch: u64,
        cancelled: &AtomicBool,
        _: &mut Timings,
    ) -> io::Result<()> {
        Self::flush(self, writer, branch, cancelled)
    }
}

// --- Stability's parts ----------------------------------------------------

impl Sampled for stability_parts::Prepared {
    fn family() -> &'static prepass::Family {
        &PARTS_FAMILY
    }

    const TIMING: &'static str = "independent.parts";
    #[cfg(test)]
    const OBSERVED: &'static str = "parts.observed";

    /// The stability inputs, lines and slice reads, the command power, the
    /// modifier, character and token tables naming parts, and the tables a
    /// country's naming reads (civil wars among them).
    fn tables() -> &'static [TableId] {
        static TABLES: LazyLock<Vec<TableId>> = LazyLock::new(|| {
            union(&[
                &[
                    TableId::Countries,
                    TableId::CountryLinks,
                    TableId::CountryStabilityInputs,
                    TableId::CountryStabilityLines,
                    TableId::CountryCommandPower,
                    TableId::ModifierDefinitions,
                    TableId::SliceReads,
                    TableId::Characters,
                    TableId::LexicalTokens,
                    TableId::CountryAppearance,
                    TableId::ReservedCountryNaming,
                    TableId::CountryCivilWars,
                    TableId::CountryWars,
                    TableId::WarRelations,
                    TableId::CountryCounterInputs,
                ],
                &installed_content_country_names::country_names::TABLES,
            ])
        });
        &TABLES
    }

    fn new(_: &Block<'_>) -> Self {
        Self::default()
    }

    /// The state its last commit saved, with the naming views it saved by
    /// hour read again from the recording.
    fn load(load: &Load<'_>, branch: u64) -> io::Result<Option<Self>> {
        let Some(mut state) =
            census::read::<Self>(load, &census::continuation(Self::family().name, branch))?
        else {
            return Ok(None);
        };
        state.restore(load.replay, branch)?;
        Ok(Some(state))
    }

    fn observe(
        &mut self,
        frame: &DecodedFrame,
        _: &Block<'_>,
        changes: &FrameChanges,
        _: &AtomicBool,
        _: &mut Timings,
    ) -> io::Result<()> {
        self.observe_changes(frame, changes)
    }

    fn bytes(&self) -> usize {
        Self::bytes(self)
    }

    fn flush(
        &mut self,
        writer: &mut store::Writer,
        _: FormatVersion,
        branch: u64,
        cancelled: &AtomicBool,
        _: &mut Timings,
    ) -> io::Result<()> {
        Self::flush(self, writer, branch, cancelled)
    }
}

// --- Equipment losses ------------------------------------------------------

impl Sampled for equipment_losses::prepared::Prepared {
    fn family() -> &'static prepass::Family {
        &LOSSES_FAMILY
    }

    const TIMING: &'static str = "independent.equipment_losses";
    #[cfg(test)]
    const OBSERVED: &'static str = "equipment_losses.observed";

    /// The destruction and combat-log tables, lend-lease and sunk convoys and
    /// ships, the division census, players and features, the
    /// slice reads, and the variant catalogue naming the equipment.
    fn tables() -> &'static [TableId] {
        static TABLES: LazyLock<Vec<TableId>> = LazyLock::new(|| {
            union(&[
                &[
                    TableId::CombatLogEquipment,
                    TableId::CombatLogState,
                    TableId::DivisionCensus,
                    TableId::EquipmentDestruction,
                    TableId::LendLeaseEquipment,
                    TableId::Players,
                    TableId::SimulationFeatures,
                    TableId::SliceReads,
                    TableId::SunkConvoys,
                    TableId::SunkShips,
                ],
                &dashboard_base::products::equipment::VariantCatalogue::TABLES,
            ])
        });
        &TABLES
    }

    /// Hourly preparation makes it from the first hour of the block it loads
    /// the branch at.
    fn new(block: &Block<'_>) -> Self {
        Self::new(block.replay, block.index.branch, block.index.first_hour)
    }

    fn observe(
        &mut self,
        frame: &DecodedFrame,
        block: &Block<'_>,
        changes: &FrameChanges,
        running: &AtomicBool,
        _: &mut Timings,
    ) -> io::Result<()> {
        self.observe_changes(frame, block.replay, running, changes)
    }

    /// Through the block's last hour, or the branch's at its last block.
    fn finish_block(&mut self, block: &Block<'_>) {
        let replay = block.replay;
        let branch = block.index.branch;
        let last_block = replay
            .index
            .iter()
            .rposition(|index| index.branch == branch);
        let at_last = last_block.is_some_and(|last| std::ptr::eq(&replay.index[last], block.index));
        let last = replay
            .branches
            .iter()
            .find(|start| start.id == branch)
            .filter(|_| at_last)
            .map_or(block.index.last_hour, |start| {
                crate::metadata::branch_bounds(replay, start).1
            });
        Self::finish_block(self, last);
    }

    fn bytes(&self) -> usize {
        Self::bytes(self)
    }

    fn flush(
        &mut self,
        writer: &mut store::Writer,
        _: FormatVersion,
        branch: u64,
        cancelled: &AtomicBool,
        _: &mut Timings,
    ) -> io::Result<()> {
        Self::flush(self, writer, branch, cancelled)
    }
}

// --- Party history ---------------------------------------------------------

/// The party history's saved page, which is its state.
fn party_section(branch: u64) -> String {
    format!("party-history-{branch}")
}

impl Sampled for party_history::prepared::Prepared {
    fn family() -> &'static prepass::Family {
        &PARTIES_FAMILY
    }

    const TIMING: &'static str = "independent.parties";
    #[cfg(test)]
    const OBSERVED: &'static str = "parties.observed";

    /// Parties and politics, the characters and portraits of their leaders,
    /// the provinces of capitals, and the tables a country's naming reads.
    fn tables() -> &'static [TableId] {
        static TABLES: LazyLock<Vec<TableId>> = LazyLock::new(|| {
            union(&[
                &[
                    TableId::Countries,
                    TableId::CountryLinks,
                    TableId::PoliticalParties,
                    TableId::Politics,
                    TableId::Characters,
                    TableId::CharacterPortraits,
                    TableId::CountryAppearance,
                    TableId::LexicalTokens,
                    TableId::Provinces,
                    TableId::ReservedCountryNaming,
                ],
                &installed_content_country_names::country_names::TABLES,
            ])
        });
        &TABLES
    }

    fn new(_: &Block<'_>) -> Self {
        Self::default()
    }

    fn observe(
        &mut self,
        frame: &DecodedFrame,
        _: &Block<'_>,
        changes: &FrameChanges,
        _: &AtomicBool,
        _: &mut Timings,
    ) -> io::Result<()> {
        self.observe_changes(frame, changes)
    }

    fn bytes(&self) -> usize {
        Self::bytes(self)
    }

    fn flush(
        &mut self,
        writer: &mut store::Writer,
        _: FormatVersion,
        branch: u64,
        cancelled: &AtomicBool,
        _: &mut Timings,
    ) -> io::Result<()> {
        Self::flush(self, writer, branch, cancelled)
    }

    /// The saved page hourly preparation writes after the party pages.
    fn save_pages(&self, writer: &mut store::Writer, branch: u64) -> io::Result<io::Result<()>> {
        let bytes = self.saved_bytes()?;
        Ok(writer.write_at(&party_section(branch), 0, bytes.as_slice()))
    }

    fn load(load: &Load<'_>, branch: u64) -> io::Result<Option<Self>> {
        load.read(&party_section(branch))?
            .map(|file| Self::read(io::BufReader::new(file)))
            .transpose()
    }

    /// Its saved page is written after its flush ([`Self::save_pages`]).
    fn save_state(&self, _: &mut store::Writer, _: u64) -> io::Result<()> {
        Ok(())
    }
}

// --- Economy ---------------------------------------------------------------

/// The `Sampled` impl of an economy family: each observes every accepted hour
/// and rebuilds only where its watched tables changed or after a gap.
macro_rules! economy_family {
    ($type:ty, $family:ident, $timing:literal, $observed:literal, $tables:expr,
     |$state:ident, $frame:ident, $block:ident, $changes:ident| $observe:expr) => {
        impl Sampled for $type {
            fn family() -> &'static prepass::Family {
                &$family
            }

            const TIMING: &'static str = $timing;
            #[cfg(test)]
            const OBSERVED: &'static str = $observed;

            fn tables() -> &'static [TableId] {
                static TABLES: LazyLock<Vec<TableId>> = LazyLock::new(|| union(&[$tables]));
                &TABLES
            }

            fn new(_: &Block<'_>) -> Self {
                Self::default()
            }

            fn observe(
                &mut self,
                $frame: &DecodedFrame,
                $block: &Block<'_>,
                $changes: &FrameChanges,
                _: &AtomicBool,
                _: &mut Timings,
            ) -> io::Result<()> {
                let $state = self;
                $observe
            }

            fn bytes(&self) -> usize {
                Self::bytes(self)
            }

            fn flush(
                &mut self,
                writer: &mut store::Writer,
                _: FormatVersion,
                branch: u64,
                cancelled: &AtomicBool,
                _: &mut Timings,
            ) -> io::Result<()> {
                Self::flush(self, writer, branch, cancelled)
            }
        }
    };
}

economy_family!(
    fuel_branches::Prepared,
    FUEL_BRANCHES_FAMILY,
    "independent.fuel_branches",
    "fuel_branches.observed",
    fuel_branches::TABLES,
    |state, frame, _block, changes| state.observe_changes(frame, changes)
);
economy_family!(
    market::Prepared,
    MARKET_FAMILY,
    "independent.market",
    "market.observed",
    market::tables(),
    |state, frame, block, changes| state.observe_changes(block.replay.format, frame, changes)
);
economy_family!(
    lend_lease_balance::Prepared,
    LEND_LEASE_BALANCE_FAMILY,
    "independent.lend_lease_balance",
    "lend_lease_balance.observed",
    lend_lease_balance::TABLES,
    |state, frame, _block, changes| state.observe_changes(frame, changes)
);
economy_family!(
    production_catalogue::Prepared,
    PRODUCTION_CATALOGUE_FAMILY,
    "independent.production_catalogue",
    "production_catalogue.observed",
    production_catalogue::tables(),
    |state, frame, block, changes| state.observe_changes(block.replay.format, frame, changes)
);
economy_family!(
    convoy_health::Prepared,
    CONVOYS_FAMILY,
    "independent.convoys",
    "convoys.observed",
    convoy_health::TABLES,
    |state, frame, _block, changes| state.observe_changes(frame, changes)
);

// --- Political state -------------------------------------------------------

/// Election schedules, governments in exile and balance-of-power membership.
impl Sampled for politics_state::history::Prepared {
    fn family() -> &'static prepass::Family {
        &POLITICS_FAMILY
    }

    const TIMING: &'static str = "independent.politics_state";
    #[cfg(test)]
    const OBSERVED: &'static str = "politics_state.observed";

    /// The politics rows, sliced exile inputs and their reads, the balance
    /// records and members, and the country identities naming them.
    fn tables() -> &'static [TableId] {
        static TABLES: LazyLock<Vec<TableId>> =
            LazyLock::new(|| union(&[politics_state::history::TABLES]));
        &TABLES
    }

    fn new(_: &Block<'_>) -> Self {
        Self::default()
    }

    fn observe(
        &mut self,
        frame: &DecodedFrame,
        _: &Block<'_>,
        changes: &FrameChanges,
        _: &AtomicBool,
        _: &mut Timings,
    ) -> io::Result<()> {
        self.observe_changes(frame, changes)
    }

    fn bytes(&self) -> usize {
        Self::bytes(self)
    }

    fn flush(
        &mut self,
        writer: &mut store::Writer,
        _: FormatVersion,
        branch: u64,
        cancelled: &AtomicBool,
        _: &mut Timings,
    ) -> io::Result<()> {
        Self::flush(self, writer, branch, cancelled)
    }
}

// --- Trade flows -----------------------------------------------------------

/// Trade flows and the naming of their resources, which hourly preparation
/// reads from the source at its flush and the family takes from the source
/// at each block it observes (the same content catalogue and resource cache).
/// Saved as the flows alone, as hourly preparation saves them.
#[derive(Default, Serialize, Deserialize)]
#[serde(transparent)]
pub(in crate::prepared::pipeline) struct Trade {
    prepared: trade_flows::prepared::Prepared,
    #[serde(skip)]
    naming: Option<ResourceNaming>,
}

impl Sampled for Trade {
    fn family() -> &'static prepass::Family {
        &TRADE_FAMILY
    }

    const TIMING: &'static str = "independent.trade";
    #[cfg(test)]
    const OBSERVED: &'static str = "trade.observed";

    /// The resource exchanges, the resources' definitions and tokens, and
    /// the countries the deals join.
    fn tables() -> &'static [TableId] {
        static TABLES: LazyLock<Vec<TableId>> = LazyLock::new(|| {
            union(&[&[
                TableId::Countries,
                TableId::CountryLinks,
                TableId::LexicalTokens,
                TableId::ResourceDefinitions,
                TableId::ResourceExchanges,
            ]])
        });
        &TABLES
    }

    fn new(_: &Block<'_>) -> Self {
        Self::default()
    }

    fn configure(&mut self, block: &Block<'_>) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("fold binds", None);
        self.naming = Some(ResourceNaming::stored(block.source, block.context));
        Ok(())
    }

    fn observe(
        &mut self,
        frame: &DecodedFrame,
        _: &Block<'_>,
        changes: &FrameChanges,
        _: &AtomicBool,
        _: &mut Timings,
    ) -> io::Result<()> {
        self.prepared.observe_changed(frame, changes)
    }

    fn bytes(&self) -> usize {
        self.prepared.bytes()
    }

    fn flush(
        &mut self,
        writer: &mut store::Writer,
        _: FormatVersion,
        branch: u64,
        cancelled: &AtomicBool,
        _: &mut Timings,
    ) -> io::Result<()> {
        let naming = self
            .naming
            .as_ref()
            .ok_or_else(|| io::Error::other("Trade flows flushed before observing a block"))?;
        self.prepared.flush_named(writer, naming, branch, cancelled)
    }
}

// --- Territory ---------------------------------------------------------------

/// Territory with its own read of the identity history, which its owned-state
/// observation drives in the same hour, as the hourly walk's kept read of the
/// identities did. The read is saved where the walk kept it
/// (`territory-identities-<branch>`), written at territory's flush before
/// territory's pages, as the walk's flush wrote it before them.
#[derive(Serialize, Deserialize)]
pub(in crate::prepared::pipeline) struct Territory {
    prepared: territory::prepared::Prepared,
    identities: HistoryScan,
}

/// Territory's read of the identity history of one branch.
fn territory_identities(branch: u64) -> String {
    format!("territory-identities-{branch}")
}

impl Sampled for Territory {
    fn family() -> &'static prepass::Family {
        &TERRITORY_FAMILY
    }

    const TIMING: &'static str = "independent.territory";
    #[cfg(test)]
    const OBSERVED: &'static str = "territory.observed";

    /// States and provinces with their owners and controllers, the countries
    /// and links they are joined through, the counters and experience gains
    /// the identity history reads, and the tables a country's naming reads.
    fn tables() -> &'static [TableId] {
        static TABLES: LazyLock<Vec<TableId>> = LazyLock::new(|| {
            union(&[
                territory::TABLES,
                &[
                    TableId::CountryCounterInputs,
                    TableId::CountryExperienceGains,
                ],
                &installed_content_country_names::country_names::TABLES,
            ])
        });
        &TABLES
    }

    fn new(_: &Block<'_>) -> Self {
        Self {
            prepared: territory::prepared::Prepared::default(),
            identities: HistoryScan::new(false, &[]),
        }
    }

    /// Its area data and breakdown content, bound at every block as the walk
    /// binds them.
    fn configure(&mut self, block: &Block<'_>) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("fold binds", None);
        // Cancellation is checked between blocks.
        self.prepared
            .configure(block.source, block.context, &AtomicBool::new(false))
    }

    /// The native owned states drive identity history separately from occupied
    /// territory. One paired-holder observation publishes every controlled,
    /// owned and occupied channel, as the walk observes them in the same hour.
    fn observe(
        &mut self,
        frame: &DecodedFrame,
        block: &Block<'_>,
        changes: &FrameChanges,
        running: &AtomicBool,
        _: &mut Timings,
    ) -> io::Result<()> {
        let owned = self
            .prepared
            .observe_owned_states(frame, &mut self.identities, Some(changes));
        #[cfg(test)]
        let owned = super::super::lanes::inject("identities", frame.hour, owned);
        owned.map_err(super::event_feed::IdentitiesRefusal::wrap)?;
        self.prepared
            .observe(frame, block.replay.format, running, changes)
    }

    fn bytes(&self) -> usize {
        self.prepared.bytes()
    }

    /// The identity history's read, then territory's pages.
    fn flush(
        &mut self,
        writer: &mut store::Writer,
        _: FormatVersion,
        branch: u64,
        cancelled: &AtomicBool,
        _: &mut Timings,
    ) -> io::Result<()> {
        self.identities.seal();
        writer
            .write_at(
                &territory_identities(branch),
                0,
                serde_json::to_vec(&self.identities)?.as_slice(),
            )
            .map_err(super::event_feed::IdentitiesRefusal::wrap)?;
        self.prepared.flush(writer, branch, cancelled)
    }

    fn load(load: &Load<'_>, branch: u64) -> io::Result<Option<Self>> {
        let Some(prepared) = census::read(load, &census::continuation("territory", branch))? else {
            return Ok(None);
        };
        let identities = match load.read(&territory_identities(branch))? {
            Some(file) => serde_json::from_reader(io::BufReader::new(file))?,
            None => HistoryScan::new(false, &[]),
        };
        Ok(Some(Self {
            prepared,
            identities,
        }))
    }

    /// Territory's state, which hourly preparation keeps in its branch
    /// state; the identity history's read is written at the flush.
    fn save_state(&self, writer: &mut store::Writer, branch: u64) -> io::Result<()> {
        census::save(
            writer,
            &census::continuation("territory", branch),
            &self.prepared,
        )
    }
}

// --- Families --------------------------------------------------------------

/// A sampled family's pre-pass family: its extract keeps nothing, as the
/// family's own thread reads each block from the shared read, so it has no
/// pool lane ([`Own::laned`]) and the extract is never run.
macro_rules! family {
    ($static:ident, $name:literal) => {
        static $static: prepass::Family = prepass::Family {
            name: $name,
            extract: concat!($name, ".extract"),
            sequential: concat!($name, ".sequential"),
            fallback: concat!($name, ".fallback"),
            tables: &[TableId::Campaign],
            fields: None,
            start: read_start,
        };
    };
}

/// Air force size: its extract steps the census through the block on a
/// pre-pass worker, reading the census's tables.
static AIR_FAMILY: prepass::Family = prepass::Family {
    name: "air",
    extract: "air.extract",
    sequential: "air.sequential",
    fallback: "air.fallback",
    tables: air_force::prepared::Air::SAMPLE_TABLES,
    fields: None,
    start: air_start,
};

fn air_start(_: FormatVersion) -> Box<dyn Extract> {
    BlockExtract::<air_force::prepared::Air>::start()
}

/// Navy size: its extract steps the census through the block on a pre-pass
/// worker, reading the census's tables.
static NAVY_FAMILY: prepass::Family = prepass::Family {
    name: "navy",
    extract: "navy.extract",
    sequential: "navy.sequential",
    fallback: "navy.fallback",
    tables: navy::prepared::Navy::SAMPLE_TABLES,
    fields: None,
    start: navy_start,
};

fn navy_start(_: FormatVersion) -> Box<dyn Extract> {
    BlockExtract::<navy::prepared::Navy>::start()
}

#[cfg(test)]
thread_local! {
    static FALLBACK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// This thread's forced-fold fallback setting, to carry to family workers.
#[cfg(test)]
pub(super) fn test_fallback() -> bool {
    FALLBACK.with(std::cell::Cell::get)
}

/// Install a caller's fallback setting until the returned guard is dropped.
#[cfg(test)]
pub(super) fn install_test_fallback(fallback: bool) -> impl Drop {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            FALLBACK.with(|value| value.set(self.0));
        }
    }
    Reset(FALLBACK.with(|value| value.replace(fallback)))
}

/// Run `run` with every force fold observing the rest of its block hour by
/// hour, as where the carried scan does not continue as the worker's.
#[cfg(test)]
pub(in crate::prepared::pipeline) fn with_test_fallback<T>(run: impl FnOnce() -> T) -> T {
    let _reset = install_test_fallback(true);
    run()
}

/// Each later hour of a block air force size published from a worker's
/// rows, counted in tests.
#[cfg(test)]
pub(in crate::prepared::pipeline) const AIR_APPLIED: &str = "air.applied";

/// Each later hour of a block navy size published from a worker's rows,
/// counted in tests.
#[cfg(test)]
pub(in crate::prepared::pipeline) const NAVY_APPLIED: &str = "navy.applied";

/// Fold a force census's block from a worker's facts: observe the block's
/// first hour on the carried state and, where the worker took its tail at a
/// later hour, the hours through the tail's hour by hour on the block read
/// alone with `tables`; then publish each later hour's rows when the carried
/// scan continues as the worker's did, or observe the rest of the block hour
/// by hour when it does not. How it folded, the last hour reached and the
/// block's last frame.
fn fold_forces<F: parts_race_prepared::Family + 'static>(
    prepared: &mut parts_race_prepared::Prepared<F>,
    tables: &[TableId],
    facts: Box<dyn Any + Send>,
    block: &Block<'_>,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
    #[cfg(test)] notes: (&'static str, &'static str),
) -> io::Result<(Folded, Option<i32>, Option<DecodedFrame>)>
where
    F::Raw: Send,
    F::Scanned: Send,
    F::Part: Send,
{
    let _cpu = ::replay_frame::cpu::scope("fold driver", None);
    let mut facts = {
        let _cpu = ::replay_frame::cpu::scope("fold facts", None);
        *facts
            .downcast::<BlockFacts<F>>()
            .map_err(|_| io::Error::other("Pre-pass facts of another family"))?
    };
    // Cancellation is checked between hours; an hour's observation runs
    // whole, so its refusals are the walk's.
    let running = AtomicBool::new(false);
    let (first, result) = {
        let _cpu = ::replay_frame::cpu::scope("fold observe", None);
        prepared.fold_first(&facts, block.replay, block.source, block.context, &running)
    };
    #[cfg(test)]
    prepass::note(notes.0);
    if !observed(first, result)? {
        return Ok((Folded::Fast, Some(first), facts.last));
    }
    let tail = facts.tail_hour();
    if tail == Some(first) && forces_continue(prepared, &mut facts) {
        return fold_forces_rest(
            prepared,
            facts,
            first,
            block,
            observed,
            #[cfg(test)]
            notes.1,
        );
    }
    let format = block.replay.format;
    let mut tracker = FrameChangeTracker::new();
    let mut reached = Some(first);
    let mut last = None;
    let mut stopped = false;
    let mut failure = None;
    // The carried scan continued as the worker's from its tail's hour: the
    // read stops there.
    let mut handed = false;
    let read = block.replay.read_block_recorded_alone(
        block.index,
        tables,
        |raw, frame| {
            let _cpu = ::replay_frame::cpu::scope("fold driver", None);
            let changes = tracker.observe_updates(frame, raw, format);
            if frame.hour == block.index.last_hour {
                last = Some(frame.clone());
            }
            if stopped || frame.hour <= first {
                return Ok(());
            }
            reached = Some(frame.hour);
            #[cfg(test)]
            prepass::note(notes.0);
            let result = {
                let _cpu = ::replay_frame::cpu::scope("fold observe", None);
                prepared.observe(
                    frame,
                    block.replay,
                    block.source,
                    block.context,
                    &running,
                    &changes,
                )
            };
            match observed(frame.hour, result) {
                Ok(live) => {
                    stopped = !live;
                    if live && tail == Some(frame.hour) && forces_continue(prepared, &mut facts) {
                        handed = true;
                        return Err(io::Error::other("Force fold continues from its tail"));
                    }
                    Ok(())
                }
                Err(error) => {
                    failure = Some(error);
                    Err(io::Error::other("Force observation stopped"))
                }
            }
        },
        check,
    );
    if let Some(error) = failure {
        return Err(error);
    }
    if handed {
        let hour = tail.unwrap_or(first);
        return fold_forces_rest(
            prepared,
            facts,
            hour,
            block,
            observed,
            #[cfg(test)]
            notes.1,
        );
    }
    read?;
    Ok((Folded::Fallback, reached, last))
}

/// Whether the carried scan, having observed the block through the worker's
/// tail's hour, continues as the worker's did.
fn forces_continue<F: parts_race_prepared::Family + 'static>(
    prepared: &parts_race_prepared::Prepared<F>,
    facts: &mut BlockFacts<F>,
) -> bool
where
    F::Raw: Send,
    F::Scanned: Send,
    F::Part: Send,
{
    let continues = prepared.fold_continues(facts);
    #[cfg(test)]
    let continues = continues && !FALLBACK.with(std::cell::Cell::get);
    continues
}

/// Publish every hour's rows after the worker's tail's, `hour`, and carry
/// the scan to the block's end: how it folded, the last hour reached and the
/// block's last frame.
fn fold_forces_rest<F: parts_race_prepared::Family + 'static>(
    prepared: &mut parts_race_prepared::Prepared<F>,
    facts: BlockFacts<F>,
    hour: i32,
    block: &Block<'_>,
    observed: &mut Observed<'_>,
    #[cfg(test)] applied: &'static str,
) -> io::Result<(Folded, Option<i32>, Option<DecodedFrame>)>
where
    F::Raw: Send,
    F::Scanned: Send,
    F::Part: Send,
{
    let _cpu = ::replay_frame::cpu::scope("fold driver", None);
    let last = {
        let _cpu = ::replay_frame::cpu::scope("fold frame", None);
        facts.last.clone()
    };
    #[cfg(test)]
    for _ in 0..facts.later_hours() {
        prepass::note(applied);
    }
    let threads = super::super::lanes::parallelism().clamp(1, 8);
    let reached = {
        let _cpu = ::replay_frame::cpu::scope("fold observe", None);
        prepared.fold_rest(facts, block.source, block.context, threads, observed)?
    };
    Ok((Folded::Fast, reached.or(Some(hour)), last))
}
family!(PARTS_FAMILY, "parts");
family!(LOSSES_FAMILY, "equipment_losses");
family!(PARTIES_FAMILY, "parties");
family!(FUEL_BRANCHES_FAMILY, "fuel_branches");
family!(MARKET_FAMILY, "market");
family!(LEND_LEASE_BALANCE_FAMILY, "lend_lease_balance");
family!(PRODUCTION_CATALOGUE_FAMILY, "production_catalogue");
family!(CONVOYS_FAMILY, "convoys");
family!(POLITICS_FAMILY, "politics_state");
family!(TRADE_FAMILY, "trade");
family!(TERRITORY_FAMILY, "territory");

/// A block a pre-pass worker read for a sampled family.
struct BlockRead;

/// The sampled families' block extract: it reads the block's campaign
/// identity and keeps nothing.
struct ReadExtract;

impl Extract for ReadExtract {
    fn observe(&mut self, _: &Replay, _: &DecodedFrame, _: &FrameChanges) -> io::Result<()> {
        Ok(())
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        Ok(Box::new(BlockRead))
    }
}

fn read_start(_: FormatVersion) -> Box<dyn Extract> {
    Box::new(ReadExtract)
}

/// A sampled family's registry entry.
pub(in crate::prepared::pipeline) struct Entry<S>(PhantomData<fn() -> S>);

/// Air force size.
pub(in crate::prepared::pipeline) static AIR: Entry<air_force::prepared::Prepared> =
    Entry(PhantomData);
/// Navy size.
pub(in crate::prepared::pipeline) static NAVY: Entry<navy::prepared::Prepared> = Entry(PhantomData);
/// Stability's parts.
pub(in crate::prepared::pipeline) static PARTS: Entry<stability_parts::Prepared> =
    Entry(PhantomData);
/// Equipment losses.
pub(in crate::prepared::pipeline) static EQUIPMENT_LOSSES: Entry<
    equipment_losses::prepared::Prepared,
> = Entry(PhantomData);
/// Parties and leaders.
pub(in crate::prepared::pipeline) static PARTIES: Entry<party_history::prepared::Prepared> =
    Entry(PhantomData);
/// Fuel by military branch.
pub(in crate::prepared::pipeline) static FUEL_BRANCHES: Entry<fuel_branches::Prepared> =
    Entry(PhantomData);
/// The international market and lend-lease.
pub(in crate::prepared::pipeline) static MARKET: Entry<market::Prepared> = Entry(PhantomData);
/// Lend-lease balances.
pub(in crate::prepared::pipeline) static LEND_LEASE_BALANCE: Entry<lend_lease_balance::Prepared> =
    Entry(PhantomData);
/// Licences, industrial organisations and designs.
pub(in crate::prepared::pipeline) static PRODUCTION_CATALOGUE: Entry<
    production_catalogue::Prepared,
> = Entry(PhantomData);
/// Convoy and trade efficiency.
pub(in crate::prepared::pipeline) static CONVOYS: Entry<convoy_health::Prepared> =
    Entry(PhantomData);
/// Election schedules, exiles and balances of power.
pub(in crate::prepared::pipeline) static POLITICS: Entry<politics_state::history::Prepared> =
    Entry(PhantomData);
/// Trade flows.
pub(in crate::prepared::pipeline) static TRADE: Entry<Trade> = Entry(PhantomData);
/// Territory, with its own read of the identity history.
pub(in crate::prepared::pipeline) static TERRITORY: Entry<Territory> = Entry(PhantomData);

impl<S: Sampled> Own for Entry<S> {
    fn family(&self) -> &'static prepass::Family {
        S::family()
    }

    fn timing(&self) -> &'static str {
        S::TIMING
    }

    /// A family that folds reads its blocks on pre-pass workers instead.
    fn shared_tables(&self, _: &super::Phase<'_>) -> &'static [TableId] {
        if S::FOLDS { &[] } else { S::tables() }
    }

    /// Only a family that folds keeps anything of an extract.
    fn laned(&self) -> bool {
        S::FOLDS
    }

    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        Ok(Box::new(SampledBranch {
            state: S::load(load, branch)?.map(Box::new),
            last: None,
            timings: None,
        }))
    }
}

/// One branch's sampled family.
struct SampledBranch<S> {
    /// None until a block is observed on a branch the family has not saved,
    /// where hourly preparation makes a new state.
    state: Option<Box<S>>,
    /// The recording position and last frame of the block it last observed,
    /// for a family hourly preparation resumes at each block.
    last: Option<(usize, DecodedFrame)>,
    /// Its observer's parts' timings in a measured run.
    timings: Timings,
}

/// `block`'s position in the recording.
fn position(block: &Block<'_>) -> Option<usize> {
    block
        .replay
        .index
        .iter()
        .position(|index| std::ptr::eq(index, block.index))
}

impl<S: Sampled> SampledBranch<S> {
    /// The frame hourly preparation's stream last accepted before `block`,
    /// when that is on `block`'s branch: the last frame of the recording's
    /// block before, kept from observing it or read alone again.
    fn previous(&mut self, block: &Block<'_>) -> io::Result<Option<DecodedFrame>> {
        let last = self.last.take();
        let Some(before) = position(block).and_then(|at| at.checked_sub(1)) else {
            return Ok(None);
        };
        let previous = &block.replay.index[before];
        if previous.branch != block.index.branch {
            return Ok(None);
        }
        if let Some((held, frame)) = last
            && held == before
        {
            return Ok(Some(frame));
        }
        super::shared::read_last(block.replay, previous, S::tables())
    }

    /// Observe the block hour by hour, as hourly preparation does, with the
    /// change facts of the block alone. The last hour observed.
    fn observe_hours(
        &mut self,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        let _cpu = ::replay_frame::cpu::scope("fold driver", None);
        let previous = if S::RESUMES {
            self.previous(block)?
        } else {
            None
        };
        let state = self.state.get_or_insert_with(|| Box::new(S::new(block)));
        state.configure(block)?;
        if let Some(frame) = previous {
            state.resume(&frame, block.replay)?;
        }
        let format = block.replay.format;
        let mut tracker = FrameChangeTracker::new();
        // Cancellation is checked between hours; an hour's observation runs
        // whole, so its refusals are the walk's.
        let running = AtomicBool::new(false);
        let timings = &mut self.timings;
        let mut reached = None;
        let mut stopped = false;
        let mut failure = None;
        let mut last = None;
        let read = block.read_alone(
            S::tables(),
            |raw, frame| {
                let _cpu = ::replay_frame::cpu::scope("fold driver", None);
                let changes = tracker.observe_updates(frame, raw, format);
                if S::RESUMES && frame.hour == block.index.last_hour {
                    last = Some(frame.clone());
                }
                if stopped {
                    return Ok(());
                }
                reached = Some(frame.hour);
                #[cfg(test)]
                prepass::note(S::OBSERVED);
                let result = {
                    let _cpu = ::replay_frame::cpu::scope("fold observe", None);
                    state.observe(frame, block, &changes, &running, timings)
                };
                match observed(frame.hour, result) {
                    Ok(live) => {
                        stopped = !live;
                        Ok(())
                    }
                    Err(error) => {
                        failure = Some(error);
                        Err(io::Error::other("Sampled family observation stopped"))
                    }
                }
            },
            check,
        );
        if let Some(error) = failure {
            return Err(error);
        }
        read?;
        if !stopped {
            state.finish_block(block);
        }
        if let (Some(at), Some(frame)) = (position(block), last) {
            self.last = Some((at, frame));
        }
        Ok(reached)
    }
}

impl<S: Sampled> Branch for SampledBranch<S> {
    fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        let _cpu = ::replay_frame::cpu::scope("fold driver", None);
        if S::FOLDS {
            let previous = if S::RESUMES {
                self.previous(block)?
            } else {
                None
            };
            let state = self.state.get_or_insert_with(|| Box::new(S::new(block)));
            state.configure(block)?;
            if let Some(frame) = previous {
                state.resume(&frame, block.replay)?;
            }
            let (folded, reached, last) = state.fold_block(facts, block, observed, check)?;
            if let (Some(at), Some(frame)) = (position(block), last) {
                self.last = Some((at, frame));
            }
            return Ok((folded, reached));
        }
        facts
            .downcast::<BlockRead>()
            .map_err(|_| io::Error::other("Pre-pass facts of another family"))?;
        let reached = self.observe_hours(block, observed, check)?;
        Ok((Folded::Fast, reached))
    }

    fn observe_block(
        &mut self,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        let _cpu = ::replay_frame::cpu::scope("fold driver", None);
        self.observe_hours(block, observed, check)
    }

    fn pending(&self) -> usize {
        self.state.as_ref().map_or(0, |state| state.bytes())
    }

    fn measure(&mut self) {
        self.timings = Some(BTreeMap::new());
    }

    fn parts(&mut self) -> Vec<(&'static str, Duration, u64)> {
        self.timings
            .as_mut()
            .map(|timings| {
                std::mem::take(timings)
                    .into_iter()
                    .map(|(name, took)| (name, took.observe + took.flush, took.calls))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn flush(
        &mut self,
        writer: &mut store::Writer,
        replay: &Replay,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let Some(state) = self.state.as_mut() else {
            return Ok(());
        };
        state.flush(writer, replay.format, branch, cancelled, &mut self.timings)
    }

    fn save(&mut self, writer: &mut store::Writer, branch: u64) -> io::Result<io::Result<()>> {
        match self.state.as_ref() {
            Some(state) => state.save_pages(writer, branch),
            None => Ok(Ok(())),
        }
    }

    fn save_state(&self, writer: &mut store::Writer, branch: u64) -> io::Result<()> {
        match self.state.as_ref() {
            Some(state) => state.save_state(writer, branch),
            None => Ok(()),
        }
    }
}
