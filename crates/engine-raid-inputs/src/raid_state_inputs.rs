//! Stored operands of state-scope raid target triggers.
//!
//! Resource amounts combine the template amount, state's own pool, three
//! resource accumulator keys and the global factor. A controller origin for
//! the state also selects infrastructure and supply-route factors. Healthy
//! building levels add grants adjusted by the controller's overrides.
//! These values are computed on demand; this reader acquires their operands.
//! Dynamic-modifier checks read the state's stored modifier container.

use crate::{
    Memory, cbuilding, cbuilding_database, cbuilding_status, cbuilding_template, ccountry,
    ccountry_resources, ccountry_supply_system, cdynamic_modifier, cdynamic_modifier_container,
    cgame_state,
    cpdx_array::Array,
    cprovince, cresource_origin, cstate, cstrategic_resource, cstrategic_resource_database,
    cstrategic_resource_pool, csupply_system, dynamic_modifier_entry,
    memory::{RecordField, RecordLayout, RecordSources},
    modifier_accumulator::{self, Value},
    modifier_definition::{DefinitionMemo, effective_memo},
    modifier_entry, read, read_slice, supply_companion, supply_node,
    walk::{Buffer, Outcome},
};

/// The global resource factor key.
/// Fixed built-in keys are checked once per attachment by
/// `fixed_layout`; a mismatch withholds the tables that use them.
pub const GLOBAL_FACTOR_KEY: u32 = crate::modifier_keys::STATE_RESOURCES_FACTOR;
/// The country infrastructure resource factor key.
pub const INFRASTRUCTURE_KEY: u32 =
    crate::modifier_keys::LOCAL_RESOURCE_GAIN_EFFICIENCY_PER_INFRASTRUCTURE;

/// Infrastructure building template pointer in the building database.
const INFRASTRUCTURE_TEMPLATE: usize = 0x3b0;

/// One state's operands shared by every resource.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct InputRow {
    pub state: u32,
    /// `cstate::RESOURCE_MODIFIER_GATE`: the additive keys count only when set.
    pub gate: u8,
    /// Effective key 0x223 in `cstate::BASE_MODIFIERS`.
    pub global_factor: i64,
    /// The controller has a resource origin whose own link identifies this state; without one both
    /// factors are one.
    pub origin: u8,
    /// The infrastructure building's level, damage included, read only with an origin; `None` when
    /// the state has no such building.
    pub infrastructure_level: Option<i32>,
    /// Effective controller key 0x292, read only with an infrastructure level;
    /// zero otherwise.
    pub infrastructure_modifier: i64,
    /// Whether the maximum state province-node value is positive, read only with an origin.
    pub supply_route: u8,
}

/// A resource's stored operands in one state; an all-zero resource has no row.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ResourceRow {
    pub state: u32,
    /// The strategic resource's lexer token.
    pub resource: u32,
    /// The state's resource record, fixed5.
    pub pool: i64,
    /// Effective accumulator values of the resource's three keys.
    pub additive: i64,
    pub factor: i64,
    pub cost: i64,
}

/// One `cstate::DYNAMIC_MODIFIERS` entry, in container order.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ModifierRow {
    pub state: u32,
    pub ordinal: u32,
    pub scope_country: u32,
    pub scope_state: i32,
    pub token_start: usize,
    pub token_len: usize,
}

/// One element of a controller's country-resources origin entry. An entry with no elements has one
/// row with `count` and `resource` zero.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct OverrideRow {
    /// The country's native id.
    pub country: u32,
    pub ordinal: u32,
    pub entry: u32,
    /// The building type token the entry applies to.
    pub building: u32,
    /// The entry's element count.
    pub count: u32,
    /// The resource's lexer token.
    pub resource: u32,
    pub amount: i64,
}

/// One resource database item: its lexer token and descriptor address.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Resource {
    token: u32,
    address: usize,
    /// The state keys copied; they are checked after every identity.
    keys_read: bool,
}

/// The database's item and key array headers (`cstrategic_resource_database::Reader::new`).
const DATABASE_FIELDS: [RecordField; 2] = [
    field(cstrategic_resource_database::ITEMS, 0, 16),
    field(cstrategic_resource_database::KEYS, 16, 16),
];
const DATABASE_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&DATABASE_FIELDS, 32).expect("valid record layout");
/// A descriptor's vtable, one-based index, token and three state keys, in
/// the order `cstrategic_resource::row` meets them.
const DESCRIPTOR_FIELDS: [RecordField; 6] = [
    field(0, 0, 8),
    field(cstrategic_resource::INDEX, 8, 4),
    field(cstrategic_resource::TOKEN_ID, 12, 4),
    field(cstrategic_resource::STATE_ADDITIVE_KEY, 16, 4),
    field(cstrategic_resource::STATE_FACTOR_KEY, 20, 4),
    field(cstrategic_resource::MODIFIER_KEY, 24, 4),
];
const DESCRIPTOR_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&DESCRIPTOR_FIELDS, 28).expect("valid record layout");
const ITEM_FIELDS: [RecordField; 1] = [field(0, 0, 8)];
const ITEM_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&ITEM_FIELDS, 8).expect("valid record layout");
const KEY_FIELDS: [RecordField; 1] = [field(0, 0, 4)];
const KEY_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&KEY_FIELDS, 4).expect("valid record layout");

/// The resource database as `cstrategic_resource::walk` authenticates it and
/// each resource's state keys, 64 items, keys and descriptors per guarded
/// call instead of eight guarded calls per resource. Identity errors keep
/// their messages and item order; the caller meets each item's key fault
/// after every identity, as the walk read them.
/// `Ok(Err(demand))` is a database larger than the buffers.
fn resource_database(
    memory: &impl Memory,
    base: usize,
    output: &mut Vec<Resource>,
    keys: &mut Vec<[u32; 3]>,
    bytes: &mut Vec<u8>,
    growing: bool,
) -> Result<Result<usize, usize>, &'static str> {
    let database = read::<u64>(
        memory,
        address(base, cstrategic_resource_database::SINGLETON_RVA)?,
    )
    .and_then(|pointer| usize::try_from(pointer).ok())
    .filter(|&pointer| pointer != 0)
    .ok_or("resource database pointer unreadable or null")?;
    let mut headers = [0_u8; 32];
    let fields = memory.copy_records(
        RecordSources::Pointers(&[database]),
        &DATABASE_LAYOUT,
        &mut headers,
    );
    let items = (fields >= 1)
        .then(|| Array::from_header(header16(&headers, 0), 8))
        .flatten()
        .ok_or("resource database items unreadable or invalid")?;
    let tokens = (fields >= 2)
        .then(|| Array::from_header(header16(&headers, 16), 4))
        .flatten()
        .ok_or("resource database keys unreadable or invalid")?;
    if items.count == 0 || tokens.count != items.count - 1 {
        return Err("resource database item and key counts disagree");
    }
    let count = items.count as usize;
    // Resource accumulation reaches every database item.
    grow(output, count - 1, growing)?;
    grow(keys, count - 1, growing)?;
    grow(bytes, BATCH * DESCRIPTOR_LAYOUT.stride(), growing)?;
    if count - 1 > output.len() {
        return Ok(Err(count - 1));
    }
    // Item pointers and key tokens, each a fault of its own.
    let mut pointers = [0_usize; BATCH];
    let mut sources = [0_usize; BATCH];
    let mut first = 0;
    while first < count {
        let len = (count - first).min(BATCH);
        for (index, source) in sources[..len].iter_mut().enumerate() {
            *source = items
                .element((first + index) as u32, 8)
                .ok_or("resource descriptor pointer unreadable or null")?;
        }
        let mut item_fields = [0_usize; BATCH];
        each_record(
            memory,
            &sources[..len],
            &ITEM_LAYOUT,
            bytes,
            |row, fields, record| {
                item_fields[row] = fields;
                pointers[row] = u64_at(record, 0) as usize;
            },
        );
        for index in 0..len {
            let item = first + index;
            let pointer = (item_fields[index] == 1)
                .then_some(pointers[index])
                .filter(|&pointer| pointer != 0);
            if item == 0 {
                pointer.ok_or("resource database has no null object")?;
                continue;
            }
            output[item - 1] = Resource {
                token: 0,
                address: pointer.unwrap_or(0),
                keys_read: false,
            };
        }
        first += len;
    }
    // Identity in item order, then the keys, as the walk met them.
    let mut descriptor_fields = [0_usize; BATCH];
    let mut records = [[0_u8; 28]; BATCH];
    let mut key_fields = [0_usize; BATCH];
    let mut key_words = [0_u32; BATCH];
    let mut first = 0;
    while first < count - 1 {
        let len = (count - 1 - first).min(BATCH);
        let mut readable = len;
        for (index, source) in sources[..len].iter_mut().enumerate() {
            *source = output[first + index].address;
            if *source == 0 {
                readable = readable.min(index);
            }
        }
        each_record(
            memory,
            &sources[..readable],
            &DESCRIPTOR_LAYOUT,
            bytes,
            |row, fields, record| {
                descriptor_fields[row] = fields;
                records[row] = record.try_into().unwrap();
            },
        );
        for (index, source) in sources[..len].iter_mut().enumerate() {
            *source = tokens
                .element((first + index) as u32, 4)
                .ok_or("resource database token unreadable")?;
        }
        each_record(
            memory,
            &sources[..len],
            &KEY_LAYOUT,
            bytes,
            |row, fields, record| {
                key_fields[row] = fields;
                key_words[row] = u32_at(record, 0);
            },
        );
        let vtable = address(base, cstrategic_resource::VTABLE_RVA)? as u64;
        for index in 0..len {
            let item = first + index + 1;
            if index >= readable {
                return Err("resource descriptor pointer unreadable or null");
            }
            let (fields, record) = (descriptor_fields[index], &records[index]);
            if fields == 0 || u64_at(record, 0) != vtable {
                return Err("resource descriptor vtable mismatch");
            }
            let id = u32_at(record, 8) as i32;
            if fields < 2 || id <= 0 {
                return Err("resource index unreadable or invalid");
            }
            if fields < 3 {
                return Err("resource token unreadable");
            }
            let token = u32_at(record, 12);
            if key_fields[index] == 0 {
                return Err("resource database token unreadable");
            }
            if id as usize != item || token != key_words[index] {
                return Err("resource descriptor disagrees with database identity");
            }
            output[item - 1].token = token;
        }
        for index in 0..len {
            let (fields, record) = (descriptor_fields[index], &records[index]);
            // A key fault is met only after every identity, at its own item.
            output[first + index].keys_read = fields == DESCRIPTOR_FIELDS.len();
            keys[first + index] = [u32_at(record, 16), u32_at(record, 20), u32_at(record, 24)];
        }
        first += len;
    }
    Ok(Ok(count - 1))
}

