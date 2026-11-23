use engine_sites::Rva;
use crate::{
    Memory, cgame_state,
    cpdx_array::Array,
    cstate,
    memory::{RecordField, RecordLayout, RecordSources},
    walk::Outcome,
};

mod identity_batch;
pub use identity_batch::IdentityBatch;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct UnitOrderRow {
    pub province: u32,
    pub mixed: [usize; 2],
    pub naval: [usize; 2],
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnitOrderWalk {
    pub outcome: Outcome,
    pub len: usize,
    pub required: usize,
}

#[cfg(any(test, feature = "test-support"))]
pub fn walk_unit_order(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut Vec<UnitOrderRow>,
    handles: &mut Vec<u64>,
) -> UnitOrderWalk {
    walk_unit_order_with_land(memory, base, state, output, handles, |_, _| Ok(()))
}

pub fn count_unit_order_with_land(memory: &impl Memory, base: usize, state: usize, land: impl FnMut(u32, u64) -> Result<(), &'static str>) -> UnitOrderWalk {
    unit_order(memory, base, state, crate::walk::Buffer::Count, crate::walk::Buffer::Count, |_| None, land, None)
}

pub fn walk_unit_order_with_land(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut Vec<UnitOrderRow>,
    handles: &mut Vec<u64>,
    land: impl FnMut(u32, u64) -> Result<(), &'static str>,
) -> UnitOrderWalk {
    walk_unit_order_with_known(memory, base, state, output, handles, |_| None, land)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct KnownUnit {
    pub handle: u64,
    pub location: usize,
}

pub fn walk_unit_order_with_known(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut Vec<UnitOrderRow>,
    handles: &mut Vec<u64>,
    known: impl Fn(usize) -> Option<KnownUnit>,
    land: impl FnMut(u32, u64) -> Result<(), &'static str>,
) -> UnitOrderWalk {
    unit_order(
        memory,
        base,
        state,
        crate::walk::Buffer::Growing(output),
        crate::walk::Buffer::Growing(handles),
        known,
        land,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn walk_unit_order_listed(
    memory: &impl Memory,
    base: usize,
    state: usize,
    listed: &[u32],
    output: &mut Vec<UnitOrderRow>,
    handles: &mut Vec<u64>,
    known: impl Fn(usize) -> Option<KnownUnit>,
    land: impl FnMut(u32, u64) -> Result<(), &'static str>,
) -> UnitOrderWalk {
    unit_order(
        memory,
        base,
        state,
        crate::walk::Buffer::Growing(output),
        crate::walk::Buffer::Growing(handles),
        known,
        land,
        Some(listed),
    )
}

pub fn unit_order_header(memory: &impl Memory, state: usize) -> Option<[u8; 16]> {
    crate::read::<[u8; 16]>(memory, state.checked_add(cgame_state::PROVINCES)?)
}

pub fn unit_order_slots(
    memory: &impl Memory,
    header: [u8; 16],
    slots: &mut Vec<usize>,
) -> Result<(), &'static str> {
    const CHUNK: usize = 512;
    slots.clear();
    let provinces = Array::from_header(header, 8)
        .filter(|a| a.count > 1)
        .ok_or("province order census invalid")?;
    crate::allocation::reserve(slots, provinces.count as usize - 1)
        .map_err(|_| "province order slot allocation failed")?;
    let mut bytes = [0u8; CHUNK * 8];
    for first in (1..provinces.count).step_by(CHUNK) {
        let n = ((provinces.count - first) as usize).min(CHUNK);
        let at = provinces
            .element(first, 8)
            .ok_or("province order slot unreadable")?;
        if !crate::read_slice(memory, at, &mut bytes[..n * 8]) {
            return Err("province order slot unreadable");
        }
        slots.extend(
            bytes[..n * 8]
                .chunks_exact(8)
                .map(|word| u64::from_le_bytes(word.try_into().unwrap()) as usize),
        );
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn unit_order(
    memory: &impl Memory,
    base: usize,
    state: usize,
    mut output: crate::walk::Buffer<'_, UnitOrderRow>,
    mut handles: crate::walk::Buffer<'_, u64>,
    known: impl Fn(usize) -> Option<KnownUnit>,
    mut land: impl FnMut(u32, u64) -> Result<(), &'static str>,
    listed: Option<&[u32]>,
) -> UnitOrderWalk {
    let mut used = 0usize;
    let mut required = 0usize;
    let result = (|| {
        let root = state
            .checked_add(cgame_state::PROVINCES)
            .ok_or("province order root overflow")?;
        let header =
            crate::read::<[u8; 16]>(memory, root).ok_or("province order root unreadable")?;
        let provinces = Array::from_header(header, 8)
            .filter(|a| a.count > 1)
            .ok_or("province order census invalid")?;
        let total = listed.map_or((provinces.count - 1) as usize, <[u32]>::len);
        output.ensure(total)?;
        let mut len = 0_usize;
        let mut missing = 0u32;
        let vtable = engine_sites::at(base, VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        let mut registry = crate::creference_registry::HeaderResolver::default();
        let mut pointer_bytes = [0u8; BATCH_ROWS * 8];
        let mut pointers = [0usize; BATCH_ROWS];
        let mut fields = [0u8; BATCH_ROWS * UNIT_ORDER_ROW_BYTES];
        let mut unit_pointers = [0u8; BATCH_ROWS * 8];
        let mut ids = [0u32; BATCH_ROWS];
        let mut slots = [0usize; BATCH_ROWS];
        let mut done = 0;
        while done < total {
            let n = (total - done).min(BATCH_ROWS);
            let sources = match listed {
                None => {
                    for (slot, id) in ids[..n].iter_mut().enumerate() {
                        *id = (done + slot + 1) as u32;
                    }
                    RecordSources::Strided {
                        base: provinces
                            .element(ids[0], 8)
                            .ok_or("province order slot unreadable")?,
                        stride: 8,
                        count: n,
                    }
                }
                Some(list) => {
                    ids[..n].copy_from_slice(&list[done..done + n]);
                    for (slot, &id) in slots[..n].iter_mut().zip(&ids[..n]) {
                        if id == 0 || id >= provinces.count {
                            return Err("province order listed province outside census");
                        }
                        *slot = provinces
                            .element(id, 8)
                            .ok_or("province order slot unreadable")?;
                    }
                    RecordSources::Pointers(&slots[..n])
                }
            };
            if memory.copy_records(sources, &POINTER_LAYOUT, &mut pointer_bytes[..n * 8]) != n {
                return Err("province order slot unreadable");
            }
            for (pointer, bytes) in pointers[..n].iter_mut().zip(pointer_bytes.chunks_exact(8)) {
                *pointer = u64::from_le_bytes(bytes.try_into().unwrap()) as usize;
            }
            if pointers[..n].contains(&0) {
                return Err("province order identity mismatch");
            }
            let completed = memory.copy_records(
                RecordSources::Pointers(&pointers[..n]),
                &UNIT_ORDER_LAYOUT,
                &mut fields[..n * UNIT_ORDER_ROW_BYTES],
            );
            if completed != n * UNIT_ORDER_FIELDS.len() {
                return Err(if completed % UNIT_ORDER_FIELDS.len() < 2 {
                    "province order identity mismatch"
                } else {
                    "province order header unreadable"
                });
            }
            done += n;
            for slot in 0..n {
                let province = ids[slot];
                let pointer = pointers[slot];
                let row = &fields[slot * UNIT_ORDER_ROW_BYTES..][..UNIT_ORDER_ROW_BYTES];
                if u64::from_le_bytes(row[..8].try_into().unwrap()) != vtable
                    || u32::from_le_bytes(row[8..12].try_into().unwrap()) != province
                {
                    return Err("province order identity mismatch");
                }
                if usize::BITS == 64
                    && u64::from_le_bytes(row[20..28].try_into().unwrap()) <= i32::MAX as u64
                    && u64::from_le_bytes(row[36..44].try_into().unwrap()) <= i32::MAX as u64
                {
                    if len == output.len() {
                        missing = missing.saturating_add(1);
                    } else {
                        output[len] = UnitOrderRow {
                            province,
                            mixed: [used, 0],
                            naval: [used, 0],
                        };
                        len += 1;
                    }
                    continue;
                }
                let mut arrays = [Array { data: 0, count: 0 }; 2];
                for (lane, at) in [12usize, 28].into_iter().enumerate() {
                    arrays[lane] = Array::from_header(row[at..at + 16].try_into().unwrap(), 8)
                        .ok_or("province order array invalid or bound exceeded")?;
                }
                let count = arrays.iter().map(|a| a.count as usize).sum::<usize>();
                required = required
                    .checked_add(count)
                    .ok_or("province order aggregate bound exceeded")?;
                let fits = handles.ensure(
                    used.checked_add(count)
                        .ok_or("province order aggregate overflow")?,
                )?;
                if len == output.len() || !fits {
                    missing = missing.saturating_add(1);
                    continue;
                }
                let mut ranges = [[0; 2]; 2];
                for lane in 0..2 {
                    ranges[lane] = [used, arrays[lane].count as usize];
                    for first in (0..arrays[lane].count).step_by(64) {
                        let n = (arrays[lane].count - first).min(64) as usize;
                        if !crate::read_slice(
                            memory,
                            arrays[lane]
                                .element(first, 8)
                                .ok_or("province order element overflow")?,
                            &mut unit_pointers[..n * 8],
                        ) {
                            return Err("province order pointers unreadable");
                        }
                        for pending in unit_pointers[..n * 8].chunks(4 * 8) {
                            let mut census = [None; 4];
                            for (slot, bytes) in pending.chunks_exact(8).enumerate() {
                                let unit = u64::from_le_bytes(bytes.try_into().unwrap()) as usize;
                                census[slot] = known(unit);
                                if unit != 0 && census[slot].is_none() {
                                    crate::prefetch::span(unit, 32);
                                    if let Some(backlink) = unit.checked_add(crate::cunit::LOCATION)
                                    {
                                        crate::prefetch::span(backlink, 8);
                                    }
                                }
                            }
                            for (slot, bytes) in pending.chunks_exact(8).enumerate() {
                                let unit = u64::from_le_bytes(bytes.try_into().unwrap()) as usize;
                                let (handle, class, location) = match census[slot] {
                                    Some(KnownUnit { handle, location }) => {
                                        (handle, 0, Some(location))
                                    }
                                    None => {
                                        let (handle, class) = crate::cunit::counter_identity(
                                            memory,
                                            base,
                                            unit,
                                            &mut registry,
                                        )?;
                                        (handle, class, None)
                                    }
                                };
                                if (lane == 1 && class != 1)
                                    || match location {
                                        Some(location) => location != pointer,
                                        None => {
                                            crate::read::<u64>(
                                                memory,
                                                unit.checked_add(crate::cunit::LOCATION)
                                                    .ok_or("province unit backlink overflow")?,
                                            ) != Some(pointer as u64)
                                        }
                                    }
                                {
                                    return Err("province unit roster backlink mismatch");
                                }
                                handles[used] = handle;
                                if lane == 0 && class == 0 {
                                    land(province, handle)?;
                                }
                                used += 1;
                            }
                        }
                    }
                }
                output[len] = UnitOrderRow {
                    province,
                    mixed: ranges[0],
                    naval: ranges[1],
                };
                len += 1;
            }
        }
        Ok(if missing == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: missing,
            }
        })
    })();
    UnitOrderWalk {
        outcome: result.unwrap_or_else(|reason| Outcome::Unavailable { reason }),
        len: used,
        required,
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DirectCombatsRow {
    pub province: u32,
    pub combats: [usize; 2],
}

pub const COMBAT_LAND: u8 = 0;
pub const COMBAT_BORDER_WAR: u8 = 1;
pub const COMBAT_NAVAL: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectCombatsWalk {
    pub outcome: Outcome,
    pub required: usize,
}

pub fn walk_direct_combats(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut Vec<DirectCombatsRow>,
    handles: &mut Vec<u64>,
    kinds: &mut Vec<u8>,
    listed: Option<&[u32]>,
) -> DirectCombatsWalk {
    walk_direct_combats_listed(
        memory,
        base,
        state,
        crate::walk::Buffer::Growing(output),
        crate::walk::Buffer::Growing(handles),
        crate::walk::Buffer::Growing(kinds),
        listed,
    )
}

pub fn walk_direct_combats_buffer(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: crate::walk::Buffer<'_, DirectCombatsRow>,
    handles: crate::walk::Buffer<'_, u64>,
    kinds: crate::walk::Buffer<'_, u8>,
) -> DirectCombatsWalk {
    walk_direct_combats_listed(memory, base, state, output, handles, kinds, None)
}

fn walk_direct_combats_listed(
    memory: &impl Memory,
    base: usize,
    state: usize,
    mut output: crate::walk::Buffer<'_, DirectCombatsRow>,
    mut handles: crate::walk::Buffer<'_, u64>,
    mut kinds: crate::walk::Buffer<'_, u8>,
    listed: Option<&[u32]>,
) -> DirectCombatsWalk {
    let mut used = 0usize;
    let mut required = 0usize;
    let result = (|| {
        let provinces = state
            .checked_add(cgame_state::PROVINCES)
            .and_then(|root| Array::read(memory, root, 8))
            .filter(|provinces| provinces.count > 1)
            .ok_or("province combat census invalid")?;
        let vtable = engine_sites::at(base, VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        let classes = [
            crate::cland_combat::VTABLE_RVA,
            crate::cland_border_war_combat::VTABLE_RVA,
            crate::cnaval_combat::VTABLE_RVA,
        ]
        .map(|rva| engine_sites::at(base, rva).map(|address| address as u64));
        let (mut len, mut missing) = (0usize, 0u32);
        let mut pointer_bytes = [0u8; BATCH_ROWS * 8];
        let mut pointers = [0usize; BATCH_ROWS];
        let mut fields = [0u8; BATCH_ROWS * DIRECT_ROW_BYTES];
        let mut ids = [0u32; BATCH_ROWS];
        let mut slots = [0usize; BATCH_ROWS];
        let total = listed.map_or((provinces.count - 1) as usize, <[u32]>::len);
        let mut done = 0;
        while done < total {
            let n = (total - done).min(BATCH_ROWS);
            let sources = match listed {
                None => {
                    for (slot, id) in ids[..n].iter_mut().enumerate() {
                        *id = (done + slot + 1) as u32;
                    }
                    RecordSources::Strided {
                        base: provinces
                            .element(ids[0], 8)
                            .ok_or("province combat slot unreadable")?,
                        stride: 8,
                        count: n,
                    }
                }
                Some(list) => {
                    ids[..n].copy_from_slice(&list[done..done + n]);
                    for (slot, &id) in slots[..n].iter_mut().zip(&ids[..n]) {
                        if id == 0 || id >= provinces.count {
                            return Err("province combat listed province outside census");
                        }
                        *slot = provinces
                            .element(id, 8)
                            .ok_or("province combat slot unreadable")?;
                    }
                    RecordSources::Pointers(&slots[..n])
                }
            };
            if memory.copy_records(sources, &POINTER_LAYOUT, &mut pointer_bytes[..n * 8]) != n {
                return Err("province combat slot unreadable");
            }
            for (pointer, bytes) in pointers[..n].iter_mut().zip(pointer_bytes.chunks_exact(8)) {
                *pointer = u64::from_le_bytes(bytes.try_into().expect("eight bytes")) as usize;
            }
            if pointers[..n].contains(&0)
                || memory.copy_records(
                    RecordSources::Pointers(&pointers[..n]),
                    &DIRECT_LAYOUT,
                    &mut fields[..n * DIRECT_ROW_BYTES],
                ) != n * DIRECT_FIELDS.len()
            {
                return Err("province combat identity unreadable");
            }
            done += n;
            for slot in 0..n {
                let province = ids[slot];
                let row = &fields[slot * DIRECT_ROW_BYTES..][..DIRECT_ROW_BYTES];
                if u64::from_le_bytes(row[..8].try_into().expect("eight bytes")) != vtable
                    || u32::from_le_bytes(row[8..12].try_into().expect("four bytes")) != province
                {
                    return Err("province combat identity mismatch");
                }
                let combats = Array::from_header(row[12..28].try_into().expect("header"), 8)
                    .ok_or("province combat vector invalid")?;
                if combats.count == 0 {
                    continue;
                }
                let start = used;
                let count = combats.count as usize;
                let needed = start
                    .checked_add(count)
                    .ok_or("province combat demand overflow")?;
                output.ensure(
                    len.checked_add(1)
                        .ok_or("province combat row demand overflow")?,
                )?;
                handles.ensure(needed)?;
                kinds.ensure(needed)?;
                let fits = len < output.len() && needed <= handles.len().min(kinds.len());
                for entry in 0..combats.count {
                    let combat = combats
                        .element(entry, 8)
                        .and_then(|address| crate::read::<u64>(memory, address))
                        .and_then(|pointer| usize::try_from(pointer).ok())
                        .filter(|&pointer| pointer != 0)
                        .ok_or("province combat pointer unreadable or null")?;
                    let header = crate::read::<[u8; 0x20]>(memory, combat)
                        .ok_or("province combat header unreadable")?;
                    let class = u64::from_le_bytes(header[..8].try_into().expect("eight bytes"));
                    let kind = classes
                        .iter()
                        .position(|&expected| expected == Some(class))
                        .ok_or("province combat class unknown")?;
                    let handle = u64::from_le_bytes(
                        header[crate::cnaval_combat::HANDLE..][..8]
                            .try_into()
                            .expect("eight bytes"),
                    );
                    if handle == 0 {
                        return Err("province combat handle null");
                    }
                    if fits {
                        handles[start + entry as usize] = handle;
                        kinds[start + entry as usize] = KINDS[kind];
                    }
                }
                required = required
                    .checked_add(count)
                    .ok_or("province combat demand overflow")?;
                if fits {
                    used += count;
                    output[len] = DirectCombatsRow {
                        province,
                        combats: [start, count],
                    };
                    len += 1;
                } else {
                    missing = missing.saturating_add(1);
                }
            }
        }
        Ok(if missing == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: missing,
            }
        })
    })();
    DirectCombatsWalk {
        outcome: result.unwrap_or_else(|reason| Outcome::Unavailable { reason }),
        required,
    }
}

pub const COMBATS: usize = 0x170;
const KINDS: [u8; 3] = [COMBAT_LAND, COMBAT_BORDER_WAR, COMBAT_NAVAL];
const DIRECT_FIELDS: [RecordField; 3] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: ID,
        destination_offset: 8,
        len: 4,
    },
    RecordField {
        source_offset: COMBATS,
        destination_offset: 12,
        len: 16,
    },
];
const DIRECT_ROW_BYTES: usize = 28;
const DIRECT_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&DIRECT_FIELDS, DIRECT_ROW_BYTES).expect("valid record layout");

pub const VTABLE_RVA: Rva = Rva(0x029e_e358);
pub const ID: usize = 0xa4;
pub const STATE: usize = 0xc0;
pub const STRATEGIC_LOCATIONS: usize = 0x140;
pub const CONTROLLER: usize = 0x188;
pub const VICTORY_POINTS: usize = 0x38;
pub const NAME_OVERRIDE: usize = 0x40;
pub const DEFINITION: usize = 0xb8;
pub const STRATEGIC_REGION: usize = 0xc8;
pub const CONTROLLER_AREA: usize = 0xd0;
pub const OCCUPANTS: usize = 0xe0;
pub const NAVAL_OCCUPANTS: usize = 0xf8;
pub const BUILDING_STATUS: usize = 0x190;
pub const BORDER_CONFLICT_TREE: usize = 0xa8;
pub const BORDER_CONFLICT_NONZERO_QWORD: usize = 0xb0;
pub const OWNER_AREA: usize = 0xd8;
pub const BORDER_WAR_SIDE_GROUP: usize = 0x4e8;

pub const MOVEMENT_BUILDINGS: usize = BUILDING_STATUS + crate::cbuilding_status::ACTIVE_BUILDINGS;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u32,
    pub state: u32,
    pub owner: u32,
    pub controller: u32,
    pub victory_points: i32,
    pub fort_positive: u8,
    pub move_gate: u8,
    pub port_sea: i32,
    pub fort_bunker: Option<i64>,
    pub controller_area: Option<u32>,
    pub owner_area: Option<u32>,
    pub area_node: Option<u32>,
}

impl Row {
    pub const MOVE_NULL_OWNER_AREA: u8 = 1;
    pub const MOVE_BORDER_CONFLICT: u8 = 2;
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ConflictRow {
    pub province: u32,
    pub groups_start: usize,
    pub groups_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConflictWalk {
    pub outcome: Outcome,
    pub groups_len: usize,
    pub required_groups: usize,
}

impl Default for ConflictWalk {
    fn default() -> Self {
        Self {
            outcome: Outcome::Unavailable {
                reason: "province conflicts not walked",
            },
            groups_len: 0,
            required_groups: 0,
        }
    }
}

#[derive(Debug, Default)]
pub struct Capture {
    pub rows: Vec<Row>,
    pub conflicts: Vec<ConflictRow>,
    pub groups: Vec<u64>,
    pub conflict_walk: ConflictWalk,
    pub areas: Vec<crate::controller_areas::Pointers>,
    pub area_nodes: crate::controller_areas::Output,
    pub area_store: Option<Box<crate::controller_areas::Store>>,
    pub province_store: Option<Box<carry::RowStore>>,
}

impl Capture {
    pub fn try_with_capacity(
        rows: usize,
        conflicts: usize,
        groups: usize,
    ) -> std::io::Result<Self> {
        Ok(Self {
            rows: crate::allocation::filled(rows, Row::default())?,
            conflicts: crate::allocation::filled(conflicts, ConflictRow::default())?,
            groups: crate::allocation::filled(groups, 0)?,
            conflict_walk: ConflictWalk::default(),
            areas: crate::allocation::filled(rows, crate::controller_areas::Pointers::default())?,
            area_nodes: crate::controller_areas::Output::default(),
            area_store: None,
            province_store: None,
        })
    }

    #[cfg(test)]
    pub fn with_capacity(rows: usize, conflicts: usize, groups: usize) -> Self {
        Self::try_with_capacity(rows, conflicts, groups).unwrap()
    }

    pub fn allocated_bytes(&self) -> usize {
        use std::mem::size_of;
        self.rows.capacity() * size_of::<Row>()
            + self.conflicts.capacity() * size_of::<ConflictRow>()
            + self.groups.capacity() * size_of::<u64>()
            + self.areas.capacity() * size_of::<crate::controller_areas::Pointers>()
            + self.area_nodes.allocated_bytes()
    }

    pub fn walk(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
    ) -> Outcome {
        let (outcome, conflicts) = walk_with_conflicts_buffer(
            memory,
            base,
            game_state,
            crate::walk::Buffer::Growing(&mut self.rows),
            ConflictBuffers {
                rows: crate::walk::Buffer::Growing(&mut self.conflicts),
                groups: crate::walk::Buffer::Growing(&mut self.groups),
            },
            crate::walk::Buffer::Growing(&mut self.areas),
            None,
        );
        self.conflict_walk = conflicts;
        outcome
    }

    pub fn walk_carrying(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        take: Option<[u64; 2]>,
    ) -> Outcome {
        let Self {
            rows,
            conflicts,
            groups,
            conflict_walk,
            areas,
            province_store,
            ..
        } = self;
        let mut hour = province_store
            .as_deref_mut()
            .map(|store| carry::Hour { store, take });
        let (outcome, walk) = walk_with_conflicts_buffer(
            memory,
            base,
            game_state,
            crate::walk::Buffer::Growing(rows),
            ConflictBuffers {
                rows: crate::walk::Buffer::Growing(conflicts),
                groups: crate::walk::Buffer::Growing(groups),
            },
            crate::walk::Buffer::Growing(areas),
            hour.as_mut(),
        );
        *conflict_walk = walk;
        outcome
    }

    pub fn refresh_carried_areas(
        &mut self,
        memory: &impl Memory,
        keep: &dyn Fn(u32) -> bool,
    ) -> Result<(), &'static str> {
        match self.province_store.as_deref_mut() {
            Some(store) => store.refresh_areas(memory, &self.rows, &mut self.areas, keep),
            None => Ok(()),
        }
    }

    pub fn carried_rows(&self) -> usize {
        self.province_store
            .as_deref()
            .map_or(0, carry::RowStore::carried_rows)
    }
}

struct ConflictBuffers<'a> {
    rows: crate::walk::Buffer<'a, ConflictRow>,
    groups: crate::walk::Buffer<'a, u64>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NameRow {
    pub province: u32,
    pub start: usize,
    pub len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Names {
    pub outcome: Outcome,
    pub bytes_len: usize,
    pub required_bytes: usize,
}

pub fn count_names(memory: &impl Memory, base: usize, game_state: usize) -> Names {
    walk_names_buffer(memory, base, game_state, crate::walk::Buffer::Count, crate::walk::Buffer::Count)
}

pub fn walk_names(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<NameRow>,
    bytes: &mut Vec<u8>,
) -> Names {
    walk_names_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(output),
        crate::walk::Buffer::Growing(bytes),
    )
}

pub fn walk_names_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: crate::walk::Buffer<'_, NameRow>,
    mut bytes: crate::walk::Buffer<'_, u8>,
) -> Names {
    use crate::{msvc_string, read, read_slice};

    let mut used = 0_usize;
    let mut required = 0_usize;
    let result = (|| {
        let root = game_state
            .checked_add(cgame_state::PROVINCES)
            .ok_or("province names root address overflow")?;
        let array =
            Array::read(memory, root, 8).ok_or("province names array unreadable or invalid")?;
        if array.count < 2 {
            return Err("province names array has no province slots");
        }
        let expected = engine_sites::at(base, VTABLE_RVA)
            .ok_or("province names module address overflow")? as u64;
        let mut len = 0_usize;
        let mut missing = 0_u32;
        let mut found_province = false;
        for slot in 1..array.count {
            let pointer = array
                .element(slot, 8)
                .and_then(|at| read::<u64>(memory, at))
                .and_then(|p| usize::try_from(p).ok())
                .ok_or("province names pointer unreadable")?;
            if pointer == 0 {
                continue;
            }
            let at = |offset| {
                pointer
                    .checked_add(offset)
                    .ok_or("province names field address overflow")
            };
            if read::<u64>(memory, pointer) != Some(expected)
                || read::<u32>(memory, at(ID)?) != Some(slot)
            {
                return Err("province names identity mismatch or unreadable");
            }
            found_province = true;
            let string = at(NAME_OVERRIDE)?;
            let head = read::<[u8; 32]>(memory, string).ok_or("province name header unreadable")?;
            let word = |at| u64::from_le_bytes(head[at..at + 8].try_into().unwrap());
            let length = word(msvc_string::LENGTH);
            let capacity = word(msvc_string::CAPACITY);
            if length > capacity || capacity < msvc_string::INLINE_CAPACITY {
                return Err("province name header invalid");
            }
            let length = usize::try_from(length).map_err(|_| "province name length overflow")?;
            let inline = capacity == msvc_string::INLINE_CAPACITY;
            let source = if inline {
                string
            } else {
                usize::try_from(word(0))
                    .ok()
                    .filter(|p| *p != 0)
                    .ok_or("province name heap pointer invalid")?
            };
            let terminated = if inline {
                head.get(length) == Some(&0)
            } else {
                source
                    .checked_add(length)
                    .and_then(|at| read::<u8>(memory, at))
                    == Some(0)
            };
            if !terminated {
                return Err("province name terminator invalid or unreadable");
            }
            if length == 0 {
                continue;
            }
            required = required
                .checked_add(length)
                .ok_or("province name arena size overflow")?;
            output.ensure(
                len.checked_add(1)
                    .ok_or("province name row count overflow")?,
            )?;
            bytes.ensure(required)?;
            if len == output.len() || length > bytes.len().saturating_sub(used) {
                missing = missing
                    .checked_add(1)
                    .ok_or("province name row count overflow")?;
                continue;
            }
            if inline {
                bytes[used..used + length].copy_from_slice(&head[..length]);
            } else if !read_slice(memory, source, &mut bytes[used..used + length]) {
                return Err("province name bytes unreadable");
            }
            output[len] = NameRow {
                province: slot,
                start: used,
                len: length,
            };
            used += length;
            len += 1;
        }
        if !found_province {
            return Err("province names census contains no provinces");
        }
        Ok(if missing == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: missing,
            }
        })
    })();
    Names {
        outcome: result.unwrap_or_else(|reason| Outcome::Unavailable { reason }),
        bytes_len: used,
        required_bytes: required,
    }
}

