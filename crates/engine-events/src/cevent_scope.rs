//! CEventScope fields copied into pending events, windows and commands.
//!
//! Scope country, actor and containing-country recipient are distinct. Zero
//! scope country is valid. The
//! ability argument shares this layout; no separate effect-scope layout is established.

/// CEventScope primary vtable.
pub const VTABLE_RVA: usize = 0x027b_df18;
/// CEventScope FROM scope pointer.
pub const FROM: usize = 0x20;
/// CEventScope ROOT binding. An address is never a recorded scope ID.
pub const ROOT: usize = 0x18;
/// CEventScope PREV binding.
pub const PREV: usize = 0x28;
/// CEventScope country tag, u32.
pub const COUNTRY_TAG: usize = 0x08;
/// CEventScope first random word, u32.
pub const RANDOM_A: usize = 0x0c;
/// CEventScope second random word, u32.
pub const RANDOM_B: usize = 0x10;
/// CEventScope strategic-region pointer.
pub const STRATEGIC_REGION: usize = 0x48;
/// CEventScope character reference (u64 pair), copied from the unit leader character field; not a unit-leader handle.
pub const CHARACTER_REFERENCE: usize = 0x50;
/// CEventScope character-reference type word, u32.
pub const CHARACTER_REFERENCE_TYPE: usize = 0x50;
/// Character-reference id u32 word; nonzero gates registry resolution.
pub const CHARACTER_REFERENCE_ID: usize = 0x54;
/// CEventScope combat-context pointer.
pub const COMBAT_CONTEXT: usize = 0x60;
/// Reference pair for scope mask 0x20; registry target class remains unknown.
pub const SCOPE_MASK_20_REFERENCE: usize = 0x68;
/// First raw u32 of the scope-mask-0x20 reference; semantic word meaning remains unknown.
pub const SCOPE_MASK_20_REFERENCE_FIRST_WORD: usize = 0x68;
/// Second raw u32 of the scope-mask-0x20 reference; semantic word meaning remains unknown.
pub const SCOPE_MASK_20_REFERENCE_SECOND_WORD: usize = 0x6c;
/// Scope state id, signed i32; values at least one admit state scope.
pub const STATE_ID: usize = 0xa8;
/// CEventScope allocation size.
pub const SIZE: usize = 0xb0;
/// CEventScope country value meaning no country.
pub const NO_COUNTRY: u32 = 0;
/// Scope copy constructor.
pub const COPY_CONSTRUCT_RVA: usize = 0x0052_85f0;
/// Default scope constructor initializes the country word to zero.
pub const DEFAULT_CONSTRUCT_RVA: usize = 0x0052_8800;
/// Memberwise scope copy preserves the country word verbatim.
pub const COPY_MEMBERS_RVA: usize = 0x0052_96c0;
/// Scope country setter.
pub const SET_COUNTRY_RVA: usize = 0x0052_dd00;
/// Scope equality used in pending-event lookup.
pub const EQUALS_RVA: usize = 0x0052_bcc0;
/// Assign a unit leader's character reference to the scope.
pub const SET_UNIT_LEADER_RVA: usize = 0x0052_efd0;
/// Resolve a character from the scope reference registry.
pub const CHARACTER_RESOLVER_RVA: usize = 0x0052_9310;
/// Resolve the scope-mask-0x20 reference when nonzero; target class remains unknown.
pub const SCOPE_MASK_20_RESOLVER_RVA: usize = 0x0052_9e30;
/// Resolve the character, then its unit-leader role.
pub const ABILITY_CASTER_RESOLVER_RVA: usize = 0x0052_94f0;
/// Exact ability-caster resolver entry bytes.
pub const ABILITY_CASTER_RESOLVER_SENTINEL: [u8; 16] = [
    0x48, 0x83, 0xec, 0x28, 0x48, 0x83, 0xc1, 0x50, 0x83, 0x39, 0x00, 0x75, 0x06, 0x83, 0x79, 0x04,
];

/// Packed object references, in native field order: character, operation,
/// ace, unit, industrial organisation, purchase contract, raid, project,
/// faction. References preserve both words without resolving unrelated objects.
pub const REFERENCE_FIELDS: [usize; 9] = [0x50, 0x58, 0x68, 0x70, 0x78, 0x80, 0x88, 0x90, 0x98];
/// Optional scope-local saved and tooltip target storage.
pub const TARGET_STORAGE: usize = 0xa0;
const ADDRESS_HASH_MULTIPLIER: usize = 0x9e37_79b9_7f4a_7c15;
/// Each saved/tooltip target container holds these native records.
const TARGET_RECORD_BYTES: usize = 0x70;

