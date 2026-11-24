
use crate::{
    clock::qpc,
    raw::RawLog,
    read_accounting,
    slicing::{SliceFingerprints, SliceSelection},
    table_capture::{Arena, CaptureRows, bounded, grow_capacity, grow_vec},
    table_conversion::{table, table_iter, unavailable},
};
use engine_allocation::{copy_vec, default_vec, map_slice, owned_vec};
use engine_characters::scientist_roster;
use engine_events::cnuke;
use engine_native_air::raid_tooltip_operands;
use engine_native_memory::{Memory, cpdx_array::Array, msvc_string, walk::Outcome};
use engine_native::{ccountry, cgame_state};
use engine_native_air::{
    ccountry_raid_status,
    craid_instance,
    craid_system,
    raid_equipment,
    raid_speed_inputs,
};
use engine_peace::peace_conference;
use engine_projects::{
    nproject_breakthrough_node, nproject_cbreakthrough_progress, nproject_cprogram,
    nproject_cprogram_status, nproject_cproject, nproject_sproject_history,
};
use engine_raid_inputs::{raid_facilities, raid_path_memo};
use engine_visibility::cplayer_status_cache;
use replay_data::{rows, rows::TableId, rows::tables};
use replay_frame::frame::TableView;
use std::mem::size_of;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

static SCIENTIST_ACTIVE_REFUSALS: AtomicU64 = AtomicU64::new(0);
static SCIENTIST_VISIBILITY_REFUSALS: AtomicU64 = AtomicU64::new(0);
static SCIENTIST_INJURY_REFUSALS: AtomicU64 = AtomicU64::new(0);
static PROGRAM_FACILITY_REFUSALS: AtomicU64 = AtomicU64::new(0);
static PROGRAM_ASSIGNMENT_REFUSALS: AtomicU64 = AtomicU64::new(0);
static PROJECT_VARIABLE_REFUSALS: AtomicU64 = AtomicU64::new(0);
static PROJECT_REWARD_REFUSALS: AtomicU64 = AtomicU64::new(0);

pub fn scientist_active_data() -> impl serde::Serialize {
    let refused = SCIENTIST_ACTIVE_REFUSALS.load(Relaxed);
    let injury = SCIENTIST_INJURY_REFUSALS.load(Relaxed);
    let visibility = SCIENTIST_VISIBILITY_REFUSALS.load(Relaxed);
    let assignment = PROGRAM_ASSIGNMENT_REFUSALS.load(Relaxed);
    let facility = PROGRAM_FACILITY_REFUSALS.load(Relaxed);
    let variables = PROJECT_VARIABLE_REFUSALS.load(Relaxed);
    let rewards = PROJECT_REWARD_REFUSALS.load(Relaxed);
    crate::status::status!({
        "refused": refused + injury + assignment + variables + rewards + facility + visibility,
        "refusals": crate::status::map([
            ("scientist active byte unreadable or address overflow", refused),
            ("scientist injury timer unreadable or address overflow", injury),
            ("scientist retained visibility template unreadable", visibility),
            ("program scientist assignments unreadable or capacity insufficient", assignment),
            ("facility speed and specialization operands unavailable", facility),
            ("project variable store unavailable", variables),
            ("pending project reward unavailable", rewards),
        ].into_iter().filter(|(_, count)| *count != 0)),
        "scope": "cumulative unavailable scientist/project detail observations; membership and skills remain available",
    })
}

fn freeze_project_variables(
    entries: &[engine_scripts::cvariables::Row],
    names: &[u8],
    seed: u32,
) -> std::io::Result<std::sync::Arc<rows::ProjectVariableStore>> {
    let entries = map_slice(entries, |entry| rows::ProjectVariable {
        name: owned_vec(copy_vec(
            &names[entry.name_start..entry.name_start + entry.name_len],
        )),
        value_raw: entry.value_raw,
    })?;
    engine_allocation::allocation_attempt()?;
    Ok(std::sync::Arc::new(rows::ProjectVariableStore {
        seed,
        entries,
    }))
}

#[cfg(any(test, feature = "test-support"))]
pub fn deferred_variable_allocation_fixture() -> Captured {
    let mut captured = Captured::vacant();
    captured.project_runtime = runtime::Runtime::allocation_failure_fixture();
    captured
}

#[cfg(any(test, feature = "test-support"))]
pub mod variable_test_kit;

pub const N: usize = 20;
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
    TableId::RaidCooldowns,
    TableId::RaidPathDistances,
    TableId::RaidPlayerAllowedTypes,
    TableId::RaidSpeedInputs,
    TableId::RaidEquipment,
    TableId::RaidFacilities,
    TableId::RaidIntelInputs,
    TableId::RaidTooltipOperands,
    TableId::ScientistDaily,
];

pub const FAMILY: [usize; 2] = [1, 2];

pub const RAW: [TableId; 2] = [TableId::ProjectPrograms, TableId::ProjectBreakthroughs];

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    pub rows: [usize; N],
    pub program_names: usize,
    pub program_candidates: usize,
    pub raid_type_tokens: usize,
    pub raid_show_for: usize,
    pub raid_transports: usize,
    pub raid_equipment_items: usize,
    pub raid_equipment_archetypes: usize,
    pub counteroperatives: usize,
}

collector_primitives::capacity_max!(Capacities {
    rows,
    program_names,
    program_candidates,
    raid_type_tokens,
    raid_show_for,
    raid_transports,
    raid_equipment_items,
    raid_equipment_archetypes,
    counteroperatives,
});

impl Capacities {
    pub fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let (projects, histories) =
            nproject_cproject::count_with_history(memory, base, state);
        let demand = |outcome| match outcome {
            Outcome::Rows { len } => Ok(len),
            Outcome::Truncated { len, unread_rows } => len
                .checked_add(unread_rows as usize)
                .ok_or("project row demand overflow"),
            Outcome::Unavailable { reason } => Err(reason),
        };
        let programs = nproject_cprogram::count(memory, base, state);
        if let Outcome::Unavailable { reason } = programs.outcome
            && reason != "program pointer scratch capacity insufficient"
        {
            return Err(reason);
        }
        grow_capacity(&mut self.program_candidates, programs.required_candidates);
        grow_capacity(&mut self.rows[0], programs.required_candidates);
        grow_capacity(&mut self.rows[1], demand(projects)?);
        grow_capacity(&mut self.rows[2], demand(histories)?);
        Ok(())
    }

    pub fn raise_for_countries(&mut self, slots: usize) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.rows[4], rows(slots, 2));
        raise(&mut self.rows[5], rows(slots, 1));
        raise(&mut self.rows[17], rows(slots, 1));
    }

    pub fn raise_for_raid_type_tokens(&mut self, tokens: usize) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.raid_type_tokens, rows(tokens, 1));
    }

    pub fn raise_for_raid_paths(&mut self, count: usize) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.rows[12], rows(count, 1));
    }

    pub fn raise_for_facilities(&mut self, count: usize) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.rows[16], rows(count, 1));
    }
}

pub const ACQUISITION_PHASE_NAMES: [&str; 5] = [
    "selection",
    "project_native",
    "runtime_native",
    "family_inclusive",
    "cold_join",
];

pub struct Captured {
    profile_acquisition: bool,
    acquisition_phases: [u64; 5],
    programs: CaptureRows<nproject_cprogram::Row>,
    facilities: CaptureRows<raid_facilities::Row>,
    raid_intel: CaptureRows<engine_raid_inputs::raid_intel_inputs::Row>,
    agency_defense: Vec<Option<super::agency_defense::Tree>>,
    tooltip_operands: CaptureRows<engine_native_air::raid_tooltip_operands::Row>,
    counteroperatives: Arena<engine_raid_inputs::raid_intel_inputs::Operative>,
    projects: CaptureRows<nproject_cproject::Row>,
    history: CaptureRows<nproject_sproject_history::Row>,
    breakthroughs: CaptureRows<nproject_cbreakthrough_progress::Row>,
    nukes: CaptureRows<cnuke::Row>,
    raid_countries: CaptureRows<ccountry_raid_status::Row>,
    raids: CaptureRows<craid_instance::Row>,
    peace: CaptureRows<peace_conference::Row>,
    scientists: CaptureRows<scientist_roster::Row>,
    scientist_skills: CaptureRows<scientist_roster::SkillRow>,
    scientist_names: Vec<u8>,
    raid_cooldowns: CaptureRows<craid_system::CooldownRow>,
    raid_paths: [CaptureRows<raid_path_memo::Row>; 2],
    raid_player: CaptureRows<cplayer_status_cache::Row>,
    raid_speeds: CaptureRows<raid_speed_inputs::Row>,
    raid_transports: Arena<i64>,
    raid_equipment: CaptureRows<raid_equipment::Row>,
    raid_equipment_variants: Arena<u64>,
    raid_equipment_amounts: Arena<i64>,
    raid_equipment_archetype_tokens: Arena<u32>,
    raid_equipment_archetype_amounts: Arena<i64>,
    speed_memo: engine_native_parts::modifier_definition::DefinitionMemo,
    raid_reads: super::RaidReadCounts,
    raid_type_tokens: Arena<u32>,
    raid_show_for: Arena<u32>,
    raid_type_memo: ccountry_raid_status::TypeTokens,
    names: Arena<u8>,
    candidates: Vec<usize>,
    required_candidates: usize,
    pub raw_mode: bool,
    project_selection: SliceSelection,
    project_changed: crate::slicing::Changed,
    project_links: Vec<u8>,
    project_owners: Vec<(u32, u32)>,
    project_read: Vec<(u32, u32)>,
    project_runtime: runtime::Runtime,
    scientist_injury_read: bool,
    rest_lent: bool,
}

