
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources},
    read,
    walk::{Buffer, Outcome},
};
use crate::{allocation, ccountry_resources, cresource_exchange, cstrategic_resource};
use engine_native::{ccountry, cgame_state, cstate};
use engine_native_parts::modifier_accumulator;
use engine_native_parts::modifier_definition;
use engine_sites::Rva;

fn effective(
    memory: &impl Memory,
    base: usize,
    accumulator: usize,
    key: u32,
    metadata: &std::cell::Cell<Option<modifier_definition::Operands>>,
) -> Result<i64, &'static str> {
    let value = modifier_accumulator::lookup_at(memory, accumulator, key)?;
    if !value.present {
        return Ok(0);
    }
    if memory.modifier_definitions().is_some() {
        return modifier_definition::effective(memory, base, key, value.raw);
    }
    let operands = match metadata.get() {
        Some(operands) => operands,
        None => {
            let operands = modifier_definition::operands(memory, base, key)?;
            metadata.set(Some(operands));
            operands
        }
    };
    Ok(operands.effective(value.raw))
}

pub const BAD_EFFICIENCY_THRESHOLD_RVA: Rva = Rva(0x33e_3598);

#[cfg(test)]
pub const CLIENT_HANDLE_TYPE: usize = 0x08;
pub const CLIENT_HANDLE_ID: usize = 0x0c;
pub const CLIENT_CONVOYS: usize = 0x28;
pub const CLIENT_EFFICIENCY_A: usize = 0x48;
#[cfg(test)]
pub const CLIENT_EFFICIENCY_B: usize = 0x50;
pub const CLIENT_CONVOYS_BASE: usize = 0x78;

pub const ORIGIN_VTABLE_RVA: Rva = Rva(0x29d_8dc0);
pub const ORIGIN_ROUTE: usize = 0x350;
pub const ORIGIN_STATE: usize = 0x80;
pub const ORIGIN_EFFICIENCY: usize = 0x348;
pub const ORIGIN_ROUTE_FACTOR: usize = 0x3d8;
pub const ORIGIN_RIGHTS: usize = 0x3e0;
pub const RIGHTS_STRIDE: usize = 8;
pub const RIGHTS_ORIGINS: usize = 0x7b8;
pub const RIGHTS_ORIGIN_STRIDE: usize = 0x10;
pub const EXCHANGE_VTABLE_RVA: Rva = cresource_exchange::VTABLE_RVA;
pub const DELIVERY_ROUTES: usize = 0x788;
pub const LEND_LEASE_VTABLE_RVA: Rva = Rva(0x2a9_9cf8);
pub const LEND_LEASE_ROUTE: usize = 0xc0;
pub const LEND_LEASE_RECEIVER: usize = 0xb8;
pub const LEND_LEASE_GIVEN: usize = 0x758;
pub const LEND_LEASE_RECEIVED: usize = 0x770;
const MARKET_STATUS_MANAGER: usize = 0x70;
const MARKET_SELLERS: usize = 0x30;
const MARKET_BUYERS: usize = 0x48;
const MARKET_COUNTRY_STRIDE: usize = 0x18;
pub const CONTRACT_CLIENT: usize = 0xf0;
pub const CONTRACT_CLIENT_VTABLE_RVA: Rva = Rva(0x29e_8330);
pub const CONTRACT_CLIENT_ROUTE: usize = 0xc8;
pub const CONTRACT_SELLER: usize = 0x58;
#[cfg(test)]
pub const CONTRACT_BUYER: usize = 0x5c;
pub const MARKET_OWNED_CONTRACTS: usize = 0;
pub const SUPPLY_VTABLE_RVA: Rva = Rva(0x2a1_eb80);
pub const SUPPLY_ROUTE: usize = 0xa0;
pub const SUPPLY_COUNTRY_SYSTEMS: usize = 0x108;
pub const SUPPLY_TRANSFERS: usize = 0xd0;
pub const SUPPLY_TRANSFER_STRIDE: usize = 40;
pub const SUPPLY_TRANSFER_CLIENT: usize = 0x20;

pub const ROUTE_VTABLE_RVA: Rva = Rva(0x29d_8d70);
pub const ROUTE_KIND: usize = 0x08;
pub const ROUTE_FROM: usize = 0x10;
pub const ROUTE_TO: usize = 0x18;
pub const ROUTE_FROM_PORT: usize = 0x20;
pub const ROUTE_TO_PORT: usize = 0x28;
pub const ROUTE_LINKS: [usize; 3] = [0x30, 0x34, 0x38];
pub const ROUTE_LAND_STATES: usize = 0x48;
pub const ROUTE_SEA_REGIONS: usize = 0x60;
pub const NAVAL: u8 = 2;
const PROVINCE_ID: usize = 0xa4;
const REGION_ID: usize = 0x58;

pub const STATE_TEMPORARY_LIST_COUNT: usize = 0x8bc;
pub const STATE_TEMPORARY_FLAG: usize = 0x1b5;

pub const LOCAL_RESOURCES_FACTOR: u32 = 0x39;
pub const PRODUCTION_OIL_FACTOR: u32 = 0xdd;

pub const SUPPLY_TRANSFER: u8 = 1;
pub const IMPORT: u8 = 2;
pub const LEND_LEASE: u8 = 4;
pub const MARKET: u8 = 6;

pub const ORIGIN_ROUTE_CLASS: u8 = 0;
pub const DELIVERY_ROUTE_CLASS: u8 = 1;
pub const SUPPLY_ROUTE_CLASS: u8 = 2;

const ONE: i128 = 100_000;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RouteKey {
    pub class: u8,
    pub country: u32,
    pub other: u32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct OriginRow {
    pub state: u32,
    pub country: u32,
    pub list: u8,
    pub ordinal: u32,
    pub efficiency: i64,
    pub route_factor: i64,
    pub local_resources_factor: i64,
    pub bad: bool,
    pub convoys: [i32; 2],
    pub convoys_base: i32,
    pub route_address: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RightsRow {
    pub ordinal: u32,
    pub state: u32,
    pub resource: u32,
    pub holder: u32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct OilRow {
    pub country: u32,
    pub factor: i64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TemporaryRow {
    pub state: u32,
    pub resource: u32,
    pub amount: i64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ClientRow {
    pub owner_ordinal: Option<u32>,
    pub other_ordinal: Option<u32>,
    pub address: usize,
    pub handle: u32,
    pub category: u8,
    pub owner: u32,
    pub other: u32,
    pub route: RouteKey,
    pub bad: bool,
    pub convoys: [i32; 2],
    pub efficiency: [i64; 2],
    pub convoys_base: i32,
    pub route_address: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RouteRow {
    pub key: RouteKey,
    pub kind: u8,
    pub from: u32,
    pub to: u32,
    pub source_state: Option<u32>,
    pub destination_state: Option<u32>,
    pub path_start: usize,
    pub path_len: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RegionWeightRow {
    pub region: u32,
    pub weight: i32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RegionalRow {
    pub country: u32,
    pub region: u32,
    pub efficiency: i64,
}

pub struct Outputs<'a> {
    pub origins: Buffer<'a, OriginRow>,
    pub rights: Buffer<'a, RightsRow>,
    pub oil: Buffer<'a, OilRow>,
    pub clients: Buffer<'a, ClientRow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcomes {
    pub origins: Outcome,
    pub rights: Outcome,
    pub oil: Outcome,
    pub clients: Outcome,
}

fn at(address: usize, offset: usize) -> Result<usize, &'static str> {
    address
        .checked_add(offset)
        .ok_or("resources map address overflow")
}

fn pointer(
    memory: &impl Memory,
    address: usize,
    what: &'static str,
) -> Result<usize, &'static str> {
    read::<u64>(memory, address)
        .and_then(|p| usize::try_from(p).ok())
        .ok_or(what)
}

fn array(
    memory: &impl Memory,
    address: usize,
    stride: usize,
    what: &'static str,
) -> Result<Array, &'static str> {
    Array::read(memory, address, stride).ok_or(what)
}

fn element(
    memory: &impl Memory,
    list: Array,
    index: u32,
    stride: usize,
    what: &'static str,
) -> Result<usize, &'static str> {
    list.element(index, stride)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .ok_or(what)
}

fn pair_i64(memory: &impl Memory, address: usize) -> Option<[i64; 2]> {
    let bytes = read::<[u8; 16]>(memory, address)?;
    Some([
        i64::from_le_bytes(bytes[..8].try_into().expect("eight bytes")),
        i64::from_le_bytes(bytes[8..].try_into().expect("eight bytes")),
    ])
}

fn pair_u32(memory: &impl Memory, address: usize) -> Option<[u32; 2]> {
    let bytes = read::<[u8; 8]>(memory, address)?;
    Some([
        u32::from_le_bytes(bytes[..4].try_into().expect("four bytes")),
        u32::from_le_bytes(bytes[4..].try_into().expect("four bytes")),
    ])
}

fn vtable(base: usize, rva: Rva) -> Result<usize, &'static str> {
    engine_sites::at(base, rva).ok_or("module address overflow")
}

fn push<T: Default>(
    output: &mut Buffer<'_, T>,
    len: &mut usize,
    unread: &mut u32,
    row: T,
) -> Result<(), &'static str> {
    let required = len
        .checked_add(1)
        .ok_or("resources map row count overflow")?;
    if output.ensure(required)? {
        output[*len] = row;
        *len = required;
    } else {
        *unread = unread
            .checked_add(1)
            .ok_or("resources map unread row count overflow")?;
    }
    Ok(())
}

fn outcome(len: usize, unread_rows: u32) -> Outcome {
    if unread_rows == 0 {
        Outcome::Rows { len }
    } else {
        Outcome::Truncated { len, unread_rows }
    }
}

pub fn bad_efficiency_threshold(memory: &impl Memory, base: usize) -> Result<i64, &'static str> {
    read::<i64>(
        memory,
        engine_sites::at(base, BAD_EFFICIENCY_THRESHOLD_RVA)
            .ok_or("resources map address overflow")?,
    )
    .ok_or("bad-efficiency threshold unreadable")
}

