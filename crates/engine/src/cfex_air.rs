//! CFEXAir active naval air slot. Constructor141962f80, update141969b60,
//! grouped icon/count binder1417daa40 and efficiency tooltip141963700.
use crate::{Memory, cequipment_variant, cnaval_combatant::field, cpdx_array::Array, msvc_string};

pub const VTABLE_RVA: usize = 0x02a0_a588;
pub const WING_CAPACITY: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Native ordered source:0 active group,1 retained group,2 independent side.
    pub source: u8,
    pub group_index: u32,
    pub index: u32,
    pub carrier_slot: i32,
    pub carrier_state: u32,
    pub air_base: u64,
    pub wings: [[u64; WING_CAPACITY]; 2],
    pub wing_counts: [usize; 2],
    pub initial_planes: i32,
    pub current_planes: i32,
    pub casualties: i32,
    pub time_duration: i32,
    pub country: u32,
    pub date: i32,
    pub flags: [u8; 2],
    pub name: msvc_string::Text<256>,
    pub variant: u64,
    pub cached_values: [i64; 6],
    pub naval_hits: [usize; 2],
    pub air_hits: [[usize; 2]; 2],
    pub names: [usize; 2],
    pub kills: [usize; 2],
    pub efficiency: crate::naval_air_efficiency_inputs::Row,
}

impl Default for Row {
    fn default() -> Self {
        Self {
            source: 0,
            group_index: 0,
            index: 0,
            carrier_slot: 0,
            carrier_state: 0,
            air_base: 0,
            wings: [[0; WING_CAPACITY]; 2],
            wing_counts: [0; 2],
            initial_planes: 0,
            current_planes: 0,
            casualties: 0,
            time_duration: 0,
            country: 0,
            date: 0,
            flags: [0; 2],
            name: Default::default(),
            variant: 0,
            cached_values: [0; 6],
            naval_hits: [0; 2],
            air_hits: [[0; 2]; 2],
            names: [0; 2],
            kills: [0; 2],
            efficiency: Default::default(),
        }
    }
}

/// CFEXAir retained naval damage+100, aircraft losses+118 and+130. Loader
///141969540 and teardown141963180 preserve the three independent arrays.
pub(crate) fn read_hits(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    row: &mut Row,
    buffers: &mut crate::cfex_member::DetailBuffers<'_>,
) -> Result<(), &'static str> {
    use crate::cfex_member::{DETAIL_ARENA_CAPACITY, append_entries};
    let mut counts = [0usize; 2];
    for (offset, lane) in [(0x100, 1), (0x118, 0), (0x130, 0)] {
        let array = Array::read(
            memory,
            pointer
                .checked_add(offset)
                .ok_or("naval plane hit array overflow")?,
            8,
        )
        .ok_or("naval plane hit array invalid")?;
        if array.count > 16384 {
            return Err("naval plane hit bound exceeded");
        }
        counts[lane] += array.count as usize;
    }
    let capacity = [buffers.plane_air_hits.len(), buffers.plane_naval_hits.len()];
    let names = Array::read(
        memory,
        pointer
            .checked_add(0xa8)
            .ok_or("naval plane names overflow")?,
        32,
    )
    .ok_or("naval plane names array invalid")?;
    if names.count > 16384 {
        return Err("naval plane names bound exceeded");
    }
    buffers.plane_names_required = buffers
        .plane_names_required
        .checked_add(names.count as usize)
        .ok_or("naval plane names demand overflow")?;
    let mut fits = true;
    for lane in 0..2 {
        buffers.plane_hit_required[lane] = buffers.plane_hit_required[lane]
            .checked_add(counts[lane])
            .ok_or("naval plane hit required overflow")?;
        fits &= buffers.plane_hit_lengths[lane]
            .checked_add(counts[lane])
            .is_some_and(|n| n <= capacity[lane]);
    }
    fits &= buffers
        .plane_names_len
        .checked_add(names.count as usize)
        .is_some_and(|n| n <= buffers.plane_names.len());
    if !fits {
        return Err(DETAIL_ARENA_CAPACITY);
    }
    row.names = append_entries(
        memory,
        pointer,
        0xa8,
        32,
        buffers.plane_names,
        &mut buffers.plane_names_len,
        |p| msvc_string::read_text(memory, p).ok_or("naval plane tooltip name invalid"),
        false,
    )?;
    row.naval_hits = append_entries(
        memory,
        pointer,
        0x100,
        8,
        buffers.plane_naval_hits,
        &mut buffers.plane_hit_lengths[1],
        |p| crate::naval_combat_hit::read(memory, base, p),
        true,
    )?;
    for (index, offset) in [0x118, 0x130].into_iter().enumerate() {
        row.air_hits[index] = append_entries(
            memory,
            pointer,
            offset,
            8,
            buffers.plane_air_hits,
            &mut buffers.plane_hit_lengths[0],
            |p| crate::naval_combat_air_hit::read(memory, base, p),
            true,
        )?;
    }
    Ok(())
}