impl Captured {
    pub fn fail_gated_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        let outcomes = TABLES.map(|table| super::failed_capture(table, hour, force_full, reason));
        if !self.raw_mode {
            self.programs.fail(outcomes[0]);
            self.breakthroughs.fail(outcomes[3]);
        }
        self.projects.fail(outcomes[1]);
        self.history.fail(outcomes[2]);
        self.project_selection.reset_daily(hour, force_full);
        self.nukes.fail(outcomes[4]);
        self.raid_countries.fail(outcomes[5]);
        self.raids.fail(outcomes[6]);
        self.peace.fail(outcomes[7]);
        self.scientists.fail(outcomes[8]);
        self.scientist_skills.fail(outcomes[9]);
        self.scientist_injury_read =
            replay_data::cadence::should_walk(TableId::ScientistDaily, hour, force_full);
        self.raid_cooldowns.fail(outcomes[11]);
        for memo in &mut self.raid_paths {
            memo.fail(outcomes[12]);
        }
        self.raid_player.fail(outcomes[13]);
        self.raid_speeds.fail(outcomes[14]);
        self.raid_equipment.fail(outcomes[15]);
        self.facilities.fail(outcomes[16]);
        self.raid_intel.fail(outcomes[17]);
        self.tooltip_operands.fail(outcomes[18]);
    }

    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: [0; N],
            program_names: 0,
            program_candidates: 0,
            raid_type_tokens: 0,
            raid_show_for: 0,
            raid_transports: 0,
            raid_equipment_items: 0,
            raid_equipment_archetypes: 0,
            counteroperatives: 0,
        })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            programs: CaptureRows::new(capacity.rows[0]),
            facilities: CaptureRows::new(capacity.rows[16]),
            raid_intel: CaptureRows::new(capacity.rows[17]),
            agency_defense: Vec::new(),
            tooltip_operands: CaptureRows::new(capacity.rows[18]),
            counteroperatives: Arena::new(capacity.counteroperatives),
            projects: CaptureRows::new(capacity.rows[1]),
            history: CaptureRows::new(capacity.rows[2]),
            breakthroughs: CaptureRows::new(capacity.rows[3]),
            nukes: CaptureRows::new(capacity.rows[4]),
            raid_countries: CaptureRows::new(capacity.rows[5]),
            raids: CaptureRows::new(capacity.rows[6]),
            peace: CaptureRows::new(capacity.rows[7]),
            scientists: CaptureRows::new(capacity.rows[8]),
            scientist_skills: CaptureRows::new(capacity.rows[9]),
            scientist_names: Vec::new(),
            raid_cooldowns: CaptureRows::new(capacity.rows[11]),
            raid_paths: std::array::from_fn(|_| CaptureRows::new(capacity.rows[12])),
            raid_player: CaptureRows::new(capacity.rows[13]),
            raid_speeds: CaptureRows::new(capacity.rows[14]),
            raid_transports: Arena::new(capacity.raid_transports),
            raid_equipment: CaptureRows::new(capacity.rows[15]),
            raid_equipment_variants: Arena::new(capacity.raid_equipment_items),
            raid_equipment_amounts: Arena::new(capacity.raid_equipment_items),
            raid_equipment_archetype_tokens: Arena::new(capacity.raid_equipment_archetypes),
            raid_equipment_archetype_amounts: Arena::new(capacity.raid_equipment_archetypes),
            speed_memo: engine_native_parts::modifier_definition::DefinitionMemo::default(),
            raid_reads: super::RaidReadCounts::default(),
            raid_type_tokens: Arena::new(capacity.raid_type_tokens),
            raid_show_for: Arena::new(capacity.raid_show_for),
            raid_type_memo: ccountry_raid_status::TypeTokens::empty(),
            names: Arena::new(capacity.program_names),
            candidates: owned_vec(default_vec(capacity.program_candidates)),
            required_candidates: 0,
            raw_mode: false,
            project_selection: SliceSelection::new(),
            project_changed: crate::slicing::Changed::default(),
            project_links: Vec::new(),
            project_owners: Vec::new(),
            project_read: Vec::new(),
            project_runtime: runtime::Runtime::default(),
            profile_acquisition: false,
            acquisition_phases: [0; 5],
            scientist_injury_read: false,
            rest_lent: false,
        }
    }

    pub fn profile_acquisition(&mut self, enabled: bool) {
        self.profile_acquisition = enabled;
    }

    pub fn projects_certified(&self) -> bool {
        self.project_changed.certified
    }

    pub fn acquisition_phases(&self) -> Option<[u64; 5]> {
        self.profile_acquisition.then_some(self.acquisition_phases)
    }

    pub fn project_selection(&self) -> &SliceSelection {
        &self.project_selection
    }

    pub fn raid_reads(&self) -> super::RaidReadCounts {
        self.raid_reads
    }

    pub fn reset(&mut self) {
        self.programs.reset();
        self.facilities.reset();
        self.raid_intel.reset();
        self.agency_defense.clear();
        self.tooltip_operands.reset();
        self.counteroperatives.reset();
        self.projects.reset();
        self.acquisition_phases = [0; 5];
        self.history.reset();
        self.breakthroughs.reset();
        self.nukes.reset();
        self.raid_countries.reset();
        self.raids.reset();
        self.peace.reset();
        self.scientists.reset();
        self.scientist_skills.reset();
        self.raid_cooldowns.reset();
        for memo in &mut self.raid_paths {
            memo.reset();
        }
        self.raid_player.reset();
        self.raid_speeds.reset();
        self.raid_transports.reset();
        self.raid_equipment.reset();
        self.raid_equipment_variants.reset();
        self.raid_equipment_amounts.reset();
        self.raid_equipment_archetype_tokens.reset();
        self.raid_equipment_archetype_amounts.reset();
        self.raid_reads = super::RaidReadCounts::default();
        self.raid_type_tokens.reset();
        self.raid_show_for.reset();
        self.names.reset();
        self.required_candidates = 0;
    }

    pub fn allocated_bytes(&self) -> usize {
        self.project_runtime.allocated_bytes()
            + self.facilities.allocated_bytes()
            + self.raid_intel.allocated_bytes()
            + self.agency_defense.capacity() * size_of::<Option<super::agency_defense::Tree>>()
            + self.tooltip_operands.allocated_bytes()
            + self.counteroperatives.allocated_bytes()
            + self.programs.allocated_bytes()
            + self.projects.allocated_bytes()
            + self.history.allocated_bytes()
            + self.breakthroughs.allocated_bytes()
            + self.nukes.allocated_bytes()
            + self.raid_countries.allocated_bytes()
            + self.raids.allocated_bytes()
            + self.peace.allocated_bytes()
            + self.scientists.allocated_bytes()
            + self.scientist_skills.allocated_bytes()
            + self.scientist_names.capacity()
            + self.raid_cooldowns.allocated_bytes()
            + self
                .raid_paths
                .iter()
                .map(CaptureRows::allocated_bytes)
                .sum::<usize>()
            + self.raid_player.allocated_bytes()
            + self.raid_speeds.allocated_bytes()
            + self.raid_transports.allocated_bytes()
            + self.raid_equipment.allocated_bytes()
            + self.raid_equipment_variants.allocated_bytes()
            + self.raid_equipment_amounts.allocated_bytes()
            + self.raid_equipment_archetype_tokens.allocated_bytes()
            + self.raid_equipment_archetype_amounts.allocated_bytes()
            + self.speed_memo.allocated_bytes()
            + self.raid_type_tokens.allocated_bytes()
            + self.raid_show_for.allocated_bytes()
            + self.raid_type_memo.allocated_bytes()
            + self.names.allocated_bytes()
            + self.candidates.capacity() * size_of::<usize>()
            + self.project_links.capacity()
            + self.project_owners.capacity() * size_of::<(u32, u32)>()
            + self.project_read.capacity() * size_of::<(u32, u32)>()
    }

    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.programs.grow(&mut capacity.rows[0]);
        self.facilities.grow(&mut capacity.rows[16]);
        self.raid_intel.grow(&mut capacity.rows[17]);
        self.tooltip_operands.grow(&mut capacity.rows[18]);
        self.counteroperatives.grow(&mut capacity.counteroperatives);
        self.projects.grow(&mut capacity.rows[1]);
        self.history.grow(&mut capacity.rows[2]);
        self.breakthroughs.grow(&mut capacity.rows[3]);
        self.nukes.grow(&mut capacity.rows[4]);
        self.raid_countries.grow(&mut capacity.rows[5]);
        self.raids.grow(&mut capacity.rows[6]);
        self.peace.grow(&mut capacity.rows[7]);
        self.scientists.grow(&mut capacity.rows[8]);
        self.scientist_skills.grow(&mut capacity.rows[9]);
        self.raid_cooldowns.grow(&mut capacity.rows[11]);
        for memo in &mut self.raid_paths {
            memo.grow(&mut capacity.rows[12]);
        }
        for memo in &mut self.raid_paths {
            grow_vec(&mut memo.rows, capacity.rows[12]);
        }
        self.raid_player.grow(&mut capacity.rows[13]);
        self.raid_speeds.grow(&mut capacity.rows[14]);
        self.raid_transports.grow(&mut capacity.raid_transports);
        self.raid_equipment.grow(&mut capacity.rows[15]);
        let mut items = capacity.raid_equipment_items;
        self.raid_equipment_variants.grow(&mut items);
        self.raid_equipment_amounts
            .grow(&mut capacity.raid_equipment_items);
        capacity.raid_equipment_items = capacity.raid_equipment_items.max(items);
        let mut archetypes = capacity.raid_equipment_archetypes;
        self.raid_equipment_archetype_tokens.grow(&mut archetypes);
        self.raid_equipment_archetype_amounts
            .grow(&mut capacity.raid_equipment_archetypes);
        capacity.raid_equipment_archetypes = capacity.raid_equipment_archetypes.max(archetypes);
        self.raid_type_tokens.grow(&mut capacity.raid_type_tokens);
        self.raid_show_for.grow(&mut capacity.raid_show_for);
        if self
            .raid_type_memo
            .grow()
            .map_err(crate::allocation::io)
            .is_err()
        {
            return;
        }
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

    #[cfg(any(test, feature = "test-support"))]
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.capture_selected(
            memory,
            memory,
            base,
            game_state,
            hour,
            force_full,
            &mut SliceFingerprints::new(),
            &crate::country_input_changes::Observer,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the hourly capture operands (memory, base, game state, hour, full flag) travel beside the roster, fingerprints and change source"
    )]
    pub fn capture_selected(
        &mut self,
        memory: &impl Memory,
        roster: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        projects: &mut SliceFingerprints,
        changes: &dyn crate::country_input_changes::Changes,
    ) -> [u64; N] {
        self.capture_selected_impl(
            memory, roster, base, game_state, hour, force_full, projects, changes, false, None,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the hourly capture operands (memory, base, game state, hour, full flag) travel beside the roster, fingerprints and change source"
    )]
    pub fn acquire_selected(
        &mut self,
        memory: &impl Memory,
        roster: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        projects: &mut SliceFingerprints,
        changes: &dyn crate::country_input_changes::Changes,
    ) -> [u64; N] {
        self.capture_selected_impl(
            memory, roster, base, game_state, hour, force_full, projects, changes, true, None,
        )
    }

    pub fn acquire_selected_partitioned(
        &mut self,
        memory: &impl Memory,
        roster: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        projects: &mut SliceFingerprints,
        changes: &dyn crate::country_input_changes::Changes,
        dispatch: ProjectDispatch<'_>,
    ) -> [u64; N] {
        self.capture_selected_impl(
            memory,
            roster,
            base,
            game_state,
            hour,
            force_full,
            projects,
            changes,
            true,
            Some(dispatch),
        )
    }

    pub fn decode_owned(&mut self) {
        self.project_runtime.decode();
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the hourly capture operands (memory, base, game state, hour, full flag) travel beside the roster, fingerprints, change source, deferral and dispatch"
    )]
    fn capture_selected_impl(
        &mut self,
        memory: &impl Memory,
        roster: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        projects: &mut SliceFingerprints,
        changes: &dyn crate::country_input_changes::Changes,
        defer_runtime: bool,
        dispatch: Option<ProjectDispatch<'_>>,
    ) -> [u64; N] {
        self.reset();
        let mut ticks = [0; N];
        let active = TABLES.map(|table| replay_data::cadence::should_walk(table, hour, force_full));
        let walk = active[0] && !self.raw_mode;
        if self.raw_mode && active[0] {
        } else if !walk {
            let skipped = Outcome::Unavailable {
                reason: replay_data::cadence::SKIPPED_REASON,
            };
            self.programs.outcome = skipped;
            self.breakthroughs.outcome = skipped;
        } else {
            let begin = qpc();
            self.capture_programs(|candidates, rows, names| {
                let _reads = read_accounting::table(TABLES[0]);
                nproject_cprogram::walk(memory, base, game_state, candidates, rows, names)
            });
            ticks[0] = elapsed(begin);
            let begin = qpc();
            capture_rows(&mut self.breakthroughs, |rows| {
                let _reads = read_accounting::table(TABLES[3]);
                nproject_cbreakthrough_progress::walk(memory, base, game_state, rows)
            });
            ticks[3] = elapsed(begin);
        }
        let begin = qpc();
        self.capture_project_family(
            roster,
            base,
            game_state,
            hour,
            force_full,
            projects,
            changes,
            defer_runtime,
            dispatch,
        );
        ticks[1] = elapsed(begin);
        if !self.rest_lent {
            let mut rest = self.take_rest(game_state, hour, force_full);
            rest.capture(memory, base);
            for (ticks, rest) in ticks.iter_mut().zip(self.restore_rest(rest)) {
                *ticks += rest;
            }
        }
        ticks
    }

    fn capture_programs(
        &mut self,
        walk: impl FnOnce(
            &mut Vec<usize>,
            &mut Vec<nproject_cprogram::Row>,
            &mut Vec<u8>,
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

    fn raid_path_outcome(&self) -> Outcome {
        let (mut len, mut unread_rows) = (0usize, 0u32);
        for memo in &self.raid_paths {
            match memo.outcome {
                Outcome::Unavailable { reason } => return Outcome::Unavailable { reason },
                Outcome::Rows { len: count } => len += count,
                Outcome::Truncated {
                    len: count,
                    unread_rows: missing,
                } => {
                    len += count;
                    let Some(total) = unread_rows.checked_add(missing) else {
                        return Outcome::Unavailable {
                            reason: "raid path memo missing row count overflow",
                        };
                    };
                    unread_rows = total;
                }
            }
        }
        if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        }
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
            self.raid_cooldowns.outcome,
            self.raid_path_outcome(),
            self.raid_player.outcome,
            self.raid_speeds.outcome,
            self.raid_equipment.outcome,
            self.facilities.outcome,
            self.raid_intel.outcome,
            self.tooltip_operands.outcome,
            if self.scientist_injury_read {
                self.scientists.outcome
            } else {
                Outcome::Unavailable {
                    reason: replay_data::cadence::SKIPPED_REASON,
                }
            },
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
                runtime: self.project_runtime.value(r),
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
            self.raid_countries_table(outcomes[5]),
            self.raids_table(outcomes[6]),
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
                    active: rows::Recorded::Value(r.active),
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
            table::<tables::RaidTargets, _>(outcomes[10], &self.raids.rows, |r| {
                rows::RaidTargetRow {
                    reference_type: r.reference_type,
                    id: r.id,
                    scope: super::raid_target_statuses::scope(&r.target),
                    raid_type: r.raid_type,
                    detected: r.detected,
                }
            }),
            table::<tables::RaidCooldowns, _>(outcomes[11], &self.raid_cooldowns.rows, |r| {
                rows::RaidCooldownRow {
                    owner_slot: r.owner_slot,
                    ordinal: r.ordinal,
                    raid_type: r.raid_type,
                    scope: super::raid_target_statuses::scope(&r.scope),
                    days: r.days,
                }
            }),
            table_iter::<tables::RaidPathDistances, _>(
                outcomes[12],
                self.raid_paths
                    .iter()
                    .zip([
                        rows::RaidPathDispatcher::Land,
                        rows::RaidPathDispatcher::Naval,
                    ])
                    .flat_map(|(memo, dispatcher)| {
                        memo.rows
                            .iter()
                            .take(memo.outcome.len())
                            .map(move |row| (dispatcher, row))
                    }),
                |(dispatcher, r)| rows::RaidPathDistanceRow {
                    dispatcher,
                    source: r.source,
                    target: r.target,
                    distance: r.distance,
                    ok: r.ok,
                },
            ),
            self.raid_player_table(outcomes[13]),
            self.raid_speed_table(outcomes[14]),
            self.raid_equipment_table(outcomes[15]),
            table::<tables::RaidFacilities, _>(outcomes[16], &self.facilities.rows, |r| {
                rows::RaidFacilityRow {
                    country: r.country,
                    ordinal: r.ordinal,
                    id: r.id,
                    owner: r.owner,
                    province: r.province,
                    active_project: r.active_project,
                    project_definition: r.project_definition,
                }
            }),
            table_iter::<tables::RaidIntelInputs, _>(outcomes[17], self.raid_intel.rows.iter().take(outcomes[17].len()).enumerate(), |(index, r)| {
                rows::RaidIntelInputRow {
                    country: r.country,
                    defense_tree: rows::Recorded::Value(self.agency_defense.get(index)
                        .and_then(Clone::clone)),
                    strategic_present: u8::from(r.strategic_present),
                    country_defense: r.country_defense,
                    crypto_strength: r.crypto_strength,
                    operatives: owned_vec(map_slice(
                        &self.counteroperatives.first
                            [r.operatives_start..r.operatives_start + r.operatives_len],
                        |o| rows::RaidCounterOperative {
                            handle: o.handle,
                            defense: o.defense,
                            context: o.context,
                        },
                    )),
                }
            }),
            table::<tables::RaidTooltipOperands, _>(
                outcomes[18],
                &self.tooltip_operands.rows,
                |r| rows::RaidTooltipOperandRow {
                    raid: r.raid,
                    ordinal: r.ordinal,
                    kind: r.kind,
                    key: r.key,
                    raw: r.raw,
                },
            ),
            table::<tables::ScientistDaily, _>(outcomes[19], &self.scientists.rows, |r| {
                rows::ScientistDailyRow {
                    character: r.character,
                    country: r.country,
                    injury_days: rows::Recorded::Value(r.injury_days),
                    template_visibility: rows::Recorded::Value(r.template_visibility),
                    template_origin: rows::Recorded::Value(r.template_origin.and_then(
                        |(start, len, line)| {
                            Some(rows::ScientistTemplateOrigin {
                                file: self
                                    .scientist_names
                                    .get(start..start.checked_add(len)?)?
                                    .to_vec(),
                                line,
                            })
                        },
                    )),
                }
            }),
        ]
    }

    fn raid_equipment_table(&self, outcome: Outcome) -> TableView {
        let len = self.raid_equipment_variants.len;
        let archetype_len = self.raid_equipment_archetype_tokens.len;
        let (Some(variants), Some(amounts), Some(tokens), Some(totals)) = (
            self.raid_equipment_variants.first.get(..len),
            self.raid_equipment_amounts.first.get(..len),
            self.raid_equipment_archetype_tokens
                .first
                .get(..archetype_len),
            self.raid_equipment_archetype_amounts
                .first
                .get(..archetype_len),
        ) else {
            return unavailable("raid equipment arena length invalid");
        };
        let range = |start: usize, count: usize, len: usize| {
            start
                .checked_add(count)
                .filter(|&end| end <= len)
                .map(|end| start..end)
        };
        let items = |r: &raid_equipment::Row| range(r.items_start, r.items_len, len);
        let archetypes =
            |r: &raid_equipment::Row| range(r.archetypes_start, r.archetypes_len, archetype_len);
        let rows = &self.raid_equipment.rows[..outcome.len().min(self.raid_equipment.rows.len())];
        if rows.iter().any(|r| items(r).is_none()) {
            return unavailable("raid equipment entries outside captured arena");
        }
        if rows.iter().any(|r| archetypes(r).is_none()) {
            return unavailable("raid equipment archetype totals outside captured arena");
        }
        table::<tables::RaidEquipment, _>(outcome, &self.raid_equipment.rows, |r| {
            let entries = items(r).unwrap_or_default();
            let archetypes = archetypes(r).unwrap_or_default();
            rows::RaidEquipmentRow {
                reference_type: r.reference_type,
                id: r.id,
                phase: r.phase,
                end_hour: r.end_hour,
                variants: owned_vec(copy_vec(&variants[entries.clone()])),
                amounts: owned_vec(copy_vec(&amounts[entries])),
                nukes: r.nukes,
                archetype_tokens: owned_vec(copy_vec(&tokens[archetypes.clone()])),
                archetype_amounts: owned_vec(copy_vec(&totals[archetypes])),
            }
        })
    }

    fn raid_speed_table(&self, outcome: Outcome) -> TableView {
        let speeds = &self.raid_transports.first[..self
            .raid_transports
            .len
            .min(self.raid_transports.first.len())];
        let list = |r: &raid_speed_inputs::Row| {
            r.transport_start
                .checked_add(r.transport_len)
                .and_then(|end| speeds.get(r.transport_start..end))
        };
        let rows = &self.raid_speeds.rows[..outcome.len().min(self.raid_speeds.rows.len())];
        if rows.iter().any(|r| list(r).is_none()) {
            return unavailable("raid transport speeds outside captured arena");
        }
        table::<tables::RaidSpeedInputs, _>(outcome, &self.raid_speeds.rows, |r| {
            rows::RaidSpeedInputRow {
                reference_type: r.reference_type,
                id: r.id,
                phase: r.phase,
                outcome: r.outcome,
                end_hour: r.end_hour,
                wing_max_speed: r.wing_max_speed,
                army_max_speed: r.army_max_speed,
                transport_speeds: owned_vec(copy_vec(list(r).unwrap_or_default())),
                transport_from_country: r.transport_from_country,
                transport_fallback: r.transport_fallback,
                ace_present: r.ace_present,
                ace_alive: r.ace_alive,
                ace_speed: r.ace_speed,
                ace_type_speed: r.ace_type_speed,
                ace_wing_planes: r.ace_wing_planes,
                ace_owner: r.ace_owner,
                wing_country: r.wing_country,
            }
        })
    }

    fn raid_player_table(&self, outcome: Outcome) -> TableView {
        let tokens = &self.raid_type_tokens.first[..self.raid_type_tokens.len];
        let rows = &self.raid_player.rows[..outcome.len().min(self.raid_player.rows.len())];
        let list = |r: &cplayer_status_cache::Row| {
            r.allowed_start
                .checked_add(r.allowed_len)
                .and_then(|end| tokens.get(r.allowed_start..end))
        };
        if rows.iter().any(|r| list(r).is_none()) {
            return unavailable("player raid cache tokens outside captured arena");
        }
        table::<tables::RaidPlayerAllowedTypes, _>(outcome, &self.raid_player.rows, |r| {
            rows::RaidPlayerAllowedTypeRow {
                country: r.country,
                allowed: owned_vec(copy_vec(list(r).unwrap_or_default())),
            }
        })
    }

    fn raid_countries_table(&self, outcome: Outcome) -> TableView {
        let Some(tokens) = self.raid_type_tokens.first.get(..self.raid_type_tokens.len) else {
            return unavailable("raid type token arena length invalid");
        };
        let list = |start: usize, len: usize| {
            start
                .checked_add(len)
                .and_then(|end| tokens.get(start..end))
        };
        let rows = &self.raid_countries.rows[..outcome.len().min(self.raid_countries.rows.len())];
        if rows.iter().any(|r| {
            list(r.eligible_start, r.eligible_len).is_none()
                || list(r.used_start, r.used_len).is_none()
        }) {
            return unavailable("raid type tokens outside captured arena");
        }
        table::<tables::RaidCountries, _>(outcome, &self.raid_countries.rows, |r| {
            rows::RaidCountryRow {
                canonical_slot: r.canonical_slot,
                country: r.country,
                eligible_types: owned_vec(copy_vec(
                    list(r.eligible_start, r.eligible_len).unwrap(),
                )),
                used_types: owned_vec(copy_vec(list(r.used_start, r.used_len).unwrap())),
            }
        })
    }

    fn raids_table(&self, outcome: Outcome) -> TableView {
        let Some(handles) = self.raid_show_for.first.get(..self.raid_show_for.len) else {
            return unavailable("raid show-for arena length invalid");
        };
        let list = |r: &craid_instance::Row| {
            r.show_for_start
                .checked_add(r.show_for_len)
                .and_then(|end| handles.get(r.show_for_start..end))
        };
        let rows = &self.raids.rows[..outcome.len().min(self.raids.rows.len())];
        if rows.iter().any(|r| list(r).is_none()) {
            return unavailable("raid show-for handles outside captured arena");
        }
        table::<tables::Raids, _>(outcome, &self.raids.rows, |r| rows::RaidRow {
            reference_type: r.reference_type,
            id: r.id,
            parent_slot: r.parent_slot,
            ordinal: r.ordinal,
            phase: r.phase,
            outcome: r.outcome,
            source: rows::RaidSource {
                kind: r.source.kind,
                kind_present: r.source.kind_present,
                province: r.source.province,
                ship_reference_type: r.source.ship_reference_type,
                ship_reference_id: r.source.ship_reference_id,
                country: r.source.country,
                building_type: r.source.building_type,
                building_location: r.source.building_location,
                building_country: r.source.building_country,
            },
            show_for: owned_vec(copy_vec(list(r).unwrap())),
            unit: r.unit,
            air_wing: r.air_wing,
            travelled: Some(r.travelled),
            preparation_hours: r.preparation_hours,
        })
    }

    fn program_table(&self, outcome: Outcome) -> TableView {
        PROGRAM_FACILITY_REFUSALS.fetch_add(
            self.programs.rows[..outcome.len().min(self.programs.rows.len())]
                .iter()
                .filter(|row| !row.details_known)
                .count() as u64,
            Relaxed,
        );
        PROGRAM_ASSIGNMENT_REFUSALS.fetch_add(
            self.programs.rows[..outcome.len().min(self.programs.rows.len())]
                .iter()
                .filter(|row| !row.assignments_known)
                .count() as u64,
            Relaxed,
        );
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
                building: owned_vec(copy_vec(
                    &self.names.first[r.building_start..r.building_start + r.building_len],
                )),
                facility: rows::Recorded::Value(r.details_known.then(|| {
                    self.names.first[r.details_start
                        ..r.details_start
                            + r.details_count as usize * engine_projects::program_details::BYTES]
                        .chunks_exact(engine_projects::program_details::BYTES)
                        .map(|bytes| {
                            let value = engine_projects::program_details::Operand::decode(bytes);
                            rows::ProjectFacilityOperand {
                                kind: value.kind,
                                scientist: value.scientist,
                                token: value.token,
                                key: value.key,
                                ordinal: value.ordinal,
                                assignment: value.assignment,
                                raw: value.raw,
                            }
                        })
                        .collect()
                })),
                assignments: rows::Recorded::Value(r.assignments_known.then(|| {
                    let start = r.assignments_start;
                    let end =
                        start + r.assignments_count as usize * nproject_cprogram::ASSIGNMENT_BYTES;
                    self.names.first[start..end]
                        .chunks_exact(nproject_cprogram::ASSIGNMENT_BYTES)
                        .map(|bytes| rows::ProjectScientistAssignment {
                            scientist: u64::from_le_bytes(bytes[..8].try_into().unwrap()),
                            target: i32::from_le_bytes(bytes[8..12].try_into().unwrap()),
                            current: i32::from_le_bytes(bytes[12..16].try_into().unwrap()),
                            timed: bytes[16],
                        })
                        .collect()
                })),
            }
        })
    }
}

fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}

impl Captured {
    #[expect(
        clippy::too_many_arguments,
        reason = "the hourly capture operands (memory, base, game state, hour, full flag) travel beside the store, change source, deferral and dispatch"
    )]
    fn capture_project_family(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        store: &mut SliceFingerprints,
        changes: &dyn crate::country_input_changes::Changes,
        defer_runtime: bool,
        dispatch: Option<ProjectDispatch<'_>>,
    ) {
        use crate::country_input_changes::{self as family, Table};
        let _reads = read_accounting::table(TABLES[1]);
        let family_started = self.profile_acquisition.then(qpc);
        self.project_selection.reset_daily(hour, force_full);
        crate::slicing::select_changed(
            &mut self.project_selection,
            store,
            &mut self.project_changed,
            |visit| changes.take(Table::Projects, visit),
            0,
        );
        self.project_read.clear();
        let fail = |captured: &mut Self, reason| {
            captured.projects.outcome = Outcome::Unavailable { reason };
            captured.history.outcome = Outcome::Unavailable { reason };
        };
        let owners = self.select_project_owners(memory, base, game_state, store);
        if let Some(started) = family_started {
            self.acquisition_phases[0] = elapsed(started);
        }
        match owners {
            Err(reason) => fail(self, reason),
            Ok((census, found_country)) => {
                if census {
                    store.finish_scan();
                    self.project_selection.finish_census(store);
                }
                let selection = &self.project_selection;
                let native_started = self.profile_acquisition.then(qpc);
                let mut runtime_acquired = false;
                let cold = if force_full && defer_runtime {
                    cold_parts::capture(
                        memory,
                        base,
                        game_state,
                        &self.project_read,
                        self.project_runtime.is_empty(),
                        dispatch,
                    )
                } else {
                    None
                };
                let (projects, histories) = if let Some(cold) = cold {
                    runtime_acquired = cold.runtime_acquired();
                    let started = self.profile_acquisition.then(qpc);
                    let outcomes = cold.join(
                        &mut self.projects.rows,
                        &mut self.history.rows,
                        &mut self.project_runtime,
                    );
                    if let Some(started) = started {
                        self.acquisition_phases[4] = elapsed(started);
                    }
                    outcomes
                } else if census && found_country && self.project_read.is_empty() {
                    self.projects.rows.clear();
                    self.history.rows.clear();
                    (Outcome::Rows { len: 0 }, Outcome::Rows { len: 0 })
                } else {
                    nproject_cproject::walk_with_history_selected(
                        memory,
                        base,
                        game_state,
                        engine_native_memory::walk::Buffer::Growing(&mut self.projects.rows),
                        engine_native_memory::walk::Buffer::Growing(&mut self.history.rows),
                        &mut |id| selection.contains(id),
                    )
                };
                if let Some(started) = native_started {
                    self.acquisition_phases[1] = elapsed(started);
                }
                self.projects.outcome = bounded(projects, self.projects.rows.len());
                self.history.outcome = bounded(histories, self.history.rows.len());
                let read = if defer_runtime {
                    runtime::Runtime::acquire
                } else {
                    runtime::Runtime::read
                };
                let runtime_started = self.profile_acquisition.then(qpc);
                if !runtime_acquired {
                    read(
                        &mut self.project_runtime,
                        memory,
                        base,
                        &self.projects.rows[..self.projects.outcome.len()],
                        force_full,
                        census
                            && matches!(self.projects.outcome, Outcome::Rows { .. })
                            && (force_full
                                || hour.rem_euclid(replay_data::cadence::STAGGER_PERIOD) == 0),
                        Some(&self.project_selection),
                    );
                } else if census && matches!(self.projects.outcome, Outcome::Rows { .. }) {
                    self.project_runtime.finish_census(
                        &self.projects.rows[..self.projects.outcome.len()],
                        Some(&self.project_selection),
                    );
                }
                if let Some(started) = runtime_started {
                    self.acquisition_phases[2] = elapsed(started);
                }
            }
        }
        family::record(
            Table::Projects,
            &family::Counts {
                marked: self.project_changed.marked() as u64,
                read: self.project_countries().count() as u64,
                full: !self.project_changed.certified,
            },
            None,
        );
        if let Some(started) = family_started {
            self.acquisition_phases[3] = elapsed(started);
        }
    }

    fn select_project_owners(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        store: &mut SliceFingerprints,
    ) -> Result<(bool, bool), &'static str> {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let links = countries.arrays()[1];
        let bytes = (links.count as usize)
            .checked_mul(4)
            .ok_or("country link table size overflow")?;
        grow_vec(&mut self.project_links, bytes);
        if self.project_links.len() < bytes {
            return Err("country link table allocation failed");
        }
        if !engine_native_memory::read_slice(memory, links.data, &mut self.project_links[..bytes]) {
            return Err("country link table unreadable");
        }
        self.project_owners.clear();
        engine_allocation::reserve(&mut self.project_owners, links.count as usize)
            .map_err(|_| "project owner allocation failed")?;
        for (raw, word) in self.project_links[..bytes]
            .chunks_exact(4)
            .enumerate()
            .skip(1)
        {
            let slot = u32::from_le_bytes(word.try_into().unwrap());
            if slot != 0 {
                self.project_owners.push((slot, raw as u32));
            }
        }
        self.project_owners.sort_unstable();
        let mut census = true;
        let mut found_country = false;
        for slot in 1..countries.count() {
            let country = match countries.at(slot) {
                Ok(Some(country)) => {
                    found_country = true;
                    country
                }
                Ok(None) => continue,
                Err(_) => {
                    census = false;
                    continue;
                }
            };
            let start = self
                .project_owners
                .partition_point(|&(owner, _)| owner < slot);
            let end = self
                .project_owners
                .partition_point(|&(owner, _)| owner <= slot);
            let raws = &self.project_owners[start..end];
            let drift = std::cell::OnceCell::new();
            let drifts = || {
                *drift.get_or_init(|| crate::country_input_changes::drifts(memory, country.pointer))
            };
            let selection = &mut self.project_selection;
            selection.scan_lazily(store, country.id, drifts);
            for &(_, raw) in raws {
                selection.scan_lazily(store, raw, drifts);
            }
            if selection.contains(country.id)
                || raws.iter().any(|&(_, raw)| selection.contains(raw))
            {
                selection.select(country.id);
                engine_allocation::reserve(&mut self.project_read, raws.len() + 1)
                    .map_err(|_| "project owner allocation failed")?;
                self.project_read.push((country.id, country.id));
                for &(_, raw) in raws {
                    selection.select(raw);
                    if raw != country.id {
                        self.project_read.push((country.id, raw));
                    }
                }
            }
        }
        Ok((census, found_country))
    }

    fn project_countries(&self) -> impl Iterator<Item = &[(u32, u32)]> {
        self.project_read
            .chunk_by(|first, second| first.0 == second.0)
    }
}