/// Row and scratch sizes; a walk reports the live sizes it needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capacity {
    pub states: usize,
    pub resources: usize,
    pub resource_rows: usize,
    pub modifier_rows: usize,
    pub token_bytes: usize,
    pub override_rows: usize,
    /// Origins read in one batch of countries.
    pub origins: usize,
    /// Route pairs held for one batch of countries.
    pub routes: usize,
    /// Provinces read in one batch of states.
    pub provinces: usize,
    /// Override entries of one country.
    pub entries: usize,
    /// Country slots and links.
    pub countries: usize,
    /// Distinct dynamic modifier definitions whose names a walk keeps.
    pub definitions: usize,
    pub name_bytes: usize,
    /// Pool, accumulator and modifier entry bytes of one batch of states.
    pub batch_bytes: usize,
}

impl Default for Capacity {
    fn default() -> Self {
        Self::zero()
    }
}

macro_rules! each_capacity {
    ($macro:ident) => {
        $macro!(
            states,
            resources,
            resource_rows,
            modifier_rows,
            token_bytes,
            override_rows,
            origins,
            routes,
            provinces,
            entries,
            countries,
            definitions,
            name_bytes,
            batch_bytes
        )
    };
}

impl Capacity {
    pub fn raise(&mut self, required: &Self) {
        macro_rules! raise {
            ($($field:ident),+) => {$(self.$field = self.$field.max(required.$field);)+};
        }
        each_capacity!(raise);
    }

    pub fn zero() -> Self {
        macro_rules! zero {
            ($($field:ident),+) => { Self { $($field: 0),+ } };
        }
        each_capacity!(zero)
    }
}

/// States, countries and records are read in groups of this many.
const BATCH: usize = 64;

#[derive(Clone, Copy)]
struct Scratch {
    pointer: usize,
    id: u32,
    controller: u32,
    /// Canonical controller slot, zero without a controller.
    slot: u32,
    fields: [u8; STATE_RECORD],
    origin: bool,
    level: Option<i32>,
    supply: bool,
    /// Scratch ran out for this state: it has no rows this hour.
    skipped: bool,
}

impl Default for Scratch {
    fn default() -> Self {
        Self {
            pointer: 0,
            id: 0,
            controller: 0,
            slot: 0,
            fields: [0; STATE_RECORD],
            origin: false,
            level: None,
            supply: false,
            skipped: false,
        }
    }
}

impl Scratch {
    fn header(&self, at: usize) -> [u8; 16] {
        header16(&self.fields, at)
    }
}

/// One controlling country, by canonical slot.
#[derive(Clone, Copy)]
struct Nation {
    pointer: usize,
    id: u32,
    resources: usize,
    /// Its states in `order`.
    first: usize,
    end: usize,
    origins: Array,
    any_origin: bool,
    companion: usize,
    routes: Array,
    nodes: Array,
    /// Its route pairs in the route buffer.
    route_start: usize,
}

const NO_ARRAY: Array = Array { data: 0, count: 0 };

impl Default for Nation {
    fn default() -> Self {
        Self {
            pointer: 0,
            id: 0,
            resources: 0,
            first: 0,
            end: 0,
            origins: NO_ARRAY,
            any_origin: false,
            companion: 0,
            routes: NO_ARRAY,
            nodes: NO_ARRAY,
            route_start: 0,
        }
    }
}

/// Caller-owned rows and scratch; only the worker walk grows storage.
pub struct Buffers {
    pub inputs: Vec<InputRow>,
    pub resources: Vec<ResourceRow>,
    pub modifiers: Vec<ModifierRow>,
    pub tokens: Vec<u8>,
    pub overrides: Vec<OverrideRow>,
    /// The countries whose overrides the walk read this hour, including
    /// those with none: only controllers are walked, the selected ones and
    /// those of every selected state.
    pub read_countries: Vec<u32>,
    descriptors: Vec<Resource>,
    resource_keys: Vec<[u32; 3]>,
    keys: Vec<u32>,
    values: Vec<Value>,
    states: Vec<Scratch>,
    order: Vec<(u32, u32)>,
    by_pointer: Vec<(u64, u32)>,
    words: Vec<u64>,
    addresses: Vec<usize>,
    bytes: Vec<u8>,
    routes: Vec<u8>,
    links: Vec<u32>,
    country_pointers: Vec<u64>,
    system_pointers: Vec<u64>,
    nations: Vec<Nation>,
    controllers: Vec<u32>,
    names: Vec<(u64, usize, usize)>,
    name_bytes: Vec<u8>,
    batch: Vec<u8>,
    /// Modifier metadata, read once per walk; its storage is kept.
    memo: DefinitionMemo,
}

impl Buffers {
    pub fn new(capacity: &Capacity) -> std::io::Result<Self> {
        let oom = crate::allocation::oom;
        let keys = capacity
            .resources
            .checked_mul(3)
            .and_then(|n| n.checked_add(2))
            .ok_or_else(oom)?;
        let words = capacity
            .states
            .max(capacity.origins)
            .max(capacity.provinces)
            .max(BATCH);
        let bytes = capacity
            .states
            .checked_mul(8)
            .ok_or_else(oom)?
            .max(capacity.countries.checked_mul(8).ok_or_else(oom)?)
            .max(capacity.origins.checked_mul(16).ok_or_else(oom)?)
            .max(capacity.provinces.checked_mul(16).ok_or_else(oom)?)
            .max(
                capacity
                    .entries
                    .checked_mul(ccountry_resources::OVERRIDE_STRIDE)
                    .ok_or_else(oom)?,
            )
            .max(BATCH * STATE_RECORD);
        let routes = capacity.routes.checked_mul(8).ok_or_else(oom)?;
        let names = capacity
            .definitions
            .checked_mul(2)
            .and_then(usize::checked_next_power_of_two)
            .ok_or_else(oom)?;
        Ok(Self {
            inputs: crate::allocation::filled(capacity.states, InputRow::default())?,
            resources: crate::allocation::filled(capacity.resource_rows, ResourceRow::default())?,
            modifiers: crate::allocation::filled(capacity.modifier_rows, ModifierRow::default())?,
            tokens: crate::allocation::filled(capacity.token_bytes, 0)?,
            overrides: crate::allocation::filled(capacity.override_rows, OverrideRow::default())?,
            descriptors: crate::allocation::filled(capacity.resources, Resource::default())?,
            resource_keys: crate::allocation::filled(capacity.resources, [0; 3])?,
            keys: crate::allocation::filled(keys, 0)?,
            values: crate::allocation::filled(keys, Value::default())?,
            states: crate::allocation::filled(capacity.states, Scratch::default())?,
            order: crate::allocation::filled(capacity.states, (0, 0))?,
            by_pointer: crate::allocation::filled(capacity.states, (0, 0))?,
            words: crate::allocation::filled(words, 0)?,
            addresses: crate::allocation::filled(words, 0)?,
            bytes: crate::allocation::filled(bytes, 0)?,
            routes: crate::allocation::filled(routes, 0)?,
            links: crate::allocation::filled(capacity.countries, 0)?,
            country_pointers: crate::allocation::filled(capacity.countries, 0)?,
            system_pointers: crate::allocation::filled(capacity.countries, 0)?,
            nations: crate::allocation::filled(capacity.countries, Nation::default())?,
            controllers: crate::allocation::filled(capacity.countries, 0)?,
            read_countries: crate::allocation::vec(capacity.countries)?,
            names: crate::allocation::filled(names, (0, 0, 0))?,
            name_bytes: crate::allocation::filled(capacity.name_bytes, 0)?,
            batch: crate::allocation::filled(capacity.batch_bytes, 0)?,
            memo: DefinitionMemo::try_with_capacity(keys)?,
        })
    }

    /// No storage at all, for a parked placeholder.
    pub fn empty() -> Self {
        Self {
            inputs: Vec::new(),
            resources: Vec::new(),
            modifiers: Vec::new(),
            tokens: Vec::new(),
            overrides: Vec::new(),
            descriptors: Vec::new(),
            resource_keys: Vec::new(),
            keys: Vec::new(),
            values: Vec::new(),
            states: Vec::new(),
            order: Vec::new(),
            by_pointer: Vec::new(),
            words: Vec::new(),
            addresses: Vec::new(),
            bytes: Vec::new(),
            routes: Vec::new(),
            links: Vec::new(),
            country_pointers: Vec::new(),
            system_pointers: Vec::new(),
            nations: Vec::new(),
            controllers: Vec::new(),
            read_countries: Vec::new(),
            names: Vec::new(),
            name_bytes: Vec::new(),
            batch: Vec::new(),
            memo: DefinitionMemo::default(),
        }
    }

    pub fn allocated_bytes(&self) -> usize {
        fn bytes<T>(v: &Vec<T>) -> usize {
            v.capacity() * size_of::<T>()
        }
        bytes(&self.inputs)
            + bytes(&self.resources)
            + bytes(&self.modifiers)
            + bytes(&self.tokens)
            + bytes(&self.overrides)
            + bytes(&self.descriptors)
            + bytes(&self.resource_keys)
            + bytes(&self.keys)
            + bytes(&self.values)
            + bytes(&self.states)
            + bytes(&self.order)
            + bytes(&self.by_pointer)
            + bytes(&self.words)
            + bytes(&self.addresses)
            + bytes(&self.bytes)
            + bytes(&self.routes)
            + bytes(&self.links)
            + bytes(&self.country_pointers)
            + bytes(&self.system_pointers)
            + bytes(&self.nations)
            + bytes(&self.controllers)
            + bytes(&self.read_countries)
            + bytes(&self.names)
            + bytes(&self.name_bytes)
            + bytes(&self.batch)
            + self.memo.allocated_bytes()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    /// Inputs, resources, modifiers and overrides.
    pub outcomes: [Outcome; 4],
    pub token_bytes: usize,
    pub required: Capacity,
}

#[derive(Default)]
struct Totals {
    lens: [usize; 4],
    token_bytes: usize,
    /// States and countries whose rows did not fit.
    unread_states: u32,
    unread_countries: u32,
    /// Rows and name bytes every state and controller needs, fitted or not.
    demand: [usize; 4],
}

/// The owners one walk reads: native state IDs for StateResourceInputs,
/// StateResources and StateDynamicModifiers, and native country IDs for
/// CountryBuildingResources (a sliced hour's slice and early reads). The
/// controller of every selected state also has its overrides read, since
/// building resource grant accumulation reads the state's controller's: a
/// country that has just come to control a state is read with it
/// (`Buffers::read_countries` lists every country read).
#[derive(Clone, Copy)]
pub struct Selection<'a> {
    pub states: &'a dyn Fn(u32) -> bool,
    pub countries: &'a dyn Fn(u32) -> bool,
}