const BATCH_ROWS: usize = 64;
const UNIT_ORDER_FIELDS: [RecordField; 4] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: ID,
        destination_offset: 8,
        len: 4,
    },
    RecordField {
        source_offset: OCCUPANTS,
        destination_offset: 12,
        len: 16,
    },
    RecordField {
        source_offset: NAVAL_OCCUPANTS,
        destination_offset: 28,
        len: 16,
    },
];
const UNIT_ORDER_ROW_BYTES: usize = 44;
const UNIT_ORDER_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&UNIT_ORDER_FIELDS, UNIT_ORDER_ROW_BYTES).expect("valid record layout");
const FIELDS_PER_ROW: usize = 5;
const FIELD_RECORD_BYTES: usize = FIELDS_PER_ROW * 8;
const DENSE_FIELDS: [RecordField; FIELDS_PER_ROW] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: ID,
        destination_offset: 8,
        len: 4,
    },
    RecordField {
        source_offset: STATE,
        destination_offset: 16,
        len: 8,
    },
    RecordField {
        source_offset: CONTROLLER,
        destination_offset: 24,
        len: 4,
    },
    RecordField {
        source_offset: VICTORY_POINTS,
        destination_offset: 32,
        len: 4,
    },
];
const DENSE_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&DENSE_FIELDS, FIELD_RECORD_BYTES).expect("valid record layout");

