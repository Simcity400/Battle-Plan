use crate::{
    clock::qpc,
    table_capture::{Arena, CaptureRows, grow_capacity, grow_vec},
    table_conversion::{table, table_iter, unavailable},
};
use collector_observers::naval_tree_changes;
use engine_allocation as allocation;
use engine_native_land::land_tactic_inputs;
use engine_native::{Memory, carmy_combat_inputs, cland_combat, walk::Outcome};
use engine_native_land::{
    combat_sources::{Images, Sources},
    land_reinforcement_inputs::speed as reserve_speed,
};
use replay_data::{
    rows::CombatModifierValue, rows::LandCombatCommanderTimedTrait, rows::LandCombatCommanderTrait,
    rows::LandCombatDetailsRow, rows::LandCombatDivisionModifierRow, rows::LandCombatDivisionRow,
    rows::LandCombatExileInputs, rows::LandCombatMemberValues, rows::NameOperands, rows::LandCombatLearningProgress, rows::LandCombatModifierRow,
    rows::LandCombatSideDetails, rows::LandCombatSideInputs, rows::LandCombatSideRow,
    rows::LandDivisionContext, rows::LandDivisionModifier, rows::LandDivisionWidthTerm,
    rows::Recorded, rows::ReserveModifierTreeRow, rows::Retired, rows::RetiredKept,
    rows::RetiredNested, rows::TableId, rows::tables,
};
use replay_data::{rows::LandCombatTacticCountryWeight, rows::LandCombatTacticInputs};
use replay_frame::frame::TableView;
use std::mem::size_of;
use std::sync::{
    OnceLock,
    atomic::{AtomicU64, Ordering},
};

static RESERVE_SHARES: AtomicU64 = AtomicU64::new(0);
static REFUSED_SHARES: AtomicU64 = AtomicU64::new(0);
static FIRST_SHARE_REFUSAL: OnceLock<&'static str> = OnceLock::new();
static REFUSED_SIDE_INPUTS: AtomicU64 = AtomicU64::new(0);
static REFUSED_TREES: AtomicU64 = AtomicU64::new(0);
static FIRST_TREE_REFUSAL: OnceLock<&'static str> = OnceLock::new();
static RESERVE_TREE_COUNTS: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
static FIRST_SIDE_INPUT_REFUSAL: OnceLock<&'static str> = OnceLock::new();

pub const N: usize = 6;
pub const TABLES: [TableId; N] = [
    TableId::LandCombatDetails,
    TableId::LandCombatDivisions,
    TableId::LandCombatModifiers,
    TableId::LandCombatSides,
    TableId::LandCombatDivisionModifiers,
    TableId::ReserveModifierTrees,
];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    combats: usize,
    census_participants: usize,
    details: usize,
    divisions: usize,
    sides: usize,
    combat_sides: usize,
    country_tags: usize,
    rosters: usize,
    modifier_members: usize,
    modifiers: usize,
    width_terms: usize,
    exile_tags: usize,
    division_names: usize,
    name_templates: usize,
    name_groups: usize,
    commander_traits: usize,
    detail_text: usize,
    timed_traits: usize,
    air_counts: usize,
    learning_traits: usize,
}

collector_primitives::capacity_max!(Capacities {
    combats,
    census_participants,
    details,
    divisions,
    sides,
    combat_sides,
    country_tags,
    rosters,
    modifier_members,
    modifiers,
    width_terms,
    exile_tags,
    division_names,
    name_templates,
    name_groups,
    commander_traits,
    detail_text,
    timed_traits,
    air_counts,
    learning_traits,
});
impl Capacities {
    pub fn raise_for_live_details(&mut self, demand: engine_native_land::land_combat_details::DetailDemand) {
        for (capacity, required) in [
            (&mut self.combats, demand.rows),
            (&mut self.details, demand.rows),
            (&mut self.combat_sides, demand.rows),
            (&mut self.sides, demand.rows.saturating_mul(2)),
            (&mut self.census_participants, demand.census_participants),
            (&mut self.divisions, demand.census_participants),
            (&mut self.modifier_members, demand.census_participants),
            (&mut self.rosters, demand.arenas.participants),
            (&mut self.air_counts, demand.arenas.air),
            (&mut self.detail_text, demand.arenas.text),
            (&mut self.commander_traits, demand.arenas.traits),
            (&mut self.timed_traits, demand.arenas.timed),
            (&mut self.learning_traits, demand.arenas.learning),
        ] {
            grow_capacity(capacity, required);
        }
    }

    pub fn raise_for_countries(&mut self, slots: usize) {
        crate::country_scale::raise(&mut self.exile_tags, crate::country_scale::rows(slots, 2));
        crate::country_scale::raise(&mut self.name_templates, slots.saturating_mul(4));
        crate::country_scale::raise(&mut self.name_groups, slots.saturating_mul(2));
    }
}
#[derive(Default, Clone, PartialEq, Eq)]
struct Detail {
    row: engine_native_land::land_combat_details::DetailsRow,
    tactic_context: land_tactic_inputs::ContextRow,
}
#[derive(Default, Clone, Copy, PartialEq, Eq)]
struct Division {
    pointer: usize,
    combat: u64,
    side: u8,
    ordinal: u32,
    province: u32,
    role: u8,
    stats: engine_native_land::division_combat_inputs::Row,
    exile: Option<engine_native_land::division_combat_inputs::ExileRow>,
    context: engine_native_land::division_combat_inputs::ContextRow,
    width_start: usize,
    reserve_speed: Option<Result<i64, &'static str>>,
    reserve_observation: Option<usize>,
    joined: bool,
}
#[derive(Default, Clone, Copy, PartialEq, Eq)]
struct ModifierSide {
    combat: u64,
    side: u8,
    border_values: Option<[i64; 3]>,
    start: usize,
    len: usize,
}
#[derive(Default, Clone, Copy, PartialEq, Eq)]
struct CombatSides {
    sets: cland_combat::CountrySetsRow,
}
#[derive(Default, Clone, Copy, PartialEq, Eq)]
struct ModifierMember {
    division: u64,
    ordinal: u32,
    start: usize,
    len: usize,
}
#[derive(Default, Clone, Copy)]
struct Progress {
    len: usize,
    required: usize,
    failure: Option<&'static str>,
}
impl Progress {
    fn outcome(self, parent: Outcome) -> Outcome {
        match (self.failure, parent) {
            (Some(reason), _) | (_, Outcome::Unavailable { reason }) => {
                Outcome::Unavailable { reason }
            }
            (_, Outcome::Truncated { .. }) => Outcome::Unavailable {
                reason: "land combat detail source census truncated",
            },
            _ if self.required > self.len => Outcome::Truncated {
                len: self.len,
                unread_rows: self
                    .required
                    .saturating_sub(self.len)
                    .min(u32::MAX as usize) as u32,
            },
            _ => Outcome::Rows { len: self.len },
        }
    }
}
#[derive(Clone, Copy)]
struct Placement {
    pointer: usize,
    combat: u64,
    side: u8,
    ordinal: u32,
    province: u32,
    role: u8,
    exile_start: usize,
}
struct Acquired {
    stats: engine_native_land::division_combat_inputs::Row,
    exile: engine_native_land::division_combat_inputs::Exile,
    context: engine_native_land::division_combat_inputs::ContextRow,
    reserve_speed: Option<Result<ReserveSpeed, &'static str>>,
}

enum ReserveSpeed {
    Value(i64),
    Observed(usize),
}

#[derive(Default)]
struct Scratch {
    widths: Vec<engine_native_land::division_combat_inputs::WidthTerm>,
    modifiers: Vec<engine_native_land::division_combat_inputs::Modifier>,
    reinforcement: reserve_speed::Scratch,
    observations: reserve_speed::Observations,
    defer_speed: bool,
}
impl Scratch {
    fn allocated_bytes(&self) -> usize {
        self.widths.capacity() * size_of::<engine_native_land::division_combat_inputs::WidthTerm>()
            + self.modifiers.capacity() * size_of::<engine_native_land::division_combat_inputs::Modifier>()
            + self.reinforcement.allocated_bytes()
            + self.observations.allocated_bytes()
    }
}

type DivisionRead = (
    engine_native_land::division_combat_inputs::Row,
    engine_native_land::division_combat_inputs::Exile,
    engine_native_land::division_combat_inputs::ContextRow,
    Option<Result<ReserveSpeed, &'static str>>,
);

struct DivisionOutputs<'r, 'n> {
    name_groups: &'r mut engine_native::cname_group_member::GroupKeys,
    names: &'r mut engine_native::cname_group_member::Names<'n>,
    exile_tags: engine_native::walk::Buffer<'r, u32>,
}

