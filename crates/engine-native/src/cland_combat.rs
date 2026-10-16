//! CLandCombat complete-object layout. All offsets use its primary base; the secondary base is
//! `BODY`. The location is a province pointer. The walker captures two ordered participant vectors
//! without event-hook or presentation data.

use crate::{
    Memory, carmy, cgame_state, cland_border_war_combat, cland_border_war_combatant,
    cland_combatant, cnaval_combat, cpdx_array::Array, cprovince, cunit, memory::Raw, read,
    walk::Outcome,
};

mod country_sets;
mod demand;
mod details;

pub use country_sets::{
    CountrySetError, CountrySetsRow, read_country_sets, read_country_sets_buffer,
};
pub use demand::{DetailDemand, detail_demand};
pub use details::{
    BorderWarInputs, CommanderLearningTrait, CommanderSource, CommanderTimedTrait, CommanderTrait,
    Commanders, DetailArenas, DetailCounts, DetailsRow, Leaders, SKILL_HOVER_KEYS, SideDetails,
    Span, commander_sources, read_details, read_details_sharing,
};

/// CLandCombat primary vtable.
pub const VTABLE_RVA: usize = 0x0299_14d8;
/// CLandCombat secondary vtable (`BODY`).
pub const BODY_VTABLE_RVA: usize = 0x0299_15b8;
/// CLandCombat secondary-base displacement.
pub const BODY: usize = 0x10;
/// Packed full u64 handle: low u32 type and high u32 id, in the reference base.
pub const HANDLE: usize = 0x18;
/// Land-combat u32 reference type.
pub const TYPE: usize = 0x18;
/// Ordinary CLandCombat type word.
pub const TYPE_WORD: u32 = 62;
/// Land-combat u32 reference id.
pub const ID: usize = 0x1c;
/// Attacking combat-side pointer.
pub const ATTACKER: usize = 0x28;
/// Defending combat-side pointer.
pub const DEFENDER: usize = 0x30;
/// Location province pointer; history uses its u32 id.
pub const PROVINCE: usize = 0x38;
/// Signed round counter, initially -1 and incremented on nonterminating
/// rounds. This is neither elapsed days nor an absolute start date.
pub const DAY: usize = 0x40;
/// CLandCombat signed duration field.
pub const DURATION: usize = 0x44;
/// Stored combat terrain, distinct from province-definition terrain.
pub const STORED_TERRAIN: usize = 0x48;
/// Land-indicator byte, not a replay frame id.
pub const INDICATOR_FRAME_HIGH: usize = 0x98;
/// CLandCombat attacker bonus, i64 fixed point /100000.
pub const ATTACKER_BONUS: usize = 0xa8;
/// CLandCombat defender bonus, i64 fixed point /100000.
pub const DEFENDER_BONUS: usize = 0xb0;
/// Signed fixed5 terrain-width factor, applied as one plus this value.
pub const WIDTH_FACTOR: usize = 0xa0;
/// Signed tactic-phase dictionary index.
pub const PHASE: usize = 0xd0;
/// CLandCombat progress getter vtable slot.
pub const PROGRESS_VTABLE_SLOT: usize = 0xc8;
/// Combat progress getter uses both side organisation sums.
pub const PROGRESS_RVA: usize = 0x013c_cb10;
/// Combat province getter returns the stored location pointer.
pub const PROVINCE_GETTER_RVA: usize = 0x00f1_c390;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Complete native reference: low u32 type, high u32 id; no collector generation.
    pub id: u64,
    pub province: u32,
    /// Native counters, without converting either one to elapsed days.
    pub duration: i32,
    pub day: i32,
    /// Ranges of complete CArmy reference handles in the caller's participant arena.
    pub attacker_start: usize,
    pub attacker_len: usize,
    pub defender_start: usize,
    pub defender_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub outcome: Outcome,
    pub participants_len: usize,
    /// Sum of both native vector lengths, including whole rows omitted for capacity.
    pub required_participants: usize,
}

/// Transient authenticated census inputs, valid only inside the caller's walk.
#[derive(Clone, Copy)]
pub struct Source {
    /// Authenticated complete object, transient and never serialized.
    pub pointer: usize,
    pub id: u64,
    /// Authenticated attacker/defender pointers, reused by hourly loss reads.
    pub sides: [usize; 2],
    pub attackers: Array,
    pub defenders: Array,
    pub provinces: Array,
}

/// The census's authenticated full participants of one combat: each element's
/// pointer and the handle `unit()` authenticated for it, attackers then
/// defenders. The hourly pass holds the simulation thread, so a later reader
/// of the same combat in the same pass would read these very words and pass
/// the same checks; it takes them instead. A reader given [`Members::none`],
/// or lists that do not match the side's count, reads them itself.
#[derive(Clone, Copy)]
pub struct Members<'a> {
    pub pointers: &'a [usize],
    pub handles: &'a [u64],
    /// Attackers are `[..attackers]`, defenders the rest.
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

    /// The census's participant at `index` of `side`, when it holds that side.
    fn at(&self, side: usize, units: Array, index: u32) -> Option<(usize, u64)> {
        let (pointers, handles) = self.side(side);
        if pointers.len() != units.count as usize || handles.len() != pointers.len() {
            return None;
        }
        Some((*pointers.get(index as usize)?, handles[index as usize]))
    }

    /// The handle the census authenticated for `pointer` on `side`, searching
    /// from `hint`: role lists usually follow the census order.
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

