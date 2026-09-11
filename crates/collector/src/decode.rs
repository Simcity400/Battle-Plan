//! Single FIFO decode worker for the research pilot family (copy/decode split).
//!
//! The game thread copies raw research bytes only (via the factored engine
//! `copy_*` helpers in `extended::governance`), enqueues the raw job with
//! hour/seq metadata, and records copy-only nanoseconds in `table_ns`. This
//! worker interprets the raw buffers off the game thread (via the factored
//! `decode_*` helpers, reached through the governance decode path), installs
//! the identical typed rows into the same pooled frame, and hands the frame
//! to the spool ready queue. Conversion, differencing, encoding and baselines
//! downstream of the ready queue are untouched.
//!
//! Ordering: every frame transits this one FIFO in game-thread production
//! order, so spool handoff order is inherently hour order and no reorder
//! buffer exists. Skip (off-refresh) hours carry no payload and bypass decode
//! work entirely, but they still transit the FIFO as ordering placeholders so
//! a slow refresh decode can never let a later skip hour overtake it.
//!
//! Backpressure: the game thread never blocks. The FIFO holds one lightweight
//! job per pooled buffer (payload byte buffers already live on the heap;
//! payload jobs are bounded and recycled, never pool memory), so an owned
//! buffer guarantees a free slot exactly as the spool ready queue does today.
//! A full queue takes the existing whole-frame drop plus `LossBatch` gap path
//! with the existing counters and reason strings.
//!
//! Queue sizing (analytic, provisional until live calibration): the FIFO depth
//! equals the buffer pool (`BUFFERS` = 256 indices, ~2 KiB). Raw payloads are
//! recycled job boxes (`JOB_POOL_KEEP` = 2 live payloads plus the one being
//! copied): a late-game refresh carries on the order of 242k technology
//! instances x 52 B (~12.6 MB) plus redundant per-occurrence template headers
//! (~552 templates x 439 countries x ~96 B ≈ 23 MB worst case, deduplicated on
//! the decode thread) plus slots (~0.2 MB), so one payload is bounded near
//! ~40 MB and the live set near ~120 MB transient, frozen after warmup and
//! recycled, never reallocated in steady state. Game-thread production is one
//! refresh per 4 game hours (hundreds of milliseconds at speed 5) against an
//! expected decode cost in the tens of milliseconds, so the steady-state queue
//! depth is ~1; depth 2 absorbs stalls and anything beyond drops through the
//! existing gap path. Live measurement (not this change) confirms or resizes.
//!
//! Calibration: per-pilot-table copy ns, decode ns and raw bytes are monotonic
//! additive counters readable from status (`calibration_json`, merged into the
//! runtime status additively). Per-frame copy-only ns also stays in the
//! existing `table_ns` slots for the pilot tables.

use crate::{
    extended::{
        characters::CharactersJob,
        construction::BuildingsJob,
        governance::{FocusJob, ResearchJob},
        intel::RelationsJob,
        lexical::LexicalJob,
        production::VariantsJob,
        projects::ProjectsJob,
    },
    sampling::{self, BUFFERS, DivisionsJob, Shared, WarsJob},
};
use crossbeam_queue::ArrayQueue;
use replay::rows::TableId;
use serde_json::{Value, json};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

/// Pilot family in calibration order: [technologies, slots, definitions].
pub const PILOT: [TableId; 3] = [
    TableId::ResearchTechnologies,
    TableId::ResearchSlots,
    TableId::TechnologyDefinitions,
];

/// Focus pair in calibration/report order: [focus, completed]. Hourly like
/// every Tier-H table (no refresh gate): the game thread stages a raw payload
/// every hour and the worker replays it before the ready handoff. The single
/// copy pass is anchor-charged to the focus slot, matching the read-group
/// convention.
pub const FOCUS: [TableId; 2] = [TableId::Focus, TableId::CompletedFocuses];

/// Relations family in calibration/report order: [opinions, tuples].
pub const RELATIONS: [TableId; 2] = [TableId::RelationOpinions, TableId::RelationTuples];

/// Characters family in calibration/report order: [characters, leaders,
/// advisors, units, traits, experience]. ActiveAbilities stays hourly.
pub const CHARACTERS: [TableId; 6] = [
    TableId::Characters,
    TableId::CountryLeaderRoles,
    TableId::AdvisorRoles,
    TableId::UnitLeaders,
    TableId::UnitLeaderTraits,
    TableId::UnitLeaderTraitExperience,
];

/// Projects family in calibration/report order: [programs, projects,
/// history, breakthroughs]. Hourly project tables stay game-side typed.
pub const PROJECTS: [TableId; 4] = [
    TableId::ProjectPrograms,
    TableId::Projects,
    TableId::ProjectHistory,
    TableId::ProjectBreakthroughs,
];

/// Lexical family: the single global dictionary refreshed on slot 3.
pub const LEXICAL: [TableId; 1] = [TableId::LexicalTokens];

/// Variants family in calibration/report order: [definitions, modules,
/// stats, upgrades]. The registry census stays game-side; only per-variant
/// details move to the worker.
pub const VARIANTS: [TableId; 4] = [
    TableId::VariantDefinitions,
    TableId::VariantModules,
    TableId::VariantStats,
    TableId::VariantUpgrades,
];

/// Buildings family in calibration/report order: [states, provinces]. The
/// split pack shares one walk with independent per-table fault masks.
pub const BUILDINGS: [TableId; 2] = [TableId::StateBuildings, TableId::ProvinceBuildings];

/// Wars family in calibration/report order: [owned walk, matrix census].
/// Hourly like every Tier-H table (no refresh gate): the game thread stages
/// a raw payload every hour and the worker replays it before the ready
/// handoff. The matrix slot carries the independent 200-check census.
pub const WARS: [TableId; 1] = [TableId::WarRelations];

/// Divisions group in calibration/report order: [divisions, equipment,
/// groups, requests, deliveries, produced]. Hourly like every Tier-H table
/// (no refresh gate). The upgrades phase cost is anchor-charged to the
/// groups slot, matching the read-group convention.
pub const DIVISIONS: [TableId; 6] = [
    TableId::Divisions,
    TableId::DivisionEquipment,
    TableId::ArmyUpgradeGroups,
    TableId::ArmyUpgradeRequests,
    TableId::ArmyUpgradeDeliveries,
    TableId::ArmyUpgradeProduced,
];

/// Live raw payloads beyond this count are not retained; further refreshes
/// take the existing gap path until the worker catches up (see module docs).
const JOB_POOL_KEEP: usize = 2;

/// One FIFO item. The payload's byte buffers already live on the heap inside
/// its vectors; pooled-frame typed rows are filled by the worker before the
/// ready-queue handoff.
#[derive(Debug)]
pub struct DecodeJob {
    pub index: usize,
    pub seq: u64,
    pub hour: i32,
    pub pilot_refresh: bool,
    pub relations_refresh: bool,
    pub characters_refresh: bool,
    pub projects_refresh: bool,
    pub lexical_refresh: bool,
    pub variants_refresh: bool,
    pub buildings_refresh: bool,
    pub wars_refresh: bool,
    pub divisions_refresh: bool,
    pub focus_refresh: bool,
    pub force_full: bool,
    pub game_elapsed: i64,
    pub payload: Option<ResearchJob>,
    pub relations_payload: Option<RelationsJob>,
    pub characters_payload: Option<CharactersJob>,
    pub projects_payload: Option<ProjectsJob>,
    pub lexical_payload: Option<LexicalJob>,
    pub variants_payload: Option<VariantsJob>,
    pub buildings_payload: Option<BuildingsJob>,
    pub wars_payload: Option<WarsJob>,
    pub divisions_payload: Option<DivisionsJob>,
    pub focus_payload: Option<FocusJob>,
}

