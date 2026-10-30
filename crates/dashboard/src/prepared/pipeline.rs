//! One accepted selected-frame walk, with domain state and cursor published together.

use super::driver::{plan, registry};
use super::frame_changes::{FrameChangeTracker, FrameChanges};
use super::prepass::{self, Folded, Outcome, PrePass};
use super::{Section, State};
use crate::prepared_focus::{FocusAccumulator, FocusEvaluator, FocusIndex};
use crate::prepared_measure::MeasureAccumulator;
use crate::prepared_production::{ProductionAccumulator, ProductionNaming};
use crate::{
    Source,
    metadata::check_cancel,
    products::{ContentOptions, LoadContext},
};
use dashboard_base::prepared::timing::{FamilyTimings, family_time, finish_timing, start_timing};
use replay::{
    container::BlockIndex, container::Replay, container::ReplayCursor, container::ReplayStream,
};
use replay_data::identity::IdentityCodec;
use replay_data::rows::TableId;
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{self, Read},
    num::NonZeroU32,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

pub use independent::economy::FuelReference;

const CHECKPOINT: &str = "preparation-checkpoint-v1";
#[cfg(test)]
pub(crate) use super::coverage::publish_coverage;
pub use super::coverage::{Coverage, coverage};
pub(crate) use super::coverage::{SCHEMA, coverage_section};
pub const MAP_PROGRESS: &str = "preparation-map-progress-v1";
// Commit budgets, never limits on actors, hours or complete replies.
const BATCH_BYTES: usize = dashboard_base::prepared::pages::BATCH_BYTES;
/// Blocks of one branch a publication batch goes through at most: a family
/// prepared on its own commits about 26 times over a 50-year branch (663
/// blocks), five of them early ([`Batches`]).
const BATCH_BLOCKS: usize = 32;

/// Where publication batches end, from the recording alone, so the hourly
/// walk and every family prepared on its own publish the same ranges however
/// fast the machine runs: after a branch's last block before another
/// branch's (or the recording's end); early, after a branch's 1st, 2nd, 4th,
/// 8th and 16th blocks (every power of two below [`BATCH_BLOCKS`]), so a view
/// at the branch's start is exact once its family has folded one block; and
/// after every [`BATCH_BLOCKS`]th block of a branch. Blocks are counted from
/// the branch's first block wherever preparation started, resumed or grew.
/// The byte budget and measured runs end batches earlier; wall time never
/// does.
pub(super) struct Batches {
    /// Each block's position within its own branch.
    pub ordinals: Vec<usize>,
    blocks: usize,
}

impl Batches {
    pub(super) fn new(replay: &Replay) -> Self {
        let mut seen = BTreeMap::new();
        let ordinals = replay
            .index
            .iter()
            .map(|index| {
                let next = seen.entry(index.branch).or_insert(0usize);
                *next += 1;
                *next - 1
            })
            .collect();
        Self {
            ordinals,
            blocks: batch_blocks(),
        }
    }

    /// Whether a batch that went through `block` ends there by the
    /// recording: at an early commit or at its block budget.
    pub(super) fn full(&self, block: usize) -> bool {
        self.ordinals
            .get(block)
            .is_some_and(|&ordinal| ends_batch(ordinal + 1, self.blocks))
    }
}

/// Whether a batch ends after a branch's `through`th block (counted from 1)
/// with a budget of `blocks`: at every power of two below the budget, then at
/// every multiple of it.
fn ends_batch(through: usize, blocks: usize) -> bool {
    (through < blocks && through.is_power_of_two()) || through.is_multiple_of(blocks)
}

/// Run `run` with every family prepared hour by hour, for query tests that
/// publish the hourly walk's pages and coverage themselves.
#[cfg(test)]
pub(crate) fn with_test_hourly<T>(run: impl FnOnce() -> T) -> T {
    independent::with_test_hourly(run)
}

/// The block budget; tests set a smaller one to commit inside small fixtures.
fn batch_blocks() -> usize {
    #[cfg(test)]
    if let Some(blocks) = TEST_BATCH_BLOCKS.with(std::cell::Cell::get) {
        return blocks;
    }
    BATCH_BLOCKS
}

#[cfg(test)]
thread_local! {
    static TEST_BATCH_BLOCKS: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    static TEST_REPLANS: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Run `run` with batches of at most `blocks` blocks of a branch, for the
/// preparations it starts on this thread.
#[cfg(test)]
pub(super) fn with_test_batch_blocks<T>(blocks: usize, run: impl FnOnce() -> T) -> T {
    assert!(blocks > 0, "a batch goes through at least one block");
    crate::prepared::with_test_cell(&TEST_BATCH_BLOCKS, Some(blocks), run)
}

#[derive(Clone, Debug)]
pub struct Accepted {
    pub sections: Vec<Section>,
    pub bytes: u64,
    pub next_block: usize,
    pub new_blocks: usize,
    /// The hourly walk's coverage, by branch.
    pub coverage: BTreeMap<String, Coverage>,
    /// Each family prepared on its own's coverage, by branch
    /// ([`independent::coverages`]).
    pub families: BTreeMap<String, BTreeMap<String, Coverage>>,
}

pub fn sections(state: State, done: usize, total: Option<usize>) -> Vec<Section> {
    registry::STATUS
        .iter()
        .map(|&(id, label)| Section {
            id: id.into(),
            label: label.into(),
            state,
            done: done.to_string(),
            total: total.map(|value| value.to_string()),
            error: None,
        })
        .collect()
}

/// A family's refusal of one branch's recorded data: the first refusing hour and
/// the family's own error text, which its queries answer with. A dependent
/// refusal names the family it came `from`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Refusal {
    hour: i32,
    error: String,
    from: Option<String>,
}

/// One branch's refused families, by family name.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
struct Refused(BTreeMap<String, Refusal>);

/// True when a family's own error refuses the recorded data rather than
/// stopping preparation (I/O, cancellation and panics still stop it).
fn refuses(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::InvalidData
}

impl Refused {
    /// Whether `family` has not refused this branch.
    fn open(&self, family: &str) -> bool {
        self.0.is_empty() || !self.0.contains_key(family)
    }

    /// Whether the hourly walk goes on with `family` on this branch: it has
    /// not refused it, and it is not prepared on its own without another
    /// family reading it in the same hour ([`independent::hourly`]). Every
    /// observation, flush, save and load of the walk asks this, so a family
    /// prepared on its own is skipped by its registry entry alone.
    fn live(&self, family: &str) -> bool {
        self.open(family) && independent::hourly(family)
    }

    /// A family's own observation of a frame or its flush. A refusal is recorded
    /// and that family stops for the rest of the branch; others continue.
    fn check<T: Default>(
        &mut self,
        family: &'static str,
        hour: i32,
        result: io::Result<T>,
    ) -> io::Result<T> {
        #[cfg(test)]
        let result = lanes::inject(family, hour, result);
        match result {
            Err(error) if refuses(&error) => {
                // A refusal of a family's own read of another names that one.
                let from = crate::products::air_superiority::block::refused_from(&error)
                    .or_else(|| independent::event_feed::refused_from(&error));
                self.refuse(family, hour, error.to_string(), from);
                Ok(T::default())
            }
            other => other,
        }
    }

    fn refuse(&mut self, family: &str, hour: i32, error: String, from: Option<&str>) {
        if !self.open(family) {
            return;
        }
        self.0.insert(
            family.to_owned(),
            Refusal {
                hour,
                error: error.clone(),
                from: from.map(str::to_owned),
            },
        );
        if let Some(row) = plan::current().family(family) {
            for observer in row.observers {
                self.refuse(
                    observer.name(),
                    hour,
                    error.clone(),
                    Some(from.unwrap_or(family)),
                );
            }
        }
    }
}

// A family's own observation or flush, skipped once the family has refused this
// branch. `$operation` keeps its value; only an InvalidData error is absorbed.
macro_rules! family {
    ($refused:ident, $family:literal, $hour:expr, $operation:expr) => {
        if $refused.live($family) {
            let result = $operation;
            $refused.check($family, $hour, result)?
        } else {
            Default::default()
        }
    };
}

#[derive(Clone, Serialize, Deserialize)]
struct Cursor {
    next_block: usize,
    previous: Option<Block>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Block {
    branch: u64,
    first: i32,
    last: i32,
    offset: u64,
    length: u64,
    raw_length: u64,
}
impl From<&BlockIndex> for Block {
    fn from(value: &BlockIndex) -> Self {
        Self {
            branch: value.branch,
            first: value.first_hour,
            last: value.last_hour,
            offset: value.offset,
            length: value.length,
            raw_length: value.raw_length,
        }
    }
}
impl Cursor {
    fn replay(&self) -> ReplayCursor {
        ReplayCursor {
            next_block: self.next_block,
            previous: self.previous.as_ref().map(|value| BlockIndex {
                branch: value.branch,
                first_hour: value.first,
                last_hour: value.last,
                offset: value.offset,
                length: value.length,
                raw_length: value.raw_length,
            }),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Checkpoint {
    schema: u32,
    cursor: Cursor,
    selected: Vec<u16>,
    header: Vec<u8>,
    index: Vec<Block>,
    sessions: Vec<Vec<u8>>,
    branches: Vec<Vec<u8>>,
    gaps: Vec<Vec<u8>>,
    map: dashboard_base::prepared_map::Progress,
    content: Option<bool>,
    display_error: Option<String>,
    display_failures: u64,
    /// Families that refused a branch's recorded data, by branch.
    refused: BTreeMap<u64, Refused>,
    /// Families prepared on their own ([`independent`]), by name, with their
    /// progress. Preparation of other families starts over when these differ.
    #[serde(default)]
    independent: BTreeMap<String, independent::Progress>,
    #[serde(default)]
    production: BTreeMap<String, Coverage>,
    /// Active payload bytes excluding this checkpoint's own file.
    bytes: u64,
}

impl Checkpoint {
    fn accepted_walk(&self) -> bool {
        self.cursor.next_block > 0
            || self.cursor.previous.is_some()
            || self.map.resume_range.is_some()
            || self.map.branches.values().any(|branch| {
                branch.checkpoints > 0
                    || branch.since_checkpoint > 0
                    || branch.last_hour.is_some()
                    || branch.last_block.is_some()
            })
    }
    fn forward_complete(&self, total: usize) -> bool {
        self.cursor.next_block == total
            && self
                .independent
                .values()
                .all(|family| family.next_block == total)
    }
}

/// The saved branch state leaves out the state of each family prepared on
/// its own that the walk does not run ([`independent::hourly`]): read back,
/// that state is new, as the walk never observes it.
macro_rules! unsaved_while_own {
    ($($check:ident => $family:literal),* $(,)?) => {
        $(
            fn $check<T>(_: &T) -> bool {
                !independent::hourly($family)
            }
        )*
    };
}
unsaved_while_own!(
    own_production => "production",
    own_construction => "construction",
    own_stockpile => "stockpile",
    own_subjects => "subjects",
    own_ideas => "ideas",
    own_experience => "experience",
    own_peace => "peace",
    own_casualties => "casualties",
    own_research => "research",
    own_projects => "projects",
    own_nukes => "nukes",
    own_factories => "factories",
    own_repairs => "repairs",
    own_feed => "event_feed",
    own_naval => "naval",
    own_divisions => "divisions",
    own_air => "air",
    own_navy => "navy",
    own_supply => "supply",
    own_stability => "stability",
    own_fuel => "fuel",
    own_equipment_losses => "equipment_losses",
    own_trade => "trade",
    own_territory => "territory",
);

/// The state of a family whose new state needs the recording, absent when
/// the saved branch state left it out: [`load_branch`] makes it new then.
#[derive(Serialize, Deserialize)]
#[serde(transparent)]
struct Unsaved<T>(Option<Box<T>>);

impl<T> Default for Unsaved<T> {
    fn default() -> Self {
        Self(None)
    }
}

impl<T> Unsaved<T> {
    fn new(state: T) -> Self {
        Self(Some(Box::new(state)))
    }

    fn missing(&self) -> bool {
        self.0.is_none()
    }
}

impl<T> std::ops::Deref for Unsaved<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.0
            .as_deref()
            .expect("a branch state read back is made whole by load_branch")
    }
}

impl<T> std::ops::DerefMut for Unsaved<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.0
            .as_deref_mut()
            .expect("a branch state read back is made whole by load_branch")
    }
}

#[derive(Serialize, Deserialize)]
struct BranchState {
    schema: u32,
    focus: Box<FocusAccumulator>,
    index: Box<FocusIndex>,
    #[serde(default, skip_serializing_if = "own_production")]
    production: Box<ProductionAccumulator>,
    measure: Box<MeasureAccumulator>,
    land: Box<crate::products::land_battles::Census>,
    #[serde(default, skip_serializing_if = "own_naval")]
    naval: Box<crate::products::naval_battles::Scan>,
    #[serde(default, skip_serializing_if = "own_divisions")]
    divisions: Box<crate::products::division_log::Scan>,
    #[serde(skip)]
    identities: Box<crate::products::country_history::HistoryScan>,
    army: Box<crate::products::army_size::prepared::Prepared>,
    #[serde(default, skip_serializing_if = "own_air")]
    air: Box<crate::products::air_force::prepared::Prepared>,
    #[serde(default, skip_serializing_if = "own_navy")]
    navy: Box<crate::products::navy::prepared::Prepared>,
    #[serde(default, skip_serializing_if = "own_nukes")]
    nukes: Box<crate::products::country_nukes::prepared::Prepared>,
    #[serde(default, skip_serializing_if = "own_factories")]
    factories: Box<crate::products::country_factories::prepared::Prepared>,
    #[serde(default, skip_serializing_if = "own_repairs")]
    repairs: Box<crate::products::dockyard_repairs::Prepared>,
    #[serde(default, skip_serializing_if = "own_supply")]
    supply: Box<crate::products::supply_health::prepared::Prepared>,
    #[serde(default, skip_serializing_if = "own_construction")]
    construction: Box<crate::products::construction_log::prepared::Prepared>,
    resources: Box<crate::products::resource_balance::prepared::Prepared>,
    #[serde(default, skip_serializing_if = "own_trade")]
    trade: Box<crate::products::trade_flows::prepared::Prepared>,
    #[serde(default, skip_serializing_if = "own_stockpile")]
    stockpile: Box<crate::products::equipment_stockpile::prepared::Prepared>,
    #[serde(skip)]
    groups: Box<crate::products::country_groups::Scan>,
    group_changes: Box<crate::products::country_groups::changes::Prepared>,
    #[serde(skip)]
    air_ledger: Box<crate::products::military_air_ledger::prepared::Prepared>,
    #[serde(skip)]
    aces: Box<crate::products::air_aces::prepared::Prepared>,
    #[serde(skip)]
    raids: Box<crate::products::raids::prepared::Prepared>,
    #[serde(skip)]
    air_superiority: Box<crate::products::air_superiority::prepared::Prepared>,
    #[serde(skip)]
    subjects: Box<crate::products::subjects::Scan>,
    #[serde(default, skip_serializing_if = "own_subjects")]
    subject_changes: Box<super::series::Accumulator>,
    #[serde(default, skip_serializing_if = "own_subjects")]
    subject_late: super::series::LateIndex,
    #[serde(skip)]
    autonomy: Box<crate::products::autonomy::ScannedAutonomy>,
    #[serde(skip)]
    tension: Box<crate::products::world_tension::Scan>,
    #[serde(skip)]
    wars: Box<crate::products::wars::prepared::Observer>,
    #[serde(skip)]
    ideas: Box<crate::products::idea_history::Scan>,
    #[serde(default, skip_serializing_if = "own_ideas")]
    idea_changes: Box<crate::products::idea_history::PreparedChanges>,
    #[serde(default, skip_serializing_if = "own_feed")]
    feed: Box<crate::products::event_feed::prepared::Observer>,
    #[serde(skip)]
    parties: Box<crate::products::party_history::prepared::Prepared>,
    #[serde(skip)]
    intelligence: Box<crate::products::intelligence_prepared::Observer>,
    #[serde(default, skip_serializing_if = "own_stability")]
    stability: Box<crate::prepared_stability::StabilityAccumulator>,
    #[serde(default, skip_serializing_if = "own_experience")]
    experience: Box<crate::prepared_stability::ExperienceAccumulator>,
    #[serde(skip)]
    ledger: Box<crate::products::politics_ledger::prepared::Prepared>,
    #[serde(default, skip_serializing_if = "own_fuel")]
    fuel: Box<crate::products::country_fuel::prepared::Prepared>,
    #[serde(default, skip_serializing_if = "own_territory")]
    territory: Box<crate::products::territory::prepared::Prepared>,
    #[serde(skip)]
    occupation: Box<crate::products::occupied_states::prepared::Prepared>,
    game_score: Box<crate::products::game_score::prepared::Prepared>,
    #[serde(skip)]
    leaders: Box<crate::products::unit_leaders::prepared::Prepared>,
    #[serde(default, skip_serializing_if = "own_research")]
    research: Box<crate::products::research::prepared::Prepared>,
    #[serde(default, skip_serializing_if = "own_peace")]
    peace: Unsaved<crate::products::peace_deals::prepared::Prepared>,
    #[serde(default, skip_serializing_if = "own_projects")]
    projects: Box<crate::products::special_projects::prepared::Prepared>,
    #[serde(default, skip_serializing_if = "own_casualties")]
    casualties: Unsaved<crate::products::war_casualties::prepared::Prepared>,
    #[serde(default, skip_serializing_if = "own_equipment_losses")]
    equipment_losses: Unsaved<crate::products::equipment_losses::prepared::Prepared>,
}
impl BranchState {
    fn resume(
        &mut self,
        frame: &replay_frame::frame::DecodedFrame,
        branch: u64,
        refused: &Refused,
    ) {
        if frame.branch == branch {
            if refused.live("naval") {
                self.naval.resume(frame);
            }
            if refused.live("divisions") {
                self.divisions.resume(frame);
            }
            if refused.live("politics_ledger") {
                self.ledger.resume(frame);
            }
            if refused.live("projects") {
                self.projects.restore_views(frame);
            }
        }
    }

