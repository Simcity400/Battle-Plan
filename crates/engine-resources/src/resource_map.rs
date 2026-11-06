//! What the resources map mode and the trade-route lines read: each resource origin's
//! icon operands, resource rights, the oil factor, temporary state resources, every
//! convoy client a route line draws for, and the routes themselves.
//!
//! The icon reads an origin's stored efficiency product; a route line reads only whether
//! its client's efficiency product (A × B, fixed5, truncated) is below the live
//! `TRADE_ROUTE_BAD_EFFICIENCY_THRESHOLD`, which picks the bad-route colour.
//!
//! Every convoy client reaches its route through its own type: an origin and a supply
//! transfer embed theirs; an exchange, a lend-lease exchange and a market contract use
//! the exporter's delivery route for the importer. Routes are named by who holds them
//! ([`RouteKey`]): an origin's by its list owner and state, a delivery route by its
//! exporter and importer, a supply transfer's by its country and client handle.

use crate::{
    Memory, allocation, ccountry, ccountry_resources, cgame_state,
    cpdx_array::Array,
    cresource_exchange, cstate, cstrategic_resource,
    memory::{RecordField, RecordLayout, RecordSources},
    modifier_accumulator, read,
    walk::{Buffer, Outcome},
};
use engine_native::modifier_definition;

/// A present key's successful metadata is shared only within this parked
/// walk. Values and presence remain current per accumulator; failed metadata
/// is never retained, and absent keys demand none. Stack storage cannot add
/// an allocation refusal to the original getter.
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
    // An existing pass memo owns its observation and allocation verdicts.
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

/// The live bad-efficiency threshold define, i64 fixed5.
pub const BAD_EFFICIENCY_THRESHOLD_RVA: usize = 0x33e_3598;

/// A convoy client's handle (u32 type, u32 id) and its two efficiency factors.
/// The type word is minted from the live dynamic-type counter, not a class tag.
#[cfg(test)]
pub const CLIENT_HANDLE_TYPE: usize = 0x08;
pub const CLIENT_HANDLE_ID: usize = 0x0c;
/// Its assigned and required convoy counts, i32.
pub const CLIENT_CONVOYS: usize = 0x28;
pub const CLIENT_EFFICIENCY_A: usize = 0x48;
#[cfg(test)]
pub const CLIENT_EFFICIENCY_B: usize = 0x50;

/// Resource-origin client: the embedded route.
pub const ORIGIN_VTABLE_RVA: usize = 0x29d_8dc0;
pub const ORIGIN_ROUTE: usize = 0x350;
pub const ORIGIN_STATE: usize = 0x80;
/// The origin's efficiency product, which the resource icon reads.
pub const ORIGIN_EFFICIENCY: usize = 0x348;
/// The supply-route factor, written at load and construction.
pub const ORIGIN_ROUTE_FACTOR: usize = 0x3d8;
/// Sorted resource rights: 8-byte entries {i32 resource index, u32 holder link}.
pub const ORIGIN_RIGHTS: usize = 0x3e0;
pub const RIGHTS_STRIDE: usize = 8;
/// Rights-holder origins: 16-byte entries, the origin pointer first.
pub const RIGHTS_ORIGINS: usize = 0x7b8;
pub const RIGHTS_ORIGIN_STRIDE: usize = 0x10;
/// Resource-exchange client ([`cresource_exchange`]).
pub const EXCHANGE_VTABLE_RVA: usize = cresource_exchange::VTABLE_RVA;
/// A country's delivery routes, one per canonical importer slot.
pub const DELIVERY_ROUTES: usize = 0x788;
/// Lend-lease client: its route pointer and receiver.
pub const LEND_LEASE_VTABLE_RVA: usize = 0x2a9_9cf8;
pub const LEND_LEASE_ROUTE: usize = 0xc0;
pub const LEND_LEASE_RECEIVER: usize = 0xb8;
pub const LEND_LEASE_GIVEN: usize = 0x758;
/// The receiver's independent lend-lease list.
pub const LEND_LEASE_RECEIVED: usize = 0x770;
/// Country market wrapper's authenticated manager pointer.
const MARKET_STATUS_MANAGER: usize = 0x70;
/// Per-country seller and buyer contract-vector tables.
const MARKET_SELLERS: usize = 0x30;
const MARKET_BUYERS: usize = 0x48;
const MARKET_COUNTRY_STRIDE: usize = 0x18;
/// Market-contract client, embedded in the contract: its route pointer.
pub const CONTRACT_CLIENT: usize = 0xf0;
pub const CONTRACT_CLIENT_VTABLE_RVA: usize = 0x29e_8330;
pub const CONTRACT_CLIENT_ROUTE: usize = 0xc8;
pub const CONTRACT_SELLER: usize = 0x58;
#[cfg(test)]
pub const CONTRACT_BUYER: usize = 0x5c;
pub const MARKET_OWNED_CONTRACTS: usize = 0;
/// Supply-transfer client: the embedded route.
pub const SUPPLY_VTABLE_RVA: usize = 0x2a1_eb80;
pub const SUPPLY_ROUTE: usize = 0xa0;
pub const SUPPLY_COUNTRY_SYSTEMS: usize = 0x108;
pub const SUPPLY_TRANSFERS: usize = 0xd0;
pub const SUPPLY_TRANSFER_STRIDE: usize = 40;
pub const SUPPLY_TRANSFER_CLIENT: usize = 0x20;