fn every(_: u32) -> bool {
    true
}

impl Selection<'static> {
    /// Every state and every country.
    pub const ALL: Self = Self {
        states: &every,
        countries: &every,
    };
}

mod census;
pub use census::{CoreRow, demand, state_census, state_cores, state_cores_growing};

/// One walk for all four tables: they share the state census and fail as
/// one. Every state record is read, for the controllers; only the selected
/// states and countries read their rows.
pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    buffers: &mut Buffers,
    select: Selection<'_>,
) -> Walk {
    walk_with(memory, base, game_state, buffers, select, false)
}

/// Worker-only acquisition: grow from each original header before filling it.
pub fn walk_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    buffers: &mut Buffers,
    select: Selection<'_>,
) -> Walk {
    walk_with(memory, base, game_state, buffers, select, true)
}

fn walk_with(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    buffers: &mut Buffers,
    select: Selection<'_>,
    growing: bool,
) -> Walk {
    let mut required = Capacity::zero();
    let mut totals = Totals::default();
    buffers.read_countries.clear();
    let result = walk_inner(
        memory,
        base,
        game_state,
        buffers,
        select,
        &mut required,
        &mut totals,
        growing,
    );
    let outcome = |len: usize, unread: u32| {
        if unread == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: unread,
            }
        }
    };
    match result {
        Ok(()) => Walk {
            outcomes: [
                outcome(totals.lens[0], totals.unread_states),
                outcome(totals.lens[1], totals.unread_states),
                outcome(totals.lens[2], totals.unread_states),
                outcome(totals.lens[3], totals.unread_countries),
            ],
            token_bytes: totals.token_bytes,
            required,
        },
        Err(reason) => Walk {
            outcomes: {
                buffers.read_countries.clear();
                [Outcome::Unavailable { reason }; 4]
            },
            token_bytes: 0,
            required,
        },
    }
}

fn need(required: &mut usize, count: usize) {
    *required = (*required).max(count);
}

fn grow<T: Default>(
    values: &mut Vec<T>,
    required: usize,
    growing: bool,
) -> Result<(), &'static str> {
    if growing {
        Buffer::Growing(values).ensure(required)?;
    }
    Ok(())
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn header16(bytes: &[u8], at: usize) -> [u8; 16] {
    bytes[at..at + 16].try_into().unwrap()
}

fn address(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("raid state input address overflow")
}

const fn field(source_offset: usize, destination_offset: usize, len: usize) -> RecordField {
    RecordField {
        source_offset,
        destination_offset,
        len,
    }
}

// A state's own fields, read in record batches: its identity and controller
// for every state, everything for the states this hour selects.
const STATE_VTABLE: usize = 0;
const STATE_PROVINCES: usize = 8;
const STATE_ID: usize = 24;
const STATE_CONTROLLER: usize = 28;
const STATE_INDICES: usize = 32;
const STATE_BUILDINGS: usize = 48;
const STATE_GATE: usize = 64;
const STATE_POOL: usize = 72;
const STATE_ACCUMULATOR: usize = 96;
const STATE_CONTAINER: usize = 112;
const STATE_HASH: usize = 168;
const STATE_RECORD: usize = 192;
const STATE_FIELDS: [RecordField; 11] = [
    field(0, STATE_VTABLE, 8),
    field(cstate::PROVINCES, STATE_PROVINCES, 16),
    field(cstate::ID, STATE_ID, 4),
    field(cstate::CONTROLLER, STATE_CONTROLLER, 4),
    field(
        cstate::BUILDING_STATUS + cbuilding_status::BUILDING_INDICES,
        STATE_INDICES,
        16,
    ),
    field(
        cstate::BUILDING_STATUS + cbuilding_status::BUILDINGS,
        STATE_BUILDINGS,
        16,
    ),
    field(cstate::RESOURCE_MODIFIER_GATE, STATE_GATE, 1),
    field(cstate::RESOURCE_POOL, STATE_POOL, 24),
    field(cstate::BASE_MODIFIERS, STATE_ACCUMULATOR, 16),
    field(cstate::DYNAMIC_MODIFIERS, STATE_CONTAINER, 0x38),
    field(cstate::COUNTRY_MODIFIERS, STATE_HASH, 17),
];
const STATE_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&STATE_FIELDS, STATE_RECORD).expect("valid record layout");
/// The fields every state is read for: identity and controller, at their
/// [`STATE_LAYOUT`] places.
const STATE_HEAD_LAYOUT: RecordLayout<'static> = RecordLayout::new(
    &[STATE_FIELDS[0], STATE_FIELDS[2], STATE_FIELDS[3]],
    STATE_RECORD,
)
.expect("valid record layout");

const COUNTRY_FIELDS: [RecordField; 3] = [
    field(0, 0, 8),
    field(ccountry::ID, 8, 4),
    field(ccountry::RESOURCES, 16, 8),
];
const COUNTRY_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&COUNTRY_FIELDS, 24).expect("valid record layout");

const RESOURCES_FIELDS: [RecordField; 4] = [
    field(0, 0, 8),
    field(ccountry_resources::OWNER, 8, 4),
    field(ccountry_resources::RESOURCE_ORIGINS, 16, 16),
    field(ccountry_resources::BUILDING_RESOURCE_OVERRIDES, 32, 16),
];
const RESOURCES_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&RESOURCES_FIELDS, 48).expect("valid record layout");

const SYSTEM_FIELDS: [RecordField; 2] = [
    field(0, 0, 16),
    field(ccountry_supply_system::SUPPLY_SYSTEM, 16, 24),
];
const SYSTEM_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&SYSTEM_FIELDS, 40).expect("valid record layout");

const COMPANION_FIELDS: [RecordField; 2] = [
    field(supply_companion::ROUTES_SUBTYPE_ZERO, 0, 16),
    field(supply_companion::NODES, 16, 16),
];
const COMPANION_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&COMPANION_FIELDS, 32).expect("valid record layout");

const ORIGIN_FIELDS: [RecordField; 2] = [field(0, 0, 8), field(cresource_origin::STATE, 8, 8)];
const ORIGIN_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&ORIGIN_FIELDS, 16).expect("valid record layout");

const PROVINCE_FIELDS: [RecordField; 2] = [field(0, 0, 8), field(cprovince::ID, 8, 4)];
const PROVINCE_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&PROVINCE_FIELDS, 16).expect("valid record layout");

const NODE_FIELDS: [RecordField; 2] = [
    field(supply_node::INDEX, 0, 8),
    field(supply_node::ROUTE_VALUE, 8, 8),
];
const NODE_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&NODE_FIELDS, 16).expect("valid record layout");

const INDEX_FIELDS: [RecordField; 1] = [field(0, 0, 4)];
const INDEX_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&INDEX_FIELDS, 8).expect("valid record layout");

const POINTER_FIELDS: [RecordField; 1] = [field(0, 0, 8)];
const POINTER_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&POINTER_FIELDS, 8).expect("valid record layout");

const BUILDING_FIELDS: [RecordField; 2] = [
    field(cbuilding::TEMPLATE, 0, 8),
    field(cbuilding::LEVEL, 8, 2),
];
const BUILDING_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&BUILDING_FIELDS, 16).expect("valid record layout");

/// The first bucket's probe distance byte.
const BUCKET_FIELDS: [RecordField; 1] = [field(4, 0, 1)];
const BUCKET_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&BUCKET_FIELDS, 8).expect("valid record layout");

/// Each row of `sources` under `layout`, 64 per guard, into `rows`; calls
/// `visit(index, fields, row)` in order with the row's copied field count
/// (all of them, or the prefix before a fault). A fault ends only its row:
/// the batch resumes after it.
fn each_record(
    memory: &impl Memory,
    sources: &[usize],
    layout: &RecordLayout<'_>,
    rows: &mut [u8],
    mut visit: impl FnMut(usize, usize, &[u8]),
) {
    let stride = layout.stride();
    let fields = layout.field_count();
    for (chunk, addresses) in sources.chunks(BATCH).enumerate() {
        let mut start = 0;
        while start < addresses.len() {
            let count = addresses.len() - start;
            let destination = &mut rows[..count * stride];
            destination.fill(0);
            let copied = memory.copy_records(
                RecordSources::Pointers(&addresses[start..]),
                layout,
                destination,
            );
            let completed = copied / fields;
            let first = chunk * BATCH + start;
            for row in 0..completed.min(count) {
                visit(first + row, fields, &rows[row * stride..(row + 1) * stride]);
            }
            if completed < count {
                let row = completed;
                visit(
                    first + row,
                    copied % fields,
                    &rows[row * stride..(row + 1) * stride],
                );
                start += completed + 1;
            } else {
                start = addresses.len();
            }
        }
    }
}

/// Every record of `sources` in one guarded batch, or `error`.
fn records(
    memory: &impl Memory,
    sources: &[usize],
    layout: &RecordLayout<'_>,
    destination: &mut [u8],
    error: &'static str,
) -> Result<(), &'static str> {
    let len = sources.len() * layout.stride();
    if memory.copy_records(
        RecordSources::Pointers(sources),
        layout,
        &mut destination[..len],
    ) != sources.len() * layout.field_count()
    {
        return Err(error);
    }
    Ok(())
}