#[allow(clippy::too_many_arguments)]
fn read_division<M: Memory, C: Memory>(
    memory: &M,
    base: usize,
    state: usize,
    countries: &Result<engine_native::ccountry::Reader<'_, C>, &'static str>,
    sources: &mut Sources,
    scratch: &mut Scratch,
    outputs: DivisionOutputs<'_, '_>,
    role: u8,
    pointer: usize,
    handle: u64,
) -> Result<DivisionRead, &'static str> {
    let stats = engine_native_land::division_combat_inputs::capture_stats(
        memory,
        base,
        sources,
        pointer,
        handle,
        outputs.name_groups,
        outputs.names,
    )?;
    let countries = countries.as_ref().map_err(|reason| *reason)?;
    let exile = match role {
        1 => engine_native_land::division_combat_inputs::capture_exile_buffer(
            memory,
            countries,
            pointer,
            outputs.exile_tags,
        )?,
        _ => engine_native_land::division_combat_inputs::Exile::Absent,
    };
    let context = engine_native_land::division_combat_inputs::capture_context_buffer(
        memory,
        base,
        countries,
        sources,
        pointer,
        handle,
        engine_native::walk::Buffer::Growing(&mut scratch.widths),
    )?;
    let reserve_speed = (role == 2).then(|| {
        if scratch.defer_speed {
            scratch
                .observations
                .acquire(
                    memory,
                    base,
                    state,
                    sources,
                    pointer,
                    handle,
                    context.base_speed,
                )
                .map(ReserveSpeed::Observed)
        } else {
            reserve_speed::capture_share(
                memory,
                base,
                state,
                sources,
                pointer,
                handle,
                context.base_speed,
                &mut scratch.reinforcement,
            )
            .map(ReserveSpeed::Value)
        }
    });
    Ok((stats, exile, context, reserve_speed))
}

