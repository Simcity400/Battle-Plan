use engine_native::cunit_leader::{CommanderLearningTrait, CommanderTrait};
use engine_native::cland_combat::{
    ATTACKER,
    ATTACKER_BONUS,
    DEFENDER,
    DEFENDER_BONUS,
    Members,
    PHASE,
    PROVINCE,
    Reader,
    STORED_TERRAIN,
    Source,
    WIDTH_FACTOR,
};
use engine_native_naval::naval_commander_inputs::SkillDefinition;
use engine_native_memory::{Memory, cpdx_array::Array, read, walk::Buffer};
use engine_native::{carmy, cland_combatant};
use std::collections::HashMap;

#[derive(Default, Clone, Copy)]
pub struct Commanders {
    sides: [Option<(Array, Selection)>; 2],
}

type Selection = Result<CommanderSource, &'static str>;

#[derive(Default)]
pub struct Leaders {
    presentations: HashMap<usize, engine_native_naval::naval_commander_inputs::Presentation>,
    next_skills: HashMap<(i32, i32), Result<SkillDefinition, &'static str>>,
    pub(crate) parents: HashMap<usize, Result<usize, &'static str>>,
    pub(crate) preferred: HashMap<usize, Result<i32, &'static str>>,
}

impl Leaders {
    pub(crate) fn shared<K: Eq + std::hash::Hash, V: Copy>(
        held: &mut HashMap<K, V>,
        key: K,
        read: impl FnOnce() -> V,
    ) -> V {
        if let Some(&value) = held.get(&key) {
            return value;
        }
        let value = read();
        if engine_allocation::advisory(|| engine_allocation::reserve_map(held, 1)).is_ok() {
            held.insert(key, value);
        }
        value
    }