/// Up to `BATCH` independent spans `(address, len)`, laid out back to back in
/// `destination`, in one guarded batch.
fn gather(
    memory: &impl Memory,
    spans: &[(usize, usize)],
    destination: &mut [u8],
    error: &'static str,
) -> Result<(), &'static str> {
    debug_assert!(spans.len() <= BATCH);
    let mut addresses = [0_usize; BATCH];
    let mut slices: [&mut [u8]; BATCH] = std::array::from_fn(|_| <&mut [u8]>::default());
    let mut rest = destination;
    for (index, &(at, len)) in spans.iter().enumerate() {
        let (head, tail) = std::mem::take(&mut rest).split_at_mut(len);
        addresses[index] = at;
        slices[index] = head;
        rest = tail;
    }
    if memory.copy_scattered_prefetched(addresses, &mut slices) != BATCH {
        return Err(error);
    }
    Ok(())
}

/// `lookup_many_at` over an owned entry copy: every key positive and
/// increasing; the wanted sorted keys' raw values.
fn scan_accumulator(
    entries: &[u8],
    keys: &[u32],
    output: &mut [Value],
) -> Result<(), &'static str> {
    output.fill(Value::default());
    let mut previous = 0;
    let mut wanted = 0;
    for entry in entries.chunks_exact(modifier_entry::STRIDE) {
        let key = u32_at(entry, 0);
        if key <= previous {
            return Err("modifier accumulator keys not positive and increasing");
        }
        previous = key;
        while wanted < keys.len() && keys[wanted] < key {
            wanted += 1;
        }
        if wanted < keys.len() && keys[wanted] == key {
            output[wanted] = Value {
                raw: u64_at(entry, 8) as i64,
                present: true,
            };
            wanted += 1;
        }
    }
    Ok(())
}

/// `dynamic_modifier_entry::header` over an owned entry copy.
fn modifier_entry(base: usize, bytes: &[u8]) -> Result<(u32, i32, usize), &'static str> {
    if u64_at(bytes, 0) != address(base, dynamic_modifier_entry::VTABLE_RVA)? as u64 {
        return Err("dynamic modifier entry vtable mismatch");
    }
    let definition = usize::try_from(u64_at(bytes, dynamic_modifier_entry::DEFINITION))
        .map_err(|_| "dynamic modifier definition pointer overflow")?;
    if bytes[dynamic_modifier_entry::ENABLED] > 1 || definition == 0 {
        return Err("dynamic modifier enabled byte or definition invalid");
    }
    let data = usize::try_from(u64_at(bytes, dynamic_modifier_entry::VALUES))
        .map_err(|_| "dynamic modifier value pointer overflow")?;
    let capacity = u32_at(bytes, dynamic_modifier_entry::VALUES_CAPACITY);
    let count = u32_at(bytes, dynamic_modifier_entry::VALUES_COUNT);
    if count > capacity || capacity > i32::MAX as u32 || (count != 0 && data == 0) {
        return Err("dynamic modifier value array invalid");
    }
    data.checked_add(
        (count as usize)
            .checked_mul(dynamic_modifier_entry::VALUE_STRIDE)
            .ok_or("dynamic modifier value array overflow")?,
    )
    .ok_or("dynamic modifier value array overflow")?;
    Ok((
        u32_at(bytes, dynamic_modifier_entry::SCOPE_COUNTRY),
        u32_at(bytes, dynamic_modifier_entry::SCOPE_STATE) as i32,
        definition,
    ))
}

/// Definition names kept for the walk, keyed by definition pointer.
struct Names<'a> {
    table: &'a mut Vec<(u64, usize, usize)>,
    bytes: &'a mut Vec<u8>,
    used: usize,
    entries: usize,
    limit: usize,
    growing: bool,
}

impl Names<'_> {
    /// The name's `(start, len)` in `bytes`, or its source when not kept.
    fn get(
        &mut self,
        memory: &impl Memory,
        base: usize,
        definition: usize,
        required: &mut Capacity,
    ) -> Result<Name, &'static str> {
        let mask = self.table.len().wrapping_sub(1);
        let mut slot = (definition as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) as usize >> 7;
        if !self.table.is_empty() {
            loop {
                slot &= mask;
                let (key, start, len) = self.table[slot];
                if key == definition as u64 {
                    return Ok(Name::Kept(start, len));
                }
                if key == 0 {
                    break;
                }
                slot += 1;
            }
        }
        let (source, len) = cdynamic_modifier::token(memory, base, definition)?;
        self.entries += 1;
        need(&mut required.definitions, self.entries);
        need(&mut required.name_bytes, self.used + len);
        if self.growing && self.entries > self.limit {
            let count = self
                .entries
                .checked_mul(2)
                .and_then(usize::checked_next_power_of_two)
                .ok_or("state modifier name dictionary length overflow")?;
            let mut table: Vec<(u64, usize, usize)> = Vec::new();
            grow(&mut table, count, true)?;
            let mask = table.len() - 1;
            for &(key, start, len) in self.table.iter().filter(|entry| entry.0 != 0) {
                let mut slot = (key.wrapping_mul(0x9e37_79b9_7f4a_7c15) as usize >> 7) & mask;
                while table[slot].0 != 0 {
                    slot = (slot + 1) & mask;
                }
                table[slot] = (key, start, len);
            }
            *self.table = table;
            self.limit = self.table.len() / 2;
            slot = (definition as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) as usize >> 7;
            slot &= mask;
            while self.table[slot].0 != 0 {
                slot = (slot + 1) & mask;
            }
        }
        grow(self.bytes, self.used + len, self.growing)?;
        if self.table.is_empty() || self.entries > self.limit || self.used + len > self.bytes.len()
        {
            return Ok(Name::Source(source, len));
        }
        if !read_slice(memory, source, &mut self.bytes[self.used..self.used + len]) {
            return Err("state dynamic modifier name unreadable");
        }
        self.table[slot] = (definition as u64, self.used, len);
        self.used += len;
        Ok(Name::Kept(self.used - len, len))
    }
}

enum Name {
    Kept(usize, usize),
    Source(usize, usize),
}

impl Name {
    fn len(&self) -> usize {
        match *self {
            Self::Kept(_, len) | Self::Source(_, len) => len,
        }
    }
}

/// `ccountry::Reader`'s link and object checks over owned link and pointer arrays.
struct Census<'a> {
    links: &'a [u32],
    pointers: &'a [u64],
}

impl Census<'_> {
    fn canonical(&self, raw: u32) -> Result<u32, &'static str> {
        if raw == 0 {
            return Err("country id is zero");
        }
        let outside = "country link unreadable or outside country table";
        let slot = *self.links.get(raw as usize).ok_or(outside)?;
        if slot == 0 || slot as usize >= self.pointers.len() {
            return Err(outside);
        }
        Ok(slot)
    }
}

