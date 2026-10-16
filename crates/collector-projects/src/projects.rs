//! Strategic state acquisition. Replay conversion and buffer growth run off-thread.

use crate::{
    clock::qpc,
    raw::RawLog,
    read_accounting,
    slicing::{SliceFingerprints, SliceSelection},
    table_capture::{Arena, CaptureRows, bounded, grow_capacity, grow_vec},
    table_conversion::{table, unavailable},
};
use engine::allocation::{copy_vec, default_vec, map_slice, owned_vec};
use engine::raid_tooltip_operands;
use engine::{
    Memory, ccountry, ccountry_raid_status, cgame_state, cnuke, cpdx_array::Array,
    cplayer_status_cache, craid_instance, craid_system, msvc_string, nproject_breakthrough_node,
    nproject_cbreakthrough_progress, nproject_cprogram, nproject_cprogram_status,
    nproject_cproject, nproject_sproject_history, peace_conference, raid_equipment,
    raid_facilities, raid_path_memo, raid_speed_inputs, scientist_roster, walk::Outcome,
};
use replay::{
    frame::TableView,
    rows::{self, TableId, tables},
};
use std::mem::size_of;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

static SCIENTIST_ACTIVE_REFUSALS: AtomicU64 = AtomicU64::new(0);

pub fn scientist_active_data() -> impl serde::Serialize {
    let refused = SCIENTIST_ACTIVE_REFUSALS.load(Relaxed);
    crate::status::status!({
        "refused": refused,
        "refusals": crate::status::map(std::iter::once(("scientist active byte unreadable or address overflow", refused)).filter(|(_, count)| *count != 0)),
        "scope": "cumulative appended scientist activity read failures; membership and skills remain available",
    })
}

pub const N: usize = 19;
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
];

/// Indices of the `projects` sliced family's tables in `TABLES`.
pub const FAMILY: [usize; 2] = [1, 2];

/// The daily tables the raw-copy job acquires, in calibration order.
pub const RAW: [TableId; 2] = [TableId::ProjectPrograms, TableId::ProjectBreakthroughs];

/// Each rooted walk produces two independently bounded tables.
/// Its entire cost is attributed once to the anchor named by each group.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[
    ("project pool and reward histories", 1, &[1, 2]),
    // RaidTargets is the raid instance walk's second projection: the same rows
    // carry the map icon's operands, so it joins that walk's cost group.
    ("raid countries and instances", 5, &[5, 6, 10, 11]),
    ("scientist rosters and specializations", 8, &[8, 9]),
];

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    pub rows: [usize; N],
    pub program_names: usize,
    pub program_candidates: usize,
    /// Eligible and used raid type tokens, and raid `show_for` handles.
    pub raid_type_tokens: usize,
    pub raid_show_for: usize,
    /// RaidSpeedInputs' transport speeds, every live raid's.
    pub raid_transports: usize,
    /// RaidEquipment's pool entries, every recorded raid's.
    pub raid_equipment_items: usize,
    /// RaidEquipment's per-archetype pool totals, every recorded raid's.
    pub raid_equipment_archetypes: usize,
    pub counteroperatives: usize,
}

impl Capacities {
    /// The live pool extent and each project's reward-history array set demand.
    /// Empty outputs count every authenticated pool/header without reading
    /// lifecycle children.
    pub fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let (projects, histories) =
            nproject_cproject::walk_with_history(memory, base, state, &mut [], &mut []);
        let demand = |outcome| match outcome {
            Outcome::Rows { len } => Ok(len),
            Outcome::Truncated { len, unread_rows } => len
                .checked_add(unread_rows as usize)
                .ok_or("project row demand overflow"),
            Outcome::Unavailable { reason } => Err(reason),
        };
        // The rooted program pointer vectors bound their own demand.
        let programs = nproject_cprogram::walk(memory, base, state, &mut [], &mut [], &mut []);
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