pub fn efficiency_product(a: i64, b: i64) -> i128 {
    i128::from(a) * i128::from(b) / ONE
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ClientStatus {
    bad: bool,
    convoys: [i32; 2],
    efficiency: [i64; 2],
    convoys_base: i32,
}

impl ClientStatus {
    fn new(efficiency: [i64; 2], convoys: [i32; 2], convoys_base: i32, threshold: i64) -> Self {
        Self {
            bad: efficiency_product(efficiency[0], efficiency[1]) < i128::from(threshold),
            convoys,
            efficiency,
            convoys_base,
        }
    }
}

fn client_efficiency(memory: &impl Memory, client: usize) -> Result<[i64; 2], &'static str> {
    pair_i64(memory, at(client, CLIENT_EFFICIENCY_A)?).ok_or("convoy client efficiency unreadable")
}

pub fn client_bad(
    memory: &impl Memory,
    client: usize,
    threshold: i64,
) -> Result<bool, &'static str> {
    let factors = client_efficiency(memory, client)?;
    Ok(efficiency_product(factors[0], factors[1]) < i128::from(threshold))
}

fn client_convoys(memory: &impl Memory, client: usize) -> Result<[i32; 2], &'static str> {
    let [assigned, required] =
        pair_u32(memory, at(client, CLIENT_CONVOYS)?).ok_or("convoy client convoys unreadable")?;
    Ok([assigned as i32, required as i32])
}

fn client_convoys_base(memory: &impl Memory, client: usize) -> Result<i32, &'static str> {
    read::<i32>(memory, at(client, CLIENT_CONVOYS_BASE)?)
        .ok_or("convoy client base convoys unreadable")
}

fn client_status(
    memory: &impl Memory,
    client: usize,
    threshold: i64,
) -> Result<ClientStatus, &'static str> {
    let spans = [
        (CLIENT_EFFICIENCY_A, 16),
        (CLIENT_CONVOYS, 8),
        (CLIENT_CONVOYS_BASE, 4),
    ]
    .map(|(offset, len)| {
        client
            .checked_add(offset)
            .filter(|at| at.checked_add(len).is_some())
    });
    let [Some(factors), Some(convoys), Some(base)] = spans else {
        let efficiency = client_efficiency(memory, client)?;
        let convoys = client_convoys(memory, client)?;
        let base = client_convoys_base(memory, client)?;
        return Ok(ClientStatus::new(efficiency, convoys, base, threshold));
    };
    let mut factor_bytes = [0; 16];
    let mut convoy_bytes = [0; 8];
    let mut base_bytes = [0; 4];
    match memory.copy_scattered(
        [factors, convoys, base],
        &mut [
            factor_bytes.as_mut_slice(),
            convoy_bytes.as_mut_slice(),
            base_bytes.as_mut_slice(),
        ],
    ) {
        0 => return Err("convoy client efficiency unreadable"),
        1 => return Err("convoy client convoys unreadable"),
        2 => return Err("convoy client base convoys unreadable"),
        _ => {}
    }
    let a = i64::from_le_bytes(factor_bytes[..8].try_into().expect("eight bytes"));
    let b = i64::from_le_bytes(factor_bytes[8..].try_into().expect("eight bytes"));
    Ok(ClientStatus::new(
        [a, b],
        [
            i32::from_le_bytes(convoy_bytes[..4].try_into().expect("four bytes")),
            i32::from_le_bytes(convoy_bytes[4..].try_into().expect("four bytes")),
        ],
        i32::from_le_bytes(base_bytes),
        threshold,
    ))
}

fn origin_factors(memory: &impl Memory, origin: usize) -> Result<(i64, i64), &'static str> {
    let addresses = [ORIGIN_EFFICIENCY, ORIGIN_ROUTE_FACTOR].map(|offset| {
        origin
            .checked_add(offset)
            .filter(|at| at.checked_add(8).is_some())
    });
    let [Some(efficiency), Some(route_factor)] = addresses else {
        let efficiency =
            read(memory, at(origin, ORIGIN_EFFICIENCY)?).ok_or("origin efficiency unreadable")?;
        let route_factor = read(memory, at(origin, ORIGIN_ROUTE_FACTOR)?)
            .ok_or("origin route factor unreadable")?;
        return Ok((efficiency, route_factor));
    };
    let mut words = [[0; 8]; 2];
    match memory.copy_scattered(
        [efficiency, route_factor],
        &mut words.each_mut().map(|word| word.as_mut_slice()),
    ) {
        0 => Err("origin efficiency unreadable"),
        1 => Err("origin route factor unreadable"),
        _ => Ok((i64::from_le_bytes(words[0]), i64::from_le_bytes(words[1]))),
    }
}

fn client_handle(memory: &impl Memory, client: usize) -> Result<u32, &'static str> {
    read(memory, at(client, CLIENT_HANDLE_ID)?).ok_or("convoy client handle unreadable")
}

fn delivery_route(
    memory: &impl Memory,
    exporter_resources: usize,
    importer_slot: u32,
) -> Result<usize, &'static str> {
    let routes = array(
        memory,
        at(exporter_resources, DELIVERY_ROUTES)?,
        8,
        "delivery routes unreadable",
    )?;
    let route = element(
        memory,
        routes,
        importer_slot,
        8,
        "importer outside the exporter's delivery routes",
    )?;
    if route == 0 {
        return Err("null delivery route");
    }
    Ok(route)
}

struct Walk<'a, M: Memory> {
    memory: &'a M,
    base: usize,
    game_state: usize,
    countries: &'a ccountry::Reader<'a, M>,
    threshold: i64,
    local_resources: std::cell::Cell<Option<modifier_definition::Operands>>,
    links: Result<Option<Array>, &'static str>,
}

struct OriginBatch {
    origin_ptrs: [usize; ORIGIN_BATCH],
    origin_bytes: [u8; ORIGIN_BATCH * ORIGIN_RECORD_STRIDE],
    state_ptrs: [usize; ORIGIN_BATCH],
    state_bytes: [u8; ORIGIN_BATCH * STATE_RECORD_STRIDE],
}