#[derive(Clone, Copy, Default)]
struct StateEntry {
    pointer: usize,
    id: u32,
    owner: u32,
}

struct StateCache(Vec<StateEntry>);

impl StateCache {
    fn new(demand: usize) -> Result<Self, &'static str> {
        let slots = if demand == 0 {
            0
        } else {
            demand
                .checked_next_power_of_two()
                .ok_or("province state cache extent overflow")?
        };
        crate::allocation::default_vec(slots)
            .map(Self)
            .map_err(|_| "province state cache allocation failed")
    }

    fn slot(&self, pointer: usize) -> usize {
        let mixed = (pointer as u64 ^ (pointer as u64 >> 32)).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        (mixed >> 32) as usize & (self.0.len() - 1)
    }

    fn resolve(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: usize,
    ) -> Result<(u32, u32), &'static str> {
        let first = if self.0.is_empty() {
            0
        } else {
            self.slot(pointer)
        };
        let mut vacant = None;
        for step in 0..self.0.len() {
            let slot = (first + step) & (self.0.len() - 1);
            let cached = self.0[slot];
            if cached.pointer == pointer {
                return Ok((cached.id, cached.owner));
            }
            if cached.pointer == 0 {
                vacant = Some(slot);
                break;
            }
        }
        let vtable = engine_sites::at(base, cstate::VTABLE_RVA)
            .ok_or("module address overflow")?;
        let mut vtable_bytes = [0; 8];
        let mut id_bytes = [0; 4];
        let mut owner_bytes = [0; 4];
        let copied = memory.copy_scattered(
            [
                pointer,
                pointer.checked_add(cstate::ID).unwrap_or(0),
                pointer.checked_add(cstate::OWNER).unwrap_or(0),
            ],
            &mut [&mut vtable_bytes, &mut id_bytes, &mut owner_bytes],
        );
        if copied == 0 || u64::from_le_bytes(vtable_bytes) != vtable as u64 {
            return Err("state vtable mismatch");
        }
        if copied < 2 {
            return Err("state id unreadable");
        }
        let id = u32::from_le_bytes(id_bytes);
        if id == 0 {
            return Err("non-null state has zero id");
        }
        if copied < 3 {
            return Err("state owner unreadable");
        }
        let owner = u32::from_le_bytes(owner_bytes);
        if let Some(slot) = vacant {
            self.0[slot] = StateEntry { pointer, id, owner };
        }
        Ok((id, owner))
    }
}

struct ProvinceBatch {
    pointers: [[u8; 8]; BATCH_ROWS],
    fields: [[u8; 8]; BATCH_ROWS * FIELDS_PER_ROW],
    pointer_count: usize,
    field_count: usize,
    len: usize,
}

impl ProvinceBatch {
    fn new() -> Self {
        Self {
            pointers: [[0; 8]; BATCH_ROWS],
            fields: [[0; 8]; BATCH_ROWS * FIELDS_PER_ROW],
            pointer_count: 0,
            field_count: 0,
            len: 0,
        }
    }