#[allow(clippy::too_many_arguments)]
fn walk_inner(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    buffers: &mut Buffers,
    select: Selection<'_>,
    required: &mut Capacity,
    totals: &mut Totals,
    growing: bool,
) -> Result<(), &'static str> {
    let Buffers {
        inputs,
        resources: resource_rows,
        modifiers,
        tokens,
        overrides,
        read_countries,
        descriptors,
        resource_keys,
        keys,
        values,
        states,
        order,
        by_pointer,
        words,
        addresses,
        bytes,
        routes,
        links,
        country_pointers,
        system_pointers,
        nations,
        controllers,
        names,
        name_bytes,
        batch,
        memo,
    } = buffers;

    // The live resource database and each resource's accumulator keys.
    let resources =
        match resource_database(memory, base, descriptors, resource_keys, bytes, growing)? {
            Ok(len) => len,
            Err(demand) => {
                need(&mut required.resources, demand);
                totals.unread_states = 1;
                totals.unread_countries = 1;
                return Ok(());
            }
        };
    need(&mut required.resources, resources);
    let key_capacity = resources
        .checked_mul(3)
        .and_then(|count| count.checked_add(2))
        .ok_or("state resource key count overflow")?;
    grow(keys, key_capacity, growing)?;
    grow(values, key_capacity, growing)?;
    let descriptors = &descriptors[..resources];
    let mut key_count = 0;
    for (descriptor, triple) in descriptors.iter().zip(&resource_keys[..resources]) {
        if !descriptor.keys_read {
            return Err("resource modifier keys unreadable");
        }
        for &key in triple {
            if key == 0 {
                return Err("resource modifier key zero");
            }
            keys[key_count] = key;
            key_count += 1;
        }
    }
    keys[key_count] = GLOBAL_FACTOR_KEY;
    keys[key_count + 1] = INFRASTRUCTURE_KEY;
    key_count += 2;
    keys[..key_count].sort_unstable();
    let mut unique = 1;
    for index in 1..key_count {
        if keys[index] != keys[unique - 1] {
            keys[unique] = keys[index];
            unique += 1;
        }
    }
    let keys = &keys[..unique];
    let resource_keys = &resource_keys[..resources];
    memo.clear();
    let memo = &*memo;

    // Pass 1: every state's identity and controller, in record batches.
    let root = address(game_state, cgame_state::STATES)
        .ok()
        .and_then(|at| Array::read(memory, at, 8))
        .filter(|array| array.count >= 2)
        .ok_or("state array unreadable or invalid")?;
    let slots = root.count as usize;
    need(&mut required.states, slots);
    grow(states, slots, growing)?;
    grow(order, slots, growing)?;
    grow(by_pointer, slots, growing)?;
    grow(words, slots.max(BATCH), growing)?;
    grow(addresses, slots.max(BATCH), growing)?;
    grow(bytes, (slots * 8).max(BATCH * STATE_RECORD), growing)?;
    if slots > states.len() || slots * 8 > bytes.len() {
        totals.unread_states = root.count;
        totals.unread_countries = 1;
        return Ok(());
    }
    let vtable = address(base, cstate::VTABLE_RVA)? as u64;
    let mut count = 0;
    let mut readable = 0;
    if !read_slice(memory, root.data, &mut bytes[..slots * 8]) {
        return Err("state pointers unreadable");
    } else {
        for (slot, word) in words[..slots].iter_mut().enumerate() {
            *word = u64_at(bytes, slot * 8);
        }
    }
    let mut next = 1;
    while next < slots {
        let mut ids = [0_u32; BATCH];
        let mut len = 0;
        while next < slots && len < BATCH {
            let pointer = usize::try_from(words[next]).map_err(|_| "state pointer overflow")?;
            if pointer != 0 {
                addresses[len] = pointer;
                ids[len] = next as u32;
                len += 1;
            }
            next += 1;
        }
        // The pointers now live in `words`; the records reuse `bytes`.
        let record = &mut bytes[..];
        records(
            memory,
            &addresses[..len],
            &STATE_HEAD_LAYOUT,
            record,
            "state resource fields unreadable",
        )?;
        for (row, &slot) in ids[..len].iter().enumerate() {
            let fields: [u8; STATE_RECORD] = record[row * STATE_RECORD..(row + 1) * STATE_RECORD]
                .try_into()
                .unwrap();
            if u64_at(&fields, STATE_VTABLE) != vtable {
                return Err("state vtable mismatch");
            }
            let id = u32_at(&fields, STATE_ID);
            if id != slot {
                return Err("state id differs from slot");
            }
            states[count] = Scratch {
                pointer: addresses[row],
                id,
                controller: u32_at(&fields, STATE_CONTROLLER),
                fields,
                ..Scratch::default()
            };
            count += 1;
            readable += 1;
        }
    }
    if readable == 0 {
        return Err("state table has no readable states");
    }
    let states = &mut states[..count];

    // Controllers, resolved once per walk over owned link and pointer arrays.
    let country_array = address(game_state, cgame_state::COUNTRIES)
        .ok()
        .and_then(|at| Array::read(memory, at, 8))
        .ok_or("country array unreadable or invalid")?;
    let link_array = address(game_state, cgame_state::COUNTRY_LINKS)
        .ok()
        .and_then(|at| Array::read(memory, at, 4))
        .ok_or("country links unreadable or invalid")?;
    if country_array.count < 2 {
        return Err("country table has no country slots");
    }
    if link_array.count == 0 {
        return Err("country links have no sentinel slot");
    }
    let country_slots = country_array.count as usize;
    let link_count = link_array.count as usize;
    need(&mut required.countries, country_slots.max(link_count));
    grow(links, country_slots.max(link_count), growing)?;
    grow(country_pointers, country_slots, growing)?;
    grow(system_pointers, country_slots, growing)?;
    grow(nations, country_slots, growing)?;
    grow(controllers, country_slots, growing)?;
    grow(bytes, (country_slots * 8).max(link_count * 4), growing)?;
    if growing {
        crate::allocation::reserve(read_countries, country_slots)
            .map_err(|_| "state controller allocation failed")?;
    }
    if country_slots.max(link_count) > links.len() || country_slots * 8 > bytes.len() {
        totals.unread_states = count as u32;
        totals.unread_countries = 1;
        return Ok(());
    }
    if !read_slice(memory, link_array.data, &mut bytes[..link_count * 4]) {
        return Err("country links unreadable");
    }
    for (index, link) in links[..link_count].iter_mut().enumerate() {
        *link = u32_at(bytes, index * 4);
    }
    if !read_slice(memory, country_array.data, &mut bytes[..country_slots * 8]) {
        return Err("country pointers unreadable");
    }
    for (index, pointer) in country_pointers[..country_slots].iter_mut().enumerate() {
        *pointer = u64_at(bytes, index * 8);
    }
    let census = Census {
        links: &links[..link_count],
        pointers: &country_pointers[..country_slots],
    };
    let nations = &mut nations[..country_slots];
    for nation in nations.iter_mut() {
        *nation = Nation::default();
    }
    let mut distinct = 0;
    let mut control = |controller: u32| -> Result<u32, &'static str> {
        let slot = census.canonical(controller)?;
        let nation = &mut nations[slot as usize];
        if nation.pointer == 0 {
            nation.pointer = usize::try_from(census.pointers[slot as usize])
                .ok()
                .filter(|&pointer| pointer != 0)
                .ok_or("country link resolves to null country")?;
            controllers[distinct] = slot;
            distinct += 1;
        }
        Ok(slot)
    };
    for state in states.iter_mut() {
        if state.controller != 0 {
            state.slot = control(state.controller)?;
        }
    }
    let controllers = &mut controllers[..distinct];
    controllers.sort_unstable();
    // Each controller's object: vtable, id and resources pointer.
    let country_vtable = address(base, ccountry::VTABLE_RVA)? as u64;
    for chunk in controllers.chunks(BATCH) {
        for (index, &slot) in chunk.iter().enumerate() {
            addresses[index] = nations[slot as usize].pointer;
        }
        records(
            memory,
            &addresses[..chunk.len()],
            &COUNTRY_LAYOUT,
            bytes,
            "country fields unreadable",
        )?;
        for (index, &slot) in chunk.iter().enumerate() {
            let at = index * 24;
            if u64_at(bytes, at) != country_vtable {
                return Err("country vtable mismatch");
            }
            let id = u32_at(bytes, at + 8);
            // `Reader::read_at` and `resolve`: the object's id, its slot and
            // its canonical slot all lead to this slot and this object.
            if census.canonical(id)? != slot || census.canonical(slot)? != slot {
                return Err("country link does not resolve to its canonical object");
            }
            let nation = &mut nations[slot as usize];
            nation.id = id;
            nation.resources = usize::try_from(u64_at(bytes, at + 16))
                .ok()
                .filter(|&pointer| pointer != 0)
                .ok_or("country resources pointer unreadable or null")?;
        }
    }

    // This hour's states; the others keep their earlier rows.
    let mut selected = 0;
    for index in 0..count {
        if (select.states)(states[index].id) {
            states[selected] = states[index];
            selected += 1;
        }
    }
    let count = selected;
    let states = &mut states[..count];
    // The selected states' own fields; the others were read for their
    // controllers alone.
    for batch in states.chunks_mut(BATCH) {
        for (address, state) in addresses.iter_mut().zip(batch.iter()) {
            *address = state.pointer;
        }
        records(
            memory,
            &addresses[..batch.len()],
            &STATE_LAYOUT,
            bytes,
            "state resource fields unreadable",
        )?;
        for (row, state) in batch.iter_mut().enumerate() {
            state
                .fields
                .copy_from_slice(&bytes[row * STATE_RECORD..(row + 1) * STATE_RECORD]);
        }
    }

    // Each controller's selected states, grouped in `order`.
    for (index, state) in states.iter().enumerate() {
        order[index] = (state.slot, index as u32);
        by_pointer[index] = (state.pointer as u64, index as u32);
    }
    let order = &mut order[..count];
    order.sort_unstable();
    let by_pointer = &mut by_pointer[..count];
    by_pointer.sort_unstable();
    let mut first = 0;
    while first < count {
        let slot = order[first].0;
        let mut end = first;
        while end < count && order[end].0 == slot {
            end += 1;
        }
        if slot != 0 {
            nations[slot as usize].first = first;
            nations[slot as usize].end = end;
        }
        first = end;
    }
    // Controllers read for a selected state or for their own overrides.
    let mut wanted = 0;
    for index in 0..controllers.len() {
        let nation = nations[controllers[index] as usize];
        if nation.first != nation.end || (select.countries)(nation.id) {
            controllers[wanted] = controllers[index];
            wanted += 1;
        }
    }
    let controllers = &mut controllers[..wanted];

    // Pass 2: resources, overrides and origins, in country batches.
    let resources_vtable = address(base, ccountry_resources::VTABLE_RVA)? as u64;
    let origin_vtable = address(base, cresource_origin::VTABLE_RVA)? as u64;
    let skip = |states: &mut [Scratch], nation: &Nation, totals: &mut Totals| {
        for &(_, index) in &order[nation.first..nation.end] {
            states[index as usize].skipped = true;
            totals.unread_states += 1;
        }
    };
    for chunk in controllers.chunks(BATCH) {
        for (index, &slot) in chunk.iter().enumerate() {
            addresses[index] = nations[slot as usize].resources;
        }
        records(
            memory,
            &addresses[..chunk.len()],
            &RESOURCES_LAYOUT,
            bytes,
            "country resources fields unreadable",
        )?;
        let mut entries = [NO_ARRAY; BATCH];
        for (index, &slot) in chunk.iter().enumerate() {
            let at = index * 48;
            if u64_at(bytes, at) != resources_vtable {
                return Err("country resources vtable mismatch");
            }
            if census.canonical(u32_at(bytes, at + 8))? != slot {
                return Err("country resources owner disagrees with country");
            }
            let origins = Array::from_header(header16(bytes, at + 16), 8)
                .ok_or("resource origins invalid")?;
            // Only a controller of a selected state reads its origins.
            let nation = &mut nations[slot as usize];
            nation.origins = if nation.first != nation.end {
                origins
            } else {
                NO_ARRAY
            };
            entries[index] = Array::from_header(
                header16(bytes, at + 32),
                ccountry_resources::OVERRIDE_STRIDE,
            )
            .ok_or("building resource overrides invalid")?;
        }
        for (index, &slot) in chunk.iter().enumerate() {
            let nation = nations[slot as usize];
            // A controller of a selected state is read with it.
            if !(select.countries)(nation.id) && nation.first == nation.end {
                continue;
            }
            let written = overrides_of(
                memory,
                entries[index],
                nation.id,
                descriptors,
                bytes,
                if growing {
                    Buffer::GrowingFrom(overrides, totals.lens[3])
                } else {
                    Buffer::Fixed(&mut overrides[totals.lens[3]..])
                },
                required,
                &mut totals.demand[3],
                growing,
            )?;
            match written {
                Some(rows) => {
                    totals.lens[3] += rows;
                    read_countries.push(nation.id);
                }
                None => totals.unread_countries += 1,
            }
        }
        // Origin vectors, then each origin's vtable and state, per group of
        // countries whose origins fit.
        let mut start = 0;
        while start < chunk.len() {
            let mut end = start;
            let mut total = 0;
            while end < chunk.len() {
                let origins = nations[chunk[end] as usize].origins.count as usize;
                need(&mut required.origins, origins);
                grow(words, total + origins, growing)?;
                grow(addresses, total + origins, growing)?;
                grow(bytes, (total + origins) * 16, growing)?;
                if total + origins > words.len() || (total + origins) * 16 > bytes.len() {
                    break;
                }
                total += origins;
                end += 1;
            }
            if end == start {
                skip(states, &nations[chunk[start] as usize], totals);
                start += 1;
                continue;
            }
            let mut spans = [(0, 0); BATCH];
            for (span, &slot) in spans.iter_mut().zip(&chunk[start..end]) {
                let origins = nations[slot as usize].origins;
                *span = (origins.data, origins.count as usize * 8);
            }
            gather(
                memory,
                &spans[..end - start],
                bytes,
                "resource origin vector unreadable",
            )?;
            non_null(bytes, &mut addresses[..total])?;
            records(
                memory,
                &addresses[..total],
                &ORIGIN_LAYOUT,
                bytes,
                "resource origin unreadable",
            )?;
            let mut origin = 0;
            for &slot in &chunk[start..end] {
                for _ in 0..nations[slot as usize].origins.count {
                    if u64_at(bytes, origin * 16) != origin_vtable {
                        return Err("resource origin vtable mismatch");
                    }
                    let pointer = u64_at(bytes, origin * 16 + 8);
                    if let Ok(found) = by_pointer.binary_search_by_key(&pointer, |&(p, _)| p) {
                        let state = &mut states[by_pointer[found].1 as usize];
                        if state.slot == slot {
                            state.origin = true;
                            nations[slot as usize].any_origin = true;
                        }
                    }
                    origin += 1;
                }
            }
            start = end;
        }
    }

    // Supply systems of controllers with an origin state.
    let supply_root = csupply_system::read_root(memory, base, game_state)?;
    let system_array = Array::read(
        memory,
        address(supply_root, csupply_system::COUNTRY_SYSTEM_DATA)?,
        8,
    )
    .ok_or("country supply array unreadable or invalid")?;
    let companion_array = Array::read(
        memory,
        address(supply_root, csupply_system::COUNTRY_COMPANION_DATA)?,
        supply_companion::STRIDE,
    )
    .ok_or("country supply companion array unreadable or invalid")?;
    if system_array.count != country_array.count || companion_array.count != country_array.count {
        return Err("country supply array counts differ from country slots");
    }
    if !read_slice(memory, system_array.data, &mut bytes[..country_slots * 8]) {
        return Err("country supply slot unreadable");
    }
    for (index, pointer) in system_pointers[..country_slots].iter_mut().enumerate() {
        *pointer = u64_at(bytes, index * 8);
    }
    let system_vtables = [
        address(base, ccountry_supply_system::VTABLE_RVA)? as u64,
        address(base, ccountry_supply_system::SECONDARY_VTABLE_RVA)? as u64,
    ];
    let infrastructure = infrastructure_template(memory, base)?;
    // Only controllers with an origin state read their supply.
    let mut supplied = 0;
    for index in 0..controllers.len() {
        let slot = controllers[index];
        if nations[slot as usize].any_origin {
            controllers[supplied] = slot;
            supplied += 1;
        }
    }
    for chunk in controllers[..supplied].chunks(BATCH) {
        for (index, &slot) in chunk.iter().enumerate() {
            addresses[index] = usize::try_from(system_pointers[slot as usize])
                .ok()
                .filter(|&pointer| pointer != 0)
                .ok_or("country has no supply system")?;
        }
        records(
            memory,
            &addresses[..chunk.len()],
            &SYSTEM_LAYOUT,
            bytes,
            "country supply authentication fields unreadable",
        )?;
        for (index, &slot) in chunk.iter().enumerate() {
            let at = index * 40;
            let companion = companion_array
                .element(slot, supply_companion::STRIDE)
                .ok_or("country supply companion address overflow")?;
            if u64_at(bytes, at) != system_vtables[0] || u64_at(bytes, at + 8) != system_vtables[1]
            {
                return Err("country supply vtable mismatch");
            }
            if u64_at(bytes, at + 16) != supply_root as u64
                || u64_at(bytes, at + 24) != nations[slot as usize].pointer as u64
                || u64_at(bytes, at + 32) != companion as u64
            {
                return Err("country supply ownership mismatch");
            }
            nations[slot as usize].companion = companion;
            addresses[index] = companion;
        }
        records(
            memory,
            &addresses[..chunk.len()],
            &COMPANION_LAYOUT,
            bytes,
            "supply routes invalid",
        )?;
        for (index, &slot) in chunk.iter().enumerate() {
            let nation = &mut nations[slot as usize];
            nation.routes = Array::from_header(header16(bytes, index * 32), 8)
                .ok_or("supply routes invalid")?;
            nation.nodes =
                Array::from_header(header16(bytes, index * 32 + 16), supply_node::STRIDE)
                    .ok_or("supply nodes invalid")?;
        }
        // Route arrays, per group of countries whose routes fit.
        let mut start = 0;
        while start < chunk.len() {
            let mut end = start;
            let mut total = 0;
            while end < chunk.len() {
                let pairs = nations[chunk[end] as usize].routes.count as usize;
                need(&mut required.routes, pairs);
                grow(routes, (total + pairs) * 8, growing)?;
                if (total + pairs) * 8 > routes.len() {
                    break;
                }
                nations[chunk[end] as usize].route_start = total;
                total += pairs;
                end += 1;
            }
            if end == start {
                skip(states, &nations[chunk[start] as usize], totals);
                start += 1;
                continue;
            }
            let mut spans = [(0, 0); BATCH];
            for (span, &slot) in spans.iter_mut().zip(&chunk[start..end]) {
                let routes = nations[slot as usize].routes;
                *span = (routes.data, routes.count as usize * 8);
            }
            gather(
                memory,
                &spans[..end - start],
                routes,
                "supply routes unreadable",
            )?;
            // Their origin states' infrastructure and supply route.
            let mut members = [0_u32; BATCH];
            let mut len = 0;
            for &slot in &chunk[start..end] {
                let nation = nations[slot as usize];
                for &(_, index) in &order[nation.first..nation.end] {
                    let state = &states[index as usize];
                    if !state.origin || state.skipped {
                        continue;
                    }
                    members[len] = index;
                    len += 1;
                    if len == BATCH {
                        origin_states(
                            memory,
                            base,
                            infrastructure,
                            &members[..len],
                            states,
                            nations,
                            routes,
                            bytes,
                            words,
                            addresses,
                            required,
                            totals,
                            growing,
                        )?;
                        len = 0;
                    }
                }
            }
            origin_states(
                memory,
                base,
                infrastructure,
                &members[..len],
                states,
                nations,
                routes,
                bytes,
                words,
                addresses,
                required,
                totals,
                growing,
            )?;
            start = end;
        }
    }

    // Pass 3: rows in state order, in batches whose arrays fit.
    let pool_len = (resources + 1) * cstrategic_resource_pool::VALUE_STRIDE;
    let pool_fields = [field(0, 0, pool_len)];
    let pool_layout =
        RecordLayout::new(&pool_fields, pool_len).ok_or("resource pool layout invalid")?;
    let mut names = Names {
        limit: names.len() / 2,
        table: names,
        bytes: name_bytes,
        used: 0,
        entries: 0,
        growing,
    };
    names.table.fill((0, 0, 0));
    let mut next = 0;
    while next < count {
        // Pools, then accumulator entries, then modifier entries.
        let mut members = [0_usize; BATCH];
        let mut accumulator_spans = [(0, 0); BATCH];
        let mut container_spans = [(0, 0); BATCH];
        let mut len = 0;
        let mut used = 0;
        while next < count && len < BATCH {
            let state = &states[next];
            if state.skipped {
                next += 1;
                continue;
            }
            let accumulator =
                Array::from_header(state.header(STATE_ACCUMULATOR), modifier_entry::STRIDE)
                    .ok_or("modifier accumulator header unreadable or invalid")?;
            let container = container_array(base, state)?;
            let size = pool_len
                + accumulator.count as usize * modifier_entry::STRIDE
                + container.count as usize * dynamic_modifier_entry::STRIDE;
            need(&mut required.batch_bytes, size);
            grow(batch, used + size, growing)?;
            if used + size > batch.len() {
                if len == 0 {
                    states[next].skipped = true;
                    totals.unread_states += 1;
                    next += 1;
                    continue;
                }
                break;
            }
            members[len] = next;
            accumulator_spans[len] = (
                accumulator.data,
                accumulator.count as usize * modifier_entry::STRIDE,
            );
            container_spans[len] = (
                container.data,
                container.count as usize * dynamic_modifier_entry::STRIDE,
            );
            used += size;
            len += 1;
            next += 1;
        }
        if len == 0 {
            continue;
        }
        let members = &members[..len];
        let mut pools = [0_usize; BATCH];
        for (pool, &member) in pools.iter_mut().zip(members) {
            let header: [u8; 24] = states[member].fields[STATE_POOL..STATE_POOL + 24]
                .try_into()
                .unwrap();
            *pool = cstrategic_resource_pool::from_header(base, header, resources as u32 + 1)?.data;
        }
        // One batch region per kind keeps each gather contiguous.
        let pool_bytes = len * pool_len;
        let accumulator_bytes: usize = accumulator_spans[..len].iter().map(|span| span.1).sum();
        let (pool_region, rest) = batch[..used].split_at_mut(pool_bytes);
        let (accumulator_region, container_region) = rest.split_at_mut(accumulator_bytes);
        if memory.copy_records(
            RecordSources::Pointers(&pools[..len]),
            &pool_layout,
            pool_region,
        ) != len
        {
            return Err("state resource pool unreadable");
        }
        gather(
            memory,
            &accumulator_spans[..len],
            accumulator_region,
            "modifier accumulator entry unreadable",
        )?;
        gather(
            memory,
            &container_spans[..len],
            container_region,
            "dynamic modifier entry unreadable",
        )?;
        // The first bucket of each origin state's country-modifier hash.
        let mut buckets = [0_usize; BATCH];
        let mut probed = [usize::MAX; BATCH];
        let mut probes = 0;
        for (row, &member) in members.iter().enumerate() {
            let state = &states[member];
            if !(state.origin && state.level.is_some()) {
                continue;
            }
            buckets[probes] = first_bucket(state, &census)?;
            probed[row] = probes;
            probes += 1;
        }
        let mut distances = [0_u8; BATCH * 8];
        records(
            memory,
            &buckets[..probes],
            &BUCKET_LAYOUT,
            &mut distances,
            "state movement input unreadable",
        )?;
        let mut pool_at = 0;
        let mut accumulator_at = 0;
        let mut container_at = 0;
        for (row, &member) in members.iter().enumerate() {
            let state = states[member];
            let pool = &pool_region[pool_at..pool_at + pool_len];
            pool_at += pool_len;
            let accumulator_len = accumulator_spans[row].1;
            let entries = &accumulator_region[accumulator_at..accumulator_at + accumulator_len];
            accumulator_at += accumulator_len;
            let container_len = container_spans[row].1;
            let container = &container_region[container_at..container_at + container_len];
            container_at += container_len;
            let values = &mut values[..keys.len()];
            scan_accumulator(entries, keys, values)?;
            let value = |key: u32| -> Result<i64, &'static str> {
                let found = values[keys.binary_search(&key).expect("collected key")];
                if found.present {
                    effective_memo(memory, base, key, found.raw, memo)
                } else {
                    Ok(0)
                }
            };
            let first_row = totals.lens[1];
            grow(resource_rows, first_row + resources, growing)?;
            let mut rows = 0;
            for (slot, (descriptor, triple)) in descriptors.iter().zip(resource_keys).enumerate() {
                let at = (slot + 1) * cstrategic_resource_pool::VALUE_STRIDE;
                let row = ResourceRow {
                    state: state.id,
                    resource: descriptor.token,
                    pool: cstrategic_resource_pool::from_record(
                        pool[at..at + 12].try_into().unwrap(),
                        descriptor.token,
                    )?,
                    additive: value(triple[0])?,
                    factor: value(triple[1])?,
                    cost: value(triple[2])?,
                };
                if row.pool != 0 || row.additive != 0 || row.factor != 0 || row.cost != 0 {
                    if let Some(slot) = resource_rows.get_mut(first_row + rows) {
                        *slot = row;
                    }
                    rows += 1;
                }
            }
            totals.demand[0] += rows;
            need(&mut required.resource_rows, totals.demand[0]);
            let entries = container.len() / dynamic_modifier_entry::STRIDE;
            totals.demand[1] += entries;
            need(&mut required.modifier_rows, totals.demand[1]);
            grow(inputs, totals.lens[0] + 1, growing)?;
            grow(modifiers, totals.lens[2] + entries, growing)?;
            let mut infrastructure_modifier = 0;
            if state.origin && state.level.is_some() {
                let accumulator = address(state.pointer, cstate::BASE_MODIFIERS)?;
                // An empty first bucket means no country entry: the base
                // accumulator; anything else takes the full probe.
                let selected = if distances[probed[row] * 8] == 0 {
                    accumulator
                } else {
                    cstate::country_modifier_accumulator(
                        memory,
                        game_state,
                        state.pointer,
                        state.controller,
                    )?
                };
                infrastructure_modifier = if selected == accumulator {
                    value(INFRASTRUCTURE_KEY)?
                } else {
                    modifier_accumulator::lookup_effective_at(
                        memory,
                        base,
                        selected,
                        INFRASTRUCTURE_KEY,
                    )?
                    .effective
                };
            }
            let fits = totals.lens[0] < inputs.len()
                && first_row + rows <= resource_rows.len()
                && totals.lens[2] + entries <= modifiers.len();
            // Modifier names: measure every one, copy only when all fit.
            let mut token_end = totals.token_bytes;
            for ordinal in 0..entries {
                let entry = &container[ordinal * dynamic_modifier_entry::STRIDE..];
                let (scope_country, scope_state, definition) = modifier_entry(base, entry)?;
                let name = names.get(memory, base, definition, required)?;
                let len = name.len();
                totals.demand[2] += len;
                let start = token_end;
                token_end += len;
                grow(tokens, token_end, growing)?;
                if !fits || token_end > tokens.len() {
                    continue;
                }
                match name {
                    Name::Kept(at, len) => {
                        tokens[start..token_end].copy_from_slice(&names.bytes[at..at + len])
                    }
                    Name::Source(source, _) => {
                        if !read_slice(memory, source, &mut tokens[start..token_end]) {
                            return Err("state dynamic modifier name unreadable");
                        }
                    }
                }
                modifiers[totals.lens[2] + ordinal] = ModifierRow {
                    state: state.id,
                    ordinal: ordinal as u32,
                    scope_country,
                    scope_state,
                    token_start: start,
                    token_len: len,
                };
            }
            need(&mut required.token_bytes, totals.demand[2]);
            if !fits || token_end > tokens.len() {
                totals.unread_states += 1;
                continue;
            }
            totals.token_bytes = token_end;
            totals.lens[1] += rows;
            totals.lens[2] += entries;
            inputs[totals.lens[0]] = InputRow {
                state: state.id,
                gate: state.fields[STATE_GATE],
                global_factor: value(GLOBAL_FACTOR_KEY)?,
                origin: u8::from(state.origin),
                infrastructure_level: state.level.filter(|_| state.origin),
                infrastructure_modifier,
                supply_route: u8::from(state.origin && state.supply),
            };
            totals.lens[0] += 1;
        }
    }
    Ok(())
}