mod cold_parts;
mod owned;
mod runtime;
pub use cold_parts::{ProjectDispatch, ProjectPart};
#[cfg(test)]
use owned::{CensusEntry, M_PROGRAMS, ProjectCensus, copy_program_row, span};
pub use owned::{ProjectsCopyContext, ProjectsCopyReport, ProjectsJob, copy_projects_refresh};

fn capture_rows<R>(captured: &mut CaptureRows<R>, walk: impl FnOnce(&mut Vec<R>) -> Outcome) {
    captured.outcome = bounded(walk(&mut captured.rows), captured.rows.len());
}

fn capture_pair<P, C>(
    parents: &mut CaptureRows<P>,
    instances: &mut CaptureRows<C>,
    walk: impl FnOnce(&mut Vec<P>, &mut Vec<C>) -> (Outcome, Outcome),
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
        || (matches!(outcome, Outcome::Rows { .. })
            && result.required_bytes != result.bytes_len
            && rows[..outcome.len()]
                .iter()
                .all(|row| row.assignments_known && row.details_known))
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
                    && (!r.assignments_known
                        || (r.assignments_count as usize)
                            .checked_mul(nproject_cprogram::ASSIGNMENT_BYTES)
                            .and_then(|count| r.assignments_start.checked_add(count))
                            .is_some_and(|end| end <= len))
                    && (!r.details_known
                        || (r.details_count as usize)
                            .checked_mul(engine_projects::program_details::BYTES)
                            .and_then(|count| r.details_start.checked_add(count))
                            .is_some_and(|end| end <= len))
            })
        })
}