static COPY_NS: [AtomicU64; 3] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
static DECODE_NS: [AtomicU64; 3] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
static RAW_BYTES: [AtomicU64; 3] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
static RELATIONS_COPY_NS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static RELATIONS_DECODE_NS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static RELATIONS_RAW_BYTES: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static CHARACTERS_COPY_NS: [AtomicU64; 6] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static CHARACTERS_DECODE_NS: [AtomicU64; 6] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static CHARACTERS_RAW_BYTES: [AtomicU64; 6] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static PROJECTS_COPY_NS: [AtomicU64; 4] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static PROJECTS_DECODE_NS: [AtomicU64; 4] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static PROJECTS_RAW_BYTES: [AtomicU64; 4] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static LEXICAL_COPY_NS: [AtomicU64; 1] = [AtomicU64::new(0)];
static LEXICAL_DECODE_NS: [AtomicU64; 1] = [AtomicU64::new(0)];
static LEXICAL_RAW_BYTES: [AtomicU64; 1] = [AtomicU64::new(0)];
static VARIANTS_COPY_NS: [AtomicU64; 4] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static VARIANTS_DECODE_NS: [AtomicU64; 4] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static VARIANTS_RAW_BYTES: [AtomicU64; 4] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static BUILDINGS_COPY_NS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static BUILDINGS_DECODE_NS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static BUILDINGS_RAW_BYTES: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static WARS_COPY_NS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static WARS_DECODE_NS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static WARS_RAW_BYTES: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static DIVISIONS_COPY_NS: [AtomicU64; 6] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static DIVISIONS_DECODE_NS: [AtomicU64; 6] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static DIVISIONS_RAW_BYTES: [AtomicU64; 6] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static FOCUS_COPY_NS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static FOCUS_DECODE_NS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static FOCUS_RAW_BYTES: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static QUEUE_HIGH_WATER: AtomicU64 = AtomicU64::new(0);
static JOBS: AtomicU64 = AtomicU64::new(0);
static REFRESH_JOBS: AtomicU64 = AtomicU64::new(0);
static FULL_REFRESH_JOBS: AtomicU64 = AtomicU64::new(0);
static DROPS: AtomicU64 = AtomicU64::new(0);
static LAST_SEQ: AtomicU64 = AtomicU64::new(0);
static LAST_HOUR: AtomicU64 = AtomicU64::new(0);
static SEQ: AtomicU64 = AtomicU64::new(0);
static STOP: AtomicBool = AtomicBool::new(false);
static WORKER: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);
static JOB_POOL: Mutex<Vec<ResearchJob>> = Mutex::new(Vec::new());
static RELATIONS_JOB_POOL: Mutex<Vec<RelationsJob>> = Mutex::new(Vec::new());
static CHARACTERS_JOB_POOL: Mutex<Vec<CharactersJob>> = Mutex::new(Vec::new());
static PROJECTS_JOB_POOL: Mutex<Vec<ProjectsJob>> = Mutex::new(Vec::new());
static LEXICAL_JOB_POOL: Mutex<Vec<LexicalJob>> = Mutex::new(Vec::new());
static VARIANTS_JOB_POOL: Mutex<Vec<VariantsJob>> = Mutex::new(Vec::new());
static BUILDINGS_JOB_POOL: Mutex<Vec<BuildingsJob>> = Mutex::new(Vec::new());
static WARS_JOB_POOL: Mutex<Vec<WarsJob>> = Mutex::new(Vec::new());
static DIVISIONS_JOB_POOL: Mutex<Vec<DivisionsJob>> = Mutex::new(Vec::new());
static FOCUS_JOB_POOL: Mutex<Vec<FocusJob>> = Mutex::new(Vec::new());

/// Pilot refresh gate: the whole family shares stagger slot 2 (`H%4==2`);
/// branch starts and keyframes force a full refresh like every other table.
pub fn pilot_refresh_hour(hour: i32, force_full: bool) -> bool {
    let walk = replay::cadence::should_walk(PILOT[0], hour, force_full);
    debug_assert_eq!(
        walk,
        replay::cadence::should_walk(PILOT[1], hour, force_full)
    );
    debug_assert_eq!(
        walk,
        replay::cadence::should_walk(PILOT[2], hour, force_full)
    );
    walk
}

/// Relations refresh gate: the whole family shares stagger slot 1 (`H%4==1`);
/// branch starts and keyframes force a full refresh like every other table.
pub fn relations_refresh_hour(hour: i32, force_full: bool) -> bool {
    let walk = replay::cadence::should_walk(RELATIONS[0], hour, force_full);
    debug_assert_eq!(
        walk,
        replay::cadence::should_walk(RELATIONS[1], hour, force_full)
    );
    walk
}

/// Characters refresh gate: the slot-0 family (tables 0..6 sans hourly
/// ActiveAbilities) shares stagger slot 0 (`H%4==0`); branch starts and
/// keyframes force a full refresh like every other table.
pub fn characters_refresh_hour(hour: i32, force_full: bool) -> bool {
    let walk = replay::cadence::should_walk(CHARACTERS[0], hour, force_full);
    for table in CHARACTERS.iter().skip(1) {
        debug_assert_eq!(walk, replay::cadence::should_walk(*table, hour, force_full));
    }
    walk
}

/// Projects refresh gate: the slot-3 project family shares stagger slot 3
/// (`H%4==3`); branch starts and keyframes force a full refresh like every
/// other table.
pub fn projects_refresh_hour(hour: i32, force_full: bool) -> bool {
    let walk = replay::cadence::should_walk(PROJECTS[0], hour, force_full);
    for table in PROJECTS.iter().skip(1) {
        debug_assert_eq!(walk, replay::cadence::should_walk(*table, hour, force_full));
    }
    walk
}

/// Lexical refresh gate: the single dictionary shares stagger slot 3
/// (`H%4==3`); branch starts and keyframes force a full refresh like every
/// other table.
pub fn lexical_refresh_hour(hour: i32, force_full: bool) -> bool {
    replay::cadence::should_walk(LEXICAL[0], hour, force_full)
}

/// Variants refresh gate: the slot-3 detail family shares stagger slot 3
/// (`H%4==3`); branch starts and keyframes force a full refresh like every
/// other table.
pub fn variants_refresh_hour(hour: i32, force_full: bool) -> bool {
    let walk = replay::cadence::should_walk(VARIANTS[0], hour, force_full);
    for table in VARIANTS.iter().skip(1) {
        debug_assert_eq!(walk, replay::cadence::should_walk(*table, hour, force_full));
    }
    walk
}

/// Buildings refresh gate: the slot-3 split pack shares stagger slot 3
/// (`H%4==3`); branch starts and keyframes force a full refresh like every
/// other table.
pub fn buildings_refresh_hour(hour: i32, force_full: bool) -> bool {
    let walk = replay::cadence::should_walk(BUILDINGS[0], hour, force_full);
    debug_assert_eq!(
        walk,
        replay::cadence::should_walk(BUILDINGS[1], hour, force_full)
    );
    walk
}

/// Monotonic frame sequence for FIFO order proofs. Gaps in the sequence are
/// dropped frames; order within the sequence is queue order.
pub fn next_seq() -> u64 {
    SEQ.fetch_add(1, Ordering::Relaxed)
}

/// Recycled raw-payload job. Cold calls allocate; steady state reuses jobs
/// with their capacities, so neither thread allocates per refresh.
pub fn acquire_job() -> ResearchJob {
    if let Ok(mut pool) = JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    ResearchJob::new()
}

