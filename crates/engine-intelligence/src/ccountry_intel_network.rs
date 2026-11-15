//! Network state rows and independently retained national caches. The daily
//! network update creates/rebuilds networks; dynamic country reuse destroys
//! them, while console growth changes state strengths without rebuilding the
//! national caches. Loading establishes independent saved cache values.

use engine_sites::Rva;
use crate::{
    Memory, ccountry, cgame_state, clocal_intel_network, cpdx_array::Array, cstrategic_operative,
    cstrategic_operative_manager, csub_intel_network, intel_network_graph, intel_network_graph_row,
    intel_network_graph_vertex, memory::Raw, read, read_slice, walk::Outcome,
};

/// Intel-network vtable.
pub const VTABLE_RVA: Rva = Rva(0x2a1_dcd8);
/// Target country id, read as i32.
pub const TARGET: usize = 0x08;
/// Constructor initializing the vtable and embedded objects.
pub const CONSTRUCTOR_RVA: Rva = Rva(0x125_ccf0);
/// Embedded local network.
pub const LOCAL_NETWORK: usize = 0x10;
/// Subnetwork array data.
pub const SUBNETWORKS_DATA: usize = 0x78;
/// Subnetwork count, i32.
#[cfg(test)]
pub const SUBNETWORKS_COUNT: usize = 0x84;
/// Embedded static-intel-source reference.
pub const STATIC_SOURCE_REFERENCE: usize = 0xa8;
/// Covered core-state count, signed i32 as serialized.
pub const COVERED_CORE_STATES: usize = 0xc0;
/// Covered controlled-state count, signed i32 as serialized.
#[cfg(test)]
pub const COVERED_CONTROLLED_STATES: usize = 0xc4;
/// Covered state worth, signed i32 as serialized.
#[cfg(test)]
pub const COVERED_WORTH: usize = 0xc8;
/// Controlled-state strength sum, signed i64.
pub const CONTROLLED_STRENGTH_SUM: usize = 0xd0;
/// Core-state strength sum, signed i64.
#[cfg(test)]
pub const CORE_STRENGTH_SUM: usize = 0xd8;
/// Cached maximum state strength, signed i64, saved independently of the sums.
pub const MAX_STATE_STRENGTH: usize = 0xe0;
/// Network serializer.
#[cfg(test)]
pub const SAVE_RVA: usize = 0x0126_4170;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Already-acquired graph position, shared with auxiliary capture only.
    pub native_ordinal: u32,
    /// Raw country references, including authenticated aliases.
    pub owner_country: u32,
    pub target_country: u32,
    pub state_id: i32,
    /// Subnetwork strength qword; negative selectors have no strength value.
    pub strength_raw: Option<i64>,
    /// Signed graph-row selector retained exactly, including unassigned values.
    pub subnetwork_selector: i32,
    /// Graph-row state strength, which raid target intel and the map mode's
    /// network layer read; independent of selector.
    pub local_strength_raw: i64,
    /// Graph-row daily gain and its parts from operatives and from adjacent states.
    pub gain_raw: i64,
    pub gain_from_operatives_raw: i64,
    pub gain_from_adjacency_raw: i64,
    /// The local network's quiet byte for the selector, which the map mode's
    /// network layer reads; zero for a selector outside its list.
    pub local_quiet: u8,
    /// The selected subnetwork's gain factor, strength target and the
    /// target's parts from operatives and from counterintelligence, which the
    /// state tooltip reads; none for a negative selector.
    pub gain_factor_raw: Option<i64>,
    pub strength_target_raw: Option<i64>,
    pub target_from_operatives_raw: Option<i64>,
    pub target_from_counterintelligence_raw: Option<i64>,
    /// The selected subnetwork's own quiet byte, which the state tooltip
    /// reads; zero for a negative selector, which selects no subnetwork.
    pub subnetwork_quiet: u8,
}

/// The selected subnetwork's copied values.
#[derive(Clone, Copy)]
struct Subnetwork {
    strength_raw: i64,
    gain_factor_raw: i64,
    strength_target_raw: i64,
    target_from_operatives_raw: i64,
    target_from_counterintelligence_raw: i64,
    quiet: u8,
}

/// Independently retained national caches, acquired once per actual network
/// during the existing network census, including networks with no vertices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    /// Worker cache identity only; never recorded.
    pub native_pointer: usize,
    pub owner_country: u32,
    pub target_country: u32,
    /// Current position in the owner's native network vector, acquired for free.
    pub ordinal: u32,
    pub covered_core_states: i32,
    pub covered_controlled_states: i32,
    pub covered_worth: i32,
    pub controlled_strength_sum: i64,
    pub core_strength_sum: i64,
    pub max_state_strength: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetworkWalk {
    pub states: Outcome,
    pub summaries: Outcome,
}

/// Shared worker storage. The daily rebuild is the only writer of the retained
/// aggregate values after construction/load. Outside it, a country reset can
/// destroy networks and console growth changes state strengths alone. Reuse
/// requires the caller's reset/growth certificate and the census's authenticated
/// network identity; attachment/load/gap/forced refresh revokes it. The census
/// selects current membership even when an aggregate is carried.
#[derive(Default)]
pub struct SummaryCache {
    rows: Vec<Summary>,
}

impl SummaryCache {
    pub const fn new() -> Self {
        Self { rows: Vec::new() }
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.capacity() * std::mem::size_of::<Summary>()
    }
    pub fn invalidate(&mut self) {
        self.rows.clear();
    }
}

/// Worker acquisition shares the established roster and network authentication.
/// Summary read failures withhold only summaries, never the existing state rows.
#[cfg(test)]
pub fn walk_with_summaries(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    states: &mut Vec<Row>,
    summaries: &mut Vec<Summary>,
) -> NetworkWalk {
    walk_with_cached_summaries(
        memory,
        base,
        game_state,
        states,
        summaries,
        &mut SummaryCache::new(),
        true,
    )
}