    fn presentation(
        &mut self,
        leader: usize,
        read: impl FnOnce() -> Result<engine_native_naval::naval_commander_inputs::Presentation, &'static str>,
    ) -> Result<engine_native_naval::naval_commander_inputs::Presentation, &'static str> {
        if let Some(Ok(copy)) = self.presentations.get(&leader).map(|held| held.copied()) {
            return Ok(copy);
        }
        let presentation = read()?;
        let held = engine_allocation::advisory(|| {
            engine_allocation::reserve_map(&mut self.presentations, 1)?;
            presentation.copied()
        });
        if let Ok(held) = held {
            self.presentations.insert(leader, held);
        }
        Ok(presentation)
    }

    pub fn allocated_bytes(&self) -> usize {
        self.presentations.capacity()
            * size_of::<(usize, engine_native_naval::naval_commander_inputs::Presentation)>()
            + self
                .presentations
                .values()
                .map(|held| held.allocated_bytes())
                .sum::<usize>()
            + self.next_skills.capacity()
                * size_of::<((i32, i32), Result<SkillDefinition, &'static str>)>()
            + self.parents.capacity() * size_of::<(usize, Result<usize, &'static str>)>()
            + self.preferred.capacity() * size_of::<(usize, Result<i32, &'static str>)>()
    }

    pub fn clear(&mut self) {
        self.presentations.clear();
        self.next_skills.clear();
        self.parents.clear();
        self.preferred.clear();
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub len: usize,
}

impl Span {
    pub fn range(self) -> std::ops::Range<usize> {
        self.start..self.start + self.len
    }
}


pub use engine_native::cunit_leader::TimedTrait as CommanderTimedTrait;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DetailCounts {
    pub participants: usize,
    pub air: usize,
    pub text: usize,
    pub traits: usize,
    pub timed: usize,
    pub learning: usize,
}

pub struct DetailArenas<'a> {
    pub participants: Buffer<'a, u64>,
    pub air: Buffer<'a, i32>,
    pub text: Buffer<'a, u8>,
    pub traits: Buffer<'a, CommanderTrait>,
    pub timed: Buffer<'a, CommanderTimedTrait>,
    pub learning: Buffer<'a, CommanderLearningTrait>,
    pub used: DetailCounts,
    pub required: DetailCounts,
    pub(super) verify_headers: bool,
}

impl<'a> DetailArenas<'a> {
    pub fn new(
        participants: &'a mut Vec<u64>,
        air: &'a mut Vec<i32>,
        text: &'a mut Vec<u8>,
        traits: &'a mut Vec<CommanderTrait>,
        timed: &'a mut Vec<CommanderTimedTrait>,
        learning: &'a mut Vec<CommanderLearningTrait>,
    ) -> Self {
        Self::with_buffers(
            Buffer::Growing(participants),
            Buffer::Growing(air),
            Buffer::Growing(text),
            Buffer::Growing(traits),
            Buffer::Growing(timed),
            Buffer::Growing(learning),
        )
    }

    pub fn count() -> Self {
        Self::with_buffers(
            Buffer::Count,
            Buffer::Count,
            Buffer::Count,
            Buffer::Count,
            Buffer::Count,
            Buffer::Count,
        )
    }

    pub fn with_buffers(
        participants: Buffer<'a, u64>,
        air: Buffer<'a, i32>,
        text: Buffer<'a, u8>,
        traits: Buffer<'a, CommanderTrait>,
        timed: Buffer<'a, CommanderTimedTrait>,
        learning: Buffer<'a, CommanderLearningTrait>,
    ) -> Self {
        Self {
            participants,
            air,
            text,
            traits,
            timed,
            learning,
            used: DetailCounts::default(),
            required: DetailCounts::default(),
            verify_headers: false,
        }
    }

    pub fn complete(&self) -> bool {
        self.used == self.required
    }
}

pub fn push<T>(arena: &mut [T], used: &mut usize, required: &mut usize, value: T) {
    *required += 1;
    if let Some(slot) = arena.get_mut(*used) {
        *slot = value;
        *used += 1;
    }
}


#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CommanderSource {
    pub pointer: usize,
    pub handle: u64,
    pub skill: i32,
    pub category: i32,
}

pub fn commander_sources(
    memory: &impl Memory,
    base: usize,
    source: Source,
    members: &Members<'_>,
    commanders: &mut Commanders,
) -> Result<[CommanderSource; 2], &'static str> {
    let reader = DetailReader(Reader {
        memory,
        base,
        provinces: source.provinces,
    });
    let border = reader
        .class(source.pointer)?
        .ok_or("land commander source is naval")?;
    let header = reader.header(source.pointer, border)?;
    if header != (source.id, source.attackers, source.defenders) {
        return Err("land commander source changed");
    }
    Ok([
        reader.selected(commanders, members, 0, source.attackers)?,
        reader.selected(commanders, members, 1, source.defenders)?,
    ])
}