impl Default for OriginBatch {
    fn default() -> Self {
        Self {
            origin_ptrs: [0; ORIGIN_BATCH],
            origin_bytes: [0; ORIGIN_BATCH * ORIGIN_RECORD_STRIDE],
            state_ptrs: [0; ORIGIN_BATCH],
            state_bytes: [0; ORIGIN_BATCH * STATE_RECORD_STRIDE],
        }
    }
}

impl OriginBatch {
    fn origin_record(&self, index: usize) -> &[u8] {
        &self.origin_bytes[index * ORIGIN_RECORD_STRIDE..(index + 1) * ORIGIN_RECORD_STRIDE]
    }

    fn state_record(&self, index: usize) -> &[u8] {
        &self.state_bytes[index * STATE_RECORD_STRIDE..(index + 1) * STATE_RECORD_STRIDE]
    }
}

const ORIGIN_BATCH: usize = 64;
const ORIGIN_PTR_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: 8,
}];
const ORIGIN_PTR_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&ORIGIN_PTR_FIELDS, 8).expect("valid origin pointer layout");
const ORIGIN_RECORD_STRIDE: usize = 84;
const ORIGIN_RECORD_FIELDS: [RecordField; 9] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: CLIENT_CONVOYS,
        destination_offset: 8,
        len: 8,
    },
    RecordField {
        source_offset: CLIENT_EFFICIENCY_A,
        destination_offset: 16,
        len: 16,
    },
    RecordField {
        source_offset: CLIENT_CONVOYS_BASE,
        destination_offset: 32,
        len: 4,
    },
    RecordField {
        source_offset: ORIGIN_STATE,
        destination_offset: 36,
        len: 8,
    },
    RecordField {
        source_offset: ORIGIN_EFFICIENCY,
        destination_offset: 44,
        len: 8,
    },
    RecordField {
        source_offset: ORIGIN_ROUTE,
        destination_offset: 52,
        len: 8,
    },
    RecordField {
        source_offset: ORIGIN_ROUTE_FACTOR,
        destination_offset: 60,
        len: 8,
    },
    RecordField {
        source_offset: ORIGIN_RIGHTS,
        destination_offset: 68,
        len: 16,
    },
];
const ORIGIN_RECORD_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&ORIGIN_RECORD_FIELDS, ORIGIN_RECORD_STRIDE)
        .expect("valid origin record layout");
const STATE_RECORD_STRIDE: usize = 24;
const STATE_RECORD_FIELDS: [RecordField; 2] = [
    RecordField {
        source_offset: cstate::ID,
        destination_offset: 0,
        len: 4,
    },
    RecordField {
        source_offset: cstate::COUNTRY_MODIFIERS,
        destination_offset: 4,
        len: 17,
    },
];
const STATE_RECORD_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&STATE_RECORD_FIELDS, STATE_RECORD_STRIDE)
        .expect("valid origin state record layout");

impl<M: Memory> Walk<'_, M> {
    fn id(&self, link: u32) -> Result<u32, &'static str> {
        Ok(self.countries.resolve(link)?.id)
    }

    fn country_resources(&self, country: engine_native_memory::country_identity::Country) -> Result<usize, &'static str> {
        crate::ccountry_resources::resources(self.memory, self.base, self.countries, country)
    }

    fn exchange(
        &self,
        client: usize,
        importer: engine_native_memory::country_identity::Country,
    ) -> Result<(usize, u32), &'static str> {
        let (_, ends) = cresource_exchange::authenticate(
            self.memory,
            self.base,
            self.countries,
            client,
            importer,
        )?;
        let route = delivery_route(
            self.memory,
            ends.exporter_resources,
            importer.canonical_slot,
        )?;
        Ok((route, ends.exporter_country.id))
    }

    fn held_route(&self, client: usize, offset: usize) -> Result<usize, &'static str> {
        let route = pointer(self.memory, at(client, offset)?, "client route unreadable")?;
        if route == 0 {
            return Err("client holds no route");
        }
        self.authenticate_route(route)?;
        Ok(route)
    }

    fn authenticate_route(&self, route: usize) -> Result<(), &'static str> {
        if pointer(self.memory, route, "route vtable unreadable")?
            != vtable(self.base, ROUTE_VTABLE_RVA)?
        {
            return Err("route vtable mismatch");
        }
        Ok(())
    }

    fn client(
        &self,
        client: usize,
        expected: Rva,
        category: u8,
        owner: u32,
        other: u32,
        route: (RouteKey, usize),
    ) -> Result<ClientRow, &'static str> {
        if pointer(self.memory, client, "convoy client vtable unreadable")?
            != vtable(self.base, expected)?
        {
            return Err("convoy client of an unexpected type");
        }
        let handle = client_handle(self.memory, client)?;
        let status = client_status(self.memory, client, self.threshold)?;
        Ok(ClientRow {
            owner_ordinal: None,
            other_ordinal: None,
            address: client,
            handle,
            category,
            owner,
            other,
            route: route.0,
            bad: status.bad,
            convoys: status.convoys,
            efficiency: status.efficiency,
            convoys_base: status.convoys_base,
            route_address: route.1,
        })
    }

    fn origin(
        &self,
        origin: usize,
        list: u8,
        ordinal: u32,
        country: engine_native_memory::country_identity::Country,
    ) -> Result<(OriginRow, usize), &'static str> {
        let memory = self.memory;
        if pointer(memory, origin, "origin vtable unreadable")?
            != vtable(self.base, ORIGIN_VTABLE_RVA)?
        {
            return Err("origin vtable mismatch");
        }
        let state_pointer = pointer(memory, at(origin, ORIGIN_STATE)?, "origin state unreadable")?;
        if state_pointer == 0 {
            return Err("origin without a state");
        }
        let state = read::<u32>(memory, at(state_pointer, cstate::ID)?)
            .ok_or("origin state id unreadable")?;
        let (efficiency, route_factor) = origin_factors(memory, origin)?;
        let accumulator = cstate::country_modifier_accumulator(
            memory,
            self.game_state,
            state_pointer,
            country.id,
        )?;
        let local_resources_factor = effective(
            memory,
            self.base,
            accumulator,
            LOCAL_RESOURCES_FACTOR,
            &self.local_resources,
        )?;
        let route_address = at(origin, ORIGIN_ROUTE)?;
        self.authenticate_route(route_address)?;
        let status = client_status(memory, origin, self.threshold)?;
        Ok((
            OriginRow {
                state,
                country: country.id,
                list,
                ordinal,
                efficiency,
                route_factor,
                local_resources_factor,
                bad: status.bad,
                convoys: status.convoys,
                convoys_base: status.convoys_base,
                route_address,
            },
            state_pointer,
        ))
    }
}

