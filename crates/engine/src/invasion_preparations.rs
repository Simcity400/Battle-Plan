//! Naval-invasion preparation operands and stored accumulated preparation.
//! Required hours are computed on demand rather than stored. This reader
//! records country, target relation, selected HQ leader and content-loaded
//! preparation terms without calling the game.
//!
//! The target is the first path province's state controller, or zero without
//! a path or state. Country preparation speed is always selected. Relation
//! preparation speed requires a positive target distinct under country aliases;
//! the relation-day term uses any present row, including reserved slot zero.
//! Leader speed requires both leader and HQ provider, and the provider's root
//! order must be this invasion. The country's preparation-day term remains separate.
//! Fixed5 scaling, truncation to days and the final hour conversion retain their order.
//!
//! Root-order selection uses the first primary order assigned to the member,
//! searching its own group before its parent. A disagreeing cache is not
//! simulation state. HQ providers are embedded unit members; their primary
//! unit accessors and member vtables authenticate that relationship. The
//! selected leader accumulator is independently scaled by the content-loaded
//! HQ communication factor.
//!
//! Route-cost inputs retain the chosen unit owner's reserved relation and
//! nonzero co-belligerent target slots. Candidates select reads without supplying
//! predicate bytes; other countries' bytes are not read. The country census
//! supplies the first division handle for the final unit-selection fallback.

use crate::{
    Memory, carmy_leader, ccountry, cdiplomacy_status, cgame_state, corder_instance, corders_group,
    corders_group_member,
    cpdx_array::Array,
    cprovince, crelation_status, cstate, ctheatre, cunit, modifier_accumulator,
    modifier_definition::{DefinitionMemo, effective_memo},
    modifier_keys,
    walk::Outcome,
};

/// A CModifier's accumulator, `+0x10` into it.
const MODIFIER_ACCUMULATOR: usize = 0x10;
/// Selected HQ leader modifier accumulator.
pub const LEADER_ACCUMULATOR: usize = carmy_leader::MOVEMENT_MODIFIER + MODIFIER_ACCUMULATOR;
/// Selected relation modifier accumulator.
pub const RELATION_ACCUMULATOR: usize = crelation_status::MODIFIER + MODIFIER_ACCUMULATOR;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// The owning group's handle and the order's instance id, as ArmyOrders.
    pub owner_group: u64,
    pub instance: u32,
    /// `COrderInstance+0xD8`, the accumulated preparation.
    pub progress: i64,
    /// `T`, the first path province's state controller; zero without a path
    /// or a state.
    pub target: u32,
    /// Key 0x1A on the country.
    pub country_term: i64,
    /// Preparation-speed key on the target relation when the target gate admits it.
    pub relation_term: Option<i64>,
    /// Key 0x1CE on the relation with `T`, when that row exists.
    pub relation_bonus: Option<i64>,
    /// Key 0x1A on the leader, when the group has a leader and an HQ provider.
    pub leader_term: Option<i64>,
    /// The HQ provider's root order is this order: the leader term counts.
    pub leader_applies: u8,
    /// Key 0x184 on the country.
    pub days_term: i64,
    /// Raw `CUnit+0x1E0` owner of the route unit, zero without one.
    pub unit_owner: u32,
    /// That owner's slot-0 relation operands.
    pub zero_relation: crelation_status::ZeroRelation,
    /// Span of that owner's nonzero co-belligerent target slots, ascending.
    pub co_belligerent_start: usize,
    pub co_belligerent_len: usize,
}

impl Row {
    pub fn key(self) -> u128 {
        (u128::from(self.owner_group) << 32) | u128::from(self.instance)
    }
}

fn field<T: crate::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|at| crate::read::<T>(memory, at))
        .ok_or("invasion preparation input unreadable")
}

fn pointer(memory: &impl Memory, at: usize, offset: usize) -> Result<usize, &'static str> {
    field::<u64>(memory, at, offset).map(|word| word as usize)
}

fn array(
    memory: &impl Memory,
    at: usize,
    offset: usize,
    stride: usize,
) -> Result<Array, &'static str> {
    at.checked_add(offset)
        .and_then(|address| Array::read(memory, address, stride))
        .ok_or("invasion preparation array unreadable or invalid")
}

fn authenticate(
    memory: &impl Memory,
    base: usize,
    object: usize,
    vtable: usize,
) -> Result<(), &'static str> {
    if object == 0
        || crate::read::<u64>(memory, object) != base.checked_add(vtable).map(|v| v as u64)
    {
        return Err("invasion preparation object identity mismatch");
    }
    Ok(())
}