/// Return a payload job to the pool after clearing it. Growth decided here
/// runs on the calling (off-game-thread) side; the game thread never grows.
pub fn recycle_job(mut job: ResearchJob) {
    job.clear();
    if job.grow_for_next() {
        // Overflowed capacities double off the game thread until copies fit.
        job.grow();
    }
    if let Ok(mut pool) = JOB_POOL.try_lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Recycled relations raw-payload job. Cold calls allocate; steady state
/// reuses jobs with their capacities, so neither thread allocates per refresh.
pub fn acquire_relations_job() -> RelationsJob {
    if let Ok(mut pool) = RELATIONS_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    RelationsJob::new()
}

/// Return a relations payload job to the pool after clearing it. Growth
/// decided here runs on the calling (off-game-thread) side.
pub fn recycle_relations_job(mut job: RelationsJob) {
    job.clear();
    if job.grow_for_next() {
        job.grow();
    }
    if let Ok(mut pool) = RELATIONS_JOB_POOL.try_lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Recycled characters raw-payload job. Cold calls allocate; steady state
/// reuses jobs with their capacities, so neither thread allocates per refresh.
pub fn acquire_characters_job() -> CharactersJob {
    if let Ok(mut pool) = CHARACTERS_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    CharactersJob::new()
}

/// Return a characters payload job to the pool after clearing it. Growth
/// decided here runs on the calling (off-game-thread) side.
pub fn recycle_characters_job(mut job: CharactersJob) {
    job.clear();
    if job.grow_for_next() {
        job.grow();
    }
    if let Ok(mut pool) = CHARACTERS_JOB_POOL.try_lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Recycled projects raw-payload job. Cold calls allocate; steady state
/// reuses jobs with their capacities, so neither thread allocates per refresh.
pub fn acquire_projects_job() -> ProjectsJob {
    if let Ok(mut pool) = PROJECTS_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    ProjectsJob::new()
}

/// Return a projects payload job to the pool after clearing it. Growth
/// decided here runs on the calling (off-game-thread) side.
pub fn recycle_projects_job(mut job: ProjectsJob) {
    job.clear();
    if job.grow_for_next() {
        job.grow();
    }
    if let Ok(mut pool) = PROJECTS_JOB_POOL.try_lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Recycled lexical raw-payload job. Cold calls allocate; steady state
/// reuses jobs with their capacities, so neither thread allocates per refresh.
pub fn acquire_lexical_job() -> LexicalJob {
    if let Ok(mut pool) = LEXICAL_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    LexicalJob::new()
}

/// Return a lexical payload job to the pool after clearing it. Growth
/// decided here runs on the calling (off-game-thread) side.
pub fn recycle_lexical_job(mut job: LexicalJob) {
    job.clear();
    if job.grow_for_next() {
        job.grow();
    }
    if let Ok(mut pool) = LEXICAL_JOB_POOL.try_lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Recycled variants raw-payload job. Cold calls allocate; steady state
/// reuses jobs with their capacities, so neither thread allocates per refresh.
pub fn acquire_variants_job() -> VariantsJob {
    if let Ok(mut pool) = VARIANTS_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    VariantsJob::new()
}

/// Return a variants payload job to the pool after clearing it. Growth
/// decided here runs on the calling (off-game-thread) side.
pub fn recycle_variants_job(mut job: VariantsJob) {
    job.clear();
    if job.grow_for_next() {
        job.grow();
    }
    if let Ok(mut pool) = VARIANTS_JOB_POOL.try_lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Recycled buildings raw-payload job. Cold calls allocate; steady state
/// reuses jobs with their capacities, so neither thread allocates per refresh.
pub fn acquire_buildings_job() -> BuildingsJob {
    if let Ok(mut pool) = BUILDINGS_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    BuildingsJob::new()
}

/// Return a buildings payload job to the pool after clearing it. Growth
/// decided here runs on the calling (off-game-thread) side.
pub fn recycle_buildings_job(mut job: BuildingsJob) {
    job.clear();
    if job.grow_for_next() {
        job.grow();
    }
    if let Ok(mut pool) = BUILDINGS_JOB_POOL.try_lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Recycled wars raw-payload job. Cold calls allocate; steady state
/// reuses jobs with their capacities, so neither thread allocates per hour.
pub fn acquire_wars_job() -> WarsJob {
    if let Ok(mut pool) = WARS_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    WarsJob::new()
}

/// Return a wars payload job to the pool after clearing it. Growth
/// decided here runs on the calling (off-game-thread) side.
pub fn recycle_wars_job(mut job: WarsJob) {
    job.clear();
    if job.grow_for_next() {
        job.grow();
    }
    if let Ok(mut pool) = WARS_JOB_POOL.try_lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Recycled divisions-group raw-payload job. Cold calls allocate; steady
/// state reuses jobs with their capacities, so neither thread allocates
/// per hour.
pub fn acquire_divisions_job() -> DivisionsJob {
    if let Ok(mut pool) = DIVISIONS_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    DivisionsJob::new()
}

/// Return a divisions payload job to the pool after clearing it. Growth
/// decided here runs on the calling (off-game-thread) side.
pub fn recycle_divisions_job(mut job: DivisionsJob) {
    job.clear();
    if job.grow_for_next() {
        job.grow();
    }
    if let Ok(mut pool) = DIVISIONS_JOB_POOL.try_lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Recycled hourly focus raw-payload job. Cold calls allocate; steady state
/// reuses jobs with their capacities, so neither thread allocates per hour.
pub fn acquire_focus_job() -> FocusJob {
    if let Ok(mut pool) = FOCUS_JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    FocusJob::new()
}

/// Return a focus payload job to the pool after clearing it. Growth
/// decided here runs on the calling (off-game-thread) side.
pub fn recycle_focus_job(mut job: FocusJob) {
    job.clear();
    if job.grow_for_next() {
        job.grow();
    }
    if let Ok(mut pool) = FOCUS_JOB_POOL.try_lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Enqueue a frame for decode (or ordered forwarding for skip hours).
/// Starts the worker lazily; never blocks. A full queue returns the job for
/// the existing gap path and counts the drop on the existing counters.
pub fn enqueue(shared: &Shared, job: DecodeJob) -> Option<DecodeJob> {
    ensure_worker();
    match shared.decode_queue.push(job) {
        Ok(()) => {
            JOBS.fetch_add(1, Ordering::Relaxed);
            // Decode-queue depth high-water for live backlog visibility.
            // Records the deepest observed FIFO occupancy (bounded by BUFFERS).
            let depth = shared.decode_queue.len() as u64;
            QUEUE_HIGH_WATER.fetch_max(depth, Ordering::Relaxed);
            None
        }
        Err(job) => {
            DROPS.fetch_add(1, Ordering::Relaxed);
            Some(job)
        }
    }
}

/// The single FIFO decode worker. Game frames arrive in hour order and hand
/// off to the spool ready queue in the same order; no reorder buffer exists.
fn decode_loop() {
    loop {
        if STOP.load(Ordering::Acquire) {
            break;
        }
        let shared_ptr = sampling::SHARED.load(Ordering::Acquire);
        if shared_ptr.is_null() {
            std::thread::yield_now();
            continue;
        }
        let shared = unsafe { &*shared_ptr };
        match shared.decode_queue.pop() {
            Some(job) => process_job(shared, &job),
            None => std::thread::sleep(std::time::Duration::from_millis(1)),
        }
    }
}

fn ensure_worker() {
    if let Ok(mut slot) = WORKER.try_lock() {
        let running = slot.as_ref().is_some_and(|handle| !handle.is_finished());
        if !running {
            STOP.store(false, Ordering::Release);
            if let Ok(handle) = std::thread::Builder::new()
                .name("hoi4-decode".into())
                .spawn(decode_loop)
            {
                *slot = Some(handle);
            }
        }
    }
}

/// Decode one FIFO item and hand its pooled frame to the spool ready queue
/// in FIFO hour order. Skip items forward without touching typed rows.
pub fn process_job(shared: &Shared, job: &DecodeJob) {
    LAST_SEQ.store(job.seq, Ordering::Relaxed);
    LAST_HOUR.store(job.hour as u64, Ordering::Relaxed);
    if job.force_full {
        FULL_REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }
    if job.pilot_refresh
        && let Some(payload) = job.payload.as_ref()
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 3];
        frame.extended.research_pilot().decode_research_payload(
            shared.base,
            payload,
            &mut decode_ticks,
        );
        // The phased split mirrors the copy attribution in
        // `extended::governance` (technologies, slots, definitions).
        for (total, ticks) in DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }
    if job.relations_refresh
        && let Some(payload) = job.relations_payload.as_ref()
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 2];
        frame.extended.relations_pilot().decode_relations_payload(
            shared.base,
            payload,
            &mut decode_ticks,
        );
        for (total, ticks) in RELATIONS_DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }
    if job.characters_refresh
        && let Some(payload) = job.characters_payload.as_ref()
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 6];
        frame.extended.characters_pilot().decode_characters_payload(
            shared.base,
            payload,
            &mut decode_ticks,
        );
        for (total, ticks) in CHARACTERS_DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }
    if job.projects_refresh
        && let Some(payload) = job.projects_payload.as_ref()
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 4];
        frame.extended.projects_pilot().decode_projects_payload(
            shared.base,
            payload,
            &mut decode_ticks,
        );
        for (total, ticks) in PROJECTS_DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }
    if job.lexical_refresh
        && let Some(payload) = job.lexical_payload.as_ref()
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 1];
        frame.extended.lexical_pilot().decode_lexical_payload(
            shared.base,
            payload,
            &mut decode_ticks,
        );
        for (total, ticks) in LEXICAL_DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }
    if job.variants_refresh
        && let Some(payload) = job.variants_payload.as_ref()
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 4];
        frame
            .extended
            .decode_variants_payload(shared.base, payload, &mut decode_ticks);
        for (total, ticks) in VARIANTS_DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }
    if job.buildings_refresh
        && let Some(payload) = job.buildings_payload.as_ref()
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 2];
        frame.extended.buildings_pilot().decode_buildings_payload(
            shared.base,
            payload,
            &mut decode_ticks,
        );
        for (total, ticks) in BUILDINGS_DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }
    if job.wars_refresh
        && let Some(payload) = job.wars_payload.as_ref()
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 2];
        sampling::decode_wars_payload(frame, shared.base, payload, &mut decode_ticks);
        for (total, ticks) in WARS_DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }
    if job.divisions_refresh
        && let Some(payload) = job.divisions_payload.as_ref()
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 6];
        sampling::decode_divisions_payload(frame, shared.base, payload, &mut decode_ticks);
        for (total, ticks) in DIVISIONS_DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }
    if job.focus_refresh
        && let Some(payload) = job.focus_payload.as_ref()
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 2];
        frame
            .extended
            .focus_pilot()
            .decode_focus_payload(shared.base, payload, &mut decode_ticks);
        for (total, ticks) in FOCUS_DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }
    // Pool invariant: this frame is owned by the worker, so it occupies no
    // ready slot and the push below always has room while the spool thread
    // lives. The worker may wait (only the game thread must never block).
    while shared.ready.push(job.index).is_err() {
        std::thread::yield_now();
    }
    shared.completed[job.index].store(job.game_elapsed.max(1), Ordering::Release);
}

/// Detach drain (runtime only, after callback quiescence): stop the worker,
/// join it, then synchronously decode any remaining FIFO items in order and
/// hand them to the spool ready queue before the existing spool drain runs.
/// The spool thread is still consuming, so handoffs make progress.
pub fn detach_drain() {
    STOP.store(true, Ordering::Release);
    if let Ok(mut slot) = WORKER.lock()
        && let Some(handle) = slot.take()
    {
        let _ = handle.join();
    }
    let shared_ptr = sampling::SHARED.load(Ordering::Acquire);
    if shared_ptr.is_null() {
        STOP.store(false, Ordering::Release);
        return;
    }
    let shared = unsafe { &*shared_ptr };
    while let Some(job) = shared.decode_queue.pop() {
        process_job(shared, &job);
    }
    STOP.store(false, Ordering::Release);
}