/// The gathered pointer vector words as non-null addresses.
fn non_null(bytes: &[u8], addresses: &mut [usize]) -> Result<(), &'static str> {
    for (index, address) in addresses.iter_mut().enumerate() {
        *address = usize::try_from(u64_at(bytes, index * 8))
            .ok()
            .filter(|&pointer| pointer != 0)
            .ok_or("pointer vector holds null")?;
    }
    Ok(())
}

/// The first country-modifier probe for the state's controller: the bucket
/// address, with the header's bounds checks.
fn first_bucket(state: &Scratch, census: &Census<'_>) -> Result<usize, &'static str> {
    let header = &state.fields[STATE_HASH..STATE_HASH + 17];
    let data = u64_at(header, 0) as usize;
    let mask = u32_at(header, 12);
    mask.checked_add(1)
        .filter(|count| count.is_power_of_two() && *count <= i32::MAX as u32)
        .ok_or("state movement hash mask invalid")?;
    // The first probe, `hash & mask`, is below the bucket count and so
    // inside the allocation; only a null table has no bucket.
    let index = census.canonical(state.controller)? & mask;
    if data == 0 {
        return Err("state movement hash probe outside allocation");
    }
    data.checked_add(index as usize * cstate::COUNTRY_MODIFIER_STRIDE)
        .ok_or("state movement hash bucket address overflow")
}

/// Infrastructure levels and supply routes of up to `BATCH` origin states
/// whose controllers' routes are in `routes`.
#[allow(clippy::too_many_arguments)]
fn origin_states(
    memory: &impl Memory,
    base: usize,
    infrastructure: Infrastructure,
    members: &[u32],
    states: &mut [Scratch],
    nations: &[Nation],
    routes: &[u8],
    bytes: &mut Vec<u8>,
    words: &mut Vec<u64>,
    addresses: &mut Vec<usize>,
    required: &mut Capacity,
    totals: &mut Totals,
    growing: bool,
) -> Result<(), &'static str> {
    if members.is_empty() {
        return Ok(());
    }
    let levels = infrastructure_levels(memory, infrastructure, members, states, bytes, addresses)?;
    route_supply(
        memory,
        base,
        members,
        states,
        nations,
        routes,
        bytes,
        words,
        addresses,
        &mut required.provinces,
        &mut totals.unread_states,
        growing,
    )?;
    for (row, &index) in members.iter().enumerate() {
        let state = &mut states[index as usize];
        if !state.skipped {
            state.level = levels[row];
        }
    }
    Ok(())
}

