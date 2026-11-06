//! CCountrySupplySystem, one object per canonical country slot.
//!
//! Paired null holes in the country and supply vectors are possible. The owner
//! backlink must resolve through CGameState's country-link mapping; a raw
//! CCountry tag is not its canonical vector index.

use crate::{
    Memory, cpdx_array::Array, csupply_system, hub_motorization_entry, memory::Raw, read,
    supply_node_record, walk::Outcome,
};

/// CCountrySupplySystem allocation size.
pub const SIZE: usize = 0x290;
/// CCountrySupplySystem primary vtable.
pub const VTABLE_RVA: usize = 0x02a1_ec78;
/// CCountrySupplySystem secondary vtable.
pub const SECONDARY_VTABLE_RVA: usize = 0x02a1_ecc8;
/// CCountrySupplySystem secondary-base displacement.
pub const SECONDARY_BASE: usize = 0x08;
/// CCountrySupplySystem owning CSupplySystem pointer.
pub const SUPPLY_SYSTEM: usize = 0x70;
/// CCountrySupplySystem owning CCountry pointer.
pub const COUNTRY: usize = 0x78;
/// CCountrySupplySystem companion pointer, validated against the canonical array slot.
pub const COMPANION: usize = 0x80;
/// Allocated train array: 0x18-byte entries with a full variant reference first and the signed
/// integer count after it.
pub const ALLOCATED_TRAINS: usize = 0x120;
/// Allocated trucks, signed i32. Add, return and distribution paths update
/// this cache; post-load roster counts add without first clearing it.
pub const ALLOCATED_TRUCKS: usize = 0x138;
/// CCountrySupplySystem disrupted-supply bucket pointer.
pub const DISRUPTED_BUCKETS: usize = 0x90;
/// CCountrySupplySystem disrupted-supply occupied u32 count.
pub const DISRUPTED_COUNT: usize = 0x98;
/// CCountrySupplySystem disrupted-supply u32 bucket mask.
pub const DISRUPTED_MASK: usize = 0x9c;
/// CCountrySupplySystem disrupted-supply u8 maximum probe distance.
pub const DISRUPTED_MAX_DISTANCE: usize = 0xa0;
/// Hub-setting array pointer, with 0x28-byte records.
pub const HUB_SETTINGS_DATA: usize = 0x188;
/// CCountrySupplySystem hub-settings u32 count.
pub const HUB_SETTINGS_COUNT: usize = 0x194;
/// CCountrySupplySystem foreign-homebase record array pointer.
pub const FOREIGN_HOMEBASE_DATA: usize = 0x1e0;
/// CCountrySupplySystem foreign-homebase u32 count.
pub const FOREIGN_HOMEBASE_COUNT: usize = 0x1ec;

/// Ordered temporary-supply deployments. Floating-harbour lookup
/// returns the first province match in this array.
pub const FLOATING_HARBOR_DEPLOYMENTS: usize = 0x1b0;
pub const FLOATING_HARBOR_DEPLOYMENT_STRIDE: usize = 0x68;
pub const FLOATING_HARBOR_PROVINCE: usize = 0x48;
pub const FLOATING_HARBOR_STATUS: usize = 0x60;

/// One deployment record: its supply system's canonical country slot (the key the map's
/// lookup uses), its place in that system's array (the first-match order), its province and
/// its status byte.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FloatingHarborDeployment {
    pub country: u32,
    pub ordinal: u32,
    pub province: i32,
    pub status: u8,
}

/// Reads every country supply system's deployment records in slot and array order. A short
/// output reports the exact number of records it did not read.
pub fn walk_floating_harbor_deployments(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [FloatingHarborDeployment],
) -> Outcome {
    walk_floating_harbor_deployments_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Fixed(output),
    )
}

pub fn walk_floating_harbor_deployments_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<FloatingHarborDeployment>,
) -> Outcome {
    walk_floating_harbor_deployments_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(output),
    )
}

