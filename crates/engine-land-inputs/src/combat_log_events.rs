use crate::combat_log_history as layout;
use engine_native_memory::{Memory, cpdx_array::Array, read};
use engine_native_parts::equipment_entry;
use crate::{ncombat_log_cequipment_loss as loss, ncombat_log_corders_group_logs as group};
use engine_sites::Rva;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fact {
    pub kind: u64,
    pub root: u64,
    pub item: u64,
    pub values: [u64; 5],
}
pub const GROUP: u64 = 1;
pub const MONTH: u64 = 2;
pub const ENEMY_EQUIPMENT: u64 = 3;
pub const MANPOWER: u64 = 4;
pub const TEMPLATE: u64 = 5;
pub const REFERENCE: u64 = 6;
pub const PREFIX_REMOVED: u64 = 7;
pub const GROUP_REMOVED: u64 = 8;
pub const BATTLE: u64 = 9;
pub const SIDE: u64 = 10;
pub const POOL: u64 = 11;
pub const EQUIPMENT: u64 = 12;
pub const MODIFIER: u64 = 13;
pub const COUNTRY: u64 = 14;
pub const SLOT: u64 = 15;
pub const COUNT: u64 = 16;
pub const RESET: u64 = 17;
pub const RELEASE: u64 = 18;
pub const OBSERVER_STATE: u64 = 20;
pub const OBSERVER_MODIFIERS: u64 = 21;
pub const OBSERVER_LEADER: u64 = 22;
pub const OBSERVER_DAMAGE: u64 = 23;
pub const ACTIVITY: u64 = 24;
pub const ACTIVITY_TEMPLATE: u64 = 25;
pub const ACTIVITY_DAMAGE: u64 = 26;
pub const ACTIVITY_EQUIPMENT: u64 = 27;
pub const OBSERVER_SIZE: usize = 0x2f0;
pub const OBSERVER_SIDE: usize = 0x58;
pub const OBSERVER_TOTAL_DAMAGE: usize = 0x50;
pub const OBSERVER_PROGRESS: usize = 0x268;
pub const OBSERVER_COUNTERS: usize = 0x270;
pub const OBSERVER_FLAGS: usize = 0x2e8;
pub const OBSERVER_GROUPS: usize = 8;
pub const ACTIVITY_SIZE: usize = 0x88;
pub const ACTIVITY_GROUP: usize = 8;
pub const ACTIVITY_TEMPLATES: usize = 0x10;
pub const ACTIVITY_DAMAGE_ENTRIES: usize = 0x28;
pub const ACTIVITY_EQUIPMENT_POOL: usize = 0x40;
pub const ACTIVITY_DEALER: usize = 0x80;
pub const ACTIVITY_TAKER: usize = 0x84;
pub const DAMAGE_FROM: usize = 8;
pub const DAMAGE_RECEIVER: usize = 12;
pub const DAMAGE_TO: usize = 16;
pub const DAMAGE_VALUE: usize = 24;
pub const ACTIVITY_VTABLE_RVA: Rva = Rva(0x029d_a080);
pub const DAMAGE_VTABLE_RVA: Rva = Rva(0x029d_a030);

pub const ARRAYS: [usize; 9] = [
    group::REASONS,
    group::REASONS + group::REASON_STRIDE,
    group::REASONS + 2 * group::REASON_STRIDE,
    group::REASONS + 3 * group::REASON_STRIDE,
    group::RECOVERED,
    group::ENEMY_LOSSES,
    layout::MANPOWER,
    layout::TEMPLATES,
    layout::REFERENCES,
];