impl<M: Memory> Walk<'_, M> {
    fn rights(
        &self,
        origin: usize,
        state: u32,
        table: &mut Table,
        output: &mut Buffer<'_, RightsRow>,
    ) -> Result<(), &'static str> {
        let rights = array(
            self.memory,
            at(origin, ORIGIN_RIGHTS)?,
            RIGHTS_STRIDE,
            "origin rights unreadable",
        )?;
        self.rights_from_array(rights, state, table, output)
    }

    fn rights_from_array(
        &self,
        rights: Array,
        state: u32,
        table: &mut Table,
        output: &mut Buffer<'_, RightsRow>,
    ) -> Result<(), &'static str> {
        for entry in 0..rights.count {
            let address = rights
                .element(entry, RIGHTS_STRIDE)
                .ok_or("origin rights element unreadable")?;
            let [resource, holder] =
                pair_u32(self.memory, address).ok_or("origin rights entry unreadable")?;
            let holder = self.id(holder)?;
            table.push(
                output,
                RightsRow {
                    ordinal: entry,
                    state,
                    resource,
                    holder,
                },
            )?;
        }
        Ok(())
    }

    fn try_read_origin_batch(
        &self,
        origins: Array,
        stride: usize,
        first: u32,
        count: usize,
        batch: &mut OriginBatch,
    ) -> bool {
        let Ok(expected_vtable) = vtable(self.base, ORIGIN_VTABLE_RVA) else {
            return false;
        };
        let Some(start) = origins.element(first, stride) else {
            return false;
        };
        let mut ptr_bytes = [0_u8; ORIGIN_BATCH * 8];
        if self.memory.copy_records(
            RecordSources::Strided {
                base: start,
                stride,
                count,
            },
            &ORIGIN_PTR_LAYOUT,
            &mut ptr_bytes[..count * 8],
        ) != count
        {
            return false;
        }
        for (slot, chunk) in batch.origin_ptrs[..count]
            .iter_mut()
            .zip(ptr_bytes[..count * 8].chunks_exact(8))
        {
            let Ok(origin) = usize::try_from(u64::from_le_bytes(chunk.try_into().unwrap())) else {
                return false;
            };
            if origin == 0 || origin.checked_add(ORIGIN_RIGHTS + 16).is_none() {
                return false;
            }
            *slot = origin;
        }
        if self.memory.copy_records(
            RecordSources::Pointers(&batch.origin_ptrs[..count]),
            &ORIGIN_RECORD_LAYOUT,
            &mut batch.origin_bytes[..count * ORIGIN_RECORD_STRIDE],
        ) != count * ORIGIN_RECORD_FIELDS.len()
        {
            return false;
        }
        for (slot, record) in batch.state_ptrs[..count].iter_mut().zip(
            batch.origin_bytes[..count * ORIGIN_RECORD_STRIDE].chunks_exact(ORIGIN_RECORD_STRIDE),
        ) {
            if u64::from_le_bytes(record[0..8].try_into().unwrap()) as usize != expected_vtable {
                return false;
            }
            let Ok(state) = usize::try_from(u64::from_le_bytes(record[36..44].try_into().unwrap()))
            else {
                return false;
            };
            if state == 0
                || state.checked_add(cstate::ID + 4).is_none()
                || state.checked_add(cstate::COUNTRY_MODIFIERS + 17).is_none()
            {
                return false;
            }
            *slot = state;
        }
        self.memory.copy_records(
            RecordSources::Pointers(&batch.state_ptrs[..count]),
            &STATE_RECORD_LAYOUT,
            &mut batch.state_bytes[..count * STATE_RECORD_STRIDE],
        ) == count * STATE_RECORD_FIELDS.len()
    }

    fn country_modifier_accumulator_from_header(
        &self,
        state_pointer: usize,
        country_id: u32,
        header: [u8; 17],
        cached_owner: &mut Option<Result<u32, &'static str>>,
    ) -> Result<usize, &'static str> {
        if self.game_state == 0 || country_id == 0 {
            return cstate::country_modifier_accumulator_with_canonical(
                self.memory,
                state_pointer,
                country_id,
                header,
                Ok,
            );
        }
        let links = self.links?.ok_or("state movement country links invalid")?;
        let owner_canonical = *cached_owner.get_or_insert_with(|| {
            links
                .element(country_id, 4)
                .and_then(|address| read(self.memory, address))
                .ok_or("state movement country link unreadable")
        });
        cstate::country_modifier_accumulator_with_canonical(
            self.memory,
            state_pointer,
            country_id,
            header,
            |raw| {
                if raw == country_id {
                    owner_canonical
                } else {
                    links
                        .element(raw, 4)
                        .and_then(|address| read(self.memory, address))
                        .ok_or("state movement country link unreadable")
                }
            },
        )
    }

    fn origin_from_batch(
        &self,
        batch: &OriginBatch,
        index: usize,
        list: u8,
        ordinal: u32,
        country: ccountry::Country,
        cached_owner: &mut Option<Result<u32, &'static str>>,
    ) -> Result<OriginRow, &'static str> {
        let origin = batch.origin_ptrs[index];
        let record = batch.origin_record(index);
        let state_pointer = batch.state_ptrs[index];
        let state_record = batch.state_record(index);
        let state = u32::from_le_bytes(state_record[0..4].try_into().unwrap());
        let modifiers: [u8; 17] = state_record[4..21].try_into().unwrap();
        let efficiency = i64::from_le_bytes(record[44..52].try_into().unwrap());
        let route_factor = i64::from_le_bytes(record[60..68].try_into().unwrap());
        let accumulator = self.country_modifier_accumulator_from_header(
            state_pointer,
            country.id,
            modifiers,
            cached_owner,
        )?;
        let local_resources_factor = effective(
            self.memory,
            self.base,
            accumulator,
            LOCAL_RESOURCES_FACTOR,
            &self.local_resources,
        )?;
        let route_address = at(origin, ORIGIN_ROUTE)?;
        if u64::from_le_bytes(record[52..60].try_into().unwrap()) as usize
            != vtable(self.base, ROUTE_VTABLE_RVA)?
        {
            return Err("route vtable mismatch");
        }
        let status = ClientStatus::new(
            [
                i64::from_le_bytes(record[16..24].try_into().unwrap()),
                i64::from_le_bytes(record[24..32].try_into().unwrap()),
            ],
            [
                i32::from_le_bytes(record[8..12].try_into().unwrap()),
                i32::from_le_bytes(record[12..16].try_into().unwrap()),
            ],
            i32::from_le_bytes(record[32..36].try_into().unwrap()),
            self.threshold,
        );
        Ok(OriginRow {
            state,
            country: country.id,
            list,
            ordinal,
            efficiency,
            route_factor,
            local_resources_factor,
            bad: status.bad,
            convoys: status.convoys,
            convoys_base: status.convoys_base,
            route_address,
        })
    }

    fn country_clients(
        &self,
        country: engine_native_memory::country_identity::Country,
        resources: usize,
        supply_systems: usize,
        table: &mut Table,
        output: &mut Buffer<'_, ClientRow>,
    ) -> Result<(), &'static str> {
        let memory = self.memory;
        let imports = array(
            memory,
            at(resources, ccountry_resources::IMPORT_ROUTES)?,
            8,
            "import list unreadable",
        )?;
        for index in 0..imports.count {
            let client = element(memory, imports, index, 8, "import exchange unreadable")?;
            if client == 0 {
                return Err("null exchange in a live import list");
            }
            let (route, exporter) = self.exchange(client, country)?;
            let key = RouteKey {
                class: DELIVERY_ROUTE_CLASS,
                country: exporter,
                other: country.id,
            };
            let row = self.client(
                client,
                EXCHANGE_VTABLE_RVA,
                IMPORT,
                country.id,
                exporter,
                (key, route),
            )?;
            table.push(output, row)?;
        }
        let given = array(
            memory,
            at(resources, LEND_LEASE_GIVEN)?,
            8,
            "lend-lease list unreadable",
        )?;
        for index in 0..given.count {
            let client = element(memory, given, index, 8, "lend-lease exchange unreadable")?;
            if client == 0 {
                return Err("null lend-lease exchange in a live list");
            }
            let receiver = read::<u32>(memory, at(client, LEND_LEASE_RECEIVER)?)
                .ok_or("lend-lease receiver unreadable")?;
            let receiver = self.id(receiver)?;
            let route = self.held_route(client, LEND_LEASE_ROUTE)?;
            let key = RouteKey {
                class: DELIVERY_ROUTE_CLASS,
                country: country.id,
                other: receiver,
            };
            let mut row = self.client(
                client,
                LEND_LEASE_VTABLE_RVA,
                LEND_LEASE,
                country.id,
                receiver,
                (key, route),
            )?;
            row.owner_ordinal = Some(index);
            table.push(output, row)?;
        }
        let system = pointer(
            memory,
            at(supply_systems, 8 * country.canonical_slot as usize)?,
            "country supply system unreadable",
        )?;
        if system == 0 {
            return Ok(());
        }
        let transfers = array(
            memory,
            at(system, SUPPLY_TRANSFERS)?,
            SUPPLY_TRANSFER_STRIDE,
            "supply transfers unreadable",
        )?;
        for index in 0..transfers.count {
            let entry = transfers
                .element(index, SUPPLY_TRANSFER_STRIDE)
                .ok_or("supply transfer unreadable")?;
            let client = pointer(
                memory,
                at(entry, SUPPLY_TRANSFER_CLIENT)?,
                "supply transfer client unreadable",
            )?;
            if client == 0 {
                return Err("supply transfer without a client");
            }
            let route = at(client, SUPPLY_ROUTE)?;
            self.authenticate_route(route)?;
            let handle = client_handle(memory, client)?;
            let key = RouteKey {
                class: SUPPLY_ROUTE_CLASS,
                country: country.id,
                other: handle,
            };
            let mut row = self.client(
                client,
                SUPPLY_VTABLE_RVA,
                SUPPLY_TRANSFER,
                country.id,
                0,
                (key, route),
            )?;
            row.owner_ordinal = Some(index);
            table.push(output, row)?;
        }
        Ok(())
    }

    fn directional_orders(&self, clients: &mut [ClientRow]) -> Result<(), &'static str> {
        let mut handles = Vec::new();
        allocation::reserve(&mut handles, clients.len())
            .map_err(|_| "convoy order index allocation failed")?;
        let mut endpoints = Vec::new();
        allocation::reserve(&mut endpoints, clients.len().saturating_mul(2))
            .map_err(|_| "convoy order endpoint allocation failed")?;
        for (index, row) in clients.iter().enumerate() {
            handles.push((row.handle, index));
            match row.category {
                LEND_LEASE => endpoints.push((row.other, LEND_LEASE, false)),
                MARKET => {
                    endpoints.push((row.owner, MARKET, true));
                    endpoints.push((row.other, MARKET, false));
                }
                _ => {}
            }
        }
        handles.sort_unstable();
        if handles.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return Err("duplicate convoy client handle in native order index");
        }
        endpoints.sort_unstable();
        endpoints.dedup();
        for (country, category, owner_side) in endpoints {
            let resolved = self.countries.resolve(country)?;
            let list = if category == LEND_LEASE {
                let resources = self.country_resources(resolved)?;
                array(
                    self.memory,
                    at(resources, LEND_LEASE_RECEIVED)?,
                    8,
                    "received lend-lease order list unreadable",
                )?
            } else {
                let manager = pointer(
                    self.memory,
                    at(self.game_state, cgame_state::MARKET_MANAGER)?,
                    "market order manager unreadable",
                )?;
                let status = pointer(
                    self.memory,
                    at(resolved.pointer, ccountry::EQUIPMENT_MARKET_STATUS)?,
                    "country market order status unreadable",
                )?;
                if status == 0
                    || pointer(
                        self.memory,
                        at(status, MARKET_STATUS_MANAGER)?,
                        "country market order backlink unreadable",
                    )? != manager
                {
                    return Err("country market order manager mismatch");
                }
                let outer = array(
                    self.memory,
                    at(
                        manager,
                        if owner_side {
                            MARKET_SELLERS
                        } else {
                            MARKET_BUYERS
                        },
                    )?,
                    MARKET_COUNTRY_STRIDE,
                    "market directional order table unreadable",
                )?;
                let header = outer
                    .element(resolved.canonical_slot, MARKET_COUNTRY_STRIDE)
                    .ok_or("market country order list missing")?;
                array(
                    self.memory,
                    header,
                    8,
                    "market country order list unreadable",
                )?
            };
            for ordinal in 0..list.count {
                let entry = element(
                    self.memory,
                    list,
                    ordinal,
                    8,
                    "convoy order entry unreadable",
                )?;
                if entry == 0 {
                    return Err("null convoy order entry");
                }
                let address = if category == MARKET {
                    at(entry, CONTRACT_CLIENT)?
                } else {
                    entry
                };
                let handle = client_handle(self.memory, address)?;
                let Ok(at) = handles.binary_search_by_key(&handle, |&(handle, _)| handle) else {
                    continue;
                };
                let row = &mut clients[handles[at].1];
                if row.category != category
                    || row.address != address
                    || (if owner_side { row.owner } else { row.other }) != country
                {
                    return Err("convoy directional order membership mismatch");
                }
                let position = if owner_side {
                    &mut row.owner_ordinal
                } else {
                    &mut row.other_ordinal
                };
                if position.replace(ordinal).is_some() {
                    return Err("duplicate convoy directional order membership");
                }
            }
        }
        if clients.iter().any(|row| match row.category {
            LEND_LEASE => row.other_ordinal.is_none(),
            MARKET => row.owner_ordinal.is_none() || row.other_ordinal.is_none(),
            _ => false,
        }) {
            return Err("convoy directional native list position unavailable");
        }
        Ok(())
    }

    fn market(
        &self,
        table: &mut Table,
        output: &mut Buffer<'_, ClientRow>,
    ) -> Result<(), &'static str> {
        let memory = self.memory;
        let manager = pointer(
            memory,
            at(self.game_state, cgame_state::MARKET_MANAGER)?,
            "market manager unreadable",
        )?;
        if manager == 0 {
            return Ok(());
        }
        let contracts = array(
            memory,
            at(manager, MARKET_OWNED_CONTRACTS)?,
            8,
            "market contracts unreadable",
        )?;
        for index in 0..contracts.count {
            let contract = element(memory, contracts, index, 8, "market contract unreadable")?;
            if contract == 0 {
                return Err("null market contract in the owned list");
            }
            let [seller, buyer] = pair_u32(memory, at(contract, CONTRACT_SELLER)?)
                .ok_or("market contract parties unreadable")?;
            let (seller, buyer) = (self.id(seller)?, self.id(buyer)?);
            let client = at(contract, CONTRACT_CLIENT)?;
            let route = self.held_route(client, CONTRACT_CLIENT_ROUTE)?;
            let key = RouteKey {
                class: DELIVERY_ROUTE_CLASS,
                country: seller,
                other: buyer,
            };
            let row = self.client(
                client,
                CONTRACT_CLIENT_VTABLE_RVA,
                MARKET,
                seller,
                buyer,
                (key, route),
            )?;
            table.push(output, row)?;
        }
        Ok(())
    }
}