struct Destination<'a> {
    sources: &'a mut Sources,
    commanders: engine_native_land::land_combat_details::Commanders,
    leaders: &'a mut engine_native_land::land_combat_details::Leaders,
    images: &'a mut Images,
    scratch: &'a mut Scratch,
    name_groups: &'a mut engine_native::cname_group_member::GroupKeys,
    names: engine_native::cname_group_member::Names<'a>,
    #[cfg(test)]
    reference: bool,
    details: &'a mut Vec<Detail>,
    divisions: &'a mut Vec<Division>,
    sides: &'a mut Vec<ModifierSide>,
    combat_sides: &'a mut Vec<CombatSides>,
    country_tags: &'a mut Vec<u32>,
    country_tags_progress: Progress,
    country_set_limit: Result<usize, &'static str>,
    rosters: &'a mut Vec<u64>,
    commander_traits: &'a mut Vec<engine_native_land::land_combat_details::CommanderTrait>,
    detail_text: &'a mut Vec<u8>,
    timed_traits: &'a mut Vec<engine_native_land::land_combat_details::CommanderTimedTrait>,
    air_counts: &'a mut Vec<i32>,
    learning_traits: &'a mut Vec<engine_native_land::land_combat_details::CommanderLearningTrait>,
    unit_scratch: &'a mut Vec<usize>,
    detail_used: engine_native_land::land_combat_details::DetailCounts,
    detail_required: engine_native_land::land_combat_details::DetailCounts,
    members: &'a mut Vec<ModifierMember>,
    modifiers: &'a mut Vec<engine_native_land::division_combat_inputs::Modifier>,
    width_terms: &'a mut Vec<engine_native_land::division_combat_inputs::WidthTerm>,
    width_progress: Progress,
    exile_tags: &'a mut Vec<u32>,
    exile_progress: Progress,
    progress: [Progress; N],
    arenas: [Progress; N],
    ticks: [u64; N],
}
impl Destination<'_> {
    fn admit(&mut self, at: Placement, acquired: Acquired) {
        let Acquired {
            stats,
            exile,
            context,
            reserve_speed,
        } = acquired;
        let (exile, exile_len) = match exile {
            engine_native_land::division_combat_inputs::Exile::Absent => (Some(None), 0),
            engine_native_land::division_combat_inputs::Exile::Row(mut row) => {
                row.cores.0 += at.exile_start;
                row.occupiers.0 += at.exile_start;
                (Some(Some(row)), row.cores.1 + row.occupiers.1)
            }
            engine_native_land::division_combat_inputs::Exile::DoesNotFit { required } => (None, required),
        };
        self.exile_progress.required += exile_len;
        self.width_progress.required += context.width_term_count;
        let start = self.width_progress.len;
        let count = context.width_term_count;
        let (Some(naming), Some(exile)) = (stats.naming, exile) else {
            return;
        };
        if !grow_vec(self.width_terms, start + count) {
            return;
        }
        let scratch = &*self.scratch;
        self.width_terms[start..start + count].copy_from_slice(&scratch.widths[..count]);
        self.width_progress.len += count;
        self.exile_progress.len += exile_len;
        debug_assert!(naming.custom.1 == 0 || naming.custom.0 == self.names.len);
        self.names.len += naming.custom.1;
        let (reserve_speed, reserve_observation) = match reserve_speed {
            Some(Ok(ReserveSpeed::Observed(index))) => (None, Some(index)),
            Some(Ok(ReserveSpeed::Value(value))) => (Some(Ok(value)), None),
            Some(Err(reason)) => (Some(Err(reason)), None),
            None => (None, None),
        };
        self.divisions[self.progress[1].len] = Division {
            pointer: at.pointer,
            combat: at.combat,
            side: at.side,
            ordinal: at.ordinal,
            province: at.province,
            role: at.role,
            stats,
            exile,
            context,
            width_start: start,
            reserve_speed,
            reserve_observation,
            joined: false,
        };
        self.progress[1].len += 1;
    }
    fn visit(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        source: cland_combat::Source,
        members: cland_combat::Members<'_>,
    ) {
        if crate::allocation::failed() {
            return;
        }
        self.commanders = engine_native_land::land_combat_details::Commanders::default();
        let bound =
            (source.attackers.count as usize).saturating_add(source.defenders.count as usize);
        if !grow_vec(self.divisions, self.progress[1].required + bound) {
            return;
        }
        if !grow_vec(self.sides, self.progress[2].required.saturating_add(2)) {
            return;
        }
        if !grow_vec(
            self.unit_scratch,
            (source.attackers.count.max(source.defenders.count)) as usize,
        ) {
            return;
        }
        let countries = engine_native::ccountry::Reader::new(memory, base, state);
        {
            let started = qpc();
            self.detail(memory, base, state, source, &members);
            self.ticks[0] += elapsed(started);
        }
        let mut side_indices = [None; 2];
        {
            let _reads = crate::read_accounting::table(TABLES[2]);
            let started = qpc();
            self.progress[2].required += 2;
            match cland_combat::read_modifiers_context(memory, base, source) {
                Ok(context) => {
                    for (side, index) in side_indices.iter_mut().enumerate() {
                        if self.progress[2].len < self.sides.len() {
                            let next = self.progress[2].len;
                            self.sides[next] = ModifierSide {
                                combat: source.id,
                                side: side as u8,
                                border_values: context.border_values[side],
                                start: self.arenas[1].len,
                                len: 0,
                            };
                            *index = Some(next);
                            self.progress[2].len += 1;
                        }
                    }
                }
                Err(reason) => {
                    self.progress[2].failure = Some(reason);
                }
            }
            self.ticks[2] += elapsed(started);
        }
        let mut images = std::mem::take(self.images);
        if self.progress[1].failure.is_none() || self.progress[2].failure.is_none() {
            let _reads = crate::read_accounting::table(TABLES[1]);
            let started = qpc();
            if images
                .capture(memory, members.pointers)
                .map_err(crate::allocation::io)
                .is_err()
            {
                *self.images = images;
                return;
            }
            self.ticks[1] += elapsed(started);
        } else {
            images.clear();
        }
        {
            let _reads = crate::read_accounting::table(TABLES[3]);
            let started = qpc();
            self.combat_sides(memory, base, source);
            self.ticks[3] += elapsed(started);
        }
        {
            let started = qpc();
            let before = self.ticks[1] + self.ticks[2];
            let mut owned = std::mem::take(self.unit_scratch);
            let result = cland_combat::visit_units(
                memory,
                base,
                source,
                &members,
                &mut owned,
                |side, ordinal, pointer, handle, province, role| {
                    if crate::allocation::failed() {
                        return;
                    }
                    let row = ordinal as usize
                        + if side == 0 {
                            0
                        } else {
                            source.attackers.count as usize
                        };
                    let divisional = &images.division(memory, row, pointer);
                    {
                        let _reads = crate::read_accounting::table(TABLES[1]);
                        let started = qpc();
                        self.progress[1].required += 1;
                        if self.progress[1].failure.is_none() {
                            let exile_start = self.exile_progress.len;
                            let result = read_division(
                                divisional,
                                base,
                                state,
                                &countries,
                                self.sources,
                                self.scratch,
                                DivisionOutputs {
                                    name_groups: self.name_groups,
                                    names: &mut self.names,
                                    exile_tags: engine_native::walk::Buffer::GrowingFrom(
                                        self.exile_tags,
                                        exile_start,
                                    ),
                                },
                                role,
                                pointer,
                                handle,
                            );
                            match result {
                                Ok((stats, exile, context, reserve_speed)) => self.admit(
                                    Placement {
                                        pointer,
                                        combat: source.id,
                                        side,
                                        ordinal,
                                        province,
                                        role,
                                        exile_start,
                                    },
                                    Acquired {
                                        stats,
                                        exile,
                                        context,
                                        reserve_speed,
                                    },
                                ),
                                Err(reason) => self.progress[1].failure = Some(reason),
                            }
                        }
                        self.ticks[1] += elapsed(started);
                    }
                    if crate::allocation::failed() {
                        return;
                    }
                    {
                        let _reads = crate::read_accounting::table(TABLES[2]);
                        let started = qpc();
                        self.modifier_member(
                            divisional,
                            base,
                            pointer,
                            handle,
                            ordinal,
                            side_indices[side as usize],
                        );
                        self.ticks[2] += elapsed(started);
                    }
                },
            );
            *self.unit_scratch = owned;
            self.ticks[0] +=
                elapsed(started).saturating_sub(self.ticks[1] + self.ticks[2] - before);
            let result = result.and_then(|visited| match visited {
                cland_combat::Visited::Complete => Ok(()),
                cland_combat::Visited::Scratch { .. } => {
                    Err("land role subsets exceed their full participants")
                }
            });
            if let Err(reason) = result {
                for index in [1, 2] {
                    {
                        self.progress[index].failure = Some(reason);
                    }
                }
            }
        }
        *self.images = images;
    }
    fn combat_sides(&mut self, memory: &impl Memory, base: usize, source: cland_combat::Source) {
        self.progress[3].required += 1;
        if self.progress[3].failure.is_some() {
            return;
        }
        let limit = match self.country_set_limit {
            Ok(limit) => limit,
            Err(reason) => {
                self.progress[3].failure = Some(reason);
                return;
            }
        };
        let start = self.country_tags_progress.len;
        match cland_combat::read_country_sets_buffer(
            memory,
            base,
            source,
            limit,
            engine_native::walk::Buffer::GrowingFrom(self.country_tags, start),
        ) {
            Ok((mut row, count)) => {
                self.country_tags_progress.required += count;
                if !grow_vec(self.combat_sides, self.progress[3].len + 1) {
                    return;
                }
                row.start = start;
                self.combat_sides[self.progress[3].len] = CombatSides { sets: row };
                self.progress[3].len += 1;
                self.country_tags_progress.len += count;
            }
            Err(cland_combat::CountrySetError::Truncated { required }) => {
                self.country_tags_progress.required += required;
            }
            Err(cland_combat::CountrySetError::Unavailable(reason)) => {
                self.progress[3].failure = Some(reason);
            }
        }
    }
    fn detail(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        source: cland_combat::Source,
        members: &cland_combat::Members<'_>,
    ) {
        self.progress[0].required += 1;
        if self.progress[0].failure.is_some() {
            return;
        }
        let at = self.detail_used;
        let roster = self.arenas[0].len;
        let mut arenas = engine_native_land::land_combat_details::DetailArenas::with_buffers(
            engine_native::walk::Buffer::GrowingFrom(self.rosters, roster),
            engine_native::walk::Buffer::GrowingFrom(self.air_counts, at.air),
            engine_native::walk::Buffer::GrowingFrom(self.detail_text, at.text),
            engine_native::walk::Buffer::GrowingFrom(self.commander_traits, at.traits),
            engine_native::walk::Buffer::GrowingFrom(self.timed_traits, at.timed),
            engine_native::walk::Buffer::GrowingFrom(self.learning_traits, at.learning),
        );
        let result = engine_native_land::land_combat_details::read_details_sharing(
            memory,
            base,
            source,
            members,
            &mut self.commanders,
            self.leaders,
            &mut arenas,
        );
        let (complete, used, required) = (arenas.complete(), arenas.used, arenas.required);
        self.arenas[0].required += required.participants;
        for (total, value) in [
            (&mut self.detail_required.air, required.air),
            (&mut self.detail_required.text, required.text),
            (&mut self.detail_required.traits, required.traits),
            (&mut self.detail_required.timed, required.timed),
            (&mut self.detail_required.learning, required.learning),
        ] {
            *total += value;
        }
        match result {
            Ok(mut row) => {
                let context = land_tactic_inputs::capture_context_sharing(
                    memory,
                    base,
                    state,
                    source,
                    members,
                    &mut self.commanders,
                    self.leaders,
                );
                if let Outcome::Unavailable { reason } = context.outcome {
                    self.progress[0].failure = Some(reason);
                    return;
                }
                if !grow_vec(self.details, self.progress[0].len + 1) || !complete {
                    return;
                }
                let text = at.text;
                for side in &mut row.sides {
                    side.committed_start += roster;
                    side.reserve_start += roster;
                    side.traits_start += at.traits;
                    side.timed_traits_start += at.timed;
                    side.air_counts.start += at.air;
                    side.learning_traits.start += at.learning;
                    for token in [
                        &mut side.tactic_token,
                        &mut side.commander_name_token,
                        &mut side.commander_description_token,
                    ] {
                        token.start += text;
                    }
                }
                row.stored_terrain_name.start += text;
                if let Some(border) = &mut row.border_war {
                    for side in &mut border.sides {
                        side.win_event.start += text;
                        side.lose_event.start += text;
                    }
                }
                for stored in &mut self.commander_traits[at.traits..at.traits + used.traits] {
                    stored.token_start += text;
                }
                for stored in &mut self.learning_traits[at.learning..at.learning + used.learning] {
                    stored.token_start += text;
                }
                self.arenas[0].len += used.participants;
                self.detail_used.air += used.air;
                self.detail_used.text += used.text;
                self.detail_used.traits += used.traits;
                self.detail_used.timed += used.timed;
                self.detail_used.learning += used.learning;
                self.details[self.progress[0].len] = Detail {
                    row,
                    tactic_context: context.row,
                };
                self.progress[0].len += 1;
            }
            Err(reason) => self.progress[0].failure = Some(reason),
        }
    }
    fn modifier_member(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: usize,
        handle: u64,
        ordinal: u32,
        side: Option<usize>,
    ) {
        self.arenas[1].required += 1;
        if self.progress[2].failure.is_some() {
            return;
        }
        let scratch = &mut self.scratch.modifiers;
        match engine_native_land::division_combat_inputs::capture_modifiers_buffer(
            memory,
            base,
            pointer,
            handle,
            engine_native::walk::Buffer::Growing(scratch),
        ) {
            Ok(count) => {
                self.arenas[2].required += count;
                let Some(side) = side else {
                    return;
                };
                if !grow_vec(self.members, self.arenas[1].len + 1)
                    || !grow_vec(self.modifiers, self.arenas[2].len + count)
                {
                    self.sides[side].len = usize::MAX;
                    return;
                }
                if self.sides[side].len == usize::MAX {
                    return;
                }
                if self.sides[side].len == 0 {
                    self.sides[side].start = self.arenas[1].len;
                }
                let start = self.arenas[2].len;
                self.modifiers[start..start + count]
                    .copy_from_slice(&self.scratch.modifiers[..count]);
                self.members[self.arenas[1].len] = ModifierMember {
                    division: handle,
                    ordinal,
                    start,
                    len: count,
                };
                self.arenas[1].len += 1;
                self.arenas[2].len += count;
                self.sides[side].len += 1;
            }
            Err(reason) => self.progress[2].failure = Some(reason),
        }
    }
    fn finish(&mut self) {
        let mut retained = 0;
        for index in 0..self.progress[2].len {
            if self.sides[index].len != usize::MAX {
                self.sides[retained] = self.sides[index];
                retained += 1;
            }
        }
        self.progress[2].len = retained;
    }
}

pub struct MemberReads<'a> {
    pub naming: engine_native::cname_group_member::Inputs,
    pub custom: &'a [u8],
    pub group: &'a [u8],
    pub stats: &'a engine_native_land::division_combat_inputs::Row,
    pub base_speed: i64,
}