    fn new(replay: &Replay, branch: u64, first: i32) -> Box<Self> {
        Box::new(Self {
            schema: SCHEMA,
            focus: Box::new(FocusAccumulator::default()),
            index: Box::new(FocusIndex::default()),
            production: Box::new(ProductionAccumulator::new(
                replay.format.version(),
                branch,
                first,
            )),
            measure: Box::new(MeasureAccumulator::new(first)),
            land: Box::new(crate::products::land_battles::Census::new(replay.format)),
            naval: Box::new(crate::products::naval_battles::Scan::default()),
            divisions: Box::new(crate::products::division_log::Scan::new(replay.format)),
            identities: Box::new(crate::products::country_history::HistoryScan::new(
                false,
                &[],
            )),
            army: Default::default(),
            air: Default::default(),
            navy: Default::default(),
            nukes: Default::default(),
            factories: Default::default(),
            repairs: Default::default(),
            supply: Default::default(),
            construction: Default::default(),
            resources: Default::default(),
            trade: Default::default(),
            stockpile: Default::default(),
            groups: Box::new(crate::products::country_groups::Scan::new(replay, branch)),
            subjects: Box::new(crate::products::subjects::Scan::new(replay, branch)),
            group_changes: Default::default(),
            subject_changes: Default::default(),
            subject_late: Default::default(),
            autonomy: Default::default(),
            tension: Default::default(),
            wars: Default::default(),
            ideas: Box::new(crate::products::idea_history::Scan::new(replay, branch)),
            idea_changes: Default::default(),
            feed: Default::default(),
            parties: Default::default(),
            air_ledger: Default::default(),
            aces: Default::default(),
            raids: Default::default(),
            air_superiority: Default::default(),
            intelligence: Default::default(),
            stability: Default::default(),
            experience: Default::default(),
            ledger: Default::default(),
            fuel: Default::default(),
            territory: Default::default(),
            occupation: Default::default(),
            game_score: Default::default(),
            leaders: Box::new(crate::products::unit_leaders::prepared::Prepared::new(
                replay, branch,
            )),
            research: Default::default(),
            peace: Unsaved::new(crate::products::peace_deals::prepared::Prepared::new(
                replay, branch,
            )),
            projects: Box::new(crate::products::special_projects::prepared::Prepared::new(
                replay, branch,
            )),
            casualties: Unsaved::new(crate::products::war_casualties::prepared::Prepared::new(
                replay, branch,
            )),
            equipment_losses: Unsaved::new(
                crate::products::equipment_losses::prepared::Prepared::new(replay, branch, first),
            ),
        })
    }
}
fn branch_section(branch: u64) -> String {
    format!("preparation-branch-{branch}")
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
/// The branch continuation is rewritten at every commit, so it is stored
/// compressed. Level 1 keeps the per-block cost small.
fn save_branch(writer: &mut store::Writer, branch: u64, state: &BranchState) -> io::Result<()> {
    writer.write_at(
        &branch_section(branch),
        0,
        compressed_json(state)?.as_slice(),
    )
}
/// The JSON encoding shared by hourly branch and independent family continuations.
fn compressed_json<T: Serialize>(value: &T) -> io::Result<Vec<u8>> {
    let _cpu = ::replay_frame::cpu::scope("save serialize", None);
    let mut encoded =
        io::BufWriter::with_capacity(1 << 20, zstd::stream::Encoder::new(Vec::new(), 1)?);
    serde_json::to_writer(&mut encoded, value)?;
    encoded
        .into_inner()
        .map_err(io::IntoInnerError::into_error)?
        .finish()
}
fn json<T: Serialize>(writer: &mut store::Writer, section: &str, value: &T) -> io::Result<()> {
    let _cpu = ::replay_frame::cpu::scope("save serialize", None);
    writer.write_at(section, 0, serde_json::to_vec(value)?.as_slice())
}
fn header(replay: &Replay) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    replay.identity.encode(&mut bytes)?;
    bytes.extend_from_slice(&replay.campaign_id);
    bytes.extend_from_slice(&replay.format.version().to_le_bytes());
    bytes.extend_from_slice(&replay.keyframe_interval.to_le_bytes());
    Ok(bytes)
}
fn sessions(replay: &Replay) -> io::Result<Vec<Vec<u8>>> {
    replay
        .sessions
        .iter()
        .map(|session| {
            let mut bytes = Vec::new();
            session.encode(&mut bytes)?;
            Ok(bytes)
        })
        .collect()
}
fn branches(replay: &Replay) -> io::Result<Vec<Vec<u8>>> {
    replay
        .branches
        .iter()
        .map(|branch| {
            let mut bytes = Vec::new();
            branch.encode(&mut bytes)?;
            Ok(bytes)
        })
        .collect()
}
fn gaps(replay: &Replay) -> io::Result<Vec<Vec<u8>>> {
    replay
        .gaps
        .iter()
        .map(|gap| {
            let mut bytes = Vec::new();
            gap.encode(&mut bytes)?;
            Ok(bytes)
        })
        .collect()
}
fn active_bytes(previous: u64, change: i128) -> io::Result<u64> {
    u64::try_from(
        i128::from(previous)
            .checked_add(change)
            .ok_or_else(|| invalid("prepared byte count overflow"))?,
    )
    .map_err(|_| invalid("prepared byte count overflow"))
}
fn last_blocks(replay: &Replay) -> BTreeMap<u64, usize> {
    replay
        .index
        .iter()
        .enumerate()
        .map(|(position, index)| (index.branch, position))
        .collect()
}
fn branch_coverage(
    map: &dashboard_base::prepared_map::Progress,
    branch: u64,
    last_blocks: &BTreeMap<u64, usize>,
) -> Option<Coverage> {
    let progress = map.branches.get(&branch)?;
    let hour = progress.last_hour?;
    let &(first, last) = map.ranges.get(&branch)?;
    let complete = progress.last_block == last_blocks.get(&branch).copied();
    Some(Coverage {
        schema: SCHEMA,
        first_hour: first,
        last_hour: if complete { last } else { hour },
        complete,
    })
}
fn accepted_coverage(
    map: &dashboard_base::prepared_map::Progress,
    last_blocks: &BTreeMap<u64, usize>,
) -> BTreeMap<String, Coverage> {
    map.branches
        .keys()
        .filter_map(|&branch| {
            branch_coverage(map, branch, last_blocks).map(|range| (branch.to_string(), range))
        })
        .collect()
}
/// Each family prepared on its own's coverage by branch, as its own coverage
/// pages publish it with `state`.
fn family_coverages(
    state: &Checkpoint,
    replay: &Replay,
    last_blocks: &BTreeMap<u64, usize>,
) -> BTreeMap<String, BTreeMap<String, Coverage>> {
    let mut families =
        independent::coverages(&state.independent, replay, &state.map.ranges, last_blocks);
    families.insert(
        "production-lines".into(),
        state
            .production
            .iter()
            .map(|(branch, coverage)| {
                let mut coverage = coverage.clone();
                coverage.complete &= branch
                    .parse::<u64>()
                    .ok()
                    .and_then(|branch| last_blocks.get(&branch))
                    .and_then(|position| replay.index.get(*position))
                    .is_some_and(|block| block.last_hour == coverage.last_hour);
                (branch.clone(), coverage)
            })
            .collect(),
    );
    families
}
/// A branch's published coverage page, carrying its family refusals so a
/// query reads both from one small page. The reply's coverage stays unchanged.
#[derive(Serialize, Deserialize)]
struct CoveragePage {
    #[serde(flatten)]
    coverage: Coverage,
    #[serde(default, skip_serializing_if = "refused_none")]
    refused: Refused,
}
fn refused_none(refused: &Refused) -> bool {
    refused.0.is_empty()
}
/// The coverage page a family prepared on its own published for `query`'s
/// branch, when `query` waits only for that family.
fn family_page(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    query: &str,
) -> io::Result<Option<(&'static str, CoveragePage)>> {
    let Some(family) = independent::gating(query) else {
        return Ok(None);
    };
    Ok(own_page(store, key, family, branch)?.map(|page| (family, page)))
}
/// The coverage page `family`, prepared on its own, published for `branch`.
fn own_page(
    store: &store::Store,
    key: &store::SourceKey,
    family: &str,
    branch: u64,
) -> io::Result<Option<CoveragePage>> {
    let Some(file) = store.read(key, &independent::coverage_section(family, branch), 0)? else {
        return Ok(None);
    };
    let page: CoveragePage = serde_json::from_reader(io::BufReader::new(file))?;
    if page.coverage.schema != SCHEMA || page.coverage.first_hour > page.coverage.last_hour {
        return Err(invalid("Unsupported prepared coverage schema"));
    }
    Ok(Some(page))
}
/// The coverage every one of `families` has on `branch`
/// ([`independent::joint`]): its own of each prepared on its own, the
/// branch's of any other; through the least of their last hours, complete
/// when every one is. None until every one has accepted some.
fn joint_coverage(
    source: &Source,
    content: Option<&ContentOptions>,
    branch: u64,
    families: &[registry::FamilyId],
    cancelled: &AtomicBool,
) -> io::Result<Option<Coverage>> {
    check_cancel(cancelled)?;
    let (store, key) = super::files(source, content)?;
    let mut walked = None;
    let mut joint: Option<Coverage> = None;
    for (position, family) in families.iter().map(|family| family.name()).enumerate() {
        let extent = if independent::registered(family) {
            own_short(
                family,
                own_page(&store, &key, family, branch)?.map(|page| page.coverage),
            )
        } else {
            if walked.is_none() {
                walked = Some(coverage(source, content, branch, cancelled)?);
            }
            walked.clone().flatten()
        };
        let Some(extent) = extent else {
            // The families after it wait too, as far as this reply knows.
            for family in &families[position + 1..] {
                super::coverage::note_short(gate_entry(family.name()));
            }
            return Ok(None);
        };
        joint = Some(match joint {
            None => extent,
            Some(held) => Coverage {
                schema: SCHEMA,
                first_hour: held.first_hour.max(extent.first_hour),
                last_hour: held.last_hour.min(extent.last_hour),
                complete: held.complete && extent.complete,
            },
        });
    }
    Ok(joint)
}
/// The coverage `product`'s queries wait for on `branch`: that of the
/// families whose pages its replies read together, while any is prepared on
/// its own ([`independent::joint`]); that of the family prepared on its own
/// whose registry entry gates the product ([`independent::gating`]); the
/// branch's otherwise; None before any is accepted. A product gated so is
/// answered while the hourly walk prepares.
pub(crate) fn product_coverage(
    source: &Source,
    content: Option<&ContentOptions>,
    branch: u64,
    product: &str,
    cancelled: &AtomicBool,
) -> io::Result<Option<Coverage>> {
    if let Some(families) = independent::joint(product) {
        return joint_coverage(source, content, branch, families, cancelled);
    }
    if independent::gating(product).is_none() {
        return coverage(source, content, branch, cancelled);
    }
    check_cancel(cancelled)?;
    let (store, key) = super::files(source, content)?;
    Ok(match family_page(&store, &key, branch, product)? {
        Some((family, page)) => own_short(family, Some(page.coverage)),
        None => {
            if let Some(family) = independent::gating(product) {
                super::coverage::note_short(family);
            }
            None
        }
    })
}
/// `extent`, the coverage of `family` prepared on its own, noted as short
/// for the query collecting what it waits for when absent or incomplete
/// ([`super::coverage::waiting`]).
fn own_short(family: &'static str, extent: Option<Coverage>) -> Option<Coverage> {
    if !extent.as_ref().is_some_and(|extent| extent.complete) {
        super::coverage::note_short(family);
    }
    extent
}
/// The status entry `family`'s coverage is listed under: its own while it
/// is prepared on its own, the hourly walk's otherwise.
fn gate_entry(family: &'static str) -> &'static str {
    if independent::registered(family) {
        family
    } else {
        super::coverage::WALK
    }
}
/// The status entries `product`'s replies wait for, as [`product_coverage`]
/// chooses its coverage: each family of a joint product, the family gating
/// it, or the hourly walk. A pending reply that consulted no coverage names
/// these.
pub(crate) fn gate(product: &str) -> Vec<&'static str> {
    let mut entries: Vec<_> = match (independent::joint(product), independent::gating(product)) {
        (Some(families), _) => families
            .iter()
            .map(|family| gate_entry(family.name()))
            .collect(),
        (None, Some(family)) => vec![family],
        (None, None) => vec![super::coverage::WALK],
    };
    entries.sort_unstable();
    entries.dedup();
    entries
}

/// Publication identities of every declared supplier of a query: its gates
/// and the families whose refusals it reads. One query scope pins these pages
/// and the data they publish to the same manifest. A replacement, including a
/// refusal or Clear followed by preparation, changes its identity.
pub(crate) fn query_pages(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    product: &str,
) -> io::Result<Vec<(String, Option<[u64; 3]>)>> {
    let mut gates: std::collections::BTreeSet<_> = gate(product).into_iter().collect();
    for family in plan::current()
        .families()
        .iter()
        .filter(|family| family.queries.contains(&product))
    {
        gates.insert(gate_entry(family.id.name()));
    }
    pages_of(store, key, branch, gates)
}

/// [`query_pages`] of a query's own gates alone, without the families it reads through
/// another's supply.
pub(crate) fn gate_pages(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    product: &str,
) -> io::Result<Vec<(String, Option<[u64; 3]>)>> {
    pages_of(store, key, branch, gate(product))
}

/// The publication identity of each of `gates` on `branch`.
fn pages_of(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    gates: impl IntoIterator<Item = &'static str>,
) -> io::Result<Vec<(String, Option<[u64; 3]>)>> {
    gates
        .into_iter()
        .map(|family| {
            let section = if family == super::coverage::WALK {
                coverage_section(branch)
            } else {
                independent::coverage_section(family, branch)
            };
            let id = store
                .range_id(key, &section, 0)?
                .map(|id| id.cache_key());
            Ok((section, id))
        })
        .collect()
}
/// The coverage page of the hourly walk and of every family on `branch`, registered or not,
/// production lines' among them: every publication to the branch replaces one of them.
pub(crate) fn published_pages(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
) -> io::Result<Vec<(String, Option<[u64; 3]>)>> {
    let plan = plan::current();
    let families: std::collections::BTreeSet<&str> = plan
        .families()
        .iter()
        .map(|family| family.id.name())
        .chain(plan.names())
        .chain(
            registry::JOBS
                .iter()
                .filter(|job| job.runner == registry::Runner::Beside)
                .map(|job| job.name),
        )
        .collect();
    std::iter::once(coverage_section(branch))
        .chain(
            families
                .into_iter()
                .map(|family| independent::coverage_section(family, branch)),
        )
        .map(|section| {
            let id = store
                .range_id(key, &section, 0)?
                .map(|id| id.cache_key());
            Ok((section, id))
        })
        .collect()
}
/// The gates readers below the dashboard ask ([`super::coverage::Gates`]):
/// [`product_coverage`] and [`refusal`].
struct Gates;

impl super::coverage::Gates for Gates {
    fn product_coverage(
        &self,
        source: &Source,
        content: Option<&ContentOptions>,
        branch: u64,
        product: &str,
        cancelled: &AtomicBool,
    ) -> io::Result<Option<Coverage>> {
        product_coverage(source, content, branch, product, cancelled)
    }

    fn refusal(
        &self,
        source: &Source,
        content: Option<&ContentOptions>,
        branch: u64,
        query: &str,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        refusal(source, content, branch, query, cancelled)
    }
}

/// Installs the pipeline's [`Gates`] for readers below the dashboard; the
/// pipeline, the query route and the service install them before reading.
pub fn install_gates() {
    static GATES: Gates = Gates;
    super::coverage::install_gates(&GATES);
    dashboard_base::products::country_styles::install_style_runs(
        &crate::products::style_runs::PREPARED_STYLES,
    );
}
/// Every prepared query of a refused family on this branch, and every query
/// reading one, answers with that family's refusal exactly as its product
/// route refused, rather than with the family's earlier prepared data. A
/// family prepared on its own keeps its refusals in its own page, which it
/// publishes before the hourly walk reaches the refusing hour: its gated
/// queries, and every query the registry lists as reading it, refuse from
/// there.
pub(crate) fn refusal(
    source: &Source,
    content: Option<&ContentOptions>,
    branch: u64,
    query: &str,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    let families: Vec<&registry::Descriptor> = plan::current()
        .families()
        .iter()
        .copied()
        .filter(|family| family.queries.contains(&query))
        .collect();
    if families.is_empty()
        && independent::gating(query).is_none()
        && independent::joint(query).is_none()
    {
        return Ok(());
    }
    check_cancel(cancelled)?;
    let (store, key) = super::files(source, content)?;
    let refused = |refusal: &Refusal| -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            refusal.error.clone(),
        ))
    };
    if let Some((family, page)) = family_page(&store, &key, branch, query)?
        && let Some(refusal) = page.refused.0.get(family)
    {
        return refused(refusal);
    }
    let page = |section: &str| -> io::Result<Option<CoveragePage>> {
        let Some(file) = store.read(&key, section, 0)? else {
            return Ok(None);
        };
        Ok(Some(serde_json::from_reader(io::BufReader::new(file))?))
    };
    let walked = page(&coverage_section(branch))?;
    // In registry order, as the hourly walk's page alone answers.
    for family in families.into_iter().map(|family| family.id.name()) {
        let own = if independent::registered(family) {
            page(&independent::coverage_section(family, branch))?
        } else {
            None
        };
        if let Some(refusal) = [own.as_ref(), walked.as_ref()]
            .into_iter()
            .flatten()
            .find_map(|page| page.refused.0.get(family))
        {
            return refused(refusal);
        }
    }
    Ok(())
}
fn save_coverage(
    writer: &mut store::Writer,
    map: &dashboard_base::prepared_map::Progress,
    branch: u64,
    last_blocks: &BTreeMap<u64, usize>,
    refused: Option<&Refused>,
) -> io::Result<()> {
    if map
        .branches
        .get(&branch)
        .is_some_and(|progress| progress.last_hour.is_some())
        && !map.ranges.contains_key(&branch)
    {
        return Err(invalid("Prepared branch range is missing"));
    }
    let Some(coverage) = branch_coverage(map, branch, last_blocks) else {
        return Ok(());
    };
    json(
        writer,
        &coverage_section(branch),
        &CoveragePage {
            coverage,
            refused: refused.cloned().unwrap_or_default(),
        },
    )
}
/// Whether a flushed branch is read back from its saved state rather than
/// continued in memory: in tests that compare both, which publish the same.
fn reload_flushed() -> bool {
    #[cfg(test)]
    {
        RELOAD_FLUSHED.with(std::cell::Cell::get)
    }
    #[cfg(not(test))]
    {
        false
    }
}

#[cfg(test)]
thread_local! {
    pub(super) static RELOAD_FLUSHED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static TEST_RESTORED: std::cell::RefCell<Option<Vec<&'static str>>> =
        const { std::cell::RefCell::new(None) };
    static TEST_ALL_TABLES: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Flushed branches continued in memory into their next block.
    pub(super) static CONTINUED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn selected() -> &'static [TableId] {
    #[cfg(test)]
    if TEST_ALL_TABLES.with(std::cell::Cell::get) {
        return &TableId::ALL;
    }
    // A partial registry returns families with content-dependent reads to
    // the walk. Keep their whole selection, including the hourly oracle's.
    if independent::walks_own() {
        return &TableId::ALL;
    }
    static SELECTED: std::sync::LazyLock<Vec<TableId>> =
        std::sync::LazyLock::new(|| plan::rows().walk_tables());
    &SELECTED
}