const ORIGINS: usize = 0;
const RIGHTS: usize = 1;
const OIL: usize = 2;
const CLIENTS: usize = 3;

#[derive(Default)]
struct Table {
    len: usize,
    unread: u32,
    failed: Option<&'static str>,
}

impl Table {
    fn outcome(&self) -> Outcome {
        match self.failed {
            Some(reason) => Outcome::Unavailable { reason },
            None => outcome(self.len, self.unread),
        }
    }

    fn run(&mut self, read: impl FnOnce(&mut Self) -> Result<(), &'static str>) {
        if self.failed.is_none()
            && let Err(reason) = read(self)
        {
            self.failed = Some(reason);
        }
    }

    fn push<T: Default>(&mut self, output: &mut Buffer<'_, T>, row: T) -> Result<(), &'static str> {
        push(output, &mut self.len, &mut self.unread, row)
    }
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut outputs: Outputs<'_>,
) -> Outcomes {
    let mut tables: [Table; 4] = Default::default();
    let outcomes = |tables: &[Table; 4]| Outcomes {
        origins: tables[ORIGINS].outcome(),
        rights: tables[RIGHTS].outcome(),
        oil: tables[OIL].outcome(),
        clients: tables[CLIENTS].outcome(),
    };
    let fail_all = |tables: &mut [Table; 4], reason| {
        for table in tables.iter_mut() {
            table.failed.get_or_insert(reason);
        }
    };
    let countries = match ccountry::Reader::new(memory, base, game_state) {
        Ok(countries) => countries,
        Err(reason) => {
            fail_all(&mut tables, reason);
            return outcomes(&tables);
        }
    };
    let threshold = bad_efficiency_threshold(memory, base);
    if let Err(reason) = threshold {
        tables[ORIGINS].failed = Some(reason);
        tables[CLIENTS].failed = Some(reason);
    }
    let links = if game_state == 0 {
        Ok(None)
    } else {
        game_state
            .checked_add(cgame_state::COUNTRY_LINKS)
            .and_then(|address| Array::read(memory, address, 4))
            .map(Some)
            .ok_or("state movement country links invalid")
    };
    let walk = Walk {
        memory,
        base,
        game_state,
        countries: &countries,
        threshold: threshold.unwrap_or_default(),
        local_resources: Default::default(),
        links,
    };
    let oil_metadata = Default::default();
    let supply_systems = (|| -> Result<usize, &'static str> {
        let supply = pointer(
            memory,
            at(game_state, cgame_state::SUPPLY_SYSTEM)?,
            "supply system unreadable",
        )?;
        if supply == 0 {
            return Err("no supply system");
        }
        pointer(
            memory,
            at(supply, SUPPLY_COUNTRY_SYSTEMS)?,
            "country supply systems unreadable",
        )
    })();
    if let Err(reason) = supply_systems {
        tables[CLIENTS].failed.get_or_insert(reason);
    }
    let mut found_country = false;
    let mut batch = OriginBatch::default();
    for slot in 1..countries.count() {
        let country = match countries.at(slot) {
            Ok(Some(country)) => country,
            Ok(None) => continue,
            Err(reason) => {
                fail_all(&mut tables, reason);
                break;
            }
        };
        found_country = true;
        tables[OIL].run(|table| {
            let oil = effective(
                memory,
                base,
                at(country.pointer, engine_native_memory::country_identity::MODIFIER_ACCUMULATOR)?,
                PRODUCTION_OIL_FACTOR,
                &oil_metadata,
            )?;
            if oil != 0 {
                table.push(
                    &mut outputs.oil,
                    OilRow {
                        country: country.id,
                        factor: oil,
                    },
                )?;
            }
            Ok(())
        });
        if tables[ORIGINS].failed.is_some()
            && tables[RIGHTS].failed.is_some()
            && tables[CLIENTS].failed.is_some()
        {
            continue;
        }
        let resources = match walk.country_resources(country) {
            Ok(resources) => resources,
            Err(reason) => {
                for index in [ORIGINS, RIGHTS, CLIENTS] {
                    tables[index].failed.get_or_insert(reason);
                }
                continue;
            }
        };
        let [origins_table, rights_table, _, clients_table] = &mut tables;
        let mut owner_canonical = None;
        if rights_table.failed.is_none() || origins_table.failed.is_none() {
            match at(resources, ccountry_resources::RESOURCE_ORIGINS)
                .and_then(|address| array(memory, address, 8, "origin list unreadable"))
            {
                Ok(origins) => {
                    let mut first = 0_u32;
                    while first < origins.count {
                        let count = ((origins.count - first) as usize).min(ORIGIN_BATCH);
                        if walk.try_read_origin_batch(origins, 8, first, count, &mut batch) {
                            rights_table.run(|table| {
                                for index in 0..count {
                                    let record = batch.origin_record(index);
                                    let state_record = batch.state_record(index);
                                    let state = u32::from_le_bytes(
                                        state_record[0..4].try_into().unwrap(),
                                    );
                                    let rights = Array::from_header(
                                        record[68..84].try_into().unwrap(),
                                        RIGHTS_STRIDE,
                                    )
                                    .ok_or("origin rights unreadable")?;
                                    walk.rights_from_array(
                                        rights,
                                        state,
                                        table,
                                        &mut outputs.rights,
                                    )?;
                                }
                                Ok(())
                            });
                            origins_table.run(|table| {
                                for index in 0..count {
                                    let row = walk.origin_from_batch(
                                        &batch,
                                        index,
                                        0,
                                        first + index as u32,
                                        country,
                                        &mut owner_canonical,
                                    )?;
                                    table.push(&mut outputs.origins, row)?;
                                }
                                Ok(())
                            });
                            first += count as u32;
                        } else {
                            rights_table.run(|table| {
                                for index in first..origins.count {
                                    let origin =
                                        element(memory, origins, index, 8, "origin unreadable")?;
                                    if origin == 0 {
                                        return Err("null origin in a live list");
                                    }
                                    if pointer(memory, origin, "origin vtable unreadable")?
                                        != vtable(base, ORIGIN_VTABLE_RVA)?
                                    {
                                        return Err("origin vtable mismatch");
                                    }
                                    let state = pointer(
                                        memory,
                                        at(origin, ORIGIN_STATE)?,
                                        "origin state unreadable",
                                    )?;
                                    if state == 0 {
                                        return Err("origin without a state");
                                    }
                                    let state = read::<u32>(memory, at(state, cstate::ID)?)
                                        .ok_or("origin state id unreadable")?;
                                    walk.rights(origin, state, table, &mut outputs.rights)?;
                                }
                                Ok(())
                            });
                            origins_table.run(|table| {
                                for index in first..origins.count {
                                    let origin =
                                        element(memory, origins, index, 8, "origin unreadable")?;
                                    if origin == 0 {
                                        return Err("null origin in a live list");
                                    }
                                    let (row, _) = walk.origin(origin, 0, index, country)?;
                                    table.push(&mut outputs.origins, row)?;
                                }
                                Ok(())
                            });
                            break;
                        }
                    }
                }
                Err(reason) => {
                    rights_table.run(|_| Err(reason));
                    origins_table.run(|_| Err(reason));
                }
            }
        }
        origins_table.run(|table| {
            let origins = array(
                memory,
                at(resources, RIGHTS_ORIGINS)?,
                RIGHTS_ORIGIN_STRIDE,
                "origin list unreadable",
            )?;
            let mut first = 0_u32;
            while first < origins.count {
                let count = ((origins.count - first) as usize).min(ORIGIN_BATCH);
                if walk.try_read_origin_batch(
                    origins,
                    RIGHTS_ORIGIN_STRIDE,
                    first,
                    count,
                    &mut batch,
                ) {
                    for index in 0..count {
                        let row = walk.origin_from_batch(
                            &batch,
                            index,
                            1,
                            first + index as u32,
                            country,
                            &mut owner_canonical,
                        )?;
                        table.push(&mut outputs.origins, row)?;
                    }
                    first += count as u32;
                } else {
                    for index in first..origins.count {
                        let origin = element(
                            memory,
                            origins,
                            index,
                            RIGHTS_ORIGIN_STRIDE,
                            "origin unreadable",
                        )?;
                        if origin == 0 {
                            return Err("null origin in a live list");
                        }
                        let (row, _) = walk.origin(origin, 1, index, country)?;
                        table.push(&mut outputs.origins, row)?;
                    }
                    break;
                }
            }
            Ok(())
        });
        clients_table.run(|table| {
            walk.country_clients(
                country,
                resources,
                supply_systems?,
                table,
                &mut outputs.clients,
            )
        });
    }
    if !found_country {
        fail_all(&mut tables, "country table has no readable countries");
    }
    tables[CLIENTS].run(|table| walk.market(table, &mut outputs.clients));
    tables[CLIENTS].run(|table| walk.directional_orders(&mut outputs.clients[..table.len]));
    outcomes(&tables)
}