fn effective(
    memory: &impl Memory,
    base: usize,
    accumulator: usize,
    key: u32,
    memo: &DefinitionMemo,
) -> Result<i64, &'static str> {
    let value = modifier_accumulator::lookup_at(memory, accumulator, key)?;
    if !value.present {
        return Ok(0);
    }
    effective_memo(memory, base, key, value.raw, memo)
}

/// Province table used by invasion-path lookup.
pub fn provinces(memory: &impl Memory, game_state: usize) -> Result<Array, &'static str> {
    array(memory, game_state, cgame_state::PROVINCES, 8)
}

/// First path province's state controller, or zero.
fn target(
    memory: &impl Memory,
    base: usize,
    provinces: Array,
    order: usize,
) -> Result<u32, &'static str> {
    let path = array(memory, order, corder_instance::PATH_DATA, 4)?;
    if path.count == 0 {
        return Ok(0);
    }
    let id: u32 = field(memory, path.data, 0)?;
    // Path lookup requires a positive in-range province ID. An out-of-range
    // dereference is refused rather than guessed.
    let province = if id == 0 {
        None
    } else {
        provinces.element(id, 8)
    }
    .ok_or("invasion path province outside the province table")?;
    let province = pointer(memory, province, 0)?;
    authenticate(memory, base, province, cprovince::VTABLE_RVA)?;
    if field::<u32>(memory, province, cprovince::ID)? != id {
        return Err("invasion path province identity mismatch");
    }
    let state = pointer(memory, province, cprovince::STATE)?;
    if state == 0 {
        return Ok(0);
    }
    authenticate(memory, base, state, cstate::VTABLE_RVA)?;
    field(memory, state, cstate::CONTROLLER)
}

/// First primary order of `group` assigned to `member`.
fn first_assigned(
    memory: &impl Memory,
    group: usize,
    member: usize,
) -> Result<Option<usize>, &'static str> {
    let orders = array(memory, group, corders_group::PRIMARY_ORDER_DATA, 8)?;
    let mut words = [0_u8; 64 * 8];
    for index in 0..orders.count {
        let order = pointer(
            memory,
            orders.element(index, 8).ok_or("group order overflow")?,
            0,
        )?;
        if order == 0 {
            return Err("group order pointer null");
        }
        let members = array(memory, order, corder_instance::ASSIGNED_MEMBER_DATA, 8)?;
        for first in (0..members.count).step_by(64) {
            let count = (members.count - first).min(64) as usize;
            let address = members
                .element(first, 8)
                .ok_or("assigned members overflow")?;
            if !crate::read_slice(memory, address, &mut words[..count * 8]) {
                return Err("assigned members unreadable");
            }
            if words[..count * 8]
                .chunks_exact(8)
                .any(|word| u64::from_le_bytes(word.try_into().unwrap()) == member as u64)
            {
                return Ok(Some(order));
            }
        }
    }
    Ok(None)
}

/// Root order without cache reuse: first assigned primary order of the
/// member's group, then its parent.
fn root_order(memory: &impl Memory, member: usize) -> Result<Option<usize>, &'static str> {
    let group = pointer(memory, member, corders_group_member::GROUP)?;
    if group == 0 {
        return Ok(None);
    }
    if let Some(order) = first_assigned(memory, group, member)? {
        return Ok(Some(order));
    }
    let parent = pointer(memory, group, corders_group::PARENT)?;
    if parent == 0 {
        return Ok(None);
    }
    first_assigned(memory, parent, member)
}

/// `C`'s diplomacy row at `slot`, or zero.
fn relation(
    memory: &impl Memory,
    base: usize,
    country: usize,
    slot: u32,
) -> Result<usize, &'static str> {
    let diplomacy = pointer(memory, country, ccountry::DIPLOMACY_STATUS)?;
    authenticate(memory, base, diplomacy, cdiplomacy_status::VTABLE_RVA)?;
    let rows = array(memory, diplomacy, cdiplomacy_status::RELATIONS, 8)?;
    let row = pointer(
        memory,
        rows.element(slot, 8)
            .ok_or("relation slot outside the diplomacy matrix")?,
        0,
    )?;
    if row != 0 {
        authenticate(memory, base, row, crelation_status::VTABLE_RVA)?;
    }
    Ok(row)
}

/// One order's row; `None` for any order that is not a naval invasion.
pub fn read(
    memory: &impl Memory,
    base: usize,
    countries: &ccountry::NativeReferenceContext<'_, impl Memory>,
    provinces: Array,
    order: usize,
    memo: &DefinitionMemo,
) -> Result<Option<Row>, &'static str> {
    if field::<i32>(memory, order, corder_instance::TYPE)? != corder_instance::TYPE_INVASION {
        return Ok(None);
    }
    read_invasion(memory, base, countries, provinces, order, memo).map(Some)
}