pub struct Captured {
    pending_owned: bool,
    details: CaptureRows<Detail>,
    divisions: CaptureRows<Division>,
    sides: CaptureRows<ModifierSide>,
    combat_sides: CaptureRows<CombatSides>,
    country_tags: Arena<u32>,
    rosters: Arena<u64>,
    members: Arena<ModifierMember>,
    modifiers: Arena<engine_native_land::division_combat_inputs::Modifier>,
    width_terms: Arena<engine_native_land::division_combat_inputs::WidthTerm>,
    exile_tags: Arena<u32>,
    division_names: Arena<u8>,
    name_cache: engine_native::cname_group_member::GroupKeys,
    leaders: engine_native_land::land_combat_details::Leaders,
    commander_traits: Arena<engine_native_land::land_combat_details::CommanderTrait>,
    detail_text: Arena<u8>,
    timed_traits: Arena<engine_native_land::land_combat_details::CommanderTimedTrait>,
    air_counts: Arena<i32>,
    learning_traits: Arena<engine_native_land::land_combat_details::CommanderLearningTrait>,
    unit_scratch: Vec<usize>,
    census: Vec<cland_combat::Row>,
    census_participants: Vec<u64>,
    census_pointers: Vec<usize>,
    required_census: usize,
    required_census_participants: usize,
    sources: Sources,
    images: Images,
    scratch: Scratch,
    reserve_trees: Vec<ReserveModifierTreeRow>,
    reserve_outcome: Outcome,
    held_reserve_trees: HeldReserveTrees,
    #[cfg(test)]
    reserve_carry_probes: usize,
    #[cfg(test)]
    reference: bool,
}

const COMMANDER_UNIT_ACCUMULATOR: usize = engine_native::carmy_leader::MOVEMENT_MODIFIER
    + engine_native::carmy_leader::MODIFIER_ACCUMULATOR;
const COMMANDER_HQ_ACCUMULATOR: usize = engine_native::carmy_leader::SEPARATE_HQ_MOVEMENT_MODIFIER
    + engine_native::carmy_leader::MODIFIER_ACCUMULATOR;

const RESERVE_TREES_UNCAPTURED: Outcome = Outcome::Unavailable {
    reason: "reserve modifier trees not captured",
};

struct ReserveSource {
    kind: u8,
    owner: u64,
    accumulator: usize,
    receiver: usize,
    leader: Option<usize>,
}

struct HeldReserveTree {
    kind: u8,
    owner: u64,
    certificate: (u64, u64),
    receiver: usize,
    accumulator: usize,
    nodes: Vec<replay_data::rows::ModifierBreakdownNode>,
}

#[derive(Default)]
struct HeldReserveTrees {
    entries: Vec<HeldReserveTree>,
    next: usize,
    #[cfg(test)]
    probes: usize,
}

impl HeldReserveTrees {
    fn new(entries: Vec<HeldReserveTree>) -> Self {
        Self {
            entries,
            ..Self::default()
        }
    }

    fn take(
        &mut self,
        source: &ReserveSource,
        certificate: (u64, u64),
    ) -> Option<Vec<replay_data::rows::ModifierBreakdownNode>> {
        let key = (source.kind, source.owner);
        while let Some(entry) = self.entries.get_mut(self.next) {
            #[cfg(test)]
            {
                self.probes += 1;
            }
            let held = (entry.kind, entry.owner);
            if held > key {
                return None;
            }
            self.next += 1;
            if held == key {
                let same = entry.certificate == certificate
                    && entry.receiver == source.receiver
                    && entry.accumulator == source.accumulator;
                return same.then(|| std::mem::take(&mut entry.nodes));
            }
        }
        None
    }
}

fn leader_name(memory: &impl Memory, leader: usize) -> Result<Vec<u8>, &'static str> {
    let mut name = Vec::new();
    let mut used = 0;
    engine_native::msvc_string::append(
        memory,
        leader
            .checked_add(engine_native::cunit_leader::NAME)
            .ok_or("reserve modifier tree leader address overflow")?,
        &mut name,
        &mut used,
    )?;
    name.truncate(used);
    Ok(name)
}

fn note_tree_refusal(reason: &'static str) {
    REFUSED_TREES.fetch_add(1, Ordering::Relaxed);
    let _ = FIRST_TREE_REFUSAL.set(reason);
}