pub fn noncombat_writer_demand(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<usize, &'static str> {
    use engine_native::{carmy, ccountry, cequipment_variant::pool as pool};
    let countries = ccountry::Reader::new(memory, base, state)?;
    let mut demand = 0usize;
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        let at = country
            .pointer
            .checked_add(ccountry::DIVISIONS)
            .ok_or("combat log division sizing address overflow")?;
        let divisions =
            Array::read(memory, at, 8).ok_or("combat log division sizing roster invalid")?;
        engine_native_memory::cpdx_array::pointers(
            memory,
            divisions,
            "combat log sizing division unreadable",
            |_, pointer| {
                header::<8>(memory, base, pointer, carmy::VTABLE_RVA)?;
                let pool_at = pointer
                    .checked_add(carmy::EQUIPMENT_POOL)
                    .ok_or("combat log pool sizing address overflow")?;
                let equipment = pool::entries(memory, base, pool_at)?;
                demand = demand
                    .checked_add(noncombat_division_demand(equipment.count as usize)?)
                    .ok_or("combat log writer sizing overflow")?;
                Ok(())
            },
        )?;
        if Array::read(memory, at, 8) != Some(divisions) {
            return Err("combat log sizing division roster changed");
        }
    }
    Ok(demand)
}
fn noncombat_division_demand(variants: usize) -> Result<usize, &'static str> {
    variants
        .checked_add(3)
        .ok_or("combat log writer sizing overflow")
}

fn field<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|p| read(memory, p))
        .ok_or("combat log event operand unreadable")
}
fn header<const N: usize>(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    vtable: Rva,
) -> Result<[u8; N], &'static str> {
    let bytes: [u8; N] = field(memory, pointer, 0)?;
    if engine_sites::at(base, vtable).map(|p| p as u64) != Some(word(&bytes, 0)) {
        return Err("combat log event class differs");
    }
    Ok(bytes)
}
fn word(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}
fn int(bytes: &[u8], at: usize) -> u64 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as u64
}
fn array(bytes: &[u8], at: usize, stride: usize) -> Result<Array, &'static str> {
    Array::from_header(bytes[at..at + 16].try_into().unwrap(), stride)
        .ok_or("combat log event array invalid")
}
fn element(array: Array, index: u32, stride: usize) -> Result<usize, &'static str> {
    array
        .element(index, stride)
        .ok_or("combat log event index invalid")
}
fn emit(put: &mut impl FnMut(Fact), kind: u64, root: u64, item: u64, values: [u64; 5]) {
    put(Fact {
        kind,
        root,
        item,
        values,
    });
}

pub fn counts(memory: &impl Memory, base: usize, pointer: usize) -> Result<[u32; 9], &'static str> {
    let bytes = header::<{ group::SIZE }>(memory, base, pointer, group::VTABLE_RVA)?;
    let mut output = [0; 9];
    for (i, at) in ARRAYS.into_iter().enumerate() {
        output[i] = array(&bytes, at, 8)?.count;
    }
    Ok(output)
}

pub fn last_month(
    memory: &impl Memory,
    base: usize,
    owner: usize,
    kind: u8,
    put: &mut impl FnMut(Fact),
) -> Result<(), &'static str> {
    let at = *ARRAYS
        .get(kind as usize)
        .filter(|_| kind < 6)
        .ok_or("combat log month kind unsupported")?;
    let list = Array::read(
        memory,
        owner
            .checked_add(at)
            .ok_or("combat log event address overflow")?,
        8,
    )
    .ok_or("combat log event month array invalid")?;
    if list.count != 0 {
        let pointer = field::<u64>(memory, element(list, list.count - 1, 8)?, 0)? as usize;
        month(memory, base, owner, pointer, kind, put)?;
    }
    Ok(())
}

pub fn month(
    memory: &impl Memory,
    base: usize,
    owner: usize,
    pointer: usize,
    kind: u8,
    put: &mut impl FnMut(Fact),
) -> Result<(), &'static str> {
    let bytes = header::<{ loss::SIZE }>(memory, base, pointer, loss::VTABLE_RVA)?;
    let p = loss::POOL;
    if engine_sites::at(base, engine_native::cequipment_variant::pool::VTABLE_RVA).map(|p| p as u64)
        != Some(word(&bytes, p))
    {
        return Err("combat log event month pool class differs");
    }
    emit(
        put,
        MONTH,
        owner as u64,
        pointer as u64,
        [
            kind as u64,
            int(&bytes, loss::REASON),
            int(&bytes, loss::DATE),
            bytes[p + engine_native::cequipment_variant::pool::ALLOW_ZERO] as u64,
            0,
        ],
    );
    if kind == layout::ENEMY_KIND {
        let items = array(
            &bytes,
            p + engine_native::cequipment_variant::pool::ITEMS,
            equipment_entry::STRIDE,
        )?;
        for ordinal in 0..items.count {
            let entry = equipment_entry::read(
                memory,
                base,
                element(items, ordinal, equipment_entry::STRIDE)?,
                ordinal,
            )?;
            emit(
                put,
                ENEMY_EQUIPMENT,
                owner as u64,
                pointer as u64,
                [
                    ordinal as u64,
                    entry.variant_handle,
                    entry.amount_raw as u64,
                    0,
                    0,
                ],
            );
        }
    }
    Ok(())
}

