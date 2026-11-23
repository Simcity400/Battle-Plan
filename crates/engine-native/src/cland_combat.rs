
use engine_sites::Rva;
use engine_native_memory::{Memory, cpdx_array::Array, memory::Raw, read, walk::Outcome};
use crate::{
    carmy,
    cgame_state,
    cland_combatant,
    cnaval_combat,
    cprovince,
    ctask_force,
    cunit,
};
use engine_native_parts::{
    cland_border_war_combat,
    cland_border_war_combatant,
    crailway_gun,
};

mod country_sets;
pub use country_sets::{
    CountrySetError, CountrySetsRow, read_country_sets_buffer,
};

pub const VTABLE_RVA: Rva = Rva(0x02a2_37b8);
pub const BODY_VTABLE_RVA: Rva = Rva(0x02a2_3898);
pub const BODY: usize = 0x10;
pub const HANDLE: usize = 0x18;
pub const TYPE: usize = 0x18;
pub const TYPE_WORD: u32 = 62;
pub const ID: usize = 0x1c;
pub const ATTACKER: usize = 0x28;
pub const DEFENDER: usize = 0x30;
pub const PROVINCE: usize = 0x38;
pub const DAY: usize = 0x40;
pub const DURATION: usize = 0x44;
pub const STORED_TERRAIN: usize = 0x48;
pub const INDICATOR_FRAME_HIGH: usize = 0x98;
pub const ATTACKER_BONUS: usize = 0xa8;
pub const DEFENDER_BONUS: usize = 0xb0;
pub const WIDTH_FACTOR: usize = 0xa0;
pub const PHASE: usize = 0xd0;
pub const DIVISION_CLASS: u32 = 0;
pub const TASK_FORCE_CLASS: u32 = 1;
pub const RAILWAY_GUN_CLASS: u32 = 13;
pub const MOVEMENT_MODIFIER: usize = 0xb8;

#[cfg(any(test, feature = "test-support"))]
pub fn movement_combat(
    memory: &impl Memory,
    base: usize,
    combat: usize,
    attackers: &mut Vec<usize>,
) -> Result<Option<i64>, &'static str> {
    let start = attackers.len();
    let result = movement_combat_side(memory, base, combat, attackers).and_then(|side| {
        side.complete?;
        movement_combat_word(memory, combat, side.land)
    });
    if result.is_err() {
        attackers.truncate(start);
    }
    result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MovementCombatSide {
    pub land: bool,
    pub complete: Result<(), &'static str>,
}

pub fn movement_combat_side(
    memory: &impl Memory,
    base: usize,
    combat: usize,
    attackers: &mut Vec<usize>,
) -> Result<MovementCombatSide, &'static str> {
    let start = attackers.len();
    let result = (|| {
        let field = |object: usize, offset: usize| {
            object
                .checked_add(offset)
                .and_then(|address| read::<u64>(memory, address))
                .ok_or("movement combat field unreadable")
        };
        let rva = |key: Rva| engine_sites::at(base, key).map(|p| p as u64);
        let vtable = Some(field(combat, 0)?);
        let land =
            if vtable == rva(VTABLE_RVA) || vtable == rva(cland_border_war_combat::VTABLE_RVA) {
                true
            } else if vtable == rva(cnaval_combat::VTABLE_RVA) {
                false
            } else {
                return Err("movement combat identity mismatch");
            };
        let side = field(combat, ATTACKER)? as usize;
        let side_vtable = Some(field(side, 0)?);
        if ![
            cland_combatant::VTABLE_RVA,
            cland_border_war_combatant::VTABLE_RVA,
            crate::cnaval_combatant::VTABLE_RVA,
        ]
        .iter()
        .any(|&known| rva(known) == side_vtable)
            || field(side, cland_combatant::COMBAT)? != combat as u64
        {
            return Err("movement combat side identity mismatch");
        }
        let units = side
            .checked_add(cland_combatant::UNITS)
            .and_then(|address| Array::read(memory, address, 8))
            .ok_or("movement combat side units invalid")?;
        engine_allocation::reserve(attackers, units.count as usize)
            .map_err(|_| "movement combat allocation failed")?;
        for index in 0..units.count {
            let at = units
                .element(index, 8)
                .ok_or("movement combat side unit index invalid")?;
            match field(at, 0) {
                Ok(pointer) => attackers.push(pointer as usize),
                Err(reason) => {
                    return Ok(MovementCombatSide {
                        land,
                        complete: Err(reason),
                    });
                }
            }
        }
        Ok(MovementCombatSide {
            land,
            complete: Ok(()),
        })
    })();
    if result.is_err() {
        attackers.truncate(start);
    }
    result
}
pub fn movement_combat_word(
    memory: &impl Memory,
    combat: usize,
    land: bool,
) -> Result<Option<i64>, &'static str> {
    if !land {
        return Ok(None);
    }
    combat
        .checked_add(MOVEMENT_MODIFIER)
        .and_then(|address| read::<i64>(memory, address))
        .map(Some)
        .ok_or("movement combat modifier unreadable")
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    pub province: u32,
    pub duration: i32,
    pub day: i32,
    pub attacker_start: usize,
    pub attacker_len: usize,
    pub defender_start: usize,
    pub defender_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub outcome: Outcome,
    pub participants_len: usize,
    pub required_participants: usize,
}