pub fn walk_regional(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    origins: &[OriginRow],
    clients: &[ClientRow],
    scratch: &mut Vec<(u32, u32)>,
    mut output: Buffer<'_, RegionalRow>,
) -> Outcome {
    let mut len = 0;
    let mut unread = 0;
    let result = (|| -> Result<(), &'static str> {
        scratch.clear();
        let mut path = Vec::new();
        let bad = origins
            .iter()
            .filter(|origin| origin.bad)
            .map(|origin| (origin.route_address, [origin.country, 0]))
            .chain(
                clients
                    .iter()
                    .filter(|client| client.bad)
                    .map(|client| (client.route_address, [client.owner, client.other])),
            );
        for (route, countries) in bad {
            path.clear();
            let row = read_route(memory, base, RouteKey::default(), route, None, &mut path)?;
            if row.kind != NAVAL {
                continue;
            }
            for country in countries.into_iter().filter(|&country| country != 0) {
                for &region in &path {
                    allocation::reserve(scratch, 1)
                        .map_err(|_| "regional pair allocation failed")?;
                    scratch.push((country, region));
                }
            }
        }
        scratch.sort_unstable();
        scratch.dedup();
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let mut start = 0;
        while start < scratch.len() {
            let country = scratch[start].0;
            let end = start + scratch[start..].partition_point(|&(id, _)| id == country);
            let resolved = countries.resolve(country)?;
            let navy = engine_native_naval::cstrategic_navy_manager::navy(memory, base, game_state, resolved)?;
            let records = engine_native_naval::cstrategic_navy::regional_convoys(memory, base, navy)?;
            for &(_, region) in &scratch[start..end] {
                let efficiency =
                    engine_native_naval::cstrategic_navy::regional_convoy_efficiency(memory, records, region)?;
                push(
                    &mut output,
                    &mut len,
                    &mut unread,
                    RegionalRow {
                        country,
                        region,
                        efficiency,
                    },
                )?;
            }
            start = end;
        }
        Ok(())
    })();
    match result {
        Ok(()) => outcome(len, unread),
        Err(reason) => Outcome::Unavailable { reason },
    }
}