pub fn walk_floating_harbor_deployments_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: crate::walk::Buffer<'_, FloatingHarborDeployment>,
) -> Outcome {
    let result = (|| {
        let systems = csupply_system::Reader::new(memory, base, game_state)?;
        let (mut len, mut unread) = (0_usize, 0_u32);
        for slot in 1..systems.count() {
            let Some(system) = systems.at(slot)? else {
                continue;
            };
            let records = array(
                memory,
                system.pointer,
                FLOATING_HARBOR_DEPLOYMENTS,
                FLOATING_HARBOR_DEPLOYMENT_STRIDE,
            )?;
            // The floating-harbour lookup reaches this complete array.
            output.ensure(
                len.checked_add(records.count as usize)
                    .ok_or("floating harbour deployment count overflow")?,
            )?;
            let take = (records.count as usize).min(output.len() - len);
            for ordinal in 0..take {
                let address = records
                    .element(ordinal as u32, FLOATING_HARBOR_DEPLOYMENT_STRIDE)
                    .and_then(|address| address.checked_add(FLOATING_HARBOR_PROVINCE))
                    .ok_or("floating harbour deployment address overflow")?;
                // The span from the province through the status byte, and nothing past it.
                let bytes = read::<[u8; FLOATING_HARBOR_STATUS - FLOATING_HARBOR_PROVINCE + 1]>(
                    memory, address,
                )
                .ok_or("floating harbour deployment unreadable")?;
                output[len] = FloatingHarborDeployment {
                    country: slot,
                    ordinal: ordinal as u32,
                    province: i32::from_le_bytes(bytes[..4].try_into().expect("fixed span")),
                    status: bytes[FLOATING_HARBOR_STATUS - FLOATING_HARBOR_PROVINCE],
                };
                len += 1;
            }
            unread = unread
                .checked_add(records.count - take as u32)
                .ok_or("floating harbour deployment count overflow")?;
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
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Current native CCountry link, resolved from the settings owner's canonical slot.
    pub owner: u32,
    /// Native CCountryTag link stored in the settings map; aliases are preserved.
    pub actor: u32,
    pub node_index: u32,
    pub map_entry_index: u32,
    pub province: i32,
    pub node_type: i32,
    /// The engine reads one byte; adjacent alignment bytes are not a value.
    pub level: u8,
}

/// Reads explicit hub settings through the canonical-country-indexed supply
/// vector. Empty maps emit no rows: absence does not imply level zero. Capacity
/// limits only stored output; remaining map counts determine the exact shortfall.
/// No allocation, retry, inferred motorization or pointer escapes occur.
pub fn walk(memory: &impl Memory, base: usize, game_state: usize, output: &mut [Row]) -> Outcome {
    walk_buffer(memory, base, game_state, crate::walk::Buffer::Fixed(output))
}

pub fn walk_growing(
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
    mut output: crate::walk::Buffer<'_, Row>,
) -> Outcome {
    let result = (|| {
        let systems = csupply_system::Reader::new(memory, base, game_state)?;
        let mut len = 0_usize;
        let mut unread_rows = 0_u32;
        for slot in 1..systems.count() {
            let Some(system) = systems.at(slot)? else {
                continue;
            };
            let nodes = array(
                memory,
                system.pointer,
                HUB_SETTINGS_DATA,
                supply_node_record::STRIDE,
            )?;
            for node_index in 0..nodes.count {
                let node = nodes
                    .element(node_index, supply_node_record::STRIDE)
                    .ok_or("hub settings node overflow")?;
                let entries = array(
                    memory,
                    node,
                    supply_node_record::MOTORIZATION_DATA,
                    hub_motorization_entry::STRIDE,
                )?;
                output.ensure(
                    len.checked_add(entries.count as usize)
                        .ok_or("hub settings row count overflow")?,
                )?;
                if entries.count == 0 {
                    continue;
                }
                if len == output.len() {
                    unread_rows = unread_rows
                        .checked_add(entries.count)
                        .ok_or("hub settings row count overflow")?;
                    continue;
                }
                let identity = field::<[u8; 8]>(memory, node, supply_node_record::PROVINCE)?;
                let province = i32::from_le_bytes(
                    identity[..4]
                        .try_into()
                        .map_err(|_| "hub province decode")?,
                );
                let node_type =
                    i32::from_le_bytes(identity[4..].try_into().map_err(|_| "hub type decode")?);
                for map_entry_index in 0..entries.count {
                    if len == output.len() {
                        unread_rows = unread_rows
                            .checked_add(entries.count - map_entry_index)
                            .ok_or("hub settings row count overflow")?;
                        break;
                    }
                    let entry = entries
                        .element(map_entry_index, hub_motorization_entry::STRIDE)
                        .ok_or("hub motorization entry overflow")?;
                    let bytes = read::<[u8; 5]>(memory, entry)
                        .ok_or("hub motorization entry unreadable")?;
                    let actor =
                        u32::from_le_bytes(bytes[..4].try_into().map_err(|_| "hub actor decode")?);
                    systems.resolve_country(actor)?;
                    output[len] = Row {
                        owner: system.country.id,
                        actor,
                        node_index,
                        map_entry_index,
                        province,
                        node_type,
                        level: bytes[hub_motorization_entry::LEVEL],
                    };
                    len += 1;
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

fn field<T: Raw>(memory: &impl Memory, pointer: usize, offset: usize) -> Result<T, &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|address| read(memory, address))
        .ok_or("country supply field unreadable")
}

fn array(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
    stride: usize,
) -> Result<Array, &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|address| Array::read(memory, address, stride))
        .ok_or("country supply array unreadable or invalid")
}

/// Shared per-country train/access operands; the node count makes empty node
/// censuses explicit. Access bytes are indexed by canonical country slot,
/// joined through CountryLinks, not by the raw Countries id.
///
/// The access array is kept sparse: `access_count` is its native count, and
/// `runs_len` runs of the walk's run arena from `runs_start` locate its maximal
/// runs of nonzero bytes as `[native start, length]`, whose bytes follow one
/// another in the value arena from `values_start`. An in-array index no run
/// covers is a native zero byte.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TrainRow {
    pub country: u32,
    pub node_count: u32,
    pub entries_start: usize,
    pub entries_len: usize,
    pub access_count: u32,
    pub runs_start: usize,
    pub runs_len: usize,
    pub values_start: usize,
    pub values_len: usize,
    pub allocated_trucks: Option<i32>,
    pub required_trucks_raw: Option<i64>,
    pub required_trains: Option<i32>,
}
pub struct TrainReport {
    pub outcome: Outcome,
    pub entries: usize,
    pub required_entries: usize,
    /// Nonzero access bytes stored and needed.
    pub access: usize,
    pub required_access: usize,
    /// Access runs stored and needed.
    pub runs: usize,
    pub required_runs: usize,
    /// Appended scalar faults never withhold the original inventory payload.
    pub vehicle_failures: [u32; 3],
}
pub const VEHICLE_READ_FAILURES: [&str; 3] = [
    "supply allocated trucks unreadable",
    "supply required trucks unreadable",
    "supply required trains unreadable",
];
/// Hard bound on allocated entries in any one country. Reads the 12 consumed
/// bytes per entry in 64-entry batches and authenticates every full reference.
/// Access bytes read per guarded copy while scanning for nonzero runs: one
/// copy per country up to 16,384 controllers, as the former dense read made.
const ACCESS_CHUNK: usize = 16 * 1024;

/// One access array's sparse demand, and whether all of it was stored.
struct SparseAccess {
    values: usize,
    runs: usize,
    stored: bool,
}

/// Scans `source` in bulk copies, appending its nonzero bytes to `values` and
/// one `[start, length]` per maximal nonzero run to `runs` while they fit. The
/// whole array is always read, so the demand is exact even when nothing fits.
fn sparse_access(
    memory: &impl Memory,
    source: Array,
    mut values: crate::walk::Buffer<'_, u8>,
    mut runs: crate::walk::Buffer<'_, [u32; 2]>,
) -> Result<SparseAccess, &'static str> {
    let mut demand = SparseAccess {
        values: 0,
        runs: 0,
        stored: true,
    };
    let mut close = |demand: &mut SparseAccess, start: u32, end: u32| {
        runs.ensure(
            demand
                .runs
                .checked_add(1)
                .ok_or("supply access run count overflow")?,
        )?;
        match runs.get_mut(demand.runs) {
            Some(run) if demand.stored => *run = [start, end - start],
            _ => demand.stored = false,
        }
        demand.runs += 1;
        Ok::<(), &'static str>(())
    };
    let mut chunk = [0_u8; ACCESS_CHUNK];
    let mut open: Option<u32> = None;
    let mut first = 0_u32;
    while first < source.count {
        let n = (source.count - first).min(ACCESS_CHUNK as u32) as usize;
        let address = source
            .data
            .checked_add(first as usize)
            .ok_or("supply controller access address overflow")?;
        if !crate::read_slice(memory, address, &mut chunk[..n]) {
            return Err("supply controller access unreadable");
        }
        let bytes = &chunk[..n];
        let mut at = 0;
        while at < n {
            if open.is_none() {
                // Most controllers are denied: skip zero words outside a run.
                while at + 8 <= n && bytes[at..at + 8] == [0; 8] {
                    at += 8;
                }
                if at == n {
                    break;
                }
            }
            let index = first + at as u32;
            if bytes[at] == 0 {
                if let Some(start) = open.take() {
                    close(&mut demand, start, index)?;
                }
            } else {
                open.get_or_insert(index);
                values.ensure(
                    demand
                        .values
                        .checked_add(1)
                        .ok_or("supply access byte count overflow")?,
                )?;
                match values.get_mut(demand.values) {
                    Some(value) if demand.stored => *value = bytes[at],
                    _ => demand.stored = false,
                }
                demand.values += 1;
            }
            at += 1;
        }
        first += n as u32;
    }
    if let Some(start) = open {
        close(&mut demand, start, source.count)?;
    }
    Ok(demand)
}

/// Walks every canonical supply slot. The country census, each controller-access
/// array and the aggregate demand are bounded only by the live census and the
/// validated native headers (count within capacity, capacity within `i32`), with
/// checked sums; no country count is chosen here. Access arrays are stored
/// sparse, so the arenas scale with nonzero access, not with the square of the
/// census. Arena or row pressure omits whole countries, reports `Truncated` and
/// keeps the exact demand so the collector grows its buffers off-thread; an
/// omitted country's access array is still read to measure that demand.
#[cfg(test)]
pub fn walk_trains(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut [TrainRow],
    entries: &mut [crate::supply_train_entry::Row],
    access: &mut [u8],
    runs: &mut [[u32; 2]],
) -> TrainReport {
    walk_trains_buffer(
        memory,
        base,
        state,
        crate::walk::Buffer::Fixed(output),
        crate::walk::Buffer::Fixed(entries),
        crate::walk::Buffer::Fixed(access),
        crate::walk::Buffer::Fixed(runs),
        None,
    )
}

pub fn walk_trains_growing(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut Vec<TrainRow>,
    entries: &mut Vec<crate::supply_train_entry::Row>,
    access: &mut Vec<u8>,
    runs: &mut Vec<[u32; 2]>,
) -> TrainReport {
    walk_trains_buffer(
        memory,
        base,
        state,
        crate::walk::Buffer::Growing(output),
        crate::walk::Buffer::Growing(entries),
        crate::walk::Buffer::Growing(access),
        crate::walk::Buffer::Growing(runs),
        None,
    )
}

/// [`walk_trains_growing`] for the countries `select` admits by native
/// country id: an unselected slot's supply objects are not read, and it is
/// neither a row nor an unread row. A selected country's row equals the
/// whole walk's.
#[allow(clippy::too_many_arguments)]
pub fn walk_trains_selected_growing(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut Vec<TrainRow>,
    entries: &mut Vec<crate::supply_train_entry::Row>,
    access: &mut Vec<u8>,
    runs: &mut Vec<[u32; 2]>,
    select: &dyn Fn(u32) -> bool,
) -> TrainReport {
    walk_trains_buffer(
        memory,
        base,
        state,
        crate::walk::Buffer::Growing(output),
        crate::walk::Buffer::Growing(entries),
        crate::walk::Buffer::Growing(access),
        crate::walk::Buffer::Growing(runs),
        Some(select),
    )
}

/// `select`, when present, admits countries by native id before their supply
/// objects are read ([`walk_trains_selected_growing`]); without it every
/// slot is read and checked.
#[allow(clippy::too_many_arguments)]
pub fn walk_trains_buffer(
    memory: &impl Memory,
    base: usize,
    state: usize,
    mut output: crate::walk::Buffer<'_, TrainRow>,
    mut entries: crate::walk::Buffer<'_, crate::supply_train_entry::Row>,
    mut access: crate::walk::Buffer<'_, u8>,
    mut runs: crate::walk::Buffer<'_, [u32; 2]>,
    select: Option<&dyn Fn(u32) -> bool>,
) -> TrainReport {
    let mut used_entries = 0;
    let mut required_entries = 0_usize;
    let mut used_access = 0;
    let mut required_access = 0_usize;
    let mut used_runs = 0;
    let mut required_runs = 0_usize;
    let mut vehicle_failures = [0_u32; 3];
    let result = (|| {
        use crate::{
            memory::{RecordField, RecordLayout, RecordSources},
            supply_companion, supply_train_entry,
        };
        const FIELDS: [RecordField; 1] = [RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 12,
        }];
        const LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&FIELDS, 12).expect("valid train entry layout");
        let systems = csupply_system::Reader::new(memory, base, state)?;
        let mut len = 0_usize;
        let mut total = 0;
        let mut bytes = [0; 64 * 12];
        for slot in 1..systems.count() {
            if let Some(select) = select
                && !systems
                    .country(slot)?
                    .is_some_and(|country| select(country.id))
            {
                continue;
            }
            let Some(system) = systems.at(slot)? else {
                continue;
            };
            total += 1;
            let trains = array(
                memory,
                system.pointer,
                ALLOCATED_TRAINS,
                supply_train_entry::STRIDE,
            )?;
            let controllers = array(
                memory,
                system.companion,
                supply_companion::RELATED_CONTROLLERS,
                1,
            )?;
            let nodes = array(
                memory,
                system.companion,
                supply_companion::NODES,
                crate::supply_node::STRIDE,
            )?;

            required_entries = required_entries
                .checked_add(trains.count as usize)
                .ok_or("train entry demand overflows")?;
            output.ensure(total)?;
            entries.ensure(required_entries)?;
            let fits = len < output.len() && entries.len() - used_entries >= trains.count as usize;
            if fits {
                let mut first = 0;
                while first < trains.count {
                    let count = (trains.count - first).min(64) as usize;
                    if memory.copy_records(
                        RecordSources::Strided {
                            base: trains
                                .element(first, supply_train_entry::STRIDE)
                                .ok_or("train array overflow")?,
                            stride: supply_train_entry::STRIDE,
                            count,
                        },
                        &LAYOUT,
                        &mut bytes,
                    ) != count
                    {
                        return Err("allocated train entries unreadable");
                    }
                    for i in 0..count {
                        entries[used_entries + first as usize + i] =
                            supply_train_entry::decode(memory, base, &bytes[i * 12..(i + 1) * 12])?;
                    }
                    first += count as u32;
                }
            }
            let (values, spans) = if fits {
                (access.suffix(used_access), runs.suffix(used_runs))
            } else {
                (
                    crate::walk::Buffer::Fixed(&mut []),
                    crate::walk::Buffer::Fixed(&mut []),
                )
            };
            let sparse = sparse_access(memory, controllers, values, spans)?;
            required_access = required_access
                .checked_add(sparse.values)
                .ok_or("controller access demand overflows")?;
            required_runs = required_runs
                .checked_add(sparse.runs)
                .ok_or("controller access demand overflows")?;
            if !fits || !sparse.stored {
                continue;
            }
            // Read the stored truck caches passively; the hourly updater must not run here.
            let allocated_trucks = system
                .pointer
                .checked_add(ALLOCATED_TRUCKS)
                .and_then(|address| read::<i32>(memory, address));
            let required_trucks_raw = system
                .companion
                .checked_add(supply_companion::REQUIRED_TRUCKS_RAW)
                .and_then(|address| read::<i64>(memory, address));
            let required_trains = system
                .companion
                .checked_add(supply_companion::REQUIRED_TRAINS)
                .and_then(|address| read::<i32>(memory, address));
            for (failures, missing) in vehicle_failures.iter_mut().zip([
                allocated_trucks.is_none(),
                required_trucks_raw.is_none(),
                required_trains.is_none(),
            ]) {
                *failures = failures.saturating_add(u32::from(missing));
            }
            output[len] = TrainRow {
                country: system.country.id,
                node_count: nodes.count,
                entries_start: used_entries,
                entries_len: trains.count as usize,
                access_count: controllers.count,
                runs_start: used_runs,
                runs_len: sparse.runs,
                values_start: used_access,
                values_len: sparse.values,
                allocated_trucks,
                required_trucks_raw,
                required_trains,
            };
            used_entries += trains.count as usize;
            used_access += sparse.values;
            used_runs += sparse.runs;
            len += 1;
        }
        Ok(if len == total {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: (total - len) as u32,
            }
        })
    })();
    TrainReport {
        outcome: result.unwrap_or_else(|reason| Outcome::Unavailable { reason }),
        entries: used_entries,
        required_entries,
        access: used_access,
        required_access,
        runs: used_runs,
        required_runs,
        vehicle_failures,
    }
}

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrainStorageDemand {
    pub countries: usize,
    pub entries: usize,
    /// Complete byte count is a safe bound for both nonzero values and runs.
    pub access: usize,
}

/// Original train roster and related-controller header demand.
pub fn train_storage_demand(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<TrainStorageDemand, &'static str> {
    let systems = csupply_system::Reader::new(memory, base, state)?;
    let mut demand = TrainStorageDemand::default();
    for slot in 1..systems.count() {
        let Some(system) = systems.at(slot)? else {
            continue;
        };
        demand.countries = demand
            .countries
            .checked_add(1)
            .ok_or("train country demand overflow")?;
        let trains = array(
            memory,
            system.pointer,
            ALLOCATED_TRAINS,
            crate::supply_train_entry::STRIDE,
        )?;
        let access = array(
            memory,
            system.companion,
            crate::supply_companion::RELATED_CONTROLLERS,
            1,
        )?;
        demand.entries = demand
            .entries
            .checked_add(trains.count as usize)
            .ok_or("train entry demand overflow")?;
        demand.access = demand
            .access
            .checked_add(access.count as usize)
            .ok_or("controller access demand overflow")?;
    }
    Ok(demand)
}

#[cfg(test)]
mod tests;