#[cfg(any(test, feature = "test-support"))]
pub mod projects_test_kit;

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "projects_scale_tests.rs"]
mod scale_tests;

#[cfg(any(test, feature = "test-support"))]
pub fn scientist_spool_fixture(daily: bool, ordinal: u32) -> [TableView; N] {
    let mut captured = Captured::with_capacities(&Capacities {
        rows: [1; N],
        ..Default::default()
    });
    captured.scientists.rows[0] = scientist_roster::Row {
        character: 7,
        country: 5000,
        ordinal,
        active: Some(1),
        injury_days: daily.then_some(8),
        template_visibility: daily.then_some(1),
        template_origin: daily.then_some((0, 22, 17)),
        ..Default::default()
    };
    captured
        .scientist_names
        .extend_from_slice(b"events/replacement.txt");
    captured.scientists.outcome = Outcome::Rows { len: 1 };
    captured.scientist_skills.outcome = Outcome::Rows { len: 0 };
    captured.scientist_injury_read = daily;
    captured.tables()
}

pub struct RestPrimary {
    game_state: usize,
    hour: i32,
    force_full: bool,
    ticks: [u64; N],
    facilities: CaptureRows<raid_facilities::Row>,
    tooltip_operands: CaptureRows<raid_tooltip_operands::Row>,
    nukes: CaptureRows<cnuke::Row>,
    raid_countries: CaptureRows<ccountry_raid_status::Row>,
    raids: CaptureRows<craid_instance::Row>,
    peace: CaptureRows<peace_conference::Row>,
    raid_cooldowns: CaptureRows<craid_system::CooldownRow>,
    raid_player: CaptureRows<cplayer_status_cache::Row>,
    raid_type_tokens: Arena<u32>,
    raid_show_for: Arena<u32>,
    raid_type_memo: ccountry_raid_status::TypeTokens,
    refused_leaders: u32,
}