/// The walk reads a block-prepared family's rows only when that block's
/// extract could not be folded. Every block starts at its own keyframe.
fn block_selected(resources_hourly: bool, army_hourly: bool) -> Vec<TableId> {
    let selected = selected();
    TableId::ALL
        .into_iter()
        .filter(|table| {
            selected.contains(table)
                || (resources_hourly
                    && crate::products::resource_balance::prepared::FAMILY
                        .tables
                        .contains(table))
                || (army_hourly && crate::products::army_size::prepared::TABLES.contains(table))
        })
        .collect()
}

fn zero_checkpoint(replay: &Replay, content: Option<&ContentOptions>) -> io::Result<Checkpoint> {
    Ok(Checkpoint {
        schema: SCHEMA,
        cursor: Cursor {
            next_block: 0,
            previous: None,
        },
        selected: selected().iter().map(|table| *table as u16).collect(),
        header: header(replay)?,
        index: replay.index.iter().map(Block::from).collect(),
        sessions: sessions(replay)?,
        branches: branches(replay)?,
        gaps: gaps(replay)?,
        map: Default::default(),
        content: content.map(|content| content.allow_unverified),
        display_error: None,
        display_failures: 0,
        refused: BTreeMap::new(),
        independent: independent::names()
            .iter()
            .map(|&name| (name.to_owned(), independent::Progress::default()))
            .collect(),
        production: BTreeMap::new(),
        bytes: 0,
    })
}

fn checkpoint_bytes(
    store: &store::Store,
    key: &store::SourceKey,
    saved: Option<&store::SourceKey>,
) -> io::Result<Option<Vec<u8>>> {
    let file = match saved {
        Some(previous) if previous != key => store.read_prefix(previous, key, CHECKPOINT, 0)?,
        _ => store.read(key, CHECKPOINT, 0)?,
    };
    file.map(|mut file| {
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(bytes)
    })
    .transpose()
}

fn checkpoint(
    store: &store::Store,
    key: &store::SourceKey,
    saved: Option<&store::SourceKey>,
) -> io::Result<Option<(Checkpoint, u64)>> {
    parse_checkpoint(checkpoint_bytes(store, key, saved)?.as_deref())
}

fn parse_checkpoint(packet: Option<&[u8]>) -> io::Result<Option<(Checkpoint, u64)>> {
    let Some(packet) = packet else {
        return Ok(None);
    };
    let state: Checkpoint = match serde_json::from_slice(packet) {
        Ok(state) => state,
        Err(_) => return Ok(None),
    };
    if state.schema != SCHEMA {
        return Ok(None);
    }
    let bytes = state
        .bytes
        .checked_add(packet.len() as u64)
        .ok_or_else(|| invalid("prepared byte count overflow"))?;
    Ok(Some((state, bytes)))
}
fn metadata_prefix(
    state: &Checkpoint,
    replay: &Replay,
    content: Option<&ContentOptions>,
) -> io::Result<bool> {
    if state.selected
        != selected()
            .iter()
            .map(|table| *table as u16)
            .collect::<Vec<_>>()
    {
        return Ok(false);
    }
    if state.map.selected != {
        let mut tables: Vec<_> = dashboard_base::prepared_map::selected_tables()
            .iter()
            .map(|table| *table as u16)
            .collect();
        tables.sort_unstable();
        tables
    } {
        return Ok(false);
    }
    // Every publisher owns a committed prefix of the same recorded index;
    // their cursors need not coincide or be ordered relative to one another.
    if !state
        .independent
        .keys()
        .map(String::as_str)
        .eq(independent::names().iter().copied())
        || state
            .independent
            .values()
            .any(|family| family.next_block > state.index.len())
    {
        return Ok(false);
    }
    if state.header != header(replay)?
        || state.content != content.map(|content| content.allow_unverified)
        || state.cursor.next_block > state.index.len()
        || state.index.len() > replay.index.len()
        || state
            .index
            .iter()
            .zip(&replay.index)
            .any(|(old, current)| old != &Block::from(current))
        || state.cursor.previous
            != state
                .cursor
                .next_block
                .checked_sub(1)
                .and_then(|position| state.index.get(position))
                .cloned()
    {
        return Ok(false);
    }
    let current_sessions = sessions(replay)?;
    let current_branches = branches(replay)?;
    let current_gaps = gaps(replay)?;
    if state
        .sessions
        .iter()
        .any(|session| !current_sessions.contains(session))
        || state
            .branches
            .iter()
            .any(|branch| !current_branches.contains(branch))
        || state.gaps.iter().any(|gap| !current_gaps.contains(gap))
    {
        return Ok(false);
    }
    // New gap metadata cannot change the accepted part of a prior branch.
    for gap in &replay.gaps {
        let mut encoded = Vec::new();
        gap.encode(&mut encoded)?;
        if !state.gaps.contains(&encoded)
            && state
                .index
                .iter()
                .any(|block| block.branch == gap.branch && gap.first_hour <= block.last)
        {
            return Ok(false);
        }
    }
    Ok(true)
}
/// Whether [`load_branch`] restores `family`'s saved state: the walk restores
/// only what it goes on with ([`Refused::live`]).
fn restores(refused: &Refused, family: &'static str) -> bool {
    let live = refused.live(family);
    #[cfg(test)]
    if live {
        TEST_RESTORED.with(|restored| {
            if let Some(restored) = restored.borrow_mut().as_mut() {
                restored.push(family);
            }
        });
    }
    live
}

/// Runs `run`, returning with its value every family whose saved state
/// [`load_branch`] restored on this thread, in order.
#[cfg(test)]
fn with_test_restored<T>(run: impl FnOnce() -> T) -> (T, Vec<&'static str>) {
    struct Reset(Option<Vec<&'static str>>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_RESTORED.with(|restored| *restored.borrow_mut() = self.0.take());
        }
    }
    let reset = Reset(TEST_RESTORED.with(|restored| restored.replace(Some(Vec::new()))));
    let value = run();
    let restored = TEST_RESTORED.with(|restored| restored.borrow_mut().take());
    drop(reset);
    (value, restored.unwrap_or_default())
}

/// Restores a branch continuation. A refused family's own pages are never read
/// again: its state stays at its default and it is not observed for the branch.
fn load_branch(
    store: &store::Store,
    key: &store::SourceKey,
    saved: Option<&store::SourceKey>,
    replay: &Replay,
    branch: u64,
    first: i32,
    refused: &Refused,
) -> io::Result<Box<BranchState>> {
    if saved.is_none() {
        return Ok(BranchState::new(replay, branch, first));
    }
    let name = branch_section(branch);
    let file = match saved {
        Some(previous) if previous != key => store.read_prefix(previous, key, &name, 0)?,
        _ => store.read(key, &name, 0)?,
    };
    let Some(file) = file else {
        return Ok(BranchState::new(replay, branch, first));
    };
    let mut state: Box<BranchState> =
        serde_json::from_reader(io::BufReader::new(zstd::stream::Decoder::new(file)?))
            .map_err(|error| invalid(&error.to_string()))?;
    if state.schema != SCHEMA {
        return Err(invalid("Unsupported prepared branch schema"));
    }
    if restores(refused, "stability") {
        state.stability.restore(replay, branch)?;
    }
    if independent::hourly("production") && state.production.schema != 1 {
        return Err(invalid("Unsupported prepared production state schema"));
    }
    // Left out while prepared on their own: new, as the walk never runs them.
    if state.peace.missing() {
        state.peace = Unsaved::new(crate::products::peace_deals::prepared::Prepared::new(
            replay, branch,
        ));
    }
    if state.equipment_losses.missing() {
        state.equipment_losses = Unsaved::new(
            crate::products::equipment_losses::prepared::Prepared::new(replay, branch, first),
        );
    }
    if state.casualties.missing() {
        state.casualties = Unsaved::new(crate::products::war_casualties::prepared::Prepared::new(
            replay, branch,
        ));
    }
    if restores(refused, "measure") {
        state.measure.validate_checkpoint()?;
    }
    if restores(refused, "identities") {
        // Kept under territory's name while the identities are prepared on their own.
        let section = format!("{}-{branch}", independent::kept_as("country-identities"));
        let file = match saved {
            Some(previous) if previous != key => store.read_prefix(previous, key, &section, 0)?,
            _ => store.read(key, &section, 0)?,
        }
        .ok_or_else(|| invalid("Prepared country identities are missing"))?;
        state.identities = serde_json::from_reader(io::BufReader::new(file))?;
    }
    if restores(refused, "groups") {
        // Kept under a reader's names while the groups are prepared on their own.
        let section = format!("{}-{branch}", independent::kept_as("country-groups"));
        let file = match saved {
            Some(previous) if previous != key => store.read_prefix(previous, key, &section, 0)?,
            _ => store.read(key, &section, 0)?,
        }
        .ok_or_else(|| invalid("Prepared country groups are missing"))?;
        state.groups = Box::new(crate::products::country_groups::Scan::read(
            io::BufReader::new(file),
        )?);
    }
    if restores(refused, "air_ledger") {
        state.air_ledger = Box::new(
            crate::products::military_air_ledger::prepared::Prepared::read(
                store, key, saved, branch,
            )?,
        );
        state
            .air_ledger
            .read_catalogues(store, key, saved, branch)?;
    }
    if restores(refused, "subjects") {
        let section = format!("subjects-{branch}");
        let file = match saved {
            Some(previous) if previous != key => store.read_prefix(previous, key, &section, 0)?,
            _ => store.read(key, &section, 0)?,
        }
        .ok_or_else(|| invalid("Prepared subjects are missing"))?;
        state.subjects = serde_json::from_reader(zstd::stream::Decoder::new(file)?)?;
    }
    if restores(refused, "autonomy") {
        state.autonomy = Box::new(
            super::series::read_saved_catalogue(store, key, saved, "autonomy", branch)?
                .ok_or_else(|| invalid("Prepared autonomy is missing"))?
                .1,
        );
    }
    if restores(refused, "tension") {
        state.tension = Box::new(
            super::series::read_saved_catalogue(store, key, saved, "world-tension", branch)?
                .ok_or_else(|| invalid("Prepared world tension is missing"))?
                .1,
        );
    }
    if restores(refused, "wars") {
        state.wars = Box::new(crate::products::wars::prepared::Observer::restore(
            super::series::read_saved_catalogue(store, key, saved, "wars", branch)?
                .ok_or_else(|| invalid("Prepared wars are missing"))?,
        ));
    }
    if restores(refused, "ideas") {
        let section = format!("idea-history-{branch}");
        let file = match saved {
            Some(previous) if previous != key => store.read_prefix(previous, key, &section, 0)?,
            _ => store.read(key, &section, 0)?,
        }
        .ok_or_else(|| invalid("Prepared idea history is missing"))?;
        state.ideas = Box::new(crate::products::idea_history::Scan::read(
            io::BufReader::new(file),
        )?);
    }
    if restores(refused, "politics_ledger") {
        let section = format!("politics-ledger-state-{branch}");
        let file = match saved {
            Some(previous) if previous != key => store.read_prefix(previous, key, &section, 0)?,
            _ => store.read(key, &section, 0)?,
        }
        .ok_or_else(|| invalid("Prepared politics ledger is missing"))?;
        state.ledger = Box::new(crate::products::politics_ledger::prepared::Prepared::read(
            io::BufReader::new(file),
        )?);
    }
    if restores(refused, "occupation") {
        let section = format!("occupation-{branch}");
        let file = match saved {
            Some(previous) if previous != key => store.read_prefix(previous, key, &section, 0)?,
            _ => store.read(key, &section, 0)?,
        }
        .ok_or_else(|| invalid("Prepared occupation state is missing"))?;
        state.occupation = Box::new(crate::products::occupied_states::prepared::Prepared::read(
            io::BufReader::new(file),
        )?);
    }
    if restores(refused, "parties") {
        let section = format!("party-history-{branch}");
        let file = match saved {
            Some(previous) if previous != key => store.read_prefix(previous, key, &section, 0)?,
            _ => store.read(key, &section, 0)?,
        }
        .ok_or_else(|| invalid("Prepared party history is missing"))?;
        state.parties = Box::new(crate::products::party_history::prepared::Prepared::read(
            io::BufReader::new(file),
        )?);
    }
    if restores(refused, "aces") {
        state.aces = Box::new(crate::products::air_aces::prepared::Prepared::resume(
            store, key, saved, replay, branch,
        )?);
    }
    if restores(refused, "raids") {
        state.raids = Box::new(crate::products::raids::prepared::Prepared::read(
            store, key, saved, branch,
        )?);
    }
    if restores(refused, "air_superiority") {
        state.air_superiority = Box::new(
            crate::products::air_superiority::prepared::Prepared::read(store, key, saved, branch)?,
        );
    }
    // The append admission probe precedes the prefix's current-key binding.
    // Restore leaders when load_branch runs again after that publication.
    if saved == Some(key) && restores(refused, "leaders") {
        let (keys, metadata) = super::series::read_catalogue(store, key, "unit-leaders", branch)?
            .ok_or_else(|| invalid("Prepared leader catalogue is missing"))?;
        let file = store
            .read(key, &format!("unit-leaders-{branch}"), 0)?
            .ok_or_else(|| invalid("Prepared leader continuation is missing"))?;
        state.leaders = Box::new(crate::products::unit_leaders::prepared::Prepared::read(
            io::BufReader::new(file),
            keys,
            metadata,
        )?);
    }
    if restores(refused, "intelligence") {
        state.intelligence = Box::new(crate::products::intelligence_prepared::Observer::load(
            store, key, saved, branch,
        )?);
    }
    Ok(state)
}

fn branches_supported(
    store: &store::Store,
    key: &store::SourceKey,
    saved: Option<&store::SourceKey>,
    replay: &Replay,
    state: &Checkpoint,
) -> io::Result<bool> {
    let mut checked = std::collections::BTreeSet::new();
    for block in state.index.iter().take(state.cursor.next_block) {
        if checked.insert(block.branch) {
            match load_branch(
                store,
                key,
                saved,
                replay,
                block.branch,
                block.first,
                &state
                    .refused
                    .get(&block.branch)
                    .cloned()
                    .unwrap_or_default(),
            ) {
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::InvalidData => return Ok(false),
                Err(error) => return Err(error),
            }
        }
    }
    Ok(true)
}

/// Publish the lightweight map binding atomically with its checkpoint pages.
pub fn write_map_progress(
    writer: &mut store::Writer,
    progress: &dashboard_base::prepared_map::Progress,
) -> io::Result<()> {
    let text = serde_json::to_vec(progress)?;
    if dashboard_base::prepared::pages::Pages::read_at(writer, MAP_PROGRESS, 0)?.as_deref()
        != Some(text.as_slice())
    {
        writer.write_at(MAP_PROGRESS, 0, text.as_slice())?;
    }
    Ok(())
}

/// The map attaches already-published checkpoints through the host's recording
/// directory. An unfinished tail does not hide accepted checkpoint pages.
pub fn map_checkpoint_binding(
    source: &Source,
    content: Option<&ContentOptions>,
) -> io::Result<
    Option<(
        std::sync::Arc<store::Store>,
        store::SourceKey,
        dashboard_base::prepared_map::Progress,
    )>,
> {
    let (store, key) = super::files(source, content)?;
    let Some(file) = store.read(&key, MAP_PROGRESS, 0)? else {
        return Ok(None);
    };
    let progress = serde_json::from_reader(io::BufReader::new(file))?;
    Ok(Some((store, key, progress)))
}

/// Count the hourly walk's unaccepted tail from index metadata and its cursor.
/// Zero does not imply that the independent families have finished. The
/// measurement journal uses this cursor; [`restore`] checks every publisher.
/// None means there is no reusable checkpoint, so the whole recording needs work.
/// This reads no recording frames or observer accumulators.
pub fn unprepared_blocks(
    source: &Source,
    content: Option<&ContentOptions>,
) -> io::Result<Option<usize>> {
    let (store, key) = super::files(source, content)?;
    let saved = store.saved_key()?;
    if !saved.as_ref().is_some_and(|previous| {
        previous == &key
            || previous.recording_size() < key.recording_size()
            || key.renames(previous)
    }) {
        return Ok(None);
    }
    let Some((state, _)) = checkpoint(&store, &key, saved.as_ref())? else {
        return Ok(None);
    };
    if !metadata_prefix(&state, source.replay_ref(), content)? {
        return Ok(None);
    }
    Ok(Some(
        source.replay_ref().index.len() - state.cursor.next_block,
    ))
}