    fn read(&mut self, memory: &impl Memory, array: Array, first: u32, capacity: usize) {
        self.read_pointers(memory, array, first);
        let mut remaining = capacity;
        let enabled: [bool; BATCH_ROWS] = std::array::from_fn(|slot| {
            if slot >= self.pointer_count || self.pointer(slot) == 0 || remaining == 0 {
                false
            } else {
                remaining -= 1;
                true
            }
        });
        if !enabled.iter().any(|&value| value) {
            self.field_count = BATCH_ROWS * FIELDS_PER_ROW;
            return;
        }
        if self.pointer_count == self.len && enabled[..self.len].iter().all(|&value| value) {
            let pointers: [usize; BATCH_ROWS] = std::array::from_fn(|slot| {
                if slot < self.len {
                    self.pointer(slot)
                } else {
                    0
                }
            });
            let copied = memory.copy_records(
                RecordSources::Pointers(&pointers[..self.len]),
                &DENSE_LAYOUT,
                &mut self.fields.as_flattened_mut()[..self.len * FIELD_RECORD_BYTES],
            );
            self.field_count = if copied == self.len * FIELDS_PER_ROW {
                BATCH_ROWS * FIELDS_PER_ROW
            } else {
                copied
            };
            return;
        }
        let addresses: [usize; BATCH_ROWS * FIELDS_PER_ROW] = std::array::from_fn(|field| {
            let slot = field / FIELDS_PER_ROW;
            if enabled[slot] {
                self.pointer(slot)
                    .checked_add([0, ID, STATE, CONTROLLER, VICTORY_POINTS][field % FIELDS_PER_ROW])
                    .unwrap_or(0)
            } else {
                0
            }
        });
        let mut destinations =
            self.fields
                .each_mut()
                .into_iter()
                .enumerate()
                .map(|(field, bytes)| {
                    &mut bytes[..if enabled[field / FIELDS_PER_ROW] {
                        [8, 4, 8, 4, 4][field % FIELDS_PER_ROW]
                    } else {
                        0
                    }]
                });
        self.field_count = memory.copy_scattered(
            addresses,
            &mut std::array::from_fn(|_| destinations.next().unwrap()),
        );
    }

    fn read_pointers(&mut self, memory: &impl Memory, array: Array, first: u32) {
        self.len = ((array.count - first) as usize).min(BATCH_ROWS);
        self.pointer_count = memory
            .copy_records(
                RecordSources::Strided {
                    base: array.element(first, 8).unwrap_or(0),
                    stride: 8,
                    count: self.len,
                },
                &POINTER_LAYOUT,
                &mut self.pointers.as_flattened_mut()[..self.len * 8],
            )
            .min(self.len);
    }

    fn read_bodies(&mut self, memory: &impl Memory, bodies: &[usize]) {
        if bodies.is_empty() {
            self.field_count = BATCH_ROWS * FIELDS_PER_ROW;
            return;
        }
        let copied = memory.copy_records(
            RecordSources::Pointers(bodies),
            &DENSE_LAYOUT,
            &mut self.fields.as_flattened_mut()[..bodies.len() * FIELD_RECORD_BYTES],
        );
        self.field_count = if copied == bodies.len() * FIELDS_PER_ROW {
            BATCH_ROWS * FIELDS_PER_ROW
        } else {
            copied
        };
    }

    fn pointer(&self, slot: usize) -> usize {
        u64::from_le_bytes(self.pointers[slot]) as usize
    }

    fn row(
        &self,
        memory: &impl Memory,
        base: usize,
        index: u32,
        slot: usize,
        states: &mut StateCache,
    ) -> Result<Row, &'static str> {
        self.row_body(memory, base, index, self.pointer(slot), slot, states)
    }

    fn row_body(
        &self,
        memory: &impl Memory,
        base: usize,
        index: u32,
        pointer: usize,
        body: usize,
        states: &mut StateCache,
    ) -> Result<Row, &'static str> {
        let vtable = engine_sites::at(base, VTABLE_RVA)
            .ok_or("module address overflow")?;
        let first = body * FIELDS_PER_ROW;
        if self.field_count <= first || u64::from_le_bytes(self.fields[first]) != vtable as u64 {
            return Err("province vtable mismatch");
        }
        let complete = |field: usize, offset: usize, reason| {
            pointer
                .checked_add(offset)
                .ok_or("province address overflow")?;
            (self.field_count > first + field)
                .then_some(())
                .ok_or(reason)
        };
        complete(1, ID, "province id unreadable")?;
        let id = u32::from_le_bytes(self.fields[first + 1][..4].try_into().unwrap());
        if id != index {
            return Err("province id differs from slot");
        }
        complete(2, STATE, "province state unreadable")?;
        let state = u64::from_le_bytes(self.fields[first + 2]);
        complete(3, CONTROLLER, "province controller unreadable")?;
        let controller = u32::from_le_bytes(self.fields[first + 3][..4].try_into().unwrap());
        let (state, owner) = if state == 0 {
            (0, 0)
        } else {
            states.resolve(
                memory,
                base,
                usize::try_from(state).map_err(|_| "state pointer overflow")?,
            )?
        };
        complete(4, VICTORY_POINTS, "province victory points unreadable")?;
        let victory_points = i32::from_le_bytes(self.fields[first + 4][..4].try_into().unwrap());
        Ok(Row {
            id,
            state,
            owner,
            controller,
            victory_points,
            ..Row::default()
        })
    }
}

#[derive(Clone, Copy, Default)]
struct MovementFields {
    fort_sum: i32,
    move_gate: u8,
    port_sea: i32,
    definition: usize,
    controller_area: usize,
    owner_area: usize,
}

struct PointerCache<T: Copy> {
    entries: [(usize, T); 64],
    len: usize,
}

impl<T: Copy + Default> PointerCache<T> {
    fn new() -> Self {
        Self {
            entries: [(0, T::default()); 64],
            len: 0,
        }
    }

    fn get(&self, pointer: usize) -> Option<T> {
        self.entries[..self.len]
            .iter()
            .find(|(cached, _)| *cached == pointer)
            .map(|(_, value)| *value)
    }

    fn insert(&mut self, pointer: usize, value: T) {
        if self.len < self.entries.len() {
            self.entries[self.len] = (pointer, value);
            self.len += 1;
        }
    }
}

const MOVEMENT_FIELDS: [RecordField; 4] = [
    RecordField {
        source_offset: DEFINITION,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: MOVEMENT_BUILDINGS,
        destination_offset: 8,
        len: 16,
    },
    RecordField {
        source_offset: BORDER_CONFLICT_NONZERO_QWORD,
        destination_offset: 24,
        len: 8,
    },
    RecordField {
        source_offset: CONTROLLER_AREA,
        destination_offset: 32,
        len: 16,
    },
];
const MOVEMENT_BYTES: usize = 48;
const MOVEMENT_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&MOVEMENT_FIELDS, MOVEMENT_BYTES).expect("valid record layout");
const DEFINITION_FIELDS: [RecordField; 3] = [
    RecordField {
        source_offset: crate::province_definition::ID,
        destination_offset: 0,
        len: 4,
    },
    RecordField {
        source_offset: crate::province_definition::TERRAIN_FLAGS,
        destination_offset: 16,
        len: 1,
    },
    RecordField {
        source_offset: crate::province_definition::PORT_SEA,
        destination_offset: 20,
        len: 4,
    },
];
const DEFINITION_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&DEFINITION_FIELDS, 24).expect("valid record layout");
const BUILDING_FIELDS: [RecordField; 2] = [
    RecordField {
        source_offset: crate::cbuilding::HEALTHY_LEVEL,
        destination_offset: 0,
        len: 2,
    },
    RecordField {
        source_offset: crate::cbuilding::TEMPLATE,
        destination_offset: 8,
        len: 8,
    },
];
const BUILDING_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&BUILDING_FIELDS, 16).expect("valid record layout");
const POINTER_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: 8,
}];
const POINTER_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&POINTER_FIELDS, 8).expect("valid record layout");

struct MovementBatch {
    weights: PointerCache<i32>,
    building_pointers: [usize; BATCH_ROWS],
    building_indirect: [u8; BATCH_ROWS * 8],
    building_fields: [u8; BATCH_ROWS * 16],
}

impl MovementBatch {
    fn new() -> Self {
        Self {
            weights: PointerCache::new(),
            building_pointers: [0; BATCH_ROWS],
            building_indirect: [0; BATCH_ROWS * 8],
            building_fields: [0; BATCH_ROWS * 16],
        }
    }

    fn fort_weight(&mut self, memory: &impl Memory, template: usize) -> Result<i32, &'static str> {
        if let Some(weight) = self.weights.get(template) {
            return Ok(weight);
        }
        let weight = template
            .checked_add(crate::cbuilding_template::MOVEMENT_FORT_WEIGHT)
            .and_then(|address| crate::read::<i32>(memory, address))
            .ok_or("province building fort weight unreadable")?;
        self.weights.insert(template, weight);
        Ok(weight)
    }

    fn fort_sum(&mut self, memory: &impl Memory, buildings: Array) -> Result<i32, &'static str> {
        let mut sum = 0_i32;
        let mut first = 0;
        while first < buildings.count {
            let take = (buildings.count - first).min(BATCH_ROWS as u32) as usize;
            let address = buildings
                .element(first, 8)
                .ok_or("province building index invalid")?;
            buildings
                .element(first + take as u32 - 1, 8)
                .ok_or("province building index invalid")?;
            if memory.copy_records(
                RecordSources::Strided {
                    base: address,
                    stride: 8,
                    count: take,
                },
                &POINTER_LAYOUT,
                &mut self.building_indirect[..take * 8],
            ) != take
            {
                return Err("province building pointer unreadable");
            }
            for (index, pointer) in self.building_pointers[..take].iter_mut().enumerate() {
                *pointer = usize::try_from(u64::from_le_bytes(
                    self.building_indirect[index * 8..index * 8 + 8]
                        .try_into()
                        .unwrap(),
                ))
                .ok()
                .filter(|value| *value != 0)
                .ok_or("province building pointer invalid")?;
            }
            if memory.copy_records(
                RecordSources::Pointers(&self.building_pointers[..take]),
                &BUILDING_LAYOUT,
                &mut self.building_fields[..take * 16],
            ) != take * BUILDING_FIELDS.len()
            {
                return Err("province building fields unreadable");
            }
            for index in 0..take {
                let record = &self.building_fields[index * 16..index * 16 + 16];
                let healthy = i16::from_le_bytes(record[..2].try_into().unwrap());
                let template =
                    usize::try_from(u64::from_le_bytes(record[8..16].try_into().unwrap()))
                        .ok()
                        .filter(|value| *value != 0)
                        .ok_or("province building template invalid")?;
                sum = sum.wrapping_add(
                    i32::from(healthy).wrapping_mul(self.fort_weight(memory, template)?),
                );
            }
            first += take as u32;
        }
        Ok(sum)
    }

    fn read(
        &mut self,
        memory: &impl Memory,
        pointers: &[usize],
        ids: &[u32],
        output: &mut [MovementFields],
    ) -> Result<(), &'static str> {
        let count = pointers.len();
        let mut raw = [0_u8; BATCH_ROWS * MOVEMENT_BYTES];
        if memory.copy_records(
            RecordSources::Pointers(pointers),
            &MOVEMENT_LAYOUT,
            &mut raw[..count * MOVEMENT_BYTES],
        ) != count * MOVEMENT_FIELDS.len()
        {
            return Err("province movement fields unreadable");
        }
        let mut buildings = [Array { data: 0, count: 0 }; BATCH_ROWS];
        let mut definitions = [0_usize; BATCH_ROWS];
        let mut gates = [(false, false); BATCH_ROWS];
        let mut areas = [(0_usize, 0_usize); BATCH_ROWS];
        for (slot, record) in raw[..count * MOVEMENT_BYTES]
            .chunks_exact(MOVEMENT_BYTES)
            .enumerate()
        {
            definitions[slot] =
                usize::try_from(u64::from_le_bytes(record[..8].try_into().unwrap()))
                    .ok()
                    .filter(|value| *value != 0)
                    .ok_or("province definition pointer invalid")?;
            buildings[slot] = Array::from_header(record[8..24].try_into().unwrap(), 8)
                .ok_or("province building array invalid")?;
            let word = |at: usize| u64::from_le_bytes(record[at..at + 8].try_into().unwrap());
            gates[slot] = (word(24) != 0, word(40) == 0);
            areas[slot] = (word(32) as usize, word(40) as usize);
        }
        if memory.copy_records(
            RecordSources::Pointers(&definitions[..count]),
            &DEFINITION_LAYOUT,
            &mut raw[..count * 24],
        ) != count * DEFINITION_FIELDS.len()
        {
            return Err("province definition fields unreadable");
        }
        for (slot, record) in raw[..count * 24].chunks_exact(24).enumerate() {
            if u32::from_le_bytes(record[..4].try_into().unwrap()) != ids[slot] {
                return Err("province definition identity mismatch");
            }
            let flags = record[16];
            let (conflict, null_area) = gates[slot];
            output[slot] = MovementFields {
                fort_sum: 0,
                move_gate: (u8::from(flags & 3 == 1 && null_area) * Row::MOVE_NULL_OWNER_AREA)
                    | (u8::from(conflict) * Row::MOVE_BORDER_CONFLICT),
                port_sea: i32::from_le_bytes(record[20..24].try_into().unwrap()),
                definition: definitions[slot],
                controller_area: areas[slot].0,
                owner_area: areas[slot].1,
            };
        }
        for (slot, fields) in output[..count].iter_mut().enumerate() {
            fields.fort_sum = self.fort_sum(memory, buildings[slot])?;
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
) -> Outcome {
    walk_with_conflicts_buffer(memory, base, game_state, crate::walk::Buffer::Growing(output), ConflictBuffers {
            rows: crate::walk::Buffer::Count,
            groups: crate::walk::Buffer::Count,
        }, crate::walk::Buffer::Count, None)
    .0
}

