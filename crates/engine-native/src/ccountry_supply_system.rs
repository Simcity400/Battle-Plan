
use crate::{
    Memory, cpdx_array::Array, csupply_system, hub_motorization_entry, memory::Raw, read,
    supply_node, supply_node_record, walk::Outcome,
};
use engine_sites::Rva;

pub const SIZE: usize = 0x290;
pub const VTABLE_RVA: Rva = Rva(0x02a1_ec78);
pub const SECONDARY_VTABLE_RVA: Rva = Rva(0x02a1_ecc8);
pub const SECONDARY_BASE: usize = 0x08;
pub const SUPPLY_SYSTEM: usize = 0x70;
pub const COUNTRY: usize = 0x78;
pub const COMPANION: usize = 0x80;
pub const ALLOCATED_TRAINS: usize = 0x120;
pub const ALLOCATED_TRUCKS: usize = 0x138;
pub const DISRUPTED_BUCKETS: usize = 0x90;
pub const DISRUPTED_COUNT: usize = 0x98;
pub const DISRUPTED_MASK: usize = 0x9c;
pub const DISRUPTED_MAX_DISTANCE: usize = 0xa0;
pub const HUB_SETTINGS_DATA: usize = 0x188;
pub const HUB_SETTINGS_COUNT: usize = 0x194;
pub const FOREIGN_HOMEBASE_DATA: usize = 0x1e0;
pub const FOREIGN_HOMEBASE_COUNT: usize = 0x1ec;

pub const FLOATING_HARBOR_DEPLOYMENTS: usize = 0x1b0;
pub const FLOATING_HARBOR_DEPLOYMENT_STRIDE: usize = 0x68;
pub const FLOATING_HARBOR_PROVINCE: usize = 0x48;
pub const FLOATING_HARBOR_STATUS: usize = 0x60;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FloatingHarborDeployment {
    pub country: u32,
    pub ordinal: u32,
    pub province: i32,
    pub status: u8,
}

pub fn count_floating_harbor_deployments(memory: &impl Memory, base: usize, game_state: usize) -> Outcome {
    walk_floating_harbor_deployments_buffer(memory, base, game_state, crate::walk::Buffer::Count)
}

pub fn walk_floating_harbor_deployments(
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
    pub owner: u32,
    pub actor: u32,
    pub node_index: u32,
    pub map_entry_index: u32,
    pub province: i32,
    pub node_type: i32,
    pub level: u8,
}

pub fn count(memory: &impl Memory, base: usize, game_state: usize) -> Outcome {
    walk_buffer(memory, base, game_state, crate::walk::Buffer::Count)
}

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
    pub access: usize,
    pub required_access: usize,
    pub runs: usize,
    pub required_runs: usize,
    pub vehicle_failures: [u32; 3],
    pub required_trucks_withheld: Option<&'static str>,
}
pub const VEHICLE_READ_FAILURES: [&str; 3] = [
    "supply allocated trucks unreadable",
    "supply required trucks unverified or unreadable",
    "supply required trains unreadable",
];
const ACCESS_CHUNK: usize = 16 * 1024;
const MOTORIZATION_FACTOR_LEVEL_1: i64 = 50_000;
const MOTORIZATION_FACTOR_LEVEL_2: i64 = 100_000;
const FIXED_SCALE: i64 = 100_000;
const FULL_FULFILLMENT_RATIO: i64 = 100_000;
const FULFILLMENT_SCALE: i64 = 10_000_000_000;

struct TruckDemandWitness<'a, M: Memory> {
    memory: &'a M,
    base: usize,
    state: usize,
    systems: &'a csupply_system::Reader<'a, M>,
    init: Option<Result<TruckWitnessContext, &'static str>>,
    province_controllers: Vec<Option<u32>>,
}

struct TruckWitnessContext {
    truck_cost_raw: i64,
    provinces: Array,
    province_vtable: usize,
    foreign_sources: Array,
}

impl<'a, M: Memory> TruckDemandWitness<'a, M> {
    fn new(
        memory: &'a M,
        base: usize,
        state: usize,
        systems: &'a csupply_system::Reader<'a, M>,
    ) -> Self {
        Self {
            memory,
            base,
            state,
            systems,
            init: None,
            province_controllers: Vec::new(),
        }
    }