#[cfg(test)]
thread_local! {
    static TEST_SNOW_CHECK: std::cell::RefCell<Option<std::rc::Rc<dyn Fn(&'static str)>>> =
        const { std::cell::RefCell::new(None) };
}

fn snow_check(cancelled: &AtomicBool, _stage: &'static str) -> io::Result<()> {
    #[cfg(test)]
    if let Some(hook) = TEST_SNOW_CHECK.with(|hook| hook.borrow().clone()) {
        hook(_stage);
    }
    check_cancel(cancelled)
}

fn snow_session(
    source: &Source,
    content: Option<&ContentOptions>,
    cancelled: &AtomicBool,
) -> io::Result<dashboard_base::map_export::Session> {
    check_cancel(cancelled)?;
    let layers = content
        .map(|options| source.content(&options.options))
        .transpose()?;
    let (store, key) = super::files(source, content)?;
    dashboard_base::map_export::Session::bind(
        source.replay_ref(),
        layers.as_ref().map(|content| &content.layers),
        source
            .replay_ref()
            .index
            .iter()
            .try_fold(0u64, |end, block| {
                block
                    .offset
                    .checked_add(block.length)
                    .map(|next| end.max(next))
                    .ok_or_else(|| invalid("snow source block end overflow"))
            })?,
        &store,
        &key,
        || snow_check(cancelled, "bind"),
    )
}

/// Restore completed preparation directly from input/index metadata. A cached
/// source does not wait for a background worker or reconstruct a historical frame.
pub fn restore(
    source: &Source,
    content: Option<&ContentOptions>,
    cancelled: &AtomicBool,
) -> io::Result<Option<Accepted>> {
    super::driver::restore(source, content, cancelled)
}

/// [`restore`] of the row backends the driver selected.
pub(super) fn restore_rows(
    source: &Source,
    content: Option<&ContentOptions>,
    cancelled: &AtomicBool,
) -> io::Result<Option<Accepted>> {
    check_cancel(cancelled)?;
    let (store, key) = super::files(source, content)?;
    if !store.is_reusable(&key)? {
        return Ok(None);
    }
    let Some((state, bytes)) = checkpoint(&store, &key, None)? else {
        return Ok(None);
    };
    if !metadata_prefix(&state, source.replay_ref(), content)?
        || !state.forward_complete(source.replay_ref().index.len())
        || !side::complete(source, content, &store, &key)?
    {
        return Ok(None);
    }
    if !branches_supported(&store, &key, Some(&key), source.replay_ref(), &state)? {
        return Ok(None);
    }
    check_cancel(cancelled)?;
    Ok(Some(Accepted {
        sections: final_sections(&state, source.replay_ref().index.len()),
        bytes,
        next_block: state.cursor.next_block,
        new_blocks: 0,
        coverage: accepted_coverage(&state.map, &last_blocks(source.replay_ref())),
        families: family_coverages(
            &state,
            source.replay_ref(),
            &last_blocks(source.replay_ref()),
        ),
    }))
}

/// Acceptance accounting only; the window and lifecycle come from one walk snapshot.
pub fn walk_window() -> Option<(&'static str, Option<(usize, usize)>)> {
    super::pace::current().map(|snapshot| (snapshot.lifecycle.name(), snapshot.window()))
}

pub fn run(
    source: &Source,
    content: Option<&ContentOptions>,
    cancelled: &AtomicBool,
    accepted: impl FnMut(&Accepted),
) -> io::Result<Accepted> {
    super::driver::prepare(source, content, cancelled, accepted)
}

/// [`run`] on the row backends the driver selected.
pub(super) fn prepare_rows(
    source: &Source,
    content: Option<&ContentOptions>,
    cancelled: &AtomicBool,
    accepted: impl FnMut(&Accepted),
) -> io::Result<Accepted> {
    run_checked(source, content, cancelled, accepted, |_| Ok(()))
}

/// Real preparation timings for the external release measurement harness.
#[derive(Default)]
pub struct PreparationTimings {
    pub hours: usize,
    pub decode: Duration,
    pub observe: Duration,
    pub wall: Duration,
    pub families: Option<BTreeMap<&'static str, FamilyTimings>>,
    pub residual: Duration,
    pub worker_observe: Duration,
    pub worker_lanes: usize,
}

// The normal path takes only the None branch: no clock reads, map or allocation.
// Keep the original expression's value and error propagation at every call site.
macro_rules! time_family {
    ($timings:ident, $family:literal, $phase:ident, $operation:expr) => {{
        // Measurement only: thread CPU by family, when the harness enables it.
        let _cpu = ::replay_frame::cpu::scope(
            if stringify!($phase) == "flush" {
                "commit"
            } else {
                "observe"
            },
            Some($family),
        );
        if $timings.is_some() {
            let started = start_timing(&$timings);
            let result = $operation;
            finish_timing(
                &mut $timings,
                started,
                $family,
                stringify!($phase) == "flush",
            );
            result
        } else {
            $operation
        }
    }};
}

pub mod cost;
pub mod external;
mod flush;
pub(crate) mod independent;
pub use independent::evaluated::occupation_row_reference;
mod lanes;
pub(crate) use lanes::parallelism;
mod production_scans;
mod side;
/// Measurement only: the hourly walk's part of the state census.
mod state_census;

/// One completed manifest commit, emitted before any next block is admitted.
pub struct MeasuredBlock {
    pub ordinal: usize,
    pub first_hour: i32,
    pub last_hour: i32,
    pub hours: usize,
    pub cumulative_hours: u64,
    pub wall: Duration,
    pub decode: Duration,
    pub families: BTreeMap<&'static str, FamilyTimings>,
    pub residual: Duration,
    pub store_bytes: u64,
    pub observe: Duration,
    pub worker_observe: Duration,
}

fn measured_prefix_hours(replay: &Replay, next_block: usize) -> u64 {
    replay.index[..next_block]
        .iter()
        .map(|index| (i64::from(index.last_hour) - i64::from(index.first_hour) + 1).max(0) as u64)
        .sum()
}

struct Measurement<'a> {
    serial: bool,
    limit: Duration,
    started: Instant,
    timings: PreparationTimings,
    cleanup_started: Option<Instant>,
    through_hours: Option<u64>,
    committed: &'a mut dyn FnMut(&MeasuredBlock) -> io::Result<()>,
    prior_wall: Instant,
    prior_decode: Duration,
    prior_hours: usize,
    prior_families: BTreeMap<&'static str, FamilyTimings>,
    prior_lane_families: BTreeMap<&'static str, FamilyTimings>,
    prior_observe: Duration,
    lane_families: Option<BTreeMap<&'static str, FamilyTimings>>,
}

/// Compare the serial and production streams with every production observer.
/// A time box finishes at a common manifest commit, preserving the accepted prefix.
#[cfg(test)]
pub fn run_measured(
    source: &Source,
    content: Option<&ContentOptions>,
    cancelled: &AtomicBool,
    serial: bool,
    limit: Duration,
    accepted: impl FnMut(&Accepted),
) -> io::Result<(Accepted, PreparationTimings)> {
    run_measured_through(
        source,
        content,
        cancelled,
        serial,
        limit,
        None,
        |_| Ok(()),
        accepted,
    )
}

/// Measurement-only prefix stop and durable per-commit evidence callback.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub fn run_measured_through(
    source: &Source,
    content: Option<&ContentOptions>,
    cancelled: &AtomicBool,
    serial: bool,
    limit: Duration,
    through_hours: Option<u64>,
    mut committed: impl FnMut(&MeasuredBlock) -> io::Result<()>,
    accepted: impl FnMut(&Accepted),
) -> io::Result<(Accepted, PreparationTimings)> {
    let started = Instant::now();
    let mut measurement = Measurement {
        serial,
        limit,
        started,
        timings: PreparationTimings::default(),
        cleanup_started: None,
        through_hours,
        committed: &mut committed,
        prior_wall: started,
        prior_decode: Duration::ZERO,
        prior_hours: 0,
        prior_families: BTreeMap::new(),
        prior_lane_families: BTreeMap::new(),
        prior_observe: Duration::ZERO,
        lane_families: None,
    };
    let result = {
        // Release callback captures before recording the post-run timings.
        let mut accepted = accepted;
        run_inner(
            source,
            content,
            cancelled,
            &mut accepted,
            &mut |_| Ok(()),
            Some(&mut measurement),
        )
    }?;
    if let Some(started) = measurement.cleanup_started
        && let Some(families) = measurement.timings.families.as_mut()
    {
        let family = families.entry("preparation.drop").or_default();
        family.flush += started.elapsed();
        family.calls += 1;
    }
    measurement.timings.wall = measurement.started.elapsed();
    measurement.timings.residual = measurement.timings.wall.saturating_sub(
        measurement
            .timings
            .families
            .as_ref()
            .map(family_time)
            .unwrap_or_default(),
    );
    measurement.timings.worker_observe = measurement
        .lane_families
        .as_ref()
        .map(family_time)
        .unwrap_or_default();
    lanes::merge(
        &mut measurement.timings.families,
        measurement.lane_families.take(),
    );
    Ok((result, measurement.timings))
}

fn run_checked(
    source: &Source,
    content: Option<&ContentOptions>,
    cancelled: &AtomicBool,
    mut accepted: impl FnMut(&Accepted),
    mut before_commit: impl FnMut(usize) -> io::Result<()>,
) -> io::Result<Accepted> {
    run_inner(
        source,
        content,
        cancelled,
        &mut accepted,
        &mut before_commit,
        None,
    )
}

fn writer_busy(error: &io::Error) -> bool {
    store::is_writer_busy(error)
}

fn writer_checkpoint_matches(
    writer: &mut store::Writer,
    expected: Option<&[u8]>,
) -> io::Result<bool> {
    let current = dashboard_base::prepared::pages::Pages::read_at(writer, CHECKPOINT, 0)?;
    Ok(current.as_deref() == expected)
}