pub(crate) fn read(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    group: usize,
) -> Result<Row, &'static str> {
    if field::<u64>(memory, pointer, 0)?
        != base
            .checked_add(VTABLE_RVA)
            .ok_or("naval plane module overflow")? as u64
        || field::<u64>(memory, pointer, 8)? != group as u64
    {
        return Err("naval plane identity or group mismatch");
    }
    let carrier = field::<u64>(memory, pointer, 0x10)? as usize;
    let carrier_slot = field(memory, pointer, 0x18)?;
    let carrier_state = if carrier == 0 {
        0
    } else {
        if field::<u64>(memory, carrier, 0)?
            != base
                .checked_add(crate::cfex_member::VTABLE_RVA)
                .ok_or("naval plane carrier module overflow")? as u64
            || field::<i32>(memory, carrier, crate::cfex_member::BATTLE_SLOT)? != carrier_slot
            || field::<u64>(memory, carrier, crate::cfex_member::GROUP)? != group as u64
        {
            return Err("naval plane carrier identity mismatch");
        }
        field(memory, carrier, crate::cfex_member::STATE)?
    };
    let mut row = Row {
        carrier_slot,
        carrier_state,
        air_base: field(memory, pointer, 0x1c)?,
        initial_planes: field(memory, pointer, 0x58)?,
        current_planes: field(memory, pointer, 0x5c)?,
        casualties: field(memory, pointer, 0x60)?,
        time_duration: field(memory, pointer, 0xf8)?,
        country: field(memory, pointer, 0x64)?,
        date: field(memory, pointer, 0x70)?,
        flags: [field(memory, pointer, 0x80)?, field(memory, pointer, 0x81)?],
        name: msvc_string::read_text(
            memory,
            pointer
                .checked_add(0x88)
                .ok_or("naval plane name overflow")?,
        )
        .ok_or("naval plane name invalid or exceeds capacity")?,
        ..Row::default()
    };
    let variant = field::<u64>(memory, pointer, 0xf0)? as usize;
    if variant != 0 {
        if field::<u64>(memory, variant, 0)?
            != base
                .checked_add(cequipment_variant::VTABLE_RVA)
                .ok_or("naval plane variant module overflow")? as u64
        {
            return Err("naval plane variant identity mismatch");
        }
        row.variant = field(memory, variant, 8)?;
    }
    for (index, offset) in [0x28, 0x40].into_iter().enumerate() {
        let address = pointer
            .checked_add(offset)
            .ok_or("naval plane wing array overflow")?;
        let array = Array::read(memory, address, 8).ok_or("naval plane wing array invalid")?;
        if array.count as usize > WING_CAPACITY {
            return Err("naval plane wing capacity exceeded");
        }
        row.wing_counts[index] = array.count as usize;
        for i in 0..array.count {
            row.wings[index][i as usize] = crate::read(
                memory,
                array
                    .element(i, 8)
                    .ok_or("naval plane wing element overflow")?,
            )
            .ok_or("naval plane wing unreadable")?;
        }
        if Array::read(memory, address, 8) != Some(array) {
            return Err("naval plane wings changed");
        }
    }
    for (index, value) in row.cached_values.iter_mut().enumerate() {
        *value = field(memory, pointer, 0xc0 + index * 8)?;
    }
    if field::<u64>(memory, pointer, 8)? != group as u64 {
        return Err("naval plane group changed");
    }
    Ok(row)
}