pub fn walk_with_cached_summaries(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    states: &mut Vec<Row>,
    summary_rows: &mut Vec<Summary>,
    cache: &mut SummaryCache,
    refresh: bool,
) -> NetworkWalk {
    let mut summary = SummaryCapture {
        output: crate::walk::Buffer::Growing(summary_rows),
        len: 0,
        failure: None,
        held: if refresh { &[] } else { &cache.rows },
        fresh: false,
    };
    let states = walk_inner(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(states),
        Some(&mut summary),
    );
    let summaries = if let Some(reason) = summary.failure {
        Outcome::Unavailable { reason }
    } else if matches!(states, Outcome::Rows { .. }) {
        Outcome::Rows { len: summary.len }
    } else {
        Outcome::Unavailable {
            reason: "intel network summary census incomplete",
        }
    };
    let changed_cache = refresh || summary.fresh;
    if let Outcome::Rows { len } = summaries {
        if !changed_cache {
            return NetworkWalk { states, summaries };
        }
        let captured = &summary_rows[..len];
        if engine_native::allocation::resize_default(&mut cache.rows, len).is_err() {
            cache.rows.clear();
            return NetworkWalk {
                states,
                summaries: Outcome::Unavailable {
                    reason: "intel network summary cache allocation failed",
                },
            };
        }
        cache.rows.copy_from_slice(captured);
        cache.rows.sort_unstable_by_key(|row| row.native_pointer);
    } else if refresh {
        // A failed refresh revokes the previous day's values. An intervening
        // membership failure publishes no new cache and cannot revoke payload
        // certificates the caller still proves; retain those for its retry.
        cache.rows.clear();
    }
    NetworkWalk { states, summaries }
}

struct SummaryCapture<'a> {
    output: crate::walk::Buffer<'a, Summary>,
    len: usize,
    failure: Option<&'static str>,
    held: &'a [Summary],
    fresh: bool,
}

impl SummaryCapture<'_> {
    fn visit(
        &mut self,
        memory: &impl Memory,
        network: usize,
        owner: u32,
        target: u32,
        ordinal: u32,
    ) {
        if self.failure.is_some() {
            return;
        }
        let result = (|| {
            if !self.output.ensure(
                self.len
                    .checked_add(1)
                    .ok_or("intel network summary demand overflow")?,
            )? {
                return Err("intel network summary storage unavailable");
            }
            let held = self
                .held
                .binary_search_by_key(&network, |row| row.native_pointer)
                .ok()
                .map(|index| self.held[index])
                .filter(|row| row.owner_country == owner && row.target_country == target);
            self.output[self.len] = if let Some(held) = held {
                Summary { ordinal, ..held }
            } else {
                self.fresh = true;
                // Read only serializer operands, not the coverage padding.
                let coverage = field::<[u8; 12]>(memory, network, COVERED_CORE_STATES)?;
                let strengths = field::<[u8; 24]>(memory, network, CONTROLLED_STRENGTH_SUM)?;
                let count = |index: usize| {
                    i32::from_le_bytes(coverage[index * 4..index * 4 + 4].try_into().unwrap())
                };
                let strength = |index: usize| {
                    i64::from_le_bytes(strengths[index * 8..index * 8 + 8].try_into().unwrap())
                };
                Summary {
                    native_pointer: network,
                    owner_country: owner,
                    target_country: target,
                    ordinal,
                    covered_core_states: count(0),
                    covered_controlled_states: count(1),
                    covered_worth: count(2),
                    controlled_strength_sum: strength(0),
                    core_strength_sum: strength(1),
                    max_state_strength: strength(2),
                }
            };
            self.len += 1;
            Ok(())
        })();
        self.failure = result.err();
    }
}

pub fn count(memory: &impl Memory, base: usize, game_state: usize) -> Outcome {
    walk_buffer(memory, base, game_state, crate::walk::Buffer::Count)
}

/// Reads the country-rooted network roster and its state-to-subnetwork mapping.
/// A network has no single stored national strength; each row retains the native
/// strength of the subnetwork selected by that state's graph vertex.
pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
) -> Outcome {
    walk_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(output),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: crate::walk::Buffer<'_, Row>,
) -> Outcome {
    walk_inner(memory, base, game_state, output, None)
}