fn wait_for_writer(
    mut open: impl FnMut() -> io::Result<store::Writer>,
    cancelled: &AtomicBool,
    published: &Accepted,
    accepted: &mut dyn FnMut(&Accepted),
) -> io::Result<store::Writer> {
    let mut waiting = false;
    loop {
        check_cancel(cancelled)?;
        match open() {
            Ok(writer) => {
                if waiting {
                    accepted(published);
                }
                return Ok(writer);
            }
            Err(error) if writer_busy(&error) => {
                if !waiting {
                    let mut status = published.clone();
                    status.sections.push(Section {
                        id: "writer".into(),
                        label: "Waiting for another writer".into(),
                        state: State::Preparing,
                        done: "0".into(),
                        total: None,
                        error: None,
                    });
                    accepted(&status);
                    waiting = true;
                }
                check_cancel(cancelled)?;
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(error) => return Err(error),
        }
    }
}

fn run_inner(
    source: &Source,
    content: Option<&ContentOptions>,
    cancelled: &AtomicBool,
    accepted: &mut dyn FnMut(&Accepted),
    before_commit: &mut dyn FnMut(usize) -> io::Result<()>,
    mut measurement: Option<&mut Measurement<'_>>,
) -> io::Result<Accepted> {
    loop {
        check_cancel(cancelled)?;
        if let Some(result) = run_planned(
            source,
            content,
            cancelled,
            accepted,
            before_commit,
            measurement.as_deref_mut(),
        )? {
            return Ok(result);
        }
        #[cfg(test)]
        if let Some(replans) = TEST_REPLANS.get() {
            TEST_REPLANS.set(Some(replans + 1));
            panic!("Unexpected preparation re-plan");
        }
    }
}

// Compile the preparation engine once, independently of caller callback types.
#[inline(never)]
fn run_planned(
    source: &Source,
    content: Option<&ContentOptions>,
    cancelled: &AtomicBool,
    accepted: &mut dyn FnMut(&Accepted),
    before_commit: &mut dyn FnMut(usize) -> io::Result<()>,
    mut measurement: Option<&mut Measurement<'_>>,
) -> io::Result<Option<Accepted>> {
    let mut family_timings: Option<BTreeMap<&'static str, FamilyTimings>> =
        measurement.is_some().then(BTreeMap::new);
    let mut lane_timings = measurement.is_some().then(BTreeMap::new);
    let mut frame_changes = FrameChangeTracker::new();
    install_gates();
    let setup_started = start_timing(&family_timings);
    check_cancel(cancelled)?;
    let (store, key) = time_family!(
        family_timings,
        "setup.files",
        observe,
        super::files(source, content)
    )?;
    let key = dashboard_base::prepared::refresh_saved_binding(source, &store, key)?;
    let replay = source.replay_ref();
    let shared_replay = source.replay();
    let map_export = time_family!(
        family_timings,
        "map_export.bind",
        observe,
        snow_session(source, content, cancelled)
    )?;
    let mut saved = time_family!(
        family_timings,
        "resume.saved-key",
        observe,
        store.saved_key()
    )?;
    // Reuse compares unchanged input stamps and the recorded index/semantic metadata.
    let reuse = saved.as_ref().is_some_and(|previous| {
        previous == &key
            || previous.recording_size() < key.recording_size()
            || key.renames(previous)
    });
    let planned_checkpoint = if reuse {
        time_family!(
            family_timings,
            "resume.checkpoint",
            observe,
            checkpoint_bytes(&store, &key, saved.as_ref())
        )?
    } else {
        None
    };
    let loaded = parse_checkpoint(planned_checkpoint.as_deref())?;
    let planned_saved = saved.clone();
    let loaded = match loaded {
        Some(pair)
            if time_family!(
                family_timings,
                "resume.prefix",
                observe,
                metadata_prefix(&pair.0, replay, content)
            )? && time_family!(
                family_timings,
                "resume.branches",
                observe,
                branches_supported(&store, &key, saved.as_ref(), replay, &pair.0)
            )? =>
        {
            Some(pair)
        }
        _ => None,
    };
    let owners_started = start_timing(&family_timings);
    let mut land_owners = if loaded
        .as_ref()
        .is_some_and(|(state, _)| state.cursor.next_block > 0)
    {
        let file = match saved.as_ref() {
            Some(previous) if previous != &key => time_family!(
                family_timings,
                "resume.observer-state",
                observe,
                store.read_prefix(previous, &key, "land-owners", 0)
            )?,
            _ => time_family!(
                family_timings,
                "resume.observer-state",
                observe,
                store.read(&key, "land-owners", 0)
            )?,
        };
        file.map(crate::products::land_owners::Scan::read)
            .transpose()?
    } else {
        Some(time_family!(
            family_timings,
            "construct.land_owners",
            observe,
            crate::products::land_owners::Scan::new(replay)
        ))
    };
    finish_timing(
        &mut family_timings,
        owners_started,
        "resume.land_owners",
        false,
    );
    let snow_unstarted = match loaded.as_ref() {
        Some((state, _)) if !state.accepted_walk() => !map_export.has_history(
            &store,
            &key,
            replay.branches.iter().map(|branch| branch.id),
            || snow_check(cancelled, "validate"),
        )?,
        _ => false,
    };
    let snow_frontiers: Vec<_> = replay
        .branches
        .iter()
        .map(|branch| {
            let cursor = loaded
                .as_ref()
                .and_then(|(state, _)| state.map.branches.get(&branch.id))
                .and_then(|progress| {
                    progress
                        .last_hour
                        .map(|hour| dashboard_base::map_export::Cursor {
                            hour,
                            block: progress.last_block.map(|block| block as u64),
                        })
                });
            (branch.id, cursor)
        })
        .collect();
    let snow_bound = !snow_unstarted
        && map_export.reusable(&store, &key, snow_frontiers.iter().cloned(), || {
            snow_check(cancelled, "validate")
        })?;
    let loaded = match loaded {
        Some(pair)
            if land_owners.is_some()
                && (snow_unstarted
                    || snow_bound
                    || match saved.as_ref() {
                        Some(_) => map_export.can_inherit(
                            &store,
                            &key,
                            snow_frontiers.iter().cloned(),
                            || snow_check(cancelled, "validate"),
                        )?,
                        None => false,
                    }) =>
        {
            Some(pair)
        }
        _ => None,
    };
    let state_started = start_timing(&family_timings);
    let (mut state, mut bytes, mut stream) = if let Some((state, bytes)) = loaded {
        if state.forward_complete(replay.index.len())
            && saved.as_ref() == Some(&key)
            && snow_bound
            && side::complete(source, content, &store, &key)?
        {
            let result = Accepted {
                sections: time_family!(
                    family_timings,
                    "progress.sections",
                    observe,
                    final_sections(&state, replay.index.len())
                ),
                bytes,
                next_block: state.cursor.next_block,
                new_blocks: 0,
                coverage: time_family!(
                    family_timings,
                    "progress.coverage",
                    observe,
                    accepted_coverage(
                        &state.map,
                        &time_family!(
                            family_timings,
                            "setup.block-index",
                            observe,
                            last_blocks(replay)
                        )
                    )
                ),
                families: family_coverages(&state, replay, &last_blocks(replay)),
            };
            time_family!(
                family_timings,
                "progress.accepted",
                observe,
                accepted(&result)
            );
            finish_timing(&mut family_timings, state_started, "setup.state", false);
            finish_timing(&mut family_timings, setup_started, "setup.overhead", false);
            if let Some(measurement) = measurement {
                measurement.timings.families = family_timings;
                measurement.cleanup_started = Some(Instant::now());
            }
            return Ok(Some(result));
        }
        let resume = match saved.as_ref() {
            Some(previous) if previous != &key => time_family!(
                family_timings,
                "resume.frame",
                observe,
                dashboard_base::prepared_map::load_resume_prefix(
                    &store,
                    previous,
                    &key,
                    &state.map,
                    selected(),
                )
            )?,
            _ => time_family!(
                family_timings,
                "resume.frame",
                observe,
                dashboard_base::prepared_map::load_resume(&store, &key, &state.map, selected())
            )?,
        };
        let accumulator = resume
            .map(|(frame, position)| {
                if position.block.checked_add(1) != Some(state.cursor.next_block) {
                    return Err(invalid("Prepared frame/cursor mismatch"));
                }
                Ok(frame)
            })
            .transpose()?;
        if state.cursor.next_block != 0 && accumulator.is_none() {
            return Err(invalid("Prepared decoder accumulator is missing"));
        }
        let stream = time_family!(
            family_timings,
            "resume.decoder",
            observe,
            ReplayStream::resume(selected(), state.cursor.replay(), accumulator)
        )?;
        (state, bytes, stream)
    } else {
        saved = None;
        (
            zero_checkpoint(replay, content)?,
            0,
            time_family!(
                family_timings,
                "construct.decoder",
                observe,
                ReplayStream::new(selected())
            )?,
        )
    };
    finish_timing(&mut family_timings, state_started, "setup.state", false);
    let mut land_owners = if saved.is_some() {
        land_owners.take().unwrap_or_else(|| {
            time_family!(
                family_timings,
                "construct.land_owners",
                observe,
                crate::products::land_owners::Scan::new(replay)
            )
        })
    } else {
        time_family!(
            family_timings,
            "construct.land_owners",
            observe,
            crate::products::land_owners::Scan::new(replay)
        )
    };
    land_owners.incomplete |= !replay.gaps.is_empty();
    let land_names = time_family!(
        family_timings,
        "content.land_names",
        observe,
        crate::content::name_localisation(source, content)
    );
    if let Err(reason) = &land_names {
        land_owners.warning(reason.clone());
    }
    let country_started = start_timing(&family_timings);
    let preferred = time_family!(
        family_timings,
        "setup.metadata",
        observe,
        source.with_metadata(|metadata| {
            metadata
                .player_country_id
                .as_ref()
                .and_then(|player| {
                    metadata
                        .countries
                        .iter()
                        .find(|country| &country.id == player)
                })
                .map(|country| country.tag.clone())
        })
    );
    finish_timing(&mut family_timings, country_started, "setup.country", false);
    let context = time_family!(
        family_timings,
        "context.construct",
        observe,
        LoadContext {
            content: content.cloned(),
        }
    );
    let naval_content = time_family!(
        family_timings,
        "content.naval",
        observe,
        crate::products::ships_sunk::prepared_content(source, &context)
    );
    let ledger_content = time_family!(
        family_timings,
        "content.politics_ledger",
        observe,
        crate::products::politics_ledger::prepared::prepared_content(source, content)
    );
    let raid_content = time_family!(
        family_timings,
        "content.raids",
        observe,
        crate::products::raids::prepared_content(source, &context)
    )
    .ok()
    .and_then(|(content, _)| content);

    let idea_content = time_family!(
        family_timings,
        "content.ideas",
        observe,
        context.content().and_then(
            |options| crate::products::politics_content::politics_content(source, options)
        )
    );
    if let Err(error) = &idea_content
        && error.kind() == io::ErrorKind::Interrupted
    {
        return Err(io::Error::new(error.kind(), error.to_string()));
    }

    let casualty_content = time_family!(
        family_timings,
        "content.casualties",
        observe,
        crate::products::war_casualties::prepared::prepared_content(source, &context)
    )?;
    let naming = time_family!(
        family_timings,
        "content.production",
        observe,
        ProductionNaming::load(source, &context)
    );
    let stockpile_content = time_family!(
        family_timings,
        "content.stockpile",
        observe,
        crate::products::equipment_stockpile::prepared::Prepared::content(source, content)
    );

    let intelligence_content = time_family!(
        family_timings,
        "content.intelligence",
        observe,
        crate::products::intelligence_prepared::Content::load(source, &context)
    )?;
    let score_naming = time_family!(
        family_timings,
        "content.game_score",
        observe,
        crate::products::game_score::Naming::load(source, &context)
    );
    let mut score_observer = time_family!(
        family_timings,
        "construct.game_score",
        observe,
        score_naming.observer(replay)
    );
    let leader_catalogue = context.content().and_then(|options| {
        time_family!(
            family_timings,
            "content.catalogue",
            observe,
            crate::catalogue::catalogue(source, options)
        )
    });
    let mut leader_content_errors: Vec<_> = leader_catalogue
        .as_ref()
        .err()
        .map(|error| format!("Leader content unavailable: {error}"))
        .into_iter()
        .collect();
    let leader_content = time_family!(
        family_timings,
        "content.leaders",
        observe,
        crate::products::unit_leaders::Content::for_catalogue(
            source,
            &context,
            leader_catalogue.ok(),
        )
    );
    if let Err(error) = &leader_content {
        leader_content_errors.push(format!("Leader content unavailable: {error}"));
    }
    let leader_content = leader_content.ok().flatten();
    let mut evaluator = time_family!(family_timings, "focus.construct", observe, FocusEvaluator);
    if let Some(content) = content
        && let Err(error) = time_family!(
            family_timings,
            "focus.configure",
            observe,
            evaluator.configure(source, content)
        )
        && error.kind() == io::ErrorKind::Interrupted
    {
        return Err(error);
    }
    let mut map = time_family!(
        family_timings,
        "construct.map",
        observe,
        dashboard_base::prepared_map::Builder::new(
            NonZeroU32::new(dashboard_base::prepared_map::CHECKPOINT_STRIDE)
                .ok_or_else(|| invalid("zero checkpoint stride"))?,
            dashboard_base::prepared_map::selected_tables(),
            state.map.clone(),
        )
    )?;
    time_family!(
        family_timings,
        "setup.map-ranges",
        observe,
        map.set_ranges(replay)
    );
    let last_blocks = time_family!(
        family_timings,
        "setup.block-index",
        observe,
        last_blocks(replay)
    );
    // Window replies are named by each block's position within its own
    // branch, and batches end by it.
    let ordinals_started = start_timing(&family_timings);
    let batches = Batches::new(replay);
    let ordinals = &batches.ordinals;
    finish_timing(
        &mut family_timings,
        ordinals_started,
        "setup.ordinals",
        false,
    );
    let mut new_blocks = 0;
    let mut published = Accepted {
        sections: time_family!(
            family_timings,
            "progress.sections",
            observe,
            final_sections(&state, replay.index.len())
        ),
        bytes,
        next_block: state.cursor.next_block,
        new_blocks,
        coverage: time_family!(
            family_timings,
            "progress.coverage",
            observe,
            accepted_coverage(&state.map, &last_blocks)
        ),
        families: family_coverages(&state, replay, &last_blocks),
    };
    if (state.forward_complete(replay.index.len())
        && saved.as_ref() == Some(&key)
        && snow_bound
        && side::complete(source, content, &store, &key)?)
        || (snow_bound
            && state
                .independent
                .values()
                .all(|family| family.next_block == replay.index.len())
            && measurement.as_ref().is_some_and(|value| {
                value.through_hours.is_some_and(|target| {
                    measured_prefix_hours(replay, state.cursor.next_block) >= target
                })
            }))
    {
        published.sections = time_family!(
            family_timings,
            "progress.sections",
            observe,
            final_sections(&state, replay.index.len())
        );
        time_family!(
            family_timings,
            "progress.accepted",
            observe,
            accepted(&published)
        );
        finish_timing(&mut family_timings, setup_started, "setup.overhead", false);
        if let Some(measurement) = measurement {
            measurement.timings.families = family_timings;
            measurement.cleanup_started = Some(Instant::now());
        }
        return Ok(Some(published));
    }
    let mut metadata_dirty = true;
    let opened = time_family!(
        family_timings,
        "publication.open",
        observe,
        wait_for_writer(
            || match saved.as_ref() {
                Some(previous) if previous != &key => store.append(previous, key.clone()),
                Some(_) => store.extend(&key),
                None => store.replace(key.clone()),
            },
            cancelled,
            &published,
            accepted,
        )
    );
    let mut writer = match opened {
        Ok(writer) => writer,
        Err(error) => {
            let current = store.saved_key()?;
            if current != planned_saved {
                return Ok(None);
            }
            let key = dashboard_base::prepared::refresh_saved_binding(source, &store, key.clone())?;
            let current_path = std::fs::canonicalize(source.path())?;
            if key.refresh_recording(&current_path)? != key
                || (current.as_ref() == Some(&key) && !store.is_reusable(&key)?)
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Recording changed; reopen it before preparing the current content",
                ));
            }
            if reuse && checkpoint_bytes(&store, &key, current.as_ref())? != planned_checkpoint {
                return Ok(None);
            }
            return Err(error);
        }
    };
    let current_saved = store.saved_key()?;
    if current_saved != planned_saved {
        return Ok(None);
    }
    let key = dashboard_base::prepared::refresh_saved_binding(source, &store, key)?;
    let current_path = std::fs::canonicalize(source.path())?;
    if key.refresh_recording(&current_path)? != key
        || (current_saved.as_ref() == Some(&key) && !store.is_reusable(&key)?)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Recording changed; reopen it before preparing the current content",
        ));
    }
    let current_checkpoint = if reuse {
        checkpoint_bytes(&store, &key, current_saved.as_ref())?
    } else {
        None
    };
    if current_checkpoint != planned_checkpoint {
        return Ok(None);
    }
    if metadata_dirty {
        if snow_unstarted {
            map_export.initialise(&mut writer)?;
        } else {
            map_export.inherit(&mut writer)?;
        }
        time_family!(
            family_timings,
            "publication.metadata",
            flush,
            crate::source::metadata_cache::write(&mut writer, source)
        )?;
        time_family!(
            family_timings,
            "map",
            flush,
            map.save_gaps(&mut writer, &replay.gaps)
        )?;
        for &branch in map.progress().branches.keys() {
            time_family!(
                family_timings,
                "coverage",
                flush,
                save_coverage(
                    &mut writer,
                    map.progress(),
                    branch,
                    &last_blocks,
                    state.refused.get(&branch)
                )
            )?;
        }
    }
    if saved.as_ref().is_some_and(|previous| previous != &key) {
        // Bind the accepted prefix to the grown recording before observing any
        // frame, so preparation reads its earlier ranges with current-key reads.
        bytes = time_family!(
            family_timings,
            "checkpoint",
            flush,
            write_checkpoint(&mut writer, &mut state, replay, &map, bytes)
        )?;
        check_cancel(cancelled)?;
        let committed_checkpoint =
            dashboard_base::prepared::pages::Pages::read_at(&mut writer, CHECKPOINT, 0)?;
        time_family!(family_timings, "store_commit", flush, writer.commit())?;
        metadata_dirty = false;
        saved = Some(key.clone());
        writer = wait_for_writer(|| store.extend(&key), cancelled, &published, accepted)?;
        if !writer_checkpoint_matches(&mut writer, committed_checkpoint.as_deref())? {
            return Ok(None);
        }
    }
    let mut accepted = accepted;
    // The walk and the families publish through the same writer. Their
    // checkpoints merge only accepted progress, and keep their sections disjoint.
    state.map = map.progress().clone();
    let ranges = state.map.ranges.clone();
    let (walk_pace, walk_ended) =
        super::pace::WalkPace::start(stream.cursor().next_block, replay.index.len());
    let phase = independent::Phase {
        source,
        replay,
        store: &store,
        key: &key,
        context: &context,
        ranges: &ranges,
        last_blocks: &last_blocks,
        batches: &batches,
        measured: measurement.is_some(),
        cancelled,
        walk_pace: &walk_pace,
    };
    let mut side = side::load(&store, &key, saved.as_ref())?.unwrap_or_default();
    let beside = production_scans::jobs(&phase, saved.is_some())?;
    side.pending = beside.iter().map(|job| job.name.to_owned()).collect();
    side::write(&mut writer, &side)?;
    if state.forward_complete(replay.index.len()) {
        // With no forward work left, make the roster durable even if cancellation
        // precedes the side publishers' first commit (or there are no jobs).
        bytes = write_checkpoint_state(&mut writer, &mut state, replay, bytes)?;
        check_cancel(cancelled)?;
        let committed_checkpoint =
            dashboard_base::prepared::pages::Pages::read_at(&mut writer, CHECKPOINT, 0)?;
        writer.commit()?;
        metadata_dirty = false;
        saved = Some(key.clone());
        writer = wait_for_writer(|| store.extend(&key), cancelled, &published, &mut accepted)?;
        if !writer_checkpoint_matches(&mut writer, committed_checkpoint.as_deref())? {
            return Ok(None);
        }
    }
    let mut committed_state = state.clone();
    let mut committed_saved = saved.clone();
    let mut committed_bytes = bytes;
    let mut independent_timings = family_timings.as_ref().map(|_| BTreeMap::new());
    let mut independent_workers = lane_timings.as_ref().map(|_| BTreeMap::new());
    finish_timing(&mut family_timings, setup_started, "setup.overhead", false);
    let walk = |walk: &mut independent::Walk<'_, '_>,
                accepted: &mut dyn FnMut(&Accepted)|
     -> io::Result<()> {
        let _walk_ended = walk_ended;
        if stream.cursor().next_block == replay.index.len() && !metadata_dirty {
            return Ok(());
        }
        let mut branch = replay
            .index
            .get(stream.cursor().next_block)
            .map(|index| {
                time_family!(
                    family_timings,
                    "branch.restore-or-construct",
                    observe,
                    load_branch(
                        &store,
                        &key,
                        saved.as_ref(),
                        replay,
                        index.branch,
                        index.first_hour,
                        state.refused.entry(index.branch).or_default(),
                    )
                )
            })
            .transpose()?;
        if let (Some(domain), Some(frame)) = (&mut branch, stream.accumulator()) {
            let next = &replay.index[stream.cursor().next_block];
            time_family!(
                family_timings,
                "resume.observers",
                observe,
                domain.resume(
                    frame,
                    next.branch,
                    state.refused.entry(next.branch).or_default()
                )
            );
            frame_changes.invalidate();
        }
        let sparse_map = selected().len() != TableId::ALL.len();
        let mut stream_tables = selected().to_vec();
        // The last flushed branch and its state, which the flush saved: the next
        // block of that branch continues it rather than reading it back.
        let mut kept: Option<(u64, Box<BranchState>)> = None;
        if state.cursor.next_block == replay.index.len() && saved.as_ref() == Some(&key) {
            return Ok(());
        }
        let section_consumer =
            replay.section_reader(stream.cursor().next_block, selected(), "walk");
        let mut writer = walk.open(accepted)?;
        let prepass = prepass::enabled().then(|| {
            walk.prepass().unwrap_or_else(|| {
                #[allow(unused_mut)]
                let mut knobs = prepass::Knobs::current();
                #[cfg(test)]
                {
                    knobs.refusals = lanes::test_refusals();
                }
                PrePass::start(
                    source.replay(),
                    stream.cursor().next_block,
                    prepass::families(),
                    prepass::workers(lanes::parallelism()),
                    knobs,
                )
            })
        });
        loop {
            let block_started = start_timing(&family_timings);
            check_cancel(cancelled)?;
            let block = stream.cursor().next_block;
            let _cpu_block = ::replay_frame::cpu::block(Some(block));
            section_consumer.at(block);
            walk_pace.at(block, replay.index.len());
            if block == replay.index.len() && !metadata_dirty {
                finish_timing(&mut family_timings, block_started, "block.overhead", false);
                break;
            }
            let mut frame_count = 0usize;
            if let Some(domain) = &mut branch {
                let refused = state.refused.entry(replay.index[block].branch).or_default();
                if refused.live("politics_ledger") {
                    domain.ledger.configure(&ledger_content);
                    domain.ledger.decode_previous_from(source.replay());
                }
                if refused.live("territory") {
                    time_family!(
                        family_timings,
                        "territory.configure",
                        observe,
                        domain.territory.configure(source, &context, cancelled)
                    )?;
                }
                if refused.live("occupation") {
                    time_family!(
                        family_timings,
                        "occupation.configure",
                        observe,
                        domain.occupation.configure(source, &context)
                    );
                }
                if refused.live("supply") {
                    time_family!(
                        family_timings,
                        "supply.configure",
                        observe,
                        domain.supply.configure(source, content)
                    )?;
                }
                if let Some(index) = replay.index.get(block) {
                    if refused.live("naval") {
                        time_family!(
                            family_timings,
                            "naval.configure",
                            observe,
                            domain.naval.configure(source, index.branch, &naval_content)
                        )?;
                    }
                    if refused.live("projects") {
                        domain.projects.configure(source, &context)?;
                    }
                    if refused.live("casualties") {
                        domain.casualties.configure(&casualty_content);
                    }
                }
                let force_context = LoadContext {
                    content: content.cloned(),
                };
                if refused.live("air") {
                    domain.air.configure(source, &force_context);
                }
                if refused.live("navy") {
                    domain.navy.configure(source, &force_context);
                }
                // Air/navy resume and construction restore below share this boundary.
                // No frame is observed between them, so one invalidation covers all.
                if stream.accumulator().is_some() {
                    frame_changes.invalidate();
                }
                if refused.live("groups") {
                    time_family!(
                        family_timings,
                        "groups",
                        observe,
                        domain.groups.begin_block()
                    );
                }
                if let Some(frame) = stream.accumulator()
                    && replay
                        .index
                        .get(block)
                        .is_some_and(|index| index.branch == frame.branch)
                {
                    if refused.live("air") {
                        time_family!(
                            family_timings,
                            "air",
                            observe,
                            domain.air.resume(frame, replay, cancelled)
                        )?;
                    }
                    if refused.live("navy") {
                        time_family!(
                            family_timings,
                            "navy",
                            observe,
                            domain.navy.resume(frame, replay, cancelled)
                        )?;
                    }
                }
                if let Some(frame) = stream.accumulator()
                    && refused.live("construction")
                {
                    time_family!(
                        family_timings,
                        "construction",
                        observe,
                        domain.construction.restore_frame(frame)
                    );
                }
            }
            let measured = measurement.is_some();
            // A family prepared from blocks observes this block hour by hour only
            // when its extract refused or failed; otherwise its facts fold after
            // the block's observation.
            let mut folds = Vec::new();
            if let Some(prepass) = &prepass
                && block < replay.index.len()
            {
                let extracted = time_family!(
                    family_timings,
                    "prepass.wait",
                    observe,
                    prepass.take(block, cancelled)
                )?;
                let mut extract_timings = measured.then(BTreeMap::new);
                for (&family, (outcome, took)) in prepass.families().iter().zip(extracted) {
                    if let Some(timings) = &mut extract_timings {
                        let entry: &mut FamilyTimings = timings.entry(family.extract).or_default();
                        entry.observe += took;
                        entry.calls += 1;
                    }
                    match outcome {
                        Outcome::Facts(facts) => folds.push((family, facts)),
                        Outcome::Failed(error) => return Err(error),
                        Outcome::Sequential | Outcome::Declined(_) => {
                            let counted = start_timing(&family_timings);
                            finish_timing(&mut family_timings, counted, family.sequential, false);
                            prepass::note(family.sequential);
                        }
                    }
                }
                lanes::merge(&mut lane_timings, extract_timings);
            }
            // The walk is one downstream chain: decoder, map/land observation,
            // folds and block-end pages. Pre-pass waits are not its measured cost.
            let walk_work_started = Instant::now();
            let hourly = |name: &str| !folds.iter().any(|(family, _)| family.name == name);
            let (resources_hourly, army_hourly) = (hourly("resources"), hourly("army"));
            let block_tables = block_selected(resources_hourly, army_hourly);
            section_consumer.declare(replay, &block_tables);
            if block_tables != stream_tables {
                stream = ReplayStream::resume(
                    &block_tables,
                    stream.cursor().clone(),
                    stream.accumulator().cloned(),
                )?;
                stream_tables = block_tables;
            }
            let serial =
                measurement.as_ref().is_some_and(|value| value.serial) || lanes::parallelism() <= 2;
            let decoding_started = measured.then(Instant::now);
            let mut observed = Duration::ZERO;
            let mut progress_output = Duration::ZERO;
            let stream_started = start_timing(&family_timings);
            let got = if branch.is_none() {
                stream.next_block_recorded_with(
                    replay,
                    |_, _| Err(invalid("Missing prepared branch accumulator")),
                    || check_cancel(cancelled),
                )?
            } else {
                let refused = state.refused.entry(replay.index[block].branch).or_default();
                std::thread::scope(|scope| -> io::Result<bool> {
                    let BranchState {
                        focus,
                        index,
                        production,
                        measure,
                        land,
                        naval,
                        divisions,
                        identities,
                        army,
                        air,
                        navy,
                        nukes,
                        factories,
                        repairs,
                        supply,
                        construction,
                        resources,
                        trade,
                        stockpile,
                        groups,
                        group_changes,
                        air_ledger,
                        aces,
                        raids,
                        air_superiority,
                        subjects,
                        subject_changes,
                        subject_late,
                        autonomy,
                        tension,
                        wars,
                        ideas,
                        idea_changes,
                        feed,
                        parties,
                        intelligence,
                        stability,
                        experience,
                        ledger,
                        fuel,
                        territory,
                        occupation,
                        game_score,
                        leaders,
                        research,
                        peace,
                        projects,
                        casualties,
                        equipment_losses,
                        ..
                    } = branch
                        .as_deref_mut()
                        .ok_or_else(|| invalid("Missing prepared branch accumulator"))?;
                    let score_observer = &mut score_observer;
                    let score_naming = &score_naming;
                    let load_context = &context;
                    let leader_content = leader_content.as_ref();
                    let stockpile_content = &stockpile_content;
                    let raid_content = raid_content.as_deref();
                    let idea_content = idea_content.as_deref().ok();
                    let intelligence_content = &intelligence_content;
                    let (prepared_store, prepared_key) = (&store, &key);
                    let mut jobs = vec![
                        lanes::Job::new(
                            &[("nukes", 4)],
                            521,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "nukes",
                                    observe,
                                    nukes.observe(frame, replay, changes)
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("factories", 5)],
                            4210,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "factories",
                                    observe,
                                    factories.observe(frame, replay, changes)
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("projects", 6)],
                            2100,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "projects",
                                    observe,
                                    projects.observe_changed(frame, changes, cancelled)
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("peace", 7)],
                            375,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "peace",
                                    observe,
                                    peace.observe_changed(frame, changes)
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("research", 8)],
                            1023,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "research",
                                    observe,
                                    research.observe_changed(frame, changes)
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("construction", 9)],
                            500,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "construction",
                                    observe,
                                    construction.observe_changed(
                                        frame,
                                        replay.format,
                                        source,
                                        changes
                                    )
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("trade", 11)],
                            792,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "trade",
                                    observe,
                                    trade.observe_changed(frame, changes)
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("measure", 12)],
                            426,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "measure",
                                    observe,
                                    measure.observe_changes(frame, changes)
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("navy", 15)],
                            2200,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "navy",
                                    observe,
                                    navy.observe(
                                        frame,
                                        replay,
                                        source,
                                        &time_family!(
                                            family_timings,
                                            "context.construct",
                                            observe,
                                            LoadContext {
                                                content: content.cloned(),
                                            }
                                        ),
                                        cancelled,
                                        changes,
                                    )
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("air", 16)],
                            3700,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "air",
                                    observe,
                                    air.observe(
                                        frame,
                                        replay,
                                        source,
                                        &time_family!(
                                            family_timings,
                                            "context.construct",
                                            observe,
                                            LoadContext {
                                                content: content.cloned(),
                                            }
                                        ),
                                        cancelled,
                                        changes,
                                    )
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("supply", 17)],
                            500,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "supply",
                                    observe,
                                    supply.observe(frame, changes)
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("stockpile", 19)],
                            4300,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "stockpile",
                                    observe,
                                    stockpile.bind_content(stockpile_content)
                                );
                                time_family!(
                                    family_timings,
                                    "stockpile",
                                    observe,
                                    stockpile.observe_changed(replay.format, frame, changes)
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("air_ledger", 21)],
                            266,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "air_ledger",
                                    observe,
                                    air_ledger.observe_changed(frame, changes)
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("aces", 22)],
                            860,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "aces",
                                    observe,
                                    aces.observe_changed(frame, replay, changes)
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("raids", 24)],
                            700,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "raids",
                                    observe,
                                    raids.observe_changed(
                                        frame,
                                        replay.format,
                                        raid_content,
                                        changes
                                    )
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("game_score", 25)],
                            1433,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "game_score",
                                    observe,
                                    game_score.observe(
                                        frame,
                                        replay,
                                        score_observer,
                                        score_naming,
                                        &mut || score_naming
                                            .content_multipliers(source, load_context),
                                        changes
                                    )
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("land", 27)],
                            2700,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "land",
                                    observe,
                                    land.configure(source, load_context)
                                );
                                time_family!(
                                    family_timings,
                                    "land",
                                    observe,
                                    land.observe_prepared(frame, changes)
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("divisions", 28)],
                            668,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "divisions",
                                    observe,
                                    divisions.configure(replay.format)
                                );
                                time_family!(
                                    family_timings,
                                    "divisions",
                                    observe,
                                    divisions.prepare_changed(frame, changes)
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("naval", 31)],
                            1500,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "naval.observe",
                                    observe,
                                    naval.observe_prepared(frame, changes)
                                );
                                time_family!(
                                    family_timings,
                                    "naval.observe_details",
                                    observe,
                                    naval.observe_details(
                                        source,
                                        load_context,
                                        frame,
                                        cancelled,
                                        changes
                                    )
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("subjects", 32)],
                            25,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "subjects",
                                    observe,
                                    subjects.prepare_changes_changed(
                                        frame,
                                        changes,
                                        subject_changes,
                                        subject_late
                                    )
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("autonomy", 33)],
                            63,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "autonomy",
                                    observe,
                                    autonomy.prepare_changed(frame, changes)
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("tension", 34)],
                            119,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "tension",
                                    observe,
                                    tension.prepare_changed(replay.format, frame, changes)
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("wars", 35)],
                            747,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "wars",
                                    observe,
                                    wars.observe_changed(frame, changes)
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("ideas", 36)],
                            500,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "ideas",
                                    observe,
                                    ideas.prepare_changes_in(
                                        frame,
                                        changes,
                                        idea_changes,
                                        idea_content
                                    )
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("parties", 37)],
                            1793,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "parties",
                                    observe,
                                    parties.observe_changes(frame, changes)
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("leaders", 38)],
                            4600,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "leaders",
                                    observe,
                                    leaders.observe(frame, replay, leader_content, changes)
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("intelligence", 39)],
                            600,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "intelligence",
                                    observe,
                                    intelligence.observe(
                                        source,
                                        intelligence_content,
                                        frame,
                                        changes,
                                        prepared_store,
                                        prepared_key,
                                        cancelled
                                    )
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("stability", 40)],
                            5048,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "stability",
                                    observe,
                                    stability.observe_timed(frame, changes, family_timings)
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("experience", 41)],
                            500,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "experience",
                                    observe,
                                    experience.observe_changes(frame, replay.format, changes)
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("politics_ledger", 42)],
                            7100,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "politics_ledger",
                                    observe,
                                    ledger.observe(frame, changes, cancelled)
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("fuel", 43)],
                            1905,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "fuel",
                                    observe,
                                    fuel.observe_changes(frame, replay.format, changes, cancelled)
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("occupation", 45)],
                            500,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "occupation",
                                    observe,
                                    occupation.observe(frame, replay.format, cancelled, changes)
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("casualties", 46)],
                            700,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "casualties",
                                    observe,
                                    casualties.observe_changes(frame, replay, cancelled, changes)
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("equipment_losses", 47)],
                            2634,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "equipment_losses",
                                    observe,
                                    equipment_losses
                                        .observe_changes(frame, replay, cancelled, changes)
                                )?;
                                Ok(())
                            },
                        ),
                        lanes::Job::new(
                            &[("repairs", 48)],
                            521,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "repairs",
                                    observe,
                                    repairs.observe(frame, replay, changes)
                                )?;
                                Ok(())
                            },
                        ),
                    ];
                    // A family whose block facts fold after the block is not
                    // observed hour by hour.
                    if resources_hourly {
                        jobs.push(lanes::Job::new(
                            &[("resources", 10)],
                            6905,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "resources",
                                    observe,
                                    resources.observe_changed(frame, changes)
                                )?;
                                Ok(())
                            },
                        ));
                    }
                    if army_hourly {
                        jobs.push(lanes::Job::new(
                            &[("army", 14)],
                            2070,
                            move |_family, frame, changes, mut family_timings| {
                                time_family!(
                                    family_timings,
                                    "army",
                                    observe,
                                    army.observe(frame, replay, changes)
                                )?;
                                Ok(())
                            },
                        ));
                    }
                    let mut lanes = lanes::Lanes::start(
                        scope,
                        jobs,
                        if serial { 0 } else { lanes::parallelism() },
                        measured,
                        refused,
                    );
                    if let Some(value) = &mut measurement {
                        value.timings.worker_lanes = lanes.count();
                    }
                    let mut observe = |updates: replay_frame::frame::RecordedFrame,
                                       frame: &DecodedFrame| {
                        let frame_started = start_timing(&family_timings);
                        let observing_started = measured.then(Instant::now);
                        check_cancel(cancelled)?;
                        let updates = std::sync::Arc::new(updates);
                        let changes: FrameChanges = frame_changes.observe_updates(
                            frame,
                            std::sync::Arc::clone(&updates),
                            replay.format,
                        );
                        let dispatched = lanes.begin(frame, changes)?;
                        let changes = dispatched.changes();
                        #[cfg(test)]
                        lanes.check_coordinator(frame, changes)?;
                        // A coordinator call's order is above every lane call before
                        // it and at most every lane call after it. `finish` keeps the
                        // coordinator's error on a tie, so the frame reports the first
                        // failing call in serial order.
                        let mut coordinator_order = 0;
                        let coordinated = (|| -> io::Result<()> {
                            let (failures, error) = family!(
                                refused,
                                "focus",
                                frame.hour,
                                time_family!(
                                    family_timings,
                                    "focus.write_frame",
                                    observe,
                                    evaluator.write_frame(
                                        &mut writer,
                                        source,
                                        content,
                                        frame,
                                        focus,
                                        index,
                                        preferred.as_deref(),
                                        cancelled,
                                        changes,
                                    )
                                )
                            );
                            state.display_failures =
                                state
                                    .display_failures
                                    .checked_add(failures)
                                    .ok_or_else(|| invalid("focus refusal count overflow"))?;
                            if state.display_error.is_none() {
                                state.display_error = error;
                            }
                            family!(
                                refused,
                                "focus",
                                frame.hour,
                                time_family!(
                                    family_timings,
                                    "focus.completions",
                                    observe,
                                    index.observe_completions(frame, source, &context, cancelled)
                                )
                            );
                            coordinator_order = 1;
                            if let Some(mut page) = family!(
                                refused,
                                "production",
                                frame.hour,
                                time_family!(
                                    family_timings,
                                    "production",
                                    observe,
                                    production.ingest_changed(
                                        replay.format,
                                        frame,
                                        naming.named(),
                                        cancelled,
                                        changes
                                    )
                                )
                            ) {
                                page.warnings.extend(naming.warnings.iter().cloned());
                                coordinator_order = 2;
                                family!(
                                    refused,
                                    "production",
                                    frame.hour,
                                    time_family!(
                                        family_timings,
                                        "production",
                                        flush,
                                        crate::prepared_production::publish_endpoint(
                                            &mut writer,
                                            production,
                                            &page,
                                        )
                                    )
                                );
                            }
                            coordinator_order = 3;
                            time_family!(
                                family_timings,
                                "land_owners",
                                observe,
                                land_owners.observe(
                                    frame,
                                    land_names.as_deref().ok(),
                                    Some(changes)
                                )
                            )?;
                            lanes.serial("nukes", frame, changes, &mut family_timings)?;
                            lanes.serial("factories", frame, changes, &mut family_timings)?;
                            lanes.serial("projects", frame, changes, &mut family_timings)?;
                            lanes.serial("peace", frame, changes, &mut family_timings)?;
                            lanes.serial("research", frame, changes, &mut family_timings)?;
                            lanes.serial("construction", frame, changes, &mut family_timings)?;
                            if resources_hourly {
                                lanes.serial("resources", frame, changes, &mut family_timings)?;
                            }
                            lanes.serial("trade", frame, changes, &mut family_timings)?;
                            lanes.serial("measure", frame, changes, &mut family_timings)?;
                            coordinator_order = 13;
                            family!(
                                refused,
                                "identities",
                                frame.hour,
                                time_family!(
                                    family_timings,
                                    "territory",
                                    observe,
                                    territory.observe_owned_states(
                                        frame,
                                        identities,
                                        Some(changes)
                                    )
                                )
                            );
                            if army_hourly {
                                lanes.serial("army", frame, changes, &mut family_timings)?;
                            }
                            lanes.serial("navy", frame, changes, &mut family_timings)?;
                            lanes.serial("air", frame, changes, &mut family_timings)?;
                            lanes.serial("supply", frame, changes, &mut family_timings)?;
                            lanes.serial("stockpile", frame, changes, &mut family_timings)?;
                            coordinator_order = 20;
                            family!(
                                refused,
                                "groups",
                                frame.hour,
                                time_family!(
                                    family_timings,
                                    "groups",
                                    observe,
                                    group_changes
                                        .observe_changed(groups, frame, changes, source, &context)
                                )
                            );
                            lanes.serial("air_ledger", frame, changes, &mut family_timings)?;
                            lanes.serial("aces", frame, changes, &mut family_timings)?;
                            coordinator_order = 23;
                            family!(
                                refused,
                                "air_superiority",
                                frame.hour,
                                time_family!(
                                    family_timings,
                                    "air_superiority",
                                    observe,
                                    air_superiority.observe_changed(frame, replay, groups, changes)
                                )
                            );
                            lanes.serial("raids", frame, changes, &mut family_timings)?;
                            lanes.serial("game_score", frame, changes, &mut family_timings)?;
                            lanes.serial("land", frame, changes, &mut family_timings)?;
                            lanes.serial("divisions", frame, changes, &mut family_timings)?;
                            lanes.serial("naval", frame, changes, &mut family_timings)?;
                            lanes.serial("subjects", frame, changes, &mut family_timings)?;
                            lanes.serial("autonomy", frame, changes, &mut family_timings)?;
                            lanes.serial("tension", frame, changes, &mut family_timings)?;
                            lanes.serial("wars", frame, changes, &mut family_timings)?;
                            lanes.serial("ideas", frame, changes, &mut family_timings)?;
                            coordinator_order = 37;
                            family!(
                                refused,
                                "event_feed",
                                frame.hour,
                                time_family!(
                                    family_timings,
                                    "event_feed",
                                    observe,
                                    feed.observe(replay, frame, identities, cancelled)
                                )
                            );
                            lanes.serial("parties", frame, changes, &mut family_timings)?;
                            lanes.serial("leaders", frame, changes, &mut family_timings)?;
                            lanes.serial("intelligence", frame, changes, &mut family_timings)?;
                            lanes.serial("stability", frame, changes, &mut family_timings)?;
                            lanes.serial("experience", frame, changes, &mut family_timings)?;
                            lanes.serial("politics_ledger", frame, changes, &mut family_timings)?;
                            lanes.serial("fuel", frame, changes, &mut family_timings)?;
                            coordinator_order = 44;
                            family!(
                                refused,
                                "territory",
                                frame.hour,
                                time_family!(family_timings, "territory", observe, {
                                    #[cfg(test)]
                                    lanes.observed_coordinator("territory", frame, changes);
                                    territory.observe(frame, replay.format, cancelled, changes)
                                })
                            );
                            lanes.serial("occupation", frame, changes, &mut family_timings)?;
                            lanes.serial("casualties", frame, changes, &mut family_timings)?;
                            lanes.serial(
                                "equipment_losses",
                                frame,
                                changes,
                                &mut family_timings,
                            )?;
                            lanes.serial("repairs", frame, changes, &mut family_timings)?;
                            frame_count += 1;
                            coordinator_order = 49;
                            time_family!(
                                family_timings,
                                "map_export.observe",
                                observe,
                                map_export.observe_in_block_after(
                                    &mut writer,
                                    frame,
                                    replay.index[*last_blocks
                                        .get(&frame.branch)
                                        .ok_or_else(|| invalid("snow branch has no block"))?]
                                    .last_hour,
                                    map.progress()
                                        .branches
                                        .get(&frame.branch)
                                        .and_then(|progress| progress.last_hour),
                                    Some(block as u64),
                                )
                            )?;
                            time_family!(family_timings, "map", observe, {
                                let position = dashboard_base::prepared_map::Position {
                                    block,
                                    next_frame: frame_count,
                                };
                                if sparse_map {
                                    map.accept_sparse(
                                        &mut writer,
                                        replay,
                                        &updates,
                                        frame,
                                        position,
                                        || check_cancel(cancelled),
                                    )
                                } else {
                                    map.accept(&mut writer, &updates, frame, position)
                                }
                            })?;

                            Ok(())
                        })();
                        lanes.finish(
                            coordinated,
                            coordinator_order,
                            &mut lane_timings,
                            refused,
                            frame.hour,
                        )?;
                        check_cancel(cancelled)?;
                        if let Some(started) = observing_started {
                            finish_timing(
                                &mut family_timings,
                                frame_started,
                                "frame.overhead",
                                false,
                            );
                            observed += started.elapsed();
                            let progress_started = Instant::now();
                            println!("FRAME block={block} hour={}", frame.hour);
                            let elapsed = progress_started.elapsed();
                            progress_output += elapsed;
                            if let Some(families) = &mut family_timings {
                                let family = families.entry("progress.frame").or_default();
                                family.observe += elapsed;
                                family.calls += 1;
                            }
                        }
                        Ok(())
                    };

                    let result = if serial {
                        stream.next_block_recorded_with(replay, &mut observe, || {
                            check_cancel(cancelled)
                        })
                    } else {
                        stream.next_block_recorded_parallel_with(
                            &shared_replay,
                            &mut observe,
                            || check_cancel(cancelled),
                        )
                    };
                    drop(lanes);
                    result
                })?
            };
            finish_timing(&mut family_timings, stream_started, "stream.acquire", false);
            // Fold each block-prepared family's facts in block order, after the
            // block's ordinary observation succeeded and before its batch decision.
            if got && !folds.is_empty() {
                let index = &replay.index[block];
                let refused = state.refused.entry(index.branch).or_default();
                let domain = branch
                    .as_mut()
                    .ok_or_else(|| invalid("Missing prepared branch accumulator"))?;
                let mut check = || check_cancel(cancelled);
                for (family, facts) in folds {
                    if !refused.live(family.name) {
                        continue;
                    }
                    let inject = |hour: i32, result: io::Result<()>| {
                        #[cfg(test)]
                        let result = lanes::inject(family.name, hour, result);
                        #[cfg(not(test))]
                        let _ = hour;
                        result
                    };
                    let started = start_timing(&family_timings);
                    let folded = match family.name {
                        "resources" => domain
                            .resources
                            .fold(facts, replay, index, &inject, &mut check),
                        "army" => domain.army.fold(facts, replay, index, &inject, &mut check),
                        name => {
                            return Err(io::Error::other(format!(
                                "No block fold for the {name} family"
                            )));
                        }
                    };
                    let fallback = matches!(folded, Ok(Folded::Fallback));
                    finish_timing(
                        &mut family_timings,
                        started,
                        if fallback {
                            family.fallback
                        } else {
                            family.name
                        },
                        false,
                    );
                    if fallback {
                        prepass::note(family.fallback);
                    }
                    match folded {
                        Ok(_) => {}
                        Err(failure) if refuses(&failure.error) => refused.refuse(
                            family.name,
                            failure.hour,
                            failure.error.to_string(),
                            None,
                        ),
                        Err(failure) => return Err(failure.error),
                    }
                }
            }
            if let Some(value) = &mut measurement {
                value.timings.hours += frame_count;
                value.timings.observe += observed;
                if serial {
                    value.timings.decode += decoding_started
                        .expect("measurement clock initialized")
                        .elapsed()
                        .saturating_sub(observed)
                        .saturating_sub(progress_output);
                }
            }
            if frame_count == 0
                && let Some(domain) = &mut branch
                && state
                    .refused
                    .get(&replay.index[block].branch)
                    .is_none_or(|refused| refused.live("tension"))
            {
                domain.tension.unreadable_block();
            }
            if got {
                let hour = stream
                    .accumulator()
                    .map_or(replay.index[block].last_hour, |frame| frame.hour);
                let refused = state.refused.entry(replay.index[block].branch).or_default();
                family!(
                    refused,
                    "aces",
                    hour,
                    time_family!(
                        family_timings,
                        "aces",
                        flush,
                        branch
                            .as_mut()
                            .ok_or_else(|| invalid("Missing ace branch accumulator"))?
                            .aces
                            .finish_block(&mut writer, source, replay, block, &context, cancelled)
                    )
                );
                let domain = branch
                    .as_mut()
                    .ok_or_else(|| invalid("Missing air branch accumulator"))?;
                if refused.live("groups") {
                    time_family!(
                        family_timings,
                        "groups",
                        flush,
                        domain.groups.finish(replay, replay.index[block].branch)
                    );
                }
                family!(
                    refused,
                    "air_superiority",
                    hour,
                    time_family!(
                        family_timings,
                        "air_superiority",
                        flush,
                        domain.air_superiority.publish_day(&domain.groups, true)
                    )
                );
                if let Some(frame) = stream.accumulator() {
                    family!(
                        refused,
                        "construction",
                        hour,
                        time_family!(
                            family_timings,
                            "construction",
                            flush,
                            branch
                                .as_mut()
                                .ok_or_else(|| invalid("Missing construction observer"))?
                                .construction
                                .end_block(frame.branch, frame.hour, source)
                        )
                    );
                }
                let index = &replay.index[block];
                land_owners.incomplete |= frame_count as i64
                    != i64::from(index.last_hour) - i64::from(index.first_hour) + 1;
                family!(
                    refused,
                    "occupation",
                    hour,
                    time_family!(
                        family_timings,
                        "occupation",
                        flush,
                        branch
                            .as_mut()
                            .ok_or_else(|| invalid("Missing occupation observer"))?
                            .occupation
                            .finish_block(&mut writer, source, block)
                    )
                );
                let ordinal = ordinals[block];
                let domain = branch
                    .as_mut()
                    .ok_or_else(|| invalid("accepted branch accumulator is missing"))?;
                family!(
                    refused,
                    "land",
                    hour,
                    time_family!(
                        family_timings,
                        "land",
                        flush,
                        domain.land.publish_block(
                            &mut writer,
                            replay,
                            index.branch,
                            ordinal,
                            cancelled
                        )
                    )
                );
                family!(
                    refused,
                    "naval",
                    hour,
                    time_family!(
                        family_timings,
                        "naval",
                        flush,
                        domain.naval.publish_block(
                            source,
                            &context,
                            &mut writer,
                            index.branch,
                            ordinal,
                            cancelled,
                        )
                    )
                );
                family!(
                    refused,
                    "divisions",
                    hour,
                    time_family!(
                        family_timings,
                        "divisions",
                        flush,
                        domain.divisions.publish_block(
                            source,
                            &context,
                            &mut writer,
                            index.branch,
                            ordinal,
                            cancelled,
                        )
                    )
                );
                family!(
                    refused,
                    "casualties",
                    hour,
                    time_family!(
                        family_timings,
                        "casualties",
                        flush,
                        domain.casualties.finish_block(
                            &mut writer,
                            index.branch,
                            index.first_hour,
                            index.last_hour,
                        )
                    )
                );
                let last = if last_blocks.get(&index.branch) == Some(&block) {
                    replay
                        .branches
                        .iter()
                        .find(|branch| branch.id == index.branch)
                        .map_or(index.last_hour, |branch| {
                            crate::metadata::branch_bounds(replay, branch).1
                        })
                } else {
                    index.last_hour
                };
                if refused.live("equipment_losses") {
                    time_family!(
                        family_timings,
                        "equipment_losses",
                        flush,
                        domain.equipment_losses.finish_block(last)
                    );
                }
                family!(
                    refused,
                    "intelligence",
                    hour,
                    time_family!(
                        family_timings,
                        "intelligence",
                        flush,
                        domain.intelligence.finish_block(
                            source,
                            &mut writer,
                            index.branch,
                            ordinal,
                            cancelled,
                        )
                    )
                );
                new_blocks += 1;
                time_family!(
                    family_timings,
                    "progress.accepted",
                    observe,
                    accepted(&Accepted {
                        sections: preparing_sections(
                            &state,
                            stream.cursor().next_block,
                            replay.index.len()
                        ),
                        bytes,
                        next_block: published.next_block,
                        new_blocks: published.new_blocks,
                        coverage: published.coverage.clone(),
                        families: published.families.clone(),
                    })
                );
            }
            walk.folded(walk_work_started.elapsed());
            check_cancel(cancelled)?;
            let same_branch = replay
                .index
                .get(stream.cursor().next_block)
                .is_some_and(|next| {
                    replay
                        .index
                        .get(block)
                        .is_some_and(|index| next.branch == index.branch)
                });
            // The batch goes on by the recording and the bytes it holds alone
            // (`Batches`), so its ranges never depend on how fast it ran.
            if got
                && same_branch
                && !batches.full(block)
                && measurement.is_none()
                && writer.bytes_written() < BATCH_BYTES as u64
                && branch.as_ref().is_none_or(|branch| {
                    branch.air_superiority.bytes()
                        + branch.raids.bytes()
                        + branch.aces.bytes()
                        + branch.air_ledger.buffered_bytes()
                        + branch.measure.buffered_bytes()
                        + branch.index.buffered_bytes()
                        + branch.autonomy.buffered_bytes()
                        + branch.tension.buffered_bytes()
                        + branch.wars.bytes()
                        + branch.army.bytes()
                        + branch.air.bytes()
                        + branch.navy.bytes()
                        + branch.nukes.bytes()
                        + branch.factories.bytes()
                        + branch.repairs.bytes()
                        + branch.supply.bytes()
                        + branch.stockpile.bytes()
                        + branch.stability.buffered_bytes()
                        + branch.experience.buffered_bytes()
                        + branch.ledger.bytes()
                        + branch.fuel.buffered_bytes()
                        + branch.territory.bytes()
                        + branch.occupation.bytes()
                        + branch.parties.bytes()
                        + branch.game_score.buffered_bytes()
                        + branch.leaders.buffered_bytes()
                        + branch.intelligence.buffered_bytes()
                        + branch.research.buffered_bytes()
                        + branch.peace.buffered_bytes()
                        + branch.projects.buffered_bytes()
                        + branch.construction.bytes()
                        + branch.resources.bytes()
                        + branch.trade.bytes()
                        + branch.production.buffered_bytes()
                        + branch.casualties.bytes()
                        + branch.equipment_losses.bytes()
                        + branch.group_changes.bytes()
                        + branch.subject_changes.bytes()
                        + branch.idea_changes.bytes()
                        + branch.feed.bytes()
                        < BATCH_BYTES
                })
            {
                finish_timing(&mut family_timings, block_started, "block.overhead", false);
                continue;
            }
            let walk_flush_started = Instant::now();
            if got {
                let flush_started = start_timing(&family_timings);
                let frame = stream
                    .accumulator()
                    .ok_or_else(|| invalid("accepted decoder frame is missing"))?;
                let mut branch = branch
                    .take()
                    .ok_or_else(|| invalid("accepted branch accumulator is missing"))?;
                let refused = state.refused.entry(frame.branch).or_default();
                {
                    // Families flush into disjoint sections, so the block's flush
                    // runs as tasks on the lane budget (`flush`): each task's steps
                    // in serial order on one thread, every step's writes, refusal
                    // check and timings applied in the serial order below.
                    let BranchState {
                        index,
                        production,
                        research,
                        peace,
                        projects,
                        measure,
                        air_ledger,
                        aces,
                        raids,
                        air_superiority,
                        game_score,
                        autonomy,
                        tension,
                        wars,
                        army,
                        factories,
                        repairs,
                        nukes,
                        navy,
                        air,
                        supply,
                        leaders,
                        construction,
                        resources,
                        trade,
                        stockpile,
                        identities,
                        groups,
                        group_changes,
                        subjects,
                        subject_changes,
                        subject_late,
                        ideas,
                        idea_changes,
                        feed,
                        parties,
                        stability,
                        experience,
                        ledger,
                        fuel,
                        territory,
                        occupation,
                        intelligence,
                        casualties,
                        equipment_losses,
                        ..
                    } = &mut *branch;
                    let branch = frame.branch;
                    let (context, naming) = (&context, &naming);
                    let (leader_content, leader_content_errors) =
                        (&leader_content, &leader_content_errors);
                    // While the groups are prepared on their own, this is air
                    // superiority's own read of them, kept under its names.
                    let group_sections = (
                        independent::kept_as("country-groups"),
                        independent::kept_as("country-group-changes"),
                    );
                    // Likewise territory's and the event feed's own read of the
                    // identities while those are prepared on their own.
                    let identity_section = independent::kept_as("country-identities");
                    // Positions are the serial order. Identities' refusal reaches
                    // event_feed and territory (registry observers), which flush
                    // after it, so they share its task; groups' dependent
                    // air_superiority flushes before groups.
                    let tasks = vec![
                        flush::Task::new(&[(0, "focus")], move |_, writer, mut family_timings| {
                            Ok(Some(time_family!(
                                family_timings,
                                "index",
                                flush,
                                index.flush(writer, cancelled)
                            )))
                        }),
                        flush::Task::new(
                            &[(1, "production"), (5, "production")],
                            move |step, writer, mut family_timings| {
                                if step == 1 {
                                    return Ok(Some(time_family!(
                                        family_timings,
                                        "production",
                                        flush,
                                        crate::prepared_production::write_catalogue(
                                            writer, production, cancelled,
                                        )
                                    )));
                                }
                                let Some(mut page) = time_family!(
                                    family_timings,
                                    "production",
                                    flush,
                                    production.flush()
                                ) else {
                                    return Ok(None);
                                };
                                page.warnings.extend(naming.warnings.iter().cloned());
                                Ok(Some(time_family!(
                                    family_timings,
                                    "production",
                                    flush,
                                    crate::prepared_production::publish_endpoint(
                                        writer, production, &page,
                                    )
                                    .map(|_| ())
                                )))
                            },
                        ),
                        flush::Task::new(
                            &[(2, "research")],
                            move |_, writer, mut family_timings| {
                                Ok(Some(time_family!(
                                    family_timings,
                                    "research",
                                    flush,
                                    research.flush(writer, branch, cancelled)
                                )))
                            },
                        ),
                        flush::Task::new(&[(3, "peace")], move |_, writer, mut family_timings| {
                            Ok(Some(time_family!(
                                family_timings,
                                "peace",
                                flush,
                                peace.flush(writer, branch, cancelled)
                            )))
                        }),
                        flush::Task::new(
                            &[(4, "projects")],
                            move |_, writer, mut family_timings| {
                                Ok(Some(time_family!(
                                    family_timings,
                                    "projects",
                                    flush,
                                    projects.flush(writer, branch, cancelled)
                                )))
                            },
                        ),
                        flush::Task::new(
                            &[(6, "measure")],
                            move |_, writer, mut family_timings| {
                                Ok(Some(time_family!(
                                    family_timings,
                                    "measure",
                                    flush,
                                    measure.flush(writer, branch, cancelled)
                                )))
                            },
                        ),
                        flush::Task::new(
                            &[(7, "air_ledger")],
                            move |_, writer, mut family_timings| {
                                Ok(Some(time_family!(
                                    family_timings,
                                    "air_ledger",
                                    flush,
                                    air_ledger.flush(writer, branch, cancelled)
                                )))
                            },
                        ),
                        flush::Task::new(&[(8, "aces")], move |_, writer, mut family_timings| {
                            Ok(Some(time_family!(
                                family_timings,
                                "aces",
                                flush,
                                aces.flush(writer, branch, cancelled)
                            )))
                        }),
                        flush::Task::new(&[(9, "raids")], move |_, writer, mut family_timings| {
                            Ok(Some(time_family!(
                                family_timings,
                                "raids",
                                flush,
                                raids.flush(writer, branch, cancelled)
                            )))
                        }),
                        flush::Task::new(
                            &[(10, "air_superiority")],
                            move |_, writer, mut family_timings| {
                                Ok(Some(time_family!(
                                    family_timings,
                                    "air_superiority",
                                    flush,
                                    air_superiority.flush(writer, branch, cancelled)
                                )))
                            },
                        ),
                        flush::Task::new(
                            &[(11, "game_score")],
                            move |_, writer, mut family_timings| {
                                Ok(Some(time_family!(
                                    family_timings,
                                    "game_score",
                                    flush,
                                    game_score.flush(writer, branch, cancelled)
                                )))
                            },
                        ),
                        flush::Task::new(
                            &[(12, "autonomy")],
                            move |_, writer, mut family_timings| {
                                Ok(Some(time_family!(
                                    family_timings,
                                    "autonomy",
                                    flush,
                                    autonomy.flush(writer, branch, cancelled)
                                )))
                            },
                        ),
                        flush::Task::new(
                            &[(13, "tension")],
                            move |_, writer, mut family_timings| {
                                Ok(Some(time_family!(
                                    family_timings,
                                    "tension",
                                    flush,
                                    tension.flush(writer, branch, cancelled)
                                )))
                            },
                        ),
                        flush::Task::new(&[(14, "wars")], move |_, writer, mut family_timings| {
                            Ok(Some(time_family!(
                                family_timings,
                                "wars",
                                flush,
                                wars.flush(writer, branch, cancelled)
                            )))
                        }),
                        flush::Task::new(&[(15, "army")], move |_, writer, mut family_timings| {
                            Ok(Some(time_family!(
                                family_timings,
                                "army",
                                flush,
                                army.flush(writer, branch, cancelled)
                            )))
                        }),
                        flush::Task::new(
                            &[(16, "factories")],
                            move |_, writer, mut family_timings| {
                                Ok(Some(time_family!(
                                    family_timings,
                                    "factories",
                                    flush,
                                    factories.flush(writer, branch, cancelled)
                                )))
                            },
                        ),
                        flush::Task::new(&[(17, "nukes")], move |_, writer, mut family_timings| {
                            Ok(Some(time_family!(
                                family_timings,
                                "nukes",
                                flush,
                                nukes.flush(writer, branch, cancelled)
                            )))
                        }),
                        flush::Task::new(&[(18, "navy")], move |_, writer, mut family_timings| {
                            Ok(Some(time_family!(
                                family_timings,
                                "navy",
                                flush,
                                navy.flush(writer, branch, cancelled)
                            )))
                        }),
                        flush::Task::new(&[(19, "air")], move |_, writer, mut family_timings| {
                            Ok(Some(time_family!(
                                family_timings,
                                "air",
                                flush,
                                air.flush(writer, branch, cancelled)
                            )))
                        }),
                        flush::Task::new(
                            &[(20, "supply")],
                            move |_, writer, mut family_timings| {
                                Ok(Some(time_family!(
                                    family_timings,
                                    "supply",
                                    flush,
                                    supply.flush(writer, branch, cancelled)
                                )))
                            },
                        ),
                        flush::Task::new(
                            &[(21, "leaders")],
                            move |_, writer, mut family_timings| {
                                Ok(Some(time_family!(
                                    family_timings,
                                    "leaders",
                                    flush,
                                    leaders.flush(
                                        writer,
                                        branch,
                                        leader_content.as_ref(),
                                        leader_content_errors,
                                        cancelled,
                                    )
                                )))
                            },
                        ),
                        flush::Task::new(
                            &[(22, "construction")],
                            move |_, writer, mut family_timings| {
                                Ok(Some(time_family!(
                                    family_timings,
                                    "construction",
                                    flush,
                                    construction.flush(
                                        writer,
                                        source,
                                        &time_family!(
                                            family_timings,
                                            "context.construct",
                                            observe,
                                            LoadContext {
                                                content: content.cloned(),
                                            }
                                        ),
                                        branch,
                                        cancelled,
                                    )
                                )))
                            },
                        ),
                        flush::Task::new(
                            &[(23, "resources")],
                            move |_, writer, mut family_timings| {
                                Ok(Some(time_family!(
                                    family_timings,
                                    "resources",
                                    flush,
                                    resources.flush(
                                        writer,
                                        source,
                                        &time_family!(
                                            family_timings,
                                            "context.construct",
                                            observe,
                                            LoadContext {
                                                content: content.cloned(),
                                            }
                                        ),
                                        branch,
                                        cancelled,
                                    )
                                )))
                            },
                        ),
                        flush::Task::new(&[(24, "trade")], move |_, writer, mut family_timings| {
                            Ok(Some(time_family!(
                                family_timings,
                                "trade",
                                flush,
                                trade.flush(writer, source, context, branch, cancelled)
                            )))
                        }),
                        flush::Task::new(
                            &[(25, "stockpile")],
                            move |_, writer, mut family_timings| {
                                Ok(Some(time_family!(
                                    family_timings,
                                    "stockpile",
                                    flush,
                                    stockpile.flush(writer, branch, cancelled)
                                )))
                            },
                        ),
                        flush::Task::new(
                            &[(26, "identities"), (32, "event_feed"), (42, "territory")],
                            move |step, writer, mut family_timings| {
                                Ok(Some(match step {
                                    0 => time_family!(family_timings, "identities", flush, {
                                        identities.seal();
                                        writer.write_at(
                                            &format!("{identity_section}-{branch}"),
                                            0,
                                            serde_json::to_vec(&*identities)?.as_slice(),
                                        )
                                    }),
                                    1 => time_family!(
                                        family_timings,
                                        "event_feed",
                                        flush,
                                        feed.flush(writer, branch, cancelled)
                                    ),
                                    _ => time_family!(
                                        family_timings,
                                        "territory",
                                        flush,
                                        territory.flush(writer, branch, cancelled)
                                    ),
                                }))
                            },
                        ),
                        flush::Task::new(
                            &[(27, "groups"), (28, "groups"), (33, "groups")],
                            move |step, writer, mut family_timings| {
                                Ok(match step {
                                    0 => {
                                        time_family!(
                                            family_timings,
                                            "groups",
                                            flush,
                                            groups.finish(replay, branch)
                                        );
                                        None
                                    }
                                    1 => Some(time_family!(
                                        family_timings,
                                        "group_changes",
                                        flush,
                                        group_changes.flush_as(
                                            writer,
                                            group_sections.1,
                                            branch,
                                            groups,
                                            cancelled
                                        )
                                    )),
                                    _ => Some(time_family!(
                                        family_timings,
                                        "groups",
                                        flush,
                                        writer.write_at(
                                            &format!("{}-{branch}", group_sections.0),
                                            0,
                                            groups.saved_bytes()?.as_slice(),
                                        )
                                    )),
                                })
                            },
                        ),
                        flush::Task::new(
                            &[(29, "subjects"), (30, "subjects"), (34, "subjects")],
                            move |step, writer, mut family_timings| {
                                Ok(Some(match step {
                                    0 => time_family!(
                                        family_timings,
                                        "subject_changes",
                                        flush,
                                        subject_changes.flush(
                                            writer,
                                            &format!("subject-changes-{branch}"),
                                            0,
                                            cancelled,
                                        )
                                    ),
                                    1 => time_family!(
                                        family_timings,
                                        "subject_changes",
                                        flush,
                                        super::series::write_catalogue(
                                            writer,
                                            "subject-changes",
                                            branch,
                                            &std::collections::BTreeMap::new(),
                                            &(
                                                subjects.observed,
                                                subjects.change_warnings(),
                                                &*subject_late,
                                            ),
                                        )
                                    ),
                                    _ => time_family!(
                                        family_timings,
                                        "subjects",
                                        flush,
                                        writer.write_at(
                                            &format!("subjects-{branch}"),
                                            0,
                                            {
                                                subjects.seal();
                                                crate::prepared::series::compress(
                                                    serde_json::to_vec(&*subjects)?.as_slice(),
                                                    3,
                                                )?
                                            }
                                            .as_slice(),
                                        )
                                    ),
                                }))
                            },
                        ),
                        flush::Task::new(
                            &[(31, "ideas"), (35, "ideas")],
                            move |step, writer, mut family_timings| {
                                Ok(Some(if step == 0 {
                                    time_family!(
                                        family_timings,
                                        "idea_changes",
                                        flush,
                                        ideas.flush_changes(
                                            writer,
                                            branch,
                                            idea_changes,
                                            cancelled
                                        )
                                    )
                                } else {
                                    time_family!(
                                        family_timings,
                                        "ideas",
                                        flush,
                                        writer.write_at(
                                            &format!("idea-history-{branch}"),
                                            0,
                                            ideas.saved_bytes()?.as_slice(),
                                        )
                                    )
                                }))
                            },
                        ),
                        flush::Task::new(
                            &[(36, "parties"), (37, "parties")],
                            move |step, writer, mut family_timings| {
                                Ok(Some(if step == 0 {
                                    time_family!(
                                        family_timings,
                                        "parties",
                                        flush,
                                        parties.flush(writer, branch, cancelled)
                                    )
                                } else {
                                    time_family!(
                                        family_timings,
                                        "parties",
                                        flush,
                                        writer.write_at(
                                            &format!("party-history-{branch}"),
                                            0,
                                            parties.saved_bytes()?.as_slice(),
                                        )
                                    )
                                }))
                            },
                        ),
                        flush::Task::new(
                            &[(38, "stability")],
                            move |_, writer, mut family_timings| {
                                Ok(Some(time_family!(
                                    family_timings,
                                    "stability",
                                    flush,
                                    stability.flush_timed(
                                        writer,
                                        branch,
                                        cancelled,
                                        family_timings
                                    )
                                )))
                            },
                        ),
                        flush::Task::new(
                            &[(39, "politics_ledger"), (40, "politics_ledger")],
                            move |step, writer, mut family_timings| {
                                Ok(Some(if step == 0 {
                                    time_family!(
                                        family_timings,
                                        "politics_ledger",
                                        flush,
                                        ledger.flush(writer, branch, cancelled)
                                    )
                                } else {
                                    time_family!(
                                        family_timings,
                                        "politics_ledger",
                                        flush,
                                        writer.write_at(
                                            &format!("politics-ledger-state-{branch}"),
                                            0,
                                            ledger.saved_bytes()?.as_slice()
                                        )
                                    )
                                }))
                            },
                        ),
                        flush::Task::new(&[(41, "fuel")], move |_, writer, mut family_timings| {
                            Ok(Some(time_family!(
                                family_timings,
                                "fuel",
                                flush,
                                fuel.flush(writer, branch, replay.format, cancelled)
                            )))
                        }),
                        flush::Task::new(
                            &[(43, "occupation"), (44, "occupation")],
                            move |step, writer, mut family_timings| {
                                Ok(Some(if step == 0 {
                                    time_family!(
                                        family_timings,
                                        "occupation",
                                        flush,
                                        occupation.flush(writer, branch, cancelled)
                                    )
                                } else {
                                    time_family!(
                                        family_timings,
                                        "occupation",
                                        flush,
                                        writer.write_at(
                                            &format!("occupation-{branch}"),
                                            0,
                                            occupation.saved_bytes()?.as_slice(),
                                        )
                                    )
                                }))
                            },
                        ),
                        flush::Task::new(
                            &[(45, "intelligence")],
                            move |_, writer, mut family_timings| {
                                Ok(Some(time_family!(
                                    family_timings,
                                    "intelligence",
                                    flush,
                                    intelligence.flush(writer, branch, cancelled)
                                )))
                            },
                        ),
                        flush::Task::new(
                            &[(46, "casualties")],
                            move |_, writer, mut family_timings| {
                                Ok(Some(time_family!(
                                    family_timings,
                                    "casualties",
                                    flush,
                                    casualties.flush(writer, branch, cancelled)
                                )))
                            },
                        ),
                        flush::Task::new(
                            &[(47, "equipment_losses")],
                            move |_, writer, mut family_timings| {
                                Ok(Some(time_family!(
                                    family_timings,
                                    "equipment_losses",
                                    flush,
                                    equipment_losses.flush(writer, branch, cancelled)
                                )))
                            },
                        ),
                        // Experience, split from stability, flushes after every
                        // earlier position; it has no dependents.
                        flush::Task::new(
                            &[(48, "experience")],
                            move |_, writer, mut family_timings| {
                                Ok(Some(time_family!(
                                    family_timings,
                                    "experience",
                                    flush,
                                    experience.flush(writer, branch, cancelled)
                                )))
                            },
                        ),
                        flush::Task::new(
                            &[(49, "repairs")],
                            move |_, writer, mut family_timings| {
                                Ok(Some(time_family!(
                                    family_timings,
                                    "repairs",
                                    flush,
                                    repairs.flush(writer, branch, cancelled)
                                )))
                            },
                        ),
                    ];
                    flush::run(
                        tasks,
                        if serial { 1 } else { lanes::parallelism() },
                        &mut writer,
                        &mut family_timings,
                        &mut lane_timings,
                        refused,
                        frame.hour,
                    )?;
                }
                let census = dashboard_base::prepared::state_census::Marks::start(&writer, || {
                    state_census::walk_state(frame.branch, frame.hour, &branch)
                });
                time_family!(
                    family_timings,
                    "branch_state",
                    flush,
                    save_branch(&mut writer, frame.branch, &branch)
                )?;
                census.print("saved", &writer);
                time_family!(
                    family_timings,
                    "map",
                    flush,
                    map.save_resume(
                        &mut writer,
                        frame,
                        dashboard_base::prepared_map::Position {
                            block,
                            next_frame: frame_count,
                        },
                        selected(),
                    )
                )?;
                time_family!(
                    family_timings,
                    "coverage",
                    flush,
                    save_coverage(
                        &mut writer,
                        map.progress(),
                        frame.branch,
                        &last_blocks,
                        state.refused.get(&frame.branch)
                    )
                )?;
                state.cursor = Cursor {
                    next_block: stream.cursor().next_block,
                    previous: stream.cursor().previous.as_ref().map(Block::from),
                };
                kept = Some((frame.branch, branch));
                finish_timing(
                    &mut family_timings,
                    flush_started,
                    "branch.flush_overhead",
                    true,
                );
            } else if !metadata_dirty {
                finish_timing(&mut family_timings, block_started, "block.overhead", false);
                break;
            }
            land_owners.complete = stream.cursor().next_block == replay.index.len();
            time_family!(
                family_timings,
                "land_owners",
                flush,
                writer.write_at("land-owners", 0, land_owners.saved_bytes()?.as_slice())
            )?;
            state.map = map.progress().clone();
            walk.published(walk_flush_started.elapsed());
            time_family!(
                family_timings,
                "progress.before-commit",
                observe,
                before_commit(stream.cursor().next_block)
            )?;
            check_cancel(cancelled)?;
            let (committed, next) = time_family!(
                family_timings,
                "store_commit",
                flush,
                walk.commit(writer, &state, &mut lane_timings)
            )?;
            state = committed;
            bytes = next.bytes;
            metadata_dirty = false;
            saved = Some(key.clone());
            published = Accepted {
                sections: time_family!(
                    family_timings,
                    "progress.sections",
                    observe,
                    final_sections(&state, replay.index.len())
                ),
                bytes,
                next_block: state.cursor.next_block,
                new_blocks,
                coverage: time_family!(
                    family_timings,
                    "progress.coverage",
                    observe,
                    accepted_coverage(&state.map, &last_blocks)
                ),
                families: family_coverages(&state, replay, &last_blocks),
            };
            time_family!(
                family_timings,
                "progress.accepted",
                observe,
                accepted(&published)
            );
            finish_timing(&mut family_timings, block_started, "block.overhead", false);
            let mut through_reached = false;
            if got && let Some(value) = &mut measurement {
                let now = Instant::now();
                let mut families = BTreeMap::new();
                for (&name, current) in family_timings.as_ref().expect("measured table") {
                    let prior = value.prior_families.get(name);
                    families.insert(
                        name,
                        FamilyTimings {
                            observe: current
                                .observe
                                .saturating_sub(prior.map_or(Duration::ZERO, |v| v.observe)),
                            flush: current
                                .flush
                                .saturating_sub(prior.map_or(Duration::ZERO, |v| v.flush)),
                            calls: current.calls - prior.map_or(0, |v| v.calls),
                        },
                    );
                }
                families.retain(|_, timing| timing.calls > 0);
                let cumulative_hours = measured_prefix_hours(replay, stream.cursor().next_block);
                let index = &replay.index[block];
                let wall = now.duration_since(value.prior_wall);
                let residual = wall.saturating_sub(family_time(&families));
                let lane_delta = lane_timings
                    .as_ref()
                    .map(|current| {
                        current
                            .iter()
                            .filter_map(|(&name, current)| {
                                let prior = value.prior_lane_families.get(name);
                                let delta = FamilyTimings {
                                    observe: current.observe.saturating_sub(
                                        prior.map_or(Duration::ZERO, |v| v.observe),
                                    ),
                                    flush: current
                                        .flush
                                        .saturating_sub(prior.map_or(Duration::ZERO, |v| v.flush)),
                                    calls: current.calls - prior.map_or(0, |v| v.calls),
                                };
                                (delta.calls > 0).then_some((name, delta))
                            })
                            .collect::<BTreeMap<_, _>>()
                    })
                    .unwrap_or_default();
                let worker_observe = family_time(&lane_delta);
                let mut combined = Some(families);
                lanes::merge(&mut combined, Some(lane_delta));
                let families = combined.unwrap();
                let row = MeasuredBlock {
                    ordinal: block,
                    first_hour: index.first_hour,
                    last_hour: index.last_hour,
                    hours: value.timings.hours - value.prior_hours,
                    cumulative_hours,
                    wall,
                    decode: value.timings.decode.saturating_sub(value.prior_decode),
                    residual,
                    families,
                    store_bytes: bytes,
                    observe: value.timings.observe.saturating_sub(value.prior_observe),
                    worker_observe,
                };
                value.prior_decode = value.timings.decode;
                value.prior_observe = value.timings.observe;
                value.prior_lane_families = lane_timings.as_ref().cloned().unwrap_or_default();
                value.prior_hours = value.timings.hours;
                time_family!(
                    family_timings,
                    "measurement.output",
                    observe,
                    (value.committed)(&row)
                )?;
                value.prior_families = family_timings.as_ref().expect("measured table").clone();
                value.prior_wall = Instant::now();
                through_reached = value
                    .through_hours
                    .is_some_and(|target| cumulative_hours >= target);
            }
            if !got
                || stream.cursor().next_block == replay.index.len()
                || through_reached
                || measurement.as_ref().is_some_and(|value| {
                    value.through_hours.is_none() && value.started.elapsed() >= value.limit
                })
            {
                break;
            }
            let branch_started = start_timing(&family_timings);
            writer = time_family!(
                family_timings,
                "publication.open",
                observe,
                walk.open(accepted)
            )?;
            let next = &replay.index[stream.cursor().next_block];
            frame_changes.invalidate();
            branch = Some(match kept.take() {
                Some((flushed, domain)) if flushed == next.branch && !reload_flushed() => {
                    #[cfg(test)]
                    CONTINUED.with(|continued| continued.set(continued.get() + 1));
                    domain
                }
                other => {
                    time_family!(family_timings, "branch.drop", flush, drop(other));
                    time_family!(
                        family_timings,
                        "branch.restore-or-construct",
                        observe,
                        load_branch(
                            &store,
                            &key,
                            saved.as_ref(),
                            replay,
                            next.branch,
                            next.first_hour,
                            state.refused.entry(next.branch).or_default(),
                        )
                    )?
                }
            });
            if let (Some(domain), Some(frame)) = (&mut branch, stream.accumulator()) {
                time_family!(
                    family_timings,
                    "resume.observers",
                    observe,
                    domain.resume(
                        frame,
                        next.branch,
                        state.refused.entry(next.branch).or_default()
                    )
                );
            }
            finish_timing(
                &mut family_timings,
                branch_started,
                "branch.setup_overhead",
                false,
            );
        }
        time_family!(family_timings, "branch.drop", flush, drop(kept));
        Ok(())
    };
    let (opened, _) = independent::prepare(
        &phase,
        writer,
        &mut committed_state,
        &mut committed_saved,
        &mut committed_bytes,
        &mut independent_timings,
        &mut independent_workers,
        beside,
        &mut accepted,
        walk,
    )?;
    drop(opened);
    state = committed_state;
    bytes = committed_bytes;
    lanes::merge(&mut family_timings, independent_timings);
    lanes::merge(&mut lane_timings, independent_workers);
    published = Accepted {
        sections: final_sections(&state, replay.index.len()),
        bytes,
        next_block: state.cursor.next_block,
        new_blocks,
        coverage: accepted_coverage(&state.map, &last_blocks),
        families: family_coverages(&state, replay, &last_blocks),
    };
    let finalizing_started = start_timing(&family_timings);
    published.sections = time_family!(
        family_timings,
        "progress.sections",
        observe,
        final_sections(&state, replay.index.len())
    );
    time_family!(
        family_timings,
        "progress.accepted",
        observe,
        accepted(&published)
    );
    finish_timing(
        &mut family_timings,
        finalizing_started,
        "preparation.finalize",
        false,
    );
    if let Some(measurement) = measurement {
        measurement.timings.families = family_timings;
        measurement.lane_families = lane_timings;
        measurement.cleanup_started = Some(Instant::now());
    }
    Ok(Some(published))
}
/// Records the current recording's metadata and the accepted cursor as the checkpoint,
/// returning the stored byte total including the new checkpoint file.
fn write_checkpoint(
    writer: &mut store::Writer,
    state: &mut Checkpoint,
    replay: &Replay,
    map: &dashboard_base::prepared_map::Builder,
    bytes: u64,
) -> io::Result<u64> {
    state.map = map.progress().clone();
    write_checkpoint_state(writer, state, replay, bytes)
}

