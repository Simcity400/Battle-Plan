//! Strategic state acquisition. Replay conversion and buffer growth run off-thread.

use crate::{
    raw::RawLog,
    read_accounting,
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows, grow_capacity, grow_vec},
};
use engine::{
    Memory, ccountry, ccountry_raid_status, cgame_state, cnuke, cpdx_array::Array, craid_instance,
    craid_province_entries, craid_system, msvc_string, nproject_breakthrough_node,
    nproject_cbreakthrough_progress, nproject_cprogram, nproject_cprogram_status,
    nproject_cproject, nproject_cproject_pool, nproject_cproject_state_machine,
    nproject_sproject_context, nproject_sproject_history, peace_conference, scientist_roster,
    walk::Outcome,
};
use replay::{
    frame::TableView,
    rows::{self, TableId, tables},
};
use std::mem::size_of;

pub const N: usize = 12;
pub const TABLES: [TableId; N] = [
    TableId::ProjectPrograms,
    TableId::Projects,
    TableId::ProjectHistory,
    TableId::ProjectBreakthroughs,
    TableId::Nukes,
    TableId::RaidCountries,
    TableId::Raids,
    TableId::PeaceConferences,
    TableId::Scientists,
    TableId::ScientistSkills,
    TableId::RaidTargets,
    TableId::RaidProvinces,
];

/// Each rooted walk produces two independently bounded tables.
/// Its entire cost is attributed once to the anchor named by each group.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[
    ("project pool and reward histories", 1, &[1, 2]),
    // RaidTargets is the raid instance walk's second projection: the same rows
    // carry the map icon's operands, so it joins that walk's cost group.
    ("raid countries and instances", 5, &[5, 6, 10]),
    ("scientist rosters and specializations", 8, &[8, 9]),
    ("raid province registrations", 11, &[11]),
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    pub rows: [usize; N],
    pub program_names: usize,
    pub program_candidates: usize,
}

impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: [
                256, 32768, 1024, 2048, 1024, 512, 256, 32, 2048, 8192, 128, 4096,
            ],
            program_names: 8192,
            program_candidates: 1024,
        }
    }
}
impl Capacities {
    /// RaidCountries has one row per canonical slot and Nukes exactly two
    /// per country (`cnuke::TYPE_COUNT`); Projects holds each country's pool
    /// of special-project definitions (the default's per-country factor).
    pub(crate) fn raise_for_countries(&mut self, slots: usize) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.rows[1], rows(slots, 64));
        raise(&mut self.rows[4], rows(slots, 2));
        raise(&mut self.rows[5], rows(slots, 1));
    }
}

pub struct Captured {
    programs: CaptureRows<nproject_cprogram::Row>,
    projects: CaptureRows<nproject_cproject::Row>,
    history: CaptureRows<nproject_sproject_history::Row>,
    breakthroughs: CaptureRows<nproject_cbreakthrough_progress::Row>,
    nukes: CaptureRows<cnuke::Row>,
    raid_countries: CaptureRows<ccountry_raid_status::Row>,
    raids: CaptureRows<craid_instance::Row>,
    peace: CaptureRows<peace_conference::Row>,
    scientists: CaptureRows<scientist_roster::Row>,
    scientist_skills: CaptureRows<scientist_roster::SkillRow>,
    raid_provinces: CaptureRows<craid_province_entries::Row>,
    names: Arena<u8>,
    candidates: Vec<usize>,
    required_candidates: usize,
    /// Raw-copy split gate for the four daily project tables (0..4).
    /// Set when an acquisition job returns a valid refresh payload;
    /// `capture` then skips their typed walks and the decode worker
    /// fills these rows before the ready handoff. Preserved across `reset`
    /// (set per refresh, cleared by the worker after decode). Hourly tables
    /// are never gated by this flag.
    pub(crate) raw_mode: bool,
}

impl Captured {
    /// Fail the typed job's tables without erasing its separately staged projects.
    pub(crate) fn fail_gated_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        let outcomes = TABLES.map(|table| super::failed_capture(table, hour, force_full, reason));
        if !self.raw_mode {
            self.programs.fail(outcomes[0]);
            self.projects.fail(outcomes[1]);
            self.history.fail(outcomes[2]);
            self.breakthroughs.fail(outcomes[3]);
        }
        self.nukes.fail(outcomes[4]);
        self.raid_countries.fail(outcomes[5]);
        self.raids.fail(outcomes[6]);
        self.peace.fail(outcomes[7]);
        self.scientists.fail(outcomes[8]);
        self.scientist_skills.fail(outcomes[9]);
        self.raid_provinces.fail(outcomes[11]);
    }

    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: [0; N],
            program_names: 0,
            program_candidates: 0,
        })
    }

    /// All capacity, outcome and cost arrays use local TABLES order.
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            programs: CaptureRows::new(capacity.rows[0]),
            projects: CaptureRows::new(capacity.rows[1]),
            history: CaptureRows::new(capacity.rows[2]),
            breakthroughs: CaptureRows::new(capacity.rows[3]),
            nukes: CaptureRows::new(capacity.rows[4]),
            raid_countries: CaptureRows::new(capacity.rows[5]),
            raids: CaptureRows::new(capacity.rows[6]),
            peace: CaptureRows::new(capacity.rows[7]),
            scientists: CaptureRows::new(capacity.rows[8]),
            scientist_skills: CaptureRows::new(capacity.rows[9]),
            raid_provinces: CaptureRows::new(capacity.rows[11]),
            names: Arena::new(capacity.program_names),
            candidates: vec![0; capacity.program_candidates],
            required_candidates: 0,
            raw_mode: false,
        }
    }

    pub fn reset(&mut self) {
        self.programs.reset();
        self.projects.reset();
        self.history.reset();
        self.breakthroughs.reset();
        self.nukes.reset();
        self.raid_countries.reset();
        self.raids.reset();
        self.peace.reset();
        self.scientists.reset();
        self.scientist_skills.reset();
        self.raid_provinces.reset();
        self.names.reset();
        self.required_candidates = 0;
    }

    pub fn allocated_bytes(&self) -> usize {
        self.programs.allocated_bytes()
            + self.projects.allocated_bytes()
            + self.history.allocated_bytes()
            + self.breakthroughs.allocated_bytes()
            + self.nukes.allocated_bytes()
            + self.raid_countries.allocated_bytes()
            + self.raids.allocated_bytes()
            + self.peace.allocated_bytes()
            + self.scientists.allocated_bytes()
            + self.scientist_skills.allocated_bytes()
            + self.raid_provinces.allocated_bytes()
            + self.names.allocated_bytes()
            + self.candidates.capacity() * size_of::<usize>()
    }

    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.programs.grow(&mut capacity.rows[0]);
        self.projects.grow(&mut capacity.rows[1]);
        self.history.grow(&mut capacity.rows[2]);
        self.breakthroughs.grow(&mut capacity.rows[3]);
        self.nukes.grow(&mut capacity.rows[4]);
        self.raid_countries.grow(&mut capacity.rows[5]);
        self.raids.grow(&mut capacity.rows[6]);
        self.peace.grow(&mut capacity.rows[7]);
        self.scientists.grow(&mut capacity.rows[8]);
        self.scientist_skills.grow(&mut capacity.rows[9]);
        self.raid_provinces.grow(&mut capacity.rows[11]);
        self.names.grow(&mut capacity.program_names);
        grow_capacity(&mut capacity.program_candidates, self.required_candidates);
        if self.required_candidates > 0
            && self.required_candidates
                >= capacity
                    .program_candidates
                    .saturating_sub(capacity.program_candidates / 4)
        {
            let required = capacity.program_candidates.saturating_add(1);
            grow_capacity(&mut capacity.program_candidates, required);
        }
        grow_vec(&mut self.candidates, capacity.program_candidates);
    }

    /// Acquisition uses preallocated buffers while the hourly callback waits.
    /// Definition/reward/specialization tokens retain their native u32 bits.
    /// Acquire current rows using the existing staggered project cadence.
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.reset();
        let mut ticks = [0; N];
        let active = TABLES.map(|table| replay::cadence::should_walk(table, hour, force_full));
        let walk = active[1] && !self.raw_mode;
        if self.raw_mode && active[1] {
            // The owned refresh supplies these rows before publication.
        } else if !walk {
            let skipped = Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON,
            };
            self.programs.outcome = skipped;
            self.projects.outcome = skipped;
            self.history.outcome = skipped;
            self.breakthroughs.outcome = skipped;
        } else {
            let begin = qpc();
            self.capture_programs(|candidates, rows, names| {
                let _reads = read_accounting::table(TABLES[0]);
                nproject_cprogram::walk(memory, base, game_state, candidates, rows, names)
            });
            ticks[0] = elapsed(begin);
            let begin = qpc();
            capture_pair(
                &mut self.projects,
                &mut self.history,
                |projects, histories| {
                    let _reads = read_accounting::table(TABLES[1]);
                    nproject_cproject::walk_with_history(
                        memory, base, game_state, projects, histories,
                    )
                },
            );
            ticks[1] = elapsed(begin);
            let begin = qpc();
            capture_rows(&mut self.breakthroughs, |rows| {
                let _reads = read_accounting::table(TABLES[3]);
                nproject_cbreakthrough_progress::walk(memory, base, game_state, rows)
            });
            ticks[3] = elapsed(begin);
        }
        macro_rules! plain {
            ($field:ident, $index:literal, $module:ident) => {{
                if active[$index] {
                    let _reads = read_accounting::table(TABLES[$index]);
                    let begin = qpc();
                    capture_rows(&mut self.$field, |rows| {
                        $module::walk(memory, base, game_state, rows)
                    });
                    ticks[$index] = elapsed(begin);
                } else {
                    self.$field.outcome = Outcome::Unavailable {
                        reason: replay::cadence::SKIPPED_REASON,
                    };
                }
            }};
        }
        plain!(nukes, 4, cnuke);
        // Slot 10 (RaidTargets) is the same walk as slot 6: one read, two tables.
        if active[5] || active[10] {
            let begin = qpc();
            capture_pair(
                &mut self.raid_countries,
                &mut self.raids,
                |parents, instances| {
                    let _reads = read_accounting::table(TABLES[5]);
                    craid_system::walk(memory, base, game_state, parents, instances)
                },
            );
            ticks[5] = elapsed(begin);
        } else {
            self.raid_countries.outcome = Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON,
            };
            self.raids.outcome = self.raid_countries.outcome;
        }
        if active[11] {
            let begin = qpc();
            capture_rows(&mut self.raid_provinces, |rows| {
                let _reads = read_accounting::table(TABLES[11]);
                craid_province_entries::walk(memory, base, game_state, rows)
            });
            ticks[11] = elapsed(begin);
        } else {
            self.raid_provinces.outcome = Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON,
            };
        }
        plain!(peace, 7, peace_conference);
        if active[8] {
            let begin = qpc();
            capture_pair(
                &mut self.scientists,
                &mut self.scientist_skills,
                |scientists, skills| {
                    let _reads = read_accounting::table(TABLES[8]);
                    let walk = scientist_roster::walk(
                        memory,
                        base,
                        game_state,
                        &mut scientist_roster::Buffers { scientists, skills },
                    );
                    (walk.scientists, walk.skills)
                },
            );
            ticks[8] = elapsed(begin);
        } else {
            self.scientists.outcome = Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON,
            };
            self.scientist_skills.outcome = self.scientists.outcome;
        }
        ticks
    }

    fn capture_programs(
        &mut self,
        walk: impl FnOnce(
            &mut [usize],
            &mut [nproject_cprogram::Row],
            &mut [u8],
        ) -> nproject_cprogram::NameWalk,
    ) {
        let result = walk(
            &mut self.candidates,
            &mut self.programs.rows,
            &mut self.names.first,
        );
        self.programs.outcome = program_outcome(result, &self.programs.rows, &self.names.first);
        self.names.len = result.bytes_len;
        self.names.required = result.required_bytes;
        self.required_candidates = result.required_candidates;
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.programs.outcome,
            self.projects.outcome,
            self.history.outcome,
            self.breakthroughs.outcome,
            self.nukes.outcome,
            self.raid_countries.outcome,
            self.raids.outcome,
            self.peace.outcome,
            self.scientists.outcome,
            self.scientist_skills.outcome,
            self.raids.outcome,
            self.raid_provinces.outcome,
        ]
    }

    pub fn tables(&self) -> [TableView; N] {
        let outcomes = self.outcomes();
        [
            self.program_table(outcomes[0]),
            table::<tables::Projects, _>(outcomes[1], &self.projects.rows, |r| rows::ProjectRow {
                owner: r.owner,
                local_id: r.local_id,
                definition_id: r.definition_id,
                program_reference: r.program_reference,
                started: r.started,
                completed: r.completed,
                state: rows::ProjectState {
                    current_offset: r.state.current_offset,
                    state_enums: r.state.state_enums,
                    not_completed: r.state.not_completed,
                    phase_progress: r.state.phase_progress,
                    phase_target: r.state.phase_target,
                    prototype_progress: r.state.prototype_progress,
                    iterations: r.state.iterations,
                },
            }),
            table::<tables::ProjectHistory, _>(outcomes[2], &self.history.rows, |r| {
                rows::ProjectHistoryRow {
                    owner: r.owner,
                    project: r.project,
                    ordinal: r.ordinal,
                    prototype_reward_handle: r.prototype_reward_handle,
                    option_handle: r.option_handle,
                    date: r.date,
                    context: rows::ProjectContext {
                        province: r.context.province,
                        progress: r.context.progress,
                        character_reference: r.context.character_reference,
                        country_present: r.context.country_present,
                        country_reference: r.context.country_reference,
                        scientist_present: r.context.scientist_present,
                        scientist_reference: r.context.scientist_reference,
                        state_present: r.context.state_present,
                        state_reference: r.context.state_reference,
                    },
                }
            }),
            table::<tables::ProjectBreakthroughs, _>(outcomes[3], &self.breakthroughs.rows, |r| {
                rows::ProjectBreakthroughRow {
                    owner: r.owner,
                    specialization: r.specialization,
                    points: r.points,
                }
            }),
            table::<tables::Nukes, _>(outcomes[4], &self.nukes.rows, |r| rows::NukeRow {
                country: r.country,
                nuke_type: r.nuke_type,
                amount_raw: r.amount_raw,
                ready: r.ready,
            }),
            table::<tables::RaidCountries, _>(outcomes[5], &self.raid_countries.rows, |r| {
                rows::RaidCountryRow {
                    canonical_slot: r.canonical_slot,
                    country: r.country,
                    priority: r.priority,
                    child_count: r.child_count,
                }
            }),
            table::<tables::Raids, _>(outcomes[6], &self.raids.rows, |r| rows::RaidRow {
                reference_type: r.reference_type,
                id: r.id,
                parent_slot: r.parent_slot,
                ordinal: r.ordinal,
                preparation_hours: r.preparation_hours,
                phase: r.phase,
                outcome: r.outcome,
                end_date_present: r.end_date_present,
                victim_country: r.victim_country,
                source: rows::RaidSource {
                    kind: r.source.kind,
                    kind_present: r.source.kind_present,
                    province: r.source.province,
                    ship_reference_type: r.source.ship_reference_type,
                    ship_reference_id: r.source.ship_reference_id,
                    country: r.source.country,
                },
            }),
            table::<tables::PeaceConferences, _>(outcomes[7], &self.peace.rows, |r| {
                rows::PeaceConferenceRow {
                    id: r.id,
                    ordinal: r.ordinal,
                    current_turn: r.current_turn,
                    finalized: r.finalized,
                }
            }),
            table::<tables::Scientists, _>(outcomes[8], &self.scientists.rows, |r| {
                rows::ScientistRow {
                    character: r.character,
                    country: r.country,
                    ordinal: r.ordinal,
                    trait_count: r.trait_count,
                    skill_count: r.skill_count,
                }
            }),
            table::<tables::ScientistSkills, _>(outcomes[9], &self.scientist_skills.rows, |r| {
                rows::ScientistSkillRow {
                    character: r.character,
                    ordinal: r.ordinal,
                    token: r.token,
                    level: r.level,
                    experience_raw: r.experience_raw,
                }
            }),
            // The raid instance rows carry the map icon's own operands.
            table::<tables::RaidTargets, _>(outcomes[10], &self.raids.rows, |r| {
                rows::RaidTargetRow {
                    reference_type: r.reference_type,
                    id: r.id,
                    province: r.target.province,
                    leader_province: r.target.leader_province,
                    state: r.target.state,
                    building_type: r.target.building_type,
                    building_location: r.target.building_location,
                    building_flags: r.target.building_flags,
                    raid_type: r.raid_type,
                    detected: r.detected,
                }
            }),
            table::<tables::RaidProvinces, _>(outcomes[11], &self.raid_provinces.rows, |r| {
                rows::RaidProvinceRow {
                    province: r.province,
                    targets: r.targets,
                    subset: r.subset,
                    owned: r.owned,
                    incoming: r.incoming,
                    words: r.words().to_vec(),
                    dropped_words: r.dropped_words,
                    offers: r.offers().to_vec(),
                    offer_count: r.offer_count,
                }
            }),
        ]
    }

    fn program_table(&self, outcome: Outcome) -> TableView {
        if self.names.required < self.names.len
            || !valid_names(
                outcome,
                &self.programs.rows,
                &self.names.first,
                self.names.len,
            )
        {
            return unavailable("program building range exceeds captured arena");
        }
        table::<tables::ProjectPrograms, _>(outcome, &self.programs.rows, |r| {
            rows::ProjectProgramRow {
                id: r.id,
                owner: r.owner,
                province: r.province,
                active_project: r.active_project,
                scientist: r.scientist,
                unread_rewards: r.unread_rewards,
                building: self.names.first[r.building_start..r.building_start + r.building_len]
                    .to_vec(),
            }
        })
    }
}

fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}

// ===========================================================================
// Scheduled raw-copy split (ProjectPrograms, Projects, ProjectHistory,
// ProjectBreakthroughs; independent tables use separate typed acquisition).
//
// The acquisition worker copies raw bytes: one shared country census, program
// pointers/rows/strings, project pool records with state machines and
// history entries, and breakthrough tree nodes. The single FIFO decode worker
// replays the existing typed
// walks against the recorded spans (`crate::raw::Recorded`), so typed rows,
// ordinals, skips, truncation counts and failure reasons are identical by
// construction (see `crate::raw` for the mirror rules).
//
// Census sharing: the three typed walks each run their own country census,
// but the read sets are identical, so the mirror records one census and all
// three replays read it from the log. Inner `resolve` calls likewise reuse
// the census's authentication spans and record only their link span.
// ===========================================================================

/// Fault-attribution bits, one per project table in `TABLES[0..4]` order.
const M_PROGRAMS: u32 = 0x01;
const M_PROJECTS: u32 = 0x02;
const M_HISTORY: u32 = 0x04;
const M_BREAKTHROUGH: u32 = 0x08;
const M_ALL: u32 = 0x0f;
const M_PROJECT_TABLES: u32 = M_PROJECTS | M_HISTORY;

/// Project identity header length (`HISTORY_COUNT + 4`); the engine keeps
/// this private, so the mirror derives it from the public count constant.
const PROJECT_HEADER_LEN: usize = nproject_cproject::HISTORY_COUNT + 4;
/// Project lifecycle flag length (`RECORD_SIZE - HEADER_LEN`).
const PROJECT_FLAG_LEN: usize = 2;
/// Project batch size; the engine documents it as error-precedence state.
const PROJECT_BATCH: u32 = 16;

/// One refresh of raw project payload. The census scratch is reused across
/// passes without allocation; growth happens on the decode side only (see
/// `crate::decode::recycle_projects_job`).
#[derive(Debug, Default)]
pub(crate) struct ProjectsJob {
    pub log: RawLog<4>,
    pub game_state: usize,
    pub active: bool,
    census: Vec<CensusEntry>,
    pointers: Vec<usize>,
    capacity_charge: crate::raw_memory::Charge,
}

#[derive(Debug, Default, Clone, Copy)]
struct CensusEntry {
    pointer: usize,
    id: u32,
    slot: u32,
    canonical: bool,
}

/// Shared census borrowed by every scheduled project walk mirror.
#[derive(Clone, Copy)]
struct ProjectCensus<'a> {
    links: &'a Array,
    countries: u32,
    census: &'a [CensusEntry],
}

impl ProjectsJob {
    pub fn new() -> Self {
        let mut job = Self {
            // Late-campaign worst case is about 21k projects with state
            // machines plus history rows; both pools start above that so
            // steady state never overflows.
            log: RawLog::new(1 << 20, 48 << 20),
            game_state: 0,
            active: false,
            census: Vec::with_capacity(crate::country_scale::census_rows(512)),
            pointers: Vec::with_capacity(32768),
            capacity_charge: crate::raw_memory::Charge::default(),
        };
        job.update_capacity_charge();
        job
    }

    /// Reset lengths for reuse, keeping every capacity. No allocation.
    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
        self.active = false;
        self.census.clear();
        self.pointers.clear();
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
    }

    /// Double every pool off the game thread until copies fit. An overflowed
    /// accepted refresh stays unavailable; growth serves the next acquisition.
    pub fn grow(&mut self) {
        self.log.grow();
        crate::raw::double_capacity(&mut self.census);
        crate::raw::double_capacity(&mut self.pointers);
        self.update_capacity_charge();
    }

    /// Off the game thread: hold one census row per observed country slot so
    /// the first hour after a large save loads does not overflow.
    pub fn presize_countries(&mut self) {
        if crate::country_scale::reserve_census(&mut self.census) {
            self.update_capacity_charge();
        }
    }

    fn update_capacity_charge(&mut self) {
        use crate::raw_memory::{Capacity, vector_bytes};
        self.capacity_charge.set(Capacity {
            auxiliary_vectors: vector_bytes(&self.census) + vector_bytes(&self.pointers),
            ..Default::default()
        });
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ProjectsCopyReport {
    /// False when staging overflowed. The accepted acquisition cannot be
    /// replayed and must fail without a second live walk.
    pub raw_ok: bool,
    pub copy_ticks: [u64; 4],
    pub raw_bytes: [u64; 4],
}

/// Acquisition-job parameters supplied at the sampling boundary.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ProjectsCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub active: bool,
}

fn pool_u32(log: &RawLog<4>, start: usize) -> u32 {
    u32::from_le_bytes(log.bytes[start..start + 4].try_into().unwrap())
}