pub struct RestAux {
    game_state: usize,
    hour: i32,
    force_full: bool,
    ticks: [u64; N],
    raid_intel: CaptureRows<engine_raid_inputs::raid_intel_inputs::Row>,
    agency_defense: Vec<Option<super::agency_defense::Tree>>,
    counteroperatives: Arena<engine_raid_inputs::raid_intel_inputs::Operative>,
    raid_speeds: CaptureRows<raid_speed_inputs::Row>,
    raid_transports: Arena<i64>,
    raid_equipment: CaptureRows<raid_equipment::Row>,
    raid_equipment_variants: Arena<u64>,
    raid_equipment_amounts: Arena<i64>,
    raid_equipment_archetype_tokens: Arena<u32>,
    raid_equipment_archetype_amounts: Arena<i64>,
    speed_memo: engine_native_parts::modifier_definition::DefinitionMemo,
}

pub struct RestPaths {
    game_state: usize,
    hour: i32,
    force_full: bool,
    ticks: [u64; N],
    scientists: CaptureRows<scientist_roster::Row>,
    scientist_skills: CaptureRows<scientist_roster::SkillRow>,
    scientist_names: Vec<u8>,
    raid_paths: [CaptureRows<raid_path_memo::Row>; 2],
    scientist_injury_read: bool,
    torn_memo_reads: u32,
}

pub struct Rest {
    primary: RestPrimary,
    aux: RestAux,
    paths: RestPaths,
}

impl Rest {
    pub fn into_triple(self) -> (RestPrimary, RestAux, RestPaths) {
        (self.primary, self.aux, self.paths)
    }

    pub fn capture(&mut self, memory: &impl Memory, base: usize) {
        self.primary.capture(memory, base);
        self.aux.capture(memory, base);
        self.paths.capture(memory, base);
    }
}

