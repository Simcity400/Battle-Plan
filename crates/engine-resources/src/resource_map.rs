
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
use engine_native_parts::modifier_entry;
use engine_sites::Rva;

fn effective_raw(
    memory: &impl Memory,
    base: usize,
    key: u32,
    raw: i64,
    metadata: &std::cell::Cell<Option<modifier_definition::Operands>>,
) -> Result<i64, &'static str> {
    if memory.modifier_definitions().is_some() {
        return modifier_definition::effective(memory, base, key, raw);
    }
    let operands = match metadata.get() {
        Some(operands) => operands,
        None => {
            let operands = modifier_definition::operands(memory, base, key)?;
            metadata.set(Some(operands));
            operands
        }
    };
    Ok(operands.effective(raw))
}

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
    effective_raw(memory, base, key, value.raw, metadata)
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

fn lazy_vtable(cache: &mut Option<u64>, base: usize, rva: Rva) -> Result<u64, &'static str> {
    match *cache {
        Some(value) => Ok(value),
        None => {
            let value = vtable(base, rva)? as u64;
            *cache = Some(value);
            Ok(value)
        }
    }
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

#[cfg(test)]
fn client_convoys(memory: &impl Memory, client: usize) -> Result<[i32; 2], &'static str> {
    let [assigned, required] =
        pair_u32(memory, at(client, CLIENT_CONVOYS)?).ok_or("convoy client convoys unreadable")?;
    Ok([assigned as i32, required as i32])
}

#[cfg(test)]
fn client_convoys_base(memory: &impl Memory, client: usize) -> Result<i32, &'static str> {
    read::<i32>(memory, at(client, CLIENT_CONVOYS_BASE)?)
        .ok_or("convoy client base convoys unreadable")
}

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
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
    local_factors: [Option<Result<i64, &'static str>>; ORIGIN_BATCH],
}

#[derive(Default)]
struct OriginBatchRead {
    live: usize,
    ptr_stop: Option<&'static str>,
    complete_origins: usize,
    partial_origin: usize,
    state_live: usize,
    state_stop: Option<&'static str>,
    complete_states: usize,
    partial_state: usize,
}

impl Default for OriginBatch {
    fn default() -> Self {
        Self {
            origin_ptrs: [0; ORIGIN_BATCH],
            origin_bytes: [0; ORIGIN_BATCH * ORIGIN_RECORD_STRIDE],
            state_ptrs: [0; ORIGIN_BATCH],
            state_bytes: [0; ORIGIN_BATCH * STATE_RECORD_STRIDE],
            local_factors: [None; ORIGIN_BATCH],
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
const RIGHTS_RECORD_STRIDE: usize = 32;
const RIGHTS_RECORD_FIELDS: [RecordField; 3] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: ORIGIN_STATE,
        destination_offset: 8,
        len: 8,
    },
    RecordField {
        source_offset: ORIGIN_RIGHTS,
        destination_offset: 16,
        len: 16,
    },
];
const RIGHTS_RECORD_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&RIGHTS_RECORD_FIELDS, RIGHTS_RECORD_STRIDE)
        .expect("valid origin rights record layout");
const STATE_ID_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: cstate::ID,
    destination_offset: 0,
    len: 4,
}];
const STATE_ID_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&STATE_ID_FIELDS, 4).expect("valid state id layout");
const ORIGIN_RECORD_STRIDE: usize = 84;
const ORIGIN_RECORD_FIELDS: [RecordField; 9] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: ORIGIN_STATE,
        destination_offset: 8,
        len: 8,
    },
    RecordField {
        source_offset: ORIGIN_RIGHTS,
        destination_offset: 16,
        len: 16,
    },
    RecordField {
        source_offset: ORIGIN_EFFICIENCY,
        destination_offset: 32,
        len: 8,
    },
    RecordField {
        source_offset: ORIGIN_ROUTE_FACTOR,
        destination_offset: 40,
        len: 8,
    },
    RecordField {
        source_offset: ORIGIN_ROUTE,
        destination_offset: 48,
        len: 8,
    },
    RecordField {
        source_offset: CLIENT_EFFICIENCY_A,
        destination_offset: 56,
        len: 16,
    },
    RecordField {
        source_offset: CLIENT_CONVOYS,
        destination_offset: 72,
        len: 8,
    },
    RecordField {
        source_offset: CLIENT_CONVOYS_BASE,
        destination_offset: 80,
        len: 4,
    },
];
const ORIGIN_RECORD_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&ORIGIN_RECORD_FIELDS, ORIGIN_RECORD_STRIDE)
        .expect("valid origin record layout");
const STATE_RECORD_STRIDE: usize = 40;
const STATE_RECORD_FIELDS: [RecordField; 3] = [
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
    RecordField {
        source_offset: cstate::BASE_MODIFIERS,
        destination_offset: 24,
        len: 16,
    },
];
const STATE_RECORD_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&STATE_RECORD_FIELDS, STATE_RECORD_STRIDE)
        .expect("valid origin state record layout");
const BUCKET_DIST_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: 4,
    destination_offset: 0,
    len: 1,
}];
const BUCKET_DIST_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&BUCKET_DIST_FIELDS, 1).expect("valid origin hash bucket distance layout");
const CLIENT_BATCH: usize = 64;
const SUPPLY_PTR_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: SUPPLY_TRANSFER_CLIENT,
    destination_offset: 0,
    len: 8,
}];
const SUPPLY_PTR_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&SUPPLY_PTR_FIELDS, 8).expect("valid supply transfer pointer layout");
const SUPPLY_RECORD_STRIDE: usize = 48;
const SUPPLY_RECORD_FIELDS: [RecordField; 6] = [
    RecordField {
        source_offset: SUPPLY_ROUTE,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: CLIENT_HANDLE_ID,
        destination_offset: 8,
        len: 4,
    },
    RecordField {
        source_offset: 0,
        destination_offset: 12,
        len: 8,
    },
    RecordField {
        source_offset: CLIENT_EFFICIENCY_A,
        destination_offset: 20,
        len: 16,
    },
    RecordField {
        source_offset: CLIENT_CONVOYS,
        destination_offset: 36,
        len: 8,
    },
    RecordField {
        source_offset: CLIENT_CONVOYS_BASE,
        destination_offset: 44,
        len: 4,
    },
];
const SUPPLY_RECORD_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&SUPPLY_RECORD_FIELDS, SUPPLY_RECORD_STRIDE)
        .expect("valid supply transfer record layout");
const GIVEN_RECORD_STRIDE: usize = 52;
const GIVEN_RECORD_FIELDS: [RecordField; 7] = [
    RecordField {
        source_offset: LEND_LEASE_RECEIVER,
        destination_offset: 0,
        len: 4,
    },
    RecordField {
        source_offset: LEND_LEASE_ROUTE,
        destination_offset: 4,
        len: 8,
    },
    RecordField {
        source_offset: 0,
        destination_offset: 12,
        len: 8,
    },
    RecordField {
        source_offset: CLIENT_HANDLE_ID,
        destination_offset: 20,
        len: 4,
    },
    RecordField {
        source_offset: CLIENT_EFFICIENCY_A,
        destination_offset: 24,
        len: 16,
    },
    RecordField {
        source_offset: CLIENT_CONVOYS,
        destination_offset: 40,
        len: 8,
    },
    RecordField {
        source_offset: CLIENT_CONVOYS_BASE,
        destination_offset: 48,
        len: 4,
    },
];
const GIVEN_RECORD_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&GIVEN_RECORD_FIELDS, GIVEN_RECORD_STRIDE)
        .expect("valid lend-lease record layout");