fn walk_inner(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: crate::walk::Buffer<'_, Row>,
    mut summaries: Option<&mut SummaryCapture<'_>>,
) -> Outcome {
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let manager = pointer(memory, game_state, cgame_state::STRATEGIC_OPERATIVE_MANAGER)?;
        // The constructor leaves this root null when La Resistance is disabled.
        if manager == 0 {
            return Ok(Outcome::Rows { len: 0 });
        }
        let strategic = array(
            memory,
            manager,
            cstrategic_operative_manager::OPERATIVES_DATA,
            8,
        )?;
        let mut len = 0_usize;
        let mut unread_rows = 0_u32;
        let mut seen = crate::slot_set::SlotSet::new();
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            let operative = strategic
                .element(slot, 8)
                .and_then(|address| read::<u64>(memory, address))
                .and_then(|pointer| usize::try_from(pointer).ok())
                .ok_or("intel strategic operative slot unreadable or outside the vector")?;
            // A null slot has no operative and no networks; the game null-checks its lookups.
            if operative == 0 {
                continue;
            }
            vtable(memory, base, operative, cstrategic_operative::VTABLE_RVA)?;
            let owner = field::<u32>(memory, operative, cstrategic_operative::OWNER)?;
            if countries.resolve(owner)?.pointer != country.pointer {
                return Err("intel strategic owner does not match country slot");
            }
            let networks = array(memory, operative, cstrategic_operative::NETWORKS_DATA, 8)?;
            seen.clear();
            for network_slot in 0..networks.count {
                let network = element_pointer(memory, networks, network_slot)?;
                vtable(memory, base, network, VTABLE_RVA)?;
                let target = field::<u32>(memory, network, TARGET)?;
                let target_country = countries.resolve(target)?;
                if target_country.pointer == country.pointer {
                    return Err("intel network target equals its owner");
                }
                // Networks are not sorted: the native removal path swaps the
                // last pointer into the removed slot. Reject duplicate targets
                // by canonical slot in O(1); the exact fallback rereads.
                if seen.insert(target_country.canonical_slot, || {
                    for previous in 0..network_slot {
                        let earlier = element_pointer(memory, networks, previous)?;
                        let earlier_target = field::<u32>(memory, earlier, TARGET)?;
                        if countries.resolve(earlier_target)?.pointer == target_country.pointer {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                })? {
                    return Err("duplicate intel network target for owner");
                }
                if let Some(summaries) = summaries.as_deref_mut() {
                    summaries.visit(memory, network, owner, target, network_slot);
                }
                let local = network
                    .checked_add(LOCAL_NETWORK)
                    .ok_or("intel address overflow")?;
                vtable(memory, base, local, clocal_intel_network::VTABLE_RVA)?;
                let graph = pointer(memory, local, clocal_intel_network::GRAPH)?;
                if graph == 0 {
                    return Err("intel graph is null");
                }
                let begin = pointer(memory, graph, intel_network_graph::ROWS_BEGIN)?;
                let end = pointer(memory, graph, intel_network_graph::ROWS_END)?;
                let row_bytes = end.checked_sub(begin).ok_or("intel graph range reversed")?;
                if row_bytes % intel_network_graph_row::STRIDE != 0
                    || (row_bytes != 0 && begin == 0)
                {
                    return Err("intel graph row range invalid");
                }
                let row_count = row_bytes / intel_network_graph_row::STRIDE;
                let vertices = array(
                    memory,
                    graph,
                    intel_network_graph::VERTICES_DATA,
                    intel_network_graph_vertex::STRIDE,
                )?;
                let subnetworks = array(
                    memory,
                    network,
                    SUBNETWORKS_DATA,
                    csub_intel_network::STRIDE,
                )?;
                let quiet = array(memory, local, clocal_intel_network::QUIET_DATA, 1)?;
                output.ensure(
                    len.checked_add(vertices.count as usize)
                        .ok_or("intel network row demand overflow")?,
                )?;
                let mut previous_state = None;
                let mut vertex_index = 0;
                while vertex_index < vertices.count {
                    if len == output.len() {
                        unread_rows = unread_rows
                            .checked_add(vertices.count - vertex_index)
                            .ok_or("intel unread row count overflow")?;
                        break;
                    }
                    const BATCH: usize = 64;
                    let count = BATCH
                        .min((vertices.count - vertex_index) as usize)
                        .min(output.len() - len);
                    let vertex = vertices
                        .element(vertex_index, intel_network_graph_vertex::STRIDE)
                        .ok_or("intel vertex address overflow")?;
                    let mut vertex_bytes = [0_u8; BATCH * intel_network_graph_vertex::STRIDE];
                    if !read_slice(
                        memory,
                        vertex,
                        &mut vertex_bytes[..count * intel_network_graph_vertex::STRIDE],
                    ) {
                        return Err("intel vertex unreadable");
                    }
                    let mut selector_addresses = [0; BATCH];
                    for (index, bytes) in vertex_bytes[..count * intel_network_graph_vertex::STRIDE]
                        .chunks_exact(intel_network_graph_vertex::STRIDE)
                        .enumerate()
                    {
                        let state_id = i32::from_le_bytes(bytes[..4].try_into().unwrap());
                        let row_index = u64::from_le_bytes(bytes[8..].try_into().unwrap());
                        if state_id <= 0
                            || previous_state.is_some_and(|previous| previous >= state_id)
                        {
                            return Err("intel state ids are invalid or not strictly ordered");
                        }
                        previous_state = Some(state_id);
                        let row_index =
                            usize::try_from(row_index).map_err(|_| "intel row index overflow")?;
                        if row_index >= row_count {
                            return Err("intel graph row index outside rows");
                        }
                        selector_addresses[index] = row_index
                            .checked_mul(intel_network_graph_row::STRIDE)
                            .and_then(|offset| begin.checked_add(offset))
                            .and_then(|address| {
                                address.checked_add(intel_network_graph_row::SUBNETWORK_SELECTOR)
                            })
                            .ok_or("intel graph row address overflow")?;
                    }
                    // One copy per row spans the selector through the state id.
                    const ROW_END: usize = intel_network_graph_row::STATE_ID + 4
                        - intel_network_graph_row::SUBNETWORK_SELECTOR;
                    let row_field = |bytes: &[u8], offset: usize| {
                        let at = offset - intel_network_graph_row::SUBNETWORK_SELECTOR;
                        i64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
                    };
                    let mut selectors = [[0_u8; ROW_END]; BATCH];
                    let mut fields = selectors.iter_mut().enumerate();
                    let mut destinations = std::array::from_fn(|_| {
                        let (index, bytes) = fields.next().unwrap();
                        &mut bytes[..if index < count { ROW_END } else { 0 }]
                    });
                    if memory.copy_scattered(selector_addresses, &mut destinations) != BATCH {
                        return Err("intel graph selector unreadable");
                    }
                    let mut strength_addresses = [0; BATCH];
                    let mut quiet_addresses = [0; BATCH];
                    for (index, bytes) in selectors[..count].iter().enumerate() {
                        const STATE_AT: usize = intel_network_graph_row::STATE_ID
                            - intel_network_graph_row::SUBNETWORK_SELECTOR;
                        let row_state =
                            i32::from_le_bytes(bytes[STATE_AT..STATE_AT + 4].try_into().unwrap());
                        let at = index * intel_network_graph_vertex::STRIDE;
                        let vertex_state =
                            i32::from_le_bytes(vertex_bytes[at..at + 4].try_into().unwrap());
                        if row_state != vertex_state {
                            return Err("intel graph row state differs from its vertex");
                        }
                        let selector = i32::from_le_bytes(bytes[..4].try_into().unwrap());
                        // Only nonnegative selectors reach a subnetwork. Negative vertices remain
                        // present without its values.
                        if selector >= 0 {
                            strength_addresses[index] = subnetworks
                                .element(selector as u32, csub_intel_network::STRIDE)
                                .ok_or("intel subnetwork selector outside array")?;
                            // A selector outside the quiet list is not quiet.
                            if (selector as u32) < quiet.count {
                                quiet_addresses[index] = quiet
                                    .element(selector as u32, 1)
                                    .ok_or("intel quiet list address overflow")?;
                            }
                        }
                    }
                    // Copy each selected subnetwork from its vtable through its quiet flag.
                    const SUBNETWORK_END: usize = csub_intel_network::QUIET + 1;
                    let subnetwork_field = |bytes: &[u8], offset: usize| {
                        i64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
                    };
                    let mut strengths = [[0_u8; SUBNETWORK_END]; BATCH];
                    let mut fields = strengths.iter_mut().enumerate();
                    let mut destinations = std::array::from_fn(|_| {
                        let (index, bytes) = fields.next().unwrap();
                        &mut bytes[..if strength_addresses[index] != 0 {
                            SUBNETWORK_END
                        } else {
                            0
                        }]
                    });
                    if memory.copy_scattered(strength_addresses, &mut destinations) != BATCH {
                        return Err("intel subnetwork strength unreadable");
                    }
                    let mut quiet_bytes = [[0_u8; 1]; BATCH];
                    let mut fields = quiet_bytes.iter_mut().enumerate();
                    let mut destinations = std::array::from_fn(|_| {
                        let (index, bytes) = fields.next().unwrap();
                        &mut bytes[..usize::from(quiet_addresses[index] != 0)]
                    });
                    if memory.copy_scattered(quiet_addresses, &mut destinations) != BATCH {
                        return Err("intel quiet list unreadable");
                    }
                    let expected_vtable =
                        engine_sites::at(base, csub_intel_network::VTABLE_RVA)
                            .ok_or("intel module address overflow")? as u64;
                    for index in 0..count {
                        let row = &selectors[index];
                        let selector = i32::from_le_bytes(row[..4].try_into().unwrap());
                        let subnetwork = if selector < 0 {
                            None
                        } else {
                            let bytes = &strengths[index];
                            if u64::from_le_bytes(bytes[..8].try_into().unwrap()) != expected_vtable
                            {
                                return Err("intel network object vtable mismatch");
                            }
                            Some(Subnetwork {
                                strength_raw: subnetwork_field(bytes, csub_intel_network::STRENGTH),
                                gain_factor_raw: subnetwork_field(
                                    bytes,
                                    csub_intel_network::GAIN_FACTOR,
                                ),
                                strength_target_raw: subnetwork_field(
                                    bytes,
                                    csub_intel_network::STRENGTH_TARGET,
                                ),
                                target_from_operatives_raw: subnetwork_field(
                                    bytes,
                                    csub_intel_network::TARGET_FROM_OPERATIVES,
                                ),
                                target_from_counterintelligence_raw: subnetwork_field(
                                    bytes,
                                    csub_intel_network::TARGET_FROM_COUNTERINTELLIGENCE,
                                ),
                                quiet: bytes[csub_intel_network::QUIET],
                            })
                        };
                        let at = index * intel_network_graph_vertex::STRIDE;
                        output[len] = Row {
                            native_ordinal: u64::from_le_bytes(
                                vertex_bytes[at + 8..at + 16].try_into().unwrap(),
                            ) as u32,
                            owner_country: owner,
                            target_country: target,
                            state_id: i32::from_le_bytes(
                                vertex_bytes[at..at + 4].try_into().unwrap(),
                            ),
                            strength_raw: subnetwork.map(|sub| sub.strength_raw),
                            subnetwork_selector: selector,
                            local_strength_raw: row_field(
                                row,
                                intel_network_graph_row::STATE_STRENGTH,
                            ),
                            gain_raw: row_field(row, intel_network_graph_row::GAIN),
                            gain_from_operatives_raw: row_field(
                                row,
                                intel_network_graph_row::GAIN_FROM_OPERATIVES,
                            ),
                            gain_from_adjacency_raw: row_field(
                                row,
                                intel_network_graph_row::GAIN_FROM_ADJACENCY,
                            ),
                            local_quiet: if quiet_addresses[index] != 0 {
                                quiet_bytes[index][0]
                            } else {
                                0
                            },
                            gain_factor_raw: subnetwork.map(|sub| sub.gain_factor_raw),
                            strength_target_raw: subnetwork.map(|sub| sub.strength_target_raw),
                            target_from_operatives_raw: subnetwork
                                .map(|sub| sub.target_from_operatives_raw),
                            target_from_counterintelligence_raw: subnetwork
                                .map(|sub| sub.target_from_counterintelligence_raw),
                            subnetwork_quiet: subnetwork.map_or(0, |sub| sub.quiet),
                        };
                        len += 1;
                    }
                    vertex_index += count as u32;
                }
            }
        }
        Ok(if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

fn field<T: Raw>(memory: &impl Memory, object: usize, offset: usize) -> Result<T, &'static str> {
    object
        .checked_add(offset)
        .and_then(|address| read(memory, address))
        .ok_or("intel network field unreadable or address overflow")
}

fn pointer(memory: &impl Memory, object: usize, offset: usize) -> Result<usize, &'static str> {
    usize::try_from(field::<u64>(memory, object, offset)?).map_err(|_| "intel pointer overflow")
}

fn array(
    memory: &impl Memory,
    object: usize,
    offset: usize,
    stride: usize,
) -> Result<Array, &'static str> {
    object
        .checked_add(offset)
        .and_then(|address| Array::read(memory, address, stride))
        .ok_or("intel array unreadable or invalid")
}