    /// RaidCountries has one row per canonical slot and Nukes exactly two
    /// per country (`cnuke::TYPE_COUNT`). Project pools use live demand.
    pub fn raise_for_countries(&mut self, slots: usize) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.rows[4], rows(slots, 2));
        raise(&mut self.rows[5], rows(slots, 1));
        raise(&mut self.rows[17], rows(slots, 1));
    }

    /// The live type-token demand of every raid parent and the player cache
    /// ([`crate::country_scale::observe_raid_arenas`]), so a full hour reads
    /// every parent however many types each lists.
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

pub struct Captured {
    programs: CaptureRows<nproject_cprogram::Row>,
    facilities: CaptureRows<raid_facilities::Row>,
    raid_intel: CaptureRows<engine::raid_intel_inputs::Row>,
    tooltip_operands: CaptureRows<engine::raid_tooltip_operands::Row>,
    counteroperatives: Arena<engine::raid_intel_inputs::Operative>,
    projects: CaptureRows<nproject_cproject::Row>,
    history: CaptureRows<nproject_sproject_history::Row>,
    breakthroughs: CaptureRows<nproject_cbreakthrough_progress::Row>,
    nukes: CaptureRows<cnuke::Row>,
    raid_countries: CaptureRows<ccountry_raid_status::Row>,
    raids: CaptureRows<craid_instance::Row>,
    peace: CaptureRows<peace_conference::Row>,
    scientists: CaptureRows<scientist_roster::Row>,
    scientist_skills: CaptureRows<scientist_roster::SkillRow>,
    raid_cooldowns: CaptureRows<craid_system::CooldownRow>,
    raid_paths: CaptureRows<raid_path_memo::Row>,
    /// The player raid cache; its tokens follow the parents' in the arena.
    raid_player: CaptureRows<cplayer_status_cache::Row>,
    raid_speeds: CaptureRows<raid_speed_inputs::Row>,
    raid_transports: Arena<i64>,
    raid_equipment: CaptureRows<raid_equipment::Row>,
    raid_equipment_variants: Arena<u64>,
    raid_equipment_amounts: Arena<i64>,
    raid_equipment_archetype_tokens: Arena<u32>,
    raid_equipment_archetype_amounts: Arena<i64>,
    /// Modifier metadata for the speed inputs' getters, read once per walk.
    speed_memo: engine::modifier_definition::DefinitionMemo,
    /// This hour's torn memo copies and refused raid rows.
    raid_reads: super::RaidReadCounts,
    raid_type_tokens: Arena<u32>,
    raid_show_for: Arena<u32>,
    /// Raid type tokens by definition, sized from the raid database.
    raid_type_memo: ccountry_raid_status::TypeTokens,
    names: Arena<u8>,
    candidates: Vec<usize>,
    required_candidates: usize,
    /// Raw-copy split gate for the daily ProjectPrograms and
    /// ProjectBreakthroughs. Set when an acquisition job returns a valid
    /// refresh payload; `capture` then skips their typed walks and the decode
    /// worker fills these rows before the ready handoff. Preserved across
    /// `reset` (set per refresh, cleared by the worker after decode). Hourly
    /// and sliced tables are never gated by this flag.
    pub raw_mode: bool,
    /// Which owners the Projects and ProjectHistory rows cover: the
    /// `projects` sliced family ([`Self::capture_project_family`]).
    project_selection: SliceSelection,
    /// This hour's project marks and comparisons.
    project_changed: crate::slicing::Changed,
    /// The country link table's words, read once per hour.
    project_links: Vec<u8>,
    /// (canonical slot, raw id) for every linked raw id, by slot.
    project_owners: Vec<(u32, u32)>,
    /// (native country id, owner raw id) of every country this hour read,
    /// grouped by country: the country's own id first, then its other raw
    /// ids ascending.
    project_read: Vec<(u32, u32)>,
}