pub fn last_manpower(
    memory: &impl Memory,
    base: usize,
    owner: usize,
    put: &mut impl FnMut(Fact),
) -> Result<(), &'static str> {
    let list = Array::read(
        memory,
        owner
            .checked_add(layout::MANPOWER)
            .ok_or("combat log event address overflow")?,
        8,
    )
    .ok_or("combat log event manpower array invalid")?;
    if list.count != 0 {
        let pointer = field::<u64>(memory, element(list, list.count - 1, 8)?, 0)? as usize;
        manpower(memory, base, owner, pointer, put)?;
    }
    Ok(())
}
pub fn manpower(
    memory: &impl Memory,
    base: usize,
    owner: usize,
    pointer: usize,
    put: &mut impl FnMut(Fact),
) -> Result<(), &'static str> {
    let bytes =
        header::<{ layout::MANPOWER_SIZE }>(memory, base, pointer, layout::MANPOWER_VTABLE_RVA)?;
    emit(
        put,
        MANPOWER,
        owner as u64,
        pointer as u64,
        [
            int(&bytes, loss::REASON),
            int(&bytes, loss::DATE),
            int(&bytes, layout::MANPOWER_LOSSES),
            int(&bytes, layout::MANPOWER_LOSSES + 4),
            int(&bytes, layout::MANPOWER_LOSSES + 8),
        ],
    );
    Ok(())
}

pub fn template(
    memory: &impl Memory,
    base: usize,
    owner: usize,
    pointer: usize,
    increment_pending: bool,
    put: &mut impl FnMut(Fact),
) -> Result<(), &'static str> {
    let bytes =
        header::<{ layout::TEMPLATE_SIZE }>(memory, base, pointer, layout::TEMPLATE_VTABLE_RVA)?;
    let total =
        (int(&bytes, layout::TEMPLATE_TOTAL) as u32).wrapping_add(u32::from(increment_pending));
    emit(
        put,
        TEMPLATE,
        owner as u64,
        pointer as u64,
        [
            word(&bytes, layout::TEMPLATE_REFERENCE),
            int(&bytes, layout::TEMPLATE_DATE),
            int(&bytes, layout::TEMPLATE_WINS),
            total as u64,
            0,
        ],
    );
    Ok(())
}

pub fn last_reference(
    memory: &impl Memory,
    owner: usize,
    put: &mut impl FnMut(Fact),
) -> Result<(), &'static str> {
    let list = Array::read(
        memory,
        owner
            .checked_add(layout::REFERENCES)
            .ok_or("combat log event address overflow")?,
        8,
    )
    .ok_or("combat log event reference array invalid")?;
    let ordinal = list
        .count
        .checked_sub(1)
        .ok_or("combat log writer appended no reference")?;
    reference(memory, owner, list, ordinal, put)
}
fn reference(
    memory: &impl Memory,
    owner: usize,
    list: Array,
    ordinal: u32,
    put: &mut impl FnMut(Fact),
) -> Result<(), &'static str> {
    let bytes: [u8; 8] = field(memory, element(list, ordinal, 8)?, 0)?;
    emit(
        put,
        REFERENCE,
        owner as u64,
        ordinal as u64,
        [int(&bytes, 0), bytes[4] as u64, 0, 0, 0],
    );
    Ok(())
}