fn walk_with_conflicts_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: crate::walk::Buffer<'_, Row>,
    mut conflicts: ConflictBuffers<'_>,
    mut areas: crate::walk::Buffer<'_, crate::controller_areas::Pointers>,
    carry: Option<&mut carry::Hour<'_>>,
) -> (Outcome, ConflictWalk) {
    let mut conflict = ConflictState::default();
    let outcome = walk_rows(
        memory,
        base,
        game_state,
        output,
        &mut areas,
        &mut conflicts,
        &mut conflict,
        carry,
    );
    let walk = match (outcome, conflict.failure) {
        (Outcome::Unavailable { reason }, _) | (_, Some(reason)) => ConflictWalk {
            outcome: Outcome::Unavailable { reason },
            groups_len: 0,
            required_groups: conflict.required_groups,
        },
        _ => ConflictWalk {
            outcome: if conflict.unread == 0 {
                Outcome::Rows { len: conflict.len }
            } else {
                Outcome::Truncated {
                    len: conflict.len,
                    unread_rows: conflict.unread,
                }
            },
            groups_len: conflict.groups_len,
            required_groups: conflict.required_groups,
        },
    };
    (outcome, walk)
}

#[derive(Default)]
struct ConflictState {
    len: usize,
    unread: u32,
    groups_len: usize,
    required_groups: usize,
    failure: Option<&'static str>,
}

pub fn conflict_group_demand(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<usize, &'static str> {
    let array = game_state
        .checked_add(cgame_state::PROVINCES)
        .and_then(|address| Array::read(memory, address, 8))
        .filter(|array| array.count >= 2)
        .ok_or("border conflict demand root invalid")?;
    let vtable = engine_sites::at(base, VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    let mut required = 0_usize;
    crate::cpdx_array::pointers(
        memory,
        array
            .slice(1, array.count - 1, 8)
            .map_err(|_| "border conflict demand slots overflow")?,
        "border conflict demand pointer unreadable",
        |slot, pointer| {
            if pointer == 0 {
                return Ok(());
            }
            if crate::read::<u64>(memory, pointer) != Some(vtable)
                || pointer
                    .checked_add(ID)
                    .and_then(|address| crate::read::<u32>(memory, address))
                    != Some(slot + 1)
            {
                return Err("border conflict demand identity mismatch");
            }
            let size = pointer
                .checked_add(BORDER_CONFLICT_NONZERO_QWORD)
                .and_then(|address| crate::read::<u64>(memory, address))
                .and_then(|size| usize::try_from(size).ok())
                .ok_or("border conflict demand size unreadable")?;
            required = size
                .checked_mul(2)
                .and_then(|size| required.checked_add(size))
                .ok_or("border conflict demand size overflow")?;
            Ok(())
        },
    )?;
    Ok(required)
}

impl ConflictState {
    fn record(
        &mut self,
        memory: &impl Memory,
        base: usize,
        province: usize,
        id: u32,
        output: &mut ConflictBuffers<'_>,
    ) {
        if self.failure.is_some() {
            return;
        }
        let start = self.groups_len;
        let admitted =
            match border_conflict_groups(memory, base, province, &mut output.groups, start) {
                Ok(admitted) => admitted,
                Err(reason) => {
                    self.failure = Some(reason);
                    return;
                }
            };
        self.required_groups = self.required_groups.saturating_add(admitted);
        if self.len < output.rows.len() && admitted <= output.groups.len() - start {
            output.rows[self.len] = ConflictRow {
                province: id,
                groups_start: start,
                groups_len: admitted,
            };
            self.len += 1;
            self.groups_len = start + admitted;
        } else {
            self.unread = self.unread.saturating_add(1);
        }
    }
}

fn border_conflict_groups(
    memory: &impl Memory,
    base: usize,
    province: usize,
    output: &mut crate::walk::Buffer<'_, u64>,
    start: usize,
) -> Result<usize, &'static str> {
    use crate::{memory::read, msvc_tree_node as node};
    let at = |pointer: usize, offset: usize| {
        pointer
            .checked_add(offset)
            .ok_or("border conflict address overflow")
    };
    let pointer = |address: usize| -> Result<usize, &'static str> {
        read::<u64>(memory, address)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or("border conflict pointer unreadable")
    };
    let size = read::<u64>(memory, at(province, BORDER_CONFLICT_NONZERO_QWORD)?)
        .ok_or("border conflict size unreadable")?;
    let head = pointer(at(province, BORDER_CONFLICT_TREE)?)?;
    if output.is_growing() {
        let required = usize::try_from(size)
            .ok()
            .and_then(|size| size.checked_mul(2))
            .and_then(|size| start.checked_add(size))
            .ok_or("border conflict group count overflow")?;
        output.ensure(required)?;
    }
    let mut admitted = 0_usize;
    let expected = |rva: Rva| {
        engine_sites::at(base, rva)
            .map(|value| value as u64)
            .ok_or("module address overflow")
    };
    let combats = [
        expected(crate::cland_combat::VTABLE_RVA)?,
        expected(crate::cland_border_war_combat::VTABLE_RVA)?,
    ];
    let sides = [
        expected(crate::cland_combatant::VTABLE_RVA)?,
        expected(crate::cland_border_war_combatant::VTABLE_RVA)?,
    ];
    let groups = [
        expected(crate::corders_group::VTABLE_RVA)?,
        expected(crate::carmy_group::VTABLE_RVA)?,
    ];
    let tree_error = |error| {
        use node::SetError::*;
        match error {
            NullLink => "border conflict tree link null",
            UnreadableNode => "border conflict tree node unreadable",
            InvalidHead => "border conflict tree head is not its sentinel",
            ForeignSentinel => "border conflict tree reached a foreign sentinel",
            ExcessNodes => "border conflict tree exceeds its size",
            MissingNodes => "border conflict tree shorter than its size",
            Cycle => "border conflict tree links cycle",
            BudgetOverflow => "border conflict traversal budget overflow",
        }
    };
    node::walk_set(memory, head, size, tree_error, |payload| {
        let combat = payload as usize;
        if combat == 0 || !combats.contains(&read::<u64>(memory, combat).unwrap_or(0)) {
            return Err("border conflict combat identity mismatch");
        }
        for side in [crate::cland_combat::DEFENDER, crate::cland_combat::ATTACKER] {
            let side = pointer(at(combat, side)?)?;
            if side == 0 || !sides.contains(&read::<u64>(memory, side).unwrap_or(0)) {
                return Err("border conflict combat side identity mismatch");
            }
            let reference = read::<u64>(memory, at(side, BORDER_WAR_SIDE_GROUP)?)
                .ok_or("border conflict side reference unreadable")?;
            let Some(group) = crate::creference_registry::resolve(memory, base, reference)? else {
                continue;
            };
            let vtable =
                read::<u64>(memory, group).ok_or("border conflict group identity unreadable")?;
            if !groups.contains(&vtable) {
                continue;
            }
            let handle = read::<u64>(memory, at(group, crate::corders_group::HANDLE)?)
                .ok_or("border conflict group handle unreadable")?;
            if handle as u32 != crate::corders_group::REFERENCE_TYPE {
                return Err("border conflict group handle type mismatch");
            }
            let border_war = read::<u64>(memory, at(group, crate::corders_group::BORDER_WAR)?)
                .ok_or("border conflict group border war unreadable")?;
            if border_war != 0 {
                let span = &output[start.min(output.len())..];
                if !span[..admitted.min(span.len())].contains(&handle) {
                    if admitted < span.len() {
                        output[start + admitted] = handle;
                    }
                    admitted += 1;
                }
            }
        }
        Ok(())
    })?;
    Ok(admitted)
}

fn fort_bunker<'a, M: Memory>(
    memory: &'a M,
    base: usize,
    game_state: usize,
    province: usize,
    controller: u32,
    countries: &mut Option<Result<crate::ccountry::Reader<'a, M>, &'static str>>,
) -> Option<i64> {
    let state = province
        .checked_add(STATE)
        .and_then(|address| crate::read::<u64>(memory, address))
        .and_then(|state| usize::try_from(state).ok())?;
    if state == 0 {
        return Some(0);
    }
    let countries = countries
        .get_or_insert_with(|| crate::ccountry::Reader::new(memory, base, game_state))
        .as_ref()
        .ok()?;
    let country = match countries.link(controller).ok()? {
        None => countries.reserved().ok()?,
        Some(slot) => countries.read_at(slot).ok()??,
    };
    crate::cstate::movement_modifier(
        memory,
        base,
        game_state,
        state,
        country.id,
        crate::modifier_keys::LAND_BUNKER_EFFECTIVENESS_FACTOR,
    )
    .ok()
}