impl Captured {
    pub fn take_rest(&mut self, game_state: usize, hour: i32, force_full: bool) -> Rest {
        self.rest_lent = true;
        let primary = RestPrimary {
            game_state,
            hour,
            force_full,
            ticks: [0; N],
            facilities: std::mem::replace(&mut self.facilities, CaptureRows::new(0)),
            tooltip_operands: std::mem::replace(&mut self.tooltip_operands, CaptureRows::new(0)),
            nukes: std::mem::replace(&mut self.nukes, CaptureRows::new(0)),
            raid_countries: std::mem::replace(&mut self.raid_countries, CaptureRows::new(0)),
            raids: std::mem::replace(&mut self.raids, CaptureRows::new(0)),
            peace: std::mem::replace(&mut self.peace, CaptureRows::new(0)),
            raid_cooldowns: std::mem::replace(&mut self.raid_cooldowns, CaptureRows::new(0)),
            raid_player: std::mem::replace(&mut self.raid_player, CaptureRows::new(0)),
            raid_type_tokens: std::mem::replace(&mut self.raid_type_tokens, Arena::new(0)),
            raid_show_for: std::mem::replace(&mut self.raid_show_for, Arena::new(0)),
            raid_type_memo: std::mem::replace(
                &mut self.raid_type_memo,
                ccountry_raid_status::TypeTokens::empty(),
            ),
            refused_leaders: self.raid_reads.refused_leaders,
        };
        let aux = RestAux {
            game_state,
            hour,
            force_full,
            ticks: [0; N],
            raid_intel: std::mem::replace(&mut self.raid_intel, CaptureRows::new(0)),
            agency_defense: std::mem::take(&mut self.agency_defense),
            counteroperatives: std::mem::replace(&mut self.counteroperatives, Arena::new(0)),
            raid_speeds: std::mem::replace(&mut self.raid_speeds, CaptureRows::new(0)),
            raid_transports: std::mem::replace(&mut self.raid_transports, Arena::new(0)),
            raid_equipment: std::mem::replace(&mut self.raid_equipment, CaptureRows::new(0)),
            raid_equipment_variants: std::mem::replace(
                &mut self.raid_equipment_variants,
                Arena::new(0),
            ),
            raid_equipment_amounts: std::mem::replace(
                &mut self.raid_equipment_amounts,
                Arena::new(0),
            ),
            raid_equipment_archetype_tokens: std::mem::replace(
                &mut self.raid_equipment_archetype_tokens,
                Arena::new(0),
            ),
            raid_equipment_archetype_amounts: std::mem::replace(
                &mut self.raid_equipment_archetype_amounts,
                Arena::new(0),
            ),
            speed_memo: std::mem::take(&mut self.speed_memo),
        };
        let paths = RestPaths {
            game_state,
            hour,
            force_full,
            ticks: [0; N],
            scientists: std::mem::replace(&mut self.scientists, CaptureRows::new(0)),
            scientist_skills: std::mem::replace(&mut self.scientist_skills, CaptureRows::new(0)),
            scientist_names: std::mem::take(&mut self.scientist_names),
            raid_paths: std::mem::replace(
                &mut self.raid_paths,
                [CaptureRows::new(0), CaptureRows::new(0)],
            ),
            scientist_injury_read: std::mem::take(&mut self.scientist_injury_read),
            torn_memo_reads: self.raid_reads.torn_memo_reads,
        };
        Rest {
            primary,
            aux,
            paths,
        }
    }

    pub fn restore_rest(&mut self, rest: Rest) -> [u64; N] {
        self.restore_rest_triple(Some(rest.primary), Some(rest.aux), Some(rest.paths), "")
    }

    pub fn restore_rest_triple(
        &mut self,
        primary: Option<RestPrimary>,
        aux: Option<RestAux>,
        paths: Option<RestPaths>,
        reason: &'static str,
    ) -> [u64; N] {
        self.rest_lent = false;
        let mut ticks = [0; N];
        match primary {
            Some(primary) => {
                self.facilities = primary.facilities;
                self.tooltip_operands = primary.tooltip_operands;
                self.nukes = primary.nukes;
                self.raid_countries = primary.raid_countries;
                self.raids = primary.raids;
                self.peace = primary.peace;
                self.raid_cooldowns = primary.raid_cooldowns;
                self.raid_player = primary.raid_player;
                self.raid_type_tokens = primary.raid_type_tokens;
                self.raid_show_for = primary.raid_show_for;
                self.raid_type_memo = primary.raid_type_memo;
                self.raid_reads.refused_leaders = primary.refused_leaders;
                for (dst, src) in ticks.iter_mut().zip(primary.ticks) {
                    *dst += src;
                }
            }
            None => self.fail_rest_primary(reason),
        }
        match aux {
            Some(aux) => {
                self.raid_intel = aux.raid_intel;
                self.agency_defense = aux.agency_defense;
                self.counteroperatives = aux.counteroperatives;
                self.raid_speeds = aux.raid_speeds;
                self.raid_transports = aux.raid_transports;
                self.raid_equipment = aux.raid_equipment;
                self.raid_equipment_variants = aux.raid_equipment_variants;
                self.raid_equipment_amounts = aux.raid_equipment_amounts;
                self.raid_equipment_archetype_tokens = aux.raid_equipment_archetype_tokens;
                self.raid_equipment_archetype_amounts = aux.raid_equipment_archetype_amounts;
                self.speed_memo = aux.speed_memo;
                for (dst, src) in ticks.iter_mut().zip(aux.ticks) {
                    *dst += src;
                }
            }
            None => self.fail_rest_aux(reason),
        }
        match paths {
            Some(paths) => {
                self.scientists = paths.scientists;
                self.scientist_skills = paths.scientist_skills;
                self.scientist_names = paths.scientist_names;
                self.raid_paths = paths.raid_paths;
                self.scientist_injury_read = paths.scientist_injury_read;
                self.raid_reads.torn_memo_reads = paths.torn_memo_reads;
                for (dst, src) in ticks.iter_mut().zip(paths.ticks) {
                    *dst += src;
                }
            }
            None => self.fail_rest_paths(reason),
        }
        ticks
    }
}

impl RestPrimary {
    pub fn capture(&mut self, memory: &impl Memory, base: usize) {
        let (game_state, hour, force_full) = (self.game_state, self.hour, self.force_full);
        let active = TABLES.map(|table| replay_data::cadence::should_walk(table, hour, force_full));
        let mut ticks = [0; N];
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
                        reason: replay_data::cadence::SKIPPED_REASON,
                    };
                }
            }};
        }
        plain!(nukes, 4, cnuke);
        if active[5] || active[10] || active[11] {
            let begin = qpc();
            let _reads = read_accounting::table(TABLES[5]);
            let walk = craid_system::walk(
                memory,
                base,
                game_state,
                craid_system::Buffers {
                    parents: &mut self.raid_countries.rows,
                    type_tokens: &mut self.raid_type_tokens.first,
                    instances: &mut self.raids.rows,
                    show_for: &mut self.raid_show_for.first,
                    cooldowns: &mut self.raid_cooldowns.rows,
                    type_memo: &mut self.raid_type_memo,
                },
            );
            self.raid_cooldowns.outcome = bounded(walk.cooldowns, self.raid_cooldowns.rows.len());
            self.raid_countries.outcome = bounded(walk.parents, self.raid_countries.rows.len());
            self.raids.outcome = bounded(walk.instances, self.raids.rows.len());
            self.raid_type_tokens.len = walk.type_tokens_len;
            self.raid_type_tokens.required = walk.required_type_tokens;
            self.raid_show_for.len = walk.show_for_len;
            self.raid_show_for.required = walk.required_show_for;
            self.refused_leaders += walk.refused_leaders;
            ticks[5] = elapsed(begin);
        } else {
            self.raid_countries.outcome = Outcome::Unavailable {
                reason: replay_data::cadence::SKIPPED_REASON,
            };
            self.raids.outcome = self.raid_countries.outcome;
            self.raid_cooldowns.outcome = self.raid_countries.outcome;
        }
        if active[13] {
            let _reads = read_accounting::table(TABLES[13]);
            let begin = qpc();
            let start = self
                .raid_type_tokens
                .len
                .min(self.raid_type_tokens.first.len());
            let walk = cplayer_status_cache::walk_buffer(
                memory,
                base,
                game_state,
                engine_native_memory::walk::Buffer::Growing(&mut self.raid_player.rows),
                engine_native_memory::walk::Buffer::GrowingFrom(&mut self.raid_type_tokens.first, start),
                &mut self.raid_type_memo,
            );
            if let Some(row) = self.raid_player.rows.first_mut() {
                row.allowed_start += start;
            }
            self.raid_type_tokens.len = start + walk.tokens_len;
            self.raid_type_tokens.required = self
                .raid_type_tokens
                .required
                .max(start)
                .saturating_add(walk.required_tokens);
            self.raid_player.outcome = bounded(walk.outcome, self.raid_player.rows.len());
            ticks[13] = elapsed(begin);
        } else {
            self.raid_player.outcome = Outcome::Unavailable {
                reason: replay_data::cadence::SKIPPED_REASON,
            };
        }
        plain!(peace, 7, peace_conference);
        plain!(facilities, 16, raid_facilities);
        plain!(tooltip_operands, 18, raid_tooltip_operands);
        self.ticks = ticks;
    }
}