pub fn seed_group(
    memory: &impl Memory,
    base: usize,
    owner: usize,
    country: u32,
    ordinal: u32,
    bytes: &[u8],
    put: &mut impl FnMut(Fact),
) -> Result<(), &'static str> {
    if bytes.len() != group::SIZE
        || engine_sites::at(base, group::VTABLE_RVA).map(|p| p as u64) != Some(word(bytes, 0))
    {
        return Err("combat log seed group class differs");
    }
    emit(
        put,
        GROUP,
        owner as u64,
        0,
        [
            country as u64,
            word(bytes, group::GROUP),
            ordinal as u64,
            0,
            0,
        ],
    );
    for (kind, at) in ARRAYS.into_iter().enumerate() {
        let list = array(bytes, at, 8)?;
        for ordinal in 0..list.count {
            if kind == 8 {
                reference(memory, owner, list, ordinal, put)?;
                continue;
            }
            let pointer = field::<u64>(memory, element(list, ordinal, 8)?, 0)? as usize;
            match kind {
                0..=5 => month(memory, base, owner, pointer, kind as u8, put)?,
                6 => manpower(memory, base, owner, pointer, put)?,
                7 => template(memory, base, owner, pointer, false, put)?,
                _ => unreachable!(),
            }
        }
    }
    Ok(())
}

pub fn battle(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    token: u64,
    put: &mut impl FnMut(Fact),
) -> Result<(), &'static str> {
    let bytes =
        header::<{ layout::BATTLE_SIZE }>(memory, base, pointer, layout::BATTLE_VTABLE_RVA)?;
    emit(
        put,
        BATTLE,
        token,
        0,
        [
            int(&bytes, layout::BATTLE_DATE),
            int(&bytes, layout::BATTLE_PROVINCE),
            int(&bytes, layout::BATTLE_FLAGS),
            0,
            0,
        ],
    );
    for (side, at) in [layout::BATTLE_ATTACKER, layout::BATTLE_DEFENDER]
        .into_iter()
        .enumerate()
    {
        side_data(
            memory,
            base,
            &bytes[at..at + layout::SIDE_SIZE],
            token,
            side as u64,
            put,
        )?;
    }
    Ok(())
}
pub fn side_data(
    memory: &impl Memory,
    base: usize,
    bytes: &[u8],
    root: u64,
    side: u64,
    put: &mut impl FnMut(Fact),
) -> Result<(), &'static str> {
    side_data_inner(memory, base, bytes, root, side, false, put)
}
fn side_data_inner(
    memory: &impl Memory,
    base: usize,
    bytes: &[u8],
    root: u64,
    side: u64,
    reuse_losses: bool,
    put: &mut impl FnMut(Fact),
) -> Result<(), &'static str> {
    if bytes.len() != layout::SIDE_SIZE
        || engine_sites::at(base, layout::SIDE_VTABLE_RVA).map(|p| p as u64) != Some(word(bytes, 0))
    {
        return Err("combat log event side class differs");
    }
    emit(
        put,
        SIDE,
        root,
        side,
        [
            if reuse_losses {
                0
            } else {
                int(bytes, layout::SIDE_MANPOWER)
            },
            word(bytes, layout::SIDE_AIR_FACTOR),
            word(bytes, layout::SIDE_LEADER),
            0,
            0,
        ],
    );
    for (kind, at) in [
        layout::SIDE_EQUIPMENT,
        layout::SIDE_CAPTURED_BY_ENEMY,
        layout::SIDE_RECOVERED,
    ]
    .into_iter()
    .enumerate()
    {
        let p = &bytes[at..];
        if engine_sites::at(base, engine_native::cequipment_variant::pool::VTABLE_RVA).map(|p| p as u64)
            != Some(word(p, 0))
        {
            return Err("combat log event side pool class differs");
        }
        let allocations = array(p, engine_native::cequipment_variant::pool::ALLOCATIONS, 24)?;
        let mut serialized = false;
        for ordinal in 0..allocations.count {
            if field::<i64>(memory, element(allocations, ordinal, 24)?, 16)? != 0 {
                serialized = true;
                break;
            }
        }
        let key = side * 3 + kind as u64;
        emit(
            put,
            POOL,
            root,
            key,
            [
                u64::from(serialized),
                p[engine_native::cequipment_variant::pool::ALLOW_ZERO] as u64,
                0,
                0,
                0,
            ],
        );
        if !serialized || reuse_losses && kind == 0 {
            continue;
        }
        let items = array(
            p,
            engine_native::cequipment_variant::pool::ITEMS,
            equipment_entry::STRIDE,
        )?;
        for ordinal in 0..items.count {
            let entry = equipment_entry::read(
                memory,
                base,
                element(items, ordinal, equipment_entry::STRIDE)?,
                ordinal,
            )?;
            emit(
                put,
                EQUIPMENT,
                root,
                key,
                [
                    ordinal as u64,
                    entry.variant_handle,
                    entry.amount_raw as u64,
                    0,
                    0,
                ],
            );
        }
    }
    let modifiers = array(bytes, layout::SIDE_MODIFIERS, 16)?;
    for ordinal in 0..modifiers.count {
        let entry = header::<16>(
            memory,
            base,
            element(modifiers, ordinal, 16)?,
            layout::MODIFIER_HOURS_VTABLE_RVA,
        )?;
        emit(
            put,
            MODIFIER,
            root,
            side,
            [ordinal as u64, int(&entry, 8), int(&entry, 12), 0, 0],
        );
    }
    let countries = array(bytes, layout::SIDE_COUNTRIES, 4)?;
    for ordinal in 0..countries.count {
        emit(
            put,
            COUNTRY,
            root,
            side,
            [
                ordinal as u64,
                field::<u32>(memory, element(countries, ordinal, 4)?, 0)? as u64,
                0,
                0,
                0,
            ],
        );
    }
    Ok(())
}