/// Additive calibration snapshot. Every counter is monotonic since attach.
pub fn calibration() -> Calibration {
    Calibration {
        jobs: JOBS.load(Ordering::Relaxed),
        refresh_jobs: REFRESH_JOBS.load(Ordering::Relaxed),
        full_refresh_jobs: FULL_REFRESH_JOBS.load(Ordering::Relaxed),
        drops: DROPS.load(Ordering::Relaxed),
        last_seq: LAST_SEQ.load(Ordering::Relaxed),
        last_hour: LAST_HOUR.load(Ordering::Relaxed) as i32,
        copy_ns: COPY_NS.each_ref().map(|cell| cell.load(Ordering::Relaxed)),
        decode_ns: DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        raw_bytes: RAW_BYTES
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        relations_copy_ns: RELATIONS_COPY_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        relations_decode_ns: RELATIONS_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        relations_raw_bytes: RELATIONS_RAW_BYTES
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        characters_copy_ns: CHARACTERS_COPY_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        characters_decode_ns: CHARACTERS_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        characters_raw_bytes: CHARACTERS_RAW_BYTES
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        projects_copy_ns: PROJECTS_COPY_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        projects_decode_ns: PROJECTS_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        projects_raw_bytes: PROJECTS_RAW_BYTES
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        lexical_copy_ns: LEXICAL_COPY_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        lexical_decode_ns: LEXICAL_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        lexical_raw_bytes: LEXICAL_RAW_BYTES
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        variants_copy_ns: VARIANTS_COPY_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        variants_decode_ns: VARIANTS_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        variants_raw_bytes: VARIANTS_RAW_BYTES
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        buildings_copy_ns: BUILDINGS_COPY_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        buildings_decode_ns: BUILDINGS_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        buildings_raw_bytes: BUILDINGS_RAW_BYTES
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        wars_copy_ns: WARS_COPY_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        wars_decode_ns: WARS_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        wars_raw_bytes: WARS_RAW_BYTES
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        divisions_copy_ns: DIVISIONS_COPY_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        divisions_decode_ns: DIVISIONS_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        divisions_raw_bytes: DIVISIONS_RAW_BYTES
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        focus_copy_ns: FOCUS_COPY_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        focus_decode_ns: FOCUS_DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        focus_raw_bytes: FOCUS_RAW_BYTES
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        queue_high_water: QUEUE_HIGH_WATER.load(Ordering::Relaxed),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Calibration {
    pub jobs: u64,
    pub refresh_jobs: u64,
    pub full_refresh_jobs: u64,
    pub drops: u64,
    pub last_seq: u64,
    pub last_hour: i32,
    pub copy_ns: [u64; 3],
    pub decode_ns: [u64; 3],
    pub raw_bytes: [u64; 3],
    pub relations_copy_ns: [u64; 2],
    pub relations_decode_ns: [u64; 2],
    pub relations_raw_bytes: [u64; 2],
    pub characters_copy_ns: [u64; 6],
    pub characters_decode_ns: [u64; 6],
    pub characters_raw_bytes: [u64; 6],
    pub projects_copy_ns: [u64; 4],
    pub projects_decode_ns: [u64; 4],
    pub projects_raw_bytes: [u64; 4],
    pub lexical_copy_ns: [u64; 1],
    pub lexical_decode_ns: [u64; 1],
    pub lexical_raw_bytes: [u64; 1],
    pub variants_copy_ns: [u64; 4],
    pub variants_decode_ns: [u64; 4],
    pub variants_raw_bytes: [u64; 4],
    pub buildings_copy_ns: [u64; 2],
    pub buildings_decode_ns: [u64; 2],
    pub buildings_raw_bytes: [u64; 2],
    pub wars_copy_ns: [u64; 2],
    pub wars_decode_ns: [u64; 2],
    pub wars_raw_bytes: [u64; 2],
    pub divisions_copy_ns: [u64; 6],
    pub divisions_decode_ns: [u64; 6],
    pub divisions_raw_bytes: [u64; 6],
    pub focus_copy_ns: [u64; 2],
    pub focus_decode_ns: [u64; 2],
    pub focus_raw_bytes: [u64; 2],
    pub queue_high_water: u64,
}

impl Calibration {
    pub fn json(&self) -> Value {
        json!({
            "scope": "decode worker totals since attach; monotonic",
            "jobs": self.jobs,
            "refresh_jobs": self.refresh_jobs,
            "full_refresh_jobs": self.full_refresh_jobs,
            "drops": self.drops,
            "last_seq": self.last_seq,
            "last_hour": self.last_hour,
            "copy_ns": self.copy_ns.as_slice(),
            "decode_ns": self.decode_ns.as_slice(),
            "raw_bytes": self.raw_bytes.as_slice(),
            "tables": ["ResearchTechnologies", "ResearchSlots", "TechnologyDefinitions"],
            "relations_copy_ns": self.relations_copy_ns.as_slice(),
            "relations_decode_ns": self.relations_decode_ns.as_slice(),
            "relations_raw_bytes": self.relations_raw_bytes.as_slice(),
            "relations_tables": ["RelationOpinions", "RelationTuples"],
            "characters_copy_ns": self.characters_copy_ns.as_slice(),
            "characters_decode_ns": self.characters_decode_ns.as_slice(),
            "characters_raw_bytes": self.characters_raw_bytes.as_slice(),
            "characters_tables": ["Characters", "CountryLeaderRoles", "AdvisorRoles", "UnitLeaders", "UnitLeaderTraits", "UnitLeaderTraitExperience"],
            "projects_copy_ns": self.projects_copy_ns.as_slice(),
            "projects_decode_ns": self.projects_decode_ns.as_slice(),
            "projects_raw_bytes": self.projects_raw_bytes.as_slice(),
            "projects_tables": ["ProjectPrograms", "Projects", "ProjectHistory", "ProjectBreakthroughs"],
            "lexical_copy_ns": self.lexical_copy_ns.as_slice(),
            "lexical_decode_ns": self.lexical_decode_ns.as_slice(),
            "lexical_raw_bytes": self.lexical_raw_bytes.as_slice(),
            "lexical_tables": ["LexicalTokens"],
            "variants_copy_ns": self.variants_copy_ns.as_slice(),
            "variants_decode_ns": self.variants_decode_ns.as_slice(),
            "variants_raw_bytes": self.variants_raw_bytes.as_slice(),
            "variants_tables": ["VariantDefinitions", "VariantModules", "VariantStats", "VariantUpgrades"],
            "buildings_copy_ns": self.buildings_copy_ns.as_slice(),
            "buildings_decode_ns": self.buildings_decode_ns.as_slice(),
            "buildings_raw_bytes": self.buildings_raw_bytes.as_slice(),
            "buildings_tables": ["StateBuildings", "ProvinceBuildings"],
            "wars_copy_ns": self.wars_copy_ns.as_slice(),
            "wars_decode_ns": self.wars_decode_ns.as_slice(),
            "wars_raw_bytes": self.wars_raw_bytes.as_slice(),
            "wars_tables": ["WarRelations", "WarRelationsMatrix"],
            "divisions_copy_ns": self.divisions_copy_ns.as_slice(),
            "divisions_decode_ns": self.divisions_decode_ns.as_slice(),
            "divisions_raw_bytes": self.divisions_raw_bytes.as_slice(),
            "divisions_tables": ["Divisions", "DivisionEquipment", "ArmyUpgradeGroups", "ArmyUpgradeRequests", "ArmyUpgradeDeliveries", "ArmyUpgradeProduced"],
            "focus_copy_ns": self.focus_copy_ns.as_slice(),
            "focus_decode_ns": self.focus_decode_ns.as_slice(),
            "focus_raw_bytes": self.focus_raw_bytes.as_slice(),
            "focus_tables": ["Focus", "CompletedFocuses"],
            "queue_high_water": self.queue_high_water,
            "queue_capacity": BUFFERS,
        })
    }
}

/// Additive status merge for the runtime statuswriters. Existing keys survive.
pub fn calibration_json() -> Value {
    calibration().json()
}

pub(crate) fn add_copy_ns(table: usize, ns: u64) {
    COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_raw_bytes(table: usize, bytes: u64) {
    RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_relations_copy_ns(table: usize, ns: u64) {
    RELATIONS_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_relations_raw_bytes(table: usize, bytes: u64) {
    RELATIONS_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_characters_copy_ns(table: usize, ns: u64) {
    CHARACTERS_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_characters_raw_bytes(table: usize, bytes: u64) {
    CHARACTERS_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_projects_copy_ns(table: usize, ns: u64) {
    PROJECTS_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_projects_raw_bytes(table: usize, bytes: u64) {
    PROJECTS_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_lexical_copy_ns(table: usize, ns: u64) {
    LEXICAL_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_lexical_raw_bytes(table: usize, bytes: u64) {
    LEXICAL_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_variants_copy_ns(table: usize, ns: u64) {
    VARIANTS_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_variants_raw_bytes(table: usize, bytes: u64) {
    VARIANTS_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_buildings_copy_ns(table: usize, ns: u64) {
    BUILDINGS_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_buildings_raw_bytes(table: usize, bytes: u64) {
    BUILDINGS_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_wars_copy_ns(table: usize, ns: u64) {
    WARS_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_wars_raw_bytes(table: usize, bytes: u64) {
    WARS_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_divisions_copy_ns(table: usize, ns: u64) {
    DIVISIONS_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_divisions_raw_bytes(table: usize, bytes: u64) {
    DIVISIONS_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_focus_copy_ns(table: usize, ns: u64) {
    FOCUS_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_focus_raw_bytes(table: usize, bytes: u64) {
    FOCUS_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

/// Bounded raw FIFO type shared with sampling's pool (indices only).
pub type DecodeQueue = ArrayQueue<DecodeJob>;

pub fn new_queue() -> DecodeQueue {
    ArrayQueue::new(BUFFERS)
}

#[cfg(test)]
pub(crate) fn reset_calibration_for_test() {
    for cell in COPY_NS
        .iter()
        .chain(DECODE_NS.iter())
        .chain(RAW_BYTES.iter())
        .chain(RELATIONS_COPY_NS.iter())
        .chain(RELATIONS_DECODE_NS.iter())
        .chain(RELATIONS_RAW_BYTES.iter())
        .chain(CHARACTERS_COPY_NS.iter())
        .chain(CHARACTERS_DECODE_NS.iter())
        .chain(CHARACTERS_RAW_BYTES.iter())
        .chain(PROJECTS_COPY_NS.iter())
        .chain(PROJECTS_DECODE_NS.iter())
        .chain(PROJECTS_RAW_BYTES.iter())
        .chain(LEXICAL_COPY_NS.iter())
        .chain(LEXICAL_DECODE_NS.iter())
        .chain(LEXICAL_RAW_BYTES.iter())
        .chain(VARIANTS_COPY_NS.iter())
        .chain(VARIANTS_DECODE_NS.iter())
        .chain(VARIANTS_RAW_BYTES.iter())
        .chain(BUILDINGS_COPY_NS.iter())
        .chain(BUILDINGS_DECODE_NS.iter())
        .chain(BUILDINGS_RAW_BYTES.iter())
        .chain(WARS_COPY_NS.iter())
        .chain(WARS_DECODE_NS.iter())
        .chain(WARS_RAW_BYTES.iter())
        .chain(DIVISIONS_COPY_NS.iter())
        .chain(DIVISIONS_DECODE_NS.iter())
        .chain(DIVISIONS_RAW_BYTES.iter())
        .chain(FOCUS_COPY_NS.iter())
        .chain(FOCUS_DECODE_NS.iter())
        .chain(FOCUS_RAW_BYTES.iter())
    {
        cell.store(0, Ordering::Relaxed);
    }
    QUEUE_HIGH_WATER.store(0, Ordering::Relaxed);
    JOBS.store(0, Ordering::Relaxed);
    REFRESH_JOBS.store(0, Ordering::Relaxed);
    FULL_REFRESH_JOBS.store(0, Ordering::Relaxed);
    DROPS.store(0, Ordering::Relaxed);
    LAST_SEQ.store(0, Ordering::Relaxed);
    LAST_HOUR.store(0, Ordering::Relaxed);
    SEQ.store(0, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extended::governance::research_test_kit;
    use std::sync::atomic::Ordering;

    /// Calibration globals are shared across tests in this binary; the tests
    /// below that reset or advance them hold this lock so assertions stay
    /// deterministic under parallel execution.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn pilot_gate_matches_stagger_slot_two_and_force_full() {
        // Slot 2 refreshes exactly on H%4==2; branch starts and keyframes
        // force a full refresh regardless of hour.
        for hour in -8..8i32 {
            let refresh = hour.rem_euclid(4) == 2;
            assert_eq!(pilot_refresh_hour(hour, false), refresh, "hour {hour}");
            assert!(pilot_refresh_hour(hour, true), "forced hour {hour}");
        }
    }

    #[test]
    fn relations_gate_matches_stagger_slot_one_and_force_full() {
        // Slot 1 refreshes exactly on H%4==1; branch starts and keyframes
        // force a full refresh regardless of hour.
        for hour in -8..8i32 {
            let refresh = hour.rem_euclid(4) == 1;
            assert_eq!(relations_refresh_hour(hour, false), refresh, "hour {hour}");
            assert!(relations_refresh_hour(hour, true), "forced hour {hour}");
        }
    }

    #[test]
    fn characters_gate_matches_stagger_slot_zero_and_force_full() {
        // Slot 0 refreshes exactly on H%4==0; branch starts and keyframes
        // force a full refresh regardless of hour.
        for hour in -8..8i32 {
            let refresh = hour.rem_euclid(4) == 0;
            assert_eq!(characters_refresh_hour(hour, false), refresh, "hour {hour}");
            assert!(characters_refresh_hour(hour, true), "forced hour {hour}");
        }
    }

    #[test]
    fn projects_gate_matches_stagger_slot_three_and_force_full() {
        // Slot 3 refreshes exactly on H%4==3; branch starts and keyframes
        // force a full refresh regardless of hour.
        for hour in -8..8i32 {
            let refresh = hour.rem_euclid(4) == 3;
            assert_eq!(projects_refresh_hour(hour, false), refresh, "hour {hour}");
            assert!(projects_refresh_hour(hour, true), "forced hour {hour}");
        }
    }

    #[test]
    fn lexical_gate_matches_stagger_slot_three_and_force_full() {
        // The dictionary shares slot 3 like the other slot-3 families.
        for hour in -8..8i32 {
            let refresh = hour.rem_euclid(4) == 3;
            assert_eq!(lexical_refresh_hour(hour, false), refresh, "hour {hour}");
            assert!(lexical_refresh_hour(hour, true), "forced hour {hour}");
        }
    }

    #[test]
    fn variants_gate_matches_stagger_slot_three_and_force_full() {
        // Variant details share slot 3 like the other slot-3 families.
        for hour in -8..8i32 {
            let refresh = hour.rem_euclid(4) == 3;
            assert_eq!(variants_refresh_hour(hour, false), refresh, "hour {hour}");
            assert!(variants_refresh_hour(hour, true), "forced hour {hour}");
        }
    }

    #[test]
    fn buildings_gate_matches_stagger_slot_three_and_force_full() {
        // The split pack shares slot 3 like the other slot-3 families.
        for hour in -8..8i32 {
            let refresh = hour.rem_euclid(4) == 3;
            assert_eq!(buildings_refresh_hour(hour, false), refresh, "hour {hour}");
            assert!(buildings_refresh_hour(hour, true), "forced hour {hour}");
        }
    }

    #[test]
    fn fifo_hands_off_every_frame_in_production_order_without_reorder() {
        let shared = Shared::new(0, 0, 1);
        let mut indices = Vec::new();
        for _ in 0..4 {
            indices.push(shared.free.pop().expect("pooled buffer"));
        }
        // Enqueue out-of-seq labels; handoff order must still be FIFO.
        for (order, index) in indices.iter().enumerate() {
            shared.completed[*index].store(0, Ordering::Relaxed);
            let job = DecodeJob {
                index: *index,
                seq: 100 + order as u64,
                hour: 200 + order as i32,
                pilot_refresh: false,
                relations_refresh: false,
                characters_refresh: false,
                projects_refresh: false,
                lexical_refresh: false,
                variants_refresh: false,
                buildings_refresh: false,
                wars_refresh: false,
                divisions_refresh: false,
                focus_refresh: false,
                force_full: false,
                game_elapsed: 7,
                payload: None,
                relations_payload: None,
                characters_payload: None,
                projects_payload: None,
                lexical_payload: None,
                variants_payload: None,
                buildings_payload: None,
                wars_payload: None,
                divisions_payload: None,
                focus_payload: None,
            };
            shared.decode_queue.push(job).unwrap();
        }
        let mut handed = Vec::new();
        while let Some(job) = shared.decode_queue.pop() {
            process_job(&shared, &job);
            handed.push(job.index);
        }
        assert_eq!(handed, indices);
        for index in indices {
            assert_eq!(shared.ready.pop(), Some(index));
            assert_eq!(shared.completed[index].load(Ordering::Acquire), 7);
        }
        assert!(shared.ready.pop().is_none());
    }

    #[test]
    fn queue_full_returns_the_job_for_the_existing_gap_path() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let shared = Shared::new(0, 0, 1);
        for seq in 0..BUFFERS {
            let job = DecodeJob {
                index: 0,
                seq: seq as u64,
                hour: 0,
                pilot_refresh: false,
                relations_refresh: false,
                characters_refresh: false,
                projects_refresh: false,
                lexical_refresh: false,
                variants_refresh: false,
                buildings_refresh: false,
                wars_refresh: false,
                divisions_refresh: false,
                focus_refresh: false,
                force_full: false,
                game_elapsed: 1,
                payload: None,
                relations_payload: None,
                characters_payload: None,
                projects_payload: None,
                lexical_payload: None,
                variants_payload: None,
                buildings_payload: None,
                wars_payload: None,
                divisions_payload: None,
                focus_payload: None,
            };
            assert!(shared.decode_queue.push(job).is_ok());
        }
        let before = DROPS.load(Ordering::Relaxed);
        let job = DecodeJob {
            index: 0,
            seq: u64::MAX,
            hour: 0,
            pilot_refresh: false,
            relations_refresh: false,
            characters_refresh: false,
            projects_refresh: false,
            lexical_refresh: false,
            variants_refresh: false,
            buildings_refresh: false,
            wars_refresh: false,
            divisions_refresh: false,
            focus_refresh: false,
            force_full: false,
            game_elapsed: 1,
            payload: None,
            relations_payload: None,
            characters_payload: None,
            projects_payload: None,
            lexical_payload: None,
            variants_payload: None,
            buildings_payload: None,
            wars_payload: None,
            divisions_payload: None,
            focus_payload: None,
        };
        assert!(enqueue(&shared, job).is_some());
        assert_eq!(DROPS.load(Ordering::Relaxed), before + 1);
    }

    #[test]
    fn calibration_counters_are_monotonic_and_additive_per_table() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        add_copy_ns(0, 11);
        add_copy_ns(0, 4);
        add_copy_ns(1, 7);
        add_raw_bytes(2, 100);
        DECODE_NS[2].fetch_add(9, Ordering::Relaxed);
        let first = calibration();
        assert_eq!(first.copy_ns, [15, 7, 0]);
        assert_eq!(first.raw_bytes, [0, 0, 100]);
        assert_eq!(first.decode_ns, [0, 0, 9]);
        add_copy_ns(0, 1);
        let second = calibration();
        assert!(second.copy_ns[0] > first.copy_ns[0]);
        assert_eq!(second.copy_ns[1], first.copy_ns[1]);
        let json = second.json();
        assert_eq!(json["copy_ns"][0], 16);
        assert_eq!(json["tables"][1], "ResearchSlots");
        assert!(json["scope"].as_str().unwrap().contains("monotonic"));
    }

    #[test]
    fn worker_decodes_a_refresh_payload_into_the_same_typed_rows() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let (bytes, layout) = research_test_kit::fixture();
        let memory = engine::memory::SliceMemory {
            base: layout.base,
            bytes: &bytes,
        };
        let shared = Shared::new(layout.base, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let direct = research_test_kit::direct(&memory, &layout, 8, 4, 8, 32, 64);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let (report, first) =
            research_test_kit::split(&memory, &layout, frame.extended.research_pilot(), true);
        assert!(report.raw_ok);
        shared.completed[index].store(0, Ordering::Relaxed);
        let job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 2,
            pilot_refresh: true,
            relations_refresh: false,
            characters_refresh: false,
            projects_refresh: false,
            lexical_refresh: false,
            variants_refresh: false,
            buildings_refresh: false,
            wars_refresh: false,
            divisions_refresh: false,
            focus_refresh: false,
            force_full: false,
            game_elapsed: 13,
            payload: Some(first),
            relations_payload: None,
            characters_payload: None,
            projects_payload: None,
            lexical_payload: None,
            variants_payload: None,
            buildings_payload: None,
            wars_payload: None,
            divisions_payload: None,
            focus_payload: None,
        };
        process_job(&shared, &job);
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 13);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        research_test_kit::assert_matches_direct(frame.extended.research_pilot(), &direct);
    }

    #[test]
    fn worker_decodes_a_relations_refresh_payload_into_the_same_typed_rows() {
        use crate::extended::intel::relations_test_kit;
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let bytes = relations_test_kit::opinionated_fixture();
        let memory = relations_test_kit::memory(&bytes);
        let shared = Shared::new(relations_test_kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        // Reference with the pooled caps so truncation behavior matches.
        let opinion_cap = frame.extended.relations_pilot().opinion_rows_cap();
        let tuple_cap = frame.extended.relations_pilot().tuple_rows_cap();
        let direct = relations_test_kit::direct(&memory, opinion_cap, tuple_cap);
        let (report, first) =
            relations_test_kit::split(&memory, frame.extended.relations_pilot(), true);
        assert!(report.raw_ok);
        shared.completed[index].store(0, Ordering::Relaxed);
        let job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 1,
            pilot_refresh: false,
            relations_refresh: true,
            characters_refresh: false,
            projects_refresh: false,
            lexical_refresh: false,
            variants_refresh: false,
            buildings_refresh: false,
            wars_refresh: false,
            divisions_refresh: false,
            focus_refresh: false,
            force_full: false,
            game_elapsed: 17,
            payload: None,
            relations_payload: Some(first),
            characters_payload: None,
            projects_payload: None,
            lexical_payload: None,
            variants_payload: None,
            buildings_payload: None,
            wars_payload: None,
            divisions_payload: None,
            focus_payload: None,
        };
        process_job(&shared, &job);
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 17);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        relations_test_kit::assert_matches_direct(frame.extended.relations_pilot(), &direct);
    }

    #[test]
    fn worker_decodes_a_characters_refresh_payload_into_the_same_typed_rows() {
        use crate::extended::characters::characters_test_kit;
        use crate::extended::characters::{CharactersCopyContext, CharactersJob};
        use replay::frame::TABLE_COUNT;
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let bytes = characters_test_kit::mapped_fixture();
        let memory = characters_test_kit::memory(&bytes);
        let shared = Shared::new(characters_test_kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let capacities = crate::extended::characters::Capacities {
            rows: [4096; 7],
            bytes: [131_072; 7],
        };
        let direct = characters_test_kit::direct(&memory, &capacities);
        let ctx = CharactersCopyContext {
            base: characters_test_kit::BASE,
            game_state: characters_test_kit::GAME_STATE,
            needed: [true; TABLE_COUNT],
        };
        let mut first = CharactersJob::new();
        let mut second = CharactersJob::new();
        let report = frame.extended.characters_pilot().copy_characters_refresh(
            &memory,
            ctx,
            &mut first,
            Some(&mut second),
        );
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 6]);
        shared.completed[index].store(0, Ordering::Relaxed);
        let job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 0,
            pilot_refresh: false,
            relations_refresh: false,
            characters_refresh: true,
            projects_refresh: false,
            lexical_refresh: false,
            variants_refresh: false,
            buildings_refresh: false,
            wars_refresh: false,
            divisions_refresh: false,
            focus_refresh: false,
            force_full: false,
            game_elapsed: 19,
            payload: None,
            relations_payload: None,
            characters_payload: Some(first),
            projects_payload: None,
            lexical_payload: None,
            variants_payload: None,
            buildings_payload: None,
            wars_payload: None,
            divisions_payload: None,
            focus_payload: None,
        };
        process_job(&shared, &job);
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 19);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        characters_test_kit::assert_matches_direct(frame.extended.characters_pilot(), &direct);
        // The worker clears the raw-mode gate it consumed.
        assert!(!frame.extended.characters_pilot().raw_mode);
    }

    #[test]
    fn worker_decodes_a_projects_refresh_payload_into_the_same_typed_rows() {
        use crate::extended::projects::projects_test_kit;
        use crate::extended::projects::{ProjectsCopyContext, ProjectsJob};
        use replay::frame::TABLE_COUNT;
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let bytes = projects_test_kit::fixture();
        let memory = projects_test_kit::memory(&bytes);
        let shared = Shared::new(projects_test_kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let capacities = crate::extended::projects::Capacities {
            rows: [256; 10],
            program_names: 8192,
            program_candidates: 1024,
        };
        let direct = projects_test_kit::direct(&memory, &capacities);
        let ctx = ProjectsCopyContext {
            base: projects_test_kit::BASE,
            game_state: projects_test_kit::GAME_STATE,
            needed: [true; TABLE_COUNT],
        };
        let mut first = ProjectsJob::new();
        let mut second = ProjectsJob::new();
        let report = frame.extended.projects_pilot().copy_projects_refresh(
            &memory,
            ctx,
            &mut first,
            Some(&mut second),
        );
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 4]);
        shared.completed[index].store(0, Ordering::Relaxed);
        let job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 3,
            pilot_refresh: false,
            relations_refresh: false,
            characters_refresh: false,
            projects_refresh: true,
            lexical_refresh: false,
            variants_refresh: false,
            buildings_refresh: false,
            wars_refresh: false,
            divisions_refresh: false,
            focus_refresh: false,
            force_full: false,
            game_elapsed: 23,
            payload: None,
            relations_payload: None,
            characters_payload: None,
            projects_payload: Some(first),
            lexical_payload: None,
            variants_payload: None,
            buildings_payload: None,
            wars_payload: None,
            divisions_payload: None,
            focus_payload: None,
        };
        process_job(&shared, &job);
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 23);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        projects_test_kit::assert_matches_direct(frame.extended.projects_pilot(), &direct);
        // The worker clears the raw-mode gate it consumed.
        assert!(!frame.extended.projects_pilot().raw_mode);
    }

    #[test]
    fn worker_decodes_a_lexical_refresh_payload_into_the_same_typed_rows() {
        use crate::extended::lexical::lexical_test_kit;
        use crate::extended::lexical::{LexicalCopyContext, LexicalJob};
        use replay::frame::TABLE_COUNT;
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let memory = lexical_test_kit::KitMemory::fixture();
        let shared = Shared::new(lexical_test_kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let capacities = crate::extended::lexical::Capacities {
            rows: 256,
            text: 8192,
        };
        let direct = lexical_test_kit::direct(&memory, &capacities);
        let ctx = LexicalCopyContext {
            base: lexical_test_kit::BASE,
            needed: [true; TABLE_COUNT],
        };
        let mut first = LexicalJob::new();
        let mut second = LexicalJob::new();
        let report = frame.extended.lexical_pilot().copy_lexical_refresh(
            &memory,
            ctx,
            &mut first,
            Some(&mut second),
        );
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true]);
        shared.completed[index].store(0, Ordering::Relaxed);
        let job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 3,
            pilot_refresh: false,
            relations_refresh: false,
            characters_refresh: false,
            projects_refresh: false,
            lexical_refresh: true,
            variants_refresh: false,
            buildings_refresh: false,
            wars_refresh: false,
            divisions_refresh: false,
            focus_refresh: false,
            force_full: false,
            game_elapsed: 29,
            payload: None,
            relations_payload: None,
            characters_payload: None,
            projects_payload: None,
            lexical_payload: Some(first),
            variants_payload: None,
            buildings_payload: None,
            wars_payload: None,
            divisions_payload: None,
            focus_payload: None,
        };
        process_job(&shared, &job);
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 29);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        lexical_test_kit::assert_matches_direct(frame.extended.lexical_pilot(), &direct);
        // The worker clears the raw-mode gate it consumed.
        assert!(!frame.extended.lexical_pilot().raw_mode);
    }

    #[test]
    fn worker_decodes_a_variants_refresh_payload_into_the_same_typed_rows() {
        use crate::extended::production::VariantsJob;
        use crate::extended::production::variants_test_kit;
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let bytes = variants_test_kit::fixture();
        let memory = variants_test_kit::memory(&bytes);
        let shared = Shared::new(variants_test_kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let capacities = crate::extended::production::Capacities::default();
        let direct = variants_test_kit::direct(&memory, &capacities);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        // The registry census always runs game-side, including here.
        frame.extended.test_seed_variant_registry(
            &memory,
            variants_test_kit::BASE,
            variants_test_kit::GAME_STATE,
        );
        let needed = [true; replay::frame::TABLE_COUNT];
        let mut first = VariantsJob::new();
        let mut second = VariantsJob::new();
        let report =
            frame
                .extended
                .copy_variants_refresh(&memory, &needed, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 4]);
        shared.completed[index].store(0, Ordering::Relaxed);
        let job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 3,
            pilot_refresh: false,
            relations_refresh: false,
            characters_refresh: false,
            projects_refresh: false,
            lexical_refresh: false,
            variants_refresh: true,
            buildings_refresh: false,
            wars_refresh: false,
            divisions_refresh: false,
            focus_refresh: false,
            force_full: false,
            game_elapsed: 31,
            payload: None,
            relations_payload: None,
            characters_payload: None,
            projects_payload: None,
            lexical_payload: None,
            variants_payload: Some(first),
            buildings_payload: None,
            wars_payload: None,
            divisions_payload: None,
            focus_payload: None,
        };
        process_job(&shared, &job);
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 31);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        variants_test_kit::assert_matches_direct(frame.extended.variants_pilot(), &direct);
        // The worker clears the raw-mode gate it consumed.
        assert!(!frame.extended.variants_pilot().raw_mode);
    }

    #[test]
    fn worker_decodes_a_buildings_refresh_payload_into_the_same_typed_rows() {
        use crate::extended::construction::BuildingsJob;
        use crate::extended::construction::buildings_test_kit;
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let bytes = buildings_test_kit::fixture();
        let memory = buildings_test_kit::memory(&bytes);
        let shared = Shared::new(buildings_test_kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let capacities = crate::extended::construction::Capacities::default();
        let direct = buildings_test_kit::direct(&memory, &capacities);
        let ctx = crate::extended::construction::BuildingsCopyContext {
            game_state: buildings_test_kit::GAME_STATE,
            needed: [true, true],
        };
        let mut first = BuildingsJob::new();
        let mut second = BuildingsJob::new();
        let report = frame.extended.buildings_pilot().copy_buildings_refresh(
            &memory,
            ctx,
            &mut first,
            Some(&mut second),
        );
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 2]);
        shared.completed[index].store(0, Ordering::Relaxed);
        let job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 3,
            pilot_refresh: false,
            relations_refresh: false,
            characters_refresh: false,
            projects_refresh: false,
            lexical_refresh: false,
            variants_refresh: false,
            buildings_refresh: true,
            wars_refresh: false,
            divisions_refresh: false,
            focus_refresh: false,
            force_full: false,
            game_elapsed: 37,
            payload: None,
            relations_payload: None,
            characters_payload: None,
            projects_payload: None,
            lexical_payload: None,
            variants_payload: None,
            buildings_payload: Some(first),
            wars_payload: None,
            divisions_payload: None,
            focus_payload: None,
        };
        process_job(&shared, &job);
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 37);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        buildings_test_kit::assert_matches_direct(frame.extended.buildings_pilot(), &direct);
        // The worker clears the raw-mode gate it consumed.
        assert!(!frame.extended.buildings_pilot().raw_mode);
    }

    #[test]
    fn worker_decodes_a_wars_refresh_payload_into_the_same_typed_rows() {
        use crate::sampling::{WarsCopyContext, WarsJob, copy_wars_refresh, wars_test_kit};
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let bytes = wars_test_kit::fixture();
        let memory = wars_test_kit::memory(&bytes);
        let shared = Shared::new(wars_test_kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let direct = wars_test_kit::direct(&memory, 64);
        let ctx = WarsCopyContext {
            base: wars_test_kit::BASE,
            game_state: wars_test_kit::GAME_STATE,
            needed_matrix: true,
        };
        let mut first = WarsJob::new();
        let mut second = WarsJob::new();
        let report = copy_wars_refresh(&memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 2]);
        // The pooled frame holds more rows than the fixture needs; the
        // replay truncates identically against the real buffers.
        shared.completed[index].store(0, Ordering::Relaxed);
        let job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 41,
            pilot_refresh: false,
            relations_refresh: false,
            characters_refresh: false,
            projects_refresh: false,
            lexical_refresh: false,
            variants_refresh: false,
            buildings_refresh: false,
            wars_refresh: true,
            divisions_refresh: false,
            focus_refresh: false,
            force_full: false,
            game_elapsed: 43,
            payload: None,
            relations_payload: None,
            characters_payload: None,
            projects_payload: None,
            lexical_payload: None,
            variants_payload: None,
            buildings_payload: None,
            wars_payload: Some(first),
            divisions_payload: None,
            focus_payload: None,
        };
        process_job(&shared, &job);
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 43);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        assert_eq!(frame.wars_result, direct.owned_outcome);
        assert_eq!(frame.war_matrix_result, Some(direct.matrix_outcome));
        assert_eq!(&frame.wars[..direct.owned.len()], &direct.owned[..]);
        assert_eq!(
            &frame.wars_matrix[..direct.matrix.len()],
            &direct.matrix[..]
        );
        // Wars decode is hourly with no refresh gate: every hour stages.
        assert_eq!(WARS, [TableId::WarRelations]);
    }

    #[test]
    fn worker_decodes_a_divisions_refresh_payload_into_the_same_typed_rows() {
        use crate::sampling::{
            DivisionsCopyContext, DivisionsJob, copy_divisions_refresh, divisions_test_kit,
        };
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let bytes = divisions_test_kit::fixture();
        let memory = divisions_test_kit::memory(&bytes);
        let shared = Shared::new(divisions_test_kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let direct = divisions_test_kit::direct(&memory, [64; 6]);
        let ctx = DivisionsCopyContext {
            base: divisions_test_kit::BASE,
            game_state: divisions_test_kit::GAME_STATE,
        };
        let mut first = DivisionsJob::new();
        let mut second = DivisionsJob::new();
        let report = copy_divisions_refresh(&memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 6]);
        // The pooled frame holds more rows than the fixture needs; the
        // replay truncates identically against the real buffers.
        shared.completed[index].store(0, Ordering::Relaxed);
        let job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 42,
            pilot_refresh: false,
            relations_refresh: false,
            characters_refresh: false,
            projects_refresh: false,
            lexical_refresh: false,
            variants_refresh: false,
            buildings_refresh: false,
            wars_refresh: false,
            divisions_refresh: true,
            focus_refresh: false,
            force_full: false,
            game_elapsed: 47,
            payload: None,
            relations_payload: None,
            characters_payload: None,
            projects_payload: None,
            lexical_payload: None,
            variants_payload: None,
            buildings_payload: None,
            wars_payload: None,
            divisions_payload: Some(first),
            focus_payload: None,
        };
        process_job(&shared, &job);
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 47);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        assert_eq!(frame.divisions_result, direct.divisions_outcome);
        assert_eq!(
            &frame.divisions[..direct.divisions.len()],
            &direct.divisions[..]
        );
        let detail = frame.extended.test_division_detail();
        assert_eq!(detail.division_equipment.outcome, direct.equipment_outcome);
        assert_eq!(
            &detail.division_equipment.rows[..direct.equipment.len()],
            &direct.equipment[..]
        );
        assert_eq!(detail.groups.outcome, direct.groups_outcome);
        assert_eq!(
            &detail.groups.rows[..direct.groups.len()],
            &direct.groups[..]
        );
        assert_eq!(detail.requests.outcome, direct.requests_outcome);
        assert_eq!(
            &detail.requests.rows[..direct.requests.len()],
            &direct.requests[..]
        );
        assert_eq!(detail.deliveries.outcome, direct.deliveries_outcome);
        assert_eq!(
            &detail.deliveries.rows[..direct.deliveries.len()],
            &direct.deliveries[..]
        );
        assert_eq!(detail.produced.outcome, direct.produced_outcome);
        assert_eq!(
            &detail.produced.rows[..direct.produced.len()],
            &direct.produced[..]
        );
        // The worker clears the raw-mode gate it consumed.
        assert!(!detail.divisions_raw_mode);
        // Divisions decode is hourly with no refresh gate.
        assert_eq!(
            DIVISIONS,
            [
                TableId::Divisions,
                TableId::DivisionEquipment,
                TableId::ArmyUpgradeGroups,
                TableId::ArmyUpgradeRequests,
                TableId::ArmyUpgradeDeliveries,
                TableId::ArmyUpgradeProduced,
            ]
        );
    }

    #[test]
    fn payload_pools_cover_analytic_worst_cases() {
        use crate::extended::{
            characters::CharactersJob, construction::BuildingsJob, governance::ResearchJob,
            intel::RelationsJob, lexical::LexicalJob, production::VariantsJob,
            projects::ProjectsJob,
        };
        // Analytic worst cases from the 2026-09-11 appendix (late-campaign
        // maxima): research 242328 instances / 1133 slots / 552 templates;
        // relations 8639 opinions / 1512 tuples; characters 13127 rows with
        // 19744 experience rows; projects 21511 rows with 1756 breakthroughs;
        // lexical 70851 tokens; variants 6354 definitions with 10975 modules
        // and 7480 upgrades; buildings 2941 + 5220 rows; wars 535 relations
        // over 439 countries; divisions 2843 with 17364 equipment rows and
        // about 4000 upgrade rows. Every cap below must exceed its shape's
        // span/byte cost so staging overflow — which takes the existing
        // whole-frame gap branch — never triggers in steady state. A
        // force-full hour stages every family at once; the combined live set
        // stays under half the measured 513 MB pool peak (per-capture
        // reports), so the split never exceeds the memory it replaces.
        let research = ResearchJob::new();
        assert!(research.tech_pool.capacity() >= 242_328 * 52);
        assert!(research.researched_pool.capacity() >= 242_328 * 16);
        assert!(research.templates.capacity() >= 552 * 439);
        assert!(research.id_pool.capacity() >= 242_328 * 16);
        assert!(research.slots.capacity() >= 1133 * 2);
        assert!(research.metas.capacity() >= 439);
        let relations = RelationsJob::new();
        assert!(relations.slot_pool.capacity() >= 8639 * 512);
        assert!(relations.tuples.capacity() >= 1512 * 4);
        assert!(relations.countries.capacity() >= 439);
        assert!(relations.metas.capacity() >= 439);
        let characters = CharactersJob::new();
        assert!(characters.spans.capacity() >= 300_000);
        assert!(characters.bytes.capacity() >= 10 << 20);
        let projects = ProjectsJob::new();
        assert!(projects.log.spans.capacity() >= 21_511 * 32);
        assert!(projects.log.bytes.capacity() >= 21_511 * 1024);
        let lexical = LexicalJob::new();
        assert!(lexical.log.spans.capacity() >= 70_851 + 16);
        assert!(lexical.log.bytes.capacity() >= 70_851 * 32 * 3);
        let variants = VariantsJob::new();
        assert!(variants.log.spans.capacity() >= (6354 + 10_975 + 7480) * 8);
        assert!(variants.log.bytes.capacity() >= (6354 + 10_975 + 7480) * 256);
        let buildings = BuildingsJob::new();
        assert!(buildings.log.spans.capacity() >= (2941 + 5220) * 16);
        assert!(buildings.log.bytes.capacity() >= (2941 + 5220) * 512);
        // Wars steady state is sparse (the 1.4 ms appendix walk proves it:
        // a dense 439-squared matrix would cost an order of magnitude more
        // to read). A pathological dense matrix (~800k spans) correctly
        // falls back to the typed walk instead of fitting.
        let wars = crate::sampling::WarsJob::new();
        assert!(wars.log.spans.capacity() >= 60_000);
        assert!(wars.log.bytes.capacity() >= 2 << 20);
        let divisions = crate::sampling::DivisionsJob::new();
        assert!(divisions.log.spans.capacity() >= 150_000);
        assert!(divisions.log.bytes.capacity() >= 4 << 20);
        // Hourly focus worst case is 439 countries with about 3.9k completed
        // rows plus token bytes (the appendix census proves the shape).
        let focus = FocusJob::new();
        assert!(focus.log.spans.capacity() >= 18_000);
        assert!(focus.log.bytes.capacity() >= 1 << 20);
        // Combined worst-case live set (one staged job per family, as on a
        // force-full hour) stays under half the measured pool peak.
        let combined = research.tech_pool.capacity()
            + research.researched_pool.capacity()
            + research.id_pool.capacity()
            + relations.slot_pool.capacity()
            + characters.bytes.capacity()
            + projects.log.bytes.capacity()
            + lexical.log.bytes.capacity()
            + variants.log.bytes.capacity()
            + buildings.log.bytes.capacity()
            + wars.log.bytes.capacity()
            + divisions.log.bytes.capacity()
            + focus.log.spans.capacity()
            + focus.log.bytes.capacity();
        assert!(combined < 256 << 20, "combined caps {combined}");
        // The FIFO holds one lightweight job per pooled buffer by
        // construction (see the pool-invariant test); 256 slots absorb any
        // realistic stall with the high-water above proving the depth live.
        assert_eq!(BUFFERS, 256);
    }

    #[test]
    fn queue_high_water_tracks_the_deepest_observed_fifo_occupancy() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let shared = Shared::new(0, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        for _ in 0..3 {
            let job = DecodeJob {
                index,
                seq: next_seq(),
                hour: 0,
                pilot_refresh: false,
                relations_refresh: false,
                characters_refresh: false,
                projects_refresh: false,
                lexical_refresh: false,
                variants_refresh: false,
                buildings_refresh: false,
                wars_refresh: false,
                divisions_refresh: false,
                focus_refresh: false,
                force_full: false,
                game_elapsed: 1,
                payload: None,
                relations_payload: None,
                characters_payload: None,
                projects_payload: None,
                lexical_payload: None,
                variants_payload: None,
                buildings_payload: None,
                wars_payload: None,
                divisions_payload: None,
                focus_payload: None,
            };
            assert!(enqueue(&shared, job).is_none());
        }
        let snapshot = calibration();
        assert_eq!(snapshot.queue_high_water, 3);
        assert_eq!(snapshot.json()["queue_high_water"], 3);
        assert_eq!(snapshot.json()["queue_capacity"], BUFFERS);
        while shared.decode_queue.pop().is_some() {}
    }

    #[test]
    fn worker_decodes_an_hourly_focus_payload_into_the_same_typed_rows() {
        use crate::extended::governance::focus_test_kit;
        use replay::frame::TABLE_COUNT;
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let fixture = focus_test_kit::Fixture::world();
        let memory = fixture.memory();
        let shared = Shared::new(focus_test_kit::BASE, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let needed = [true; TABLE_COUNT];
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let mut typed = focus_test_kit::Fixture::captured();
        typed.capture(&memory, focus_test_kit::BASE, 0x1_0000, &needed, 0, true);
        frame.extended.focus_pilot().focus_raw_mode = true;
        let (report, first) =
            focus_test_kit::Fixture::copy_first(frame.extended.focus_pilot(), &memory, None);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true, true]);
        shared.completed[index].store(0, Ordering::Relaxed);
        let job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 7,
            pilot_refresh: false,
            relations_refresh: false,
            characters_refresh: false,
            projects_refresh: false,
            lexical_refresh: false,
            variants_refresh: false,
            buildings_refresh: false,
            wars_refresh: false,
            divisions_refresh: false,
            focus_refresh: true,
            force_full: false,
            game_elapsed: 23,
            payload: None,
            relations_payload: None,
            characters_payload: None,
            projects_payload: None,
            lexical_payload: None,
            variants_payload: None,
            buildings_payload: None,
            wars_payload: None,
            divisions_payload: None,
            focus_payload: Some(first),
        };
        process_job(&shared, &job);
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 23);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let pilot = frame.extended.focus_pilot();
        assert!(!pilot.focus_raw_mode);
        focus_test_kit::Fixture::assert_matches_typed(pilot, &typed);
        let snapshot = calibration();
        assert_eq!(snapshot.focus_copy_ns, [0, 0]);
        assert!(snapshot.refresh_jobs >= 1);
        assert_eq!(snapshot.json()["focus_tables"][0], "Focus");
        assert_eq!(snapshot.json()["focus_tables"][1], "CompletedFocuses");
    }

    #[test]
    fn focus_calibration_counters_are_monotonic_and_additive_per_table() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        add_focus_copy_ns(0, 11);
        add_focus_copy_ns(0, 4);
        add_focus_copy_ns(1, 0);
        add_focus_raw_bytes(1, 100);
        let snapshot = calibration();
        assert_eq!(snapshot.focus_copy_ns, [15, 0]);
        assert_eq!(snapshot.focus_raw_bytes, [0, 100]);
        assert_eq!(snapshot.json()["focus_copy_ns"][0], 15);
        assert_eq!(snapshot.json()["focus_tables"][0], "Focus");
        // Recycled jobs keep their capacities for the next hour.
        let job = acquire_focus_job();
        assert!(job.log.spans.capacity() >= 1 << 16);
        recycle_focus_job(job);
        let again = acquire_focus_job();
        assert!(again.log.spans.capacity() >= 1 << 16);
        recycle_focus_job(again);
    }
}