/// Route object.
pub const ROUTE_VTABLE_RVA: usize = 0x29d_8d70;
pub const ROUTE_KIND: usize = 0x08;
pub const ROUTE_FROM: usize = 0x10;
pub const ROUTE_TO: usize = 0x18;
pub const ROUTE_FROM_PORT: usize = 0x20;
pub const ROUTE_TO_PORT: usize = 0x28;
/// The route's sender, receiver and convoy-owner country links.
pub const ROUTE_LINKS: [usize; 3] = [0x30, 0x34, 0x38];
pub const ROUTE_LAND_STATES: usize = 0x48;
pub const ROUTE_SEA_REGIONS: usize = 0x60;
/// Naval routes draw their ports and sea regions; other kinds their states.
pub const NAVAL: u8 = 2;
const PROVINCE_ID: usize = 0xa4;
const REGION_ID: usize = 0x58;

/// State temporary-resource gates.
pub const STATE_TEMPORARY_LIST_COUNT: usize = 0x8bc;
pub const STATE_TEMPORARY_FLAG: usize = 0x1b5;

/// Modifier keys: `local_resources_factor` for (state, country) and the country's
/// `production_oil_factor`.
pub const LOCAL_RESOURCES_FACTOR: u32 = 0x39;
pub const PRODUCTION_OIL_FACTOR: u32 = 0xdd;

/// Client categories, the route lines' layers.
pub const SUPPLY_TRANSFER: u8 = 1;
pub const IMPORT: u8 = 2;
pub const LEND_LEASE: u8 = 4;
pub const MARKET: u8 = 6;

/// Route classes ([`RouteKey`]).
pub const ORIGIN_ROUTE_CLASS: u8 = 0;
pub const DELIVERY_ROUTE_CLASS: u8 = 1;
pub const SUPPLY_ROUTE_CLASS: u8 = 2;

const ONE: i128 = 100_000;

/// Who holds a route: `country` is the owner of its sliced rows.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RouteKey {
    pub class: u8,
    pub country: u32,
    pub other: u32,
}

/// One resource origin: `list` 0 is the controller's own, 1 a rights holder's.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct OriginRow {
    pub state: u32,
    /// The country whose list holds it (raw country id).
    pub country: u32,
    pub list: u8,
    pub ordinal: u32,
    pub efficiency: i64,
    pub route_factor: i64,
    pub local_resources_factor: i64,
    /// Its line is the bad colour: its client's efficiency product below the threshold.
    pub bad: bool,
    /// Assigned and required convoys.
    pub convoys: [i32; 2],
    /// Where its route lives this hour; never published.
    pub route_address: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RightsRow {
    pub ordinal: u32,
    pub state: u32,
    /// One-based resource database index.
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
    /// The resource's lexer token.
    pub resource: u32,
    pub amount: i64,
}

/// One convoy client other than an origin.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ClientRow {
    /// Native directional list positions, independently for both endpoint countries.
    pub owner_ordinal: Option<u32>,
    pub other_ordinal: Option<u32>,
    /// Authenticated client address used to join directional lists; never published.
    pub address: usize,
    /// The client's handle id.
    pub handle: u32,
    pub category: u8,
    pub owner: u32,
    pub other: u32,
    pub route: RouteKey,
    pub bad: bool,
    /// Assigned and required convoys.
    pub convoys: [i32; 2],
    /// Where its route lives this hour; never published.
    pub route_address: usize,
}