fn pool_u64(log: &RawLog<4>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

/// One mirrored span read. `None` addrs (address arithmetic overflow) record
/// nothing because the typed walk fails locally without reads; every other
/// failure is logged with the affected-table mask.
fn span(
    log: &mut RawLog<4>,
    memory: &impl Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

/// Mirror of `walk_tables`' read set for one country: pool header, batched
/// identity/flag records, state machines and history entries. Pooled
/// capacities are ignored (a superset is recorded); validation, precedence
/// and error scope are local parses, so any failure aborts the walk exactly
/// like the typed error.
fn copy_project_tables(
    log: &mut RawLog<4>,
    memory: &impl Memory,
    base: usize,
    links: &Array,
    countries: u32,
    census: &[CensusEntry],
) {
    let expected = match base.checked_add(nproject_cproject::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return,
    };
    let shared = ProjectCensus {
        links,
        countries,
        census,
    };
    for country in census.iter().filter(|entry| entry.canonical) {
        let Some(status) = copy_status(log, memory, base, shared, country, M_PROJECT_TABLES) else {
            return;
        };
        let Some(pool_start) = span(
            log,
            memory,
            status.checked_add(nproject_cprogram_status::PROJECT_POOL),
            8,
            M_PROJECT_TABLES,
        ) else {
            return;
        };
        let pool = match usize::try_from(pool_u64(log, pool_start))
            .ok()
            .filter(|&p| p != 0)
        {
            Some(pool) => pool,
            None => return,
        };
        let Some(pool_vtable) = span(log, memory, Some(pool), 8, M_PROJECT_TABLES) else {
            return;
        };
        if pool_u64(log, pool_vtable)
            != base
                .checked_add(nproject_cproject_pool::VTABLE_RVA)
                .unwrap_or(0) as u64
        {
            return;
        }
        let Some(pool_array) = span(
            log,
            memory,
            pool.checked_add(nproject_cproject_pool::PROJECTS),
            16,
            M_PROJECT_TABLES,
        ) else {
            return;
        };
        let mut raw = [0u8; 16];
        raw.copy_from_slice(&log.bytes[pool_array..pool_array + 16]);
        let pool_array = match Array::from_header(raw, nproject_cproject::POOL_STRIDE) {
            Some(array) => array,
            None => return,
        };
        let mut authenticated_owner = country.id;
        let mut hist_dead = false;
        let mut index = 0u32;
        while index < pool_array.count {
            let count = (pool_array.count - index).min(PROJECT_BATCH);
            // Identity gather in row-major order with the walk's completed-
            // field precedence. The mirror always gathers both fields; the
            // replay's truncated passes request a prefix subset.
            let mut headers = [0usize; PROJECT_BATCH as usize];
            let mut copied = 0u32;
            let mut stopped = false;
            for ordinal in 0..count {
                let ptr = match ((index + ordinal) as usize)
                    .checked_mul(nproject_cproject::POOL_STRIDE)
                    .and_then(|offset| pool_array.data.checked_add(offset))
                {
                    Some(ptr) => ptr,
                    // Typed address overflow aborts the whole walk.
                    None => return,
                };
                match span(log, memory, Some(ptr), PROJECT_HEADER_LEN, M_PROJECTS) {
                    Some(start) => {
                        headers[ordinal as usize] = start;
                        copied += 1;
                    }
                    None => {
                        stopped = true;
                        break;
                    }
                }
                match span(
                    log,
                    memory,
                    ptr.checked_add(nproject_cproject::STARTED),
                    PROJECT_FLAG_LEN,
                    M_PROJECTS,
                ) {
                    Some(_) => copied += 1,
                    None => {
                        stopped = true;
                        break;
                    }
                }
                if log.overflowed {
                    return;
                }
            }
            if copied < count * 2 && copied.is_multiple_of(2) {
                return;
            }
            let header_count = copied.div_ceil(2);
            if header_count == 0 {
                return;
            }
            let _ = stopped;
            for ordinal in 0..header_count {
                let header = headers[ordinal as usize];
                if pool_u64(log, header) != expected
                    || pool_u32(log, header + nproject_cproject::TYPE)
                        != nproject_cproject::REFERENCE_TYPE
                    || pool_u32(log, header + nproject_cproject::ID) == 0
                {
                    return;
                }
                let owner = pool_u32(log, header + nproject_cproject::OWNER);
                if owner != authenticated_owner {
                    match copy_resolve(log, memory, shared, owner, M_PROJECT_TABLES) {
                        Some(pointer) if pointer == country.pointer => {
                            authenticated_owner = owner;
                        }
                        _ => return,
                    }
                }
                let machine =
                    match usize::try_from(pool_u64(log, header + nproject_cproject::STATE_MACHINE))
                        .ok()
                    {
                        Some(machine) => machine,
                        None => return,
                    };
                if span(
                    log,
                    memory,
                    Some(machine),
                    nproject_cproject_state_machine::SIZE,
                    M_PROJECTS,
                )
                .is_none()
                    && !log.overflowed
                {
                    // A failed machine gather fails projects only; history
                    // continues below exactly like the typed walk.
                }
                if log.overflowed {
                    return;
                }
                if !hist_dead {
                    copy_project_history(log, memory, base, header, owner, &mut hist_dead);
                    if log.overflowed {
                        return;
                    }
                }
            }
            index += header_count;
        }
    }
}

/// Mirror of one record's history read set: array header plus every entry's
/// row and context spans. Sets `hist_dead` on the first typed abort point;
/// later history takes are skipped exactly like the filtered walk.
fn copy_project_history(
    log: &mut RawLog<4>,
    memory: &impl Memory,
    base: usize,
    header: usize,
    owner: u32,
    hist_dead: &mut bool,
) {
    let id = pool_u32(log, header + nproject_cproject::ID);
    let data = match usize::try_from(pool_u64(log, header + nproject_cproject::HISTORY)) {
        Ok(data) => data,
        Err(_) => {
            *hist_dead = true;
            return;
        }
    };
    let capacity = pool_u32(log, header + nproject_cproject::HISTORY_CAPACITY);
    let count = pool_u32(log, header + nproject_cproject::HISTORY_COUNT);
    if capacity > i32::MAX as u32
        || count > capacity
        || (count != 0 && data == 0)
        || (count as usize)
            .checked_mul(nproject_sproject_history::STRIDE)
            .and_then(|len| data.checked_add(len))
            .is_none()
    {
        *hist_dead = true;
        return;
    }
    let expected = match base.checked_add(nproject_sproject_history::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => {
            *hist_dead = true;
            return;
        }
    };
    let context_expected = match base.checked_add(nproject_sproject_context::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => {
            *hist_dead = true;
            return;
        }
    };
    for ordinal in 0..count {
        let address = (ordinal as usize)
            .checked_mul(nproject_sproject_history::STRIDE)
            .and_then(|offset| data.checked_add(offset));
        let Some(address) = address else {
            *hist_dead = true;
            return;
        };
        let Some(vtable) = span(log, memory, Some(address), 8, M_HISTORY) else {
            *hist_dead = true;
            return;
        };
        if pool_u64(log, vtable) != expected {
            *hist_dead = true;
            return;
        }
        let handles = span(
            log,
            memory,
            address.checked_add(nproject_sproject_history::PROTOTYPE_REWARD_HANDLE),
            8,
            M_HISTORY,
        );
        let date = span(
            log,
            memory,
            address.checked_add(nproject_sproject_history::DATE),
            4,
            M_HISTORY,
        );
        if handles.is_none() || date.is_none() {
            *hist_dead = true;
            return;
        }
        let context = match address.checked_add(nproject_sproject_history::CONTEXT) {
            Some(context) => context,
            None => {
                *hist_dead = true;
                return;
            }
        };
        if !copy_project_context(log, memory, context_expected, context) {
            *hist_dead = true;
            return;
        }
        let _ = (owner, id, ordinal);
    }
}

/// Mirror of `SProjectContext::read_row`: vtable, fixed scatter and optional
/// payloads by presence byte. Presence is parsed from the recorded bytes, so
/// the optional read set matches the typed walk exactly.
fn copy_project_context(
    log: &mut RawLog<4>,
    memory: &impl Memory,
    expected: u64,
    address: usize,
) -> bool {
    let Some(vtable) = span(log, memory, Some(address), 8, M_HISTORY) else {
        return false;
    };
    if pool_u64(log, vtable) != expected {
        return false;
    }
    let fixed = span(
        log,
        memory,
        address.checked_add(nproject_sproject_context::PROVINCE),
        16,
        M_HISTORY,
    );
    let country_present = span(
        log,
        memory,
        address.checked_add(nproject_sproject_context::COUNTRY_PRESENT),
        1,
        M_HISTORY,
    );
    let scientist_present = span(
        log,
        memory,
        address.checked_add(nproject_sproject_context::SCIENTIST_PRESENT),
        1,
        M_HISTORY,
    );
    let state_present = span(
        log,
        memory,
        address.checked_add(nproject_sproject_context::STATE_PRESENT),
        1,
        M_HISTORY,
    );
    let (Some(fixed), Some(country_present), Some(scientist_present), Some(state_present)) =
        (fixed, country_present, scientist_present, state_present)
    else {
        return false;
    };
    for (present_start, offset) in [
        (
            country_present,
            nproject_sproject_context::COUNTRY_REFERENCE,
        ),
        (
            scientist_present,
            nproject_sproject_context::SCIENTIST_REFERENCE,
        ),
        (state_present, nproject_sproject_context::STATE_REFERENCE),
    ] {
        if log.bytes[present_start] != 0
            && span(log, memory, address.checked_add(offset), 4, M_HISTORY).is_none()
        {
            return false;
        }
    }
    let _ = fixed;
    true
}

/// Parsed breakthrough tree links for the traversal mirror.
#[derive(Clone, Copy)]
struct BreakthroughLinks {
    left: usize,
    parent: usize,
    right: usize,
    nil: bool,
}

/// Mirror of `Links::read`: the 24-byte link span plus the nil byte.
fn copy_breakthrough_links(
    log: &mut RawLog<4>,
    memory: &impl Memory,
    address: usize,
) -> Option<BreakthroughLinks> {
    if address == 0 {
        return None;
    }
    let pointers = span(log, memory, Some(address), 24, M_BREAKTHROUGH)?;
    let nil = span(
        log,
        memory,
        address.checked_add(nproject_breakthrough_node::IS_NIL),
        1,
        M_BREAKTHROUGH,
    )?;
    if log.bytes[nil] > 1 {
        return None;
    }
    let pointer = |offset: usize| {
        usize::try_from(u64::from_le_bytes(
            log.bytes[pointers + offset..pointers + offset + 8]
                .try_into()
                .unwrap(),
        ))
        .ok()
    };
    Some(BreakthroughLinks {
        left: pointer(nproject_breakthrough_node::LEFT)?,
        parent: pointer(nproject_breakthrough_node::PARENT)?,
        right: pointer(nproject_breakthrough_node::RIGHT)?,
        nil: log.bytes[nil] != 0,
    })
}

/// Brent-style cycle guard with no reads, mirroring the typed `Chain`.
struct BreakthroughChain {
    checkpoint: usize,
    power: u64,
    distance: u64,
}

impl BreakthroughChain {
    fn new(start: usize) -> Self {
        Self {
            checkpoint: start,
            power: 1,
            distance: 0,
        }
    }

    fn visit(&mut self, address: usize) -> bool {
        self.distance += 1;
        if address == self.checkpoint {
            return false;
        }
        if self.distance == self.power {
            self.checkpoint = address;
            self.power = self.power.saturating_mul(2);
            self.distance = 0;
        }
        true
    }
}

/// Mirror of `successor`: the identical link-following sequence with local
/// cycle detection. Any failure aborts the breakthroughs walk.
fn copy_breakthrough_successor(
    log: &mut RawLog<4>,
    memory: &impl Memory,
    header: usize,
    mut current: usize,
    links: BreakthroughLinks,
) -> Option<usize> {
    if links.right != header {
        let mut candidate = links.right;
        let mut chain = BreakthroughChain::new(current);
        loop {
            if !chain.visit(candidate) {
                return None;
            }
            let child = copy_breakthrough_links(log, memory, candidate)?;
            if child.nil || child.parent != current {
                return None;
            }
            if child.left == header {
                return Some(candidate);
            }
            current = candidate;
            candidate = child.left;
        }
    }
    let mut parent = links.parent;
    let mut chain = BreakthroughChain::new(current);
    loop {
        if parent == header {
            return Some(header);
        }
        if !chain.visit(parent) {
            return None;
        }
        let ancestor = copy_breakthrough_links(log, memory, parent)?;
        if ancestor.nil {
            return None;
        }
        if ancestor.left == current {
            return Some(parent);
        }
        if ancestor.right != current {
            return None;
        }
        current = parent;
        parent = ancestor.parent;
    }
}

/// Mirror of `walk_progress`'s read set: vtable, tree header, sentinel,
/// root and the in-order node/payload sequence. Capacity truncation is
/// ignored (a superset is recorded); order and coherence checks are local.
fn copy_breakthrough_tree(
    log: &mut RawLog<4>,
    memory: &impl Memory,
    base: usize,
    status: usize,
) -> bool {
    let address = match status.checked_add(nproject_cprogram_status::BREAKTHROUGH) {
        Some(address) => address,
        None => return false,
    };
    let expected = match base.checked_add(nproject_cbreakthrough_progress::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    let Some(vtable) = span(log, memory, Some(address), 8, M_BREAKTHROUGH) else {
        return false;
    };
    if pool_u64(log, vtable) != expected {
        return false;
    }
    let Some(header_start) = span(
        log,
        memory,
        address.checked_add(nproject_cbreakthrough_progress::TREE_HEADER),
        16,
        M_BREAKTHROUGH,
    ) else {
        return false;
    };
    let header = match usize::try_from(pool_u64(log, header_start)).ok() {
        Some(header) => header,
        None => return false,
    };
    let count = match u32::try_from(u64::from_le_bytes(
        log.bytes[header_start + 8..header_start + 16]
            .try_into()
            .unwrap(),
    )) {
        Ok(count) => count,
        Err(_) => return false,
    };
    let Some(sentinel) = copy_breakthrough_links(log, memory, header) else {
        return false;
    };
    if !sentinel.nil {
        return false;
    }
    if count == 0 {
        return sentinel.left == header && sentinel.parent == header && sentinel.right == header;
    }
    let Some(root) = copy_breakthrough_links(log, memory, sentinel.parent) else {
        return false;
    };
    if root.nil || root.parent != header {
        return false;
    }
    let mut current = sentinel.left;
    let mut previous_key = None;
    for ordinal in 0..count {
        if current == header {
            return false;
        }
        let Some(links) = copy_breakthrough_links(log, memory, current) else {
            return false;
        };
        if links.nil {
            return false;
        }
        let Some(value) = span(
            log,
            memory,
            current.checked_add(nproject_breakthrough_node::SPECIALIZATION_HANDLE),
            8,
            M_BREAKTHROUGH,
        ) else {
            return false;
        };
        let specialization = pool_u32(log, value);
        if previous_key.is_some_and(|key| key >= specialization as i32) {
            return false;
        }
        previous_key = Some(specialization as i32);
        let last = current;
        let Some(next) = copy_breakthrough_successor(log, memory, header, current, links) else {
            return false;
        };
        if ordinal + 1 == count && (next != header || sentinel.right != last) {
            return false;
        }
        current = next;
    }
    true
}

/// Mirror of `walk`'s read set for breakthroughs across the shared census.
fn copy_breakthroughs(
    log: &mut RawLog<4>,
    memory: &impl Memory,
    base: usize,
    links: &Array,
    countries: u32,
    census: &[CensusEntry],
) {
    for country in census.iter().filter(|entry| entry.canonical) {
        let shared = ProjectCensus {
            links,
            countries,
            census,
        };
        let Some(status) = copy_status(log, memory, base, shared, country, M_BREAKTHROUGH) else {
            return;
        };
        if !copy_breakthrough_tree(log, memory, base, status) {
            return;
        }
        if log.overflowed {
            return;
        }
    }
}

/// One structural pass over the scheduled read set: shared census first, then
/// the three walk mirrors in typed order. Failures stop only the affected
/// walk; the replay derives every outcome from the same recorded spans.
fn copy_pass(memory: &impl Memory, base: usize, game_state: usize, job: &mut ProjectsJob) {
    let _reads = read_accounting::table(TABLES[0]);
    job.game_state = game_state;
    job.census.clear();
    let (links, countries) =
        match copy_census(&mut job.log, memory, base, game_state, &mut job.census) {
            Some(census) => census,
            None => return,
        };
    if job.log.overflowed {
        return;
    }
    copy_programs(
        &mut job.log,
        memory,
        base,
        &links,
        countries,
        &job.census,
        &mut job.pointers,
    );
    if job.log.overflowed {
        return;
    }
    copy_project_tables(&mut job.log, memory, base, &links, countries, &job.census);
    if job.log.overflowed {
        return;
    }
    copy_breakthroughs(&mut job.log, memory, base, &links, countries, &job.census);
}

/// Shared census plus the tables it authenticates. The walks replayed at
/// decode time re-derive everything else from the recorded spans.
fn copy_census(
    log: &mut RawLog<4>,
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    census: &mut Vec<CensusEntry>,
) -> Option<(Array, u32)> {
    let countries_start = span(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRIES),
        16,
        M_ALL,
    )?;
    let links_start = span(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRY_LINKS),
        16,
        M_ALL,
    )?;
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[countries_start..countries_start + 16]);
    let countries = Array::from_header(raw, 8)?;
    raw.copy_from_slice(&log.bytes[links_start..links_start + 16]);
    let links = Array::from_header(raw, 4)?;
    if countries.count < 2 || links.count == 0 {
        return None;
    }
    let vtable = base.checked_add(ccountry::VTABLE_RVA)? as u64;
    for slot in 1..countries.count {
        // Full `read_at` authentication in walk order.
        let pointer_addr = (slot as usize)
            .checked_mul(8)
            .and_then(|offset| countries.data.checked_add(offset));
        let pointer_start = span(log, memory, pointer_addr, 8, M_ALL)?;
        let pointer = match usize::try_from(pool_u64(log, pointer_start))
            .ok()
            .filter(|&pointer| pointer != 0)
        {
            Some(pointer) => pointer,
            // Null slots are skipped without reads, like the typed walk.
            None => continue,
        };
        let vtable_start = span(log, memory, Some(pointer), 8, M_ALL)?;
        let id_start = span(log, memory, pointer.checked_add(ccountry::ID), 4, M_ALL)?;
        if pool_u64(log, vtable_start) != vtable {
            return None;
        }
        let id = pool_u32(log, id_start);
        let Some(Some(canonical)) = copy_link(log, memory, &links, countries.count, id) else {
            return None;
        };
        let Some(Some(canonical_slot)) = copy_link(log, memory, &links, countries.count, slot)
        else {
            return None;
        };
        if canonical_slot != canonical {
            return None;
        }
        let Some(Some(check)) = copy_link(log, memory, &links, countries.count, canonical) else {
            return None;
        };
        if check != canonical {
            return None;
        }
        let canonical_addr = (canonical as usize)
            .checked_mul(8)
            .and_then(|offset| countries.data.checked_add(offset));
        let canonical_pointer = span(log, memory, canonical_addr, 8, M_ALL)?;
        if pool_u64(log, canonical_pointer) != pointer as u64 {
            return None;
        }
        if census.len() == census.capacity() {
            log.overflowed = true;
            return None;
        }
        census.push(CensusEntry {
            pointer,
            id,
            slot,
            canonical: canonical == slot,
        });
    }
    Some((links, countries.count))
}

/// Acquire one refresh into an owned job.
/// The caller applies the costs after acquisition has completed.
/// The caller schedules this only for the four daily project tables.
pub(crate) fn copy_projects_refresh(
    memory: &impl Memory,
    ctx: ProjectsCopyContext,
    job: &mut ProjectsJob,
) -> ProjectsCopyReport {
    job.clear();
    job.active = ctx.active;
    if !ctx.active {
        return ProjectsCopyReport {
            raw_ok: true,
            ..Default::default()
        };
    }
    let started = qpc();
    copy_pass(memory, ctx.base, ctx.game_state, job);
    ProjectsCopyReport {
        raw_ok: !job.log.overflowed,
        copy_ticks: [elapsed(started), 0, 0, 0],
        raw_bytes: job.log.raw_bytes,
    }
}

