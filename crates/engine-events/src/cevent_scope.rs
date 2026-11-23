
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x0285_2168);
pub const FROM: usize = 0x20;
pub const ROOT: usize = 0x18;
pub const PREV: usize = 0x28;
pub const COUNTRY_TAG: usize = 0x08;
pub const RANDOM_A: usize = 0x0c;
pub const RANDOM_B: usize = 0x10;
pub const STRATEGIC_REGION: usize = 0x48;
pub const COMBAT_CONTEXT: usize = 0x60;
pub const STATE_ID: usize = 0xa8;
pub const SIZE: usize = 0xb0;
pub const NO_COUNTRY: u32 = 0;
pub const SET_COUNTRY_RVA: Rva = Rva(0x0058_1970);
pub const EQUALS_RVA: Rva = Rva(0x0057_f3a0);

pub const REFERENCE_FIELDS: [usize; 9] = [0x50, 0x58, 0x68, 0x70, 0x78, 0x80, 0x88, 0x90, 0x98];
pub const TARGET_STORAGE: usize = 0xa0;
const ADDRESS_HASH_MULTIPLIER: usize = 0x9e37_79b9_7f4a_7c15;
const TARGET_RECORD_BYTES: usize = 0x70;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Operands {
    pub country: u32,
    pub random: [u32; 2],
    pub bindings: [u32; 3],
    pub state: i32,
    pub references: [u64; 9],
    pub strategic_region: Option<u32>,
    pub combat_context_unrecorded: bool,
    pub target_storage_unrecorded: bool,
    pub tooltip_storage_unrecorded: bool,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Node {
    address: usize,
    index_slot: usize,
    pub operands: Operands,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct IndexSlot {
    address: usize,
    id: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Graph {
    pub len: usize,
    pub required: usize,
    pub failure: Option<&'static str>,
}

pub fn copy_graph(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    root: usize,
    nodes: &mut [Node],
    index: &mut [IndexSlot],
) -> Graph {
    let mut len = 0;
    let mut required = 1;
    let mut count = 0;
    let result = (|| {
        if root == 0 {
            return Err("event scope root is null");
        }
        if nodes.is_empty() || index.len() <= nodes.len() {
            return Err("event scope graph needs prepared storage");
        }
        let mut intern = |address: usize, nodes: &mut [Node]| -> Result<u32, &'static str> {
            if address == 0 {
                return Err("event scope binding is null");
            }
            let mut slot = address.wrapping_mul(ADDRESS_HASH_MULTIPLIER) % index.len();
            loop {
                if index[slot].address == address {
                    return Ok(index[slot].id);
                }
                if index[slot].address == 0 {
                    required = count + 1;
                    let id = u32::try_from(count).map_err(|_| "event scope ID overflow")?;
                    let node = nodes
                        .get_mut(count)
                        .ok_or("event scope graph needs prepared storage")?;
                    *node = Node {
                        address,
                        index_slot: slot,
                        operands: Operands::default(),
                    };
                    index[slot] = IndexSlot { address, id };
                    count += 1;
                    return Ok(id);
                }
                slot = (slot + 1) % index.len();
            }
        };
        intern(root, nodes)?;
        let mut discovered = 1;
        let vtable = engine_sites::at(base, VTABLE_RVA)
            .ok_or("event scope vtable unreadable or differs")? as u64;
        while len < discovered {
            let address = nodes[len].address;
            let field = |offset: usize| {
                address
                    .checked_add(offset)
                    .ok_or("event scope field address overflow")
            };
            if engine_native_memory::read::<u64>(memory, address) != Some(vtable) {
                return Err("event scope vtable unreadable or differs");
            }
            let country =
                engine_native_memory::read(memory, field(COUNTRY_TAG)?).ok_or("event scope country unreadable")?;
            let random = [
                engine_native_memory::read(memory, field(RANDOM_A)?)
                    .ok_or("event scope random word unreadable")?,
                engine_native_memory::read(memory, field(RANDOM_B)?)
                    .ok_or("event scope random word unreadable")?,
            ];
            let mut bindings = [0; 3];
            for (binding, offset) in bindings.iter_mut().zip([ROOT, FROM, PREV]) {
                let next = engine_native_memory::read::<u64>(memory, field(offset)?)
                    .ok_or("event scope binding unreadable")?;
                *binding = intern(next as usize, nodes)?;
                discovered = discovered.max(*binding as usize + 1);
            }
            let mut references = [0; 9];
            for (reference, offset) in references.iter_mut().zip(REFERENCE_FIELDS) {
                *reference = engine_native_memory::read(memory, field(offset)?)
                    .ok_or("event scope object reference unreadable")?;
            }
            let region = engine_native_memory::read::<u64>(memory, field(STRATEGIC_REGION)?)
                .ok_or("event scope strategic region pointer unreadable")?;
            let region = region as usize;
            let strategic_region = if region == 0 {
                None
            } else {
                use engine_native_parts::cstrategic_region;
                let vtable = engine_sites::at(base, cstrategic_region::VTABLE_RVA)
                    .ok_or("event scope strategic region vtable unreadable or differs")?;
                if engine_native_memory::read::<u64>(memory, region) != Some(vtable as u64) {
                    return Err("event scope strategic region vtable unreadable or differs");
                }
                Some(
                    region
                        .checked_add(cstrategic_region::ID)
                        .and_then(|at| engine_native_memory::read(memory, at))
                        .ok_or("event scope strategic region identity unreadable")?,
                )
            };
            let targets = engine_native_memory::read::<u64>(memory, field(TARGET_STORAGE)?)
                .ok_or("event scope target storage pointer unreadable")?
                as usize;
            let (target_storage_unrecorded, tooltip_storage_unrecorded) = if targets == 0 {
                (false, false)
            } else {
                let saved = engine_native_memory::cpdx_array::Array::read(memory, targets, TARGET_RECORD_BYTES);
                let tooltip = targets
                    .checked_add(engine_native_memory::cpdx_array::STRIDE)
                    .and_then(|at| engine_native_memory::cpdx_array::Array::read(memory, at, TARGET_RECORD_BYTES));
                (
                    saved.is_none_or(|rows| rows.count != 0),
                    tooltip.is_none_or(|rows| rows.count != 0),
                )
            };
            nodes[len].operands = Operands {
                country,
                random,
                bindings,
                references,
                strategic_region,
                state: engine_native_memory::read(memory, field(STATE_ID)?)
                    .ok_or("event scope state unreadable")?,
                combat_context_unrecorded: engine_native_memory::read::<u64>(memory, field(COMBAT_CONTEXT)?)
                    .ok_or("event scope combat context pointer unreadable")?
                    != 0,
                target_storage_unrecorded,
                tooltip_storage_unrecorded,
            };
            len += 1;
        }
        Ok(())
    })();
    for node in &nodes[..count] {
        index[node.index_slot] = IndexSlot::default();
    }
    Graph {
        len,
        required,
        failure: result.err(),
    }
}

#[cfg(test)]
mod graph_tests;