#[derive(Clone, Copy)]
pub struct Source {
    pub pointer: usize,
    pub id: u64,
    pub sides: [usize; 2],
    pub attackers: Array,
    pub defenders: Array,
    pub provinces: Array,
}

#[derive(Clone, Copy)]
pub struct Members<'a> {
    pub pointers: &'a [usize],
    pub handles: &'a [u64],
    pub attackers: usize,
}

impl<'a> Members<'a> {
    pub const fn none() -> Self {
        Self {
            pointers: &[],
            handles: &[],
            attackers: 0,
        }
    }

    fn side(&self, side: usize) -> (&'a [usize], &'a [u64]) {
        let split = self.attackers.min(self.pointers.len());
        let (pointers, handles) = if side == 0 {
            (
                &self.pointers[..split],
                &self.handles[..split.min(self.handles.len())],
            )
        } else {
            (
                &self.pointers[split..],
                self.handles.get(split..).unwrap_or(&[]),
            )
        };
        (pointers, handles)
    }

    fn at(&self, side: usize, units: Array, index: u32) -> Option<(usize, u64)> {
        let (pointers, handles) = self.side(side);
        if pointers.len() != units.count as usize || handles.len() != pointers.len() {
            return None;
        }
        Some((*pointers.get(index as usize)?, handles[index as usize]))
    }