/// Outcome of [`visit_units`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visited {
    Complete,
    /// The role-subset scratch was too small for this combat's committed and
    /// reserve lists; no unit was visited. Grow it to `required` off-thread.
    Scratch {
        required: usize,
    },
}

/// Visit authenticated full participants once. Side 0 is attacker and side 1
/// defender. The province is the current native location, not movement target.
/// `scratch` holds one side's committed and reserve pointers; the live lists
/// are its only bound.
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
    /// Modifier, dig-in factor, terrain factor. None for ordinary land combat.
    pub border_values: [Option<[i64; 3]>; 2],
}

/// Side-icon operands are independent of terrain, tactic and CAS summary
/// fields. Flag-only icons use static text without a number.
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

/// Reads the mixed registry once, skipping only authenticated naval objects.
/// Off-thread sizing census: authenticate the same combat/side headers without
/// reading participant payloads or retaining native pointers after this call.
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

/// Reads the mixed registry once, skipping only authenticated naval objects.
/// A combat is retained only with both complete, ordered participant lists.
/// Capacity omits whole combats; the spool thread grows the row/participant pool.
/// No allocation, engine calls, retries or presentation reads occur here.
/// Duplicate combat handles and duplicate members must be rejected when converting
/// these rows off the game thread, as with the other native roster walkers.
pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [Row],
    participants: &mut [u64],
) -> Walk {
    walk_with(memory, base, game_state, output, participants, |_| {})
}

/// Visits only complete authenticated rows. A visitor owns its independent
/// failure/capacity result, and must also consume the returned parent outcome.
pub fn walk_with(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [Row],
    participants: &mut [u64],
    mut visit: impl FnMut(Source),
) -> Walk {
    walk_members(
        memory,
        base,
        game_state,
        output,
        participants,
        &mut [],
        |source, _| visit(source),
    )
}

/// Worker rows and participant handles grow from original registry and
/// side headers without retaining optional unit pointers.
pub fn walk_with_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    participants: &mut Vec<u64>,
    mut visit: impl FnMut(Source),
) -> Walk {
    walk_members_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(output),
        crate::walk::Buffer::Growing(participants),
        crate::walk::Buffer::Fixed(&mut []),
        |source, _| visit(source),
    )
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CoreDemand {
    pub rows: usize,
    pub participants: usize,
}

/// Authenticated header demand only; no participant or presentation payload.
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

/// [`walk_with`], also keeping each participant's pointer beside its handle
/// (`pointers` as long as `participants`) and lending the visitor the combat's
/// authenticated [`Members`]. Shorter pointer storage lends [`Members::none`].
pub fn walk_members(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [Row],
    participants: &mut [u64],
    pointers: &mut [usize],
    visit: impl FnMut(Source, Members<'_>),
) -> Walk {
    walk_members_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Fixed(output),
        crate::walk::Buffer::Fixed(participants),
        crate::walk::Buffer::Fixed(pointers),
        visit,
    )
}

/// Original registry and side counts bound worker destinations before
/// rows or participant payloads are acquired.
pub fn walk_members_growing(
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
        crate::walk::Buffer::Growing(output),
        crate::walk::Buffer::Growing(participants),
        crate::walk::Buffer::Growing(pointers),
        visit,
    )
}