/// One route's drawn structure: ports and sea regions for a naval route, end states and
/// land states otherwise; no ends and no path when it draws nothing.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RouteRow {
    pub key: RouteKey,
    pub kind: u8,
    pub from: u32,
    pub to: u32,
    /// Native state endpoints, acquired only for a market-referenced route.
    pub source_state: Option<u32>,
    pub destination_state: Option<u32>,
    /// Range in the path arena.
    pub path_start: usize,
    pub path_len: usize,
}

/// A country's delivery efficiency in one strategic sea region, fixed5: where a bad route
/// line draws its hotspot colour for a viewer from that country.
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

fn vtable(base: usize, rva: usize) -> Result<usize, &'static str> {
    base.checked_add(rva).ok_or("module address overflow")
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

/// The live threshold that turns a route line the bad colour.
pub fn bad_efficiency_threshold(memory: &impl Memory, base: usize) -> Result<i64, &'static str> {
    read::<i64>(memory, at(base, BAD_EFFICIENCY_THRESHOLD_RVA)?)
        .ok_or("bad-efficiency threshold unreadable")
}

/// A client's or exchange's efficiency: the product of its two fixed5
/// factors, widened and truncated toward zero, as the game compares it.
pub fn efficiency_product(a: i64, b: i64) -> i128 {
    i128::from(a) * i128::from(b) / ONE
}

/// Whether the client's line is the bad colour: its efficiency product below `threshold`.
pub fn client_bad(
    memory: &impl Memory,
    client: usize,
    threshold: i64,
) -> Result<bool, &'static str> {
    let factors = pair_i64(memory, at(client, CLIENT_EFFICIENCY_A)?)
        .ok_or("convoy client efficiency unreadable")?;
    Ok(efficiency_product(factors[0], factors[1]) < i128::from(threshold))
}

/// The client's assigned and required convoys.
fn client_convoys(memory: &impl Memory, client: usize) -> Result<[i32; 2], &'static str> {
    let [assigned, required] =
        pair_u32(memory, at(client, CLIENT_CONVOYS)?).ok_or("convoy client convoys unreadable")?;
    Ok([assigned as i32, required as i32])
}

fn client_status(
    memory: &impl Memory,
    client: usize,
    threshold: i64,
) -> Result<(bool, [i32; 2]), &'static str> {
    let factors = client
        .checked_add(CLIENT_EFFICIENCY_A)
        .filter(|at| at.checked_add(16).is_some());
    let convoys = client
        .checked_add(CLIENT_CONVOYS)
        .filter(|at| at.checked_add(8).is_some());
    let (Some(factors), Some(convoys)) = (factors, convoys) else {
        let bad = client_bad(memory, client, threshold)?;
        return Ok((bad, client_convoys(memory, client)?));
    };
    let mut factor_bytes = [0; 16];
    let mut convoy_bytes = [0; 8];
    match memory.copy_scattered(
        [factors, convoys],
        &mut [factor_bytes.as_mut_slice(), convoy_bytes.as_mut_slice()],
    ) {
        0 => return Err("convoy client efficiency unreadable"),
        1 => return Err("convoy client convoys unreadable"),
        _ => {}
    }
    let a = i64::from_le_bytes(factor_bytes[..8].try_into().expect("eight bytes"));
    let b = i64::from_le_bytes(factor_bytes[8..].try_into().expect("eight bytes"));
    Ok((
        efficiency_product(a, b) < i128::from(threshold),
        [
            i32::from_le_bytes(convoy_bytes[..4].try_into().expect("four bytes")),
            i32::from_le_bytes(convoy_bytes[4..].try_into().expect("four bytes")),
        ],
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

/// The client's handle id. Its vtable and list membership authenticate it.
fn client_handle(memory: &impl Memory, client: usize) -> Result<u32, &'static str> {
    read(memory, at(client, CLIENT_HANDLE_ID)?).ok_or("convoy client handle unreadable")
}

/// A delivery route: the exporter's route for the importer's canonical slot.
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
}