    fn handle(&self, side: usize, pointer: usize, hint: &mut usize) -> Option<u64> {
        let (pointers, handles) = self.side(side);
        if handles.len() != pointers.len() {
            return None;
        }
        let count = pointers.len();
        for step in 0..count {
            let index = (*hint + step) % count;
            if pointers[index] == pointer {
                *hint = index + 1;
                return Some(handles[index]);
            }
        }
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visited {
    Complete,
    Scratch {
        required: usize,
    },
}

pub fn visit_units(
    memory: &impl Memory,
    base: usize,
    source: Source,
    members: &Members<'_>,
    scratch: &mut [usize],
    mut visit: impl FnMut(u8, u32, usize, u64, u32, u8),
) -> Result<Visited, &'static str> {
    let reader = Reader {
        memory,
        base,
        provinces: source.provinces,
    };
    let border = reader
        .class(source.pointer)?
        .ok_or("land units source is naval")?;
    let header = reader.header(source.pointer, border)?;
    if header != (source.id, source.attackers, source.defenders) {
        return Err("land units source changed");
    }
    let mut roles = [[Array { data: 0, count: 0 }; 2]; 2];
    for (side, roles) in roles.iter_mut().enumerate() {
        let side_pointer = reader.pointer(source.pointer, [ATTACKER, DEFENDER][side])?;
        *roles = [
            reader.array(side_pointer, cland_combatant::COMMITTED, 8)?,
            reader.array(side_pointer, cland_combatant::RESERVES, 8)?,
        ];
    }
    let required = roles
        .iter()
        .map(|[committed, reserves]| committed.count as usize + reserves.count as usize)
        .max()
        .unwrap_or(0);
    if required > scratch.len() {
        return Ok(Visited::Scratch { required });
    }
    for (side, units) in [source.attackers, source.defenders].into_iter().enumerate() {
        let [committed, reserves] = roles[side];
        let (first, second) = scratch.split_at_mut(committed.count as usize);
        let subset = [first, &mut second[..reserves.count as usize]];
        for (which, array) in [committed, reserves].into_iter().enumerate() {
            for index in 0..array.count {
                subset[which][index as usize] = reader.element(array, index)?;
            }
            subset[which].sort_unstable();
            if subset[which].windows(2).any(|pair| pair[0] == pair[1]) {
                return Err("land role subset has duplicate unit");
            }
        }
        let mut matched = [0_u32; 2];
        for ordinal in 0..units.count {
            let (unit, handle) = reader.participant(members, side, units, ordinal)?;
            let province = reader.province_id(reader.pointer(unit, cunit::LOCATION)?)?;
            let committed_member = subset[0].binary_search(&unit).is_ok();
            let reserve_member = subset[1].binary_search(&unit).is_ok();
            if committed_member && reserve_member {
                return Err("land role subsets overlap");
            }
            matched[0] += u32::from(committed_member);
            matched[1] += u32::from(reserve_member);
            let role = if committed_member {
                1
            } else if reserve_member {
                2
            } else {
                0
            };
            visit(side as u8, ordinal, unit, handle, province, role);
        }
        if matched != [committed.count, reserves.count] {
            return Err("land role subset outside full participants");
        }
    }
    Ok(Visited::Complete)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ModifierContext {
    pub border_values: [Option<[i64; 3]>; 2],
}

pub fn read_modifiers_context(
    memory: &impl Memory,
    base: usize,
    source: Source,
) -> Result<ModifierContext, &'static str> {
    let reader = Reader {
        memory,
        base,
        provinces: source.provinces,
    };
    let border = reader
        .class(source.pointer)?
        .ok_or("land modifiers source is naval")?;
    let header = reader.header(source.pointer, border)?;
    if header != (source.id, source.attackers, source.defenders) {
        return Err("land modifiers source changed");
    }
    let mut row = ModifierContext::default();
    for (side, offset) in [ATTACKER, DEFENDER].into_iter().enumerate() {
        let pointer = reader.pointer(source.pointer, offset)?;
        if border {
            row.border_values[side] = Some([
                reader.field(pointer, cland_combatant::BORDER_MODIFIER)?,
                reader.field(pointer, cland_combatant::BORDER_DIG_IN_FACTOR)?,
                reader.field(pointer, cland_combatant::BORDER_TERRAIN_FACTOR)?,
            ]);
        }
    }
    Ok(row)
}

pub fn visit_sides(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut visit: impl FnMut([usize; 2]) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let registry = game_state
        .checked_add(cgame_state::COMBATS)
        .and_then(|address| address.checked_add(8))
        .and_then(|address| Array::read(memory, address, 8))
        .ok_or("combat registry unreadable or invalid")?;
    let reader = Reader {
        memory,
        base,
        provinces: Array::default(),
    };
    for index in 0..registry.count {
        let pointer = reader.element(registry, index)?;
        if let Some(border) = reader.class(pointer)? {
            let (_, _, _, sides) = reader.header_with_sides(pointer, border)?;
            visit(sides)?;
        }
    }
    Ok(())
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    participants: &mut Vec<u64>,
    classes: &mut Vec<u32>,
) -> Walk {
    walk_with(
        memory,
        base,
        game_state,
        output,
        participants,
        classes,
        |_| {},
    )
}

pub fn walk_with(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    participants: &mut Vec<u64>,
    classes: &mut Vec<u32>,
    mut visit: impl FnMut(Source),
) -> Walk {
    walk_members_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(output),
        engine_native_memory::walk::Buffer::Growing(participants),
        engine_native_memory::walk::Buffer::Count,
        Some(engine_native_memory::walk::Buffer::Growing(classes)),
        |source, _| visit(source),
    )
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CoreDemand {
    pub rows: usize,
    pub participants: usize,
}

pub fn core_demand(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<CoreDemand, &'static str> {
    let registry = game_state
        .checked_add(cgame_state::COMBATS + 8)
        .and_then(|address| Array::read(memory, address, 8))
        .ok_or("combat registry unreadable or invalid")?;
    let reader = Reader {
        memory,
        base,
        provinces: Array::default(),
    };
    let mut demand = CoreDemand {
        rows: registry.count as usize,
        ..Default::default()
    };
    for index in 0..registry.count {
        let pointer = reader.element(registry, index)?;
        let Some(border) = reader.class(pointer)? else {
            continue;
        };
        let (_, attackers, defenders, _) = reader.header_with_sides(pointer, border)?;
        for units in [attackers, defenders] {
            demand.participants = demand
                .participants
                .checked_add(units.count as usize)
                .ok_or("land combat participant demand overflow")?;
        }
    }
    Ok(demand)
}

pub fn walk_members(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    participants: &mut Vec<u64>,
    pointers: &mut Vec<usize>,
    visit: impl FnMut(Source, Members<'_>),
) -> Walk {
    walk_members_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(output),
        engine_native_memory::walk::Buffer::Growing(participants),
        engine_native_memory::walk::Buffer::Growing(pointers),
        None,
        visit,
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "Fixed and growing entry points share the same walker and caller-owned buffers."
)]
fn walk_members_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
    mut participants: engine_native_memory::walk::Buffer<'_, u64>,
    mut pointers: engine_native_memory::walk::Buffer<'_, usize>,
    mut classes: Option<engine_native_memory::walk::Buffer<'_, u32>>,
    mut visit: impl FnMut(Source, Members<'_>),
) -> Walk {
    let keep = pointers.is_growing()
        || (!participants.is_growing() && pointers.len() >= participants.len());
    let result = (|| {
        let registry = game_state
            .checked_add(cgame_state::COMBATS)
            .and_then(|address| address.checked_add(8))
            .and_then(|address| Array::read(memory, address, 8))
            .ok_or("combat registry unreadable or invalid")?;
        output.ensure(registry.count as usize)?;
        let provinces = game_state
            .checked_add(cgame_state::PROVINCES)
            .and_then(|address| Array::read(memory, address, 8))
            .ok_or("province array unreadable or invalid")?;
        let reader = Reader {
            memory,
            base,
            provinces,
        };
        let mut len = 0;
        let mut participants_len = 0_usize;
        let mut required_participants = 0_usize;
        let mut unread_rows = 0_u32;
        for index in 0..registry.count {
            let pointer = reader.element(registry, index)?;
            let Some(border_war) = reader.class(pointer)? else {
                continue;
            };
            let (id, attacker, defender, sides) = reader.header_with_sides(pointer, border_war)?;
            let count = (attacker.count as usize)
                .checked_add(defender.count as usize)
                .ok_or("land combat participant count overflow")?;
            required_participants = required_participants
                .checked_add(count)
                .ok_or("land combat participant count overflow")?;
            participants.ensure(required_participants)?;
            if keep {
                pointers.ensure(required_participants)?;
            }
            let room = match &mut classes {
                Some(classes) => {
                    classes.ensure(required_participants)?;
                    participants.len().min(classes.len())
                }
                None => participants.len(),
            };
            if len == output.len() || count > room - participants_len {
                unread_rows = unread_rows
                    .checked_add(1)
                    .ok_or("land combat unread row count overflow")?;
                continue;
            }
            let mut row = reader.row(pointer, id)?;
            row.attacker_start = participants_len;
            row.attacker_len = attacker.count as usize;
            row.defender_start = participants_len + row.attacker_len;
            row.defender_len = defender.count as usize;
            for (source, start) in [
                (attacker, row.attacker_start),
                (defender, row.defender_start),
            ] {
                for member in 0..source.count {
                    let unit = reader.element(source, member)?;
                    participants[start + member as usize] = match &mut classes {
                        Some(classes) => {
                            let (handle, class) = reader.unit(unit)?;
                            classes[start + member as usize] = class;
                            handle
                        }
                        None => reader.division(unit)?,
                    };
                    if keep {
                        pointers[start + member as usize] = unit;
                    }
                }
            }
            output[len] = row;
            let span = participants_len..participants_len + count;
            visit(
                Source {
                    pointer,
                    id,
                    sides,
                    attackers: attacker,
                    defenders: defender,
                    provinces,
                },
                if keep {
                    Members {
                        pointers: &pointers[span.clone()],
                        handles: &participants[span],
                        attackers: attacker.count as usize,
                    }
                } else {
                    Members::none()
                },
            );
            len += 1;
            participants_len += count;
        }
        Ok(Walk {
            outcome: if unread_rows == 0 {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated { len, unread_rows }
            },
            participants_len,
            required_participants,
        })
    })();
    result.unwrap_or_else(|reason| Walk {
        outcome: Outcome::Unavailable { reason },
        participants_len: 0,
        required_participants: 0,
    })
}

pub struct Reader<'a, M> {
    pub memory: &'a M,
    pub base: usize,
    pub provinces: Array,
}