impl Captured {
    /// Fail the typed job's tables without erasing its separately staged projects.
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
        self.raid_cooldowns.fail(outcomes[11]);
        self.raid_paths.fail(outcomes[12]);
        self.raid_player.fail(outcomes[13]);
        self.raid_speeds.fail(outcomes[14]);
        self.raid_equipment.fail(outcomes[15]);
        self.facilities.fail(outcomes[16]);
        self.raid_intel.fail(outcomes[17]);
        self.tooltip_operands.fail(outcomes[18]);
    }

    /// Allocation-free placeholder while a worker owns the capture buffers.
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

    /// All capacity, outcome and cost arrays use local TABLES order.
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            programs: CaptureRows::new(capacity.rows[0]),
            facilities: CaptureRows::new(capacity.rows[16]),
            raid_intel: CaptureRows::new(capacity.rows[17]),
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
            raid_cooldowns: CaptureRows::new(capacity.rows[11]),
            raid_paths: CaptureRows::new(capacity.rows[12]),
            raid_player: CaptureRows::new(capacity.rows[13]),
            raid_speeds: CaptureRows::new(capacity.rows[14]),
            raid_transports: Arena::new(capacity.raid_transports),
            raid_equipment: CaptureRows::new(capacity.rows[15]),
            raid_equipment_variants: Arena::new(capacity.raid_equipment_items),
            raid_equipment_amounts: Arena::new(capacity.raid_equipment_items),
            raid_equipment_archetype_tokens: Arena::new(capacity.raid_equipment_archetypes),
            raid_equipment_archetype_amounts: Arena::new(capacity.raid_equipment_archetypes),
            speed_memo: engine::modifier_definition::DefinitionMemo::default(),
            raid_reads: super::RaidReadCounts::default(),
            raid_type_tokens: Arena::new(capacity.raid_type_tokens),
            raid_show_for: Arena::new(capacity.raid_show_for),
            raid_type_memo: ccountry_raid_status::TypeTokens::empty(),
            names: Arena::new(capacity.program_names),
            candidates: owned_vec(default_vec(capacity.program_candidates)),
            required_candidates: 0,
            raw_mode: false,
            project_selection: SliceSelection::new(capacity.rows[1] != 0),
            project_changed: crate::slicing::Changed::default(),
            project_links: Vec::new(),
            project_owners: Vec::new(),
            project_read: Vec::new(),
        }
    }

    /// The owners whose project rows this frame's read replaced.
    pub fn project_selection(&self) -> &SliceSelection {
        &self.project_selection
    }

    /// This hour's torn memo copies and refused raid rows.
    pub fn raid_reads(&self) -> super::RaidReadCounts {
        self.raid_reads
    }

    pub fn reset(&mut self) {
        self.programs.reset();
        self.facilities.reset();
        self.raid_intel.reset();
        self.tooltip_operands.reset();
        self.counteroperatives.reset();
        self.projects.reset();
        self.history.reset();
        self.breakthroughs.reset();
        self.nukes.reset();
        self.raid_countries.reset();
        self.raids.reset();
        self.peace.reset();
        self.scientists.reset();
        self.scientist_skills.reset();
        self.raid_cooldowns.reset();
        self.raid_paths.reset();
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
        self.facilities.allocated_bytes()
            + self.raid_intel.allocated_bytes()
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
            + self.raid_cooldowns.allocated_bytes()
            + self.raid_paths.allocated_bytes()
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
        self.raid_paths.grow(&mut capacity.rows[12]);
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

    /// A capture whose project store is new: every country is first seen.
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

    /// Worker acquisition grows from original headers while the callback waits.
    /// Definition/reward/specialization tokens retain their native u32 bits.
    /// ProjectPrograms and ProjectBreakthroughs follow their daily slot;
    /// Projects and ProjectHistory read the owners `projects` and
    /// `changes` select ([`Self::capture_project_family`]).
    #[allow(clippy::too_many_arguments)] // The capture context, the family's store and its marks.
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
        self.reset();
        let mut ticks = [0; N];
        let active = TABLES.map(|table| replay::cadence::should_walk(table, hour, force_full));
        let walk = active[0] && !self.raw_mode;
        if self.raw_mode && active[0] {
            // The owned refresh supplies these rows before publication.
        } else if !walk {
            let skipped = Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON,
            };
            self.programs.outcome = skipped;
            self.breakthroughs.outcome = skipped;
        } else {
            let begin = qpc();
            self.capture_programs(|candidates, rows, names| {
                let _reads = read_accounting::table(TABLES[0]);
                nproject_cprogram::walk_growing(memory, base, game_state, candidates, rows, names)
            });
            ticks[0] = elapsed(begin);
            let begin = qpc();
            capture_rows(&mut self.breakthroughs, |rows| {
                let _reads = read_accounting::table(TABLES[3]);
                nproject_cbreakthrough_progress::walk_growing(memory, base, game_state, rows)
            });
            ticks[3] = elapsed(begin);
        }
        let begin = qpc();
        self.capture_project_family(
            roster, base, game_state, hour, force_full, projects, changes,
        );
        ticks[1] = elapsed(begin);
        macro_rules! plain {
            ($field:ident, $index:literal, $module:ident) => {{
                if active[$index] {
                    let _reads = read_accounting::table(TABLES[$index]);
                    let begin = qpc();
                    capture_rows(&mut self.$field, |rows| {
                        $module::walk_growing(memory, base, game_state, rows)
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
        if active[5] || active[10] || active[11] {
            let begin = qpc();
            let _reads = read_accounting::table(TABLES[5]);
            let walk = craid_system::walk_growing(
                memory,
                base,
                game_state,
                craid_system::GrowingBuffers {
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
            self.raid_reads.refused_leaders += walk.refused_leaders;
            ticks[5] = elapsed(begin);
        } else {
            self.raid_countries.outcome = Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON,
            };
            self.raids.outcome = self.raid_countries.outcome;
            self.raid_cooldowns.outcome = self.raid_countries.outcome;
        }
        if active[13] {
            let _reads = read_accounting::table(TABLES[13]);
            let begin = qpc();
            // The cache's tokens follow the parents' in the shared arena.
            let start = self
                .raid_type_tokens
                .len
                .min(self.raid_type_tokens.first.len());
            let walk = cplayer_status_cache::walk_buffer(
                memory,
                base,
                game_state,
                engine::walk::Buffer::Growing(&mut self.raid_player.rows),
                engine::walk::Buffer::GrowingFrom(&mut self.raid_type_tokens.first, start),
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
                reason: replay::cadence::SKIPPED_REASON,
            };
        }
        if active[14] {
            let _reads = read_accounting::table(TABLES[14]);
            let begin = qpc();
            let walk = raid_speed_inputs::walk_growing(
                memory,
                base,
                game_state,
                &self.speed_memo,
                &mut self.raid_speeds.rows,
                &mut self.raid_transports.first,
            );
            self.raid_transports.len = walk.transport_len;
            self.raid_transports.required = walk.required_transport;
            self.raid_speeds.outcome = bounded(walk.outcome, self.raid_speeds.rows.len());
            ticks[14] = elapsed(begin);
        } else {
            self.raid_speeds.outcome = Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON,
            };
        }
        if active[15] {
            let _reads = read_accounting::table(TABLES[15]);
            let begin = qpc();
            let walk = raid_equipment::walk_growing(
                memory,
                base,
                game_state,
                &mut self.raid_equipment.rows,
                raid_equipment::GrowingArenas {
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
                reason: replay::cadence::SKIPPED_REASON,
            };
        }
        plain!(peace, 7, peace_conference);
        plain!(facilities, 16, raid_facilities);
        plain!(tooltip_operands, 18, raid_tooltip_operands);
        if !active[17] {
            self.raid_intel.outcome = Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON,
            };
        } else {
            let begin = qpc();
            let _reads = read_accounting::table(TABLES[17]);
            let mut carry = super::raid_intel_carry::begin();
            let (rows, operatives) = (&mut self.raid_intel.rows, &mut self.counteroperatives.first);
            let read = match carry.as_mut() {
                Some(carry) => engine::raid_intel_inputs::walk_growing(
                    memory, base, game_state, rows, operatives, carry,
                ),
                None => engine::raid_intel_inputs::walk_growing(
                    memory,
                    base,
                    game_state,
                    rows,
                    operatives,
                    &mut engine::raid_intel_inputs::NoCarry,
                ),
            };
            if let Some(carry) = carry {
                carry.finish(!matches!(read.outcome, Outcome::Unavailable { .. }));
            }
            self.raid_intel.outcome = read.outcome;
            self.counteroperatives.required = read.operatives;
            self.counteroperatives.len = read.operatives.min(self.counteroperatives.first.len());
            ticks[17] = elapsed(begin);
        }
        if active[12] {
            let _reads = read_accounting::table(TABLES[12]);
            let begin = qpc();
            let torn = &mut self.raid_reads.torn_memo_reads;
            capture_rows(&mut self.raid_paths, |rows| {
                raid_path_memo::walk_growing(memory, base, game_state, rows, torn)
            });
            ticks[12] = elapsed(begin);
        } else {
            self.raid_paths.outcome = Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON,
            };
        }
        if active[8] {
            let begin = qpc();
            capture_pair(
                &mut self.scientists,
                &mut self.scientist_skills,
                |scientists, skills| {
                    let _reads = read_accounting::table(TABLES[8]);
                    let walk = scientist_roster::walk_growing(
                        memory,
                        base,
                        game_state,
                        &mut scientist_roster::GrowingBuffers { scientists, skills },
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
            self.raid_paths.outcome,
            self.raid_player.outcome,
            self.raid_speeds.outcome,
            self.raid_equipment.outcome,
            self.facilities.outcome,
            self.raid_intel.outcome,
            self.tooltip_operands.outcome,
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
            // The raid instance rows carry the map icon's own operands.
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
            table::<tables::RaidPathDistances, _>(outcomes[12], &self.raid_paths.rows, |r| {
                rows::RaidPathDistanceRow {
                    source: r.source,
                    target: r.target,
                    distance: r.distance,
                    ok: r.ok,
                }
            }),
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
            table::<tables::RaidIntelInputs, _>(outcomes[17], &self.raid_intel.rows, |r| {
                rows::RaidIntelInputRow {
                    country: r.country,
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
            }
        })
    }
}

fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}

impl Captured {
    /// Projects and ProjectHistory, the `projects` sliced family: every
    /// country at the daily boundary, whose pool pass advances every project,
    /// and on a full
    /// hour; in between the new and unpublished countries and those a hooked
    /// project writer marked ([`crate::country_input_changes`]). A row's
    /// owner is its project's stored owner tag (`CProject+0x1C`), any raw id
    /// linking to the pool's country, so a read country selects, and the
    /// census keeps alive, every raw id its link table entries name. A read
    /// of an unmarked owner compares a digest of its projects' identity and
    /// lifecycle (definition, program reference, started and completed) with
    /// its last read; the state machine and the reward history, which the
    /// daily pass advances, are left out.
    #[allow(clippy::too_many_arguments)]
    fn capture_project_family(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        store: &mut SliceFingerprints,
        changes: &dyn crate::country_input_changes::Changes,
    ) {
        use crate::country_input_changes::{self as family, Table};
        let _reads = read_accounting::table(TABLES[1]);
        self.project_selection.reset_daily(hour, force_full);
        crate::slicing::select_changed(
            &mut self.project_selection,
            store,
            &mut self.project_changed,
            |visit| changes.take(Table::Projects, visit),
            0,
            changes.every(Table::Projects),
        );
        self.project_read.clear();
        let fail = |captured: &mut Self, reason| {
            captured.projects.outcome = Outcome::Unavailable { reason };
            captured.history.outcome = Outcome::Unavailable { reason };
        };
        match self.select_project_owners(memory, base, game_state, store) {
            Err(reason) => fail(self, reason),
            Ok(census) => {
                if census {
                    store.finish_scan();
                    self.project_selection.finish_census(store);
                }
                let selection = &self.project_selection;
                let (projects, histories) = nproject_cproject::walk_with_history_selected(
                    memory,
                    base,
                    game_state,
                    engine::walk::Buffer::Growing(&mut self.projects.rows),
                    engine::walk::Buffer::Growing(&mut self.history.rows),
                    &mut |id| selection.contains(id),
                );
                self.projects.outcome = bounded(projects, self.projects.rows.len());
                self.history.outcome = bounded(histories, self.history.rows.len());
                self.compare_project_owners(store, changes);
            }
        }
        family::record(
            Table::Projects,
            &family::Counts {
                marked: self.project_changed.marked() as u64,
                read: self.project_countries().count() as u64,
                verified: self.project_changed.verified,
                mismatches: self.project_changed.mismatches,
                full: !self.project_changed.certified,
            },
            None,
        );
    }

    /// The hour's census: every raw id linking to each live country is an
    /// owner the family keeps; a country is read when its native id or any of
    /// its raw ids is selected, and then all of them are. Ok(false) when a
    /// census slot could not be read (no owner is known gone).
    fn select_project_owners(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        store: &mut SliceFingerprints,
    ) -> Result<bool, &'static str> {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let links = countries.arrays()[1];
        let bytes = (links.count as usize)
            .checked_mul(4)
            .ok_or("country link table size overflow")?;
        grow_vec(&mut self.project_links, bytes);
        if self.project_links.len() < bytes {
            return Err("country link table allocation failed");
        }
        if !engine::read_slice(memory, links.data, &mut self.project_links[..bytes]) {
            return Err("country link table unreadable");
        }
        self.project_owners.clear();
        engine::allocation::reserve(&mut self.project_owners, links.count as usize)
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
        for slot in 1..countries.count() {
            let country = match countries.at(slot) {
                Ok(Some(country)) => country,
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
            // Only a country that owns a state runs the daily pool pass
            // ([`crate::country_input_changes::drifts`]); its print is
            // whether it does, checked at the daily boundary, so a change
            // since the last boundary is read there.
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
                engine::allocation::reserve(&mut self.project_read, raws.len() + 1)
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
        Ok(census)
    }

    /// Each read country's owner raw ids, its own id first.
    fn project_countries(&self) -> impl Iterator<Item = &[(u32, u32)]> {
        self.project_read
            .chunk_by(|first, second| first.0 == second.0)
    }

    /// Holds each read country's digest, over the rows of all its owner raw
    /// ids, and compares an unmarked one with its last read: a writer marks
    /// the country, whichever raw id its projects name. An incomplete read
    /// holds nothing.
    fn compare_project_owners(
        &mut self,
        store: &mut SliceFingerprints,
        changes: &dyn crate::country_input_changes::Changes,
    ) {
        let (Outcome::Rows { len }, Outcome::Rows { .. }) =
            (self.projects.outcome, self.history.outcome)
        else {
            for country in self
                .project_read
                .chunk_by(|first, second| first.0 == second.0)
            {
                self.project_changed.unread(store, country[0].0);
            }
            return;
        };
        // The walk leaves its rows in key order (owner, local id).
        let rows = &self.projects.rows[..len];
        for country in self
            .project_read
            .chunk_by(|first, second| first.0 == second.0)
        {
            let mut digest = crate::slicing::Digest::new();
            for &(_, owner) in country {
                let start = rows.partition_point(|row| row.owner < owner);
                let end = rows.partition_point(|row| row.owner <= owner);
                digest.u64(u64::from(owner)).u64((end - start) as u64);
                for row in &rows[start..end] {
                    digest
                        .u64(u64::from(row.local_id))
                        .u64(u64::from(row.definition_id))
                        .u64(row.program_reference)
                        .u64(u64::from(row.started))
                        .u64(u64::from(row.completed));
                }
            }
            if self
                .project_changed
                .read(store, country[0].0, digest.finish())
            {
                changes.mismatch(
                    crate::country_input_changes::Table::Projects,
                    crate::country_input_changes::Mismatch {
                        country: country[0].0,
                        field: "Projects (identity and lifecycle)",
                    },
                );
            }
        }
    }
}

mod owned;
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
#[cfg(any(test, feature = "test-support"))]
pub mod projects_test_kit;

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "projects_scale_tests.rs"]
mod scale_tests;