pub const SKILL_HOVER_KEYS: [u32; 2] = [0x5f, 0x60];

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SideDetails {
    pub committed_start: usize,
    pub committed_len: usize,
    pub reserve_start: usize,
    pub reserve_len: usize,
    pub tactic_id: i32,
    pub tactic_kind: u8,
    pub tactic_counter_id: u32,
    pub tactic_token: Span,
    pub air_counts: Span,
    pub anti_air_attack: i64,
    pub air_kills: u32,
    pub damage: [i64; 6],
    pub organisation_loss: [i64; 24],
    pub organisation_loss_count: u32,
    pub commander: u64,
    pub commander_skill: i32,
    pub commander_category: i32,
    pub commander_country: u32,
    pub commander_experience: i64,
    pub next_skill: Option<engine_native_naval::naval_commander_inputs::SkillDefinition>,
    pub learning_threshold: Option<i64>,
    pub skill_modifiers: Option<[i64; 2]>,
    pub side_input_refusals: [Option<&'static str>; 3],
    pub commander_name_token: Span,
    pub commander_description_token: Span,
    pub commander_formation: u64,
    pub presentation: engine_native_naval::naval_commander_inputs::Presentation,
    pub traits_start: usize,
    pub traits_len: usize,
    pub timed_traits_start: usize,
    pub timed_traits_len: usize,
    pub learning_traits: Span,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BorderWarSide {
    pub country: i32,
    pub state: i32,
    pub win_event: Span,
    pub lose_event: Span,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BorderWarInputs {
    pub transfer_state: u8,
    pub sides: [BorderWarSide; 2],
    pub combat_width: i64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DetailsRow {
    pub id: u64,
    pub province: u32,
    pub phase: i32,
    pub width_factor: i64,
    pub attacker_bonus: i64,
    pub defender_bonus: i64,
    pub stored_terrain_valid: u8,
    pub stored_terrain_name: Span,
    pub sides: [SideDetails; 2],
    pub border_war: Option<BorderWarInputs>,
}

pub fn read_border_war(
    reader: &DetailReader<'_, impl Memory>,
    source: Source,
    arenas: &mut DetailArenas<'_>,
) -> Result<BorderWarInputs, &'static str> {
    use engine_native_parts::{
        cland_border_war_combat as combat,
        cland_border_war_combatant as side,
    };
    let mut row = BorderWarInputs {
        transfer_state: reader.field(source.pointer, combat::TRANSFER_STATE)?,
        combat_width: reader.field(source.pointer, combat::COMBAT_WIDTH)?,
        ..Default::default()
    };
    for (index, offset) in [ATTACKER, DEFENDER].into_iter().enumerate() {
        let pointer = reader.pointer(source.pointer, offset)?;
        let country = reader.field(pointer, side::COUNTRY)?;
        let state = reader.pointer(pointer, side::STATE)?;
        reader.vtable(state, 0, engine_native::cstate::VTABLE_RVA)?;
        let state = reader.field::<i32>(state, engine_native::cstate::ID)?;
        if state <= 0 {
            return Err("border-war state id invalid");
        }
        let events = reader.field::<[u8; 64]>(pointer, side::WIN_EVENT)?;
        let lose_offset = side::LOSE_EVENT - side::WIN_EVENT;
        row.sides[index] = BorderWarSide {
            country,
            state,
            win_event: reader.string(
                pointer,
                side::WIN_EVENT,
                &events[..lose_offset],
                &BORDER_WIN_EVENT,
                arenas,
            )?,
            lose_event: reader.string(
                pointer,
                side::LOSE_EVENT,
                &events[lose_offset..],
                &BORDER_LOSE_EVENT,
                arenas,
            )?,
        };
    }
    Ok(row)
}

pub fn copy_detail_spans<const N: usize>(
    memory: &impl Memory,
    sources: [(usize, usize); N],
    destinations: &mut [&mut [u8]; N],
) -> Result<(), &'static str> {
    if memory.copy_scattered_prefetched(
        sources.map(|(pointer, offset)| pointer.checked_add(offset).unwrap_or(0)),
        destinations,
    ) == N
    {
        Ok(())
    } else {
        Err("land combat field unreadable or address overflow")
    }
}

const BORDER_WIN_EVENT: engine_native::cname_group_member::StringErrors = [
    "border-war win event header invalid",
    "border-war win event pointer unreadable or null",
    "border-war win event bytes unreadable",
    "border-war win event terminator unreadable or invalid",
];
const BORDER_LOSE_EVENT: engine_native::cname_group_member::StringErrors = [
    "border-war lose event header invalid",
    "border-war lose event pointer unreadable or null",
    "border-war lose event bytes unreadable",
    "border-war lose event terminator unreadable or invalid",
];
const SELECTED_NAME: engine_native::cname_group_member::StringErrors = [
    "land combat commander name token header invalid",
    "land combat commander name token pointer unreadable or null",
    "land combat commander name token bytes unreadable",
    "land combat commander name token terminator unreadable or invalid",
];
const SELECTED_DESCRIPTION: engine_native::cname_group_member::StringErrors = [
    "land combat commander description token header invalid",
    "land combat commander description token pointer unreadable or null",
    "land combat commander description token bytes unreadable",
    "land combat commander description token terminator unreadable or invalid",
];

pub fn read_details(
    memory: &impl Memory,
    base: usize,
    source: Source,
    members: &Members<'_>,
    commanders: &mut Commanders,
    arenas: &mut DetailArenas<'_>,
) -> Result<DetailsRow, &'static str> {
    read_details_sharing(
        memory,
        base,
        source,
        members,
        commanders,
        &mut Leaders::default(),
        arenas,
    )
}

pub fn read_details_sharing(
    memory: &impl Memory,
    base: usize,
    source: Source,
    members: &Members<'_>,
    commanders: &mut Commanders,
    leaders: &mut Leaders,
    arenas: &mut DetailArenas<'_>,
) -> Result<DetailsRow, &'static str> {
    let reader = DetailReader(Reader {
        memory,
        base,
        provinces: source.provinces,
    });
    let border = reader
        .class(source.pointer)?
        .ok_or("land details source is naval")?;
    let (id, attackers, defenders) = reader.header(source.pointer, border)?;
    if id != source.id || attackers != source.attackers || defenders != source.defenders {
        return Err("land details source identity changed");
    }
    let province_id = reader.row(source.pointer, id)?.province;
    let (mut phase, mut width_factor, mut attacker_bonus, mut defender_bonus) =
        ([0; 4], [0; 8], [0; 8], [0; 8]);
    copy_detail_spans(
        memory,
        [
            (source.pointer, PHASE),
            (source.pointer, WIDTH_FACTOR),
            (source.pointer, ATTACKER_BONUS),
            (source.pointer, DEFENDER_BONUS),
        ],
        &mut [
            &mut phase,
            &mut width_factor,
            &mut attacker_bonus,
            &mut defender_bonus,
        ],
    )?;
    let mut row = DetailsRow {
        id,
        province: province_id,
        phase: i32::from_le_bytes(phase),
        width_factor: i64::from_le_bytes(width_factor),
        attacker_bonus: i64::from_le_bytes(attacker_bonus),
        defender_bonus: i64::from_le_bytes(defender_bonus),
        ..DetailsRow::default()
    };
    if border {
        row.border_war = Some(read_border_war(&reader, source, arenas)?);
    }
    let stored_terrain = reader.pointer(source.pointer, STORED_TERRAIN)?;
    let stored_vtable = reader.field::<u64>(stored_terrain, 0)?;
    if ![
        engine_native_parts::cterrain_type::VTABLE_RVA,
        engine_native_parts::cterrain_type::NULL_VTABLE_RVA,
    ]
    .into_iter()
    .any(|key| engine_sites::at(base, key).map(|p| p as u64) == Some(stored_vtable))
    {
        return Err("land stored terrain vtable mismatch");
    }
    row.stored_terrain_valid = reader.field(stored_terrain, engine_native_parts::cterrain_type::VALID)?;
    if row.stored_terrain_valid != 0 {
        row.stored_terrain_name =
            reader.text(stored_terrain, engine_native_parts::cterrain_type::NAME, arenas)?;
    }
    for (index, offset) in [ATTACKER, DEFENDER].into_iter().enumerate() {
        let pointer = reader.pointer(source.pointer, offset)?;
        let side = &mut row.sides[index];
        let units = [attackers, defenders][index];
        reader.commander(
            pointer, members, commanders, leaders, index, units, side, arenas,
        )?;
        for (which, vector_offset) in [cland_combatant::COMMITTED, cland_combatant::RESERVES]
            .into_iter()
            .enumerate()
        {
            let vector = reader.array(pointer, vector_offset, 8)?;
            let (start, count) = (arenas.used.participants, vector.count as usize);
            arenas.participants.ensure(
                start
                    .checked_add(count)
                    .ok_or("land participant arena overflow")?,
            )?;
            if which == 0 {
                side.committed_start = start;
                side.committed_len = count;
            } else {
                side.reserve_start = start;
                side.reserve_len = count;
            }
            let mut hint = 0;
            for member in 0..vector.count {
                let unit = reader.element(vector, member)?;
                let handle = reader.known_unit(members, index, unit, &mut hint)?;
                push(
                    &mut arenas.participants,
                    &mut arenas.used.participants,
                    &mut arenas.required.participants,
                    handle,
                );
            }
            reader.verify_array(pointer, vector_offset, 8, vector, arenas)?;
        }
        let tactic = reader.pointer(pointer, cland_combatant::TACTIC)?;
        let tactic_vtable = if tactic == 0 {
            0
        } else {
            reader.field::<u64>(tactic, 0)?
        };
        if Some(tactic_vtable)
            == engine_sites::at(base, engine_native_parts::ccombat_tactic::VTABLE_RVA)
                .map(|v| v as u64)
        {
            side.tactic_kind = 2;
            side.tactic_id = reader.field(tactic, engine_native_parts::ccombat_tactic::ID)?;
            side.tactic_counter_id = reader.field(tactic, engine_native_parts::ccombat_tactic::COUNTER_ID)?;
            side.tactic_token = reader.text(tactic, engine_native_parts::ccombat_tactic::TOKEN, arenas)?;
        } else if tactic == 0 {
            side.tactic_kind = 0;
        } else if Some(tactic_vtable)
            != engine_sites::at(base, engine_native_parts::cnull_combat_tactic::VTABLE_RVA)
                .map(|v| v as u64)
        {
            return Err("land detail tactic vtable mismatch");
        } else {
            side.tactic_kind = 1;
        }
        let air = reader.array(pointer, cland_combatant::AIR_SUPPORT, 8)?;
        arenas.air.ensure(
            arenas
                .used
                .air
                .checked_add(air.count as usize)
                .ok_or("land air arena overflow")?,
        )?;
        side.air_counts = Span {
            start: arenas.used.air,
            len: air.count as usize,
        };
        for member in 0..air.count {
            let planes = reader.field(
                reader.element(air, member)?,
                engine_native_parts::land_combat_air_entry::PLANE_COUNT,
            )?;
            push(
                &mut arenas.air,
                &mut arenas.used.air,
                &mut arenas.required.air,
                planes,
            );
        }
        reader.verify_array(pointer, cland_combatant::AIR_SUPPORT, 8, air, arenas)?;
        let (mut anti_air, mut kills, mut damage, mut loss_count) =
            ([0; 8], [0; 4], [0; 48], [0; 4]);
        copy_detail_spans(
            memory,
            [
                (pointer, cland_combatant::ANTI_AIR_ATTACK),
                (pointer, cland_combatant::AIR_KILLS),
                (pointer, cland_combatant::AIR_STRENGTH_DAMAGE),
                (pointer, cland_combatant::ORGANISATION_LOSS_COUNT),
            ],
            &mut [&mut anti_air, &mut kills, &mut damage, &mut loss_count],
        )?;
        side.anti_air_attack = i64::from_le_bytes(anti_air);
        side.air_kills = u32::from_le_bytes(kills);
        for (value, bytes) in side.damage.iter_mut().zip(damage.chunks_exact(8)) {
            *value = i64::from_le_bytes(bytes.try_into().unwrap());
        }
        side.organisation_loss_count = u32::from_le_bytes(loss_count);
        if side.organisation_loss_count >= 24 {
            let samples = reader.pointer(pointer, cland_combatant::ORGANISATION_LOSS_WINDOW)?;
            let loss = reader.field::<[u8; 192]>(samples, 0)?;
            for (value, bytes) in side.organisation_loss.iter_mut().zip(loss.chunks_exact(8)) {
                *value = i64::from_le_bytes(bytes.try_into().unwrap());
            }
        }
    }
    Ok(row)
}