impl Captured {
    /// Mark an accepted acquisition failure without retrying live memory.
    /// Only the four project tables are affected; independent tables retain their results.
    pub(crate) fn fail_projects_capture(&mut self, active: bool, reason: &'static str) {
        let reason = if active {
            reason
        } else {
            replay::cadence::SKIPPED_REASON
        };
        self.programs.outcome = Outcome::Unavailable { reason };
        self.projects.outcome = Outcome::Unavailable { reason };
        self.history.outcome = Outcome::Unavailable { reason };
        self.breakthroughs.outcome = Outcome::Unavailable { reason };
        self.names.reset();
        self.required_candidates = 0;
        self.raw_mode = false;
    }

    /// Decode-worker interpretation of a raw refresh payload into the pooled
    /// typed rows. Outcomes, arena bytes and candidate counts match the typed
    /// walk exactly because the identical walks run against the recorded
    /// spans. Clears
    /// `raw_mode` so the next capture starts clean.
    pub(crate) fn decode_projects_payload(
        &mut self,
        base: usize,
        job: &ProjectsJob,
        decode_ticks: &mut [u64; 4],
    ) {
        if !job.active {
            self.fail_projects_capture(false, replay::cadence::SKIPPED_REASON);
            return;
        }
        let started = qpc();
        let recorded = crate::raw::Recorded::build(&job.log);
        let first = nproject_cprogram::walk(
            &recorded,
            base,
            job.game_state,
            &mut self.candidates,
            &mut self.programs.rows,
            &mut self.names.first,
        );
        self.programs.outcome = program_outcome(first, &self.programs.rows, &self.names.first);
        self.names.len = first.bytes_len;
        self.names.required = first.required_bytes;
        self.required_candidates = first.required_candidates;
        let (projects, histories) = nproject_cproject::walk_with_history(
            &recorded,
            base,
            job.game_state,
            &mut self.projects.rows,
            &mut self.history.rows,
        );
        self.projects.outcome = bounded(projects, self.projects.rows.len());
        self.history.outcome = bounded(histories, self.history.rows.len());
        self.breakthroughs.outcome = bounded(
            nproject_cbreakthrough_progress::walk(
                &recorded,
                base,
                job.game_state,
                &mut self.breakthroughs.rows,
            ),
            self.breakthroughs.rows.len(),
        );
        decode_ticks[0] += elapsed(started);
        self.raw_mode = false;
    }
}

/// Mirror of `Reader::link`: one link-table span with local bounds checks.
/// Outer `None` aborts the walk; inner `None` is a readable absence (which
/// the typed callers also turn into a walk error). Either way the replay
/// derives the identical reason from the same recorded span.
fn copy_link(
    log: &mut RawLog<4>,
    memory: &impl Memory,
    links: &Array,
    countries: u32,
    raw: u32,
) -> Option<Option<u32>> {
    if raw == 0 || raw >= links.count {
        return None;
    }
    let offset = (raw as usize).checked_mul(4)?;
    let start = span(log, memory, links.data.checked_add(offset), 4, M_ALL)?;
    let slot = pool_u32(log, start);
    if slot == 0 {
        return Some(None);
    }
    if slot >= countries {
        return None;
    }
    Some(Some(slot))
}

/// Mirror of `Reader::resolve` without re-reading authentication: the link
/// span is recorded here and the `read_at` spans come from the shared
/// census, which covered every slot. Local checks match the walk exactly.
fn copy_resolve(
    log: &mut RawLog<4>,
    memory: &impl Memory,
    census: ProjectCensus<'_>,
    raw: u32,
    mask: u32,
) -> Option<usize> {
    if raw == 0 || raw >= census.links.count {
        return None;
    }
    let offset = (raw as usize).checked_mul(4)?;
    let start = span(log, memory, census.links.data.checked_add(offset), 4, mask)?;
    let slot = pool_u32(log, start);
    if slot == 0 || slot >= census.countries {
        return None;
    }
    let entry = census.census.iter().find(|entry| entry.slot == slot)?;
    if !entry.canonical {
        return None;
    }
    Some(entry.pointer)
}

/// Mirror of `from_country`: status pointer, 20-byte header and owner
/// resolution. `None` aborts the calling walk exactly like the typed error.
fn copy_status(
    log: &mut RawLog<4>,
    memory: &impl Memory,
    base: usize,
    census: ProjectCensus<'_>,
    country: &CensusEntry,
    mask: u32,
) -> Option<usize> {
    let pointer_start = span(
        log,
        memory,
        country.pointer.checked_add(ccountry::PROGRAM_STATUS),
        8,
        mask,
    )?;
    let status = usize::try_from(pool_u64(log, pointer_start))
        .ok()
        .filter(|&status| status != 0)?;
    let header_start = span(log, memory, Some(status), 20, mask)?;
    let mut header = [0u8; 20];
    header.copy_from_slice(&log.bytes[header_start..header_start + 20]);
    let expected = base.checked_add(nproject_cprogram_status::VTABLE_RVA)? as u64;
    let secondary = base.checked_add(nproject_cprogram_status::SECONDARY_VTABLE_RVA)? as u64;
    if u64::from_le_bytes(header[..8].try_into().unwrap()) != expected
        || u64::from_le_bytes(header[8..16].try_into().unwrap()) != secondary
    {
        return None;
    }
    let owner = u32::from_le_bytes(header[16..20].try_into().unwrap());
    if copy_resolve(log, memory, census, owner, mask)? != country.pointer {
        return None;
    }
    Some(status)
}