fn write_checkpoint_state(
    writer: &mut store::Writer,
    state: &mut Checkpoint,
    replay: &Replay,
    bytes: u64,
) -> io::Result<u64> {
    state.index = replay.index.iter().map(Block::from).collect();
    state.sessions = sessions(replay)?;
    state.branches = branches(replay)?;
    state.gaps = gaps(replay)?;
    state.refused.retain(|_, refused| !refused.0.is_empty());
    write_map_progress(writer, &state.map)?;
    // write_at replaces the old checkpoint; exclude its own size from the new
    // stored total, then add the serialized new file after writing it.
    let prior_checkpoint = writer
        .read_at(CHECKPOINT, 0)?
        .map(|range| range.len())
        .unwrap_or(0);
    state.bytes = active_bytes(bytes, writer.payload_bytes_change())?
        .checked_sub(prior_checkpoint)
        .ok_or_else(|| invalid("prepared byte count underflow"))?;
    let packet = serde_json::to_vec(&state)?;
    writer.write_at(CHECKPOINT, 0, packet.as_slice())?;
    state
        .bytes
        .checked_add(packet.len() as u64)
        .ok_or_else(|| invalid("prepared byte count overflow"))
}
fn final_sections(state: &Checkpoint, total: usize) -> Vec<Section> {
    let mut result = sections(State::Ready, state.cursor.next_block, Some(total));
    if let Some(reason) = &state.display_error {
        let display = result
            .iter_mut()
            .find(|section| section.id == "focus-display")
            .unwrap();
        display.state = State::Failed;
        display.error = Some(format!(
            "{reason} ({} country observations)",
            state.display_failures
        ));
    }
    refused_sections(&mut result, &all_refused(state));
    result
}
/// Status rows while preparing, at `done` of `total` blocks: a family
/// prepared on its own shows its own progress.
fn preparing_sections(state: &Checkpoint, done: usize, total: usize) -> Vec<Section> {
    let mut result = sections(State::Preparing, done, Some(total));
    independent::sections(&mut result, state, total);
    refused_sections(&mut result, &all_refused(state));
    result
}
/// Every family refusal, by branch: the hourly walk's, and those of families
/// prepared on their own that the walk has not reached yet.
fn all_refused(state: &Checkpoint) -> BTreeMap<u64, Refused> {
    let mut all = state.refused.clone();
    for progress in state.independent.values() {
        for (&branch, refused) in &progress.refused {
            let families = &mut all.entry(branch).or_default().0;
            for (family, refusal) in &refused.0 {
                families
                    .entry(family.clone())
                    .or_insert_with(|| refusal.clone());
            }
        }
    }
    all.retain(|_, refused| !refused.0.is_empty());
    all
}
/// Fails each refused family's status rows with every branch's reason. A
/// dependent row names the family whose refusal stopped it.
fn refused_sections(sections: &mut [Section], refused: &BTreeMap<u64, Refused>) {
    for (branch, families) in refused {
        for (name, refusal) in &families.0 {
            let Some(family) = plan::current().family(name) else {
                continue;
            };
            let from = refusal
                .from
                .as_deref()
                .and_then(|from| plan::current().family(from)?.status.first())
                .map_or(String::new(), |row| format!("{row} refused: "));
            let reason = format!(
                "Branch {branch} refused from hour {}: {from}{}",
                refusal.hour, refusal.error
            );
            for section in sections
                .iter_mut()
                .filter(|section| family.status.contains(&section.id.as_str()))
            {
                section.state = State::Failed;
                section.error = Some(
                    section
                        .error
                        .take()
                        .map_or_else(|| reason.clone(), |prior| format!("{prior}; {reason}")),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests;