impl<M: Memory> Walk<'_, M> {
    fn id(&self, link: u32) -> Result<u32, &'static str> {
        Ok(self.countries.resolve(link)?.id)
    }

    fn country_resources(&self, country: ccountry::Country) -> Result<usize, &'static str> {
        crate::ccountry_resources::resources(self.memory, self.base, self.countries, country)
    }

    /// An import exchange's route and its exporter: the exchange authenticated as the walked
    /// importer's ([`cresource_exchange::authenticate`]), and the exporter's delivery route for
    /// the importer's canonical slot.
    fn exchange(
        &self,
        client: usize,
        importer: ccountry::Country,
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

    /// A pointer-held route, authenticated as a route object.
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
        expected: usize,
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
        let (bad, convoys) = client_status(self.memory, client, self.threshold)?;
        Ok(ClientRow {
            owner_ordinal: None,
            other_ordinal: None,
            address: client,
            handle,
            category,
            owner,
            other,
            route: route.0,
            bad,
            convoys,
            route_address: route.1,
        })
    }

    fn origin(
        &self,
        origin: usize,
        list: u8,
        ordinal: u32,
        country: ccountry::Country,
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
        // A rights-holder origin carries its own route to the holder, as an own origin does.
        let route_address = at(origin, ORIGIN_ROUTE)?;
        self.authenticate_route(route_address)?;
        let (bad, convoys) = client_status(memory, origin, self.threshold)?;
        Ok((
            OriginRow {
                state,
                country: country.id,
                list,
                ordinal,
                efficiency,
                route_factor,
                local_resources_factor,
                bad,
                convoys,
                route_address,
            },
            state_pointer,
        ))
    }
}