#[allow(clippy::too_many_arguments)]
fn walk_rows(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: crate::walk::Buffer<'_, Row>,
    areas: &mut crate::walk::Buffer<'_, crate::controller_areas::Pointers>,
    conflicts: &mut ConflictBuffers<'_>,
    conflict: &mut ConflictState,
    carry: Option<&mut carry::Hour<'_>>,
) -> Outcome {
    let prepared = (|| {
        let array = game_state
            .checked_add(cgame_state::PROVINCES)
            .and_then(|p| Array::read(memory, p, 8))
            .ok_or("province array unreadable or invalid")?;
        if array.count < 2 {
            return Err("province table has no province slots");
        }
        let required = array.count.saturating_sub(1) as usize;
        output
            .ensure(required)
            .and_then(|_| conflicts.rows.ensure(required))
            .and_then(|_| areas.ensure(required))?;
        Ok((array, StateCache::new(required.min(output.len()))?))
    })();
    let (array, mut states) = match prepared {
        Ok(prepared) => prepared,
        Err(reason) => {
            if let Some(hour) = carry {
                hour.store.forget();
            }
            return Outcome::Unavailable { reason };
        }
    };
    match carry {
        None => read_rows(
            memory,
            base,
            game_state,
            array,
            &mut output[..],
            &mut areas[..],
            conflicts,
            conflict,
            &mut states,
        ),
        Some(hour) => carry::walk(
            hour,
            memory,
            base,
            game_state,
            array,
            &mut output[..],
            &mut areas[..],
            conflicts,
            conflict,
            &mut states,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn read_rows(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    array: Array,
    output: &mut [Row],
    areas: &mut [crate::controller_areas::Pointers],
    conflicts: &mut ConflictBuffers<'_>,
    conflict: &mut ConflictState,
    states: &mut StateCache,
) -> Outcome {
    let mut batch = ProvinceBatch::new();
    let mut movement = MovementBatch::new();
    let mut countries = None;
    let mut pointers = [0_usize; BATCH_ROWS];
    let mut ids = [0_u32; BATCH_ROWS];
    let mut fields = [MovementFields::default(); BATCH_ROWS];
    let mut first = 1;
    let mut len = 0_usize;
    let mut unread_rows = 0;
    while first < array.count {
        batch.read(memory, array, first, output.len() - len);
        let admitted = len;
        let mut staged = 0;
        for slot in 0..batch.pointer_count {
            if batch.pointer(slot) == 0 {
                continue;
            }
            if len == output.len() {
                unread_rows += 1;
                continue;
            }
            match batch.row(memory, base, first + slot as u32, slot, states) {
                Ok(row) => {
                    output[len] = row;
                    pointers[staged] = batch.pointer(slot);
                    ids[staged] = row.id;
                    staged += 1;
                    len += 1;
                }
                Err(reason) => return Outcome::Unavailable { reason },
            }
        }
        if staged != 0
            && let Err(reason) = movement.read(
                memory,
                &pointers[..staged],
                &ids[..staged],
                &mut fields[..staged],
            )
        {
            return Outcome::Unavailable { reason };
        }
        for (index, ((row, fields), &pointer)) in output[admitted..len]
            .iter_mut()
            .zip(&fields[..staged])
            .zip(&pointers[..staged])
            .enumerate()
        {
            row.fort_positive = u8::from(fields.fort_sum > 0);
            row.fort_bunker = if fields.fort_sum > 0 && (row.controller as i32) > 0 {
                fort_bunker(
                    memory,
                    base,
                    game_state,
                    pointer,
                    row.controller,
                    &mut countries,
                )
            } else {
                None
            };
            row.move_gate = fields.move_gate;
            row.port_sea = fields.port_sea;
            if let Some(slot) = areas.get_mut(admitted + index) {
                *slot = crate::controller_areas::Pointers {
                    province: pointer,
                    definition: fields.definition,
                    controller: fields.controller_area,
                    owner: fields.owner_area,
                };
            }
            if fields.move_gate & Row::MOVE_BORDER_CONFLICT != 0 {
                conflict.record(memory, base, pointer, row.id, conflicts);
            }
        }
        if batch.pointer_count < batch.len {
            return Outcome::Unavailable {
                reason: if array
                    .element(first + batch.pointer_count as u32, 8)
                    .is_none()
                {
                    "table address overflow"
                } else {
                    "table pointer unreadable"
                },
            };
        }
        first += batch.len as u32;
    }
    if unread_rows != 0 {
        Outcome::Truncated { len, unread_rows }
    } else if len == 0 {
        Outcome::Unavailable {
            reason: "province table has no readable provinces",
        }
    } else {
        Outcome::Rows { len }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct GeometryProvince {
    pointer: usize,
    vtable: Option<Result<u64, &'static str>>,
    id: Option<Result<u32, &'static str>>,
    definition: Option<Result<usize, &'static str>>,
    authenticated_id: Option<u32>,
    validated_definition: Option<usize>,
    fields: crate::province_definition::GeometryFields,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GeometryCapacities {
    pub provinces: usize,
    pub neighbour_entries: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GeometryCounters {
    pub hits: u64,
    pub misses: u64,
}

#[derive(Default)]
struct HeldIdentities {
    world: u64,
    ids: std::collections::HashMap<usize, u32, std::hash::BuildHasherDefault<ProvincePairHasher>>,
}

fn geometry_memo<T: Copy>(
    counters: &mut GeometryCounters,
    slot: &mut Option<Result<T, &'static str>>,
    read: impl FnOnce() -> Result<T, &'static str>,
) -> Result<T, &'static str> {
    if let Some(value) = *slot {
        counters.hits = counters.hits.saturating_add(1);
        return value;
    }
    counters.misses = counters.misses.saturating_add(1);
    *slot.insert(read())
}

pub struct GeometryPacket {
    base: usize,
    root: Result<Array, &'static str>,
    capacities: GeometryCapacities,
    required: GeometryCapacities,
    slots: std::collections::HashMap<
        u32,
        Result<usize, &'static str>,
        std::hash::BuildHasherDefault<ProvincePairHasher>,
    >,
    indices:
        std::collections::HashMap<usize, usize, std::hash::BuildHasherDefault<ProvincePairHasher>>,
    provinces: Vec<GeometryProvince>,
    neighbours: Vec<crate::province_neighbour::GeometryFields>,
    counters: GeometryCounters,
    held: HeldIdentities,
    holding: bool,
    held_counts: [u64; 2],
}

impl Default for GeometryPacket {
    fn default() -> Self {
        Self::vacant()
    }
}

impl GeometryPacket {
    pub fn try_with_capacities(capacities: GeometryCapacities) -> std::io::Result<Self> {
        let mut packet = Self::vacant();
        packet.grow(capacities)?;
        Ok(packet)
    }

    #[cfg(test)]
    pub fn with_capacities(capacities: GeometryCapacities) -> Self {
        Self::try_with_capacities(capacities).unwrap()
    }

    pub fn vacant() -> Self {
        Self {
            base: 0,
            root: Err("unit geometry is not initialized"),
            capacities: GeometryCapacities {
                provinces: 0,
                neighbour_entries: 0,
            },
            required: GeometryCapacities {
                provinces: 0,
                neighbour_entries: 0,
            },
            slots: std::collections::HashMap::with_hasher(Default::default()),
            indices: std::collections::HashMap::with_hasher(Default::default()),
            provinces: Vec::new(),
            neighbours: Vec::new(),
            counters: GeometryCounters::default(),
            held: HeldIdentities::default(),
            holding: false,
            held_counts: [0; 2],
        }
    }

    pub fn hold_identities(&mut self, world: u64) {
        if self.held.world != world {
            self.held.ids.clear();
            self.held.world = world;
        }
        self.holding = true;
    }

    pub fn held_counts(&self) -> [u64; 2] {
        self.held_counts
    }

    pub fn grow(&mut self, requested: GeometryCapacities) -> std::io::Result<()> {
        let capacities = GeometryCapacities {
            provinces: self.capacities.provinces.max(requested.provinces),
            neighbour_entries: self
                .capacities
                .neighbour_entries
                .max(requested.neighbour_entries),
        };
        let slots = capacities.provinces.saturating_sub(self.slots.len());
        let indices = capacities.provinces.saturating_sub(self.indices.len());
        let provinces = capacities.provinces.saturating_sub(self.provinces.len());
        let neighbours = capacities
            .neighbour_entries
            .saturating_sub(self.neighbours.len());
        crate::allocation::reserve_map(&mut self.slots, slots)?;
        crate::allocation::reserve_map(&mut self.indices, indices)?;
        crate::allocation::reserve_exact(&mut self.provinces, provinces)?;
        crate::allocation::reserve_exact(&mut self.neighbours, neighbours)?;
        self.capacities = capacities;
        Ok(())
    }

    pub fn required_capacities(&self) -> GeometryCapacities {
        self.required
    }

    pub fn counters(&self) -> GeometryCounters {
        self.counters
    }

    pub fn ready(&self) -> Result<(), &'static str> {
        self.root.map(|_| ())
    }

    pub fn province_count(&self) -> Result<usize, &'static str> {
        self.root.map(|root| root.count as usize)
    }

    pub fn overflowed(&self) -> bool {
        self.required.provinces > self.capacities.provinces
            || self.required.neighbour_entries > self.capacities.neighbour_entries
    }

    pub fn allocated_bytes(&self) -> usize {
        self.provinces.capacity() * std::mem::size_of::<GeometryProvince>()
            + self.neighbours.capacity()
                * std::mem::size_of::<crate::province_neighbour::GeometryFields>()
            + self.slots.capacity()
                * (std::mem::size_of::<(u32, Result<usize, &'static str>)>() + 1)
            + self.indices.capacity() * (std::mem::size_of::<(usize, usize)>() + 1)
            + self.held.ids.capacity() * (std::mem::size_of::<(usize, u32)>() + 1)
    }

    pub fn clear(&mut self) {
        self.base = 0;
        self.root = Err("unit geometry is not initialized");
        self.slots.clear();
        self.indices.clear();
        self.provinces.clear();
        self.neighbours.clear();
        self.holding = false;
        self.held_counts = [0; 2];
        self.counters = GeometryCounters::default();
        self.required = GeometryCapacities {
            provinces: 0,
            neighbour_entries: 0,
        };
    }

    pub fn begin(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        self.clear();
        self.base = base;
        self.root = state
            .checked_add(cgame_state::PROVINCES)
            .and_then(|address| crate::read(memory, address))
            .and_then(|header| Array::from_header(header, 8))
            .ok_or("unit geometry province root invalid");
        if let Ok(root) = self.root {
            self.grow(GeometryCapacities {
                provinces: root.count as usize,
                neighbour_entries: self.capacities.neighbour_entries,
            })
            .map_err(|_| "unit geometry allocation failed")?;
        }
        self.root.map(|_| ())
    }

    fn slot(&mut self, memory: &impl Memory, id: u32) -> Result<usize, &'static str> {
        self.slot_from_batch(memory, id, None)
    }

    fn slot_from_batch(
        &mut self,
        memory: &impl Memory,
        id: u32,
        batch: Option<&IdentityBatch>,
    ) -> Result<usize, &'static str> {
        if let Some(value) = self.slots.get(&id) {
            self.counters.hits = self.counters.hits.saturating_add(1);
            return *value;
        }
        self.counters.misses = self.counters.misses.saturating_add(1);
        self.required.provinces = self
            .required
            .provinces
            .max(self.slots.len().saturating_add(1));
        if self.required.provinces > self.capacities.provinces {
            self.grow(self.required)
                .map_err(|_| "unit geometry allocation failed")?;
        }
        let value = batch
            .and_then(|batch| batch.slot(id))
            .map(Ok)
            .unwrap_or_else(|| {
                self.root.and_then(|array| {
                    array
                        .element(id, 8)
                        .and_then(|address| crate::read::<u64>(memory, address))
                        .map(|pointer| pointer as usize)
                        .ok_or("unit geometry province outside census or unreadable")
                })
            });
        self.slots.insert(id, value);
        value
    }

    fn index(&mut self, pointer: usize) -> Result<usize, &'static str> {
        if let Some(&index) = self.indices.get(&pointer) {
            return Ok(index);
        }
        self.required.provinces = self
            .required
            .provinces
            .max(self.provinces.len().saturating_add(1));
        if self.required.provinces > self.capacities.provinces {
            self.grow(self.required)
                .map_err(|_| "unit geometry allocation failed")?;
        }
        let index = self.provinces.len();
        self.provinces.push(GeometryProvince {
            pointer,
            ..GeometryProvince::default()
        });
        self.indices.insert(pointer, index);
        Ok(index)
    }

    fn identity(
        &mut self,
        memory: &impl Memory,
        pointer: usize,
        allow_zero: bool,
        known_slot: Option<u32>,
    ) -> Result<(u32, usize), &'static str> {
        self.identity_from_batch(memory, pointer, allow_zero, known_slot, None)
    }

    fn identity_from_batch(
        &mut self,
        memory: &impl Memory,
        pointer: usize,
        allow_zero: bool,
        known_slot: Option<u32>,
        batch: Option<&IdentityBatch>,
    ) -> Result<(u32, usize), &'static str> {
        self.root?;
        let index = self.index(pointer)?;
        if let Some(id) = self.provinces[index].authenticated_id {
            self.counters.hits = self.counters.hits.saturating_add(2);
            if id == 0 && !allow_zero {
                return Err("unit geometry province identity mismatch");
            }
            if known_slot != Some(id) {
                self.counters.hits = self.counters.hits.saturating_add(1);
            }
            return Ok((id, index));
        }
        if self.holding
            && let Some(&id) = self.held.ids.get(&pointer)
        {
            self.held_counts[0] = self.held_counts[0].saturating_add(1);
            self.provinces[index].authenticated_id = Some(id);
            return Ok((id, index));
        }
        let province = &mut self.provinces[index];
        let (vtable, gathered) = if province.vtable.is_none() && province.id.is_none() {
            let (mut vtable, mut id) = ([0_u8; 8], [0_u8; 4]);
            let read = match batch.and_then(|batch| batch.identity(pointer)) {
                Some((table, value)) => {
                    vtable.copy_from_slice(&table.to_le_bytes());
                    id.copy_from_slice(&value.to_le_bytes());
                    2
                }
                None => memory.copy_scattered_prefetched(
                    [pointer, pointer.checked_add(ID).unwrap_or(0)],
                    &mut [&mut vtable, &mut id],
                ),
            };
            let unreadable = "unit geometry field unreadable";
            let vtable = if read > 0 {
                Ok(u64::from_le_bytes(vtable))
            } else {
                Err(unreadable)
            };
            province.vtable = Some(vtable);
            if read > 0 {
                province.id = Some(if read > 1 {
                    Ok(u32::from_le_bytes(id))
                } else {
                    Err(unreadable)
                });
            }
            self.counters.misses = self.counters.misses.saturating_add(1);
            (vtable, read > 0)
        } else {
            (
                geometry_memo(&mut self.counters, &mut province.vtable, || {
                    geometry_field(memory, pointer, 0)
                }),
                false,
            )
        };
        if engine_sites::at(self.base, VTABLE_RVA).map(|value| value as u64) != Some(vtable?) {
            return Err("unit geometry province identity mismatch");
        }
        let id = match self.provinces[index].id {
            Some(id) if gathered => {
                self.counters.misses = self.counters.misses.saturating_add(1);
                id
            }
            _ => geometry_memo(&mut self.counters, &mut self.provinces[index].id, || {
                geometry_field(memory, pointer, ID)
            }),
        }?;
        if id == 0 && !allow_zero {
            return Err("unit geometry province identity mismatch");
        }
        if known_slot != Some(id) && self.slot_from_batch(memory, id, batch)? != pointer {
            return Err("unit geometry province differs from census");
        }
        self.provinces[index].authenticated_id = Some(id);
        if self.holding && id != 0 {
            self.held_counts[1] = self.held_counts[1].saturating_add(1);
            let ids = &mut self.held.ids;
            if crate::allocation::advisory(|| crate::allocation::reserve_map(&mut *ids, 1)).is_ok()
            {
                ids.insert(pointer, id);
            }
        }
        Ok((id, index))
    }

    pub fn acquire_province_id(
        &mut self,
        memory: &impl Memory,
        pointer: usize,
    ) -> Result<u32, &'static str> {
        if pointer == 0 {
            return Ok(0);
        }
        self.identity(memory, pointer, false, None)
            .map(|(id, _)| id)
    }

    pub fn stage_province_ids(
        &self,
        memory: &impl Memory,
        selected: &[usize],
        batch: &mut IdentityBatch,
    ) {
        let mut pointers = [0_usize; 64];
        let mut len = 0;
        for &pointer in selected {
            if pointer != 0
                && !pointers[..len].contains(&pointer)
                && !(self.holding && self.held.ids.contains_key(&pointer))
                && self.indices.get(&pointer).is_none_or(|&index| {
                    let province = &self.provinces[index];
                    province.vtable.is_none()
                        && province.id.is_none()
                        && province.authenticated_id.is_none()
                })
            {
                pointers[len] = pointer;
                len += 1;
            }
        }
        if let Ok(root) = self.root {
            batch.capture(memory, self.base, root, &pointers[..len], |id| {
                !self.slots.contains_key(&id)
            });
        }
    }

    pub fn acquire_province_id_batched(
        &mut self,
        memory: &impl Memory,
        pointer: usize,
        batch: &IdentityBatch,
    ) -> Result<u32, &'static str> {
        if pointer == 0 {
            return Ok(0);
        }
        self.identity_from_batch(memory, pointer, false, None, Some(batch))
            .map(|(id, _)| id)
    }

    #[cfg(test)]
    pub fn acquire_pointer(
        &mut self,
        memory: &impl Memory,
        id: u32,
    ) -> Result<usize, &'static str> {
        let pointer = self.slot(memory, id)?;
        if id == 0 || pointer == 0 || self.identity(memory, pointer, false, Some(id))?.0 != id {
            return Err("unit geometry province identity mismatch");
        }
        Ok(pointer)
    }

    #[cfg(test)]
    pub fn acquire_route_pointer(
        &mut self,
        memory: &impl Memory,
        id: u32,
    ) -> Result<usize, &'static str> {
        let pointer = self.slot(memory, id)?;
        if pointer != 0 && self.identity(memory, pointer, true, Some(id))?.0 != id {
            return Err("unit geometry province identity mismatch");
        }
        Ok(pointer)
    }

    #[cfg(test)]
    pub fn acquire_definition(
        &mut self,
        memory: &impl Memory,
        id: u32,
    ) -> Result<usize, &'static str> {
        self.definition(memory, id)
            .map(|(_, definition)| definition)
    }

    fn definition(
        &mut self,
        memory: &impl Memory,
        id: u32,
    ) -> Result<(usize, usize), &'static str> {
        let pointer = self.slot(memory, id)?;
        if pointer == 0 {
            return Err("unit geometry province definition mismatch");
        }
        let (observed_id, index) = self.identity(memory, pointer, true, Some(id))?;
        if observed_id != id {
            return Err("unit geometry province identity mismatch");
        }
        if let Some(definition) = self.provinces[index].validated_definition {
            self.counters.hits = self.counters.hits.saturating_add(2);
            return Ok((index, definition));
        }
        let definition = geometry_memo(
            &mut self.counters,
            &mut self.provinces[index].definition,
            || geometry_field::<u64>(memory, pointer, DEFINITION).map(|value| value as usize),
        )?;
        if definition == 0 {
            return Err("unit geometry province definition mismatch");
        }
        let definition_id = geometry_memo(
            &mut self.counters,
            &mut self.provinces[index].fields.id,
            || geometry_field(memory, definition, crate::province_definition::ID),
        )?;
        if definition_id != id {
            return Err("unit geometry province definition mismatch");
        }
        self.provinces[index].validated_definition = Some(definition);
        Ok((index, definition))
    }

    #[cfg(test)]
    pub fn acquire_flags(&mut self, memory: &impl Memory, id: u32) -> Result<u8, &'static str> {
        let (index, definition) = self.definition(memory, id)?;
        geometry_memo(
            &mut self.counters,
            &mut self.provinces[index].fields.flags,
            || {
                geometry_field(
                    memory,
                    definition,
                    crate::province_definition::TERRAIN_FLAGS,
                )
            },
        )
    }

    fn adjacency(&mut self, memory: &impl Memory, id: u32) -> Result<(usize, Array), &'static str> {
        let (index, definition) = self.definition(memory, id)?;
        let header = geometry_memo(
            &mut self.counters,
            &mut self.provinces[index].fields.neighbours,
            || geometry_field(memory, definition, crate::province_definition::NEIGHBOURS),
        )?;
        let array = Array::from_header(header, crate::province_neighbour::STRIDE)
            .ok_or("unit geometry adjacency header invalid")?;
        Ok((index, array))
    }

    fn neighbour_match(
        &mut self,
        memory: &impl Memory,
        from: u32,
        target: u32,
        negative_absent: bool,
    ) -> Result<Option<(usize, usize, usize)>, &'static str> {
        let (province, neighbours) = self.adjacency(memory, from)?;
        if negative_absent && (target as i32) < 0 {
            return Ok(None);
        }
        let additional =
            (neighbours.count as usize).saturating_sub(self.provinces[province].fields.prefix_len);
        let required = self
            .neighbours
            .len()
            .checked_add(additional)
            .ok_or("unit geometry adjacency demand overflow")?;
        self.required.neighbour_entries = self.required.neighbour_entries.max(required);
        self.grow(GeometryCapacities {
            provinces: self.capacities.provinces,
            neighbour_entries: required,
        })
        .map_err(|_| "unit geometry allocation failed")?;
        let mut link = self.provinces[province].fields.prefix_start;
        let mut previous: Option<usize> = None;
        for slot in 0..neighbours.count as usize {
            let address = neighbours
                .element(slot as u32, crate::province_neighbour::STRIDE)
                .ok_or("unit geometry adjacency overflow")?;
            let index = if let Some(index) = link {
                index
            } else {
                self.required.neighbour_entries = self
                    .required
                    .neighbour_entries
                    .max(self.neighbours.len().saturating_add(1));
                let index = self.neighbours.len();
                self.neighbours
                    .push(crate::province_neighbour::GeometryFields::default());
                if let Some(previous) = previous {
                    self.neighbours[previous].next = Some(index);
                } else {
                    self.provinces[province].fields.prefix_start = Some(index);
                }
                self.provinces[province].fields.prefix_len += 1;
                index
            };
            let id = geometry_memo(&mut self.counters, &mut self.neighbours[index].id, || {
                geometry_field(memory, address, crate::province_neighbour::PROVINCE_ID)
            })?;
            if id == target {
                return Ok(Some((index, slot, address)));
            }
            previous = Some(index);
            link = self.neighbours[index].next;
        }
        Ok(None)
    }

    pub fn acquire_crossing(
        &mut self,
        memory: &impl Memory,
        from: u32,
        to: u32,
    ) -> Result<Option<u8>, &'static str> {
        let Some((index, _, address)) = self.neighbour_match(memory, from, to, false)? else {
            return Ok(None);
        };
        geometry_memo(
            &mut self.counters,
            &mut self.neighbours[index].crossing,
            || geometry_field(memory, address, crate::province_neighbour::CROSSING_KIND),
        )
        .map(Some)
    }
}