pub struct DetailReader<'a, M>(pub Reader<'a, M>);

impl<'a, M> std::ops::Deref for DetailReader<'a, M> {
    type Target = Reader<'a, M>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<M: Memory> DetailReader<'_, M> {
    fn selected(
        &self,
        commanders: &mut Commanders,
        members: &Members<'_>,
        side: usize,
        units: Array,
    ) -> Result<CommanderSource, &'static str> {
        if let Some((held, selection)) = commanders.sides[side]
            && held == units
        {
            return selection;
        }
        let selection = self.commander_source(members, side, units);
        commanders.sides[side] = Some((units, selection));
        selection
    }

    fn commander_source(
        &self,
        members: &Members<'_>,
        side: usize,
        units: Array,
    ) -> Result<CommanderSource, &'static str> {
        use engine_native::{carmy_leader, corders_group, cunit_leader};
        use engine_native_parts::cunit_leader_skill;
        let mut selected = CommanderSource::default();
        for index in 0..units.count {
            let (unit, _) = self.participant(members, side, units, index)?;
            let group = self.pointer(unit, carmy::ORDERS_GROUP)?;
            if group == 0 {
                continue;
            }
            self.vtable(group, 0, corders_group::VTABLE_RVA)?;
            let leader = self.pointer(group, corders_group::LEADER)?;
            if leader == 0 {
                continue;
            }
            self.vtable(leader, 0, carmy_leader::VTABLE_RVA)?;
            let handle = self.field::<u64>(leader, cunit_leader::TYPE)?;
            if handle as u32 != cunit_leader::REFERENCE_TYPE || handle >> 32 == 0 {
                return Err("land commander reference invalid");
            }
            let skill = self.pointer(leader, cunit_leader::SKILL)?;
            self.vtable(skill, 0, cunit_leader_skill::VTABLE_RVA)?;
            let skill = self.field(skill, cunit_leader_skill::LEVEL)?;
            let category = self.field(leader, cunit_leader::CATEGORY)?;
            if selected.pointer == 0
                || skill > selected.skill
                || (skill == selected.skill && selected.category == 0 && category == 1)
            {
                selected = CommanderSource {
                    pointer: leader,
                    handle,
                    skill,
                    category,
                };
            }
        }
        Ok(selected)
    }

    fn skill_modifiers(
        &self,
        members: &Members<'_>,
        side: usize,
        units: Array,
        leader: usize,
    ) -> Result<[i64; 2], &'static str> {
        use engine_native::carmy_leader::{
            MODIFIER_ACCUMULATOR,
            MOVEMENT_MODIFIER,
            SEPARATE_HQ_MOVEMENT_MODIFIER,
        };
        let mut separate = units.count != 0;
        for index in 0..units.count {
            let (unit, _) = self.participant(members, side, units, index)?;
            if !engine_native::carmy_leader::separate_hq_order(self.memory, self.base, unit)? {
                separate = false;
                break;
            }
        }
        let block = if separate {
            SEPARATE_HQ_MOVEMENT_MODIFIER
        } else {
            MOVEMENT_MODIFIER
        };
        let accumulator = leader
            .checked_add(block + MODIFIER_ACCUMULATOR)
            .ok_or("land commander modifier address overflow")?;
        let mut values = [engine_native_memory::modifier_cache::Value::default(); 2];
        engine_native_parts::modifier_accumulator::lookup_many_at(
            self.memory,
            accumulator,
            &SKILL_HOVER_KEYS,
            &mut values,
        )?;
        Ok(values.map(|value| value.raw))
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one commander reads the shared members, commanders, leaders, units and arenas into the side row"
    )]
    pub fn commander(
        &self,
        side: usize,
        members: &Members<'_>,
        commanders: &mut Commanders,
        leaders: &mut Leaders,
        index: usize,
        units: Array,
        row: &mut SideDetails,
        arenas: &mut DetailArenas<'_>,
    ) -> Result<(), &'static str> {
        use engine_native::{carmy_leader, cunit_leader};
        use engine_native_parts::cunit_leader_trait;
        let commander = self.selected(commanders, members, index, units)?;
        let selected = commander.pointer;
        row.commander = commander.handle;
        row.commander_skill = commander.skill;
        row.commander_category = commander.category;
        if selected == 0 {
            row.commander_country = if self.field::<i32>(side, 0x48)? == 0 {
                let combat = self.pointer(side, cland_combatant::COMBAT)?;
                self.field(self.pointer(combat, PROVINCE)?, 0x188)?
            } else {
                self.field(self.pointer(side, cland_combatant::COUNTRIES)?, 0)?
            };
            return Ok(());
        }
        row.commander_country = self.field(selected, 0x120)?;
        row.commander_experience = self.field(selected, cunit_leader::EXPERIENCE)?;
        let refused = &mut row.side_input_refusals;
        row.next_skill = Leaders::shared(
            &mut leaders.next_skills,
            (commander.skill, commander.category),
            || {
                engine_native_naval::naval_commander_inputs::capture_next_skill(
                    self.memory,
                    self.base,
                    commander.skill,
                    commander.category,
                )
            },
        )
        .inspect_err(|reason| refused[0] = Some(*reason))
        .ok();
        row.learning_threshold =
            engine_sites::at(self.base, engine_native_naval::naval_commander_inputs::LEARNING_THRESHOLD_RVA)
                .ok_or("land combat field unreadable or address overflow")
                .and_then(|address| self.field(address, 0))
                .inspect_err(|reason| refused[1] = Some(*reason))
                .ok();
        row.skill_modifiers = self
            .skill_modifiers(members, index, units, selected)
            .inspect_err(|reason| refused[2] = Some(*reason))
            .ok();
        for (offset, unreadable, errors, span) in [
            (
                cunit_leader::TOKEN,
                "land combat commander name token header unreadable",
                &SELECTED_NAME,
                &mut row.commander_name_token,
            ),
            (
                cunit_leader::DESCRIPTION_TOKEN,
                "land combat commander description token header unreadable",
                &SELECTED_DESCRIPTION,
                &mut row.commander_description_token,
            ),
        ] {
            let header: [u8; engine_native_memory::msvc_string::STRIDE] = selected
                .checked_add(offset)
                .and_then(|address| read(self.memory, address))
                .ok_or(unreadable)?;
            *span = self.string(selected, offset, &header, errors, arenas)?;
        }
        if (commander.category as u32) < 2 {
            row.commander_formation = self.field(selected, carmy_leader::FORMATION_REFERENCE)?;
        }
        row.presentation = leaders.presentation(selected, || {
            engine_native_naval::naval_commander_inputs::capture_presentation(self.memory, self.base, selected)
        })?;
        let traits = self.array(selected, cunit_leader::TRAIT_DATA, 8)?;
        arenas.traits.ensure(
            arenas
                .used
                .traits
                .checked_add(traits.count as usize)
                .ok_or("land trait arena overflow")?,
        )?;
        row.traits_start = arenas.used.traits;
        row.traits_len = traits.count as usize;
        for ordinal in 0..traits.count {
            let pointer = self.element(traits, ordinal)?;
            self.vtable(pointer, 0, cunit_leader_trait::VTABLE_RVA)?;
            let token = self.text(pointer, cunit_leader_trait::TOKEN, arenas)?;
            let value = CommanderTrait {
                token_id: self.field(pointer, cunit_leader_trait::TOKEN_ID)?,
                definition_ordinal: self.field(pointer, cunit_leader_trait::ORDINAL)?,
                kind: self.field(pointer, cunit_leader_trait::KIND)?,
                relevant: self.field(pointer, 0xa74)?,
                token_start: token.start,
                token_len: token.len,
            };
            push(
                &mut arenas.traits,
                &mut arenas.used.traits,
                &mut arenas.required.traits,
                value,
            );
        }
        let timed = self.array(
            selected,
            cunit_leader::TIMED_TRAIT_DATA,
            cunit_leader::TIMED_TRAIT_STRIDE,
        )?;
        arenas.timed.ensure(
            arenas
                .used
                .timed
                .checked_add(timed.count as usize)
                .ok_or("land timed trait arena overflow")?,
        )?;
        row.timed_traits_start = arenas.used.timed;
        row.timed_traits_len = timed.count as usize;
        cunit_leader::read_timed_traits(self.memory, self.base, timed, |value| {
            push(
                &mut arenas.timed,
                &mut arenas.used.timed,
                &mut arenas.required.timed,
                value,
            );
        })?;
        let learning = self.array(selected, cunit_leader::TRAIT_EXPERIENCE_DATA, 16)?;
        arenas.learning.ensure(
            arenas
                .used
                .learning
                .checked_add(learning.count as usize)
                .ok_or("land learning trait arena overflow")?,
        )?;
        row.learning_traits = Span {
            start: arenas.used.learning,
            len: learning.count as usize,
        };
        for index in 0..learning.count {
            let entry = learning
                .element(index, 16)
                .ok_or("land learning trait address overflow")?;
            let pointer = self.pointer(entry, 0)?;
            self.vtable(pointer, 0, cunit_leader_trait::VTABLE_RVA)?;
            let token = self.text(pointer, cunit_leader_trait::TOKEN, arenas)?;
            let value = CommanderLearningTrait {
                token_id: self.field(pointer, cunit_leader_trait::TOKEN_ID)?,
                definition_ordinal: self.field(pointer, cunit_leader_trait::ORDINAL)?,
                required_experience: self
                    .field(pointer, cunit_leader_trait::REQUIRED_EXPERIENCE)?,
                experience: self.field(entry, 8)?,
                token_start: token.start,
                token_len: token.len,
            };
            push(
                &mut arenas.learning,
                &mut arenas.used.learning,
                &mut arenas.required.learning,
                value,
            );
        }
        for (offset, stride, vector) in [
            (cunit_leader::TRAIT_DATA, 8, traits),
            (
                cunit_leader::TIMED_TRAIT_DATA,
                cunit_leader::TIMED_TRAIT_STRIDE,
                timed,
            ),
            (cunit_leader::TRAIT_EXPERIENCE_DATA, 16, learning),
        ] {
            self.verify_array(selected, offset, stride, vector, arenas)?;
        }
        Ok(())
    }

    pub fn text(
        &self,
        pointer: usize,
        offset: usize,
        arenas: &mut DetailArenas<'_>,
    ) -> Result<Span, &'static str> {
        let address = pointer
            .checked_add(offset)
            .ok_or("land detail string address overflow")?;
        let header = if arenas.verify_headers {
            Some(self.field::<[u8; 32]>(pointer, offset)?)
        } else {
            None
        };
        let (source, len) = engine_native_parts::cdynamic_modifier::literal(self.memory, address)?;
        let start = arenas.used.text;
        arenas.required.text = arenas
            .required
            .text
            .checked_add(len)
            .ok_or("land detail text overflows")?;
        arenas
            .text
            .ensure(start.checked_add(len).ok_or("land detail text overflows")?)?;
        if let Some(output) = start
            .checked_add(len)
            .and_then(|end| arenas.text.get_mut(start..end))
        {
            if !engine_native_memory::read_slice(self.memory, source, output) {
                return Err("land detail string unreadable");
            }
            arenas.used.text += len;
        }
        if let Some(header) = header
            && read::<[u8; 32]>(self.memory, address) != Some(header)
        {
            return Err("land detail demand string changed");
        }
        Ok(Span { start, len })
    }

    fn string(
        &self,
        pointer: usize,
        offset: usize,
        header: &[u8],
        errors: &engine_native::cname_group_member::StringErrors,
        arenas: &mut DetailArenas<'_>,
    ) -> Result<Span, &'static str> {
        let mut names = engine_native::cname_group_member::Names {
            bytes: arenas.text.suffix(0),
            len: arenas.used.text,
            required: arenas.required.text,
        };
        let copied =
            engine_native::cname_group_member::copy_string(self.memory, header, &mut names, errors)?;
        let len = names.required - arenas.required.text;
        arenas.required.text = names.required;
        let start = arenas.used.text;
        if copied.is_some() {
            arenas.used.text += len;
        }
        if arenas.verify_headers {
            let address = pointer
                .checked_add(offset)
                .ok_or("land detail string address overflow")?;
            let observed = read::<[u8; 32]>(self.memory, address);
            if observed.as_ref().map(|value| value.as_slice()) != Some(header) {
                return Err("land detail demand string changed");
            }
        }
        Ok(Span { start, len })
    }

    fn verify_array(
        &self,
        pointer: usize,
        offset: usize,
        stride: usize,
        vector: Array,
        arenas: &DetailArenas<'_>,
    ) -> Result<(), &'static str> {
        if arenas.verify_headers && self.array(pointer, offset, stride)? != vector {
            return Err("land detail demand array changed");
        }
        Ok(())
    }
}