impl Captured {
    pub fn try_grow_hint(&mut self, c: &Capacities) -> Result<(), &'static str> {
        crate::table_capture::grow_hint! {
            self.census => c.combats,
            self.census_participants => c.census_participants,
            self.census_pointers => c.census_participants,
            self.details.rows => c.details,
            self.rosters.first => c.rosters,
            self.air_counts.first => c.air_counts,
            self.detail_text.first => c.detail_text,
            self.commander_traits.first => c.commander_traits,
            self.timed_traits.first => c.timed_traits,
            self.learning_traits.first => c.learning_traits,
        }
    }
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            combats: 0,
            census_participants: 0,
            details: 0,
            divisions: 0,
            sides: 0,
            combat_sides: 0,
            country_tags: 0,
            rosters: 0,
            modifier_members: 0,
            modifiers: 0,
            width_terms: 0,
            exile_tags: 0,
            division_names: 0,
            name_templates: 0,
            name_groups: 0,
            commander_traits: 0,
            detail_text: 0,
            timed_traits: 0,
            air_counts: 0,
            learning_traits: 0,
        })
    }

    pub fn with_capacities(c: &Capacities) -> Self {
        Self {
            pending_owned: false,
            details: CaptureRows::new(c.details),
            divisions: CaptureRows::new(c.divisions),
            sides: CaptureRows::new(c.sides),
            combat_sides: CaptureRows::new(c.combat_sides),
            country_tags: Arena::new(c.country_tags),
            rosters: Arena::new(c.rosters),
            members: Arena::new(c.modifier_members),
            modifiers: Arena::new(c.modifiers),
            width_terms: Arena::new(c.width_terms),
            exile_tags: Arena::new(c.exile_tags),
            division_names: Arena::new(c.division_names),
            name_cache: engine_native::cname_group_member::GroupKeys::try_with_capacity(
                c.name_templates,
                c.name_groups,
            )
            .map_err(crate::allocation::io)
            .unwrap_or_default(),
            leaders: engine_native_land::land_combat_details::Leaders::default(),
            commander_traits: Arena::new(c.commander_traits),
            detail_text: Arena::new(c.detail_text),
            timed_traits: Arena::new(c.timed_traits),
            air_counts: Arena::new(c.air_counts),
            learning_traits: Arena::new(c.learning_traits),
            unit_scratch: Vec::new(),
            census: allocation::owned_vec(allocation::default_vec(c.combats)),
            census_participants: allocation::owned_vec(allocation::default_vec(
                c.census_participants,
            )),
            census_pointers: allocation::owned_vec(allocation::default_vec(c.census_participants)),
            required_census: 0,
            required_census_participants: 0,
            sources: Sources::new(),
            images: Images::default(),
            scratch: Scratch::default(),
            reserve_trees: Vec::new(),
            reserve_outcome: RESERVE_TREES_UNCAPTURED,
            held_reserve_trees: HeldReserveTrees::default(),
            #[cfg(test)]
            reserve_carry_probes: 0,
            #[cfg(test)]
            reference: false,
        }
    }
    pub fn reset(&mut self) {
        if self.pending_owned {
            self.held_reserve_trees = HeldReserveTrees::default();
        }
        self.pending_owned = false;
        self.scratch.observations.reset();
        self.details.reset();
        self.divisions.reset();
        self.sides.reset();
        self.combat_sides.reset();
        self.country_tags.reset();
        self.rosters.reset();
        self.members.reset();
        self.modifiers.reset();
        self.width_terms.reset();
        self.exile_tags.reset();
        self.division_names.reset();
        self.commander_traits.reset();
        self.detail_text.reset();
        self.timed_traits.reset();
        self.air_counts.reset();
        self.learning_traits.reset();
        self.required_census = 0;
        self.required_census_participants = 0;
        self.reserve_trees.clear();
        self.reserve_outcome = RESERVE_TREES_UNCAPTURED;
    }
    pub fn fail(&mut self, reason: &'static str) {
        self.reset();
        self.held_reserve_trees = HeldReserveTrees::default();
        let outcome = Outcome::Unavailable { reason };
        self.details.outcome = outcome;
        self.divisions.outcome = outcome;
        self.sides.outcome = outcome;
        self.combat_sides.outcome = outcome;
        self.reserve_outcome = outcome;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.details.allocated_bytes()
            + self
                .details
                .rows
                .iter()
                .flat_map(|row| &row.row.sides)
                .map(|side| side.presentation.allocated_bytes())
                .sum::<usize>()
            + self.reserve_trees.capacity() * size_of::<ReserveModifierTreeRow>()
            + self.divisions.allocated_bytes()
            + self.sides.allocated_bytes()
            + self.combat_sides.allocated_bytes()
            + self.country_tags.allocated_bytes()
            + self.rosters.allocated_bytes()
            + self.members.allocated_bytes()
            + self.modifiers.allocated_bytes()
            + self.width_terms.allocated_bytes()
            + self.exile_tags.allocated_bytes()
            + self.division_names.allocated_bytes()
            + self.name_cache.allocated_bytes()
            + self.leaders.allocated_bytes()
            + self.commander_traits.allocated_bytes()
            + self.detail_text.allocated_bytes()
            + self.timed_traits.allocated_bytes()
            + self.air_counts.allocated_bytes()
            + self.learning_traits.allocated_bytes()
            + self.unit_scratch.capacity() * size_of::<usize>()
            + self.census.capacity() * size_of::<cland_combat::Row>()
            + self.census_participants.capacity() * size_of::<u64>()
            + self.census_pointers.capacity() * size_of::<usize>()
            + self.scratch.allocated_bytes()
    }
    pub fn grow(&mut self, c: &mut Capacities) {
        c.divisions = c.divisions.max(self.divisions.outcome.len());
        c.width_terms = c.width_terms.max(self.width_terms.len);
        c.exile_tags = c.exile_tags.max(self.exile_tags.len);
        c.division_names = c.division_names.max(self.division_names.len);
        self.details.grow(&mut c.details);
        self.divisions.grow(&mut c.divisions);
        self.sides.grow(&mut c.sides);
        self.combat_sides.grow(&mut c.combat_sides);
        self.country_tags.grow(&mut c.country_tags);
        self.rosters.grow(&mut c.rosters);
        self.members.grow(&mut c.modifier_members);
        self.modifiers.grow(&mut c.modifiers);
        self.width_terms.grow(&mut c.width_terms);
        self.exile_tags.grow(&mut c.exile_tags);
        self.division_names.grow(&mut c.division_names);
        let current = self.name_cache.capacities();
        let sized = [c.name_templates, c.name_groups].map(|n| n.checked_next_power_of_two());
        if sized
            .iter()
            .zip(current)
            .any(|(wanted, current)| wanted.is_some_and(|wanted| wanted > current))
        {
            let Ok(cache) = engine_native::cname_group_member::GroupKeys::try_with_capacity(
                c.name_templates,
                c.name_groups,
            )
            .map_err(crate::allocation::io) else {
                return;
            };
            self.name_cache = cache;
        }
        self.commander_traits.grow(&mut c.commander_traits);
        self.detail_text.grow(&mut c.detail_text);
        self.timed_traits.grow(&mut c.timed_traits);
        self.air_counts.grow(&mut c.air_counts);
        self.learning_traits.grow(&mut c.learning_traits);
        grow_capacity(&mut c.combats, self.required_census);
        grow_capacity(
            &mut c.census_participants,
            self.required_census_participants,
        );
        grow_vec(&mut self.census, c.combats);
        grow_vec(&mut self.census_participants, c.census_participants);
        grow_vec(&mut self.census_pointers, c.census_participants);
    }
    fn pass(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> ([Outcome; N], [u64; N]) {
        self.sources.clear();
        if crate::allocation::failed() {
            return (
                [Outcome::Unavailable {
                    reason: "collector allocation failed",
                }; N],
                [0; N],
            );
        }
        let _reads = crate::read_accounting::table(TABLES[0]);
        self.name_cache.reset();
        self.leaders.clear();
        let started = qpc();
        let mut destination = Destination {
            sources: &mut self.sources,
            commanders: engine_native_land::land_combat_details::Commanders::default(),
            leaders: &mut self.leaders,
            images: &mut self.images,
            scratch: &mut self.scratch,
            name_groups: &mut self.name_cache,
            names: engine_native::cname_group_member::Names {
                bytes: engine_native::walk::Buffer::Growing(&mut self.division_names.first),
                len: 0,
                required: 0,
            },
            #[cfg(test)]
            reference: self.reference,
            details: &mut self.details.rows,
            divisions: &mut self.divisions.rows,
            sides: &mut self.sides.rows,
            combat_sides: &mut self.combat_sides.rows,
            country_tags: &mut self.country_tags.first,
            country_tags_progress: Progress::default(),
            country_set_limit: engine_native::ccountry::Reader::new(memory, base, state)
                .map(|countries| countries.count() as usize + 1),
            rosters: &mut self.rosters.first,
            commander_traits: &mut self.commander_traits.first,
            detail_text: &mut self.detail_text.first,
            timed_traits: &mut self.timed_traits.first,
            air_counts: &mut self.air_counts.first,
            learning_traits: &mut self.learning_traits.first,
            unit_scratch: &mut self.unit_scratch,
            detail_used: engine_native_land::land_combat_details::DetailCounts::default(),
            detail_required: engine_native_land::land_combat_details::DetailCounts::default(),
            members: &mut self.members.first,
            modifiers: &mut self.modifiers.first,
            width_terms: &mut self.width_terms.first,
            width_progress: Progress::default(),
            exile_tags: &mut self.exile_tags.first,
            exile_progress: Progress::default(),
            progress: [Progress::default(); N],
            arenas: [Progress::default(); N],
            ticks: [0; N],
        };
        let census = cland_combat::walk_members(
            memory,
            base,
            state,
            &mut self.census,
            &mut self.census_participants,
            &mut self.census_pointers,
            |source, members| {
                #[cfg(test)]
                if destination.reference {
                    return destination.visit_reference(memory, base, state, source, members);
                }
                destination.visit(memory, base, state, source, members)
            },
        );
        destination.finish();
        let outcomes = destination.progress.map(|p| p.outcome(census.outcome));
        let arenas = destination.arenas;
        let width = destination.width_progress;
        let exile = destination.exile_progress;
        let names = (destination.names.len, destination.names.required);
        let country_tags = destination.country_tags_progress;
        let (detail_used, detail_required) = (destination.detail_used, destination.detail_required);
        let mut ticks = destination.ticks;
        ticks[0] += elapsed(started).saturating_sub(ticks.iter().sum::<u64>());
        self.required_census = self.required_census.max(match census.outcome {
            Outcome::Truncated { len, unread_rows } => len.saturating_add(unread_rows as usize),
            _ => census.outcome.len(),
        });
        self.required_census_participants = self
            .required_census_participants
            .max(census.required_participants);
        self.rosters.len = arenas[0].len;
        self.rosters.required = arenas[0].required;
        self.commander_traits.len = detail_used.traits;
        self.commander_traits.required = detail_required.traits;
        self.detail_text.len = detail_used.text;
        self.detail_text.required = detail_required.text;
        self.timed_traits.len = detail_used.timed;
        self.timed_traits.required = detail_required.timed;
        self.air_counts.len = detail_used.air;
        self.air_counts.required = detail_required.air;
        self.learning_traits.len = detail_used.learning;
        self.learning_traits.required = detail_required.learning;
        self.width_terms.len = width.len;
        self.width_terms.required = width.required;
        self.exile_tags.len = exile.len;
        self.exile_tags.required = exile.required;
        (self.division_names.len, self.division_names.required) = names;
        self.members.len = arenas[1].len;
        self.members.required = arenas[1].required;
        self.modifiers.len = arenas[2].len;
        self.modifiers.required = arenas[2].required;
        self.country_tags.len = country_tags.len;
        self.country_tags.required = country_tags.required;
        (outcomes, ticks)
    }
    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        self.scratch.defer_speed = false;
        let mut ticks = self.capture_observed(memory, base, state);
        let start = qpc();
        self.settle_owned();
        ticks[0] += elapsed(start);
        ticks
    }

    pub fn capture_deferred(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> [u64; N] {
        self.scratch.defer_speed = true;
        let ticks = self.capture_observed(memory, base, state);
        self.scratch.defer_speed = false;
        ticks
    }

    fn capture_observed(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        if self.pending_owned {
            self.held_reserve_trees = HeldReserveTrees::default();
            self.pending_owned = false;
        }
        self.scratch.observations.reset();
        let started = qpc();
        let (outcomes, mut ticks) = self.pass(memory, base, state);
        self.details.outcome = outcomes[0];
        self.divisions.outcome = outcomes[1];
        self.sides.outcome = outcomes[2];
        self.combat_sides.outcome = outcomes[3];
        self.pending_owned = true;
        let trees = qpc();
        {
            let _reads = crate::read_accounting::table(TABLES[5]);
            self.capture_reserve_trees(memory, base, &naval_tree_changes::Observer);
        }
        ticks[5] = elapsed(trees);
        if crate::allocation::failed() {
            self.settle_owned();
            self.fail("collector allocation failed");
            return ticks;
        }
        ticks[0] += elapsed(started).saturating_sub(ticks.iter().sum::<u64>());
        ticks
    }

    pub fn owned_tail_allocated_bytes(&self) -> usize {
        self.leaders.allocated_bytes()
    }

    pub fn member_divisions(&self) -> impl ExactSizeIterator<Item = u64> + '_ {
        let len = self.divisions.outcome.len().min(self.divisions.rows.len());
        self.divisions.rows[..len].iter().map(|row| row.stats.division)
    }
    pub fn join_members(&mut self, mut published: impl FnMut(&MemberReads<'_>) -> bool) {
        let names = &self.division_names.first;
        let span = |(start, len): (usize, usize)| names.get(start..start.checked_add(len)?);
        for row in &mut self.divisions.rows {
            row.joined = false;
        }
        let len = self.divisions.outcome.len().min(self.divisions.rows.len());
        for row in &mut self.divisions.rows[..len] {
            let Some(naming) = row.stats.naming else {
                continue;
            };
            let (Some(custom), Some(group)) = (span(naming.custom), span(naming.group)) else {
                continue;
            };
            row.joined = published(&MemberReads {
                naming,
                custom,
                group,
                stats: &row.stats,
                base_speed: row.context.base_speed,
            });
        }
    }
    pub fn settle_owned(&mut self) {
        if !self.pending_owned {
            return;
        }
        let len = self.divisions.outcome.len().min(self.divisions.rows.len());
        for row in &mut self.divisions.rows[..len] {
            if let Some(index) = row.reserve_observation {
                row.reserve_speed = Some(self.scratch.observations.compute(index));
                row.reserve_observation = None;
            }
        }
        self.count_reserve_shares();
        self.count_side_input_refusals();
        self.sources.clear();
        self.leaders.clear();
        self.scratch.observations.reset();
        if crate::allocation::failed() {
            self.held_reserve_trees = HeldReserveTrees::default();
        }
        self.pending_owned = false;
    }
    fn capture_reserve_trees(
        &mut self,
        memory: &impl Memory,
        base: usize,
        changes: &impl naval_tree_changes::Changes,
    ) {
        self.reserve_trees.clear();
        let result = self.reserve_tree_rows(memory, base, changes);
        self.reserve_outcome = match result {
            Ok(outcome) => outcome,
            Err(reason) => {
                self.reserve_trees.clear();
                Outcome::Unavailable { reason }
            }
        };
    }
    fn reserve_tree_rows(
        &mut self,
        memory: &impl Memory,
        base: usize,
        changes: &impl naval_tree_changes::Changes,
    ) -> Result<Outcome, &'static str> {
        const ALLOCATION: &str = "collector allocation failed";
        let (len, unread) = match self.divisions.outcome {
            Outcome::Rows { len } => (len, 0),
            Outcome::Truncated { len, unread_rows } => (len, unread_rows),
            Outcome::Unavailable { reason } => return Err(reason),
        };
        let divisions = self
            .divisions
            .rows
            .get(..len)
            .ok_or("reserve modifier tree division census outside its rows")?;
        let mut sources: Vec<ReserveSource> = Vec::new();
        for row in divisions.iter().filter(|row| row.role == 2) {
            sources.try_reserve(2).map_err(|_| ALLOCATION)?;
            sources.push(ReserveSource {
                kind: 0,
                owner: u64::from(row.context.owner),
                accumulator: row.context.country_accumulator,
                receiver: row
                    .context
                    .country_accumulator
                    .wrapping_sub(engine_native::ccountry::MODIFIER_ACCUMULATOR),
                leader: None,
            });
            let kind = row.context.leader_modifier_kind;
            if kind != 0 {
                let offset = if kind == 1 {
                    COMMANDER_UNIT_ACCUMULATOR
                } else {
                    COMMANDER_HQ_ACCUMULATOR
                };
                let accumulator = row.context.leader_accumulator;
                let leader = accumulator.wrapping_sub(offset);
                sources.push(ReserveSource {
                    kind,
                    owner: row.stats.commander,
                    accumulator,
                    receiver: leader,
                    leader: Some(leader),
                });
            }
        }
        sources.sort_unstable_by_key(|source| (source.kind as u128) << 64 | source.owner as u128);
        sources.dedup_by_key(|source| (source.kind, source.owner));
        naval_tree_changes::reserve_trees(sources.len());
        self.reserve_trees
            .try_reserve(sources.len())
            .map_err(|_| ALLOCATION)?;
        let mut held = std::mem::take(&mut self.held_reserve_trees);
        let mut kept = Vec::new();
        kept.try_reserve(sources.len()).map_err(|_| ALLOCATION)?;
        let mut counts = [0_u64; 2];
        for source in sources {
            let certificate = if source.kind < 2 && source.accumulator != 0 {
                changes.sequence(source.receiver)
            } else {
                None
            };
            let carried = certificate.and_then(|certificate| held.take(&source, certificate));
            let fresh =
                || -> Result<Vec<replay_data::rows::ModifierBreakdownNode>, &'static str> {
                    if source.accumulator == 0 {
                        return Err("reserve modifier tree accumulator missing");
                    }
                    let tree = engine_native_politics::modifier_breakdown::printed_nodes(
                        memory,
                        base,
                        source.accumulator,
                        engine_executable::modifier_keys::LAND_REINFORCE_RATE,
                    )?;
                    replay_data::rows::printed_tree(&tree).map_err(|_| ALLOCATION)
                };
            let nodes = match carried {
                Some(entry) => {
                    counts[0] += 1;
                    Ok(entry)
                }
                None => {
                    counts[1] += 1;
                    fresh()
                }
            };
            if let (Some(certificate), Ok(nodes)) = (certificate, &nodes) {
                let nodes = engine_allocation::collect_vec(nodes.iter().cloned())
                    .map_err(|_| ALLOCATION)?;
                kept.push(HeldReserveTree {
                    kind: source.kind,
                    owner: source.owner,
                    certificate,
                    receiver: source.receiver,
                    accumulator: source.accumulator,
                    nodes,
                });
            }
            let root = nodes
                .as_ref()
                .map_or(1, |nodes| nodes.first().map_or(0, |node| node.value));
            let (name, nodes) = match (source.leader, nodes) {
                (Some(_), Ok(_)) if root == 0 => continue,
                (Some(leader), Ok(nodes)) => match leader_name(memory, leader) {
                    Ok(name) => (name, Some(nodes)),
                    Err(reason) => {
                        note_tree_refusal(reason);
                        (Vec::new(), None)
                    }
                },
                (None, Ok(nodes)) => (Vec::new(), Some(nodes)),
                (_, Err(reason)) => {
                    note_tree_refusal(reason);
                    (Vec::new(), None)
                }
            };
            self.reserve_trees.push(ReserveModifierTreeRow {
                kind: source.kind,
                owner: source.owner,
                name,
                nodes,
            });
        }
        #[cfg(test)]
        {
            self.reserve_carry_probes = held.probes;
        }
        self.held_reserve_trees = HeldReserveTrees::new(kept);
        for (total, count) in RESERVE_TREE_COUNTS.iter().zip(counts) {
            total.fetch_add(count, Ordering::Relaxed);
        }
        Ok(if unread == 0 {
            Outcome::Rows {
                len: self.reserve_trees.len(),
            }
        } else {
            Outcome::Truncated {
                len: self.reserve_trees.len(),
                unread_rows: unread,
            }
        })
    }
    fn count_reserve_shares(&self) {
        let len = match self.divisions.outcome {
            Outcome::Rows { len } | Outcome::Truncated { len, .. } => len,
            Outcome::Unavailable { .. } => 0,
        };
        let (mut computed, mut refused) = (0_u64, 0_u64);
        for row in &self.divisions.rows[..len.min(self.divisions.rows.len())] {
            match row.reserve_speed {
                Some(Ok(_)) => computed += 1,
                Some(Err(reason)) => {
                    refused += 1;
                    let _ = FIRST_SHARE_REFUSAL.set(reason);
                }
                None => {}
            }
        }
        RESERVE_SHARES.fetch_add(computed, Ordering::Relaxed);
        REFUSED_SHARES.fetch_add(refused, Ordering::Relaxed);
    }
    fn count_side_input_refusals(&self) {
        let len = match self.details.outcome {
            Outcome::Rows { len } | Outcome::Truncated { len, .. } => len,
            Outcome::Unavailable { .. } => 0,
        };
        let mut refused = 0_u64;
        for detail in &self.details.rows[..len.min(self.details.rows.len())] {
            for reason in detail
                .row
                .sides
                .iter()
                .flat_map(|side| side.side_input_refusals)
                .flatten()
            {
                refused += 1;
                let _ = FIRST_SIDE_INPUT_REFUSAL.set(reason);
            }
        }
        REFUSED_SIDE_INPUTS.fetch_add(refused, Ordering::Relaxed);
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.details.outcome,
            self.divisions.outcome,
            self.sides.outcome,
            self.combat_sides.outcome,
            self.members_outcome(),
            self.reserve_outcome,
        ]
    }
    fn read_sides(&self) -> Option<&[ModifierSide]> {
        match self.sides.outcome {
            Outcome::Rows { len } | Outcome::Truncated { len, .. } => self.sides.rows.get(..len),
            Outcome::Unavailable { .. } => None,
        }
    }
    fn members_outcome(&self) -> Outcome {
        match self.sides.outcome {
            Outcome::Unavailable { reason } => return Outcome::Unavailable { reason },
            Outcome::Rows { .. } | Outcome::Truncated { .. } => {}
        }
        let Some(sides) = self.read_sides() else {
            return Outcome::Unavailable {
                reason: "walker row count exceeds allocated buffer",
            };
        };
        let read: usize = sides.iter().map(|side| side.len).sum();
        match self.members.required.saturating_sub(read) {
            0 => Outcome::Rows { len: read },
            unread => Outcome::Truncated {
                len: read,
                unread_rows: unread.min(u32::MAX as usize) as u32,
            },
        }
    }
    fn tactic_inputs(context: &land_tactic_inputs::ContextRow) -> LandCombatTacticInputs {
        LandCombatTacticInputs {
            preferred_tactic_ids: context.preferred_tactic_ids,
            country_weights: context
                .country_weights
                .map(|w| LandCombatTacticCountryWeight {
                    country: w.country,
                    tactic_id: w.tactic_id,
                    modifier_key: w.modifier_key,
                    metadata_flags: w.metadata_flags,
                    metadata_gate: w.metadata_gate,
                    country_factor_raw: w.country_factor_raw,
                    accumulator_raw: w.accumulator_raw,
                    accumulator_entry_present: w.accumulator_entry_present,
                }),
            country_weight_present: context.country_weight_present,
        }
    }
    fn side_inputs(&self, s: &engine_native_land::land_combat_details::SideDetails) -> LandCombatSideInputs {
        let text = |start: usize, len: usize| {
            allocation::owned_vec(allocation::copy_vec(
                &self.detail_text.first[start..start + len],
            ))
        };
        let timed = &self.timed_traits.first
            [s.timed_traits_start..s.timed_traits_start + s.timed_traits_len];
        LandCombatSideInputs {
            anti_air_attack: s.anti_air_attack,
            prevented_damage: [s.damage[4], s.damage[5]],
            skill_progress: skill_progress(
                s.commander_experience,
                s.next_skill,
                s.side_input_refusals[0].is_some(),
            ),
            skill_modifiers: s
                .skill_modifiers
                .or(s.side_input_refusals[2].is_none().then_some([0; 2])),
            description_token: text(
                s.commander_description_token.start,
                s.commander_description_token.len,
            ),
            timed_traits: allocation::owned_vec(allocation::collect_vec(timed.iter().map(|t| {
                LandCombatCommanderTimedTrait {
                    token_id: t.token_id,
                    definition_ordinal: t.definition_ordinal,
                    remaining_days: t.remaining_days,
                }
            }))),
            learning_traits: match s.learning_threshold {
                Some(threshold) => Some(allocation::owned_vec(allocation::collect_vec(
                    self.learning_traits.first[s.learning_traits.range()]
                        .iter()
                        .filter_map(|entry| {
                            let progress =
                                experience_ratio(entry.experience, entry.required_experience);
                            (progress > threshold).then(|| LandCombatLearningProgress {
                                token_id: entry.token_id,
                                definition_ordinal: entry.definition_ordinal,
                                progress,
                                token: text(entry.token_start, entry.token_len),
                            })
                        }),
                ))),
                None if s.side_input_refusals[1].is_some() => None,
                None => Some(Vec::new()),
            },
        }
    }
    pub fn tables(&self) -> [TableView; N] {
        let outcomes = self.outcomes();
        let member_key = |row: &Division| (row.combat, row.side, row.ordinal);
        let members = match outcomes[1] {
            Outcome::Rows { len } => self.divisions.rows.get(..len).filter(|rows| {
                rows.windows(2)
                    .all(|pair| member_key(&pair[0]) < member_key(&pair[1]))
            }),
            _ => None,
        };
        let published = |combat: u64, side: u8, ordinal: u32, division: u64| {
            members.is_some_and(|rows| {
                rows.binary_search_by_key(&(combat, side, ordinal), member_key)
                    .is_ok_and(|at| rows[at].stats.division == division)
            })
        };
        [
            table::<tables::LandCombatDetails, _>(outcomes[0], &self.details.rows, |captured| {
                let r = &captured.row;
                LandCombatDetailsRow {
                    tactic_inputs: Self::tactic_inputs(&captured.tactic_context),
                    border_war: Recorded::Value(r.border_war.map(|border| {
                        replay_data::rows::LandBorderWarInputs {
                            transfer_state: border.transfer_state,
                            combat_width: Recorded::Value(border.combat_width),
                            sides: border
                                .sides
                                .map(|side| replay_data::rows::LandBorderWarSide {
                                    country: side.country,
                                    state: side.state,
                                    win_event: allocation::owned_vec(allocation::copy_vec(
                                        &self.detail_text.first[side.win_event.range()],
                                    )),
                                    lose_event: allocation::owned_vec(allocation::copy_vec(
                                        &self.detail_text.first[side.lose_event.range()],
                                    )),
                                }),
                        }
                    })),
                    combat: r.id,
                    province: r.province,
                    phase: r.phase,
                    width_factor: r.width_factor,
                    attacker_bonus: r.attacker_bonus,
                    defender_bonus: r.defender_bonus,
                    stored_terrain_name: allocation::owned_vec(allocation::copy_vec(
                        &self.detail_text.first[r.stored_terrain_name.range()],
                    )),
                    stored_terrain_valid: r.stored_terrain_valid,
                    side_inputs: Recorded::Value(std::array::from_fn(|side| {
                        self.side_inputs(&r.sides[side])
                    })),
                    sides: std::array::from_fn(|side| {
                        let s = &r.sides[side];
                        LandCombatSideDetails {
                            presentation: commander_presentation(&s.presentation),
                            committed: allocation::owned_vec(allocation::copy_vec(
                                &self.rosters.first
                                    [s.committed_start..s.committed_start + s.committed_len],
                            )),
                            reserves: allocation::owned_vec(allocation::copy_vec(
                                &self.rosters.first
                                    [s.reserve_start..s.reserve_start + s.reserve_len],
                            )),
                            tactic_id: s.tactic_id,
                            tactic_kind: s.tactic_kind,
                            tactic_counter_id: s.tactic_counter_id,
                            tactic_token: allocation::owned_vec(allocation::copy_vec(
                                &self.detail_text.first[s.tactic_token.range()],
                            )),
                            air_counts: allocation::owned_vec(allocation::copy_vec(
                                &self.air_counts.first[s.air_counts.range()],
                            )),
                            anti_air_attack: RetiredKept::default(),
                            air_kills: s.air_kills,
                            damage: [s.damage[0], s.damage[1]],
                            ground_damage: RetiredNested::default(),
                            prevented_damage: RetiredKept::default(),
                            organisation_loss_sum: LandCombatSideDetails::loss_sum(
                                &s.organisation_loss,
                            ),
                            organisation_loss_count: s.organisation_loss_count,
                            commander: s.commander,
                            commander_skill: s.commander_skill,
                            commander_category: s.commander_category,
                            commander_country: s.commander_country,
                            commander_experience: RetiredNested::default(),
                            commander_name_token: allocation::owned_vec(allocation::copy_vec(
                                &self.detail_text.first[s.commander_name_token.range()],
                            )),
                            commander_description_token: RetiredNested::default(),
                            commander_formation: s.commander_formation,
                            timed_traits: RetiredNested::default(),
                            learning_traits: RetiredNested::default(),
                            traits: allocation::owned_vec(allocation::collect_vec(
                                self.commander_traits.first
                                    [s.traits_start..s.traits_start + s.traits_len]
                                    .iter()
                                    .map(|t| LandCombatCommanderTrait {
                                        token_id: t.token_id,
                                        definition_ordinal: t.definition_ordinal,
                                        kind: t.kind,
                                        relevant: t.relevant,
                                        token: allocation::owned_vec(allocation::copy_vec(
                                            &self.detail_text.first
                                                [t.token_start..t.token_start + t.token_len],
                                        )),
                                    }),
                            )),
                        }
                    }),
                }
            }),
            table::<tables::LandCombatDivisions, _>(outcomes[1], &self.divisions.rows, |r| {
                let s = r.stats;
                let naming = s.naming.unwrap_or_default();
                let name = |(start, len): (usize, usize)| {
                    allocation::owned_vec(allocation::copy_vec(
                        &self.division_names.first[start..start + len],
                    ))
                };
                LandCombatDivisionRow {
                    combat: r.combat,
                    side: r.side,
                    ordinal: r.ordinal,
                    division: s.division,
                    role: r.role,
                    province: r.province,
                    own: (!r.joined).then(|| LandCombatMemberValues {
                        naming: NameOperands {
                            kind: naming.kind,
                            order: naming.order,
                            ordered: naming.ordered,
                            programmatic: naming.programmatic,
                            custom: name(naming.custom),
                            group: name(naming.group),
                        },
                        strength_combat_factor: s.strength_combat_factor,
                        fuel_inputs: s.fuel_inputs,
                        stat_values: s.stat_values,
                        armour: s.armour,
                        piercing: s.piercing,
                        air_attack: s.air_attack,
                        reconnaissance: s.reconnaissance,
                        initiative: s.initiative,
                        base_speed: r.context.base_speed,
                    }),
                    commander: s.commander,
                    direction_group: s.direction_group,
                    categories: s.categories,
                    exile: r.exile.map(|exile| {
                        let tags = |(start, len): (usize, usize)| {
                            allocation::owned_vec(allocation::copy_vec(
                                &self.exile_tags.first[start..start + len],
                            ))
                        };
                        LandCombatExileInputs {
                            origin_type: exile.origin_type,
                            country: exile.country,
                            country_fallback: exile.country_fallback,
                            location_cores: tags(exile.cores),
                            occupiers: tags(exile.occupiers),
                        }
                    }),
                    scales: s.scales,
                    reinforcement_speed: r.reserve_speed.and_then(Result::ok),
                    context: LandDivisionContext {
                        template: r.context.template,
                        country_modifiers: r.context.country_modifiers.map(|m| {
                            LandDivisionModifier {
                                raw: m.raw,
                                present: m.present,
                            }
                        }),
                        leader_modifiers: r.context.leader_modifiers.map(|m| {
                            LandDivisionModifier {
                                raw: m.raw,
                                present: m.present,
                            }
                        }),
                        modifier_metadata: r.context.modifier_metadata,
                        leader_modifier_kind: r.context.leader_modifier_kind,
                        feature_flags: r.context.feature_flags,
                        eligibility_callback: r.context.eligibility_callback,
                    },
                    width_terms: allocation::owned_vec(allocation::collect_vec(
                        self.width_terms.first
                            [r.width_start..r.width_start + r.context.width_term_count]
                            .iter()
                            .map(|t| LandDivisionWidthTerm {
                                subunit: t.subunit,
                                count: t.count,
                                width: t.width,
                            }),
                    )),
                }
            }),
            table::<tables::LandCombatModifiers, _>(outcomes[2], &self.sides.rows, |r| {
                LandCombatModifierRow {
                    combat: r.combat,
                    side: r.side,
                    border_values: r.border_values,
                    divisions: Retired,
                }
            }),
            table::<tables::LandCombatSides, _>(outcomes[3], &self.combat_sides.rows, |r| {
                let mut start = r.sets.start;
                let mut set = |index: usize| {
                    let tags = allocation::owned_vec(allocation::copy_vec(
                        &self.country_tags.first[start..start + r.sets.lens[index]],
                    ));
                    start += r.sets.lens[index];
                    tags
                };
                LandCombatSideRow {
                    combat: r.sets.combat,
                    indicator: r.sets.indicator,
                    flags: r.sets.flags,
                    attacker_primary: set(0),
                    attacker_secondary: set(1),
                    defender_primary: set(2),
                    defender_secondary: set(3),
                }
            }),
            match (outcomes[4], self.read_sides()) {
                (Outcome::Unavailable { reason }, _) => unavailable(reason),
                (outcome, Some(sides)) => table_iter::<tables::LandCombatDivisionModifiers, _>(
                    outcome,
                    sides.iter().flat_map(|side| {
                        self.members.first[side.start..side.start + side.len]
                            .iter()
                            .map(move |member| (side, member))
                    }),
                    |(side, member)| LandCombatDivisionModifierRow {
                        combat: side.combat,
                        side: side.side,
                        ordinal: member.ordinal,
                        division: (!published(side.combat, side.side, member.ordinal, member.division))
                            .then_some(member.division),
                        entries: allocation::owned_vec(allocation::collect_vec(
                            self.modifiers.first[member.start..member.start + member.len]
                                .iter()
                                .map(|v| CombatModifierValue {
                                    id: v.id,
                                    value_a: v.value_a,
                                    value_b: v.value_b,
                                }),
                        )),
                    },
                ),
                (_, None) => unavailable("walker row count exceeds allocated buffer"),
            },
            table::<tables::ReserveModifierTrees, _>(outcomes[5], &self.reserve_trees, |row| {
                ReserveModifierTreeRow {
                    kind: row.kind,
                    owner: row.owner,
                    name: allocation::owned_vec(allocation::copy_vec(&row.name)),
                    nodes: row.nodes.as_ref().map(|nodes| {
                        allocation::owned_vec(allocation::collect_vec(nodes.iter().cloned()))
                    }),
                }
            }),
        ]
    }
}
pub(super) fn experience_ratio(experience: i64, required: i32) -> i64 {
    let denominator = i64::from(required) * 100_000;
    if denominator == 0
        || (experience as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd
    {
        return 0xffff_ffff;
    }
    experience.wrapping_mul(100_000) / denominator
}

pub(super) fn skill_progress(
    experience: i64,
    next: Option<engine_native_naval::naval_commander_inputs::SkillDefinition>,
    refused: bool,
) -> i64 {
    match next {
        Some(next) if next.present != 0 && next.valid != 0 => {
            experience_ratio(experience, next.required_experience)
        }
        Some(_) => LandCombatSideInputs::NO_PROGRESS,
        None if refused => LandCombatSideInputs::UNREAD_PROGRESS,
        None => LandCombatSideInputs::NO_PROGRESS,
    }
}

pub fn commander_presentation(
    p: &engine_native_naval::naval_commander_inputs::Presentation,
) -> replay_data::rows::CombatCommanderPresentation {
    use replay_data::{rows::CombatCommanderPortraitEntry, rows::CombatCommanderPresentation};
    CombatCommanderPresentation {
        character: p.character,
        character_state: p.character_state,
        character_country: p.character_country,
        character_gender: p.character_gender,
        character_role_leader: p.character_role_leader,
        character_fallback_leader: p.character_fallback_leader,
        character_role_source: p.character_role_source,
        fallback_text: std::array::from_fn(|index| {
            allocation::owned_vec(allocation::copy_vec(
                &p.fallback_text[index][..p.fallback_text_len[index]],
            ))
        }),
        portraits: allocation::owned_vec(allocation::collect_vec(
            p.portraits[..p.portraits_len]
                .iter()
                .map(|v| CombatCommanderPortraitEntry {
                    role: v.role,
                    size: v.size,
                    is_set: v.is_set,
                    kind: v.kind,
                    text: allocation::owned_vec(allocation::copy_vec(
                        &p.portrait_text[v.text_start..v.text_start + v.text_len],
                    )),
                }),
        )),
        role_traits: allocation::owned_vec(allocation::collect_vec(
            p.role_traits[..p.role_traits_len]
                .iter()
                .map(|v| LandCombatCommanderTrait {
                    token_id: v.token_id,
                    definition_ordinal: v.definition_ordinal,
                    kind: v.kind,
                    relevant: v.relevant,
                    token: allocation::owned_vec(allocation::copy_vec(
                        &p.role_trait_text[v.token_start..v.token_start + v.token_len],
                    )),
                }),
        )),
    }
}
#[cfg(test)]
mod reference_walk;

fn elapsed(start: i64) -> u64 {
    qpc().saturating_sub(start).max(0) as u64
}

#[cfg(test)]
mod tests;

#[cfg(any(test, feature = "test-support"))]
mod family;

#[cfg(any(test, feature = "test-support"))]
pub fn allocation_failure_exercise(arm: impl FnOnce(), disarm: impl FnOnce()) {
    family::allocation_failure_exercise(arm, disarm);
}