/// Infrastructure lookup for up to `BATCH` states: the index, building pointer, template and level;
/// `None` without a building.
fn infrastructure_levels(
    memory: &impl Memory,
    infrastructure: Infrastructure,
    members: &[u32],
    states: &[Scratch],
    bytes: &mut [u8],
    addresses: &mut [usize],
) -> Result<[Option<i32>; BATCH], &'static str> {
    let mut levels = [None; BATCH];
    let mut rows = [0_usize; BATCH];
    let mut len = 0;
    for (row, &index) in members.iter().enumerate() {
        let state = &states[index as usize];
        let indices = Array::from_header(state.header(STATE_INDICES), 4)
            .ok_or("state building indices invalid")?;
        if !infrastructure.lookup_enabled || indices.count == 0 {
            continue;
        }
        addresses[len] = indices
            .element(infrastructure.ordinal, 4)
            .ok_or("infrastructure ordinal outside the state's building indices")?;
        rows[len] = row;
        len += 1;
    }
    records(
        memory,
        &addresses[..len],
        &INDEX_LAYOUT,
        bytes,
        "infrastructure ordinal outside the state's building indices",
    )?;
    let mut found = 0;
    for position in 0..len {
        let index = u32_at(bytes, position * 8) as i32;
        if index == cbuilding_status::NO_BUILDING_INDEX {
            continue;
        }
        let row = rows[position];
        let buildings =
            Array::from_header(states[members[row] as usize].header(STATE_BUILDINGS), 8)
                .ok_or("state buildings invalid")?;
        addresses[found] = u32::try_from(index)
            .ok()
            .and_then(|index| buildings.element(index, 8))
            .ok_or("infrastructure index outside the state's buildings")?;
        rows[found] = row;
        found += 1;
    }
    records(
        memory,
        &addresses[..found],
        &POINTER_LAYOUT,
        bytes,
        "infrastructure index outside the state's buildings",
    )?;
    let mut built = 0;
    for position in 0..found {
        let building = usize::try_from(u64_at(bytes, position * 8))
            .map_err(|_| "infrastructure index outside the state's buildings")?;
        if building == 0 {
            continue;
        }
        addresses[built] = building;
        rows[built] = rows[position];
        built += 1;
    }
    records(
        memory,
        &addresses[..built],
        &BUILDING_LAYOUT,
        bytes,
        "infrastructure building unreadable",
    )?;
    for position in 0..built {
        if u64_at(bytes, position * 16) != infrastructure.template as u64 {
            return Err("infrastructure building template mismatch");
        }
        levels[rows[position]] = Some(i32::from(u16::from_le_bytes(
            bytes[position * 16 + 8..position * 16 + 10]
                .try_into()
                .unwrap(),
        ) as i16));
    }
    Ok(levels)
}