pub fn observer(
    memory: &impl Memory,
    base: usize,
    side: usize,
    put: &mut impl FnMut(Fact),
) -> Result<(), &'static str> {
    let pointer = side
        .checked_add(engine_native::cland_combatant::STATS_OBSERVER)
        .ok_or("combat log observer address overflow")?;
    let bytes = header::<OBSERVER_SIZE>(memory, base, pointer, crate::cstats_observer::VTABLE_RVA)?;
    emit(
        put,
        OBSERVER_STATE,
        0,
        0,
        [
            word(&bytes, OBSERVER_TOTAL_DAMAGE),
            word(&bytes, OBSERVER_PROGRESS),
            bytes[OBSERVER_FLAGS] as u64,
            0,
            0,
        ],
    );
    for first in (0..30).step_by(5) {
        emit(
            put,
            OBSERVER_MODIFIERS,
            0,
            first as u64,
            std::array::from_fn(|i| int(&bytes, OBSERVER_COUNTERS + (first + i) * 4)),
        );
    }
    side_data_inner(
        memory,
        base,
        &bytes[OBSERVER_SIDE..OBSERVER_SIDE + layout::SIDE_SIZE],
        0,
        0,
        true,
        put,
    )?;
    let leaders = array(&bytes, crate::cstats_observer::LEADERS, 12)?;
    for ordinal in 0..leaders.count {
        let entry: [u8; 12] = field(memory, element(leaders, ordinal, 12)?, 0)?;
        emit(
            put,
            OBSERVER_LEADER,
            0,
            ordinal as u64,
            [word(&entry, 0), int(&entry, 8), 0, 0, 0],
        );
    }
    let damages = array(&bytes, crate::cstats_observer::DAMAGE, 32)?;
    for ordinal in 0..damages.count {
        let entry = header::<32>(
            memory,
            base,
            element(damages, ordinal, 32)?,
            DAMAGE_VTABLE_RVA,
        )?;
        emit(
            put,
            OBSERVER_DAMAGE,
            0,
            ordinal as u64,
            [
                int(&entry, DAMAGE_FROM),
                int(&entry, DAMAGE_RECEIVER),
                int(&entry, DAMAGE_TO),
                word(&entry, DAMAGE_VALUE),
                0,
            ],
        );
    }
    let groups = array(&bytes, OBSERVER_GROUPS, 8)?;
    for ordinal in 0..groups.count {
        let pointer = field::<u64>(memory, element(groups, ordinal, 8)?, 0)? as usize;
        let entry = header::<ACTIVITY_SIZE>(memory, base, pointer, ACTIVITY_VTABLE_RVA)?;
        emit(
            put,
            ACTIVITY,
            0,
            ordinal as u64,
            [
                word(&entry, ACTIVITY_GROUP),
                int(&entry, ACTIVITY_DEALER),
                int(&entry, ACTIVITY_TAKER),
                0,
                0,
            ],
        );
        let templates = array(&entry, ACTIVITY_TEMPLATES, 8)?;
        for index in 0..templates.count {
            emit(
                put,
                ACTIVITY_TEMPLATE,
                0,
                ordinal as u64,
                [
                    index as u64,
                    field::<u64>(memory, element(templates, index, 8)?, 0)?,
                    0,
                    0,
                    0,
                ],
            );
        }
        let damage = array(&entry, ACTIVITY_DAMAGE_ENTRIES, 16)?;
        for index in 0..damage.count {
            let value: [u8; 16] = field(memory, element(damage, index, 16)?, 0)?;
            emit(
                put,
                ACTIVITY_DAMAGE,
                0,
                ordinal as u64,
                [index as u64, word(&value, 0), word(&value, 8), 0, 0],
            );
        }
        let pool_at = ACTIVITY_EQUIPMENT_POOL;
        if engine_sites::at(base, engine_native::cequipment_variant::pool::VTABLE_RVA).map(|p| p as u64)
            != Some(word(&entry, pool_at))
        {
            return Err("combat log activity pool class differs");
        }
        let allocations = array(
            &entry,
            pool_at + engine_native::cequipment_variant::pool::ALLOCATIONS,
            24,
        )?;
        let mut serialized = false;
        for index in 0..allocations.count {
            if field::<i64>(memory, element(allocations, index, 24)?, 16)? != 0 {
                serialized = true;
                break;
            }
        }
        emit(
            put,
            ACTIVITY_EQUIPMENT,
            0,
            ordinal as u64,
            [
                u64::MAX,
                u64::from(serialized),
                entry[pool_at + engine_native::cequipment_variant::pool::ALLOW_ZERO] as u64,
                0,
                0,
            ],
        );
        if serialized {
            let items = array(
                &entry,
                pool_at + engine_native::cequipment_variant::pool::ITEMS,
                equipment_entry::STRIDE,
            )?;
            for index in 0..items.count {
                let item = equipment_entry::read(
                    memory,
                    base,
                    element(items, index, equipment_entry::STRIDE)?,
                    index,
                )?;
                emit(
                    put,
                    ACTIVITY_EQUIPMENT,
                    0,
                    ordinal as u64,
                    [
                        index as u64,
                        item.variant_handle,
                        item.amount_raw as u64,
                        0,
                        0,
                    ],
                );
            }
        }
    }
    Ok(())
}