/// Pointer-free scope operands. ROOT/FROM/PREV binding IDs index the acquisition's graph,
/// including self-links and shared nodes. Unknown pointer-owned operands are
/// explicit, separately counted demands, never silently represented as absent.
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

/// Acquisition-only work slot. Its address must not leave native acquisition.
#[derive(Debug, Default, Clone, Copy)]
pub struct Node {
    address: usize,
    index_slot: usize,
    pub operands: Operands,
}

/// Prepared address-to-ID index; the caller reuses this storage for every
/// occurrence. Empty slots carry no semantic country/scope ID.
#[derive(Debug, Default, Clone, Copy)]
pub struct IndexSlot {
    address: usize,
    id: u32,
}

/// A complete prefix is never a complete graph on shortage or read failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Graph {
    pub len: usize,
    pub required: usize,
    pub failure: Option<&'static str>,
}

/// Copy exactly the graph reachable from one occurrence, without allocation,
/// recursion, a selected depth cap, or touching any country's other events.
/// `index` needs more slots than `nodes`; a shortage requests prepared growth.
/// The graph IDs follow first encounter in native binding order.
pub fn copy_graph(
    memory: &impl crate::Memory,
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
        // Discovering a node initializes its work slot. The sentinel address
        // after the discovered prefix is unnecessary: all IDs originate here.
        let mut discovered = 1;
        while len < discovered {
            let address = nodes[len].address;
            let field = |offset: usize| {
                address
                    .checked_add(offset)
                    .ok_or("event scope field address overflow")
            };
            if crate::read::<u64>(memory, address)
                != base.checked_add(VTABLE_RVA).map(|value| value as u64)
            {
                return Err("event scope vtable unreadable or differs");
            }
            let country =
                crate::read(memory, field(COUNTRY_TAG)?).ok_or("event scope country unreadable")?;
            let random = [
                crate::read(memory, field(RANDOM_A)?)
                    .ok_or("event scope random word unreadable")?,
                crate::read(memory, field(RANDOM_B)?)
                    .ok_or("event scope random word unreadable")?,
            ];
            let mut bindings = [0; 3];
            for (binding, offset) in bindings.iter_mut().zip([ROOT, FROM, PREV]) {
                let next = crate::read::<u64>(memory, field(offset)?)
                    .ok_or("event scope binding unreadable")?;
                *binding = intern(next as usize, nodes)?;
                discovered = discovered.max(*binding as usize + 1);
            }
            let mut references = [0; 9];
            for (reference, offset) in references.iter_mut().zip(REFERENCE_FIELDS) {
                *reference = crate::read(memory, field(offset)?)
                    .ok_or("event scope object reference unreadable")?;
            }
            let region = crate::read::<u64>(memory, field(STRATEGIC_REGION)?)
                .ok_or("event scope strategic region pointer unreadable")?;
            let region = region as usize;
            let strategic_region = if region == 0 {
                None
            } else {
                use engine_native::cstrategic_region;
                if crate::read::<u64>(memory, region)
                    != base
                        .checked_add(cstrategic_region::VTABLE_RVA)
                        .map(|value| value as u64)
                {
                    return Err("event scope strategic region vtable unreadable or differs");
                }
                Some(
                    region
                        .checked_add(cstrategic_region::ID)
                        .and_then(|at| crate::read(memory, at))
                        .ok_or("event scope strategic region identity unreadable")?,
                )
            };
            let targets = crate::read::<u64>(memory, field(TARGET_STORAGE)?)
                .ok_or("event scope target storage pointer unreadable")?
                as usize;
            let (target_storage_unrecorded, tooltip_storage_unrecorded) = if targets == 0 {
                (false, false)
            } else {
                let saved = crate::cpdx_array::Array::read(memory, targets, TARGET_RECORD_BYTES);
                let tooltip = targets
                    .checked_add(crate::cpdx_array::STRIDE)
                    .and_then(|at| crate::cpdx_array::Array::read(memory, at, TARGET_RECORD_BYTES));
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
                state: crate::read(memory, field(STATE_ID)?)
                    .ok_or("event scope state unreadable")?,
                combat_context_unrecorded: crate::read::<u64>(memory, field(COMBAT_CONTEXT)?)
                    .ok_or("event scope combat context pointer unreadable")?
                    != 0,
                target_storage_unrecorded,
                tooltip_storage_unrecorded,
            };
            len += 1;
        }
        Ok(())
    })();
    // Reset only slots this occurrence used, including the discovered suffix
    // after failure. Preparing a large graph never makes later small events
    // clear the whole retained capacity.
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