impl<M: Memory> Walk<'_, M> {
    /// An own origin's resource rights.
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

    /// A country's imports (each exchange draws the exporter's delivery route for this
    /// importer), the lend-lease it gives (its delivery route for the receiver) and its naval
    /// supply transfers (each client embeds its route; the supply system's country table is
    /// indexed by canonical slot).
    fn country_clients(
        &self,
        country: ccountry::Country,
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

    /// Read each reached endpoint list once. Match authenticated client addresses rather
    /// than inferring its position from a handle or the global owning contract vector.
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

    /// Market contracts: the seller's delivery route for the buyer.
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

/// The tables [`walk_buffer`] fills, as indexes into its per-table state.
const ORIGINS: usize = 0;
const RIGHTS: usize = 1;
const OIL: usize = 2;
const CLIENTS: usize = 3;

/// One table's rows so far, or why it is unavailable.
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

    /// Runs `read` unless the table has failed, keeping its failure.
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

/// Reads every country's origins (own and rights-holder), their rights, each country's
/// oil factor and every convoy client a route line draws for: imports, lend-lease given,
/// supply transfers and market contracts. Each table has its own outcome: a read only one
/// table needs refuses that table alone. Rights read only their own origin identities,
/// independently of origin efficiencies and routes.
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
    // Only the bad bits read the threshold: the origins' and the clients'.
    let threshold = bad_efficiency_threshold(memory, base);
    if let Err(reason) = threshold {
        tables[ORIGINS].failed = Some(reason);
        tables[CLIENTS].failed = Some(reason);
    }
    let walk = Walk {
        memory,
        base,
        game_state,
        countries: &countries,
        threshold: threshold.unwrap_or_default(),
        local_resources: Default::default(),
    };
    let oil_metadata = Default::default();
    // The supply transfers' country systems, which only the clients read.
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
                at(country.pointer, ccountry::MODIFIER_ACCUMULATOR)?,
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
        rights_table.run(|table| {
            let origins = array(
                memory,
                at(resources, ccountry_resources::RESOURCE_ORIGINS)?,
                8,
                "origin list unreadable",
            )?;
            for index in 0..origins.count {
                let origin = element(memory, origins, index, 8, "origin unreadable")?;
                if origin == 0 {
                    return Err("null origin in a live list");
                }
                if pointer(memory, origin, "origin vtable unreadable")?
                    != vtable(base, ORIGIN_VTABLE_RVA)?
                {
                    return Err("origin vtable mismatch");
                }
                let state = pointer(memory, at(origin, ORIGIN_STATE)?, "origin state unreadable")?;
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
            for (list, offset, stride) in [
                (0_u8, ccountry_resources::RESOURCE_ORIGINS, 8),
                (1, RIGHTS_ORIGINS, RIGHTS_ORIGIN_STRIDE),
            ] {
                let origins = array(
                    memory,
                    at(resources, offset)?,
                    stride,
                    "origin list unreadable",
                )?;
                for index in 0..origins.count {
                    let origin = element(memory, origins, index, stride, "origin unreadable")?;
                    if origin == 0 {
                        return Err("null origin in a live list");
                    }
                    let (row, _) = walk.origin(origin, list, index, country)?;
                    table.push(&mut outputs.origins, row)?;
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

/// The regional delivery efficiency of every country that sees a bad naval route line, in
/// each sea region of that route: a bad line's region draws its hotspot colour when the
/// viewing country's efficiency there is below the regional threshold. A route's line is
/// drawn for each country taking part (an origin's country, an exchange's importer and
/// exporter, a lend-lease giver and receiver, a contract's seller and buyer, a supply
/// transfer's country). `scratch` holds the (country, region) pairs; a region outside the
/// country's records is full, as the game takes it.
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
            let navy = crate::cstrategic_navy_manager::navy(memory, base, game_state, resolved)?;
            let records =
                crate::cstrategic_navy::regional_convoys(memory, base, game_state, navy, resolved)?;
            for &(_, region) in &scratch[start..end] {
                let efficiency =
                    crate::cstrategic_navy::regional_convoy_efficiency(memory, records, region)?;
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

/// Every state's temporary resources: each resource's temporary key in the state's base
/// modifiers, for a state whose temporary-resource list is not empty or whose flag is set;
/// a zero amount has no row. `descriptors` is this hour's complete resource census.
///
/// The state pointers are copied 64 at a time and their two gate fields gathered in one
/// batch; each resource's key is read once, at the first gated state that needs it. A
/// batch that faults or whose fields would overflow is read state by state, so every
/// fault keeps its message and its place.
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
        let mut keys = Vec::new();
        allocation::reserve(&mut keys, descriptors.len())
            .map_err(|_| "resource temporary key allocation failed")?;
        let mut gated = |state: usize, count: i32, flag: u8| -> Result<(), &'static str> {
            if count == 0 && flag == 0 {
                return Ok(());
            }
            let id = read::<u32>(memory, at(state, cstate::ID)?).ok_or("state id unreadable")?;
            let accumulator = at(state, cstate::BASE_MODIFIERS)?;
            for (index, descriptor) in descriptors.iter().enumerate() {
                let key = match keys.get(index) {
                    Some(&key) => key,
                    None => {
                        let key = read::<u32>(
                            memory,
                            at(descriptor.address, cstrategic_resource::TEMPORARY_KEY)?,
                        )
                        .ok_or("resource temporary key unreadable")?;
                        keys.push(key);
                        key
                    }
                };
                if key == 0 {
                    continue;
                }
                let amount =
                    modifier_accumulator::lookup_effective_at(memory, base, accumulator, key)?
                        .effective;
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
            // Both copies stop at their first fault; no faulted source is read again.
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
                    // Past an overflowing pointer: read as the serial walk does, never gathered.
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

/// The route at `address`, held under `key`, with its path in `path` from `path.len()`.
/// Market demand supplies the game state to authenticate the native endpoint states;
/// other consumers read only the drawn structure.
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

/// Each path element's id, 64 elements per scattered pointer copy and record batch. Every
/// source is read once: a batch stops at its first fault, null or overflowing element, and
/// the ids before it are kept, as a serial walk keeps them.
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
    while first < elements.count {
        let batch = ((elements.count - first) as usize).min(BATCH);
        let mut addresses = [0_usize; BATCH];
        let mut words = [[0_u8; 8]; BATCH];
        // An element outside the array ends the batch before it.
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
        let mut destinations: [&mut [u8]; BATCH] = std::array::from_fn(|_| <&mut [u8]>::default());
        for (destination, word) in destinations.iter_mut().zip(words[..spans].iter_mut()) {
            *destination = word.as_mut_slice();
        }
        let copied = memory
            .copy_scattered(addresses, &mut destinations)
            .min(spans);
        let mut items = [0_usize; BATCH];
        let mut stop = None;
        let mut live = 0;
        for (index, word) in words[..copied].iter().enumerate() {
            let item = u64::from_le_bytes(*word) as usize;
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
        let mut ids = [0_u8; BATCH * 4];
        let read = memory.copy_records(
            RecordSources::Pointers(&items[..live]),
            &id_layout,
            &mut ids[..live * 4],
        );
        for id in ids[..read.min(live) * 4].chunks_exact(4) {
            allocation::reserve(path, 1).map_err(|_| "route path allocation failed")?;
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