pub fn walk_region_weights(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: Buffer<'_, RegionWeightRow>,
) -> Outcome {
    use engine_native_parts::cstrategic_region as region;
    const BATCH: usize = 64;
    const FIELDS: [RecordField; 3] = [
        RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 8,
        },
        RecordField {
            source_offset: region::ID,
            destination_offset: 8,
            len: 4,
        },
        RecordField {
            source_offset: region::ROUTE_WEIGHT,
            destination_offset: 12,
            len: 4,
        },
    ];
    const LAYOUT: RecordLayout<'static> =
        RecordLayout::new(&FIELDS, 16).expect("valid strategic region layout");
    let mut len = 0;
    let mut unread = 0;
    let result = (|| -> Result<(), &'static str> {
        let expected = vtable(base, region::VTABLE_RVA)? as u64;
        let roster = array(
            memory,
            at(game_state, cgame_state::STRATEGIC_REGIONS)?,
            8,
            "strategic region roster unreadable",
        )?;
        let mut ptr_bytes = [0_u8; BATCH * 8];
        let mut pointers = [0_usize; BATCH];
        let mut records = [0_u8; BATCH * 16];
        let mut first = 0_u32;
        while first < roster.count {
            let count = ((roster.count - first) as usize).min(BATCH);
            let start = roster
                .element(first, 8)
                .ok_or("strategic region roster slot unreadable")?;
            let batch_ptrs = engine_native::read_slice(memory, start, &mut ptr_bytes[..count * 8]);
            let mut live = 0_usize;
            for offset in 0..count {
                let pointer = if batch_ptrs {
                    usize::try_from(u64::from_le_bytes(
                        ptr_bytes[offset * 8..(offset + 1) * 8].try_into().unwrap(),
                    ))
                    .map_err(|_| "strategic region roster slot unreadable")?
                } else {
                    element(
                        memory,
                        roster,
                        first + offset as u32,
                        8,
                        "strategic region roster slot unreadable",
                    )?
                };
                if pointer == 0 {
                    continue;
                }
                at(pointer, region::ID)?;
                at(pointer, region::ROUTE_WEIGHT)?;
                pointers[live] = pointer;
                live += 1;
            }
            if live != 0 {
                let copied = memory.copy_records(
                    RecordSources::Pointers(&pointers[..live]),
                    &LAYOUT,
                    &mut records[..live * 16],
                );
                let complete = copied / FIELDS.len();
                for index in 0..live {
                    let (vtable_word, id_val, weight_val) = if index < complete {
                        let rec = &records[index * 16..(index + 1) * 16];
                        (
                            u64::from_le_bytes(rec[..8].try_into().unwrap()),
                            u32::from_le_bytes(rec[8..12].try_into().unwrap()),
                            i32::from_le_bytes(rec[12..16].try_into().unwrap()),
                        )
                    } else {
                        let pointer = pointers[index];
                        let mut identity = [0; 8];
                        let mut id = [0; 4];
                        let mut weight = [0; 4];
                        if memory.copy_scattered(
                            [
                                pointer,
                                at(pointer, region::ID)?,
                                at(pointer, region::ROUTE_WEIGHT)?,
                            ],
                            &mut [&mut identity, &mut id, &mut weight],
                        ) != 3
                        {
                            return Err("strategic region route weight unreadable");
                        }
                        (
                            u64::from_le_bytes(identity),
                            u32::from_le_bytes(id),
                            i32::from_le_bytes(weight),
                        )
                    };
                    if vtable_word != expected {
                        return Err("strategic region vtable mismatch");
                    }
                    push(
                        &mut output,
                        &mut len,
                        &mut unread,
                        RegionWeightRow {
                            region: id_val,
                            weight: weight_val,
                        },
                    )?;
                }
            }
            first += count as u32;
        }
        Ok(())
    })();
    match result {
        Ok(()) => outcome(len, unread),
        Err(reason) => Outcome::Unavailable { reason },
    }
}

pub fn walk_temporary(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    descriptors: &[cstrategic_resource::Row],
    mut output: Buffer<'_, TemporaryRow>,
) -> Outcome {
    const BATCH: usize = 64;
    const GATE_FIELDS: [RecordField; 2] = [
        RecordField {
            source_offset: STATE_TEMPORARY_LIST_COUNT,
            destination_offset: 0,
            len: 4,
        },
        RecordField {
            source_offset: STATE_TEMPORARY_FLAG,
            destination_offset: 4,
            len: 1,
        },
    ];
    const GATE_LAYOUT: RecordLayout<'static> =
        RecordLayout::new(&GATE_FIELDS, 8).expect("valid record layout");
    const WORD_FIELDS: [RecordField; 1] = [RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    }];
    const WORD_LAYOUT: RecordLayout<'static> =
        RecordLayout::new(&WORD_FIELDS, 8).expect("valid record layout");
    let mut len = 0;
    let mut unread = 0;
    let result = (|| -> Result<(), &'static str> {
        let states = array(
            memory,
            at(game_state, cgame_state::STATES)?,
            8,
            "state table unreadable",
        )?;
        let mut keys_loaded = false;
        let mut descriptor_keys = Vec::new();
        let mut sorted_keys = Vec::new();
        let mut sorted_values = Vec::new();
        let mut operands_cache: Vec<Option<modifier_definition::Operands>> = Vec::new();
        let mut gated = |state: usize, count: i32, flag: u8| -> Result<(), &'static str> {
            if count == 0 && flag == 0 {
                return Ok(());
            }
            let id = read::<u32>(memory, at(state, cstate::ID)?).ok_or("state id unreadable")?;
            let accumulator = at(state, cstate::BASE_MODIFIERS)?;
            if !keys_loaded {
                allocation::reserve(&mut descriptor_keys, descriptors.len())
                    .map_err(|_| "resource temporary key allocation failed")?;
                allocation::reserve(&mut sorted_keys, descriptors.len())
                    .map_err(|_| "resource temporary key allocation failed")?;
                for descriptor in descriptors {
                    let key = read::<u32>(
                        memory,
                        at(descriptor.address, cstrategic_resource::TEMPORARY_KEY)?,
                    )
                    .ok_or("resource temporary key unreadable")?;
                    descriptor_keys.push(key);
                    if key != 0 {
                        sorted_keys.push(key);
                    }
                }
                sorted_keys.sort_unstable();
                sorted_keys.dedup();
                allocation::reserve(&mut sorted_values, sorted_keys.len())
                    .map_err(|_| "resource temporary key allocation failed")?;
                sorted_values.resize(sorted_keys.len(), modifier_accumulator::Value::default());
                allocation::reserve(&mut operands_cache, sorted_keys.len())
                    .map_err(|_| "resource temporary key allocation failed")?;
                operands_cache.resize(sorted_keys.len(), None);
                keys_loaded = true;
            }
            if sorted_keys.is_empty() {
                return Ok(());
            }
            modifier_accumulator::lookup_many_at(
                memory,
                accumulator,
                &sorted_keys,
                &mut sorted_values,
            )?;
            for (index, descriptor) in descriptors.iter().enumerate() {
                let key = descriptor_keys[index];
                if key == 0 {
                    continue;
                }
                let slot = sorted_keys.binary_search(&key).unwrap();
                let value = sorted_values[slot];
                if !value.present {
                    continue;
                }
                let amount = if memory.modifier_definitions().is_some() {
                    modifier_definition::effective(memory, base, key, value.raw)?
                } else {
                    let operands = match operands_cache[slot] {
                        Some(operands) => operands,
                        None => {
                            let operands = modifier_definition::operands(memory, base, key)?;
                            operands_cache[slot] = Some(operands);
                            operands
                        }
                    };
                    operands.effective(value.raw)
                };
                if amount != 0 {
                    push(
                        &mut output,
                        &mut len,
                        &mut unread,
                        TemporaryRow {
                            state: id,
                            resource: descriptor.token,
                            amount,
                        },
                    )?;
                }
            }
            Ok(())
        };
        let mut first = 0;
        while first < states.count {
            let batch = ((states.count - first) as usize).min(BATCH);
            let mut words = [0_u8; BATCH * 8];
            let copied = states.element(first, 8).map_or(0, |start| {
                memory.copy_records(
                    RecordSources::Strided {
                        base: start,
                        stride: 8,
                        count: batch,
                    },
                    &WORD_LAYOUT,
                    &mut words[..batch * 8],
                )
            });
            let state_at =
                |index: usize| u64::from_le_bytes(words[index * 8..][..8].try_into().unwrap());
            let mut pointers = [0_usize; BATCH];
            let mut live = 0;
            for index in 0..copied {
                let state = state_at(index) as usize;
                if state == 0 {
                    continue;
                }
                if at(state, STATE_TEMPORARY_LIST_COUNT).is_err()
                    || at(state, STATE_TEMPORARY_FLAG).is_err()
                {
                    break;
                }
                pointers[live] = state;
                live += 1;
            }
            let mut gates = [0_u8; BATCH * 8];
            let fields = memory.copy_records(
                RecordSources::Pointers(&pointers[..live]),
                &GATE_LAYOUT,
                &mut gates[..live * 8],
            );
            let mut row = 0;
            for index in 0..batch {
                if index >= copied {
                    return Err("state unreadable");
                }
                let state = state_at(index) as usize;
                if state == 0 {
                    continue;
                }
                let (count, flag) = if row < live {
                    if fields < row * 2 + 1 {
                        return Err("state temporary list unreadable");
                    }
                    if fields < row * 2 + 2 {
                        return Err("state temporary flag unreadable");
                    }
                    let gate = &gates[row * 8..][..8];
                    (i32::from_le_bytes(gate[..4].try_into().unwrap()), gate[4])
                } else {
                    let count = read::<i32>(memory, at(state, STATE_TEMPORARY_LIST_COUNT)?)
                        .ok_or("state temporary list unreadable")?;
                    let flag = read::<u8>(memory, at(state, STATE_TEMPORARY_FLAG)?)
                        .ok_or("state temporary flag unreadable")?;
                    (count, flag)
                };
                row += 1;
                gated(state, count, flag)?;
            }
            first += batch as u32;
        }
        Ok(())
    })();
    match result {
        Ok(()) => outcome(len, unread),
        Err(reason) => Outcome::Unavailable { reason },
    }
}