/// Mirror of one program row's read set: identity, field scatter, optional
/// active-project handle, building terminator and payload, owner link.
/// String parses are local; any failure aborts the programs walk.
fn copy_program_row(
    log: &mut RawLog<4>,
    memory: &impl Memory,
    base: usize,
    census: ProjectCensus<'_>,
    pointer: usize,
) -> bool {
    let expected = match base.checked_add(nproject_cprogram::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    let Some(identity) = span(log, memory, Some(pointer), 16, M_PROGRAMS) else {
        return false;
    };
    if pool_u64(log, identity) != expected
        || pool_u32(log, identity + nproject_cprogram::TYPE) != nproject_cprogram::REFERENCE_TYPE
    {
        return false;
    }
    let id = pool_u32(log, identity + nproject_cprogram::ID);
    if id == 0 {
        return false;
    }
    let at = |offset| pointer.checked_add(offset);
    let mut spans = [0usize; 5];
    // Whole-group admission is only a path selection. A tight arena or a
    // missing later address must retain the scalar prefix and its exact demand.
    if let [
        Some(owner),
        Some(owner_copy),
        Some(scientist),
        Some(active),
        Some(building),
    ] = [
        at(nproject_cprogram::OWNER_PRIMARY),
        at(nproject_cprogram::OWNER_COPY),
        at(nproject_cprogram::SCIENTIST_REFERENCE_TYPE),
        at(nproject_cprogram::ACTIVE_FLAG),
        at(nproject_cprogram::BUILDING),
    ] && log.spans.capacity() - log.spans.len() >= 5
        && log
            .bytes
            .len()
            .checked_add(61)
            .is_some_and(|end| end <= log.bytes.capacity() && end <= u32::MAX as usize)
    {
        let Some((starts, 5)) = log.take_scattered_prefetched(
            memory,
            [owner, owner_copy, scientist, active, building],
            [8, 12, 8, 1, 32],
            [M_PROGRAMS; 5],
        ) else {
            return false;
        };
        spans = starts;
    } else {
        for (slot, (offset, len)) in [
            (nproject_cprogram::OWNER_PRIMARY, 8),
            (nproject_cprogram::OWNER_COPY, 12),
            (nproject_cprogram::SCIENTIST_REFERENCE_TYPE, 8),
            (nproject_cprogram::ACTIVE_FLAG, 1),
            (nproject_cprogram::BUILDING, 32),
        ]
        .iter()
        .enumerate()
        {
            let Some(start) = span(log, memory, at(*offset), *len, M_PROGRAMS) else {
                return false;
            };
            spans[slot] = start;
        }
    }
    if log.bytes[spans[0]..spans[0] + 8] != log.bytes[spans[1]..spans[1] + 8] {
        return false;
    }
    let owner = pool_u32(log, spans[0]);
    if log.bytes[spans[3]] > 1 {
        return false;
    }
    if log.bytes[spans[3]] == 1
        && span(
            log,
            memory,
            at(nproject_cprogram::ACTIVE_PROJECT_HANDLE),
            4,
            M_PROGRAMS,
        )
        .is_none()
    {
        return false;
    }
    let mut building = [0u8; 32];
    building.copy_from_slice(&log.bytes[spans[4]..spans[4] + 32]);
    let name_len = u64::from_le_bytes(building[16..24].try_into().unwrap());
    let name_capacity = u64::from_le_bytes(building[24..32].try_into().unwrap());
    if name_capacity < msvc_string::INLINE_CAPACITY || name_len > name_capacity {
        return false;
    }
    let Ok(building_len) = usize::try_from(name_len) else {
        return false;
    };
    let source = if name_capacity == msvc_string::INLINE_CAPACITY {
        at(nproject_cprogram::BUILDING)
    } else {
        usize::try_from(u64::from_le_bytes(building[..8].try_into().unwrap()))
            .ok()
            .filter(|&source| source != 0)
    };
    let Some(source) = source else {
        return false;
    };
    if span(log, memory, source.checked_add(building_len), 1, M_PROGRAMS).is_none() {
        return false;
    }
    if building_len != 0 && span(log, memory, Some(source), building_len, M_PROGRAMS).is_none() {
        return false;
    }
    copy_resolve(log, memory, census, owner, M_PROGRAMS).is_some()
}

/// Mirror of `nproject_cprogram::walk`'s read set across the shared census.
/// The walk runs in two phases — a pointer census over every country, then
/// row reads — so a row failure must not skip later countries' census
/// spans: the replay censuses everything before decoding the first row.
fn copy_programs(
    log: &mut RawLog<4>,
    memory: &impl Memory,
    base: usize,
    links: &Array,
    countries: u32,
    census: &[CensusEntry],
    pointers: &mut Vec<usize>,
) {
    // Phase 1: pointer census. Only census-span failures abort the walk;
    // row failures below never reach back into this phase.
    pointers.clear();
    let shared = ProjectCensus {
        links,
        countries,
        census,
    };
    let mut census_ok = true;
    for country in census.iter().filter(|entry| entry.canonical) {
        let Some(status) = copy_status(log, memory, base, shared, country, M_PROGRAMS) else {
            census_ok = false;
            break;
        };
        let Some(array_start) = span(
            log,
            memory,
            status.checked_add(nproject_cprogram_status::PROGRAMS),
            16,
            M_PROGRAMS,
        ) else {
            census_ok = false;
            break;
        };
        let mut raw = [0u8; 16];
        raw.copy_from_slice(&log.bytes[array_start..array_start + 16]);
        let Some(array) = Array::from_header(raw, 8) else {
            census_ok = false;
            break;
        };
        for index in 0..array.count {
            let addr = (index as usize)
                .checked_mul(8)
                .and_then(|offset| array.data.checked_add(offset));
            let pointer = addr
                .and_then(|addr| span(log, memory, Some(addr), 8, M_PROGRAMS))
                .and_then(|start| usize::try_from(pool_u64(log, start)).ok())
                .filter(|&pointer| pointer != 0);
            let Some(pointer) = pointer else {
                census_ok = false;
                break;
            };
            if pointers.len() == pointers.capacity() {
                log.overflowed = true;
                census_ok = false;
                break;
            }
            pointers.push(pointer);
        }
        if !census_ok || log.overflowed {
            break;
        }
    }
    if !census_ok {
        return;
    }
    // Phase 2: row reads in census order (a superset of the walk's sorted
    // deduplicated order; duplicates record identical spans harmlessly).
    for &pointer in pointers.iter() {
        if !copy_program_row(log, memory, base, shared, pointer) {
            return;
        }
        if log.overflowed {
            return;
        }
    }
}

fn bounded(outcome: Outcome, capacity: usize) -> Outcome {
    if outcome.len() > capacity {
        Outcome::Unavailable {
            reason: "walker row count exceeds allocated buffer",
        }
    } else if matches!(outcome, Outcome::Truncated { unread_rows: 0, .. }) {
        Outcome::Unavailable {
            reason: "walker marked truncation without missing rows",
        }
    } else {
        outcome
    }
}

fn capture_rows<R>(captured: &mut CaptureRows<R>, walk: impl FnOnce(&mut [R]) -> Outcome) {
    captured.outcome = bounded(walk(&mut captured.rows), captured.rows.len());
}

fn capture_pair<P, C>(
    parents: &mut CaptureRows<P>,
    instances: &mut CaptureRows<C>,
    walk: impl FnOnce(&mut [P], &mut [C]) -> (Outcome, Outcome),
) {
    let (parent, child) = walk(&mut parents.rows, &mut instances.rows);
    parents.outcome = bounded(parent, parents.rows.len());
    instances.outcome = bounded(child, instances.rows.len());
}

fn program_outcome(
    result: nproject_cprogram::NameWalk,
    rows: &[nproject_cprogram::Row],
    bytes: &[u8],
) -> Outcome {
    let outcome = bounded(result.outcome, rows.len());
    if result.required_bytes < result.bytes_len
        || (matches!(outcome, Outcome::Rows { .. }) && result.required_bytes != result.bytes_len)
        || !valid_names(outcome, rows, bytes, result.bytes_len)
    {
        Outcome::Unavailable {
            reason: "program building range exceeds captured arena",
        }
    } else {
        outcome
    }
}

fn valid_names(
    outcome: Outcome,
    rows: &[nproject_cprogram::Row],
    bytes: &[u8],
    len: usize,
) -> bool {
    len <= bytes.len()
        && rows.get(..outcome.len()).is_some_and(|rows| {
            rows.iter().all(|r| {
                r.building_start
                    .checked_add(r.building_len)
                    .is_some_and(|end| end <= len)
            })
        })
}

/// Shared synthetic address space for the projects copy/decode split
/// harness: two countries, one program with a heap building string, one
/// project with a valid state machine and history context, and a single
/// breakthrough node.
#[cfg(test)]
pub(crate) mod projects_test_kit {
    use super::copy_projects_refresh;
    use super::{
        Capacities, Captured, ProjectsCopyContext, ProjectsCopyReport, ProjectsJob, bounded,
        program_outcome,
    };
    use crate::table_capture::CaptureRows;
    use engine::{
        ccountry, cgame_state, msvc_string, nproject_breakthrough_node,
        nproject_cbreakthrough_progress, nproject_cprogram, nproject_cprogram_status,
        nproject_cproject, nproject_cproject_pool, nproject_cproject_state_machine,
        nproject_cprototype_project_state as prototype, nproject_csimple_project_state as simple,
        nproject_sproject_context, nproject_sproject_history,
    };

    pub(crate) const BASE: usize = 0x140000000;
    pub(crate) const GAME_STATE: usize = 0x10000;
    const ADDRESS: usize = 0x10000;
    const CTAB: usize = 0x2000;
    const LTAB: usize = 0x2100;
    pub(crate) const C1: usize = 0x3000;
    const C2: usize = 0x3100;
    pub(crate) const S1: usize = 0x4000;
    const PARR: usize = 0x5000;
    pub(crate) const PROG1: usize = 0x5100;
    const POOL1: usize = 0x6000;
    pub(crate) const PROJ1: usize = 0x6200;
    pub(crate) const MACH1: usize = 0x7000;
    pub(crate) const HIST1: usize = 0x8000;
    const BT_HEAD: usize = 0x9100;
    pub(crate) const BT_NODE: usize = 0x9200;
    pub(crate) const HEAP: usize = 0xA000;

    pub(crate) fn memory(bytes: &[u8]) -> engine::memory::SliceMemory<'_> {
        engine::memory::SliceMemory {
            base: ADDRESS,
            bytes,
        }
    }

    fn put<const COUNT: usize>(bytes: &mut [u8], offset: usize, value: [u8; COUNT]) {
        bytes[offset..offset + COUNT].copy_from_slice(&value);
    }

    fn pointer(bytes: &mut [u8], offset: usize, target: usize) {
        put(bytes, offset, ((ADDRESS + target) as u64).to_le_bytes());
    }

    fn array(bytes: &mut [u8], offset: usize, data: usize, count: u32) {
        pointer(bytes, offset, data);
        put(bytes, offset + 8, count.to_le_bytes());
        put(bytes, offset + 12, count.to_le_bytes());
    }

    fn heap_string(bytes: &mut [u8], header: usize, payload: usize, value: &[u8]) {
        assert!(value.len() > msvc_string::INLINE_CAPACITY as usize);
        put(bytes, header, ((ADDRESS + payload) as u64).to_le_bytes());
        put(bytes, header + 16, (value.len() as u64).to_le_bytes());
        put(bytes, header + 24, (value.len() as u64).to_le_bytes());
        bytes[payload..payload + value.len()].copy_from_slice(value);
        bytes[payload + value.len()] = 0;
    }

    fn country(bytes: &mut [u8], slot: u32, id: u32, object: usize, status: usize) {
        pointer(bytes, CTAB + slot as usize * 8, object);
        put(bytes, LTAB + id as usize * 4, id.to_le_bytes());
        put(
            bytes,
            object,
            ((BASE + ccountry::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(bytes, object + ccountry::ID, id.to_le_bytes());
        pointer(bytes, object + ccountry::PROGRAM_STATUS, status);
        put(
            bytes,
            status,
            ((BASE + nproject_cprogram_status::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(
            bytes,
            status + nproject_cprogram_status::SECONDARY_VTABLE,
            ((BASE + nproject_cprogram_status::SECONDARY_VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(
            bytes,
            status + nproject_cprogram_status::OWNER,
            id.to_le_bytes(),
        );
    }

    fn empty_status(bytes: &mut [u8], status: usize, id: u32, pool: usize) {
        put(
            bytes,
            status,
            ((BASE + nproject_cprogram_status::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(
            bytes,
            status + nproject_cprogram_status::SECONDARY_VTABLE,
            ((BASE + nproject_cprogram_status::SECONDARY_VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(
            bytes,
            status + nproject_cprogram_status::OWNER,
            id.to_le_bytes(),
        );
        pointer(bytes, status + nproject_cprogram_status::PROJECT_POOL, pool);
        put(
            bytes,
            pool,
            ((BASE + nproject_cproject_pool::VTABLE_RVA) as u64).to_le_bytes(),
        );
        array(bytes, pool + nproject_cproject_pool::PROJECTS, 0, 0);
        array(bytes, status + nproject_cprogram_status::PROGRAMS, 0, 0);
        // Empty breakthrough tree, inline in the status object.
        let tree = status + nproject_cprogram_status::BREAKTHROUGH;
        put(
            bytes,
            tree,
            ((BASE + nproject_cbreakthrough_progress::VTABLE_RVA) as u64).to_le_bytes(),
        );
        let head = tree + 0x40;
        pointer(
            bytes,
            tree + nproject_cbreakthrough_progress::TREE_HEADER,
            head,
        );
        put(
            bytes,
            tree + nproject_cbreakthrough_progress::TREE_SIZE,
            0_u64.to_le_bytes(),
        );
        for link in [0, 8, 16] {
            pointer(bytes, head + link, head);
        }
        bytes[head + nproject_breakthrough_node::IS_NIL] = 1;
    }

    pub(crate) fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0xB000];
        // The game-state object lives at the image base here, so census
        // headers sit at the raw engine offsets.
        array(&mut bytes, cgame_state::COUNTRIES, CTAB, 3);
        array(&mut bytes, cgame_state::COUNTRY_LINKS, LTAB, 3);
        country(&mut bytes, 1, 1, C1, S1);
        country(&mut bytes, 2, 2, C2, S1 + 0x200);
        pointer(
            &mut bytes,
            S1 + nproject_cprogram_status::PROJECT_POOL,
            POOL1,
        );
        put(
            &mut bytes,
            POOL1,
            ((BASE + nproject_cproject_pool::VTABLE_RVA) as u64).to_le_bytes(),
        );
        array(
            &mut bytes,
            POOL1 + nproject_cproject_pool::PROJECTS,
            PROJ1,
            1,
        );
        array(&mut bytes, S1 + nproject_cprogram_status::PROGRAMS, PARR, 1);
        pointer(&mut bytes, PARR, PROG1);
        // Program with a heap building string and an active project.
        put(
            &mut bytes,
            PROG1,
            ((BASE + nproject_cprogram::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            PROG1 + nproject_cprogram::TYPE,
            nproject_cprogram::REFERENCE_TYPE.to_le_bytes(),
        );
        put(
            &mut bytes,
            PROG1 + nproject_cprogram::ID,
            5_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            PROG1 + nproject_cprogram::OWNER_PRIMARY,
            1_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            PROG1 + nproject_cprogram::PROVINCE_PRIMARY,
            13413_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            PROG1 + nproject_cprogram::OWNER_COPY,
            1_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            PROG1 + nproject_cprogram::PROVINCE_COPY,
            13413_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            PROG1 + nproject_cprogram::SCIENTIST_REFERENCE_TYPE,
            0x1122334455667788_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            PROG1 + nproject_cprogram::UNREAD_PROTOTYPE_REWARDS,
            7_i32.to_le_bytes(),
        );
        bytes[PROG1 + nproject_cprogram::ACTIVE_FLAG] = 1;
        put(
            &mut bytes,
            PROG1 + nproject_cprogram::ACTIVE_PROJECT_HANDLE,
            9_u32.to_le_bytes(),
        );
        heap_string(
            &mut bytes,
            PROG1 + nproject_cprogram::BUILDING,
            HEAP,
            b"program-building-01",
        );
        // Project with a valid state machine and one history entry.
        put(
            &mut bytes,
            PROJ1,
            ((BASE + nproject_cproject::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            PROJ1 + nproject_cproject::TYPE,
            nproject_cproject::REFERENCE_TYPE.to_le_bytes(),
        );
        put(
            &mut bytes,
            PROJ1 + nproject_cproject::ID,
            3_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            PROJ1 + nproject_cproject::DEFINITION_HANDLE,
            45_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            PROJ1 + nproject_cproject::OWNER,
            1_u32.to_le_bytes(),
        );
        pointer(&mut bytes, PROJ1 + nproject_cproject::STATE_MACHINE, MACH1);
        put(
            &mut bytes,
            PROJ1 + nproject_cproject::PROGRAM_REFERENCE_TYPE,
            0x99_u64.to_le_bytes(),
        );
        array(&mut bytes, PROJ1 + nproject_cproject::HISTORY, HIST1, 1);
        bytes[PROJ1 + nproject_cproject::STARTED] = 1;
        // State machine with the current simple state first.
        put(
            &mut bytes,
            MACH1,
            ((BASE + nproject_cproject_state_machine::VTABLE_RVA) as u64).to_le_bytes(),
        );
        pointer(
            &mut bytes,
            MACH1 + nproject_cproject_state_machine::CURRENT_STATE,
            MACH1 + nproject_cproject_state_machine::SIMPLE_STATE_1,
        );
        bytes[MACH1 + nproject_cproject_state_machine::NOT_COMPLETED] = 1;
        for (index, offset) in [
            nproject_cproject_state_machine::PROTOTYPE_STATE,
            nproject_cproject_state_machine::SIMPLE_STATE_1,
            nproject_cproject_state_machine::SIMPLE_STATE_2,
            nproject_cproject_state_machine::SIMPLE_STATE_3,
        ]
        .into_iter()
        .enumerate()
        {
            let vtable = if index == 0 {
                prototype::VTABLE_RVA
            } else {
                simple::VTABLE_RVA
            };
            put(
                &mut bytes,
                MACH1 + offset,
                ((BASE + vtable) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                MACH1 + offset + simple::STATE_ENUM,
                (index as u32 + 10).to_le_bytes(),
            );
        }
        // History entry with country and state references present.
        put(
            &mut bytes,
            HIST1,
            ((BASE + nproject_sproject_history::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            HIST1 + nproject_sproject_history::PROTOTYPE_REWARD_HANDLE,
            11_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            HIST1 + nproject_sproject_history::OPTION_HANDLE,
            22_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            HIST1 + nproject_sproject_history::DATE,
            333_i32.to_le_bytes(),
        );
        let context = HIST1 + nproject_sproject_history::CONTEXT;
        put(
            &mut bytes,
            context,
            ((BASE + nproject_sproject_context::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            context + nproject_sproject_context::PROVINCE,
            (-5_i32).to_le_bytes(),
        );
        put(
            &mut bytes,
            context + nproject_sproject_context::PROGRESS,
            6_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            context + nproject_sproject_context::CHARACTER_REFERENCE,
            0x77_u64.to_le_bytes(),
        );
        bytes[context + nproject_sproject_context::COUNTRY_PRESENT] = 1;
        put(
            &mut bytes,
            context + nproject_sproject_context::COUNTRY_REFERENCE,
            1_u32.to_le_bytes(),
        );
        bytes[context + nproject_sproject_context::STATE_PRESENT] = 1;
        put(
            &mut bytes,
            context + nproject_sproject_context::STATE_REFERENCE,
            9_u32.to_le_bytes(),
        );
        // Single-node breakthrough tree, inline in the status object.
        put(
            &mut bytes,
            S1 + nproject_cprogram_status::BREAKTHROUGH,
            ((BASE + nproject_cbreakthrough_progress::VTABLE_RVA) as u64).to_le_bytes(),
        );
        pointer(
            &mut bytes,
            S1 + nproject_cprogram_status::BREAKTHROUGH
                + nproject_cbreakthrough_progress::TREE_HEADER,
            BT_HEAD,
        );
        put(
            &mut bytes,
            S1 + nproject_cprogram_status::BREAKTHROUGH
                + nproject_cbreakthrough_progress::TREE_SIZE,
            1_u64.to_le_bytes(),
        );
        pointer(&mut bytes, BT_HEAD, BT_NODE);
        pointer(&mut bytes, BT_HEAD + 8, BT_NODE);
        pointer(&mut bytes, BT_HEAD + 16, BT_NODE);
        bytes[BT_HEAD + nproject_breakthrough_node::IS_NIL] = 1;
        pointer(&mut bytes, BT_NODE, BT_HEAD);
        pointer(&mut bytes, BT_NODE + 8, BT_HEAD);
        pointer(&mut bytes, BT_NODE + 16, BT_HEAD);
        bytes[BT_NODE + nproject_breakthrough_node::IS_NIL] = 0;
        put(
            &mut bytes,
            BT_NODE + nproject_breakthrough_node::SPECIALIZATION_HANDLE,
            77_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            BT_NODE + nproject_breakthrough_node::POINTS,
            (-8_i32).to_le_bytes(),
        );
        empty_status(&mut bytes, S1 + 0x200, 2, 0x6100);
        bytes
    }

    /// Reference typed walks with identical pooled buffers, outcomes and
    /// arenas installed exactly like `decode_projects_payload` installs them.
    pub(crate) fn direct(memory: &impl engine::Memory, capacities: &Capacities) -> Captured {
        let mut captured = Captured::with_capacities(capacities);
        let first = nproject_cprogram::walk(
            memory,
            BASE,
            GAME_STATE,
            &mut captured.candidates,
            &mut captured.programs.rows,
            &mut captured.names.first,
        );
        captured.programs.outcome =
            program_outcome(first, &captured.programs.rows, &captured.names.first);
        captured.names.len = first.bytes_len;
        captured.names.required = first.required_bytes;
        captured.required_candidates = first.required_candidates;
        let (projects, histories) = nproject_cproject::walk_with_history(
            memory,
            BASE,
            GAME_STATE,
            &mut captured.projects.rows,
            &mut captured.history.rows,
        );
        captured.projects.outcome = bounded(projects, captured.projects.rows.len());
        captured.history.outcome = bounded(histories, captured.history.rows.len());
        captured.breakthroughs.outcome = bounded(
            nproject_cbreakthrough_progress::walk(
                memory,
                BASE,
                GAME_STATE,
                &mut captured.breakthroughs.rows,
            ),
            captured.breakthroughs.rows.len(),
        );
        captured
    }

    /// Capture one current payload, then decode into fresh pooled buffers.
    pub(crate) fn split(
        memory: &impl engine::Memory,
        capacities: &Capacities,
    ) -> (Captured, ProjectsCopyReport, ProjectsJob) {
        let mut captured = Captured::with_capacities(capacities);
        let ctx = ProjectsCopyContext {
            base: BASE,
            game_state: GAME_STATE,
            active: true,
        };
        let mut first = ProjectsJob::new();
        let report = copy_projects_refresh(memory, ctx, &mut first);
        assert!(report.raw_ok);
        let mut ticks = [0u64; 4];
        captured.decode_projects_payload(BASE, &first, &mut ticks);
        (captured, report, first)
    }

    fn assert_rows<R: PartialEq + std::fmt::Debug>(
        name: &str,
        split: &CaptureRows<R>,
        direct: &CaptureRows<R>,
    ) {
        assert_eq!(split.outcome, direct.outcome, "{name} outcome");
        let len = split.outcome.len();
        assert_eq!(direct.outcome.len(), len, "{name} length");
        assert_eq!(&split.rows[..len], &direct.rows[..len], "{name} rows");
    }

    pub(crate) fn assert_matches_direct(split: &Captured, direct: &Captured) {
        assert_rows("programs", &split.programs, &direct.programs);
        assert_rows("projects", &split.projects, &direct.projects);
        assert_rows("history", &split.history, &direct.history);
        assert_rows("breakthroughs", &split.breakthroughs, &direct.breakthroughs);
        assert_eq!(
            &split.names.first[..split.names.len],
            &direct.names.first[..direct.names.len],
            "program names arena"
        );
        assert_eq!(
            split.names.required, direct.names.required,
            "names required"
        );
        assert_eq!(
            split.required_candidates, direct.required_candidates,
            "required candidates"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_job_capacity_accounting_tracks_all_vectors_through_growth_clear_and_drop() {
        use crate::raw_memory::{Capacity, Charge, Tracker, vector_bytes};
        static LOCAL: Tracker = Tracker::new();
        let mut job = ProjectsJob::default();
        job.census.reserve_exact(3);
        job.pointers.reserve_exact(3);
        job.capacity_charge = Charge::empty_on(&LOCAL);
        job.update_capacity_charge();
        let expected = |job: &ProjectsJob| vector_bytes(&job.census) + vector_bytes(&job.pointers);
        let initial = LOCAL.snapshot().current.auxiliary_vectors;
        assert_eq!(initial, expected(&job));
        job.grow();
        let grown = LOCAL.snapshot().current.auxiliary_vectors;
        assert!(grown > initial);
        assert_eq!(grown, expected(&job));
        job.clear();
        assert_eq!(LOCAL.snapshot().current.auxiliary_vectors, grown);
        drop(job);
        assert_eq!(LOCAL.snapshot().current, Capacity::default());
    }

    use replay::rows::TableSchema;

    fn small() -> (Capacities, Captured) {
        let capacities = Capacities {
            rows: [2; N],
            program_names: 4,
            program_candidates: 2,
        };
        let captured = Captured::with_capacities(&capacities);
        (capacities, captured)
    }

    #[test]
    fn daily_project_siblings_do_no_inactive_native_work_after_shared_faults() {
        struct Unreadable;
        impl Memory for Unreadable {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                false
            }
        }
        let memory = engine::memory::CountedMemory::new(Unreadable);
        let (_, mut captured) = small();
        for hour in 0..24 {
            let accounting = read_accounting::Capture::new();
            captured.capture(&memory, 0, 0x1000, hour, false);
            let counts = accounting.finish();
            for (index, outcome) in captured.outcomes().into_iter().enumerate() {
                if !replay::cadence::should_walk(TABLES[index], hour, false) {
                    assert_eq!(
                        outcome,
                        Outcome::Unavailable {
                            reason: replay::cadence::SKIPPED_REASON
                        }
                    );
                    assert_eq!(counts[TABLES[index].index()].spans, 0);
                }
            }
        }
    }

    #[test]
    fn daily_project_job_skips_empty_storage_and_preserves_raw_typed_identity() {
        use super::projects_test_kit as kit;
        let bytes = kit::fixture();
        let (capacities, _) = small();
        for hour in 0..24 {
            let active = replay::cadence::should_walk(TableId::Projects, hour, false);
            assert_eq!(
                active,
                Some(hour as u32) == replay::cadence::refresh_slot(TableId::Projects)
            );
            let memory = ProgramSource::new(&bytes);
            let mut job = if active {
                ProjectsJob::new()
            } else {
                ProjectsJob::default()
            };
            let report = copy_projects_refresh(
                &memory,
                ProjectsCopyContext {
                    base: kit::BASE,
                    game_state: kit::GAME_STATE,
                    active,
                },
                &mut job,
            );
            assert!(report.raw_ok);
            assert_eq!(job.active, active);
            let mut captured = Captured::with_capacities(&capacities);
            captured.nukes.outcome = Outcome::Rows { len: 1 };
            let mut ticks = [0; 4];
            captured.decode_projects_payload(kit::BASE, &job, &mut ticks);
            if active {
                let direct = kit::direct(&kit::memory(&bytes), &capacities);
                kit::assert_matches_direct(&captured, &direct);
                assert!(!memory.calls.borrow().is_empty());
            } else {
                assert!(memory.calls.borrow().is_empty());
                assert_eq!(report.copy_ticks, [0; 4]);
                assert_eq!(report.raw_bytes, [0; 4]);
                assert_eq!(ticks, [0; 4]);
                assert!(captured.outcomes()[..4].iter().all(|outcome| *outcome
                    == Outcome::Unavailable {
                        reason: replay::cadence::SKIPPED_REASON,
                    }));
                captured.fail_projects_capture(false, "accepted worker stopped");
                assert_eq!(
                    captured.programs.outcome,
                    Outcome::Unavailable {
                        reason: replay::cadence::SKIPPED_REASON
                    }
                );
            }
            assert_eq!(captured.nukes.outcome, Outcome::Rows { len: 1 });
        }
    }

    #[test]
    fn failed_project_job_hides_prior_rows_and_preserves_independent_results() {
        let (_, mut captured) = small();
        captured.programs.outcome = Outcome::Rows { len: 1 };
        captured.projects.outcome = Outcome::Rows { len: 1 };
        captured.history.outcome = Outcome::Rows { len: 1 };
        captured.breakthroughs.outcome = Outcome::Rows { len: 1 };
        captured.nukes.outcome = Outcome::Rows { len: 1 };
        captured.raids.outcome = Outcome::Rows { len: 1 };
        let independent = captured.outcomes()[4..].to_vec();
        captured.raw_mode = true;
        captured.fail_projects_capture(true, "project acquisition worker failed");
        assert!(!captured.raw_mode);
        assert!(
            captured.outcomes()[..4]
                .iter()
                .all(|outcome| matches!(outcome, Outcome::Unavailable { .. }))
        );
        assert_eq!(&captured.outcomes()[4..], independent);
        assert!(
            captured.tables()[..4]
                .iter()
                .all(|table| matches!(table, TableView::Unavailable { .. }))
        );
    }

    fn complete<S: TableSchema>(view: &TableView) -> &[S::Row] {
        let TableView::Complete(rows) = view else {
            panic!("expected complete table")
        };
        rows.rows::<S>().unwrap()
    }

    #[test]
    fn conversion_preserves_native_keys_fixed_values_presence_and_literal_building_bytes() {
        let (_, mut captured) = small();
        captured.programs.outcome = Outcome::Rows { len: 1 };
        captured.programs.rows[0] = nproject_cprogram::Row {
            id: u32::MAX,
            active_project: Some(0),
            scientist: u64::MAX,
            unread_rewards: i32::MIN,
            building_start: 1,
            building_len: 3,
            ..Default::default()
        };
        captured.names.first.copy_from_slice(&[99, 0xff, 0, b'a']);
        captured.names.len = 4;
        captured.names.required = 4;
        captured.projects.outcome = Outcome::Rows { len: 1 };
        captured.projects.rows[0] = nproject_cproject::Row {
            owner: u32::MAX,
            local_id: u32::MAX,
            definition_id: u32::MAX,
            program_reference: u64::MAX,
            started: 1,
            completed: 0,
            state: engine::nproject_cproject_state_machine::State {
                current_offset: u32::MAX,
                state_enums: [1, 2, 3, u32::MAX],
                not_completed: 1,
                phase_progress: i64::MIN,
                phase_target: i32::MAX,
                prototype_progress: i32::MIN,
                iterations: -17,
            },
        };
        captured.history.outcome = Outcome::Rows { len: 1 };
        captured.history.rows[0] = nproject_sproject_history::Row {
            owner: u32::MAX,
            project: u32::MAX,
            ordinal: u32::MAX,
            prototype_reward_handle: u32::MAX,
            option_handle: u32::MAX - 1,
            date: i32::MIN,
            context: engine::nproject_sproject_context::Row {
                province: i32::MIN,
                progress: i32::MAX,
                character_reference: u64::MAX,
                country_present: 1,
                country_reference: Some(u32::MAX),
                scientist_present: 0,
                scientist_reference: None,
                state_present: 1,
                state_reference: Some(0),
            },
        };
        captured.breakthroughs.outcome = Outcome::Rows { len: 1 };
        captured.breakthroughs.rows[0] = nproject_cbreakthrough_progress::Row {
            owner: u32::MAX,
            specialization: u32::MAX,
            points: i32::MIN,
        };
        captured.nukes.outcome = Outcome::Rows { len: 1 };
        captured.nukes.rows[0] = cnuke::Row {
            country: u32::MAX,
            nuke_type: 1,
            amount_raw: i64::MAX,
            ready: i32::MIN,
        };
        captured.raid_countries.outcome = Outcome::Rows { len: 1 };
        captured.raid_countries.rows[0] = ccountry_raid_status::Row {
            canonical_slot: 0,
            country: -1,
            priority: i32::MIN,
            child_count: 1,
        };
        captured.raids.outcome = Outcome::Rows { len: 1 };
        captured.raids.rows[0] = craid_instance::Row {
            reference_type: u32::MAX - 1,
            id: u32::MAX,
            parent_slot: 0,
            ordinal: 7,
            preparation_hours: i32::MIN,
            phase: 3,
            outcome: -1,
            end_date_present: 1,
            victim_country: -1,
            source: engine::craid_source::Row {
                kind: None,
                kind_present: 0,
                province: u32::MAX,
                ship_reference_type: u32::MAX,
                ship_reference_id: u32::MAX - 1,
                country: -1,
            },
            ..Default::default()
        };
        captured.peace.outcome = Outcome::Rows { len: 1 };
        captured.peace.rows[0] = peace_conference::Row {
            id: u64::MAX,
            ordinal: u32::MAX,
            current_turn: i32::MIN,
            finalized: 1,
        };
        let views = captured.tables();
        let program = &complete::<tables::ProjectPrograms>(&views[0])[0];
        assert_eq!(
            (
                program.active_project,
                program.scientist,
                program.unread_rewards
            ),
            (Some(0), u64::MAX, i32::MIN)
        );
        assert_eq!(program.building, [0xff, 0, b'a']);
        let project = &complete::<tables::Projects>(&views[1])[0];
        assert_eq!(project.key(), u128::from(u64::MAX));
        assert_eq!(
            (project.definition_id, project.program_reference),
            (u32::MAX, u64::MAX)
        );
        assert_eq!(
            (
                project.state.phase_progress,
                project.state.phase_target,
                project.state.prototype_progress,
                project.state.iterations
            ),
            (i64::MIN, i32::MAX, i32::MIN, -17)
        );
        let history = &complete::<tables::ProjectHistory>(&views[2])[0];
        assert_eq!(history.key(), (1_u128 << 96) - 1);
        assert_eq!(
            (history.prototype_reward_handle, history.option_handle),
            (u32::MAX, u32::MAX - 1)
        );
        assert_eq!(
            (
                history.context.country_reference,
                history.context.scientist_reference,
                history.context.state_reference
            ),
            (Some(u32::MAX), None, Some(0))
        );
        let breakthrough = &complete::<tables::ProjectBreakthroughs>(&views[3])[0];
        assert_eq!(
            (breakthrough.key(), breakthrough.points),
            (u128::from(u64::MAX), i32::MIN)
        );
        let nuke = &complete::<tables::Nukes>(&views[4])[0];
        assert_eq!((nuke.amount_raw, nuke.ready), (i64::MAX, i32::MIN));
        let parent = &complete::<tables::RaidCountries>(&views[5])[0];
        assert_eq!(
            (parent.canonical_slot, parent.country, parent.priority),
            (0, -1, i32::MIN)
        );
        let raid = &complete::<tables::Raids>(&views[6])[0];
        assert_eq!(
            raid.key(),
            (u128::from(u32::MAX) << 32) | u128::from(u32::MAX - 1)
        );
        assert_eq!(
            (
                raid.source.kind,
                raid.source.province,
                raid.source.ship_reference_id
            ),
            (None, u32::MAX, u32::MAX - 1)
        );
        let peace = &complete::<tables::PeaceConferences>(&views[7])[0];
        assert_eq!(
            (peace.id, peace.ordinal, peace.current_turn, peace.finalized),
            (u64::MAX, u32::MAX, i32::MIN, 1)
        );
    }

    #[test]
    fn invalid_building_ranges_and_impossible_row_counts_refuse_without_panicking() {
        let (_, mut captured) = small();
        captured.programs.outcome = Outcome::Rows { len: 1 };
        captured.names.len = 2;
        captured.names.required = 2;
        for (start, len) in [(1, 2), (usize::MAX, 1), (4, 0)] {
            captured.programs.rows[0].building_start = start;
            captured.programs.rows[0].building_len = len;
            assert!(matches!(
                captured.program_table(captured.programs.outcome),
                TableView::Unavailable { .. }
            ));
        }
        for outcome in [
            Outcome::Rows { len: 3 },
            Outcome::Truncated {
                len: 1,
                unread_rows: 0,
            },
        ] {
            assert!(matches!(bounded(outcome, 2), Outcome::Unavailable { .. }));
        }
        assert!(matches!(
            program_outcome(
                nproject_cprogram::NameWalk {
                    outcome: Outcome::Rows { len: 0 },
                    bytes_len: 0,
                    required_bytes: 1,
                    required_candidates: 1,
                },
                &[],
                &[],
            ),
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn program_capture_reads_once_and_preserves_primary_literal_and_truncation() {
        let (_, mut captured) = small();
        let mut calls = 0;
        captured.capture_programs(|_, rows, bytes| {
            calls += 1;
            rows[0] = nproject_cprogram::Row {
                id: 1,
                building_len: 1,
                ..Default::default()
            };
            bytes[0] = 0xff;
            nproject_cprogram::NameWalk {
                outcome: Outcome::Rows { len: 1 },
                bytes_len: 1,
                required_bytes: 1,
                required_candidates: 1,
            }
        });
        assert_eq!(calls, 1);
        assert_eq!(captured.programs.outcome, Outcome::Rows { len: 1 });
        assert_eq!(captured.names.first[0], 0xff);
        captured.reset();
        captured.capture_programs(|_, _, _| nproject_cprogram::NameWalk {
            outcome: Outcome::Truncated {
                len: 0,
                unread_rows: 1,
            },
            bytes_len: 0,
            required_bytes: 9,
            required_candidates: 1,
        });
        assert_eq!(
            captured.programs.outcome,
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
        assert_eq!(captured.names.required, 9);
    }

    #[test]
    fn shared_raid_capture_reads_once_and_retains_independent_capacity_outcomes() {
        let (_, mut captured) = small();
        let mut calls = 0;
        capture_pair(
            &mut captured.raid_countries,
            &mut captured.raids,
            |parents, instances| {
                calls += 1;
                parents[0] = ccountry_raid_status::Row {
                    child_count: 1,
                    ..Default::default()
                };
                instances[0] = craid_instance::Row {
                    id: 7,
                    preparation_hours: 13,
                    ..Default::default()
                };
                (Outcome::Rows { len: 1 }, Outcome::Rows { len: 1 })
            },
        );
        assert_eq!(calls, 1);
        assert_eq!(&captured.outcomes()[5..7], &[Outcome::Rows { len: 1 }; 2]);
        assert_eq!(captured.raids.rows[0].preparation_hours, 13);
        captured.raids.rows.clear();
        capture_pair(
            &mut captured.raid_countries,
            &mut captured.raids,
            |parents, instances| {
                calls += 1;
                assert_eq!(parents.len(), 2);
                assert!(instances.is_empty());
                (
                    Outcome::Rows { len: 1 },
                    Outcome::Truncated {
                        len: 0,
                        unread_rows: 1,
                    },
                )
            },
        );
        assert_eq!(calls, 2);
        assert_eq!(captured.raid_countries.outcome, Outcome::Rows { len: 1 });
        assert_eq!(
            captured.raids.outcome,
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
    }

    #[test]
    fn scratch_and_arena_growth_preserve_primary_requirements_after_failure() {
        let (mut capacities, mut captured) = small();
        captured.capture_programs(|_, _, _| nproject_cprogram::NameWalk {
            outcome: Outcome::Unavailable {
                reason: "program pointer scratch capacity insufficient",
            },
            bytes_len: 0,
            required_bytes: 17,
            required_candidates: 9,
        });
        captured.projects.outcome = Outcome::Truncated {
            len: 2,
            unread_rows: 63,
        };
        captured.grow(&mut capacities);
        assert!(captured.candidates.len() >= 9 && captured.names.first.len() >= 17);
        assert!(captured.projects.rows.len() >= 65);
        let before = captured.allocated_bytes();
        captured.reset();
        captured.grow(&mut capacities);
        assert_eq!(captured.allocated_bytes(), before);
    }

    fn split_capacities() -> Capacities {
        Capacities {
            rows: [256; N],
            program_names: 8192,
            program_candidates: 1024,
        }
    }

    struct ProgramSource<'a> {
        memory: engine::memory::SliceMemory<'a>,
        pointer: usize,
        fail: Option<usize>,
        partial: usize,
        calls: std::cell::RefCell<Vec<(usize, usize)>>,
        scatters: std::cell::RefCell<Vec<Vec<(usize, usize)>>>,
    }

    impl<'a> ProgramSource<'a> {
        fn new(bytes: &'a [u8]) -> Self {
            use super::projects_test_kit as kit;
            Self {
                memory: kit::memory(bytes),
                pointer: kit::GAME_STATE + kit::PROG1,
                fail: None,
                partial: 0,
                calls: Default::default(),
                scatters: Default::default(),
            }
        }
    }

    impl Memory for ProgramSource<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            use super::projects_test_kit as kit;
            let ordinal = self.calls.borrow().len();
            self.calls.borrow_mut().push((address, destination.len()));
            if self.fail == Some(ordinal) {
                let partial = self.partial.min(destination.len());
                destination[..partial].fill(0xe3);
                return false;
            }
            // Relocate only the program object for checked-address tests;
            // heap text and country links retain their fixture addresses.
            let source = address
                .checked_sub(self.pointer)
                .filter(|&offset| offset < nproject_cprogram::OWNER_COPY + 12)
                .map_or(address, |offset| kit::GAME_STATE + kit::PROG1 + offset);
            self.memory.copy(source, destination)
        }

        fn copy_scattered_prefetched<const M: usize>(
            &self,
            addresses: [usize; M],
            destinations: &mut [&mut [u8]; M],
        ) -> usize {
            self.scatters.borrow_mut().push(
                addresses
                    .iter()
                    .copied()
                    .zip(destinations.iter().map(|field| field.len()))
                    .collect(),
            );
            self.copy_scattered(addresses, destinations)
        }
    }

    fn program_links(bytes: &[u8]) -> Array {
        let start = cgame_state::COUNTRY_LINKS;
        Array::from_header(bytes[start..start + 16].try_into().unwrap(), 4).unwrap()
    }

    fn copy_fixture_program(log: &mut RawLog<4>, memory: &ProgramSource<'_>) -> bool {
        use super::projects_test_kit as kit;
        let links = program_links(memory.memory.bytes);
        copy_program_row(
            log,
            memory,
            kit::BASE,
            ProjectCensus {
                links: &links,
                countries: 3,
                census: &[CensusEntry {
                    pointer: kit::GAME_STATE + kit::C1,
                    id: 1,
                    slot: 1,
                    canonical: true,
                }],
            },
            memory.pointer,
        )
    }

    /// Exact successful fixture observations through the original scalar path.
    /// Optional/invalid fixture cases shorten or replace this explicit script.
    fn program_script(bytes: &[u8], pointer: usize) -> Vec<(Option<usize>, usize)> {
        use super::projects_test_kit as kit;
        let at = |offset| pointer.checked_add(offset);
        let heap = kit::GAME_STATE + kit::HEAP;
        let text_len = b"program-building-01".len();
        vec![
            (Some(pointer), 16),
            (at(nproject_cprogram::OWNER_PRIMARY), 8),
            (at(nproject_cprogram::OWNER_COPY), 12),
            (at(nproject_cprogram::SCIENTIST_REFERENCE_TYPE), 8),
            (at(nproject_cprogram::ACTIVE_FLAG), 1),
            (at(nproject_cprogram::BUILDING), 32),
            (at(nproject_cprogram::ACTIVE_PROJECT_HANDLE), 4),
            (Some(heap + text_len), 1),
            (Some(heap), text_len),
            (Some(program_links(bytes).data + 4), 4),
        ]
    }

    fn scalar_program_script(
        log: &mut RawLog<4>,
        memory: &ProgramSource<'_>,
        script: &[(Option<usize>, usize)],
    ) -> bool {
        script
            .iter()
            .all(|&(address, len)| span(log, memory, address, len, M_PROGRAMS).is_some())
    }

    fn dirty_program_log(span_capacity: usize, byte_capacity: usize) -> RawLog<4> {
        let mut log = RawLog::new(span_capacity, byte_capacity);
        log.bytes.extend_from_slice(&vec![0xa7; byte_capacity]);
        log.clear();
        log
    }

    fn assert_program_logs_equal(actual: &RawLog<4>, expected: &RawLog<4>) {
        assert_eq!(actual.spans, expected.spans);
        assert_eq!(actual.bytes, expected.bytes);
        assert_eq!(actual.fault, expected.fault);
        assert_eq!(actual.raw_bytes, expected.raw_bytes);
        assert_eq!(actual.overflowed, expected.overflowed);
        assert_eq!(actual.has_bulk, expected.has_bulk);
        assert_eq!(actual.grow_for_next(), expected.grow_for_next());
    }

    #[test]
    fn program_body_batch_matches_scalar_at_every_fault_and_partial_byte() {
        let bytes = projects_test_kit::fixture();
        let pointer = projects_test_kit::GAME_STATE + projects_test_kit::PROG1;
        let script = program_script(&bytes, pointer);
        for failed in 0..=script.len() {
            for partial in 0..=script.get(failed).map_or(0, |field| field.1) {
                let source = ProgramSource {
                    fail: (failed < script.len()).then_some(failed),
                    partial,
                    ..ProgramSource::new(&bytes)
                };
                let reference = ProgramSource {
                    fail: source.fail,
                    partial,
                    ..ProgramSource::new(&bytes)
                };
                let mut actual = dirty_program_log(16, 256);
                let mut expected = dirty_program_log(16, 256);
                assert_eq!(
                    copy_fixture_program(&mut actual, &source),
                    scalar_program_script(&mut expected, &reference, &script),
                    "failed={failed}, partial={partial}"
                );
                assert_program_logs_equal(&actual, &expected);
                assert_eq!(*source.calls.borrow(), *reference.calls.borrow());
                assert_eq!(source.calls.borrow().len(), (failed + 1).min(script.len()));
                assert!(actual.bytes.get(actual.bytes.len()).is_none());
                assert_eq!(source.scatters.borrow().len(), usize::from(failed != 0));
                if failed != 0 {
                    let body: Vec<_> = script[1..6]
                        .iter()
                        .map(|&(address, len)| (address.unwrap(), len))
                        .collect();
                    assert_eq!(source.scatters.borrow()[0], body);
                }
                if failed == script.len() {
                    assert_eq!(
                        actual.spans[1..6].iter().map(|span| span.len).sum::<u32>(),
                        61
                    );
                    assert!(actual.spans.iter().all(|span| span.mask == M_PROGRAMS));
                    assert_eq!(actual.raw_bytes, [105, 0, 0, 0]);
                }
            }
        }
    }

    #[test]
    fn program_body_batch_preserves_every_capacity_prefix_and_growth() {
        let bytes = projects_test_kit::fixture();
        let pointer = projects_test_kit::GAME_STATE + projects_test_kit::PROG1;
        let script = program_script(&bytes, pointer);
        for span_capacity in 0..=11 {
            for byte_capacity in 0..=106 {
                let source = ProgramSource::new(&bytes);
                let reference = ProgramSource::new(&bytes);
                let mut actual = dirty_program_log(span_capacity, byte_capacity);
                let mut expected = dirty_program_log(span_capacity, byte_capacity);
                assert_eq!(
                    copy_fixture_program(&mut actual, &source),
                    scalar_program_script(&mut expected, &reference, &script),
                    "spans={span_capacity}, bytes={byte_capacity}"
                );
                assert_program_logs_equal(&actual, &expected);
                assert_eq!(*source.calls.borrow(), *reference.calls.borrow());
                assert_eq!(
                    source.scatters.borrow().len(),
                    usize::from(span_capacity >= 6 && byte_capacity >= 77)
                );
                actual.grow();
                expected.grow();
                assert_eq!(actual.spans.capacity(), expected.spans.capacity());
                assert_eq!(actual.bytes.capacity(), expected.bytes.capacity());
            }
        }
        // Identity leaves nine body bytes: publish eight, then demand twelve
        // more (20 body bytes total), rather than preflighting all 61.
        let source = ProgramSource::new(&bytes);
        let mut log = dirty_program_log(16, 25);
        assert!(!copy_fixture_program(&mut log, &source));
        assert_eq!(log.bytes.len(), 24);
        assert_eq!(source.calls.borrow().len(), 2);
        assert!(source.scatters.borrow().is_empty());
        log.grow();
        assert_eq!(log.bytes.capacity(), 50);
    }

    #[test]
    fn program_body_batch_preserves_checked_address_and_canonical_prefixes() {
        let bytes = projects_test_kit::fixture();
        for pointer in [
            0,
            usize::MAX - 15,
            usize::MAX - 16,
            usize::MAX - 32,
            usize::MAX - 0x100,
            usize::MAX - 0x170,
            usize::MAX - 0x178,
            usize::MAX - 0x17c,
            0x0000_7fff_ffff_ff00,
            0x0000_8000_0000_0000,
        ] {
            let source = ProgramSource {
                pointer,
                ..ProgramSource::new(&bytes)
            };
            let reference = ProgramSource {
                pointer,
                ..ProgramSource::new(&bytes)
            };
            let script = program_script(&bytes, pointer);
            let mut actual = dirty_program_log(16, 256);
            let mut expected = dirty_program_log(16, 256);
            assert_eq!(
                copy_fixture_program(&mut actual, &source),
                scalar_program_script(&mut expected, &reference, &script),
                "pointer={pointer:#x}"
            );
            assert_program_logs_equal(&actual, &expected);
            assert_eq!(*source.calls.borrow(), *reference.calls.borrow());
            if script[1..6].iter().any(|field| field.0.is_none()) {
                assert!(source.scatters.borrow().is_empty());
            }
        }
    }

    #[test]
    fn program_body_batch_keeps_identity_owner_presence_and_string_gates() {
        use super::projects_test_kit as kit;
        let pointer = kit::GAME_STATE + kit::PROG1;
        for case in 0..=14 {
            let mut bytes = kit::fixture();
            let mut script = program_script(&bytes, pointer);
            let building = kit::PROG1 + nproject_cprogram::BUILDING;
            let success = match case {
                0 => true,
                1..=3 => {
                    let (offset, width) = match case {
                        1 => (0, 8),
                        2 => (nproject_cprogram::TYPE, 4),
                        _ => (nproject_cprogram::ID, 4),
                    };
                    bytes[kit::PROG1 + offset..kit::PROG1 + offset + width].fill(0);
                    script.truncate(1);
                    false
                }
                4 => {
                    bytes[kit::PROG1 + nproject_cprogram::OWNER_COPY] = 2;
                    script.truncate(6);
                    false
                }
                5 => {
                    bytes[kit::PROG1 + nproject_cprogram::ACTIVE_FLAG] = 2;
                    script.truncate(6);
                    false
                }
                6 => {
                    bytes[kit::PROG1 + nproject_cprogram::ACTIVE_FLAG] = 0;
                    script.remove(6);
                    true
                }
                7..=10 => {
                    match case {
                        7 => bytes[building + 24..building + 32]
                            .copy_from_slice(&14_u64.to_le_bytes()),
                        8 => bytes[building + 16..building + 24]
                            .copy_from_slice(&20_u64.to_le_bytes()),
                        9 => bytes[building..building + 8].fill(0),
                        _ => bytes[building..building + 8]
                            .copy_from_slice(&((usize::MAX - 3) as u64).to_le_bytes()),
                    }
                    script.truncate(7);
                    false
                }
                11 | 12 => {
                    bytes[building..building + 32].fill(0);
                    let len = if case == 11 { 0 } else { 8 };
                    if len != 0 {
                        bytes[building..building + len].copy_from_slice(b"facility");
                    }
                    bytes[building + 16..building + 24]
                        .copy_from_slice(&(len as u64).to_le_bytes());
                    bytes[building + 24..building + 32]
                        .copy_from_slice(&msvc_string::INLINE_CAPACITY.to_le_bytes());
                    script[7] = (Some(pointer + nproject_cprogram::BUILDING + len), 1);
                    if len == 0 {
                        script.remove(8);
                    } else {
                        script[8] = (Some(pointer + nproject_cprogram::BUILDING), len);
                    }
                    true
                }
                13 => {
                    for offset in [
                        nproject_cprogram::OWNER_PRIMARY,
                        nproject_cprogram::OWNER_COPY,
                    ] {
                        bytes[kit::PROG1 + offset..kit::PROG1 + offset + 4].fill(0);
                    }
                    script.truncate(9);
                    false
                }
                _ => {
                    // The mirror stages the terminator; its value is validated
                    // during typed decoding, not by an extra live read/gate.
                    bytes[kit::HEAP + b"program-building-01".len()] = 7;
                    true
                }
            };
            let source = ProgramSource::new(&bytes);
            let reference = ProgramSource::new(&bytes);
            let mut actual = dirty_program_log(16, 256);
            let mut expected = dirty_program_log(16, 256);
            assert_eq!(
                copy_fixture_program(&mut actual, &source),
                success,
                "case={case}"
            );
            assert!(scalar_program_script(&mut expected, &reference, &script));
            assert_program_logs_equal(&actual, &expected);
            assert_eq!(
                *source.calls.borrow(),
                *reference.calls.borrow(),
                "case={case}"
            );
            assert_eq!(
                source.scatters.borrow().len(),
                usize::from(!(1..=3).contains(&case))
            );
        }
        let bytes = kit::fixture();
        let source = ProgramSource::new(&bytes);
        let links = program_links(&bytes);
        let mut log = dirty_program_log(16, 256);
        assert!(!copy_program_row(
            &mut log,
            &source,
            usize::MAX,
            ProjectCensus {
                links: &links,
                countries: 3,
                census: &[]
            },
            pointer,
        ));
        assert!(source.calls.borrow().is_empty());
        assert!(source.scatters.borrow().is_empty());
        assert!(log.spans.is_empty());
    }

    #[test]
    fn program_body_batch_preserves_duplicate_observations() {
        let bytes = projects_test_kit::fixture();
        let source = ProgramSource::new(&bytes);
        let reference = ProgramSource::new(&bytes);
        let script = program_script(&bytes, source.pointer);
        let mut actual = dirty_program_log(32, 256);
        let mut expected = dirty_program_log(32, 256);
        for _ in 0..2 {
            assert!(copy_fixture_program(&mut actual, &source));
            assert!(scalar_program_script(&mut expected, &reference, &script));
        }
        assert_program_logs_equal(&actual, &expected);
        assert_eq!(*source.calls.borrow(), *reference.calls.borrow());
        assert_eq!(source.scatters.borrow().len(), 2);
        assert_eq!(actual.spans.len(), 20);
        assert_eq!(actual.raw_bytes, [210, 0, 0, 0]);
    }

    #[test]
    fn program_body_batch_keeps_plan_blocking_and_cold_retirement() {
        use super::projects_test_kit as kit;
        for failed in [false, true] {
            let bytes = kit::fixture();
            let source = ProgramSource::new(&bytes);
            let mut log = dirty_program_log(16, 256);
            log.initialize_read_plan(16, 16, 128);
            assert!(!log.begin_planned_capture(&source, [0; 4]));
            assert!(log.take(&source, source.pointer, 16, M_PROGRAMS).is_some());
            let owner = log
                .take(
                    &source,
                    source.pointer + nproject_cprogram::OWNER_PRIMARY,
                    8,
                    M_PROGRAMS,
                )
                .unwrap();
            assert_eq!(log.bytes[owner], 1);
            log.finish_planned_capture();
            log.prepare_read_plan();
            log.clear();

            let mut changed = bytes.clone();
            changed[kit::PROG1 + nproject_cprogram::OWNER_PRIMARY] = 2;
            let current = ProgramSource {
                fail: failed.then_some(1),
                ..ProgramSource::new(&changed)
            };
            assert!(!log.begin_planned_capture(&current, [0; 4]));
            assert_eq!(current.calls.borrow().len(), 2);
            assert!(!copy_fixture_program(&mut log, &current));
            assert!(log.planned_capture_failed());
            assert!(log.planned_capture_requires_no_retry());
            assert_eq!(log.fault, [true; 4]);
            assert_eq!(log.spans.len(), 1);
            assert_eq!(log.bytes.len(), 16);
            assert_eq!(current.calls.borrow().len(), 2);
            assert!(current.scatters.borrow().is_empty());
            log.finish_planned_capture();
            log.prepare_read_plan();
            log.clear();

            // Retired plans permit fresh discovery, while the body scatter
            // keeps that new capture ineligible for a scalar compiled plan.
            for _ in 0..2 {
                let fresh = ProgramSource::new(&bytes);
                assert!(!log.begin_planned_capture(&fresh, [0; 4]));
                assert!(fresh.calls.borrow().is_empty());
                assert!(copy_fixture_program(&mut log, &fresh));
                assert!(!log.planned_capture_failed());
                assert_eq!(fresh.scatters.borrow().len(), 1);
                assert_eq!(fresh.calls.borrow().len(), 10);
                log.finish_planned_capture();
                log.prepare_read_plan();
                log.clear();
            }
        }
    }

    #[test]
    fn split_copy_matches_direct_walk() {
        use super::projects_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let capacities = split_capacities();
        let direct = kit::direct(&memory, &capacities);
        let memory = engine::memory::CountedMemory::new(memory);
        let native = read_accounting::Capture::new();
        let (split, report, _) = kit::split(&memory, &capacities);
        let counts = native.finish();
        let mut expected_counts = read_accounting::ZERO;
        expected_counts[TABLES[0].index()] = counts[TABLES[0].index()];
        assert!(counts[TABLES[0].index()].spans > 0);
        assert!(counts[TABLES[0].index()].bytes > 0);
        assert_eq!(counts, expected_counts);
        assert!(report.raw_ok);
        kit::assert_matches_direct(&split, &direct);
        // Spot-checks: program identity with heap building bytes, project
        // lifecycle and state, history context, breakthrough payload.
        assert_eq!(split.programs.outcome, Outcome::Rows { len: 1 });
        let program = &split.programs.rows[0];
        assert_eq!((program.id, program.owner, program.province), (5, 1, 13413));
        assert_eq!(program.active_project, Some(9));
        assert_eq!(
            &split.names.first
                [program.building_start..program.building_start + program.building_len],
            b"program-building-01"
        );
        assert_eq!(split.projects.outcome, Outcome::Rows { len: 1 });
        let project = &split.projects.rows[0];
        assert_eq!(
            (project.owner, project.local_id, project.definition_id),
            (1, 3, 45)
        );
        assert_eq!((project.started, project.completed), (1, 0));
        assert_eq!(split.history.outcome, Outcome::Rows { len: 1 });
        let history = &split.history.rows[0];
        assert_eq!(
            (
                history.owner,
                history.project,
                history.prototype_reward_handle,
                history.option_handle,
                history.date
            ),
            (1, 3, 11, 22, 333)
        );
        assert_eq!(
            (
                history.context.province,
                history.context.country_reference,
                history.context.state_reference
            ),
            (-5, Some(1), Some(9))
        );
        assert_eq!(split.breakthroughs.outcome, Outcome::Rows { len: 1 });
        let breakthrough = &split.breakthroughs.rows[0];
        assert_eq!(
            (
                breakthrough.owner,
                breakthrough.specialization,
                breakthrough.points
            ),
            (1, 77, -8)
        );
    }

    #[test]
    fn projects_truncation_matrix_matches_direct_with_same_caps() {
        // B3: zero/one/exact-fit/overflow-by-one/massive run typed AND
        // copy→decode against the SAME memory with the SAME pooled caps,
        // asserting identical outcome (incl. len/unread) and identical rows.
        // Fixture holds 1 program, 1 project, 1 history, 1 breakthrough, so
        // cap 1 is both the one-row and exact-fit arm, cap 0 is both the
        // zero-cap, overflow-by-one (total 1 == cap+1) and massive arm;
        // u32 unread saturation is not reachable and is covered by the
        // walk's own saturating arithmetic.
        use super::projects_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        for rows in [[0; N], [1; N], [256; N]] {
            let capacities = Capacities {
                rows,
                program_names: 8192,
                program_candidates: 1024,
            };
            let direct = kit::direct(&memory, &capacities);
            let (split, report, _) = kit::split(&memory, &capacities);
            assert!(report.raw_ok, "rows={rows:?}");
            kit::assert_matches_direct(&split, &direct);
        }
        // Arena arms: names/candidates truncation agrees on both paths.
        for (names, candidates) in [(0, 0), (4, 1), (8192, 1024)] {
            let capacities = Capacities {
                rows: [256; N],
                program_names: names,
                program_candidates: candidates,
            };
            let direct = kit::direct(&memory, &capacities);
            let (split, report, _) = kit::split(&memory, &capacities);
            assert!(report.raw_ok);
            kit::assert_matches_direct(&split, &direct);
        }
        // Exact-fit arm pins Rows on every table.
        let capacities = Capacities {
            rows: [1; N],
            program_names: 8192,
            program_candidates: 1024,
        };
        let (split, _, _) = kit::split(&memory, &capacities);
        assert_eq!(split.programs.outcome, Outcome::Rows { len: 1 });
        assert_eq!(split.projects.outcome, Outcome::Rows { len: 1 });
        assert_eq!(split.history.outcome, Outcome::Rows { len: 1 });
        assert_eq!(split.breakthroughs.outcome, Outcome::Rows { len: 1 });
    }

    #[test]
    fn program_inline_building_name_is_copied_for_the_arena_read() {
        use super::projects_test_kit as kit;
        let mut bytes = kit::fixture();
        let at = kit::PROG1 + nproject_cprogram::BUILDING;
        bytes[at..at + 32].fill(0);
        bytes[at..at + 8].copy_from_slice(b"facility");
        bytes[at + 16..at + 24].copy_from_slice(&8_u64.to_le_bytes());
        bytes[at + 24..at + 32].copy_from_slice(&msvc_string::INLINE_CAPACITY.to_le_bytes());
        let memory = kit::memory(&bytes);
        let capacities = split_capacities();
        let direct = kit::direct(&memory, &capacities);
        let (split, report, _) = kit::split(&memory, &capacities);
        assert!(report.raw_ok);
        kit::assert_matches_direct(&split, &direct);
        assert_eq!(split.programs.outcome, Outcome::Rows { len: 1 });
        assert_eq!(&split.names.first[..8], b"facility");
    }

    #[test]
    fn split_copy_matches_direct_walk_under_truncation() {
        use super::projects_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let capacities = Capacities {
            rows: [1; N],
            program_names: 4,
            program_candidates: 1,
        };
        let direct = kit::direct(&memory, &capacities);
        let (split, report, _) = kit::split(&memory, &capacities);
        assert!(report.raw_ok);
        kit::assert_matches_direct(&split, &direct);
    }

    #[test]
    fn split_copy_matches_direct_walk_on_fault_matrix() {
        use super::projects_test_kit as kit;
        use engine::memory::SliceMemory;
        struct Fault<'a> {
            memory: SliceMemory<'a>,
            fault: std::ops::Range<usize>,
        }
        impl engine::Memory for Fault<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let Some(end) = address.checked_add(output.len()) else {
                    return false;
                };
                if address < self.fault.end && end > self.fault.start {
                    return false;
                }
                self.memory.copy(address, output)
            }
        }
        // Primary raw fault attribution remains independent of decoded outcomes.
        for (offset, fault_mask) in [
            (cgame_state::COUNTRIES, [true, true, true, true]),
            (kit::C1, [true, true, true, true]),
            (kit::S1, [true, true, true, true]),
            (kit::PROG1, [true, false, false, false]),
            (kit::HEAP, [true, false, false, false]),
            (kit::PROJ1, [false, true, false, false]),
            (kit::MACH1, [false, true, false, false]),
            (kit::HIST1, [false, false, true, false]),
            (
                kit::HIST1 + nproject_sproject_history::CONTEXT,
                [false, false, true, false],
            ),
            (kit::BT_NODE, [false, false, false, true]),
        ] {
            let bytes = kit::fixture();
            let fault = kit::GAME_STATE + offset;
            let memory = Fault {
                memory: SliceMemory {
                    base: kit::GAME_STATE,
                    bytes: &bytes,
                },
                fault: fault..fault + 1,
            };
            let capacities = split_capacities();
            let direct = kit::direct(&memory, &capacities);
            let (split, report, job) = kit::split(&memory, &capacities);
            assert!(report.raw_ok, "offset={offset:x}");
            kit::assert_matches_direct(&split, &direct);
            assert_eq!(job.log.fault, fault_mask, "offset={offset:x}");
        }
    }

    #[test]
    fn staging_overflow_falls_back_to_typed_walk() {
        use super::projects_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let ctx = ProjectsCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            active: true,
        };
        let mut tiny = ProjectsJob {
            log: crate::raw::RawLog::new(8, 8),
            game_state: 0,
            active: false,
            census: Vec::with_capacity(1),
            pointers: Vec::with_capacity(1),
            capacity_charge: crate::raw_memory::Charge::default(),
        };
        tiny.update_capacity_charge();
        let report = copy_projects_refresh(&memory, ctx, &mut tiny);
        assert!(!report.raw_ok);
        assert!(tiny.log.overflowed);
        assert!(tiny.grow_for_next());
    }
}