pub fn count(memory: &impl Memory, base: usize, id: u32) -> Result<i16, &'static str> {
    let list = Array::read(
        memory,
        engine_sites::at(base, layout::BATTLE_COUNTS_RVA)
            .ok_or("combat log event address overflow")?,
        2,
    )
    .ok_or("combat log event count array invalid")?;
    field(memory, element(list, id, 2)?, 0)
}

pub fn settled_counts(
    memory: &impl Memory,
    base: usize,
    ids: &[u32],
    visit: &mut impl FnMut(u32, i16),
) -> Result<(), &'static str> {
    if ids.is_empty() {
        return Ok(());
    }
    let counts = Array::read(
        memory,
        engine_sites::at(base, layout::BATTLE_COUNTS_RVA)
            .ok_or("combat log event address overflow")?,
        2,
    )
    .ok_or("combat log event count array invalid")?;
    let used = Array::read(
        memory,
        engine_sites::at(base, layout::BATTLE_USED_RVA)
            .ok_or("combat log event address overflow")?,
        1,
    )
    .ok_or("combat log event occupancy array invalid")?;
    if counts.count != used.count {
        return Err("combat log lifetime array lengths differ");
    }
    for &id in ids {
        let count = field::<i16>(memory, element(counts, id, 2)?, 0)?;
        let occupied = field::<u8>(memory, element(used, id, 1)?, 0)? != 0;
        if count < 0 || (count > 0) != occupied {
            return Err("combat log settled count and occupancy differ");
        }
        visit(id, count);
    }
    Ok(())
}

#[cfg(test)]
mod writer_demand_tests {
    use super::*;
    #[test]
    fn noncombat_demand_includes_every_variant_and_creation_pool_manpower_writers() {
        assert_eq!(noncombat_division_demand(0), Ok(3));
        assert_eq!(noncombat_division_demand(11_133), Ok(11_136));
        assert!(noncombat_division_demand(usize::MAX).is_err());
    }
}