fn read_invasion(
    memory: &impl Memory,
    base: usize,
    countries: &ccountry::NativeReferenceContext<'_, impl Memory>,
    provinces: Array,
    order: usize,
    memo: &DefinitionMemo,
) -> Result<Row, &'static str> {
    let group = pointer(memory, order, corder_instance::OWNER)?;
    if group == 0 {
        return Err("invasion order has no group");
    }
    let country = countries.resolve(field(memory, group, corders_group::COUNTRY)?)?;
    let accumulator = country
        .pointer
        .checked_add(ccountry::MODIFIER_ACCUMULATOR)
        .ok_or("invasion country address overflow")?;
    let mut row = Row {
        owner_group: field(memory, group, corders_group::HANDLE)?,
        instance: field(memory, order, corder_instance::ID)?,
        progress: field(memory, order, corder_instance::ACCUMULATED_PREPARATION)?,
        target: target(memory, base, provinces, order)?,
        country_term: effective(
            memory,
            base,
            accumulator,
            modifier_keys::INVASION_PREPARATION,
            memo,
        )?,
        days_term: effective(
            memory,
            base,
            accumulator,
            modifier_keys::NAVAL_INVASION_PREP_DAYS,
            memo,
        )?,
        ..Row::default()
    };
    let target = row.target;
    // A positive target resolves through its country link. Zero targets
    // select reserved slot zero without reading that link.
    let slot = countries.canonical_slot(target)?;
    let relation = relation(memory, base, country.pointer, slot)?;
    let relation_accumulator = || {
        relation
            .checked_add(RELATION_ACCUMULATOR)
            .ok_or("invasion relation address overflow")
    };
    if (target as i32) > 0
        && target != country.id
        && (country.id == 0 || slot != countries.canonical_slot(country.id)?)
        && relation != 0
    {
        row.relation_term = Some(effective(
            memory,
            base,
            relation_accumulator()?,
            modifier_keys::INVASION_PREPARATION,
            memo,
        )?);
    }
    if relation != 0 {
        row.relation_bonus = Some(effective(
            memory,
            base,
            relation_accumulator()?,
            modifier_keys::INVASION_PREPARATION_AGAINST,
            memo,
        )?);
    }
    let leader = pointer(memory, group, corders_group::LEADER)?;
    let provider = pointer(memory, group, corders_group::HQ_PROVIDER)?;
    if leader != 0 && provider != 0 {
        row.leader_term = Some(effective(
            memory,
            base,
            leader
                .checked_add(LEADER_ACCUMULATOR)
                .ok_or("invasion leader address overflow")?,
            modifier_keys::INVASION_PREPARATION,
            memo,
        )?);
        row.leader_applies = u8::from(root_order(memory, provider)? == Some(order));
    }
    Ok(row)
}

/// Route unit: the first assigned member, then the group's first member,
/// then the request country's first division. Member access authenticates its
/// enclosing unit. A positive theatre volunteer reference selects the request
/// country; otherwise use its owner. The game reaches these only when both
/// port seas resolve; this reader records them for every invasion.
fn route_unit(
    memory: &impl Memory,
    countries: &ccountry::NativeReferenceContext<'_, impl Memory>,
    order: usize,
    group: usize,
) -> Result<usize, &'static str> {
    for (at, offset) in [
        (order, corder_instance::ASSIGNED_MEMBER_DATA),
        (group, corders_group::MEMBER_DATA),
    ] {
        let members = array(memory, at, offset, 8)?;
        if members.count != 0 {
            let unit = pointer(memory, members.data, 0)?
                .checked_sub(cunit::SCHEDULED_MEMBER)
                .ok_or("invasion unit member pointer invalid")?;
            if unit != 0 {
                return Ok(unit);
            }
        }
    }
    let theatre = pointer(memory, group, corders_group::THEATRE)?;
    if theatre == 0 {
        return Err("invasion group has no theatre");
    }
    let volunteers: i32 = field(memory, theatre, ctheatre::VOLUNTEERS_THEATRE)?;
    let raw = if volunteers >= 1 {
        volunteers as u32
    } else {
        field(memory, theatre, ctheatre::OWNER)?
    };
    let roster = array(
        memory,
        countries.resolve(raw)?.pointer,
        ccountry::DIVISIONS,
        8,
    )?;
    if roster.count == 0 {
        return Ok(0);
    }
    pointer(memory, roster.data, 0)
}

/// What [`walk`] wrote: its rows and the co-belligerent slots they span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub outcome: Outcome,
    pub slots: usize,
    pub required_slots: usize,
}