/// Positive maximum province-node value for up to `BATCH` states whose
/// controllers' routes are in `routes`, reading province vectors and IDs in
/// groups that fit. A state whose provinces alone do not fit is skipped,
/// counted in `unread` and its province count kept in `provinces`.
#[allow(clippy::too_many_arguments)]
fn route_supply(
    memory: &impl Memory,
    base: usize,
    members: &[u32],
    states: &mut [Scratch],
    nations: &[Nation],
    routes: &[u8],
    bytes: &mut Vec<u8>,
    words: &mut Vec<u64>,
    addresses: &mut Vec<usize>,
    provinces_required: &mut usize,
    unread: &mut u32,
    growing: bool,
) -> Result<(), &'static str> {
    let mut spans = [(0, 0); BATCH];
    let mut kept = [0_usize; BATCH];
    let mut counts = [0_usize; BATCH];
    let mut len = 0;
    let mut total = 0;
    for (row, &index) in members.iter().enumerate() {
        let provinces = Array::from_header(states[index as usize].header(STATE_PROVINCES), 8)
            .ok_or("state provinces invalid")?;
        let count = provinces.count as usize;
        need(provinces_required, count);
        // The maximum node value requires the full province vector.
        grow(words, total + count, growing)?;
        grow(addresses, total + count, growing)?;
        grow(bytes, (total + count) * 16, growing)?;
        if total + count > words.len() || (total + count) * 16 > bytes.len() {
            if count > words.len() || count * 16 > bytes.len() {
                states[index as usize].skipped = true;
                *unread += 1;
                continue;
            }
            // Fits alone: finish these, then take the rest.
            origin_states_supply(
                memory,
                base,
                &spans[..len],
                &kept[..len],
                &counts[..len],
                total,
                members,
                states,
                nations,
                routes,
                bytes,
                words,
                addresses,
            )?;
            len = 0;
            total = 0;
        }
        spans[len] = (provinces.data, count * 8);
        kept[len] = row;
        counts[len] = count;
        total += count;
        len += 1;
    }
    origin_states_supply(
        memory,
        base,
        &spans[..len],
        &kept[..len],
        &counts[..len],
        total,
        members,
        states,
        nations,
        routes,
        bytes,
        words,
        addresses,
    )
}

/// Positive maximum province-node value for states whose province vectors are `spans`: a province's
/// route node has a positive route value.
#[allow(clippy::too_many_arguments)]
fn origin_states_supply(
    memory: &impl Memory,
    base: usize,
    spans: &[(usize, usize)],
    kept: &[usize],
    counts: &[usize],
    total: usize,
    members: &[u32],
    states: &mut [Scratch],
    nations: &[Nation],
    routes: &[u8],
    bytes: &mut [u8],
    words: &mut [u64],
    addresses: &mut [usize],
) -> Result<(), &'static str> {
    if spans.is_empty() {
        return Ok(());
    }
    gather(memory, spans, bytes, "pointer vector unreadable")?;
    non_null(bytes, &mut addresses[..total])?;
    let vtable = address(base, cprovince::VTABLE_RVA)? as u64;
    records(
        memory,
        &addresses[..total],
        &PROVINCE_LAYOUT,
        bytes,
        "state province unreadable",
    )?;
    let mut province = 0;
    let mut found = 0;
    let mut owners = [0_usize; BATCH];
    let mut firsts = [0_usize; BATCH];
    for (position, (&row, &count)) in kept.iter().zip(counts).enumerate() {
        let state = &states[members[row] as usize];
        let nation = &nations[state.slot as usize];
        let pairs = nation.routes.count as usize;
        let routes = &routes[nation.route_start * 8..(nation.route_start + pairs) * 8];
        firsts[position] = found;
        owners[position] = row;
        for _ in 0..count {
            if u64_at(bytes, province * 16) != vtable {
                return Err("state province vtable mismatch");
            }
            let id = u32_at(bytes, province * 16 + 8) as i32;
            province += 1;
            // Lower bound over the sorted (province, node) pairs.
            let (mut low, mut high) = (0, pairs);
            while low < high {
                let middle = low + (high - low) / 2;
                if (u32_at(routes, middle * 8) as i32) < id {
                    low = middle + 1;
                } else {
                    high = middle;
                }
            }
            if low == pairs || u32_at(routes, low * 8) as i32 != id {
                continue;
            }
            let node = u32_at(routes, low * 8 + 4);
            if node as i32 == -1 {
                continue;
            }
            // The province addresses are consumed; node addresses reuse them.
            addresses[found] = nation
                .nodes
                .element(node, supply_node::STRIDE)
                .ok_or("supply route node outside the companion's nodes")?;
            words[found] = u64::from(id as u32) << 32 | u64::from(node);
            found += 1;
        }
    }
    // The node records reuse the province record bytes, which are consumed.
    records(
        memory,
        &addresses[..found],
        &NODE_LAYOUT,
        bytes,
        "supply route node unreadable",
    )?;
    for position in 0..kept.len() {
        let end = if position + 1 < kept.len() {
            firsts[position + 1]
        } else {
            found
        };
        let mut positive = false;
        for (node, &expected) in words.iter().enumerate().take(end).skip(firsts[position]) {
            let identity =
                u64::from(u32_at(bytes, node * 16 + 4)) << 32 | u64::from(u32_at(bytes, node * 16));
            if identity != expected {
                return Err("supply route node identity mismatch");
            }
            positive |= u64_at(bytes, node * 16 + 8) as i64 > 0;
        }
        states[members[owners[position]] as usize].supply = positive;
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Infrastructure {
    template: usize,
    lookup_enabled: bool,
    ordinal: u32,
}

fn infrastructure_template(
    memory: &impl Memory,
    base: usize,
) -> Result<Infrastructure, &'static str> {
    let database = read::<u64>(memory, address(base, cbuilding_database::SINGLETON_RVA)?)
        .and_then(|pointer| usize::try_from(pointer).ok())
        .filter(|&pointer| pointer != 0)
        .ok_or("building database unreadable or null")?;
    let template = read::<u64>(memory, address(database, INFRASTRUCTURE_TEMPLATE)?)
        .and_then(|pointer| usize::try_from(pointer).ok())
        .filter(|&pointer| pointer != 0)
        .ok_or("infrastructure template unreadable or null")?;
    Ok(Infrastructure {
        template,
        lookup_enabled: read::<u8>(
            memory,
            address(template, cbuilding_template::LOOKUP_ENABLED_CANDIDATE)?,
        )
        .ok_or("infrastructure lookup flag unreadable")?
            != 0,
        ordinal: read::<u32>(
            memory,
            address(template, cbuilding_template::DATABASE_ORDINAL)?,
        )
        .ok_or("infrastructure ordinal unreadable")?,
    })
}

/// The state container's authenticated entry array: no country or leader, and this state's ID.
fn container_array(base: usize, state: &Scratch) -> Result<Array, &'static str> {
    let bytes = &state.fields[STATE_CONTAINER..STATE_CONTAINER + 0x38];
    if u64_at(bytes, 0) != address(base, cdynamic_modifier_container::VTABLE_RVA)? as u64 {
        return Err("state dynamic modifier container vtable mismatch");
    }
    if u64_at(bytes, cdynamic_modifier_container::COUNTRY) != 0
        || u32_at(bytes, cdynamic_modifier_container::STATE) != state.id
    {
        return Err("state dynamic modifier container owner mismatch");
    }
    Array::from_header(
        header16(bytes, cdynamic_modifier_container::ENTRIES),
        dynamic_modifier_entry::STRIDE,
    )
    .ok_or("state dynamic modifier array invalid")
}

/// Rows for one country's overrides; `None` when they did not fit.
#[allow(clippy::too_many_arguments)]
fn overrides_of(
    memory: &impl Memory,
    entries: Array,
    country: u32,
    descriptors: &[Resource],
    bytes: &mut Vec<u8>,
    mut output: Buffer<'_, OverrideRow>,
    required: &mut Capacity,
    demand: &mut usize,
    growing: bool,
) -> Result<Option<usize>, &'static str> {
    let count = entries.count as usize;
    need(&mut required.entries, count);
    if count == 0 {
        return Ok(Some(0));
    }
    let entry_len = count * ccountry_resources::OVERRIDE_STRIDE;
    grow(bytes, entry_len, growing)?;
    if entry_len > bytes.len() {
        return Ok(None);
    }
    if !read_slice(memory, entries.data, &mut bytes[..entry_len]) {
        return Err("building resource overrides unreadable");
    }
    let mut rows = 0;
    for entry in 0..count {
        let at = entry * ccountry_resources::OVERRIDE_STRIDE;
        let building = u32_at(bytes, at + ccountry_resources::OVERRIDE_BUILDING);
        let list = Array::from_header(
            header16(bytes, at + ccountry_resources::OVERRIDE_RESOURCES),
            ccountry_resources::OVERRIDE_RESOURCE_STRIDE,
        )
        .ok_or("building resource override list invalid")?;
        output.ensure(rows + (list.count as usize).max(1))?;
        let base_row = OverrideRow {
            country,
            entry: entry as u32,
            building,
            count: list.count,
            ..OverrideRow::default()
        };
        if list.count == 0 {
            if let Some(slot) = output.get_mut(rows) {
                *slot = OverrideRow {
                    ordinal: rows as u32,
                    ..base_row
                };
            }
            rows += 1;
            continue;
        }
        for element in 0..list.count {
            let record = read::<[u8; 16]>(
                memory,
                list.element(element, ccountry_resources::OVERRIDE_RESOURCE_STRIDE)
                    .ok_or("override element address overflow")?,
            )
            .ok_or("override element unreadable")?;
            let resource = match u32_at(&record, 0) {
                0 => 0,
                index => {
                    descriptors
                        .get(index as usize - 1)
                        .ok_or("override resource outside the resource database")?
                        .token
                }
            };
            if let Some(slot) = output.get_mut(rows) {
                *slot = OverrideRow {
                    ordinal: rows as u32,
                    resource,
                    amount: u64_at(&record, 8) as i64,
                    ..base_row
                };
            }
            rows += 1;
        }
    }
    *demand += rows;
    need(&mut required.override_rows, *demand);
    Ok((rows <= output.len()).then_some(rows))
}

#[cfg(test)]
mod tests;