impl RestAux {
    pub fn capture(&mut self, memory: &impl Memory, base: usize) {
        let (game_state, hour, force_full) = (self.game_state, self.hour, self.force_full);
        let active = TABLES.map(|table| replay_data::cadence::should_walk(table, hour, force_full));
        let mut ticks = [0; N];
        if active[14] {
            let _reads = read_accounting::table(TABLES[14]);
            let begin = qpc();
            let walk = raid_speed_inputs::walk(
                memory,
                base,
                game_state,
                &self.speed_memo,
                &mut self.raid_speeds.rows,
                &mut self.raid_transports.first,
                &collector_observers::variant_stat_witness::ObserverStatWitness,
            );
            self.raid_transports.len = walk.transport_len;
            self.raid_transports.required = walk.required_transport;
            self.raid_speeds.outcome = bounded(walk.outcome, self.raid_speeds.rows.len());
            ticks[14] = elapsed(begin);
        } else {
            self.raid_speeds.outcome = Outcome::Unavailable {
                reason: replay_data::cadence::SKIPPED_REASON,
            };
        }
        if active[15] {
            let _reads = read_accounting::table(TABLES[15]);
            let begin = qpc();
            let walk = raid_equipment::walk(
                memory,
                base,
                game_state,
                &mut self.raid_equipment.rows,
                raid_equipment::Arenas {
                    variants: &mut self.raid_equipment_variants.first,
                    amounts: &mut self.raid_equipment_amounts.first,
                    archetype_tokens: &mut self.raid_equipment_archetype_tokens.first,
                    archetype_amounts: &mut self.raid_equipment_archetype_amounts.first,
                },
            );
            for arena in [
                &mut self.raid_equipment_variants.len,
                &mut self.raid_equipment_amounts.len,
            ] {
                *arena = walk.items_len;
            }
            for arena in [
                &mut self.raid_equipment_archetype_tokens.len,
                &mut self.raid_equipment_archetype_amounts.len,
            ] {
                *arena = walk.archetypes_len;
            }
            self.raid_equipment_variants.required = walk.required_items;
            self.raid_equipment_amounts.required = walk.required_items;
            self.raid_equipment_archetype_tokens.required = walk.required_archetypes;
            self.raid_equipment_archetype_amounts.required = walk.required_archetypes;
            self.raid_equipment.outcome = bounded(walk.outcome, self.raid_equipment.rows.len());
            ticks[15] = elapsed(begin);
        } else {
            self.raid_equipment.outcome = Outcome::Unavailable {
                reason: replay_data::cadence::SKIPPED_REASON,
            };
        }
        if !active[17] {
            self.raid_intel.outcome = Outcome::Unavailable {
                reason: replay_data::cadence::SKIPPED_REASON,
            };
        } else {
            let begin = qpc();
            let _reads = read_accounting::table(TABLES[17]);
            let mut carry = super::raid_intel_carry::begin();
            let (rows, operatives) = (&mut self.raid_intel.rows, &mut self.counteroperatives.first);
            let read = match carry.as_mut() {
                Some(carry) => engine_raid_inputs::raid_intel_inputs::walk(
                    memory, base, game_state, rows, operatives, carry,
                ),
                None => engine_raid_inputs::raid_intel_inputs::walk(
                    memory,
                    base,
                    game_state,
                    rows,
                    operatives,
                    &mut engine_raid_inputs::raid_intel_inputs::NoCarry,
                ),
            };
            if let Some(carry) = carry {
                carry.finish(!matches!(read.outcome, Outcome::Unavailable { .. }));
            }
            self.raid_intel.outcome = read.outcome;
            if let Outcome::Rows { len } | Outcome::Truncated { len, .. } = read.outcome {
                super::agency_defense::snapshot(memory, base, game_state, &self.raid_intel.rows[..len], &mut self.agency_defense);
            }
            self.counteroperatives.required = read.operatives;
            self.counteroperatives.len = read.operatives.min(self.counteroperatives.first.len());
            ticks[17] = elapsed(begin);
        }
        self.ticks = ticks;
    }
}

impl RestPaths {
    pub fn capture(&mut self, memory: &impl Memory, base: usize) {
        let (game_state, hour, force_full) = (self.game_state, self.hour, self.force_full);
        let active = TABLES.map(|table| replay_data::cadence::should_walk(table, hour, force_full));
        let mut ticks = [0; N];
        if active[12] {
            let _reads = read_accounting::table(TABLES[12]);
            let begin = qpc();
            for (memo, dispatcher) in self.raid_paths.iter_mut().zip([
                raid_path_memo::Dispatcher::Land,
                raid_path_memo::Dispatcher::Naval,
            ]) {
                capture_rows(memo, |rows| {
                    raid_path_memo::walk(
                        memory,
                        base,
                        game_state,
                        dispatcher,
                        rows,
                        &mut self.torn_memo_reads,
                    )
                });
            }
            ticks[12] = elapsed(begin);
        } else {
            for memo in &mut self.raid_paths {
                memo.outcome = Outcome::Unavailable {
                    reason: replay_data::cadence::SKIPPED_REASON,
                };
            }
        }
        self.scientist_injury_read = false;
        if active[8] {
            let begin = qpc();
            let names = &mut self.scientist_names;
            capture_pair(
                &mut self.scientists,
                &mut self.scientist_skills,
                |scientists, skills| {
                    let _reads = read_accounting::table(TABLES[8]);
                    let walk = scientist_roster::walk_with_injury(
                        memory,
                        base,
                        game_state,
                        &mut scientist_roster::Buffers {
                            scientists,
                            skills,
                            names,
                        },
                        active[19],
                    );
                    (walk.scientists, walk.skills)
                },
            );
            SCIENTIST_ACTIVE_REFUSALS.fetch_add(
                self.scientists.rows[..self.scientists.outcome.len()]
                    .iter()
                    .filter(|row| row.active.is_none())
                    .count() as u64,
                Relaxed,
            );
            self.scientist_injury_read = active[19];
            if active[19] {
                SCIENTIST_VISIBILITY_REFUSALS.fetch_add(
                    self.scientists.rows[..self.scientists.outcome.len()]
                        .iter()
                        .filter(|row| {
                            row.template_visibility.is_none()
                                || row.template_visibility == Some(1)
                                    && row.template_origin.is_none()
                        })
                        .count() as u64,
                    Relaxed,
                );
                SCIENTIST_INJURY_REFUSALS.fetch_add(
                    self.scientists.rows[..self.scientists.outcome.len()]
                        .iter()
                        .filter(|row| row.injury_days.is_none())
                        .count() as u64,
                    Relaxed,
                );
            }
            ticks[8] = elapsed(begin);
        } else {
            self.scientists.outcome = Outcome::Unavailable {
                reason: replay_data::cadence::SKIPPED_REASON,
            };
            self.scientist_skills.outcome = self.scientists.outcome;
        }
        self.ticks = ticks;
    }
}

impl Captured {
    fn fail_rest_primary(&mut self, reason: &'static str) {
        let failed = Outcome::Unavailable { reason };
        self.facilities.outcome = failed;
        self.tooltip_operands.outcome = failed;
        self.nukes.outcome = failed;
        self.raid_countries.outcome = failed;
        self.raids.outcome = failed;
        self.peace.outcome = failed;
        self.raid_cooldowns.outcome = failed;
        self.raid_player.outcome = failed;
    }

    fn fail_rest_aux(&mut self, reason: &'static str) {
        let failed = Outcome::Unavailable { reason };
        self.raid_intel.outcome = failed;
        self.raid_speeds.outcome = failed;
        self.raid_equipment.outcome = failed;
    }

    fn fail_rest_paths(&mut self, reason: &'static str) {
        self.scientist_injury_read = false;
        let failed = Outcome::Unavailable { reason };
        self.scientists.outcome = failed;
        self.scientist_skills.outcome = failed;
        for memo in &mut self.raid_paths {
            memo.outcome = failed;
        }
    }

    pub fn fail_rest(&mut self, reason: &'static str) {
        self.rest_lent = false;
        self.fail_rest_primary(reason);
        self.fail_rest_aux(reason);
        self.fail_rest_paths(reason);
    }
}