/// Every naval invasion of a complete ArmyOrders census of this pass, in
/// census order. The province table is read only once an invasion is found.
/// Country references share the pass's lazy native reference context: raw
/// zero never requires the positive country/link headers. A row that does not
/// fit is counted; any failure makes
/// the table unavailable. An owner's co-belligerent slots are read once per
/// walk; a row whose slots do not fit is counted with the demand.
#[allow(clippy::too_many_arguments)]
pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    visits: &[corder_instance::Visit],
    source: Outcome,
    memo: &DefinitionMemo,
    countries: &ccountry::NativeReferenceContext<'_, impl Memory>,
    output: &mut [Row],
    slots: &mut [u32],
) -> Walk {
    walk_buffer(
        memory,
        base,
        game_state,
        visits,
        source,
        memo,
        countries,
        crate::walk::Buffer::Fixed(output),
        crate::walk::Buffer::Fixed(slots),
    )
}

/// Worker acquisition follows the complete order census and original route
/// operands without publishing a capacity-short hour.
#[allow(clippy::too_many_arguments)]
pub fn walk_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    visits: &[corder_instance::Visit],
    source: Outcome,
    memo: &DefinitionMemo,
    countries: &ccountry::NativeReferenceContext<'_, impl Memory>,
    output: &mut Vec<Row>,
    slots: &mut Vec<u32>,
) -> Walk {
    walk_buffer(
        memory,
        base,
        game_state,
        visits,
        source,
        memo,
        countries,
        crate::walk::Buffer::Growing(output),
        crate::walk::Buffer::Growing(slots),
    )
}

#[allow(clippy::too_many_arguments)]
fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    visits: &[corder_instance::Visit],
    source: Outcome,
    memo: &DefinitionMemo,
    countries: &ccountry::NativeReferenceContext<'_, impl Memory>,
    mut output: crate::walk::Buffer<'_, Row>,
    mut slots: crate::walk::Buffer<'_, u32>,
) -> Walk {
    let (mut used, mut required_slots) = (0, 0);
    let result = (|| {
        let Outcome::Rows { len: expected } = source else {
            return Err("invasion preparations require a complete army order census");
        };
        output.ensure(expected)?;
        let mut context = None;
        let (mut len, mut unread, mut seen) = (0, 0_u32, 0);
        for visit in visits.iter().filter(|visit| visit.pointer != 0) {
            seen += 1;
            let order = visit.pointer;
            if field::<i32>(memory, order, corder_instance::TYPE)? != corder_instance::TYPE_INVASION
            {
                continue;
            }
            if context.is_none() {
                context = Some(provinces(memory, game_state)?);
            }
            let Some(table) = context else {
                unreachable!("the context was just read");
            };
            let mut row = read_invasion(memory, base, countries, table, order, memo)?;
            let group = pointer(memory, order, corder_instance::OWNER)?;
            let unit = route_unit(memory, countries, order, group)?;
            let mut fits = true;
            if unit != 0 {
                row.unit_owner = field(memory, unit, cunit::LOGICAL_OWNER)?;
                if let Some(earlier) = output[..len]
                    .iter()
                    .find(|earlier| earlier.unit_owner == row.unit_owner)
                {
                    row.zero_relation = earlier.zero_relation;
                    row.co_belligerent_start = earlier.co_belligerent_start;
                    row.co_belligerent_len = earlier.co_belligerent_len;
                } else {
                    // Resolve the selected country from its already computed canonical slot.
                    let owner = countries.resolve(row.unit_owner)?;
                    row.zero_relation = crelation_status::zero_relation(memory, base, owner)?;
                    let free = slots.suffix(used);
                    let (written, needed) = crelation_status::co_belligerents_native_buffer(
                        memory, base, countries, owner, free,
                    )?;
                    required_slots += needed;
                    fits = written == needed;
                    if fits {
                        row.co_belligerent_start = used;
                        row.co_belligerent_len = written;
                        used += written;
                    }
                }
            }
            match output.get_mut(len).filter(|_| fits) {
                Some(slot) => {
                    *slot = row;
                    len += 1;
                }
                None => {
                    unread = unread
                        .checked_add(1)
                        .ok_or("invasion unread row count overflow")?;
                }
            }
        }
        if seen != expected {
            return Err("invasion source census count mismatch");
        }
        Ok(if unread == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: unread,
            }
        })
    })();
    Walk {
        outcome: result.unwrap_or_else(|reason| Outcome::Unavailable { reason }),
        slots: used,
        required_slots: required_slots.max(used),
    }
}

#[cfg(test)]
mod tests;