fn walk_members_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: crate::walk::Buffer<'_, Row>,
    mut participants: crate::walk::Buffer<'_, u64>,
    mut pointers: crate::walk::Buffer<'_, usize>,
    mut visit: impl FnMut(Source, Members<'_>),
) -> Walk {
    let keep = pointers.is_growing()
        || (!participants.is_growing() && pointers.len() >= participants.len());
    let result = (|| {
        let registry = game_state
            .checked_add(cgame_state::COMBATS)
            // The mixed registry's CPdxArray begins after its eight-byte vptr.
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
            if len == output.len() || count > participants.len() - participants_len {
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
                    participants[start + member as usize] = reader.unit(unit)?;
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

struct Reader<'a, M> {
    memory: &'a M,
    base: usize,
    provinces: Array,
}

impl<M: Memory> Reader<'_, M> {
    /// The full participant at `index` of `side`'s `units`: the census's
    /// pointer and handle when it holds them, else read.
    fn participant(
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
        Ok((unit, self.unit(unit)?))
    }

    /// `unit(pointer)` for a division of `side`: the census's handle when it
    /// is one of that side's participants, else read.
    fn known_unit(
        &self,
        members: &Members<'_>,
        side: usize,
        pointer: usize,
        hint: &mut usize,
    ) -> Result<u64, &'static str> {
        match members.handle(side, pointer, hint) {
            Some(handle) => Ok(handle),
            None => self.unit(pointer),
        }
    }

    fn array(&self, pointer: usize, offset: usize, stride: usize) -> Result<Array, &'static str> {
        pointer
            .checked_add(offset)
            .and_then(|address| Array::read(self.memory, address, stride))
            .ok_or("land detail array unreadable or invalid")
    }

    fn province_id(&self, pointer: usize) -> Result<u32, &'static str> {
        self.vtable(pointer, 0, cprovince::VTABLE_RVA)?;
        let id = self.field(pointer, cprovince::ID)?;
        if id == 0 || self.element(self.provinces, id)? != pointer {
            return Err("land detail province identity mismatch");
        }
        Ok(id)
    }
    fn field<T: Raw>(&self, pointer: usize, offset: usize) -> Result<T, &'static str> {
        pointer
            .checked_add(offset)
            .and_then(|address| read(self.memory, address))
            .ok_or("land combat field unreadable or address overflow")
    }

    fn pointer(&self, pointer: usize, offset: usize) -> Result<usize, &'static str> {
        usize::try_from(self.field::<u64>(pointer, offset)?)
            .map_err(|_| "land combat pointer overflow")
    }

    fn element(&self, array: Array, index: u32) -> Result<usize, &'static str> {
        array
            .element(index, 8)
            .and_then(|address| read::<u64>(self.memory, address))
            .and_then(|pointer| usize::try_from(pointer).ok())
            .filter(|pointer| *pointer != 0)
            .ok_or("combat roster pointer unreadable or null")
    }

    fn vtable(&self, pointer: usize, offset: usize, rva: usize) -> Result<(), &'static str> {
        let expected = self
            .base
            .checked_add(rva)
            .ok_or("module address overflow")?;
        if self.field::<u64>(pointer, offset)? == expected as u64 {
            Ok(())
        } else {
            Err("land combat object vtable mismatch")
        }
    }

    fn class(&self, pointer: usize) -> Result<Option<bool>, &'static str> {
        let vtable = self.field::<u64>(pointer, 0)?;
        let rva = usize::try_from(vtable)
            .ok()
            .and_then(|vtable| vtable.checked_sub(self.base))
            .ok_or("combat registry object vtable mismatch")?;
        match rva {
            VTABLE_RVA => Ok(Some(false)),
            cland_border_war_combat::VTABLE_RVA => Ok(Some(true)),
            cnaval_combat::VTABLE_RVA => Ok(None),
            _ => Err("unclassified combat registry object"),
        }
    }

    fn header(
        &self,
        pointer: usize,
        border_war: bool,
    ) -> Result<(u64, Array, Array), &'static str> {
        self.header_with_sides(pointer, border_war)
            .map(|(id, a, d, _)| (id, a, d))
    }

    fn header_with_sides(
        &self,
        pointer: usize,
        border_war: bool,
    ) -> Result<(u64, Array, Array, [usize; 2]), &'static str> {
        // The secondary vptr and full reference occupy one exact 16-byte span.
        let identity = self.field::<[u8; 16]>(pointer, BODY)?;
        let secondary = u64::from_le_bytes(identity[..8].try_into().unwrap());
        let expected = self
            .base
            .checked_add(if border_war {
                cland_border_war_combat::BODY_VTABLE_RVA
            } else {
                BODY_VTABLE_RVA
            })
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

    fn row(&self, pointer: usize, id: u64) -> Result<Row, &'static str> {
        // Location pointer and two signed counters fill these 16 bytes exactly.
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

    fn side(
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

    fn unit(&self, pointer: usize) -> Result<u64, &'static str> {
        let mut primary = [0; 8];
        // Secondary vptr and handle occupy the exact contiguous 16 bytes.
        let mut secondary_and_handle = [0; 16];
        let secondary = pointer
            .checked_add(carmy::SECONDARY_BASE)
            .ok_or("land combat division address overflow")?;
        if self.memory.copy_scattered(
            [pointer, secondary],
            &mut [&mut primary, &mut secondary_and_handle],
        ) != 2
        {
            return Err("land combat division identity unreadable");
        }
        let primary = u64::from_le_bytes(primary);
        let secondary = u64::from_le_bytes(secondary_and_handle[..8].try_into().unwrap());
        if self.base.checked_add(carmy::VTABLE_RVA).map(|p| p as u64) != Some(primary)
            || self
                .base
                .checked_add(carmy::SECONDARY_VTABLE_RVA)
                .map(|p| p as u64)
                != Some(secondary)
        {
            return Err("land combat division vtable mismatch");
        }
        let id = u64::from_le_bytes(secondary_and_handle[8..].try_into().unwrap());
        if id == 0 {
            Err("land combat division has null handle")
        } else {
            // cunit::HANDLE follows the secondary base's vtable; both words remain native.
            debug_assert_eq!(cunit::HANDLE, carmy::SECONDARY_BASE + 8);
            Ok(id)
        }
    }
}

#[cfg(test)]
mod tests;