impl<M: Memory> Reader<'_, M> {
    pub fn participant(
        &self,
        members: &Members<'_>,
        side: usize,
        units: Array,
        index: u32,
    ) -> Result<(usize, u64), &'static str> {
        if let Some(found) = members.at(side, units, index) {
            return Ok(found);
        }
        let unit = self.element(units, index)?;
        Ok((unit, self.division(unit)?))
    }

    pub fn known_unit(
        &self,
        members: &Members<'_>,
        side: usize,
        pointer: usize,
        hint: &mut usize,
    ) -> Result<u64, &'static str> {
        match members.handle(side, pointer, hint) {
            Some(handle) => Ok(handle),
            None => self.division(pointer),
        }
    }

    pub fn array(&self, pointer: usize, offset: usize, stride: usize) -> Result<Array, &'static str> {
        pointer
            .checked_add(offset)
            .and_then(|address| Array::read(self.memory, address, stride))
            .ok_or("land detail array unreadable or invalid")
    }

    pub fn province_id(&self, pointer: usize) -> Result<u32, &'static str> {
        self.vtable(pointer, 0, cprovince::VTABLE_RVA)?;
        let id = self.field(pointer, cprovince::ID)?;
        if id == 0 || self.element(self.provinces, id)? != pointer {
            return Err("land detail province identity mismatch");
        }
        Ok(id)
    }
    pub fn field<T: Raw>(&self, pointer: usize, offset: usize) -> Result<T, &'static str> {
        pointer
            .checked_add(offset)
            .and_then(|address| read(self.memory, address))
            .ok_or("land combat field unreadable or address overflow")
    }

    pub fn pointer(&self, pointer: usize, offset: usize) -> Result<usize, &'static str> {
        usize::try_from(self.field::<u64>(pointer, offset)?)
            .map_err(|_| "land combat pointer overflow")
    }

    pub fn element(&self, array: Array, index: u32) -> Result<usize, &'static str> {
        array
            .element(index, 8)
            .and_then(|address| read::<u64>(self.memory, address))
            .and_then(|pointer| usize::try_from(pointer).ok())
            .filter(|pointer| *pointer != 0)
            .ok_or("combat roster pointer unreadable or null")
    }

    pub fn vtable(&self, pointer: usize, offset: usize, key: Rva) -> Result<(), &'static str> {
        let expected = engine_sites::at(self.base, key).ok_or("module address overflow")?;
        if self.field::<u64>(pointer, offset)? == expected as u64 {
            Ok(())
        } else {
            Err("land combat object vtable mismatch")
        }
    }

    pub fn class(&self, pointer: usize) -> Result<Option<bool>, &'static str> {
        let vtable = self.field::<u64>(pointer, 0)?;
        let vtable =
            usize::try_from(vtable).map_err(|_| "combat registry object vtable mismatch")?;
        let classes = [
            VTABLE_RVA,
            cland_border_war_combat::VTABLE_RVA,
            cnaval_combat::VTABLE_RVA,
        ];
        match engine_sites::which(self.base, vtable, &classes) {
            Some(VTABLE_RVA) => Ok(Some(false)),
            Some(cland_border_war_combat::VTABLE_RVA) => Ok(Some(true)),
            Some(cnaval_combat::VTABLE_RVA) => Ok(None),
            _ => Err("unclassified combat registry object"),
        }
    }

    pub fn header(
        &self,
        pointer: usize,
        border_war: bool,
    ) -> Result<(u64, Array, Array), &'static str> {
        self.header_with_sides(pointer, border_war)
            .map(|(id, a, d, _)| (id, a, d))
    }

    pub fn header_with_sides(
        &self,
        pointer: usize,
        border_war: bool,
    ) -> Result<(u64, Array, Array, [usize; 2]), &'static str> {
        let identity = self.field::<[u8; 16]>(pointer, BODY)?;
        let secondary = u64::from_le_bytes(identity[..8].try_into().unwrap());
        let expected = engine_sites::at(
            self.base,
            if border_war {
                cland_border_war_combat::BODY_VTABLE_RVA
            } else {
                BODY_VTABLE_RVA
            },
        )
        .ok_or("module address overflow")?;
        if secondary != expected as u64 {
            return Err("land combat object vtable mismatch");
        }
        let id = u64::from_le_bytes(identity[8..].try_into().unwrap());
        if id == 0 || (!border_war && id as u32 != TYPE_WORD) {
            return Err("land combat handle invalid");
        }
        let sides = self.field::<[u8; 16]>(pointer, ATTACKER)?;
        let attacker = usize::try_from(u64::from_le_bytes(sides[..8].try_into().unwrap()))
            .map_err(|_| "land combat pointer overflow")?;
        let defender = usize::try_from(u64::from_le_bytes(sides[8..].try_into().unwrap()))
            .map_err(|_| "land combat pointer overflow")?;
        if attacker == defender {
            return Err("land combat sides are the same object");
        }
        Ok((
            id,
            self.side(attacker, pointer, border_war, 1)?,
            self.side(defender, pointer, border_war, 0)?,
            [attacker, defender],
        ))
    }

    pub fn row(&self, pointer: usize, id: u64) -> Result<Row, &'static str> {
        let fields = self.field::<[u8; 16]>(pointer, PROVINCE)?;
        let province = usize::try_from(u64::from_le_bytes(fields[..8].try_into().unwrap()))
            .map_err(|_| "land combat pointer overflow")?;
        self.vtable(province, 0, cprovince::VTABLE_RVA)?;
        let province_id = self.field::<u32>(province, cprovince::ID)?;
        if province_id == 0 || self.element(self.provinces, province_id)? != province {
            return Err("land combat province differs from registered slot");
        }
        Ok(Row {
            id,
            province: province_id,
            day: i32::from_le_bytes(fields[8..12].try_into().unwrap()),
            duration: i32::from_le_bytes(fields[12..].try_into().unwrap()),
            ..Row::default()
        })
    }

    pub fn side(
        &self,
        pointer: usize,
        combat: usize,
        border_war: bool,
        attacker: u8,
    ) -> Result<Array, &'static str> {
        self.vtable(
            pointer,
            0,
            if border_war {
                cland_border_war_combatant::VTABLE_RVA
            } else {
                cland_combatant::VTABLE_RVA
            },
        )?;
        if self.pointer(pointer, cland_combatant::COMBAT)? != combat {
            return Err("land combat side parent mismatch");
        }
        if self.field::<u8>(pointer, cland_combatant::IS_ATTACKER)? != attacker {
            return Err("land combat side discriminator mismatch");
        }
        pointer
            .checked_add(cland_combatant::UNITS)
            .and_then(|address| Array::read(self.memory, address, 8))
            .ok_or("land combat participant vector unreadable or invalid")
    }

    fn unit(&self, pointer: usize) -> Result<(u64, u32), &'static str> {
        let mut primary = [0; 12];
        let mut secondary_and_handle = [0; 16];
        debug_assert_eq!(carmy::SECONDARY_BASE, ctask_force::SECONDARY_BASE);
        let secondary = pointer
            .checked_add(carmy::SECONDARY_BASE)
            .ok_or("land combat participant address overflow")?;
        if self.memory.copy_scattered(
            [pointer, secondary],
            &mut [&mut primary, &mut secondary_and_handle],
        ) != 2
        {
            return Err("land combat participant identity unreadable");
        }
        debug_assert_eq!(cunit::CLASS, 8);
        let class = u32::from_le_bytes(primary[8..].try_into().unwrap());
        let (primary_rva, secondary_rva) = match class {
            DIVISION_CLASS => (carmy::VTABLE_RVA, carmy::SECONDARY_VTABLE_RVA),
            TASK_FORCE_CLASS => (ctask_force::VTABLE_RVA, ctask_force::SECONDARY_VTABLE_RVA),
            RAILWAY_GUN_CLASS => (crailway_gun::VTABLE_RVA, crailway_gun::SECONDARY_VTABLE_RVA),
            _ => return Err("land combat participant has an unknown unit class"),
        };
        let primary = u64::from_le_bytes(primary[..8].try_into().unwrap());
        let secondary = u64::from_le_bytes(secondary_and_handle[..8].try_into().unwrap());
        if engine_sites::at(self.base, primary_rva).map(|p| p as u64) != Some(primary)
            || engine_sites::at(self.base, secondary_rva).map(|p| p as u64) != Some(secondary)
        {
            return Err("land combat participant vtable mismatch");
        }
        let id = u64::from_le_bytes(secondary_and_handle[8..].try_into().unwrap());
        if id == 0 {
            Err("land combat participant has null handle")
        } else {
            debug_assert_eq!(cunit::HANDLE, carmy::SECONDARY_BASE + 8);
            Ok((id, class))
        }
    }

    pub fn division(&self, pointer: usize) -> Result<u64, &'static str> {
        match self.unit(pointer)? {
            (handle, DIVISION_CLASS) => Ok(handle),
            _ => Err("land combat participant is not a division"),
        }
    }
}