fn element_pointer(memory: &impl Memory, array: Array, index: u32) -> Result<usize, &'static str> {
    array
        .element(index, 8)
        .and_then(|address| read::<u64>(memory, address))
        .and_then(|pointer| usize::try_from(pointer).ok())
        .filter(|&pointer| pointer != 0)
        .ok_or("intel roster pointer unreadable, null or outside array")
}

fn vtable(
    memory: &impl Memory,
    base: usize,
    object: usize,
    key: Rva,
) -> Result<(), &'static str> {
    let expected = engine_sites::at(base, key).ok_or("intel module address overflow")?;
    if read::<u64>(memory, object) == Some(expected as u64) {
        Ok(())
    } else {
        Err("intel network object vtable mismatch")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;

    const GS: usize = 0x1000;
    const MANAGER: usize = 0x9000;
    const STRATEGIC: usize = 0xa000;
    const NETWORK: usize = 0xb000;
    const GRAPH: usize = 0xc000;
    const VERTICES: usize = 0xd000;
    const GRAPH_ROWS: usize = 0xe000;
    const SUBNETWORKS: usize = 0xf000;
    const QUIET: usize = 0x10800;

    #[test]
    fn network_summaries_keep_independent_caches_and_quiet_carries_read_no_payload() {
        let mut bytes = fixture();
        for (offset, value) in [
            (COVERED_CORE_STATES, 17),
            (COVERED_CONTROLLED_STATES, 29),
            (COVERED_WORTH, u32::MAX),
        ] {
            p32(&mut bytes, NETWORK + offset, value);
        }
        for (offset, value) in [
            (CONTROLLED_STRENGTH_SUM, i64::MIN),
            (CORE_STRENGTH_SUM, -7),
            (MAX_STATE_STRENGTH, i64::MAX),
        ] {
            p64(&mut bytes, NETWORK + offset, value as u64);
        }
        let mut states = Vec::new();
        let mut summaries = Vec::new();
        let mut cache = SummaryCache::new();
        let read = walk_with_cached_summaries(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            GS,
            &mut states,
            &mut summaries,
            &mut cache,
            true,
        );
        assert_eq!(read.summaries, Outcome::Rows { len: 1 });
        let expected = summaries[0];
        assert_eq!(
            (
                expected.covered_core_states,
                expected.covered_controlled_states,
                expected.covered_worth
            ),
            (17, 29, -1)
        );
        assert_eq!(
            (
                expected.controlled_strength_sum,
                expected.core_strength_sum,
                expected.max_state_strength
            ),
            (i64::MIN, -7, i64::MAX)
        );

        struct DenyPayload<'a>(&'a [u8]);
        impl Memory for DenyPayload<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address < NETWORK + MAX_STATE_STRENGTH + 8
                    && address.saturating_add(output.len()) > NETWORK + COVERED_CORE_STATES
                {
                    return false;
                }
                SliceMemory {
                    base: 0,
                    bytes: self.0,
                }
                .copy(address, output)
            }
        }
        // The already-authenticated object's values carry without touching its
        // aggregate payload. A failed forced read cannot turn into a carry.
        let denied = DenyPayload(&bytes);
        let read = walk_with_cached_summaries(
            &denied,
            0,
            GS,
            &mut states,
            &mut summaries,
            &mut cache,
            false,
        );
        assert_eq!(read.summaries, Outcome::Rows { len: 1 });
        assert_eq!(summaries[0], expected);
        let read = walk_with_cached_summaries(
            &denied,
            0,
            GS,
            &mut states,
            &mut summaries,
            &mut cache,
            true,
        );
        assert!(matches!(read.summaries, Outcome::Unavailable { .. }));
        assert_eq!(read.states, Outcome::Rows { len: 2 });
        assert!(cache.rows.is_empty());
    }

    #[test]
    fn empty_network_has_a_summary_and_zero_vertices_are_not_absent_membership() {
        let mut bytes = fixture();
        header(&mut bytes, GRAPH + intel_network_graph::VERTICES_DATA, 0, 0);
        p64(&mut bytes, GRAPH + intel_network_graph::ROWS_BEGIN, 0);
        p64(&mut bytes, GRAPH + intel_network_graph::ROWS_END, 0);
        header(&mut bytes, NETWORK + SUBNETWORKS_DATA, 0, 0);
        let mut states = Vec::new();
        let mut summaries = Vec::new();
        let read = walk_with_summaries(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            GS,
            &mut states,
            &mut summaries,
        );
        assert_eq!(read.states, Outcome::Rows { len: 0 });
        assert_eq!(read.summaries, Outcome::Rows { len: 1 });
        assert_eq!(
            (summaries[0].owner_country, summaries[0].target_country),
            (3, 2)
        );
    }

    fn p32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn p64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn header(bytes: &mut [u8], at: usize, data: usize, count: u32) {
        p64(bytes, at, data as u64);
        p32(bytes, at + 8, count);
        p32(bytes, at + 12, count);
    }
    #[test]
    fn growing_intel_network_rows_follow_new_native_vertices_without_new_countries() {
        let mut bytes = fixture();
        let mut rows = Vec::new();
        let mut expected = vec![Row::default(); 2];
        assert_eq!(capture(&bytes, &mut expected), Outcome::Rows { len: 2 });
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                GS,
                &mut rows
            ),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(&rows[..2], &expected);
        const COUNT: usize = 5_000;
        const VERTEX_DATA: usize = 0x20000;
        const ROW_DATA: usize = 0x40000;
        bytes.resize(ROW_DATA + COUNT * intel_network_graph_row::STRIDE, 0);
        p64(
            &mut bytes,
            GRAPH + intel_network_graph::ROWS_BEGIN,
            ROW_DATA as u64,
        );
        p64(
            &mut bytes,
            GRAPH + intel_network_graph::ROWS_END,
            (ROW_DATA + COUNT * intel_network_graph_row::STRIDE) as u64,
        );
        header(
            &mut bytes,
            GRAPH + intel_network_graph::VERTICES_DATA,
            VERTEX_DATA,
            COUNT as u32,
        );
        for index in 0..COUNT {
            let vertex = VERTEX_DATA + index * intel_network_graph_vertex::STRIDE;
            p32(&mut bytes, vertex, index as u32 + 1);
            p64(&mut bytes, vertex + 8, index as u64);
            let row = ROW_DATA + index * intel_network_graph_row::STRIDE;
            p32(
                &mut bytes,
                row + intel_network_graph_row::SUBNETWORK_SELECTOR,
                u32::MAX,
            );
            p64(
                &mut bytes,
                row + intel_network_graph_row::STATE_STRENGTH,
                index as u64,
            );
            p32(
                &mut bytes,
                row + intel_network_graph_row::STATE_ID,
                index as u32 + 1,
            );
        }
        let mut expected = vec![Row::default(); COUNT];
        let outcome = capture(&bytes, &mut expected);
        assert_eq!(outcome, Outcome::Rows { len: COUNT });
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                GS,
                &mut rows
            ),
            outcome
        );
        assert_eq!(&rows[..COUNT], expected.as_slice());
    }

    #[test]
    fn one_owner_with_networks_in_every_other_country_is_checked_for_duplicates() {
        use crate::ccountry::synthetic::World;
        const COUNT: u32 = 4;
        let mut world = World::new(0, COUNT, 0x10);
        let manager = world.alloc(0x100);
        let operatives = world.alloc((COUNT as usize + 1) * 8);
        let operative_objects = world.alloc((COUNT as usize + 1) * 0x20);
        let graph = world.alloc(0x100);
        let pointers = world.alloc((COUNT as usize - 1) * 8);
        let networks = world.alloc((COUNT as usize - 1) * 0x100);
        let gs = world.game_state();
        world.u64(
            gs + cgame_state::STRATEGIC_OPERATIVE_MANAGER,
            manager as u64,
        );
        world.array(
            manager + cstrategic_operative_manager::OPERATIVES_DATA,
            operatives,
            COUNT + 1,
        );
        for slot in 1..=COUNT as usize {
            let operative = operative_objects + slot * 0x20;
            world.u64(operatives + slot * 8, operative as u64);
            world.u64(operative, cstrategic_operative::VTABLE_RVA.0 as u64);
            world.u32(operative + cstrategic_operative::OWNER, slot as u32);
        }
        // Country 1 runs a network against every other country; the shared
        // graph has no rows or vertices, so each network yields no rows.
        let owner = operative_objects + 0x20;
        world.array(
            owner + cstrategic_operative::NETWORKS_DATA,
            pointers,
            COUNT - 1,
        );
        for index in 0..COUNT as usize - 1 {
            let network = networks + index * 0x100;
            world.u64(pointers + index * 8, network as u64);
            world.u64(network, VTABLE_RVA.0 as u64);
            world.u32(network + TARGET, index as u32 + 2);
            world.u64(
                network + LOCAL_NETWORK,
                clocal_intel_network::VTABLE_RVA.0 as u64,
            );
            world.u64(
                network + LOCAL_NETWORK + clocal_intel_network::GRAPH,
                graph as u64,
            );
        }
        let run = |world: &World| walk(&world.memory(), 0, gs, &mut Vec::new());
        assert_eq!(run(&world), Outcome::Rows { len: 0 });
        let mut states = Vec::new();
        let mut summaries = Vec::new();
        let captured =
            walk_with_summaries(&world.memory(), 0, gs, &mut states, &mut summaries);
        assert_eq!(captured.states, Outcome::Rows { len: 0 });
        assert_eq!(
            captured.summaries,
            Outcome::Rows {
                len: COUNT as usize - 1
            }
        );
        assert_eq!(summaries[COUNT as usize - 2].target_country, COUNT);
        assert_eq!(summaries[COUNT as usize - 2].ordinal, COUNT - 2);
        // The last network repeats the first target.
        world.u32(networks + (COUNT as usize - 2) * 0x100 + TARGET, 2);
        assert_eq!(
            run(&world),
            Outcome::Unavailable {
                reason: "duplicate intel network target for owner"
            }
        );
    }

    #[test]
    fn a_null_strategic_slot_runs_no_networks_but_an_unreadable_slot_fails() {
        use crate::ccountry::synthetic::World;
        const COUNT: u32 = 4;
        let mut world = World::new(0, COUNT, 0x10);
        let manager = world.alloc(0x100);
        let operatives = world.alloc((COUNT as usize + 1) * 8);
        let operative_objects = world.alloc((COUNT as usize + 1) * 0x20);
        let graph = world.alloc(0x100);
        let pointers = world.alloc((COUNT as usize - 1) * 8);
        let networks = world.alloc((COUNT as usize - 1) * 0x100);
        let gs = world.game_state();
        world.u64(
            gs + cgame_state::STRATEGIC_OPERATIVE_MANAGER,
            manager as u64,
        );
        world.array(
            manager + cstrategic_operative_manager::OPERATIVES_DATA,
            operatives,
            COUNT + 1,
        );
        for slot in 1..=COUNT as usize {
            let operative = operative_objects + slot * 0x20;
            world.u64(operatives + slot * 8, operative as u64);
            world.u64(operative, cstrategic_operative::VTABLE_RVA.0 as u64);
            world.u32(operative + cstrategic_operative::OWNER, slot as u32);
        }
        let owner = operative_objects + 0x20;
        world.array(
            owner + cstrategic_operative::NETWORKS_DATA,
            pointers,
            COUNT - 1,
        );
        for index in 0..COUNT as usize - 1 {
            let network = networks + index * 0x100;
            world.u64(pointers + index * 8, network as u64);
            world.u64(network, VTABLE_RVA.0 as u64);
            world.u32(network + TARGET, index as u32 + 2);
            world.u64(
                network + LOCAL_NETWORK,
                clocal_intel_network::VTABLE_RVA.0 as u64,
            );
            world.u64(
                network + LOCAL_NETWORK + clocal_intel_network::GRAPH,
                graph as u64,
            );
        }
        // Live country 3, a target of country 1, has no operative object.
        world.u64(operatives + 3 * 8, 0);
        let mut states = Vec::new();
        let mut summaries = Vec::new();
        let captured =
            walk_with_summaries(&world.memory(), 0, gs, &mut states, &mut summaries);
        assert_eq!(captured.states, Outcome::Rows { len: 0 });
        assert_eq!(
            captured.summaries,
            Outcome::Rows {
                len: COUNT as usize - 1
            }
        );
        assert_eq!(
            summaries[..COUNT as usize - 1]
                .iter()
                .map(|row| (row.owner_country, row.target_country))
                .collect::<Vec<_>>(),
            [(1, 2), (1, 3), (1, 4)]
        );
        let unavailable = Outcome::Unavailable {
            reason: "intel strategic operative slot unreadable or outside the vector",
        };
        // A vector without the last live country's slot is not an absence.
        world.u32(
            manager + cstrategic_operative_manager::OPERATIVES_DATA + 12,
            COUNT,
        );
        assert_eq!(walk(&world.memory(), 0, gs, &mut Vec::new()), unavailable);
        let captured =
            walk_with_summaries(&world.memory(), 0, gs, &mut states, &mut summaries);
        assert_eq!(captured.states, unavailable);
        assert!(matches!(captured.summaries, Outcome::Unavailable { .. }));
        // Nor is a slot that cannot be read.
        world.array(
            manager + cstrategic_operative_manager::OPERATIVES_DATA,
            0x4000_0000,
            COUNT + 1,
        );
        assert_eq!(walk(&world.memory(), 0, gs, &mut Vec::new()), unavailable);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x11000];
        header(&mut bytes, GS + cgame_state::COUNTRIES, 0x3000, 4);
        header(&mut bytes, GS + cgame_state::COUNTRY_LINKS, 0x3100, 4);
        for (index, (country, link)) in [(0, 0), (0x4000, 1), (0x6000, 2), (0x4000, 1)]
            .into_iter()
            .enumerate()
        {
            p64(&mut bytes, 0x3000 + index * 8, country);
            p32(&mut bytes, 0x3100 + index * 4, link);
        }
        for (country, id) in [(0x4000, 3), (0x6000, 2)] {
            p64(&mut bytes, country, ccountry::VTABLE_RVA.0 as u64);
            p32(&mut bytes, country + ccountry::ID, id);
        }
        p64(
            &mut bytes,
            GS + cgame_state::STRATEGIC_OPERATIVE_MANAGER,
            MANAGER as u64,
        );
        header(
            &mut bytes,
            MANAGER + cstrategic_operative_manager::OPERATIVES_DATA,
            0x9100,
            4,
        );
        p64(&mut bytes, 0x9108, STRATEGIC as u64);
        p64(&mut bytes, 0x9110, 0xa400);
        p64(
            &mut bytes,
            STRATEGIC,
            cstrategic_operative::VTABLE_RVA.0 as u64,
        );
        p32(&mut bytes, STRATEGIC + cstrategic_operative::OWNER, 3);
        p64(&mut bytes, 0xa400, cstrategic_operative::VTABLE_RVA.0 as u64);
        p32(&mut bytes, 0xa400 + cstrategic_operative::OWNER, 2);
        header(
            &mut bytes,
            STRATEGIC + cstrategic_operative::NETWORKS_DATA,
            0xa800,
            1,
        );
        p64(&mut bytes, 0xa800, NETWORK as u64);
        p64(&mut bytes, NETWORK, VTABLE_RVA.0 as u64);
        p32(&mut bytes, NETWORK + TARGET, 2);
        p64(
            &mut bytes,
            NETWORK + LOCAL_NETWORK,
            clocal_intel_network::VTABLE_RVA.0 as u64,
        );
        p64(
            &mut bytes,
            NETWORK + LOCAL_NETWORK + clocal_intel_network::GRAPH,
            GRAPH as u64,
        );
        p64(
            &mut bytes,
            GRAPH + intel_network_graph::ROWS_BEGIN,
            GRAPH_ROWS as u64,
        );
        p64(
            &mut bytes,
            GRAPH + intel_network_graph::ROWS_END,
            (GRAPH_ROWS + 2 * intel_network_graph_row::STRIDE) as u64,
        );
        header(
            &mut bytes,
            GRAPH + intel_network_graph::VERTICES_DATA,
            VERTICES,
            2,
        );
        header(&mut bytes, NETWORK + SUBNETWORKS_DATA, SUBNETWORKS, 2);
        for (index, (id, strength)) in [(1, i64::MIN), (i32::MAX, i64::MAX)]
            .into_iter()
            .enumerate()
        {
            p32(
                &mut bytes,
                VERTICES + index * intel_network_graph_vertex::STRIDE,
                id as u32,
            );
            p64(
                &mut bytes,
                VERTICES + index * intel_network_graph_vertex::STRIDE + 8,
                index as u64,
            );
            p32(
                &mut bytes,
                GRAPH_ROWS
                    + index * intel_network_graph_row::STRIDE
                    + intel_network_graph_row::SUBNETWORK_SELECTOR,
                index as u32,
            );
            let row = GRAPH_ROWS + index * intel_network_graph_row::STRIDE;
            p64(
                &mut bytes,
                row + intel_network_graph_row::STATE_STRENGTH,
                (!strength) as u64,
            );
            p32(
                &mut bytes,
                row + intel_network_graph_row::STATE_ID,
                id as u32,
            );
            let base = 10 * (index as u64 + 1);
            p64(&mut bytes, row + intel_network_graph_row::GAIN, base + 1);
            p64(
                &mut bytes,
                row + intel_network_graph_row::GAIN_FROM_OPERATIVES,
                base + 2,
            );
            p64(
                &mut bytes,
                row + intel_network_graph_row::GAIN_FROM_ADJACENCY,
                base + 3,
            );
            let subnetwork = SUBNETWORKS + index * csub_intel_network::STRIDE;
            p64(
                &mut bytes,
                subnetwork,
                csub_intel_network::VTABLE_RVA.0 as u64,
            );
            p64(
                &mut bytes,
                subnetwork + csub_intel_network::STRENGTH,
                strength as u64,
            );
            for (offset, value) in [
                (csub_intel_network::GAIN_FACTOR, base + 4),
                (csub_intel_network::STRENGTH_TARGET, base + 5),
                (csub_intel_network::TARGET_FROM_OPERATIVES, base + 6),
                (
                    csub_intel_network::TARGET_FROM_COUNTERINTELLIGENCE,
                    base + 7,
                ),
            ] {
                p64(&mut bytes, subnetwork + offset, value);
            }
            bytes[subnetwork + csub_intel_network::QUIET] = index as u8;
        }
        // The local list makes subnetwork 1 quiet and is one byte too short
        // to cover a selector of 2.
        header(
            &mut bytes,
            NETWORK + LOCAL_NETWORK + clocal_intel_network::QUIET_DATA,
            QUIET,
            2,
        );
        bytes[QUIET] = 0;
        bytes[QUIET + 1] = 1;
        bytes
    }
    fn capture(bytes: &[u8], output: &mut Vec<Row>) -> Outcome {
        walk(&SliceMemory { base: 0, bytes }, 0, GS, output)
    }

    #[test]
    fn state_strengths_preserve_native_aliases_and_all_i64_bits() {
        let bytes = fixture();
        let mut output = vec![Row::default(); 2];
        assert_eq!(capture(&bytes, &mut output), Outcome::Rows { len: 2 });
        assert_eq!(
            output[0],
            Row {
                native_ordinal: 0,
                owner_country: 3,
                target_country: 2,
                state_id: 1,
                strength_raw: Some(i64::MIN),
                subnetwork_selector: 0,
                local_strength_raw: i64::MAX,
                gain_raw: 11,
                gain_from_operatives_raw: 12,
                gain_from_adjacency_raw: 13,
                local_quiet: 0,
                gain_factor_raw: Some(14),
                strength_target_raw: Some(15),
                target_from_operatives_raw: Some(16),
                target_from_counterintelligence_raw: Some(17),
                subnetwork_quiet: 0,
            }
        );
        assert_eq!(
            output[1],
            Row {
                native_ordinal: 1,
                owner_country: 3,
                target_country: 2,
                state_id: i32::MAX,
                strength_raw: Some(i64::MAX),
                subnetwork_selector: 1,
                local_strength_raw: i64::MIN,
                gain_raw: 21,
                gain_from_operatives_raw: 22,
                gain_from_adjacency_raw: 23,
                local_quiet: 1,
                gain_factor_raw: Some(24),
                strength_target_raw: Some(25),
                target_from_operatives_raw: Some(26),
                target_from_counterintelligence_raw: Some(27),
                subnetwork_quiet: 1,
            }
        );
        assert_eq!(
            count(&SliceMemory { base: 0, bytes: &bytes }, 0, GS),
            Outcome::Truncated {
                len: 0,
                unread_rows: 2
            }
        );
    }

    #[test]
    fn a_selector_past_the_local_quiet_list_is_not_quiet_but_keeps_its_subnetwork_flag() {
        let mut bytes = fixture();
        header(
            &mut bytes,
            NETWORK + LOCAL_NETWORK + clocal_intel_network::QUIET_DATA,
            QUIET,
            1,
        );
        let mut output = vec![Row::default(); 2];
        assert_eq!(capture(&bytes, &mut output), Outcome::Rows { len: 2 });
        assert_eq!(output[1].local_quiet, 0);
        assert_eq!(output[1].subnetwork_quiet, 1);
        // With the full list the same selector reads its quiet byte.
        header(
            &mut bytes,
            NETWORK + LOCAL_NETWORK + clocal_intel_network::QUIET_DATA,
            QUIET,
            2,
        );
        assert_eq!(capture(&bytes, &mut output), Outcome::Rows { len: 2 });
        assert_eq!(output[1].local_quiet, 1);
    }

    #[test]
    fn a_graph_row_naming_another_state_than_its_vertex_fails() {
        let mut bytes = fixture();
        p32(
            &mut bytes,
            GRAPH_ROWS + intel_network_graph_row::STRIDE + intel_network_graph_row::STATE_ID,
            7,
        );
        let mut output = vec![Row::default(); 2];
        assert_eq!(
            capture(&bytes, &mut output),
            Outcome::Unavailable {
                reason: "intel graph row state differs from its vertex"
            }
        );
    }

    #[test]
    fn every_negative_selector_preserves_vertex_without_reading_a_subnetwork() {
        let mut bytes = fixture();
        header(&mut bytes, NETWORK + SUBNETWORKS_DATA, 0, 0);
        let mut output = vec![Row::default(); 2];
        for first in [-1_i32, -2, i32::MIN] {
            for index in 0..2 {
                p32(
                    &mut bytes,
                    GRAPH_ROWS
                        + index * intel_network_graph_row::STRIDE
                        + intel_network_graph_row::SUBNETWORK_SELECTOR,
                    first as u32,
                );
            }
            assert_eq!(capture(&bytes, &mut output), Outcome::Rows { len: 2 });
            assert_eq!(output[0].state_id, 1);
            assert_eq!(output[1].state_id, i32::MAX);
            assert!(
                output
                    .iter()
                    .all(|row| row.strength_raw.is_none() && row.subnetwork_selector == first)
            );
        }
    }

    #[test]
    fn scattered_graph_rows_cross_batch_boundary_and_capacity_remains_exact() {
        let mut bytes = fixture();
        bytes.resize(0x14000, 0);
        let begin = 0x12000;
        p64(
            &mut bytes,
            GRAPH + intel_network_graph::ROWS_BEGIN,
            begin as u64,
        );
        p64(
            &mut bytes,
            GRAPH + intel_network_graph::ROWS_END,
            (begin + 65 * intel_network_graph_row::STRIDE) as u64,
        );
        header(
            &mut bytes,
            GRAPH + intel_network_graph::VERTICES_DATA,
            VERTICES,
            65,
        );
        for index in 0..65 {
            p32(
                &mut bytes,
                VERTICES + index * intel_network_graph_vertex::STRIDE,
                index as u32 + 1,
            );
            p64(
                &mut bytes,
                VERTICES + index * intel_network_graph_vertex::STRIDE + 8,
                (64 - index) as u64,
            );
            p32(
                &mut bytes,
                begin
                    + (64 - index) * intel_network_graph_row::STRIDE
                    + intel_network_graph_row::SUBNETWORK_SELECTOR,
                if index % 2 == 0 { 1 } else { (-2_i32) as u32 },
            );
            p32(
                &mut bytes,
                begin
                    + (64 - index) * intel_network_graph_row::STRIDE
                    + intel_network_graph_row::STATE_ID,
                index as u32 + 1,
            );
        }
        let mut rows = vec![Row::default(); 65];
        assert_eq!(capture(&bytes, &mut rows), Outcome::Rows { len: 65 });
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(row.state_id, index as i32 + 1);
            assert_eq!(row.strength_raw, (index % 2 == 0).then_some(i64::MAX));
            assert_eq!(row.subnetwork_selector, if index % 2 == 0 { 1 } else { -2 });
        }
        // Reading a bad row fails the table.
        p32(
            &mut bytes,
            begin + intel_network_graph_row::SUBNETWORK_SELECTOR,
            2,
        );
        assert!(matches!(
            capture(&bytes, &mut rows),
            Outcome::Unavailable {
                reason: "intel subnetwork selector outside array"
            }
        ));
    }

    #[test]
    fn network_absence_and_empty_graph_are_valid_but_corrupt_headers_are_not() {
        let mut bytes = fixture();
        let mut output = vec![Row::default(); 2];
        p64(&mut bytes, GS + cgame_state::STRATEGIC_OPERATIVE_MANAGER, 0);
        assert_eq!(capture(&bytes, &mut output), Outcome::Rows { len: 0 });
        p64(
            &mut bytes,
            GS + cgame_state::STRATEGIC_OPERATIVE_MANAGER,
            MANAGER as u64,
        );
        header(&mut bytes, GRAPH + intel_network_graph::VERTICES_DATA, 0, 0);
        p64(&mut bytes, GRAPH + intel_network_graph::ROWS_BEGIN, 0);
        p64(&mut bytes, GRAPH + intel_network_graph::ROWS_END, 0);
        header(&mut bytes, NETWORK + SUBNETWORKS_DATA, 0, 0);
        assert_eq!(capture(&bytes, &mut output), Outcome::Rows { len: 0 });
        p32(&mut bytes, NETWORK + SUBNETWORKS_COUNT, 1);
        assert!(matches!(
            capture(&bytes, &mut output),
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn invalid_roots_graph_indices_selectors_and_partial_strength_reads_fail() {
        let fixture = fixture();
        let mut output = vec![Row::default(); 2];
        for (offset, value) in [
            (STRATEGIC + cstrategic_operative::OWNER, 2),
            (NETWORK + TARGET, 3),
            (NETWORK + SUBNETWORKS_COUNT, u32::MAX),
            (GRAPH_ROWS + intel_network_graph_row::SUBNETWORK_SELECTOR, 2),
            (VERTICES + intel_network_graph_vertex::STRIDE, 1),
        ] {
            let mut bytes = fixture.clone();
            p32(&mut bytes, offset, value);
            assert!(
                matches!(capture(&bytes, &mut output), Outcome::Unavailable { .. }),
                "offset {offset:#x}"
            );
        }
        let mut bytes = fixture.clone();
        p64(&mut bytes, VERTICES + 8, u64::MAX);
        assert!(matches!(
            capture(&bytes, &mut output),
            Outcome::Unavailable { .. }
        ));
        let mut bytes = fixture.clone();
        p64(&mut bytes, SUBNETWORKS, 0);
        assert!(matches!(
            capture(&bytes, &mut output),
            Outcome::Unavailable { .. }
        ));
        let end = SUBNETWORKS + csub_intel_network::STRIDE + csub_intel_network::STRENGTH + 7;
        assert!(matches!(
            capture(&fixture[..end], &mut output),
            Outcome::Unavailable { .. }
        ));
    }
}