pub fn read_route(
    memory: &impl Memory,
    base: usize,
    key: RouteKey,
    address: usize,
    market_game_state: Option<usize>,
    path: &mut Vec<u32>,
) -> Result<RouteRow, &'static str> {
    const HEADER: usize = 0x70;
    let header = read::<[u8; HEADER]>(memory, address).ok_or("route unreadable")?;
    let word = |offset: usize| {
        u64::from_le_bytes(
            header[offset..offset + 8]
                .try_into()
                .expect("inside the header"),
        ) as usize
    };
    if word(0) != vtable(base, ROUTE_VTABLE_RVA)? {
        return Err("route vtable mismatch");
    }
    let kind = header[ROUTE_KIND];
    let mut row = RouteRow {
        key,
        kind,
        path_start: path.len(),
        ..RouteRow::default()
    };
    if let Some(game_state) = market_game_state {
        let states = array(
            memory,
            at(game_state, cgame_state::STATES)?,
            8,
            "market route state roster unreadable",
        )?;
        let state_id = |state: usize| -> Result<Option<u32>, &'static str> {
            if state == 0 {
                return Ok(None);
            }
            if pointer(memory, state, "market route state identity unreadable")?
                != vtable(base, cstate::VTABLE_RVA)?
            {
                return Err("market route state vtable mismatch");
            }
            let id = read::<u32>(memory, at(state, cstate::ID)?)
                .ok_or("market route state id unreadable")?;
            if element(
                memory,
                states,
                id,
                8,
                "market route state roster slot unreadable",
            )? != state
            {
                return Err("market route state roster backlink mismatch");
            }
            Ok(Some(id))
        };
        row.source_state = state_id(word(ROUTE_FROM))?;
        row.destination_state = state_id(word(ROUTE_TO))?;
    }
    let (ends, list, id_offset, unreadable) = if kind == NAVAL {
        (
            [word(ROUTE_FROM_PORT), word(ROUTE_TO_PORT)],
            ROUTE_SEA_REGIONS,
            REGION_ID,
            "sea region id unreadable",
        )
    } else {
        (
            [word(ROUTE_FROM), word(ROUTE_TO)],
            ROUTE_LAND_STATES,
            cstate::ID,
            "route state id unreadable",
        )
    };
    let elements = Array::from_header(
        header[list..list + 16]
            .try_into()
            .expect("inside the header"),
        8,
    )
    .ok_or("route path header invalid")?;
    if ends[0] == 0 || ends[1] == 0 || elements.count == 0 {
        return Ok(row);
    }
    let end_id = if kind == NAVAL {
        PROVINCE_ID
    } else {
        cstate::ID
    };
    row.from = read::<u32>(memory, at(ends[0], end_id)?).ok_or("route end unreadable")?;
    row.to = read::<u32>(memory, at(ends[1], end_id)?).ok_or("route end unreadable")?;
    route_path(memory, elements, id_offset, unreadable, path)?;
    row.path_len = path.len() - row.path_start;
    Ok(row)
}

fn route_path(
    memory: &impl Memory,
    elements: Array,
    id_offset: usize,
    unreadable: &'static str,
    path: &mut Vec<u32>,
) -> Result<(), &'static str> {
    const BATCH: usize = 64;
    let id_fields = [RecordField {
        source_offset: id_offset,
        destination_offset: 0,
        len: 4,
    }];
    let id_layout = RecordLayout::new(&id_fields, 4).ok_or("route path layout invalid")?;
    let mut first = 0;
    let mut raw = [0_u8; BATCH * 8];
    let mut items = [0_usize; BATCH];
    let mut ids = [0_u8; BATCH * 4];
    while first < elements.count {
        let batch = ((elements.count - first) as usize).min(BATCH);
        let copied = if let Some(start) = elements.element(first, 8)
            && start.checked_add(batch * 8).is_some()
            && engine_native::read_slice(memory, start, &mut raw[..batch * 8])
        {
            batch
        } else {
            let mut addresses = [0_usize; BATCH];
            let mut words = [[0_u8; 8]; BATCH];
            let mut spans = batch;
            for (index, address) in addresses[..batch].iter_mut().enumerate() {
                match elements.element(first + index as u32, 8) {
                    Some(slot) => *address = slot,
                    None => {
                        spans = index;
                        break;
                    }
                }
            }
            let mut destinations: [&mut [u8]; BATCH] =
                std::array::from_fn(|_| <&mut [u8]>::default());
            for (destination, word) in destinations.iter_mut().zip(words[..spans].iter_mut()) {
                *destination = word.as_mut_slice();
            }
            let copied = memory
                .copy_scattered(addresses, &mut destinations)
                .min(spans);
            for (index, word) in words[..copied].iter().enumerate() {
                raw[index * 8..(index + 1) * 8].copy_from_slice(word);
            }
            copied
        };
        let mut stop = None;
        let mut live = 0;
        for (index, chunk) in raw[..copied * 8].chunks_exact(8).enumerate() {
            let item = u64::from_le_bytes(chunk.try_into().unwrap()) as usize;
            if item == 0 {
                stop = Some(Err("null route path element"));
                break;
            }
            if item.checked_add(id_offset).is_none() {
                stop = Some(at(item, id_offset).map(|_| ()));
                break;
            }
            items[index] = item;
            live += 1;
        }
        let stop = stop.unwrap_or(if live < batch {
            Err("route path element unreadable")
        } else {
            Ok(())
        });
        let read = memory.copy_records(
            RecordSources::Pointers(&items[..live]),
            &id_layout,
            &mut ids[..live * 4],
        );
        let ready = read.min(live);
        allocation::reserve(path, ready).map_err(|_| "route path allocation failed")?;
        for id in ids[..ready * 4].chunks_exact(4) {
            path.push(u32::from_le_bytes(id.try_into().expect("four bytes")));
        }
        if read < live {
            return Err(unreadable);
        }
        stop?;
        first += batch as u32;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