pub mod carry;
pub(crate) mod static_geometry;
pub use static_geometry::{
    StaticCounters, StaticEdge, StaticGeometry, StaticGeometryKey, StaticProvince,
    static_geometry_demand_key,
};

#[derive(Default)]
struct ProvincePairHasher(u64);

impl std::hash::Hasher for ProvincePairHasher {
    fn finish(&self) -> u64 {
        let value = (self.0 ^ (self.0 >> 32)).wrapping_mul(0xd6e8_feb8_6659_fd93);
        value ^ (value >> 32)
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.write_u64(u64::from(byte));
        }
    }

    fn write_u64(&mut self, value: u64) {
        self.0 = (self.0 ^ value).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }
    fn write_u32(&mut self, value: u32) {
        self.write_u64(u64::from(value));
    }
    fn write_usize(&mut self, value: usize) {
        self.write_u64(value as u64);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MovementProvince {
    pub state: Result<usize, &'static str>,
    pub controller: Result<u32, &'static str>,
    pub controller_area: Result<usize, &'static str>,
    pub strategic_region: Result<usize, &'static str>,
    pub terrain_flags: Result<u8, &'static str>,
}
impl Default for MovementProvince {
    fn default() -> Self {
        Self {
            state: Ok(0),
            controller: Ok(0),
            controller_area: Ok(0),
            strategic_region: Ok(0),
            terrain_flags: Ok(crate::province_definition::LAND_FLAG),
        }
    }
}
impl MovementProvince {
    pub fn land(self) -> Result<bool, &'static str> {
        self.terrain_flags
            .map(|flags| flags & crate::province_definition::LAND_FLAG != 0)
    }
}