const CONTRACT_RECORD_STRIDE: usize = 56;
const CONTRACT_RECORD_FIELDS: [RecordField; 7] = [
    RecordField {
        source_offset: CONTRACT_SELLER,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: CONTRACT_CLIENT + CONTRACT_CLIENT_ROUTE,
        destination_offset: 8,
        len: 8,
    },
    RecordField {
        source_offset: CONTRACT_CLIENT,
        destination_offset: 16,
        len: 8,
    },
    RecordField {
        source_offset: CONTRACT_CLIENT + CLIENT_HANDLE_ID,
        destination_offset: 24,
        len: 4,
    },
    RecordField {
        source_offset: CONTRACT_CLIENT + CLIENT_EFFICIENCY_A,
        destination_offset: 28,
        len: 16,
    },
    RecordField {
        source_offset: CONTRACT_CLIENT + CLIENT_CONVOYS,
        destination_offset: 44,
        len: 8,
    },
    RecordField {
        source_offset: CONTRACT_CLIENT + CLIENT_CONVOYS_BASE,
        destination_offset: 52,
        len: 4,
    },
];
const CONTRACT_RECORD_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&CONTRACT_RECORD_FIELDS, CONTRACT_RECORD_STRIDE)
        .expect("valid market contract record layout");
const CLIENT_HANDLE_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: CLIENT_HANDLE_ID,
    destination_offset: 0,
    len: 4,
}];
const CLIENT_HANDLE_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&CLIENT_HANDLE_FIELDS, 4).expect("valid client handle layout");
const CONTRACT_HANDLE_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: CONTRACT_CLIENT + CLIENT_HANDLE_ID,
    destination_offset: 0,
    len: 4,
}];
const CONTRACT_HANDLE_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&CONTRACT_HANDLE_FIELDS, 4).expect("valid contract handle layout");

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
    ) -> Result<([u8; cresource_exchange::SIZE], usize, u32), &'static str> {
        let (bytes, ends) = cresource_exchange::authenticate(
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
        Ok((bytes, route, ends.exporter_country.id))
    }

    #[cfg(test)]
    fn authenticate_route(&self, route: usize) -> Result<(), &'static str> {
        if pointer(self.memory, route, "route vtable unreadable")?
            != vtable(self.base, ROUTE_VTABLE_RVA)?
        {
            return Err("route vtable mismatch");
        }
        Ok(())
    }

    #[cfg(test)]
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

    #[cfg(test)]
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

    fn read_rights_only(
        &self,
        origins: Array,
        mut first: u32,
        expected_vtable: &mut Option<u64>,
        table: &mut Table,
        output: &mut Buffer<'_, RightsRow>,
    ) -> Result<(), &'static str> {
        let mut ptr_bytes = [0_u8; ORIGIN_BATCH * 8];
        let mut origin_ptrs = [0_usize; ORIGIN_BATCH];
        let mut records = [0_u8; ORIGIN_BATCH * RIGHTS_RECORD_STRIDE];
        let mut state_ptrs = [0_usize; ORIGIN_BATCH];
        let mut state_ids = [0_u8; ORIGIN_BATCH * 4];
        while first < origins.count {
            let count = ((origins.count - first) as usize).min(ORIGIN_BATCH);
            let start = origins.element(first, 8).ok_or("origin unreadable")?;
            let copied_ptrs = self.memory.copy_records(
                RecordSources::Strided {
                    base: start,
                    stride: 8,
                    count,
                },
                &ORIGIN_PTR_LAYOUT,
                &mut ptr_bytes[..count * 8],
            );
            let mut live = 0;
            let mut ptr_stop = None;
            for i in 0..copied_ptrs {
                let Ok(origin) = usize::try_from(u64::from_le_bytes(
                    ptr_bytes[i * 8..(i + 1) * 8].try_into().unwrap(),
                )) else {
                    ptr_stop = Some("origin unreadable");
                    break;
                };
                if origin == 0 {
                    ptr_stop = Some("null origin in a live list");
                    break;
                }
                if origin.checked_add(ORIGIN_RIGHTS + 16).is_none() {
                    ptr_stop = Some("resources map address overflow");
                    break;
                }
                origin_ptrs[live] = origin;
                live += 1;
            }
            if ptr_stop.is_none() && copied_ptrs < count {
                ptr_stop = Some("origin unreadable");
            }
            let copied_fields = if live != 0 {
                self.memory.copy_records(
                    RecordSources::Pointers(&origin_ptrs[..live]),
                    &RIGHTS_RECORD_LAYOUT,
                    &mut records[..live * RIGHTS_RECORD_STRIDE],
                )
            } else {
                0
            };
            let complete = copied_fields / RIGHTS_RECORD_FIELDS.len();
            let partial = copied_fields % RIGHTS_RECORD_FIELDS.len();
            let mut state_live = 0;
            let mut state_stop = None;
            for i in 0..complete {
                let rec = &records[i * RIGHTS_RECORD_STRIDE..(i + 1) * RIGHTS_RECORD_STRIDE];
                match lazy_vtable(expected_vtable, self.base, ORIGIN_VTABLE_RVA) {
                    Ok(expected) if u64::from_le_bytes(rec[0..8].try_into().unwrap()) == expected => {}
                    Ok(_) => {
                        state_stop = Some("origin vtable mismatch");
                        break;
                    }
                    Err(reason) => {
                        state_stop = Some(reason);
                        break;
                    }
                }
                let Ok(state) = usize::try_from(u64::from_le_bytes(rec[8..16].try_into().unwrap()))
                else {
                    state_stop = Some("origin without a state");
                    break;
                };
                if state == 0 {
                    state_stop = Some("origin without a state");
                    break;
                }
                if state.checked_add(cstate::ID + 4).is_none() {
                    state_stop = Some("resources map address overflow");
                    break;
                }
                state_ptrs[state_live] = state;
                state_live += 1;
            }
            let copied_states = if state_live != 0 {
                self.memory.copy_records(
                    RecordSources::Pointers(&state_ptrs[..state_live]),
                    &STATE_ID_LAYOUT,
                    &mut state_ids[..state_live * 4],
                )
            } else {
                0
            };
            for i in 0..copied_states {
                let rec = &records[i * RIGHTS_RECORD_STRIDE..(i + 1) * RIGHTS_RECORD_STRIDE];
                let state = u32::from_le_bytes(state_ids[i * 4..(i + 1) * 4].try_into().unwrap());
                let rights = Array::from_header(rec[16..32].try_into().unwrap(), RIGHTS_STRIDE)
                    .ok_or("origin rights unreadable")?;
                self.rights_from_array(rights, state, table, output)?;
            }
            if copied_states < state_live {
                return Err("origin state id unreadable");
            }
            if let Some(reason) = state_stop {
                return Err(reason);
            }
            if complete < live {
                let rec = &records[complete * RIGHTS_RECORD_STRIDE..(complete + 1) * RIGHTS_RECORD_STRIDE];
                if partial == 0 {
                    return Err("origin vtable unreadable");
                }
                if u64::from_le_bytes(rec[0..8].try_into().unwrap())
                    != lazy_vtable(expected_vtable, self.base, ORIGIN_VTABLE_RVA)?
                {
                    return Err("origin vtable mismatch");
                }
                if partial == 1 {
                    return Err("origin state unreadable");
                }
                let state = usize::try_from(u64::from_le_bytes(rec[8..16].try_into().unwrap()))
                    .ok()
                    .filter(|&p| p != 0)
                    .ok_or("origin without a state")?;
                let _ = read::<u32>(self.memory, at(state, cstate::ID)?)
                    .ok_or("origin state id unreadable")?;
                return Err("origin rights unreadable");
            }
            if let Some(reason) = ptr_stop {
                return Err(reason);
            }
            first += count as u32;
        }
        Ok(())
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "batched origin acquisition threads list slice, cached owner, vtable cache and reusable scratch buffers"
    )]
    fn read_origin_batch(
        &self,
        origins: Array,
        stride: usize,
        (first, count): (u32, usize),
        country_id: u32,
        cached_owner: &mut Option<Result<u32, &'static str>>,
        expected_vtable: &mut Option<u64>,
        batch: &mut OriginBatch,
    ) -> OriginBatchRead {
        let mut status = OriginBatchRead::default();
        let Some(start) = origins.element(first, stride) else {
            status.ptr_stop = Some("origin unreadable");
            return status;
        };
        let mut ptr_bytes = [0_u8; ORIGIN_BATCH * 8];
        let copied_ptrs = self.memory.copy_records(
            RecordSources::Strided {
                base: start,
                stride,
                count,
            },
            &ORIGIN_PTR_LAYOUT,
            &mut ptr_bytes[..count * 8],
        );
        for chunk in ptr_bytes[..copied_ptrs * 8].chunks_exact(8) {
            let Ok(origin) = usize::try_from(u64::from_le_bytes(chunk.try_into().unwrap())) else {
                status.ptr_stop = Some("origin unreadable");
                break;
            };
            if origin == 0 {
                status.ptr_stop = Some("null origin in a live list");
                break;
            }
            if origin.checked_add(ORIGIN_RIGHTS + 16).is_none() {
                status.ptr_stop = Some("resources map address overflow");
                break;
            }
            batch.origin_ptrs[status.live] = origin;
            status.live += 1;
        }
        if status.ptr_stop.is_none() && copied_ptrs < count {
            status.ptr_stop = Some("origin unreadable");
        }
        let copied_origins = if status.live != 0 {
            self.memory.copy_records(
                RecordSources::Pointers(&batch.origin_ptrs[..status.live]),
                &ORIGIN_RECORD_LAYOUT,
                &mut batch.origin_bytes[..status.live * ORIGIN_RECORD_STRIDE],
            )
        } else {
            0
        };
        status.complete_origins = copied_origins / ORIGIN_RECORD_FIELDS.len();
        status.partial_origin = copied_origins % ORIGIN_RECORD_FIELDS.len();
        for i in 0..status.complete_origins {
            let record = batch.origin_record(i);
            match lazy_vtable(expected_vtable, self.base, ORIGIN_VTABLE_RVA) {
                Ok(expected)
                    if u64::from_le_bytes(record[0..8].try_into().unwrap()) == expected => {}
                Ok(_) => {
                    status.state_stop = Some("origin vtable mismatch");
                    break;
                }
                Err(reason) => {
                    status.state_stop = Some(reason);
                    break;
                }
            }
            let Ok(state) = usize::try_from(u64::from_le_bytes(record[8..16].try_into().unwrap()))
            else {
                status.state_stop = Some("origin without a state");
                break;
            };
            if state == 0 {
                status.state_stop = Some("origin without a state");
                break;
            }
            if state.checked_add(cstate::ID + 4).is_none()
                || state.checked_add(cstate::COUNTRY_MODIFIERS + 17).is_none()
                || state.checked_add(cstate::BASE_MODIFIERS + 16).is_none()
            {
                status.state_stop = Some("resources map address overflow");
                break;
            }
            batch.state_ptrs[status.state_live] = state;
            status.state_live += 1;
        }
        let copied_states = if status.state_live != 0 {
            self.memory.copy_records(
                RecordSources::Pointers(&batch.state_ptrs[..status.state_live]),
                &STATE_RECORD_LAYOUT,
                &mut batch.state_bytes[..status.state_live * STATE_RECORD_STRIDE],
            )
        } else {
            0
        };
        status.complete_states = copied_states / STATE_RECORD_FIELDS.len();
        status.partial_state = copied_states % STATE_RECORD_FIELDS.len();
        let ready = status.complete_states;
        batch.local_factors[..ready].fill(None);
        let owner_hash = if self.game_state == 0 || country_id == 0 {
            Some(country_id)
        } else if let Ok(Some(links)) = self.links {
            cached_owner
                .get_or_insert_with(|| {
                    links
                        .element(country_id, 4)
                        .and_then(|address| read(self.memory, address))
                        .ok_or("state movement country link unreadable")
                })
                .ok()
        } else {
            None
        };
        if let Some(hash) = owner_hash
            && ready != 0
        {
            let mut bucket_rows = [0_usize; ORIGIN_BATCH];
            let mut bucket_ptrs = [0_usize; ORIGIN_BATCH];
            let mut bucket_count = 0_usize;
            for i in 0..ready {
                let state_record = batch.state_record(i);
                let header = &state_record[4..21];
                let data = u64::from_le_bytes(header[..8].try_into().unwrap()) as usize;
                let mask = u32::from_le_bytes(header[12..16].try_into().unwrap());
                let Some(buckets) = mask
                    .checked_add(1)
                    .filter(|c| c.is_power_of_two() && *c <= i32::MAX as u32)
                else {
                    continue;
                };
                let Some(sentinel) = buckets.checked_add(u32::from(header[16])) else {
                    continue;
                };
                let index = hash & mask;
                if data == 0 || index >= sentinel {
                    continue;
                }
                let Some(bucket) =
                    data.checked_add(index as usize * cstate::COUNTRY_MODIFIER_STRIDE)
                else {
                    continue;
                };
                if bucket.checked_add(5).is_none() {
                    continue;
                }
                bucket_rows[bucket_count] = i;
                bucket_ptrs[bucket_count] = bucket;
                bucket_count += 1;
            }
            let mut dist = [0_u8; ORIGIN_BATCH];
            let copied = if bucket_count != 0 {
                self.memory.copy_records(
                    RecordSources::Pointers(&bucket_ptrs[..bucket_count]),
                    &BUCKET_DIST_LAYOUT,
                    &mut dist[..bucket_count],
                )
            } else {
                0
            };
            if copied < bucket_count {
                batch.local_factors[bucket_rows[copied]] =
                    Some(Err("state country modifier entry unreadable"));
            }
            const ACC_GATHER: usize = 16;
            const ACC_BUF_BYTES: usize = 4096;
            let mut acc_buf = [0_u8; ACC_BUF_BYTES];
            let mut acc_spans = [(0_usize, 0_usize); ACC_GATHER];
            let mut acc_rows = [0_usize; ACC_GATHER];
            let mut acc_len = 0_usize;
            let mut acc_used = 0_usize;
            let flush_acc = |acc_len: &mut usize,
                             acc_used: &mut usize,
                             acc_spans: &[(usize, usize); ACC_GATHER],
                             acc_rows: &[usize; ACC_GATHER],
                             acc_buf: &mut [u8; ACC_BUF_BYTES],
                             local_factors: &mut [Option<Result<i64, &'static str>>; ORIGIN_BATCH]| {
                if *acc_len == 0 {
                    return;
                }
                let mut addrs = [0_usize; ACC_GATHER];
                let mut slices: [&mut [u8]; ACC_GATHER] =
                    std::array::from_fn(|_| <&mut [u8]>::default());
                let mut rest = &mut acc_buf[..*acc_used];
                for idx in 0..*acc_len {
                    let (addr, len) = acc_spans[idx];
                    let (head, tail) = std::mem::take(&mut rest).split_at_mut(len);
                    addrs[idx] = addr;
                    slices[idx] = head;
                    rest = tail;
                }
                let ok = self.memory.copy_scattered_prefetched(addrs, &mut slices);
                let mut offset = 0_usize;
                for idx in 0..ok.min(*acc_len) {
                    let len = acc_spans[idx].1;
                    let entries = &acc_buf[offset..offset + len];
                    offset += len;
                    let mut previous_key = 0_u32;
                    let mut valid = true;
                    let mut found = None;
                    for entry in entries.chunks_exact(modifier_entry::STRIDE) {
                        let key = u32::from_le_bytes(entry[..4].try_into().unwrap());
                        if key <= previous_key {
                            valid = false;
                            break;
                        }
                        previous_key = key;
                        if key == LOCAL_RESOURCES_FACTOR {
                            found = Some(i64::from_le_bytes(entry[8..16].try_into().unwrap()));
                        }
                    }
                    local_factors[acc_rows[idx]] = Some(if !valid {
                        Err("modifier accumulator keys not positive and increasing")
                    } else {
                        match found {
                            Some(raw) => effective_raw(
                                self.memory,
                                self.base,
                                LOCAL_RESOURCES_FACTOR,
                                raw,
                                &self.local_resources,
                            ),
                            None => Ok(0),
                        }
                    });
                }
                if ok < *acc_len {
                    local_factors[acc_rows[ok]] = Some(Err("modifier accumulator entry unreadable"));
                }
                *acc_len = 0;
                *acc_used = 0;
            };
            for k in 0..copied {
                if dist[k] != 0 {
                    continue;
                }
                let i = bucket_rows[k];
                let acc_addr = batch.state_ptrs[i] + cstate::BASE_MODIFIERS;
                if self
                    .memory
                    .modifier_accumulator_value(acc_addr, LOCAL_RESOURCES_FACTOR)
                    .is_some()
                {
                    continue;
                }
                let state_record = batch.state_record(i);
                let Some(arr) = Array::from_header(
                    state_record[24..40].try_into().unwrap(),
                    modifier_entry::STRIDE,
                ) else {
                    batch.local_factors[i] =
                        Some(Err("modifier accumulator header unreadable or invalid"));
                    continue;
                };
                if arr.count == 0 {
                    batch.local_factors[i] = Some(Ok(0));
                    continue;
                }
                if arr.count > 64 {
                    continue;
                }
                let byte_len = arr.count as usize * modifier_entry::STRIDE;
                if arr.data == 0 || arr.data.checked_add(byte_len).is_none() {
                    continue;
                }
                if acc_len == ACC_GATHER || acc_used + byte_len > ACC_BUF_BYTES {
                    flush_acc(
                        &mut acc_len,
                        &mut acc_used,
                        &acc_spans,
                        &acc_rows,
                        &mut acc_buf,
                        &mut batch.local_factors,
                    );
                }
                acc_spans[acc_len] = (arr.data, byte_len);
                acc_rows[acc_len] = i;
                acc_len += 1;
                acc_used += byte_len;
            }
            flush_acc(
                &mut acc_len,
                &mut acc_used,
                &acc_spans,
                &acc_rows,
                &mut acc_buf,
                &mut batch.local_factors,
            );
        }
        status
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

    #[expect(
        clippy::too_many_arguments,
        reason = "origin row assembly borrows batch scratch, cached owner, and lazy route vtable"
    )]
    fn origin_from_batch(
        &self,
        batch: &OriginBatch,
        index: usize,
        list: u8,
        ordinal: u32,
        country: engine_native_memory::country_identity::Country,
        cached_owner: &mut Option<Result<u32, &'static str>>,
        expected_route: &mut Option<u64>,
    ) -> Result<OriginRow, &'static str> {
        let origin = batch.origin_ptrs[index];
        let record = batch.origin_record(index);
        let state_pointer = batch.state_ptrs[index];
        let state_record = batch.state_record(index);
        let state = u32::from_le_bytes(state_record[0..4].try_into().unwrap());
        let modifiers: [u8; 17] = state_record[4..21].try_into().unwrap();
        let efficiency = i64::from_le_bytes(record[32..40].try_into().unwrap());
        let route_factor = i64::from_le_bytes(record[40..48].try_into().unwrap());
        let local_resources_factor = match batch.local_factors[index] {
            Some(result) => result?,
            None => {
                let accumulator = self.country_modifier_accumulator_from_header(
                    state_pointer,
                    country.id,
                    modifiers,
                    cached_owner,
                )?;
                effective(
                    self.memory,
                    self.base,
                    accumulator,
                    LOCAL_RESOURCES_FACTOR,
                    &self.local_resources,
                )?
            }
        };
        let route_address = at(origin, ORIGIN_ROUTE)?;
        if u64::from_le_bytes(record[48..56].try_into().unwrap())
            != lazy_vtable(expected_route, self.base, ROUTE_VTABLE_RVA)?
        {
            return Err("route vtable mismatch");
        }
        let status = ClientStatus::new(
            [
                i64::from_le_bytes(record[56..64].try_into().unwrap()),
                i64::from_le_bytes(record[64..72].try_into().unwrap()),
            ],
            [
                i32::from_le_bytes(record[72..76].try_into().unwrap()),
                i32::from_le_bytes(record[76..80].try_into().unwrap()),
            ],
            i32::from_le_bytes(record[80..84].try_into().unwrap()),
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

    #[expect(
        clippy::too_many_arguments,
        reason = "batch completion forwards the batch state, caches and output table without re-reading faulted fields"
    )]
    fn finish_origins_batch(
        &self,
        batch: &OriginBatch,
        read: &OriginBatchRead,
        list: u8,
        first: u32,
        country: engine_native_memory::country_identity::Country,
        cached_owner: &mut Option<Result<u32, &'static str>>,
        expected_vtable: &mut Option<u64>,
        expected_route: &mut Option<u64>,
        table: &mut Table,
        output: &mut Buffer<'_, OriginRow>,
    ) -> Result<(), &'static str> {
        for index in 0..read.complete_states {
            let row = self.origin_from_batch(
                batch,
                index,
                list,
                first + index as u32,
                country,
                cached_owner,
                expected_route,
            )?;
            table.push(output, row)?;
        }
        if read.complete_states < read.state_live {
            return Err(match read.partial_state {
                0 => "origin state id unreadable",
                1 => "state country modifiers unreadable",
                _ => "modifier accumulator header unreadable or invalid",
            });
        }
        if let Some(reason) = read.state_stop {
            return Err(reason);
        }
        if read.complete_origins < read.live {
            let record = batch.origin_record(read.complete_origins);
            if read.partial_origin == 0 {
                return Err("origin vtable unreadable");
            }
            if u64::from_le_bytes(record[0..8].try_into().unwrap())
                != lazy_vtable(expected_vtable, self.base, ORIGIN_VTABLE_RVA)?
            {
                return Err("origin vtable mismatch");
            }
            if read.partial_origin == 1 {
                return Err("origin state unreadable");
            }
            if u64::from_le_bytes(record[8..16].try_into().unwrap()) == 0 {
                return Err("origin without a state");
            }
            if read.partial_origin >= 6
                && u64::from_le_bytes(record[48..56].try_into().unwrap())
                    != lazy_vtable(expected_route, self.base, ROUTE_VTABLE_RVA)?
            {
                return Err("route vtable mismatch");
            }
            return Err(match read.partial_origin {
                2 => "origin rights unreadable",
                3 => "origin efficiency unreadable",
                4 => "origin route factor unreadable",
                5 => "route vtable unreadable",
                6 => "convoy client efficiency unreadable",
                7 => "convoy client convoys unreadable",
                _ => "convoy client base convoys unreadable",
            });
        }
        if let Some(reason) = read.ptr_stop {
            return Err(reason);
        }
        Ok(())
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
        let mut expected_route = None;
        let imports = array(
            memory,
            at(resources, ccountry_resources::IMPORT_ROUTES)?,
            8,
            "import list unreadable",
        )?;
        let mut ptr_bytes = [0_u8; CLIENT_BATCH * 8];
        let mut first = 0_u32;
        while first < imports.count {
            let count = ((imports.count - first) as usize).min(CLIENT_BATCH);
            let start = imports
                .element(first, 8)
                .ok_or("import exchange unreadable")?;
            let copied = memory.copy_records(
                RecordSources::Strided {
                    base: start,
                    stride: 8,
                    count,
                },
                &ORIGIN_PTR_LAYOUT,
                &mut ptr_bytes[..count * 8],
            );
            for offset in 0..copied {
                let client = usize::try_from(u64::from_le_bytes(
                    ptr_bytes[offset * 8..(offset + 1) * 8].try_into().unwrap(),
                ))
                .map_err(|_| "import exchange unreadable")?;
                if client == 0 {
                    return Err("null exchange in a live import list");
                }
                let (bytes, route, exporter) = self.exchange(client, country)?;
                let key = RouteKey {
                    class: DELIVERY_ROUTE_CLASS,
                    country: exporter,
                    other: country.id,
                };
                let handle = u32::from_le_bytes(
                    bytes[CLIENT_HANDLE_ID..CLIENT_HANDLE_ID + 4]
                        .try_into()
                        .unwrap(),
                );
                let status = ClientStatus::new(
                    [
                        i64::from_le_bytes(
                            bytes[CLIENT_EFFICIENCY_A..CLIENT_EFFICIENCY_A + 8]
                                .try_into()
                                .unwrap(),
                        ),
                        i64::from_le_bytes(
                            bytes[CLIENT_EFFICIENCY_A + 8..CLIENT_EFFICIENCY_A + 16]
                                .try_into()
                                .unwrap(),
                        ),
                    ],
                    [
                        i32::from_le_bytes(
                            bytes[CLIENT_CONVOYS..CLIENT_CONVOYS + 4]
                                .try_into()
                                .unwrap(),
                        ),
                        i32::from_le_bytes(
                            bytes[CLIENT_CONVOYS + 4..CLIENT_CONVOYS + 8]
                                .try_into()
                                .unwrap(),
                        ),
                    ],
                    i32::from_le_bytes(
                        bytes[CLIENT_CONVOYS_BASE..CLIENT_CONVOYS_BASE + 4]
                            .try_into()
                            .unwrap(),
                    ),
                    self.threshold,
                );
                table.push(
                    output,
                    ClientRow {
                        owner_ordinal: None,
                        other_ordinal: None,
                        address: client,
                        handle,
                        category: IMPORT,
                        owner: country.id,
                        other: exporter,
                        route: key,
                        bad: status.bad,
                        convoys: status.convoys,
                        efficiency: status.efficiency,
                        convoys_base: status.convoys_base,
                        route_address: route,
                    },
                )?;
            }
            if copied < count {
                return Err("import exchange unreadable");
            }
            first += count as u32;
        }
        let given = array(
            memory,
            at(resources, LEND_LEASE_GIVEN)?,
            8,
            "lend-lease list unreadable",
        )?;
        let mut expected_given = None;
        let mut ptrs = [0_usize; CLIENT_BATCH];
        let mut given_records = [0_u8; CLIENT_BATCH * GIVEN_RECORD_STRIDE];
        let mut route_ptrs = [0_usize; CLIENT_BATCH];
        let mut route_vtables = [0_u8; CLIENT_BATCH * 8];
        let mut first = 0_u32;
        while first < given.count {
            let count = ((given.count - first) as usize).min(CLIENT_BATCH);
            let start = given
                .element(first, 8)
                .ok_or("lend-lease exchange unreadable")?;
            let copied_ptrs = memory.copy_records(
                RecordSources::Strided {
                    base: start,
                    stride: 8,
                    count,
                },
                &ORIGIN_PTR_LAYOUT,
                &mut ptr_bytes[..count * 8],
            );
            let mut live = 0;
            let mut ptr_stop = None;
            for i in 0..copied_ptrs {
                let Ok(client) = usize::try_from(u64::from_le_bytes(
                    ptr_bytes[i * 8..(i + 1) * 8].try_into().unwrap(),
                )) else {
                    ptr_stop = Some("lend-lease exchange unreadable");
                    break;
                };
                if client == 0 {
                    ptr_stop = Some("null lend-lease exchange in a live list");
                    break;
                }
                if client.checked_add(LEND_LEASE_ROUTE + 8).is_none() {
                    ptr_stop = Some("resources map address overflow");
                    break;
                }
                ptrs[live] = client;
                live += 1;
            }
            if ptr_stop.is_none() && copied_ptrs < count {
                ptr_stop = Some("lend-lease exchange unreadable");
            }
            let copied_fields = if live != 0 {
                memory.copy_records(
                    RecordSources::Pointers(&ptrs[..live]),
                    &GIVEN_RECORD_LAYOUT,
                    &mut given_records[..live * GIVEN_RECORD_STRIDE],
                )
            } else {
                0
            };
            let complete = copied_fields / GIVEN_RECORD_FIELDS.len();
            let partial = copied_fields % GIVEN_RECORD_FIELDS.len();
            let route_scan = complete + usize::from(complete < live && partial >= 2);
            let mut route_live = 0;
            for i in 0..route_scan {
                let rec = &given_records[i * GIVEN_RECORD_STRIDE..(i + 1) * GIVEN_RECORD_STRIDE];
                let Ok(route) = usize::try_from(u64::from_le_bytes(rec[4..12].try_into().unwrap()))
                else {
                    break;
                };
                if route == 0 {
                    break;
                }
                route_ptrs[route_live] = route;
                route_live += 1;
            }
            let copied_routes = if route_live != 0 {
                memory.copy_records(
                    RecordSources::Pointers(&route_ptrs[..route_live]),
                    &ORIGIN_PTR_LAYOUT,
                    &mut route_vtables[..route_live * 8],
                )
            } else {
                0
            };
            for i in 0..complete {
                let rec = &given_records[i * GIVEN_RECORD_STRIDE..(i + 1) * GIVEN_RECORD_STRIDE];
                let receiver = self.id(u32::from_le_bytes(rec[0..4].try_into().unwrap()))?;
                if i >= route_live {
                    return Err("client holds no route");
                }
                if i >= copied_routes {
                    return Err("route vtable unreadable");
                }
                if u64::from_le_bytes(route_vtables[i * 8..(i + 1) * 8].try_into().unwrap())
                    != lazy_vtable(&mut expected_route, self.base, ROUTE_VTABLE_RVA)?
                {
                    return Err("route vtable mismatch");
                }
                if u64::from_le_bytes(rec[12..20].try_into().unwrap())
                    != lazy_vtable(&mut expected_given, self.base, LEND_LEASE_VTABLE_RVA)?
                {
                    return Err("convoy client of an unexpected type");
                }
                let handle = u32::from_le_bytes(rec[20..24].try_into().unwrap());
                let status = ClientStatus::new(
                    [
                        i64::from_le_bytes(rec[24..32].try_into().unwrap()),
                        i64::from_le_bytes(rec[32..40].try_into().unwrap()),
                    ],
                    [
                        i32::from_le_bytes(rec[40..44].try_into().unwrap()),
                        i32::from_le_bytes(rec[44..48].try_into().unwrap()),
                    ],
                    i32::from_le_bytes(rec[48..52].try_into().unwrap()),
                    self.threshold,
                );
                let key = RouteKey {
                    class: DELIVERY_ROUTE_CLASS,
                    country: country.id,
                    other: receiver,
                };
                table.push(
                    output,
                    ClientRow {
                        owner_ordinal: Some(first + i as u32),
                        other_ordinal: None,
                        address: ptrs[i],
                        handle,
                        category: LEND_LEASE,
                        owner: country.id,
                        other: receiver,
                        route: key,
                        bad: status.bad,
                        convoys: status.convoys,
                        efficiency: status.efficiency,
                        convoys_base: status.convoys_base,
                        route_address: route_ptrs[i],
                    },
                )?;
            }
            if complete < live {
                let rec = &given_records
                    [complete * GIVEN_RECORD_STRIDE..(complete + 1) * GIVEN_RECORD_STRIDE];
                if partial >= 1 {
                    let _ = self.id(u32::from_le_bytes(rec[0..4].try_into().unwrap()))?;
                }
                if partial >= 2 {
                    if complete >= route_live {
                        return Err("client holds no route");
                    }
                    if complete >= copied_routes {
                        return Err("route vtable unreadable");
                    }
                    if u64::from_le_bytes(
                        route_vtables[complete * 8..(complete + 1) * 8]
                            .try_into()
                            .unwrap(),
                    ) != lazy_vtable(&mut expected_route, self.base, ROUTE_VTABLE_RVA)?
                    {
                        return Err("route vtable mismatch");
                    }
                }
                if partial >= 3
                    && u64::from_le_bytes(rec[12..20].try_into().unwrap())
                        != lazy_vtable(&mut expected_given, self.base, LEND_LEASE_VTABLE_RVA)?
                {
                    return Err("convoy client of an unexpected type");
                }
                return Err(match partial {
                    0 => "lend-lease receiver unreadable",
                    1 => "client route unreadable",
                    2 => "convoy client vtable unreadable",
                    3 => "convoy client handle unreadable",
                    4 => "convoy client efficiency unreadable",
                    5 => "convoy client convoys unreadable",
                    _ => "convoy client base convoys unreadable",
                });
            }
            if let Some(reason) = ptr_stop {
                return Err(reason);
            }
            first += count as u32;
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
        let mut expected_supply = None;
        let mut supply_records = [0_u8; CLIENT_BATCH * SUPPLY_RECORD_STRIDE];
        let mut first = 0_u32;
        while first < transfers.count {
            let count = ((transfers.count - first) as usize).min(CLIENT_BATCH);
            let start = transfers
                .element(first, SUPPLY_TRANSFER_STRIDE)
                .ok_or("supply transfer unreadable")?;
            let copied_ptrs = memory.copy_records(
                RecordSources::Strided {
                    base: start,
                    stride: SUPPLY_TRANSFER_STRIDE,
                    count,
                },
                &SUPPLY_PTR_LAYOUT,
                &mut ptr_bytes[..count * 8],
            );
            let mut live = 0;
            let mut ptr_stop = None;
            for i in 0..copied_ptrs {
                let Ok(client) = usize::try_from(u64::from_le_bytes(
                    ptr_bytes[i * 8..(i + 1) * 8].try_into().unwrap(),
                )) else {
                    ptr_stop = Some("supply transfer client unreadable");
                    break;
                };
                if client == 0 {
                    ptr_stop = Some("supply transfer without a client");
                    break;
                }
                if client.checked_add(SUPPLY_ROUTE + 8).is_none() {
                    ptr_stop = Some("resources map address overflow");
                    break;
                }
                ptrs[live] = client;
                live += 1;
            }
            if ptr_stop.is_none() && copied_ptrs < count {
                ptr_stop = Some("supply transfer client unreadable");
            }
            let copied_fields = if live != 0 {
                memory.copy_records(
                    RecordSources::Pointers(&ptrs[..live]),
                    &SUPPLY_RECORD_LAYOUT,
                    &mut supply_records[..live * SUPPLY_RECORD_STRIDE],
                )
            } else {
                0
            };
            let complete = copied_fields / SUPPLY_RECORD_FIELDS.len();
            let partial = copied_fields % SUPPLY_RECORD_FIELDS.len();
            for i in 0..complete {
                let rec = &supply_records[i * SUPPLY_RECORD_STRIDE..(i + 1) * SUPPLY_RECORD_STRIDE];
                if u64::from_le_bytes(rec[0..8].try_into().unwrap())
                    != lazy_vtable(&mut expected_route, self.base, ROUTE_VTABLE_RVA)?
                {
                    return Err("route vtable mismatch");
                }
                if u64::from_le_bytes(rec[12..20].try_into().unwrap())
                    != lazy_vtable(&mut expected_supply, self.base, SUPPLY_VTABLE_RVA)?
                {
                    return Err("convoy client of an unexpected type");
                }
                let client = ptrs[i];
                let handle = u32::from_le_bytes(rec[8..12].try_into().unwrap());
                let status = ClientStatus::new(
                    [
                        i64::from_le_bytes(rec[20..28].try_into().unwrap()),
                        i64::from_le_bytes(rec[28..36].try_into().unwrap()),
                    ],
                    [
                        i32::from_le_bytes(rec[36..40].try_into().unwrap()),
                        i32::from_le_bytes(rec[40..44].try_into().unwrap()),
                    ],
                    i32::from_le_bytes(rec[44..48].try_into().unwrap()),
                    self.threshold,
                );
                let key = RouteKey {
                    class: SUPPLY_ROUTE_CLASS,
                    country: country.id,
                    other: handle,
                };
                table.push(
                    output,
                    ClientRow {
                        owner_ordinal: Some(first + i as u32),
                        other_ordinal: None,
                        address: client,
                        handle,
                        category: SUPPLY_TRANSFER,
                        owner: country.id,
                        other: 0,
                        route: key,
                        bad: status.bad,
                        convoys: status.convoys,
                        efficiency: status.efficiency,
                        convoys_base: status.convoys_base,
                        route_address: client + SUPPLY_ROUTE,
                    },
                )?;
            }
            if complete < live {
                let rec = &supply_records
                    [complete * SUPPLY_RECORD_STRIDE..(complete + 1) * SUPPLY_RECORD_STRIDE];
                if partial >= 1
                    && u64::from_le_bytes(rec[0..8].try_into().unwrap())
                        != lazy_vtable(&mut expected_route, self.base, ROUTE_VTABLE_RVA)?
                {
                    return Err("route vtable mismatch");
                }
                if partial >= 3
                    && u64::from_le_bytes(rec[12..20].try_into().unwrap())
                        != lazy_vtable(&mut expected_supply, self.base, SUPPLY_VTABLE_RVA)?
                {
                    return Err("convoy client of an unexpected type");
                }
                return Err(match partial {
                    0 => "route vtable unreadable",
                    1 => "convoy client handle unreadable",
                    2 => "convoy client vtable unreadable",
                    3 => "convoy client efficiency unreadable",
                    4 => "convoy client convoys unreadable",
                    _ => "convoy client base convoys unreadable",
                });
            }
            if let Some(reason) = ptr_stop {
                return Err(reason);
            }
            first += count as u32;
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
        let mut market_manager = None;
        let mut market_sellers = None;
        let mut market_buyers = None;
        let mut ptr_bytes = [0_u8; CLIENT_BATCH * 8];
        let mut entry_ptrs = [0_usize; CLIENT_BATCH];
        let mut handle_bytes = [0_u8; CLIENT_BATCH * 4];
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
                let manager = match market_manager {
                    Some(manager) => manager,
                    None => {
                        let manager = pointer(
                            self.memory,
                            at(self.game_state, cgame_state::MARKET_MANAGER)?,
                            "market order manager unreadable",
                        )?;
                        market_manager = Some(manager);
                        manager
                    }
                };
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
                let outer = if owner_side {
                    match market_sellers {
                        Some(outer) => outer,
                        None => {
                            let outer = array(
                                self.memory,
                                at(manager, MARKET_SELLERS)?,
                                MARKET_COUNTRY_STRIDE,
                                "market directional order table unreadable",
                            )?;
                            market_sellers = Some(outer);
                            outer
                        }
                    }
                } else {
                    match market_buyers {
                        Some(outer) => outer,
                        None => {
                            let outer = array(
                                self.memory,
                                at(manager, MARKET_BUYERS)?,
                                MARKET_COUNTRY_STRIDE,
                                "market directional order table unreadable",
                            )?;
                            market_buyers = Some(outer);
                            outer
                        }
                    }
                };
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
            let handle_layout = if category == MARKET {
                &CONTRACT_HANDLE_LAYOUT
            } else {
                &CLIENT_HANDLE_LAYOUT
            };
            let max_offset = if category == MARKET {
                CONTRACT_CLIENT + CLIENT_HANDLE_ID + 4
            } else {
                CLIENT_HANDLE_ID + 4
            };
            let mut apply_ordinal = |ordinal: u32, address: usize, handle: u32| -> Result<(), &'static str> {
                let Ok(at) = handles.binary_search_by_key(&handle, |&(handle, _)| handle) else {
                    return Ok(());
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
                Ok(())
            };
            let mut first = 0_u32;
            while first < list.count {
                let count = ((list.count - first) as usize).min(CLIENT_BATCH);
                let start = list
                    .element(first, 8)
                    .ok_or("convoy order entry unreadable")?;
                let copied_ptrs = self.memory.copy_records(
                    RecordSources::Strided {
                        base: start,
                        stride: 8,
                        count,
                    },
                    &ORIGIN_PTR_LAYOUT,
                    &mut ptr_bytes[..count * 8],
                );
                let mut live = 0;
                let mut ptr_stop = None;
                for i in 0..copied_ptrs {
                    let Ok(entry) = usize::try_from(u64::from_le_bytes(
                        ptr_bytes[i * 8..(i + 1) * 8].try_into().unwrap(),
                    )) else {
                        ptr_stop = Some("convoy order entry unreadable");
                        break;
                    };
                    if entry == 0 {
                        ptr_stop = Some("null convoy order entry");
                        break;
                    }
                    if entry.checked_add(max_offset).is_none() {
                        ptr_stop = Some("resources map address overflow");
                        break;
                    }
                    entry_ptrs[live] = entry;
                    live += 1;
                }
                if ptr_stop.is_none() && copied_ptrs < count {
                    ptr_stop = Some("convoy order entry unreadable");
                }
                let copied_handles = if live != 0 {
                    self.memory.copy_records(
                        RecordSources::Pointers(&entry_ptrs[..live]),
                        handle_layout,
                        &mut handle_bytes[..live * 4],
                    )
                } else {
                    0
                };
                for i in 0..copied_handles {
                    let address = if category == MARKET {
                        entry_ptrs[i] + CONTRACT_CLIENT
                    } else {
                        entry_ptrs[i]
                    };
                    let handle =
                        u32::from_le_bytes(handle_bytes[i * 4..(i + 1) * 4].try_into().unwrap());
                    apply_ordinal(first + i as u32, address, handle)?;
                }
                if copied_handles < live {
                    return Err("convoy client handle unreadable");
                }
                if let Some(reason) = ptr_stop {
                    return Err(reason);
                }
                first += count as u32;
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
        let mut expected_vtable = None;
        let mut expected_route = None;
        let mut ptr_bytes = [0_u8; CLIENT_BATCH * 8];
        let mut ptrs = [0_usize; CLIENT_BATCH];
        let mut records = [0_u8; CLIENT_BATCH * CONTRACT_RECORD_STRIDE];
        let mut route_ptrs = [0_usize; CLIENT_BATCH];
        let mut route_vtables = [0_u8; CLIENT_BATCH * 8];
        let mut first = 0_u32;
        while first < contracts.count {
            let count = ((contracts.count - first) as usize).min(CLIENT_BATCH);
            let start = contracts
                .element(first, 8)
                .ok_or("market contract unreadable")?;
            let copied_ptrs = memory.copy_records(
                RecordSources::Strided {
                    base: start,
                    stride: 8,
                    count,
                },
                &ORIGIN_PTR_LAYOUT,
                &mut ptr_bytes[..count * 8],
            );
            let mut live = 0;
            let mut ptr_stop = None;
            for i in 0..copied_ptrs {
                let Ok(contract) = usize::try_from(u64::from_le_bytes(
                    ptr_bytes[i * 8..(i + 1) * 8].try_into().unwrap(),
                )) else {
                    ptr_stop = Some("market contract unreadable");
                    break;
                };
                if contract == 0 {
                    ptr_stop = Some("null market contract in the owned list");
                    break;
                }
                if contract
                    .checked_add(CONTRACT_CLIENT + CONTRACT_CLIENT_ROUTE + 8)
                    .is_none()
                {
                    ptr_stop = Some("resources map address overflow");
                    break;
                }
                ptrs[live] = contract;
                live += 1;
            }
            if ptr_stop.is_none() && copied_ptrs < count {
                ptr_stop = Some("market contract unreadable");
            }
            let copied_fields = if live != 0 {
                memory.copy_records(
                    RecordSources::Pointers(&ptrs[..live]),
                    &CONTRACT_RECORD_LAYOUT,
                    &mut records[..live * CONTRACT_RECORD_STRIDE],
                )
            } else {
                0
            };
            let complete = copied_fields / CONTRACT_RECORD_FIELDS.len();
            let partial = copied_fields % CONTRACT_RECORD_FIELDS.len();
            let route_scan = complete + usize::from(complete < live && partial >= 2);
            let mut route_live = 0;
            for i in 0..route_scan {
                let rec = &records[i * CONTRACT_RECORD_STRIDE..(i + 1) * CONTRACT_RECORD_STRIDE];
                let Ok(route) = usize::try_from(u64::from_le_bytes(rec[8..16].try_into().unwrap()))
                else {
                    break;
                };
                if route == 0 {
                    break;
                }
                route_ptrs[route_live] = route;
                route_live += 1;
            }
            let copied_routes = if route_live != 0 {
                memory.copy_records(
                    RecordSources::Pointers(&route_ptrs[..route_live]),
                    &ORIGIN_PTR_LAYOUT,
                    &mut route_vtables[..route_live * 8],
                )
            } else {
                0
            };
            for i in 0..complete {
                let rec = &records[i * CONTRACT_RECORD_STRIDE..(i + 1) * CONTRACT_RECORD_STRIDE];
                let seller = self.id(u32::from_le_bytes(rec[0..4].try_into().unwrap()))?;
                let buyer = self.id(u32::from_le_bytes(rec[4..8].try_into().unwrap()))?;
                if i >= route_live {
                    return Err("client holds no route");
                }
                if i >= copied_routes {
                    return Err("route vtable unreadable");
                }
                if u64::from_le_bytes(route_vtables[i * 8..(i + 1) * 8].try_into().unwrap())
                    != lazy_vtable(&mut expected_route, self.base, ROUTE_VTABLE_RVA)?
                {
                    return Err("route vtable mismatch");
                }
                if u64::from_le_bytes(rec[16..24].try_into().unwrap())
                    != lazy_vtable(&mut expected_vtable, self.base, CONTRACT_CLIENT_VTABLE_RVA)?
                {
                    return Err("convoy client of an unexpected type");
                }
                let client = ptrs[i] + CONTRACT_CLIENT;
                let handle = u32::from_le_bytes(rec[24..28].try_into().unwrap());
                let status = ClientStatus::new(
                    [
                        i64::from_le_bytes(rec[28..36].try_into().unwrap()),
                        i64::from_le_bytes(rec[36..44].try_into().unwrap()),
                    ],
                    [
                        i32::from_le_bytes(rec[44..48].try_into().unwrap()),
                        i32::from_le_bytes(rec[48..52].try_into().unwrap()),
                    ],
                    i32::from_le_bytes(rec[52..56].try_into().unwrap()),
                    self.threshold,
                );
                let key = RouteKey {
                    class: DELIVERY_ROUTE_CLASS,
                    country: seller,
                    other: buyer,
                };
                table.push(
                    output,
                    ClientRow {
                        owner_ordinal: None,
                        other_ordinal: None,
                        address: client,
                        handle,
                        category: MARKET,
                        owner: seller,
                        other: buyer,
                        route: key,
                        bad: status.bad,
                        convoys: status.convoys,
                        efficiency: status.efficiency,
                        convoys_base: status.convoys_base,
                        route_address: route_ptrs[i],
                    },
                )?;
            }
            if complete < live {
                let rec = &records
                    [complete * CONTRACT_RECORD_STRIDE..(complete + 1) * CONTRACT_RECORD_STRIDE];
                if partial >= 1 {
                    let _ = self.id(u32::from_le_bytes(rec[0..4].try_into().unwrap()))?;
                    let _ = self.id(u32::from_le_bytes(rec[4..8].try_into().unwrap()))?;
                }
                if partial >= 2 {
                    if complete >= route_live {
                        return Err("client holds no route");
                    }
                    if complete >= copied_routes {
                        return Err("route vtable unreadable");
                    }
                    if u64::from_le_bytes(
                        route_vtables[complete * 8..(complete + 1) * 8]
                            .try_into()
                            .unwrap(),
                    ) != lazy_vtable(&mut expected_route, self.base, ROUTE_VTABLE_RVA)?
                    {
                        return Err("route vtable mismatch");
                    }
                }
                if partial >= 3
                    && u64::from_le_bytes(rec[16..24].try_into().unwrap())
                        != lazy_vtable(&mut expected_vtable, self.base, CONTRACT_CLIENT_VTABLE_RVA)?
                {
                    return Err("convoy client of an unexpected type");
                }
                return Err(match partial {
                    0 => "market contract parties unreadable",
                    1 => "client route unreadable",
                    2 => "convoy client vtable unreadable",
                    3 => "convoy client handle unreadable",
                    4 => "convoy client efficiency unreadable",
                    5 => "convoy client convoys unreadable",
                    _ => "convoy client base convoys unreadable",
                });
            }
            if let Some(reason) = ptr_stop {
                return Err(reason);
            }
            first += count as u32;
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
        let mut expected_origin_vtable = None;
        let mut expected_route_vtable = None;
        if rights_table.failed.is_none() || origins_table.failed.is_none() {
            match at(resources, ccountry_resources::RESOURCE_ORIGINS)
                .and_then(|address| array(memory, address, 8, "origin list unreadable"))
            {
                Ok(origins) => {
                    let mut first = 0_u32;
                    while first < origins.count {
                        if origins_table.failed.is_some() {
                            rights_table.run(|table| {
                                walk.read_rights_only(
                                    origins,
                                    first,
                                    &mut expected_origin_vtable,
                                    table,
                                    &mut outputs.rights,
                                )
                            });
                            break;
                        }
                        let count = ((origins.count - first) as usize).min(ORIGIN_BATCH);
                        let read_batch = walk.read_origin_batch(
                            origins,
                            8,
                            (first, count),
                            country.id,
                            &mut owner_canonical,
                            &mut expected_origin_vtable,
                            &mut batch,
                        );
                        let mut rights_resume = None;
                        rights_table.run(|table| {
                            for index in 0..read_batch.complete_origins {
                                let record = batch.origin_record(index);
                                if u64::from_le_bytes(record[0..8].try_into().unwrap())
                                    != lazy_vtable(
                                        &mut expected_origin_vtable,
                                        base,
                                        ORIGIN_VTABLE_RVA,
                                    )?
                                {
                                    return Err("origin vtable mismatch");
                                }
                                let state_ptr = usize::try_from(u64::from_le_bytes(
                                    record[8..16].try_into().unwrap(),
                                ))
                                .ok()
                                .filter(|&p| p != 0)
                                .ok_or("origin without a state")?;
                                let state = if index < read_batch.complete_states
                                    || (index == read_batch.complete_states
                                        && index < read_batch.state_live
                                        && read_batch.partial_state >= 1)
                                {
                                    u32::from_le_bytes(
                                        batch.state_record(index)[0..4].try_into().unwrap(),
                                    )
                                } else if index == read_batch.complete_states
                                    && index < read_batch.state_live
                                {
                                    return Err("origin state id unreadable");
                                } else {
                                    read::<u32>(memory, at(state_ptr, cstate::ID)?)
                                        .ok_or("origin state id unreadable")?
                                };
                                let rights = Array::from_header(
                                    record[16..32].try_into().unwrap(),
                                    RIGHTS_STRIDE,
                                )
                                .ok_or("origin rights unreadable")?;
                                walk.rights_from_array(rights, state, table, &mut outputs.rights)?;
                            }
                            if read_batch.complete_origins < read_batch.live {
                                let record = batch.origin_record(read_batch.complete_origins);
                                if read_batch.partial_origin == 0 {
                                    return Err("origin vtable unreadable");
                                }
                                if u64::from_le_bytes(record[0..8].try_into().unwrap())
                                    != lazy_vtable(
                                        &mut expected_origin_vtable,
                                        base,
                                        ORIGIN_VTABLE_RVA,
                                    )?
                                {
                                    return Err("origin vtable mismatch");
                                }
                                if read_batch.partial_origin == 1 {
                                    return Err("origin state unreadable");
                                }
                                let state_ptr = usize::try_from(u64::from_le_bytes(
                                    record[8..16].try_into().unwrap(),
                                ))
                                .ok()
                                .filter(|&p| p != 0)
                                .ok_or("origin without a state")?;
                                let state = read::<u32>(memory, at(state_ptr, cstate::ID)?)
                                    .ok_or("origin state id unreadable")?;
                                if read_batch.partial_origin == 2 {
                                    return Err("origin rights unreadable");
                                }
                                let rights = Array::from_header(
                                    record[16..32].try_into().unwrap(),
                                    RIGHTS_STRIDE,
                                )
                                .ok_or("origin rights unreadable")?;
                                walk.rights_from_array(rights, state, table, &mut outputs.rights)?;
                                rights_resume =
                                    Some(first + read_batch.complete_origins as u32 + 1);
                                return Ok(());
                            }
                            if let Some(reason) = read_batch.ptr_stop {
                                return Err(reason);
                            }
                            Ok(())
                        });
                        origins_table.run(|table| {
                            walk.finish_origins_batch(
                                &batch,
                                &read_batch,
                                0,
                                first,
                                country,
                                &mut owner_canonical,
                                &mut expected_origin_vtable,
                                &mut expected_route_vtable,
                                table,
                                &mut outputs.origins,
                            )
                        });
                        if rights_table.failed.is_some() && origins_table.failed.is_some() {
                            break;
                        }
                        first = rights_resume.unwrap_or(first + count as u32);
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
                let read_batch = walk.read_origin_batch(
                    origins,
                    RIGHTS_ORIGIN_STRIDE,
                    (first, count),
                    country.id,
                    &mut owner_canonical,
                    &mut expected_origin_vtable,
                    &mut batch,
                );
                walk.finish_origins_batch(
                    &batch,
                    &read_batch,
                    1,
                    first,
                    country,
                    &mut owner_canonical,
                    &mut expected_origin_vtable,
                    &mut expected_route_vtable,
                    table,
                    &mut outputs.origins,
                )?;
                first += count as u32;
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
        let mut route_paths = Vec::new();
        let mut cached_routes: Vec<(usize, Option<(usize, usize)>)> = Vec::new();
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
            let range = match cached_routes.binary_search_by_key(&route, |&(addr, _)| addr) {
                Ok(index) => cached_routes[index].1,
                Err(insert_at) => {
                    let start = route_paths.len();
                    let row =
                        read_route(memory, base, RouteKey::default(), route, None, &mut route_paths)?;
                    let range = if row.kind == NAVAL {
                        Some((start, route_paths.len()))
                    } else {
                        route_paths.truncate(start);
                        None
                    };
                    allocation::reserve(&mut cached_routes, 1)
                        .map_err(|_| "regional route cache allocation failed")?;
                    cached_routes.insert(insert_at, (route, range));
                    range
                }
            };
            let Some((start, end)) = range else {
                continue;
            };
            for country in countries.into_iter().filter(|&country| country != 0) {
                allocation::reserve(scratch, end - start)
                    .map_err(|_| "regional pair allocation failed")?;
                for &region in &route_paths[start..end] {
                    scratch.push((country, region));
                }
            }
        }
        if scratch.is_empty() {
            return Ok(());
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
        let mut expected = None;
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
            let copied_ptrs = memory.copy_records(
                RecordSources::Strided {
                    base: start,
                    stride: 8,
                    count,
                },
                &ORIGIN_PTR_LAYOUT,
                &mut ptr_bytes[..count * 8],
            );
            let mut live = 0_usize;
            let mut ptr_stop = None;
            for offset in 0..copied_ptrs {
                let Ok(pointer) = usize::try_from(u64::from_le_bytes(
                    ptr_bytes[offset * 8..(offset + 1) * 8].try_into().unwrap(),
                )) else {
                    ptr_stop = Some("strategic region roster slot unreadable");
                    break;
                };
                if pointer == 0 {
                    continue;
                }
                if at(pointer, region::ID).is_err() || at(pointer, region::ROUTE_WEIGHT).is_err() {
                    ptr_stop = Some("resources map address overflow");
                    break;
                }
                pointers[live] = pointer;
                live += 1;
            }
            if ptr_stop.is_none() && copied_ptrs < count {
                ptr_stop = Some("strategic region roster slot unreadable");
            }
            if live != 0 {
                let copied = memory.copy_records(
                    RecordSources::Pointers(&pointers[..live]),
                    &LAYOUT,
                    &mut records[..live * 16],
                );
                let complete = copied / FIELDS.len();
                let partial = copied % FIELDS.len();
                for index in 0..complete {
                    let rec = &records[index * 16..(index + 1) * 16];
                    if u64::from_le_bytes(rec[..8].try_into().unwrap())
                        != lazy_vtable(&mut expected, base, region::VTABLE_RVA)?
                    {
                        return Err("strategic region vtable mismatch");
                    }
                    push(
                        &mut output,
                        &mut len,
                        &mut unread,
                        RegionWeightRow {
                            region: u32::from_le_bytes(rec[8..12].try_into().unwrap()),
                            weight: i32::from_le_bytes(rec[12..16].try_into().unwrap()),
                        },
                    )?;
                }
                if complete < live {
                    let rec = &records[complete * 16..(complete + 1) * 16];
                    if partial >= 1
                        && u64::from_le_bytes(rec[..8].try_into().unwrap())
                            != lazy_vtable(&mut expected, base, region::VTABLE_RVA)?
                    {
                        return Err("strategic region vtable mismatch");
                    }
                    return Err("strategic region route weight unreadable");
                }
            }
            if let Some(reason) = ptr_stop {
                return Err(reason);
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
                sorted_values.resize(
                    sorted_keys.len(),
                    engine_native_memory::modifier_cache::Value::default(),
                );
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
        let mut spans = batch;
        for index in 0..batch {
            if elements.element(first + index as u32, 8).is_none() {
                spans = index;
                break;
            }
        }
        let copied = if spans != 0
            && let Some(start) = elements.element(first, 8)
        {
            memory.copy_records(
                RecordSources::Strided {
                    base: start,
                    stride: 8,
                    count: spans,
                },
                &ORIGIN_PTR_LAYOUT,
                &mut raw[..spans * 8],
            )
        } else {
            0
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