    fn context(&mut self) -> Result<&TruckWitnessContext, &'static str> {
        if self.init.is_none() {
            self.init = Some(self.load_context());
        }
        match self.init.as_ref().expect("initialized above") {
            Ok(context) => Ok(context),
            Err(reason) => Err(reason),
        }
    }

    fn load_context(&self) -> Result<TruckWitnessContext, &'static str> {
        let define_at = engine_sites::resolve(
            self.base,
            crate::supply_companion::SUPPLY_HUB_FULL_MOTORIZATION_TRUCK_COST_RVA,
        )
        .ok()
        .ok_or("supply truck cost define site unlocated")?;
        let truck_cost_raw =
            read::<i64>(self.memory, define_at).ok_or("supply truck cost define unreadable")?;
        if truck_cost_raw < 0 {
            return Err("supply truck cost define negative");
        }
        let province_vtable = engine_sites::resolve(self.base, crate::cprovince::VTABLE_RVA)
            .ok()
            .ok_or("supply truck witness province vtable unlocated")?;
        let provinces = self
            .state
            .checked_add(crate::cgame_state::PROVINCES)
            .and_then(|at| Array::read(self.memory, at, 8))
            .ok_or("supply truck witness province array unreadable")?;
        let foreign_sources = array(
            self.memory,
            self.systems.root(),
            csupply_system::FOREIGN_SOURCE_DATA,
            csupply_system::FOREIGN_SOURCE_STRIDE,
        )
        .map_err(|_| "supply truck witness foreign source array unreadable")?;
        Ok(TruckWitnessContext {
            truck_cost_raw,
            provinces,
            province_vtable,
            foreign_sources,
        })
    }

    fn province_controller(&mut self, province: i32) -> Result<u32, &'static str> {
        let index =
            usize::try_from(province).map_err(|_| "supply truck witness province id negative")?;
        let (provinces, expected_vtable) = {
            let ctx = self.context()?;
            (ctx.provinces, ctx.province_vtable)
        };
        if index >= provinces.count as usize {
            return Err("supply truck witness province outside census");
        }
        if index < self.province_controllers.len()
            && let Some(controller) = self.province_controllers[index]
        {
            return Ok(controller);
        }
        let pointer = provinces
            .element(index as u32, 8)
            .and_then(|at| read::<u64>(self.memory, at))
            .and_then(|value| usize::try_from(value).ok())
            .filter(|&pointer| pointer != 0)
            .ok_or("supply truck witness province pointer unreadable")?;
        let vtable = read::<u64>(self.memory, pointer)
            .ok_or("supply truck witness province vtable unreadable")?;
        if vtable != expected_vtable as u64 {
            return Err("supply truck witness province vtable mismatch");
        }
        let id = pointer
            .checked_add(crate::cprovince::ID)
            .and_then(|at| read::<u32>(self.memory, at))
            .ok_or("supply truck witness province id unreadable")?;
        if id != index as u32 {
            return Err("supply truck witness province id mismatch");
        }
        let controller = pointer
            .checked_add(crate::cprovince::CONTROLLER)
            .and_then(|at| read::<u32>(self.memory, at))
            .ok_or("supply truck witness province controller unreadable")?;
        if index >= self.province_controllers.len() {
            let grow = index + 1 - self.province_controllers.len();
            if crate::allocation::reserve(&mut self.province_controllers, grow).is_ok() {
                self.province_controllers.resize(index + 1, None);
                self.province_controllers[index] = Some(controller);
            }
        } else {
            self.province_controllers[index] = Some(controller);
        }
        Ok(controller)
    }

    fn hub_default_level(
        &self,
        settings: Array,
        province: i32,
        kind: i32,
        my_slot: u32,
    ) -> Result<u8, &'static str> {
        let mut lo = 0_u32;
        let mut hi = settings.count;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let record = settings
                .element(mid, supply_node_record::STRIDE)
                .ok_or("supply hub settings address overflow")?;
            let key = read::<[u8; 8]>(self.memory, record)
                .ok_or("supply hub settings key unreadable")?;
            let rec_province = i32::from_le_bytes(key[..4].try_into().unwrap());
            let rec_kind = i32::from_le_bytes(key[4..].try_into().unwrap());
            if (rec_province, rec_kind) < (province, kind) {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        if lo == settings.count {
            return Ok(0);
        }
        let record = settings
            .element(lo, supply_node_record::STRIDE)
            .ok_or("supply hub settings address overflow")?;
        let key =
            read::<[u8; 8]>(self.memory, record).ok_or("supply hub settings key unreadable")?;
        let rec_province = i32::from_le_bytes(key[..4].try_into().unwrap());
        let rec_kind = i32::from_le_bytes(key[4..].try_into().unwrap());
        if (rec_province, rec_kind) != (province, kind) {
            return Ok(0);
        }
        let actors = array(
            self.memory,
            record,
            supply_node_record::MOTORIZATION_DATA,
            hub_motorization_entry::STRIDE,
        )
        .map_err(|_| "supply hub motorization array unreadable")?;
        let mut a_lo = 0_u32;
        let mut a_hi = actors.count;
        while a_lo < a_hi {
            let mid = a_lo + (a_hi - a_lo) / 2;
            let entry = actors
                .element(mid, hub_motorization_entry::STRIDE)
                .ok_or("supply hub motorization entry overflow")?;
            let actor = read::<u32>(self.memory, entry)
                .ok_or("supply hub motorization actor unreadable")?;
            let actor_slot = self
                .systems
                .canonical_slot(actor)?
                .ok_or("supply hub motorization actor unresolved")?;
            if actor_slot < my_slot {
                a_lo = mid + 1;
            } else {
                a_hi = mid;
            }
        }
        if a_lo == actors.count {
            return Ok(0);
        }
        let entry = actors
            .element(a_lo, hub_motorization_entry::STRIDE)
            .ok_or("supply hub motorization entry overflow")?;
        let bytes = read::<[u8; 5]>(self.memory, entry)
            .ok_or("supply hub motorization entry unreadable")?;
        let actor = u32::from_le_bytes(bytes[..4].try_into().unwrap());
        let actor_slot = self
            .systems
            .canonical_slot(actor)?
            .ok_or("supply hub motorization actor unresolved")?;
        if actor_slot == my_slot {
            Ok(bytes[hub_motorization_entry::LEVEL])
        } else {
            Ok(0)
        }
    }

    fn node_motorization_level(
        &self,
        node: usize,
        my_slot: u32,
        default_level: u8,
    ) -> Result<u8, &'static str> {
        let overrides = array(self.memory, node, supply_node::MOTORIZATION_OVERRIDES, 8)
            .map_err(|_| "supply node motorization override array unreadable")?;
        for index in 0..overrides.count {
            let entry = overrides
                .element(index, 8)
                .ok_or("supply node motorization override overflow")?;
            let bytes = read::<[u8; 8]>(self.memory, entry)
                .ok_or("supply node motorization override unreadable")?;
            let actor_slot = i32::from_le_bytes(bytes[..4].try_into().unwrap());
            if actor_slot == my_slot as i32 {
                return Ok(bytes[4]);
            }
        }
        Ok(default_level)
    }

    fn verify(
        &mut self,
        slot: u32,
        companion: usize,
        allocated_trucks: Option<i32>,
    ) -> Result<i64, &'static str> {
        let stored_required = companion
            .checked_add(crate::supply_companion::REQUIRED_TRUCKS_RAW)
            .and_then(|address| read::<i64>(self.memory, address))
            .ok_or("supply required trucks unreadable")?;
        let stored_ratio = companion
            .checked_add(crate::supply_companion::TRUCK_FULFILLMENT_RATIO)
            .and_then(|address| read::<i64>(self.memory, address))
            .ok_or("supply truck fulfillment ratio unreadable")?;
        let (truck_cost_raw, foreign_sources) = {
            let ctx = self.context()?;
            (ctx.truck_cost_raw, ctx.foreign_sources)
        };
        let entry = foreign_sources
            .element(slot, csupply_system::FOREIGN_SOURCE_STRIDE)
            .ok_or("supply truck witness foreign source slot missing")?;
        let related = array(
            self.memory,
            entry,
            csupply_system::FOREIGN_SOURCE_RELATED_COUNTRIES,
            4,
        )
        .map_err(|_| "supply truck witness related countries unreadable")?;
        let mut expected_required = 0_i64;
        for rel_index in 0..related.count {
            let rel_tag = related
                .element(rel_index, 4)
                .and_then(|at| read::<u32>(self.memory, at))
                .ok_or("supply truck witness related country unreadable")?;
            if rel_tag == 0 {
                return Err("supply truck witness encountered null related country");
            }
            let rel_country = self
                .systems
                .resolve_country(rel_tag)
                .map_err(|_| "supply truck witness related country unresolved")?;
            let rel_system = self
                .systems
                .at(rel_country.canonical_slot)
                .map_err(|_| "supply truck witness related supply system unreadable")?
                .ok_or("supply truck witness related supply system absent")?;
            let rel_nodes = array(
                self.memory,
                rel_system.companion,
                crate::supply_companion::NODES,
                supply_node::STRIDE,
            )
            .map_err(|_| "supply truck witness related nodes unreadable")?;
            let rel_settings = array(
                self.memory,
                rel_system.pointer,
                HUB_SETTINGS_DATA,
                supply_node_record::STRIDE,
            )
            .map_err(|_| "supply truck witness related hub settings unreadable")?;
            for node_index in 0..rel_nodes.count {
                let node = rel_nodes
                    .element(node_index, supply_node::STRIDE)
                    .ok_or("supply truck witness node overflow")?;
                let header_bytes = node
                    .checked_add(supply_node::PROVINCE)
                    .and_then(|at| read::<[u8; 8]>(self.memory, at))
                    .ok_or("supply truck witness node header unreadable")?;
                let province = i32::from_le_bytes(header_bytes[..4].try_into().unwrap());
                let kind = u32::from_le_bytes(header_bytes[4..].try_into().unwrap());
                if kind != supply_node::KIND_SUPPLY_HUB && kind != supply_node::KIND_NAVAL_BASE {
                    continue;
                }
                let controller = self.province_controller(province)?;
                if !self.systems.same_country(controller, rel_country.id)? {
                    continue;
                }
                let default_level =
                    self.hub_default_level(rel_settings, province, kind as i32, slot)?;
                let level = self.node_motorization_level(node, slot, default_level)?;
                let factor = match level {
                    0 => 0,
                    1 => MOTORIZATION_FACTOR_LEVEL_1,
                    2 => MOTORIZATION_FACTOR_LEVEL_2,
                    _ => return Err("supply hub motorization level unsupported"),
                };
                let cost = factor
                    .checked_mul(truck_cost_raw)
                    .ok_or("supply truck cost overflow")?
                    / FIXED_SCALE;
                expected_required = expected_required
                    .checked_add(cost)
                    .ok_or("supply required trucks overflow")?;
            }
        }
        if stored_required != expected_required {
            return Err("supply required trucks witness mismatch");
        }
        let expected_ratio = if expected_required == 0 {
            FULL_FULFILLMENT_RATIO
        } else {
            let allocated =
                allocated_trucks.ok_or("supply allocated trucks missing for truck witness")?;
            let numerator = i64::from(allocated)
                .checked_mul(FULFILLMENT_SCALE)
                .ok_or("supply truck fulfillment ratio overflow")?;
            numerator
                .checked_div(expected_required)
                .ok_or("supply truck fulfillment ratio division failed")?
                .min(FULL_FULFILLMENT_RATIO)
        };
        if stored_ratio != expected_ratio {
            return Err("supply truck fulfillment ratio witness mismatch");
        }
        Ok(stored_required)
    }
}

struct SparseAccess {
    values: usize,
    runs: usize,
    stored: bool,
}

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

pub fn walk_trains(
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

#[allow(clippy::too_many_arguments)]
pub fn walk_trains_selected(
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
    let mut required_trucks_withheld = None;
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
        let mut truck_witness = TruckDemandWitness::new(memory, base, state, &systems);
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
                    crate::walk::Buffer::Count,
                    crate::walk::Buffer::Count,
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
            let allocated_trucks = system
                .pointer
                .checked_add(ALLOCATED_TRUCKS)
                .and_then(|address| read::<i32>(memory, address));
            let required_trucks_raw =
                match truck_witness.verify(slot, system.companion, allocated_trucks) {
                    Ok(value) => Some(value),
                    Err(reason) => {
                        required_trucks_withheld.get_or_insert(reason);
                        None
                    }
                };
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
        required_trucks_withheld,
    }
}

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrainStorageDemand {
    pub countries: usize,
    pub entries: usize,
    pub access: usize,
}

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