pub fn movement_province(
    memory: &impl Memory,
    base: usize,
    provinces: Array,
    id: u32,
) -> Result<MovementProvince, &'static str> {
    let entry = provinces
        .element(id, 8)
        .ok_or("movement province outside the province array")?;
    let pointer = crate::read::<u64>(memory, entry)
        .and_then(|pointer| usize::try_from(pointer).ok())
        .ok_or("movement province entry unreadable")?;
    let field = |offset: usize| -> Result<u64, &'static str> {
        pointer
            .checked_add(offset)
            .and_then(|address| crate::read::<u64>(memory, address))
            .ok_or("movement province field unreadable")
    };
    if pointer == 0
        || Some(field(0)?) != engine_sites::at(base, VTABLE_RVA).map(|p| p as u64)
        || pointer
            .checked_add(ID)
            .and_then(|address| crate::read::<u32>(memory, address))
            != Some(id)
    {
        return Err("movement province identity mismatch");
    }
    let terrain_flags = field(DEFINITION).and_then(|definition| {
        (definition as usize)
            .checked_add(crate::province_definition::TERRAIN_FLAGS)
            .filter(|_| definition != 0)
            .and_then(|address| crate::read::<u8>(memory, address))
            .ok_or("movement province definition flags unreadable")
    });
    Ok(MovementProvince {
        state: field(STATE).map(|value| value as usize),
        controller: pointer
            .checked_add(CONTROLLER)
            .and_then(|address| crate::read::<u32>(memory, address))
            .ok_or("movement province controller unreadable"),
        controller_area: field(CONTROLLER_AREA).map(|value| value as usize),
        strategic_region: field(STRATEGIC_REGION).map(|value| value as usize),
        terrain_flags,
    })
}

const MOVEMENT_PROVINCE_ID_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: ID,
    destination_offset: 0,
    len: 4,
}];
const MOVEMENT_PROVINCE_ID_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&MOVEMENT_PROVINCE_ID_FIELDS, 4).expect("valid record layout");
const MOVEMENT_PROVINCE_FIELDS: [RecordField; 5] = [
    RecordField {
        source_offset: DEFINITION,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: STATE,
        destination_offset: 8,
        len: 8,
    },
    RecordField {
        source_offset: CONTROLLER,
        destination_offset: 16,
        len: 4,
    },
    RecordField {
        source_offset: CONTROLLER_AREA,
        destination_offset: 24,
        len: 8,
    },
    RecordField {
        source_offset: STRATEGIC_REGION,
        destination_offset: 32,
        len: 8,
    },
];
const MOVEMENT_PROVINCE_BYTES: usize = 40;
const MOVEMENT_PROVINCE_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&MOVEMENT_PROVINCE_FIELDS, MOVEMENT_PROVINCE_BYTES)
        .expect("valid record layout");
const TERRAIN_FLAG_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: crate::province_definition::TERRAIN_FLAGS,
    destination_offset: 0,
    len: 1,
}];
const TERRAIN_FLAG_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&TERRAIN_FLAG_FIELDS, 1).expect("valid record layout");

fn gather_rows(
    memory: &impl Memory,
    sources: &[usize],
    layout: &RecordLayout<'_>,
    out: &mut [u8],
    mut failed: impl FnMut(usize),
) {
    let (fields, stride) = (layout.fields().len(), layout.stride());
    let mut start = 0;
    while start < sources.len() {
        let copied = memory.copy_records(
            RecordSources::Pointers(&sources[start..]),
            layout,
            &mut out[start * stride..sources.len() * stride],
        );
        let complete = start + copied / fields;
        if complete == sources.len() {
            return;
        }
        failed(complete);
        start = complete + 1;
    }
}

type MovementResults = [Option<Result<MovementProvince, &'static str>>; BATCH_ROWS];

struct MovementRows {
    rows: [usize; BATCH_ROWS],
    sources: [usize; BATCH_ROWS],
    len: usize,
}

impl MovementRows {
    const EMPTY: Self = Self {
        rows: [0; BATCH_ROWS],
        sources: [0; BATCH_ROWS],
        len: 0,
    };

    fn push(&mut self, row: usize, source: usize) {
        (self.rows[self.len], self.sources[self.len]) = (row, source);
        self.len += 1;
    }

    fn gather(
        &mut self,
        memory: &impl Memory,
        layout: &RecordLayout<'_>,
        results: &mut MovementResults,
        alone: impl Fn(usize) -> Result<MovementProvince, &'static str>,
        mut next: impl FnMut(usize, usize, u64) -> Result<usize, &'static str>,
    ) {
        let stride = layout.stride();
        let mut bytes = [0u8; BATCH_ROWS * 8];
        let rows = &self.rows;
        gather_rows(
            memory,
            &self.sources[..self.len],
            layout,
            &mut bytes[..self.len * stride],
            |slot| results[rows[slot]] = Some(alone(rows[slot])),
        );
        let open = std::mem::take(&mut self.len);
        for slot in 0..open {
            let (row, source) = (self.rows[slot], self.sources[slot]);
            if results[row].is_some() {
                continue;
            }
            let mut word = [0u8; 8];
            word[..stride].copy_from_slice(&bytes[slot * stride..][..stride]);
            match next(row, source, u64::from_le_bytes(word)) {
                Ok(source) => self.push(row, source),
                Err(reason) => results[row] = Some(Err(reason)),
            }
        }
    }
}

pub fn movement_provinces(
    memory: &impl Memory,
    base: usize,
    provinces: Array,
    ids: &[u32],
    mut each: impl FnMut(u32, Result<MovementProvince, &'static str>),
) {
    const MISMATCH: &str = "movement province identity mismatch";
    let vtable = engine_sites::at(base, VTABLE_RVA).map(|pointer| pointer as u64);
    for ids in ids.chunks(BATCH_ROWS) {
        let mut results: MovementResults = [const { None }; BATCH_ROWS];
        let alone = |row: usize| movement_province(memory, base, provinces, ids[row]);
        let mut open = MovementRows::EMPTY;
        for (row, &id) in ids.iter().enumerate() {
            match provinces.element(id, 8) {
                Some(entry) => open.push(row, entry),
                None => results[row] = Some(Err("movement province outside the province array")),
            }
        }
        open.gather(memory, &POINTER_LAYOUT, &mut results, &alone, |_, _, pointer| {
            match usize::try_from(pointer) {
                Ok(0) => Err(MISMATCH),
                Ok(pointer) => Ok(pointer),
                Err(_) => Err("movement province entry unreadable"),
            }
        });
        open.gather(memory, &POINTER_LAYOUT, &mut results, &alone, |_, pointer, word| {
            (Some(word) == vtable).then_some(pointer).ok_or(MISMATCH)
        });
        open.gather(
            memory,
            &MOVEMENT_PROVINCE_ID_LAYOUT,
            &mut results,
            &alone,
            |row, pointer, id| (id == u64::from(ids[row])).then_some(pointer).ok_or(MISMATCH),
        );
        let mut payload = [0u8; BATCH_ROWS * MOVEMENT_PROVINCE_BYTES];
        let rows = &open.rows;
        gather_rows(
            memory,
            &open.sources[..open.len],
            &MOVEMENT_PROVINCE_LAYOUT,
            &mut payload[..open.len * MOVEMENT_PROVINCE_BYTES],
            |slot| results[rows[slot]] = Some(alone(rows[slot])),
        );
        let mut defined = MovementRows::EMPTY;
        for slot in 0..open.len {
            let row = open.rows[slot];
            if results[row].is_some() {
                continue;
            }
            let record = &payload[slot * MOVEMENT_PROVINCE_BYTES..][..MOVEMENT_PROVINCE_BYTES];
            let word = |at: usize| u64::from_le_bytes(record[at..at + 8].try_into().unwrap());
            results[row] = Some(Ok(MovementProvince {
                state: Ok(word(8) as usize),
                controller: Ok(u32::from_le_bytes(record[16..20].try_into().unwrap())),
                controller_area: Ok(word(24) as usize),
                strategic_region: Ok(word(32) as usize),
                terrain_flags: Err("movement province definition flags unreadable"),
            }));
            if word(0) != 0 {
                defined.push(row, word(0) as usize);
            }
        }
        let (mut flags, mut unreadable) = ([0u8; BATCH_ROWS], [false; BATCH_ROWS]);
        gather_rows(
            memory,
            &defined.sources[..defined.len],
            &TERRAIN_FLAG_LAYOUT,
            &mut flags[..defined.len],
            |slot| unreadable[slot] = true,
        );
        for slot in 0..defined.len {
            if !unreadable[slot]
                && let Some(Ok(province)) = &mut results[defined.rows[slot]]
            {
                province.terrain_flags = Ok(flags[slot]);
            }
        }
        for (row, result) in results.into_iter().enumerate().take(ids.len()) {
            each(ids[row], result.expect("every province has an outcome"));
        }
    }
}

fn geometry_field<T: crate::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|address| crate::read(memory, address))
        .ok_or("unit geometry field unreadable")
}

#[cfg(test)]
mod unit_order_tests;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod direct_combat_tests;

#[cfg(all(test, windows, target_env = "msvc", target_arch = "x86_64"))]
mod bench;
