//! Stored raid-offer launch-gate inputs, evaluated per viewing country.
//! The canonical-country pass acquires allocated command power, capacity modifiers
//! and ace-country modifiers for every country. Selected countries additionally
//! supply convoy reservations, stockpile archetype totals, strategic-air site lists
//! and access containers, ace-range operands and autonomous-controller state.
//!
//! Unselected countries reach only accumulator and allocated-power headers and
//! the four [`HOURLY`] keys. Other launch operands belong to the caller's selected
//! daily slice. No game function is called; effective values use the stored
//! definition clamp and eligibility metadata.

use crate::{
    Memory, cace, cair_base, cconvoys, ccountry, ccountry_air_container, cequipment_type,
    cequipment_variant_pool, cgame_state,
    cpdx_array::Array,
    cproduction_status, cstrategic_air, cstrategic_air_manager,
    memory::{RecordField, RecordLayout, RecordSources},
    modifier_accumulator,
    modifier_definition::{DefinitionMemo, effective_memo},
    read,
    walk::Outcome,
};

/// Launch-gate modifier keys in [`CountryRow::modifiers`] order. Fixed numeric
/// indices are checked by the attachment's fixed-layout contract; a mismatch
/// withholds the raid-launch, command-power or ace-country table. They supply air
/// range, naval-commando range, ace range/speed, command-power capacity and ace
/// effectiveness. [`HOURLY`] columns belong to [`CommandPowerRow`] for every
/// country; the remaining columns belong to selected [`CountryRow`] values.
pub const MODIFIER_KEYS: [u32; MODIFIERS] = [
    crate::modifier_keys::AIR_RANGE_FACTOR,
    crate::modifier_keys::NAVAL_COMMANDO_RAID_DISTANCE,
    crate::modifier_keys::AIR_ACE_BONUSES_FACTOR,
    crate::modifier_keys::MAX_COMMAND_POWER,
    crate::modifier_keys::MAX_COMMAND_POWER_MULT,
    crate::modifier_keys::ACE_EFFECTIVENESS_FACTOR,
];
pub const MODIFIERS: usize = 6;
/// [`MODIFIER_KEYS`] position of `air_range_factor`, which the aces also read.
const AIR_RANGE: usize = 0;
/// [`MODIFIER_KEYS`] positions of the command power capacity pair.
pub const MAX_COMMAND_POWER: usize = 3;
pub const MAX_COMMAND_POWER_MULT: usize = 4;
/// [`MODIFIER_KEYS`] positions of the ace country modifiers.
pub const AIR_ACE_BONUSES: usize = 2;
pub const ACE_EFFECTIVENESS: usize = 5;
/// The [`MODIFIER_KEYS`] positions read for every country each walk: the
/// per-country modifier totals the raid code reads live.
pub const HOURLY: [usize; 4] = [
    AIR_ACE_BONUSES,
    MAX_COMMAND_POWER,
    MAX_COMMAND_POWER_MULT,
    ACE_EFFECTIVENESS,
];

/// The launch keys the walk reads. `None` leaves a column unknown; the
/// collector always passes [`Keys::FIXED`], and tests leave keys out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Keys {
    pub keys: [Option<u32>; MODIFIERS],
}

impl Keys {
    /// Every launch key at its fixed index.
    pub const FIXED: Self = Self {
        keys: [
            Some(MODIFIER_KEYS[0]),
            Some(MODIFIER_KEYS[1]),
            Some(MODIFIER_KEYS[2]),
            Some(MODIFIER_KEYS[3]),
            Some(MODIFIER_KEYS[4]),
            Some(MODIFIER_KEYS[5]),
        ],
    };

    /// The registered keys of the [`MODIFIER_KEYS`] positions `wanted`
    /// accepts, ascending with their positions.
    fn sorted(&self, wanted: impl Fn(usize) -> bool) -> ([(u32, usize); MODIFIERS], usize) {
        let mut sorted = [(0, 0); MODIFIERS];
        let mut len = 0;
        for (column, key) in self.keys.iter().enumerate() {
            if let Some(key) = key
                && wanted(column)
            {
                sorted[len] = (*key, column);
                len += 1;
            }
        }
        sorted[..len].sort_unstable();
        (sorted, len)
    }
}

/// Ace-type definition whose embedded modifier accumulator supplies range terms.
pub const ACE_TYPE_DEFINITION: usize = 0x170;
pub const ACE_TYPE_MODIFIERS: usize = 0x38 + 0x10;
/// A convoy reservation: `{ i32 priority, i32 quantity }`.
const RESERVATION: usize = 8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CountryRow {
    pub canonical_slot: u32,
    /// Effective values for [`MODIFIER_KEYS`], with `None` for an unrequested key.
    /// [`HOURLY`] columns belong to [`CommandPowerRow`] and are omitted by conversion.
    pub modifiers: [Option<i64>; MODIFIERS],
    /// `CGameState+0x370[slot]`; `None` past the controller arrays.
    pub autonomous: Option<u32>,
    /// Flattened `(priority, quantity)` pairs in the caller's arena.
    pub reservations_start: usize,
    pub reservations_len: usize,
}

/// Every country's command power operands and ace country modifiers, each
/// walk.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommandPowerRow {
    pub canonical_slot: u32,
    /// `CCountry+0x1F8`.
    pub allocated_raw: i64,
    /// `max_command_power` and `max_command_power_mult`; `None` for a key
    /// the walk was not given.
    pub maximum: Option<i64>,
    pub maximum_factor: Option<i64>,
    /// `ace_effectiveness_factor` and `air_ace_bonuses_factor`, likewise.
    pub ace_effectiveness: Option<i64>,
    pub ace_bonuses: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ArchetypeRow {
    pub canonical_slot: u32,
    /// The archetype's lexer token (`CEquipmentType + 8`).
    pub archetype: u32,
    pub quantity_raw: i64,
}

/// One site of a country's `CStrategicAir + 0x10` (list 0) or `+ 0x28` (list 1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SiteRow {
    pub canonical_slot: u32,
    /// Position across both lists, list 0 first.
    pub ordinal: u32,
    pub list: u8,
    pub base: u64,
    /// The country's container capacity at the site, `None` without one.
    pub container_capacity_raw: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AceRow {
    pub ace: u64,
    pub canonical_slot: u32,
    pub wing: u64,
    pub alive: u8,
    /// Effective key 5 on the ace's own modifiers (`+0x30`).
    pub range: i64,
    /// Effective key 5 on its type definition's modifiers.
    pub type_range: i64,
}

pub struct Buffers<'a> {
    pub memo: &'a DefinitionMemo,
    pub keys: &'a Keys,
    pub countries: crate::walk::Buffer<'a, CountryRow>,
    pub command_power: crate::walk::Buffer<'a, CommandPowerRow>,
    pub reservations: crate::walk::Buffer<'a, u32>,
    pub archetypes: crate::walk::Buffer<'a, ArchetypeRow>,
    pub sites: crate::walk::Buffer<'a, SiteRow>,
    pub aces: crate::walk::Buffer<'a, AceRow>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Walk {
    pub countries: Outcome,
    pub command_power: Outcome,
    pub archetypes: Outcome,
    pub sites: Outcome,
    pub aces: Outcome,
    pub reservations_len: usize,
    pub required_reservations: usize,
}

#[derive(Default)]
struct Bounded {
    len: usize,
    unread: u32,
}

impl Bounded {
    fn push<T: Default>(
        &mut self,
        output: &mut crate::walk::Buffer<'_, T>,
        row: T,
    ) -> Result<(), &'static str> {
        output.ensure(self.len.checked_add(1).ok_or("raid input count overflow")?)?;
        match output.get_mut(self.len) {
            Some(slot) => {
                *slot = row;
                self.len += 1;
            }
            None => {
                self.unread = self
                    .unread
                    .checked_add(1)
                    .ok_or("raid input count overflow")?
            }
        }
        Ok(())
    }

    fn outcome(&self) -> Outcome {
        if self.unread == 0 {
            Outcome::Rows { len: self.len }
        } else {
            Outcome::Truncated {
                len: self.len,
                unread_rows: self.unread,
            }
        }
    }
}

fn field<T: crate::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
    what: &'static str,
) -> Result<T, &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|address| read::<T>(memory, address))
        .ok_or(what)
}

fn pointer(
    memory: &impl Memory,
    at: usize,
    offset: usize,
    what: &'static str,
) -> Result<usize, &'static str> {
    usize::try_from(field::<u64>(memory, at, offset, what)?).map_err(|_| what)
}

fn array(
    memory: &impl Memory,
    at: usize,
    offset: usize,
    stride: usize,
    what: &'static str,
) -> Result<Array, &'static str> {
    at.checked_add(offset)
        .and_then(|address| Array::read(memory, address, stride))
        .ok_or(what)
}

fn element(
    memory: &impl Memory,
    array: Array,
    index: u32,
    what: &'static str,
) -> Result<usize, &'static str> {
    array
        .element(index, 8)
        .and_then(|address| read::<u64>(memory, address))
        .and_then(|value| usize::try_from(value).ok())
        .ok_or(what)
}

fn vtable(
    memory: &impl Memory,
    base: usize,
    at: usize,
    rva: usize,
    what: &'static str,
) -> Result<(), &'static str> {
    let expected = base.checked_add(rva).ok_or("module address overflow")?;
    (read::<u64>(memory, at) == Some(expected as u64))
        .then_some(())
        .ok_or(what)
}

/// Effective lookup on an accumulator.
fn effective(
    memory: &impl Memory,
    base: usize,
    accumulator: usize,
    key: u32,
    memo: &DefinitionMemo,
) -> Result<i64, &'static str> {
    let value = modifier_accumulator::lookup_at(memory, accumulator, key)?;
    if !value.present {
        return Ok(0);
    }
    effective_memo(memory, base, key, value.raw, memo)
}

/// Pointers, rows and records copied per guarded call.
const BATCH: usize = 64;

const fn layout(fields: &'static [RecordField], stride: usize) -> RecordLayout<'static> {
    match RecordLayout::new(fields, stride) {
        Some(layout) => layout,
        None => panic!("invalid record layout"),
    }
}

/// A reservation's `{ priority, quantity }`.
const RESERVATION_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: RESERVATION,
}];
const RESERVATION_LAYOUT: RecordLayout<'static> = layout(&RESERVATION_FIELDS, RESERVATION);
/// A container's vtable and capacity.
const CONTAINER_FIELDS: [RecordField; 2] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: ccountry_air_container::CAPACITY_RAW,
        destination_offset: 8,
        len: 8,
    },
];
const CONTAINER_LAYOUT: RecordLayout<'static> = layout(&CONTAINER_FIELDS, 16);
/// An ace's vtable and reference, wing, alive byte and type definition.
const ACE_FIELDS: [RecordField; 4] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 16,
    },
    RecordField {
        source_offset: cace::ASSIGNED_WING_REFERENCE,
        destination_offset: 16,
        len: 8,
    },
    RecordField {
        source_offset: cace::ALIVE,
        destination_offset: 24,
        len: 1,
    },
    RecordField {
        source_offset: ACE_TYPE_DEFINITION,
        destination_offset: 25,
        len: 8,
    },
];
const ACE_BYTES: usize = 33;
const ACE_LAYOUT: RecordLayout<'static> = layout(&ACE_FIELDS, ACE_BYTES);

/// Successful original shared dependencies of one acquisition.
#[derive(Default)]
struct Shared<T> {
    values: std::collections::HashMap<usize, T>,
}
impl<T: Copy> Shared<T> {
    fn new() -> Self {
        Self {
            values: std::collections::HashMap::new(),
        }
    }
    fn get_or(
        &mut self,
        key: usize,
        load: impl FnOnce() -> Result<T, &'static str>,
    ) -> Result<T, &'static str> {
        if let Some(&value) = self.values.get(&key) {
            return Ok(value);
        }
        let value = load()?;
        crate::allocation::reserve_map(&mut self.values, 1)
            .map_err(|_| "raid shared dependency allocation failed")?;
        self.values.insert(key, value);
        Ok(value)
    }
}

/// Lower-bound search of a sorted accumulator for each registered launch key,
/// ascending by column. `probe` uses the owned copy or one guarded read per probe
/// when the array exceeds [`ACCUMULATOR_ENTRIES`].
fn search(
    count: u32,
    mut probe: impl FnMut(u32) -> Result<[u8; 16], &'static str>,
    sorted: &[(u32, usize)],
    out: &mut [modifier_accumulator::Value; MODIFIERS],
) -> Result<(), &'static str> {
    let mut lower = 0;
    for &(key, column) in sorted {
        let value = &mut out[column];
        // Keys ascend, so each search starts where the previous one ended.
        let mut remaining = count - lower;
        let mut found = None;
        while remaining > 0 {
            let half = remaining / 2;
            let entry = probe(lower + half)?;
            if u32::from_le_bytes(entry[..4].try_into().unwrap()) < key {
                lower += half + 1;
                remaining -= half + 1;
            } else {
                found = Some(entry);
                remaining = half;
            }
        }
        *value = match found {
            Some(entry) if u32::from_le_bytes(entry[..4].try_into().unwrap()) == key => {
                modifier_accumulator::Value {
                    raw: i64::from_le_bytes(entry[8..].try_into().unwrap()),
                    present: true,
                }
            }
            _ => modifier_accumulator::Value::default(),
        };
    }
    Ok(())
}

/// Copies the pointer array's elements `first..` into `out`, up to its length.
fn pointers(
    memory: &impl Memory,
    array: Array,
    first: u32,
    out: &mut [usize; BATCH],
    what: &'static str,
) -> Result<usize, &'static str> {
    let count = ((array.count - first) as usize).min(BATCH);
    let source = array.slice(first, count as u32, 8).map_err(|_| what)?;
    // Copy the whole requested span before callers validate entries; a short read still wins.
    crate::cpdx_array::pointers(memory, source, what, |index, pointer| {
        out[index as usize] = pointer;
        Ok(())
    })?;
    Ok(count)
}

/// Up to [`BATCH`] spans `(address, len)`, back to back in `destination`, in
/// one guarded call; returns how many leading spans were copied.
fn gather(memory: &impl Memory, spans: &[(usize, usize)], destination: &mut [u8]) -> usize {
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
    memory
        .copy_scattered_prefetched(addresses, &mut slices)
        .min(spans.len())
}

/// A country's production status pointer.
const PRODUCTION_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: ccountry::PRODUCTION_STATUS,
    destination_offset: 0,
    len: 8,
}];
const PRODUCTION_LAYOUT: RecordLayout<'static> = layout(&PRODUCTION_FIELDS, 8);
/// `cproduction_status::manager`'s vtable and country, then
/// `cequipment_variant_pool::allocation_entries`' pool vtable and header.
const MANAGER_FIELDS: [RecordField; 4] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: cproduction_status::COUNTRY,
        destination_offset: 8,
        len: 8,
    },
    RecordField {
        source_offset: cproduction_status::STOCKPILE_POOL,
        destination_offset: 16,
        len: 8,
    },
    RecordField {
        source_offset: cproduction_status::STOCKPILE_POOL + cequipment_variant_pool::ALLOCATIONS,
        destination_offset: 24,
        len: 16,
    },
];
const MANAGER_BYTES: usize = 40;
const MANAGER_LAYOUT: RecordLayout<'static> = layout(&MANAGER_FIELDS, MANAGER_BYTES);
/// Allocation rows one guarded gather copies; a longer pool is read alone.
const STOCK_ROWS: usize = 1024;
const ROW: usize = crate::allocation_equipment_row::STRIDE;

/// StockpileArchetypes for the selected canonical countries: 64 countries per guarded
/// call for the production pointers and for the managers and pools, and the
/// allocation rows of consecutive countries gathered up to [`STOCK_ROWS`]
/// per guarded call. Each country keeps the failure order and messages of
/// `cproduction_status::manager` and `cequipment_variant_pool::allocation_entries`.
fn stockpiles(
    memory: &impl Memory,
    base: usize,
    reader: &ccountry::Reader<'_, impl Memory>,
    selected: &dyn Fn(u32) -> bool,
    archetypes: &mut Bounded,
    output: &mut crate::walk::Buffer<'_, ArchetypeRow>,
) -> Result<(), &'static str> {
    let module = |rva: usize| {
        base.checked_add(rva)
            .map(|address| address as u64)
            .ok_or("module address overflow")
    };
    let archetype_vtable = module(cequipment_type::VTABLE_RVA)?;
    let production_vtable = module(cproduction_status::VTABLE_RVA)?;
    let pool_vtable = module(cequipment_variant_pool::VTABLE_RVA)?;
    // Archetype tokens, shared across countries.
    let mut tokens = Shared::<u32>::new();
    let mut rows = [0_u8; STOCK_ROWS * ROW];
    let mut slot = 1;
    while slot < reader.count() {
        let mut slots = [0_u32; BATCH];
        let mut countries = [0_usize; BATCH];
        let mut len = 0;
        while slot < reader.count() && len < BATCH {
            if !selected(slot) {
                slot += 1;
                continue;
            }
            if let Some(country) = reader.at(slot)? {
                slots[len] = slot;
                countries[len] = country.pointer;
                len += 1;
            }
            slot += 1;
        }
        // Each country's production status, up to the first unreadable one.
        let mut words = [0_u8; BATCH * 8];
        let copied = memory.copy_records(
            RecordSources::Pointers(&countries[..len]),
            &PRODUCTION_LAYOUT,
            &mut words[..len * 8],
        );
        let mut managers = [0_usize; BATCH];
        let mut read = len;
        for (index, manager) in managers[..len].iter_mut().enumerate() {
            if index < copied {
                *manager = usize::try_from(u64::from_le_bytes(
                    words[index * 8..index * 8 + 8].try_into().unwrap(),
                ))
                .unwrap_or(0);
            }
            if *manager == 0 {
                read = index;
                break;
            }
        }
        // Their managers and pools, then each country's allocation array or
        // its first failure.
        let mut records = [0_u8; BATCH * MANAGER_BYTES];
        let completed = memory.copy_records(
            RecordSources::Pointers(&managers[..read]),
            &MANAGER_LAYOUT,
            &mut records[..read * MANAGER_BYTES],
        );
        let mut pools = [Err("production status pointer unreadable or null"); BATCH];
        for (index, pool) in pools[..read].iter_mut().enumerate() {
            let record = &records[index * MANAGER_BYTES..][..MANAGER_BYTES];
            let fields = completed
                .saturating_sub(index * MANAGER_FIELDS.len())
                .min(MANAGER_FIELDS.len());
            let word = |at: usize| u64::from_le_bytes(record[at..at + 8].try_into().unwrap());
            *pool = if fields == 0 || word(0) != production_vtable {
                Err("production status vtable mismatch")
            } else if fields < 2 || word(8) != countries[index] as u64 {
                Err("production status country mismatch")
            } else if fields < 3 {
                Err("equipment pool vtable unreadable")
            } else if word(16) != pool_vtable {
                Err("equipment pool vtable mismatch")
            } else {
                (fields == MANAGER_FIELDS.len())
                    .then(|| Array::from_header(record[24..40].try_into().unwrap(), ROW))
                    .flatten()
                    .ok_or("equipment pool array invalid or unreadable")
            };
            if pool.is_err() {
                break;
            }
        }
        // The rows of consecutive countries that fit, in slot order.
        let mut next = 0;
        while next < len {
            let pool = pools[next]?;
            if pool.count as usize > STOCK_ROWS {
                // A long pool reads alone, in chunks.
                let mut first = 0;
                while first < pool.count {
                    let count = ((pool.count - first) as usize).min(STOCK_ROWS);
                    let address = pool
                        .element(first, ROW)
                        .ok_or("stockpile allocation address overflow")?;
                    if !memory.copy(address, &mut rows[..count * ROW]) {
                        return Err("equipment allocation unreadable");
                    }
                    emit(
                        memory,
                        &rows[..count * ROW],
                        slots[next],
                        archetype_vtable,
                        &mut tokens,
                        archetypes,
                        output,
                    )?;
                    first += count as u32;
                }
                next += 1;
                continue;
            }
            let mut spans = [(0_usize, 0_usize); BATCH];
            let mut group = 0;
            let mut total = 0;
            while next + group < len {
                let Ok(pool) = pools[next + group] else {
                    break;
                };
                let bytes = pool.count as usize * ROW;
                if total + bytes > rows.len() {
                    break;
                }
                spans[group] = (pool.data, bytes);
                total += bytes;
                group += 1;
            }
            let copied = gather(memory, &spans[..group], &mut rows[..total]);
            let mut at = 0;
            for index in 0..group {
                if index >= copied {
                    return Err("equipment allocation unreadable");
                }
                let bytes = spans[index].1;
                emit(
                    memory,
                    &rows[at..at + bytes],
                    slots[next + index],
                    archetype_vtable,
                    &mut tokens,
                    archetypes,
                    output,
                )?;
                at += bytes;
            }
            next += group;
        }
    }
    Ok(())
}

/// One country's allocation rows as StockpileArchetypes rows.
fn emit(
    memory: &impl Memory,
    rows: &[u8],
    slot: u32,
    archetype_vtable: u64,
    tokens: &mut Shared<u32>,
    archetypes: &mut Bounded,
    output: &mut crate::walk::Buffer<'_, ArchetypeRow>,
) -> Result<(), &'static str> {
    for row in rows.chunks_exact(ROW) {
        let archetype = usize::try_from(u64::from_le_bytes(row[..8].try_into().unwrap()))
            .map_err(|_| "equipment archetype pointer overflow")?;
        let token = tokens.get_or(archetype, || {
            let identity = read::<[u8; 12]>(memory, archetype)
                .filter(|_| archetype != 0)
                .ok_or("equipment archetype vtable unreadable")?;
            if u64::from_le_bytes(identity[..8].try_into().unwrap()) != archetype_vtable {
                return Err("equipment archetype vtable mismatch");
            }
            Ok(u32::from_le_bytes(identity[8..].try_into().unwrap()))
        })?;
        archetypes.push(
            output,
            ArchetypeRow {
                canonical_slot: slot,
                archetype: token,
                quantity_raw: i64::from_le_bytes(
                    row[crate::allocation_equipment_row::AMOUNT..ROW]
                        .try_into()
                        .unwrap(),
                ),
            },
        )?;
    }
    Ok(())
}

/// A country's headers in the order the walk uses them: the modifier
/// accumulator and the allocated command power (every country), then the
/// convoy reservations and the ace roster (a selected country).
const SELECTED_FIELDS: [RecordField; 4] = [
    RecordField {
        source_offset: ccountry::MODIFIER_ACCUMULATOR,
        destination_offset: 0,
        len: 16,
    },
    RecordField {
        source_offset: ccountry::ALLOCATED_COMMAND_POWER,
        destination_offset: 16,
        len: 8,
    },
    RecordField {
        source_offset: ccountry::CONVOY_STATE + cconvoys::RESERVATIONS,
        destination_offset: 24,
        len: 16,
    },
    RecordField {
        source_offset: ccountry::ACES,
        destination_offset: 40,
        len: 16,
    },
];
const SELECTED_BYTES: usize = 56;
const SELECTED_LAYOUT: RecordLayout<'static> = layout(&SELECTED_FIELDS, SELECTED_BYTES);
/// An unselected country's headers: the accumulator and allocated power only.
const COMMAND_POWER_FIELDS: [RecordField; 2] = [SELECTED_FIELDS[0], SELECTED_FIELDS[1]];
const COMMAND_POWER_LAYOUT: RecordLayout<'static> = layout(&COMMAND_POWER_FIELDS, 24);
/// Accumulator entries a selected country's search copies in one guarded
/// call; a longer accumulator is searched one probe per guarded call.
const ACCUMULATOR_ENTRIES: usize = 1024;
/// A strategic-air object's vtable and both site list headers.
const STRATEGIC_FIELDS: [RecordField; 3] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: cstrategic_air::FIRST_AIR_SITES,
        destination_offset: 8,
        len: 16,
    },
    RecordField {
        source_offset: cstrategic_air::SECOND_AIR_SITES,
        destination_offset: 24,
        len: 16,
    },
];
const STRATEGIC_BYTES: usize = 40;
const STRATEGIC_LAYOUT: RecordLayout<'static> = layout(&STRATEGIC_FIELDS, STRATEGIC_BYTES);
/// A site's vtable and handle, then its country container table header.
const SITE_FIELDS: [RecordField; 2] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 16,
    },
    RecordField {
        source_offset: cair_base::COUNTRY_CONTAINERS,
        destination_offset: 16,
        len: 16,
    },
];
const SITE_BYTES: usize = 32;
const SITE_LAYOUT: RecordLayout<'static> = layout(&SITE_FIELDS, SITE_BYTES);
/// One pointer at its address.
const POINTER_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: 8,
}];
const POINTER_LAYOUT: RecordLayout<'static> = layout(&POINTER_FIELDS, 8);

/// The walk's rows and arena length.
#[derive(Default)]
struct Sliced {
    countries: Bounded,
    command_power: Bounded,
    sites: Bounded,
    aces: Bounded,
    reservations_len: usize,
}

/// CountryCommandPower for every canonical slot, and RaidCountryInputs,
/// RaidAirSites and AceRangeInputs for the slots `selected` accepts, in slot
/// order.
#[allow(clippy::too_many_arguments)]
fn sliced(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    reader: &ccountry::Reader<'_, impl Memory>,
    buffers: &mut Buffers<'_>,
    selected: &dyn Fn(u32) -> bool,
    rows: &mut Sliced,
    required_reservations: &mut usize,
) -> Result<(), &'static str> {
    let memo = buffers.memo;
    let keys = *buffers.keys;
    // A selected country evaluates every launch key; the others only the
    // hourly ones.
    let (every, every_len) = keys.sorted(|_| true);
    let (power, power_len) = keys.sorted(|column| HOURLY.contains(&column));
    let (every, power) = (&every[..every_len], &power[..power_len]);
    // The aces read `air_range_factor`: without it their table is unknown.
    let air_range = keys.keys[AIR_RANGE];
    let module = |rva: usize| {
        base.checked_add(rva)
            .map(|address| address as u64)
            .ok_or("module address overflow")
    };
    let (site_vtable, container_vtable, ace_vtable, strategic_vtable) = (
        module(cair_base::VTABLE_RVA)?,
        module(ccountry_air_container::VTABLE_RVA)?,
        module(cace::VTABLE_RVA)?,
        module(cstrategic_air::VTABLE_RVA)?,
    );
    // Ace type ranges, shared across countries.
    let mut type_ranges = Shared::<i64>::new();
    let mut list = [0_usize; BATCH];
    let humans = array(
        memory,
        game_state,
        cgame_state::HUMAN_STATUS,
        1,
        "human-controller counts invalid",
    )?;
    let autonomous = array(
        memory,
        game_state,
        cgame_state::AUTONOMOUS_STATUS,
        1,
        "autonomous flags invalid",
    )?;
    let manager = pointer(
        memory,
        game_state,
        cgame_state::STRATEGIC_AIR_MANAGER,
        "strategic air manager unreadable",
    )?;
    vtable(
        memory,
        base,
        manager,
        cstrategic_air_manager::VTABLE_RVA,
        "strategic air manager vtable mismatch",
    )?;
    let air = array(
        memory,
        manager,
        cstrategic_air_manager::COUNTRY_AIR,
        8,
        "strategic air country table invalid",
    )?;
    const STRIDE: usize = crate::modifier_entry::STRIDE;
    let mut entries = [0_u8; ACCUMULATOR_ENTRIES * STRIDE];
    for slot in 1..reader.count() {
        let Some(country) = reader.at(slot)? else {
            continue;
        };
        // The sliced tables read only the selected countries.
        let chosen = selected(slot);
        let (layout, bytes, sorted) = if chosen {
            (&SELECTED_LAYOUT, SELECTED_BYTES, every)
        } else {
            (&COMMAND_POWER_LAYOUT, 24, power)
        };
        // Every header the country needs, in one guarded call; a field that
        // did not copy fails at its turn with its own error.
        let mut header = [0_u8; SELECTED_BYTES];
        let fields = memory.copy_records(
            RecordSources::Pointers(&[country.pointer]),
            layout,
            &mut header[..bytes],
        );
        let accumulator = (fields >= 1)
            .then(|| Array::from_header(header[..16].try_into().unwrap(), STRIDE))
            .flatten()
            .ok_or("modifier accumulator header unreadable or invalid")?;
        let mut values = [modifier_accumulator::Value::default(); MODIFIERS];
        if accumulator.count as usize <= ACCUMULATOR_ENTRIES {
            // The whole accumulator in one guarded call, searched locally.
            let bytes = &mut entries[..accumulator.count as usize * STRIDE];
            if !bytes.is_empty() && !memory.copy(accumulator.data, bytes) {
                return Err("modifier accumulator entry unreadable");
            }
            let bytes = &*bytes;
            search(
                accumulator.count,
                |index| Ok(bytes[index as usize * STRIDE..][..16].try_into().unwrap()),
                sorted,
                &mut values,
            )?;
        } else {
            search(
                accumulator.count,
                |index| {
                    accumulator
                        .element(index, STRIDE)
                        .and_then(|at| read::<[u8; 16]>(memory, at))
                        .ok_or("modifier accumulator entry unreadable")
                },
                sorted,
                &mut values,
            )?;
        }
        let mut modifiers = [None; MODIFIERS];
        for &(key, column) in sorted {
            let value = values[column];
            modifiers[column] = Some(if value.present {
                effective_memo(memory, base, key, value.raw, memo)?
            } else {
                0
            });
        }
        // Command power and the ace country modifiers, every country.
        if fields < 2 {
            return Err("allocated command power unreadable");
        }
        rows.command_power.push(
            &mut buffers.command_power,
            CommandPowerRow {
                canonical_slot: slot,
                allocated_raw: i64::from_le_bytes(header[16..24].try_into().unwrap()),
                maximum: modifiers[MAX_COMMAND_POWER],
                maximum_factor: modifiers[MAX_COMMAND_POWER_MULT],
                ace_effectiveness: modifiers[ACE_EFFECTIVENESS],
                ace_bonuses: modifiers[AIR_ACE_BONUSES],
            },
        )?;
        if !chosen {
            continue;
        }
        let flag = if slot < humans.count {
            Some(u32::from(
                autonomous
                    .element(slot, 1)
                    .and_then(|address| read::<u8>(memory, address))
                    .ok_or("autonomous flag unreadable")?,
            ))
        } else {
            None
        };
        // Convoy reservations.
        let held = (fields >= 3)
            .then(|| Array::from_header(header[24..40].try_into().unwrap(), 8))
            .flatten()
            .ok_or("convoy reservations invalid")?;
        let start = rows.reservations_len;
        let words = held.count as usize * 2;
        *required_reservations += words;
        buffers.reservations.ensure(*required_reservations)?;
        let fits = buffers.reservations.len() - start >= words;
        let mut first = 0;
        while first < held.count {
            let count = pointers(
                memory,
                held,
                first,
                &mut list,
                "convoy reservation unreadable",
            )?;
            if list[..count].contains(&0) {
                return Err("convoy reservation unreadable or null");
            }
            let mut pairs = [0_u8; BATCH * RESERVATION];
            if memory.copy_records(
                RecordSources::Pointers(&list[..count]),
                &RESERVATION_LAYOUT,
                &mut pairs[..count * RESERVATION],
            ) != count
            {
                return Err("convoy reservation unreadable or null");
            }
            if fits {
                for (index, pair) in pairs[..count * RESERVATION]
                    .chunks_exact(RESERVATION)
                    .enumerate()
                {
                    let at = start + (first as usize + index) * 2;
                    buffers.reservations[at] = u32::from_le_bytes(pair[..4].try_into().unwrap());
                    buffers.reservations[at + 1] =
                        u32::from_le_bytes(pair[4..].try_into().unwrap());
                }
            }
            first += count as u32;
        }
        if fits {
            rows.reservations_len += words;
            rows.countries.push(
                &mut buffers.countries,
                CountryRow {
                    canonical_slot: slot,
                    modifiers,
                    autonomous: flag,
                    reservations_start: start,
                    reservations_len: words,
                },
            )?;
        } else {
            rows.countries.unread = rows
                .countries
                .unread
                .checked_add(1)
                .ok_or("raid input count overflow")?;
        }
        // Raid source sites: the object and both list headers in one call.
        let strategic = if slot < air.count {
            element(memory, air, slot, "strategic air country unreadable")?
        } else {
            0
        };
        if strategic != 0 {
            let mut lists = [0_u8; STRATEGIC_BYTES];
            let listed = memory.copy_records(
                RecordSources::Pointers(&[strategic]),
                &STRATEGIC_LAYOUT,
                &mut lists,
            );
            if listed == 0 || u64::from_le_bytes(lists[..8].try_into().unwrap()) != strategic_vtable
            {
                return Err("strategic air country vtable mismatch");
            }
            let mut ordinal = 0;
            for list_index in 0..2_u8 {
                let at = 8 + 16 * list_index as usize;
                let sites = (listed > 1 + list_index as usize)
                    .then(|| Array::from_header(lists[at..at + 16].try_into().unwrap(), 8))
                    .flatten()
                    .ok_or("air site list invalid")?;
                let mut first = 0;
                while first < sites.count {
                    let count = pointers(memory, sites, first, &mut list, "air site unreadable")?;
                    sites_batch(
                        memory,
                        &list[..count],
                        slot,
                        list_index,
                        [site_vtable, container_vtable],
                        &mut ordinal,
                        &mut rows.sites,
                        &mut buffers.sites,
                    )?;
                    first += count as u32;
                }
            }
        }
        // Aces.
        let roster = (fields >= 4)
            .then(|| Array::from_header(header[40..56].try_into().unwrap(), 8))
            .flatten()
            .ok_or("ace roster invalid")?;
        let mut first = 0;
        while first < roster.count {
            let count = pointers(memory, roster, first, &mut list, "ace unreadable")?;
            if list[..count].contains(&0) {
                return Err("ace unreadable");
            }
            let mut records = [0_u8; BATCH * ACE_BYTES];
            let copied = memory.copy_records(
                RecordSources::Pointers(&list[..count]),
                &ACE_LAYOUT,
                &mut records[..count * ACE_BYTES],
            );
            for (index, record) in records[..count * ACE_BYTES]
                .chunks_exact(ACE_BYTES)
                .enumerate()
            {
                if copied < index * ACE_FIELDS.len() + 1
                    || u64::from_le_bytes(record[..8].try_into().unwrap()) != ace_vtable
                {
                    return Err("ace vtable mismatch");
                }
                if copied < (index + 1) * ACE_FIELDS.len() {
                    return Err("ace fields unreadable");
                }
                let ace = list[index];
                let definition =
                    usize::try_from(u64::from_le_bytes(record[25..33].try_into().unwrap()))
                        .map_err(|_| "ace type unreadable")?;
                let type_range = if definition == 0 {
                    0
                } else {
                    type_ranges.get_or(definition, || {
                        effective(
                            memory,
                            base,
                            definition
                                .checked_add(ACE_TYPE_MODIFIERS)
                                .ok_or("ace type modifier address overflow")?,
                            air_range.ok_or("air_range_factor key not given")?,
                            memo,
                        )
                    })?
                };
                rows.aces.push(
                    &mut buffers.aces,
                    AceRow {
                        ace: u64::from_le_bytes(record[8..16].try_into().unwrap()),
                        canonical_slot: slot,
                        wing: u64::from_le_bytes(record[16..24].try_into().unwrap()),
                        alive: record[24],
                        range: effective(
                            memory,
                            base,
                            ace.checked_add(cace::MODIFIERS)
                                .ok_or("ace modifier address overflow")?,
                            air_range.ok_or("air_range_factor key not given")?,
                            memo,
                        )?,
                        type_range,
                    },
                )?;
            }
            first += count as u32;
        }
    }
    Ok(())
}

/// Up to [`BATCH`] sites of one list in three guarded calls: every site's
/// identity and container table header, the country's container pointer in
/// each table that reaches its slot, and every present container's vtable
/// and capacity. The first failing site reports its own error, in the order
/// a per-site read would meet it.
#[allow(clippy::too_many_arguments)]
fn sites_batch(
    memory: &impl Memory,
    list: &[usize],
    slot: u32,
    list_index: u8,
    [site_vtable, container_vtable]: [u64; 2],
    ordinal: &mut u32,
    sites: &mut Bounded,
    output: &mut crate::walk::Buffer<'_, SiteRow>,
) -> Result<(), &'static str> {
    let count = list.len();
    // A null site fails at its turn; only the sites before it are read.
    let readable = list.iter().position(|&site| site == 0).unwrap_or(count);
    let mut identities = [0_u8; BATCH * SITE_BYTES];
    let completed = memory.copy_records(
        RecordSources::Pointers(&list[..readable]),
        &SITE_LAYOUT,
        &mut identities[..readable * SITE_BYTES],
    );
    // Each site's handle and its container address, up to the first site
    // whose identity or table fails.
    let mut handles = [0_u64; BATCH];
    let mut addresses = [0_usize; BATCH];
    let mut owners = [usize::MAX; BATCH];
    let mut wanted = 0;
    let mut failure = None;
    for index in 0..count {
        let record = &identities[index * SITE_BYTES..][..SITE_BYTES];
        let fields = completed.saturating_sub(index * SITE_FIELDS.len()).min(2);
        if index >= readable
            || fields == 0
            || u64::from_le_bytes(record[..8].try_into().unwrap()) != site_vtable
        {
            failure = Some((index, "air site vtable mismatch"));
            break;
        }
        let Some(table) = (fields == 2)
            .then(|| Array::from_header(record[16..32].try_into().unwrap(), 8))
            .flatten()
        else {
            failure = Some((index, "air site containers invalid"));
            break;
        };
        handles[index] = u64::from_le_bytes(record[8..16].try_into().unwrap());
        if slot < table.count {
            let Some(address) = table.element(slot, 8) else {
                failure = Some((index, "air site container unreadable"));
                break;
            };
            addresses[wanted] = address;
            owners[wanted] = index;
            wanted += 1;
        }
    }
    let mut words = [0_u8; BATCH * 8];
    let found = memory.copy_records(
        RecordSources::Pointers(&addresses[..wanted]),
        &POINTER_LAYOUT,
        &mut words[..wanted * 8],
    );
    // A container read that stopped before the identity failure comes first.
    if found < wanted {
        return Err("air site container unreadable");
    }
    if let Some((_, reason)) = failure {
        return Err(reason);
    }
    let mut containers = [0_usize; BATCH];
    for (element, &owner) in owners[..wanted].iter().enumerate() {
        containers[owner] = usize::try_from(u64::from_le_bytes(
            words[element * 8..element * 8 + 8].try_into().unwrap(),
        ))
        .map_err(|_| "air site container unreadable")?;
    }
    // Every present container's vtable and capacity, in one guard.
    let mut present = [0_usize; BATCH];
    let mut present_len = 0;
    for &container in &containers[..count] {
        if container != 0 {
            present[present_len] = container;
            present_len += 1;
        }
    }
    let mut records = [0_u8; BATCH * 16];
    if memory.copy_records(
        RecordSources::Pointers(&present[..present_len]),
        &CONTAINER_LAYOUT,
        &mut records[..present_len * 16],
    ) != present_len * 2
    {
        return Err("air site container capacity unreadable");
    }
    let mut next = 0;
    for index in 0..count {
        let capacity = if containers[index] == 0 {
            None
        } else {
            let record = &records[next * 16..next * 16 + 16];
            next += 1;
            if u64::from_le_bytes(record[..8].try_into().unwrap()) != container_vtable {
                return Err("air site container vtable mismatch");
            }
            Some(i64::from_le_bytes(record[8..].try_into().unwrap()))
        };
        sites.push(
            output,
            SiteRow {
                canonical_slot: slot,
                ordinal: *ordinal,
                list: list_index,
                base: handles[index],
                container_capacity_raw: capacity,
            },
        )?;
        *ordinal += 1;
    }
    Ok(())
}

/// One pass over the canonical countries. CountryCommandPower covers every
/// country; RaidCountryInputs, StockpileArchetypes, RaidAirSites and
/// AceRangeInputs cover only the canonical slots `selected` accepts (their
/// daily slice and new or unpublished owners). StockpileArchetypes fails
/// apart from the rest; CountryCommandPower is read in the same country pass
/// as the sliced tables and fails with them.
/// The reservation arena demand of every country: two words per convoy
/// reservation (`CConvoys` reservations, [`CountryRow::reservations_len`]).
/// Header words only, for presizing the arena before a full hour.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct StorageDemand {
    pub countries: usize,
    pub archetypes: usize,
    pub sites: usize,
    pub aces: usize,
    pub reservations: usize,
}

/// Original allocation, site and ace headers size the complete first slice without a country multiplier.
pub fn storage_demand(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<StorageDemand, &'static str> {
    let countries = ccountry::Reader::new(memory, base, state)?;
    let manager = pointer(
        memory,
        state,
        cgame_state::STRATEGIC_AIR_MANAGER,
        "strategic air manager unreadable",
    )?;
    vtable(
        memory,
        base,
        manager,
        cstrategic_air_manager::VTABLE_RVA,
        "strategic air manager vtable mismatch",
    )?;
    let air = array(
        memory,
        manager,
        cstrategic_air_manager::COUNTRY_AIR,
        8,
        "strategic air country table invalid",
    )?;
    let mut demand = StorageDemand::default();
    let add = |target: &mut usize, count: usize| -> Result<(), &'static str> {
        *target = target
            .checked_add(count)
            .ok_or("raid input sizing demand overflow")?;
        Ok(())
    };
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        add(&mut demand.countries, 1)?;
        let held = array(
            memory,
            country.pointer,
            ccountry::CONVOY_STATE + cconvoys::RESERVATIONS,
            RESERVATION,
            "reservation header invalid",
        )?;
        add(&mut demand.reservations, held.count as usize * 2)?;
        let aces = array(
            memory,
            country.pointer,
            ccountry::ACES,
            8,
            "ace roster invalid",
        )?;
        add(&mut demand.aces, aces.count as usize)?;
        let production = cproduction_status::manager(memory, base, country)?;
        let pool = production
            .checked_add(cproduction_status::STOCKPILE_POOL)
            .ok_or("raid input sizing pool address overflow")?;
        add(
            &mut demand.archetypes,
            cequipment_variant_pool::allocation_entries(memory, base, pool)?.count as usize,
        )?;
        let strategic = if slot < air.count {
            element(memory, air, slot, "strategic air country unreadable")?
        } else {
            0
        };
        if strategic == 0 {
            continue;
        }
        vtable(
            memory,
            base,
            strategic,
            cstrategic_air::VTABLE_RVA,
            "strategic air country vtable mismatch",
        )?;
        for offset in [
            cstrategic_air::FIRST_AIR_SITES,
            cstrategic_air::SECOND_AIR_SITES,
        ] {
            let sites = array(memory, strategic, offset, 8, "air site list invalid")?;
            add(&mut demand.sites, sites.count as usize)?;
        }
    }
    Ok(demand)
}

pub fn required_reservations(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<usize, &'static str> {
    let reader = ccountry::Reader::new(memory, base, game_state)?;
    let mut words = 0_usize;
    for slot in 1..reader.count() {
        let Some(country) = reader.at(slot)? else {
            continue;
        };
        let held = country
            .pointer
            .checked_add(ccountry::CONVOY_STATE + cconvoys::RESERVATIONS)
            .and_then(|at| read::<[u8; 16]>(memory, at))
            .and_then(|header| Array::from_header(header, 8))
            .ok_or("convoy reservations invalid")?;
        words = words
            .checked_add(held.count as usize * 2)
            .ok_or("convoy reservation demand overflow")?;
    }
    Ok(words)
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut buffers: Buffers<'_>,
    selected: &dyn Fn(u32) -> bool,
) -> Walk {
    buffers.memo.clear();
    let mut archetypes = Bounded::default();
    let mut rows = Sliced::default();
    let mut required_reservations = 0;
    let (stockpile, result) = match ccountry::Reader::new(memory, base, game_state) {
        Ok(reader) => (
            stockpiles(
                memory,
                base,
                &reader,
                selected,
                &mut archetypes,
                &mut buffers.archetypes,
            ),
            sliced(
                memory,
                base,
                game_state,
                &reader,
                &mut buffers,
                selected,
                &mut rows,
                &mut required_reservations,
            ),
        ),
        Err(reason) => (Err(reason), Err(reason)),
    };
    let archetypes = match stockpile {
        Ok(()) => archetypes.outcome(),
        Err(reason) => Outcome::Unavailable { reason },
    };
    match result {
        Ok(()) => Walk {
            countries: rows.countries.outcome(),
            command_power: rows.command_power.outcome(),
            archetypes,
            sites: rows.sites.outcome(),
            aces: rows.aces.outcome(),
            reservations_len: rows.reservations_len,
            required_reservations,
        },
        Err(reason) => {
            let failed = Outcome::Unavailable { reason };
            Walk {
                countries: failed,
                command_power: failed,
                archetypes,
                sites: failed,
                aces: failed,
                reservations_len: 0,
                required_reservations,
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{allocation_equipment_row, modifier_definition};
    use std::collections::BTreeMap;

    // The fixtures register the launch keys at the static indices.
    const AIR_RANGE: u32 = 0x05;
    const MAX_COMMAND_POWER: u32 = 0x14b;
    const MAX_COMMAND_POWER_FACTOR: u32 = 0x14d;
    const MODIFIERS: [u32; super::MODIFIERS] = [
        AIR_RANGE,
        0x21,
        0x72,
        MAX_COMMAND_POWER,
        MAX_COMMAND_POWER_FACTOR,
        0x25a,
    ];
    pub(crate) const KEYS: Keys = Keys::FIXED;

    const GS: usize = 0x1000;
    const COUNTRIES: usize = 0x2000;
    const LINKS: usize = 0x2100;
    const HUMANS: usize = 0x2200;
    const FLAGS: usize = 0x2300;
    const MANAGER: usize = 0x2400;
    const AIR: usize = 0x2500;
    const COUNTRY: usize = 0x10_000;
    const PRODUCTION: usize = 0x20_000;
    const ALLOCATIONS: usize = 0x21_000;
    const ARCHETYPE: usize = 0x22_000;
    const RESERVATIONS: usize = 0x23_000;
    const RESERVATION: usize = 0x23_100;
    const STRATEGIC: usize = 0x24_000;
    const SITES: usize = 0x24_800;
    const SITE: usize = 0x25_000;
    const CONTAINERS: usize = 0x25_800;
    const CONTAINER: usize = 0x26_000;
    const ROSTER: usize = 0x27_000;
    const ACE: usize = 0x28_000;
    const ACE_TYPE: usize = 0x29_000;
    const DEFINITIONS: usize = 0x100_000;

    #[derive(Default)]
    struct Synthetic(BTreeMap<usize, u8>);

    impl Memory for Synthetic {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            for (index, byte) in destination.iter_mut().enumerate() {
                let Some(value) = self.0.get(&(address + index)) else {
                    return false;
                };
                *byte = *value;
            }
            true
        }
    }

    impl Put for Synthetic {
        fn bytes(&mut self, address: usize, bytes: &[u8]) {
            self.0
                .extend(bytes.iter().enumerate().map(|(i, b)| (address + i, *b)));
        }
    }

    /// Fixture writers over any test image.
    trait Put {
        fn bytes(&mut self, address: usize, bytes: &[u8]);
        fn u32(&mut self, address: usize, value: u32) {
            self.bytes(address, &value.to_le_bytes());
        }
        fn u64(&mut self, address: usize, value: u64) {
            self.bytes(address, &value.to_le_bytes());
        }
        fn array(&mut self, address: usize, data: usize, count: u32) {
            self.u64(address, data as u64);
            self.u32(address + 8, count);
            self.u32(address + 12, count);
        }
        /// A sorted accumulator of `(key, raw)` entries.
        fn accumulator(&mut self, address: usize, data: usize, entries: &[(u32, i64)]) {
            self.array(address, data, entries.len() as u32);
            for (index, &(key, raw)) in entries.iter().enumerate() {
                self.u64(data + index * 16, u64::from(key));
                self.u64(data + index * 16 + 8, raw as u64);
            }
        }
    }

    fn fixture() -> Synthetic {
        let mut memory = Synthetic::default();
        memory.u64(modifier_definition::TABLE_RVA, DEFINITIONS as u64);
        for key in MODIFIERS {
            let row = DEFINITIONS + key as usize * modifier_definition::STRIDE;
            memory.u32(row + modifier_definition::FEATURE_SELECTOR, 0);
            // The capacity factor clamps to [0, 1].
            let flags = if key == MAX_COMMAND_POWER_FACTOR {
                modifier_definition::CLAMP_MAXIMUM
            } else {
                0
            };
            memory.u32(row + modifier_definition::FLAGS, flags);
        }
        memory.array(GS + cgame_state::COUNTRIES, COUNTRIES, 2);
        memory.array(GS + cgame_state::COUNTRY_LINKS, LINKS, 3);
        memory.u64(COUNTRIES + 8, COUNTRY as u64);
        memory.u32(LINKS + 4, 1);
        memory.u32(LINKS + 8, 1);
        memory.u64(COUNTRY, ccountry::VTABLE_RVA as u64);
        memory.u32(COUNTRY + ccountry::ID, 2);
        memory.u64(COUNTRY + ccountry::ALLOCATED_COMMAND_POWER, 3_000_000);
        memory.accumulator(
            COUNTRY + ccountry::MODIFIER_ACCUMULATOR,
            0x11_000,
            &[
                (AIR_RANGE, 20_000),
                (0x72, 7_000),
                (MAX_COMMAND_POWER, 1_000_000),
                (MAX_COMMAND_POWER_FACTOR, 250_000),
                (0x25a, 30_000),
            ],
        );
        memory.array(GS + cgame_state::HUMAN_STATUS, HUMANS, 2);
        memory.array(GS + cgame_state::AUTONOMOUS_STATUS, FLAGS, 2);
        memory.bytes(FLAGS + 1, &[1]);
        memory.array(
            COUNTRY + ccountry::CONVOY_STATE + cconvoys::RESERVATIONS,
            RESERVATIONS,
            1,
        );
        memory.u64(RESERVATIONS, RESERVATION as u64);
        memory.u32(RESERVATION, 1);
        memory.u32(RESERVATION + 4, 12);
        memory.u64(COUNTRY + ccountry::PRODUCTION_STATUS, PRODUCTION as u64);
        memory.u64(PRODUCTION, cproduction_status::VTABLE_RVA as u64);
        memory.u64(PRODUCTION + cproduction_status::COUNTRY, COUNTRY as u64);
        let pool = PRODUCTION + cproduction_status::STOCKPILE_POOL;
        memory.u64(pool, cequipment_variant_pool::VTABLE_RVA as u64);
        memory.array(pool + cequipment_variant_pool::ALLOCATIONS, ALLOCATIONS, 1);
        memory.u64(
            ALLOCATIONS + allocation_equipment_row::ARCHETYPE,
            ARCHETYPE as u64,
        );
        memory.u64(ALLOCATIONS + 8, 0);
        memory.u64(ALLOCATIONS + allocation_equipment_row::AMOUNT, 4_000_000);
        memory.u64(ARCHETYPE, cequipment_type::VTABLE_RVA as u64);
        memory.u32(ARCHETYPE + cequipment_type::TOKEN, 0x3309);
        memory.u32(ARCHETYPE + cequipment_type::ARCHETYPE_ID, 7);
        memory.u64(GS + cgame_state::STRATEGIC_AIR_MANAGER, MANAGER as u64);
        memory.u64(MANAGER, cstrategic_air_manager::VTABLE_RVA as u64);
        memory.array(MANAGER + cstrategic_air_manager::COUNTRY_AIR, AIR, 2);
        memory.u64(AIR + 8, STRATEGIC as u64);
        memory.u64(STRATEGIC, cstrategic_air::VTABLE_RVA as u64);
        memory.array(STRATEGIC + cstrategic_air::FIRST_AIR_SITES, SITES, 1);
        memory.array(STRATEGIC + cstrategic_air::SECOND_AIR_SITES, SITES, 0);
        memory.u64(SITES, SITE as u64);
        memory.u64(SITE, cair_base::VTABLE_RVA as u64);
        memory.u64(SITE + cair_base::HANDLE, 0x0000_0009_0000_0041);
        memory.array(SITE + cair_base::COUNTRY_CONTAINERS, CONTAINERS, 2);
        memory.u64(CONTAINERS, 0);
        memory.u64(CONTAINERS + 8, CONTAINER as u64);
        memory.u64(CONTAINER, ccountry_air_container::VTABLE_RVA as u64);
        memory.u64(CONTAINER + ccountry_air_container::CAPACITY_RAW, 100_000);
        memory.array(COUNTRY + ccountry::ACES, ROSTER, 1);
        memory.u64(ROSTER, ACE as u64);
        memory.u64(ACE, cace::VTABLE_RVA as u64);
        memory.u64(ACE + cace::REFERENCE, 0x0000_0003_0000_0077);
        memory.u64(ACE + cace::ASSIGNED_WING_REFERENCE, 0x0000_0005_0000_0045);
        memory.bytes(ACE + cace::ALIVE, &[1]);
        memory.accumulator(ACE + cace::MODIFIERS, 0x12_000, &[(AIR_RANGE, 10_000)]);
        memory.u64(ACE + ACE_TYPE_DEFINITION, ACE_TYPE as u64);
        // The range operand is the type's embedded accumulator, independently of its header constant.
        memory.accumulator(ACE_TYPE + 0x48, 0x13_000, &[(AIR_RANGE, 5_000)]);
        // The enclosing modifier head is distinct from its accumulator.
        memory.u64(ACE_TYPE + 0x38, 0x14_000);
        memory.u64(ACE_TYPE + 0x40, 1);
        memory
    }

    struct Run {
        walk: Walk,
        country: CountryRow,
        command_power: CommandPowerRow,
        pairs: Vec<u32>,
        archetype: ArchetypeRow,
        site: SiteRow,
        ace: AceRow,
    }

    fn run(memory: &Synthetic, reservations: usize) -> Run {
        run_selected(memory, reservations, &|_| true)
    }

    fn run_selected(
        memory: &Synthetic,
        reservations: usize,
        selected: &dyn Fn(u32) -> bool,
    ) -> Run {
        run_keyed(memory, reservations, selected, &KEYS)
    }

    fn run_keyed(
        memory: &Synthetic,
        reservations: usize,
        selected: &dyn Fn(u32) -> bool,
        keys: &Keys,
    ) -> Run {
        let mut countries = [CountryRow::default(); 2];
        let mut command_power = [CommandPowerRow::default(); 2];
        let mut pairs = vec![0; reservations];
        let mut archetypes = [ArchetypeRow::default(); 2];
        let mut sites = [SiteRow::default(); 2];
        let mut aces = [AceRow::default(); 2];
        let walk = walk(
            memory,
            0,
            GS,
            Buffers {
                memo: &DefinitionMemo::default(),
                keys,
                countries: crate::walk::Buffer::Fixed(&mut countries),
                command_power: crate::walk::Buffer::Fixed(&mut command_power),
                reservations: crate::walk::Buffer::Fixed(&mut pairs),
                archetypes: crate::walk::Buffer::Fixed(&mut archetypes),
                sites: crate::walk::Buffer::Fixed(&mut sites),
                aces: crate::walk::Buffer::Fixed(&mut aces),
            },
            selected,
        );
        Run {
            walk,
            country: countries[0],
            command_power: command_power[0],
            pairs,
            archetype: archetypes[0],
            site: sites[0],
            ace: aces[0],
        }
    }

    /// A launch key the running game does not register by name leaves its
    /// value `None`, never zero; the aces, which read `air_range_factor`,
    /// are unknown without it, with the name.
    #[test]
    fn a_key_left_out_leaves_its_value_unknown() {
        let mut keys = KEYS;
        keys.keys[3] = None;
        let run = run_keyed(&fixture(), 2, &|_| true, &keys);
        assert_eq!(run.walk.countries, Outcome::Rows { len: 1 });
        assert_eq!(run.country.modifiers[3], None);
        assert_eq!(run.country.modifiers[4], Some(100_000));
        let mut keys = KEYS;
        keys.keys[0] = None;
        let run = run_keyed(&fixture(), 2, &|_| true, &keys);
        assert_eq!(
            run.walk.aces,
            Outcome::Unavailable {
                reason: "air_range_factor key not given"
            }
        );
    }

    /// An unselected country evaluates only the hourly keys, the command
    /// power pair and the ace country modifiers: a daily launch key whose
    /// definition is unreadable fails only a selected read.
    #[test]
    fn an_unselected_country_evaluates_only_the_hourly_keys() {
        let mut memory = fixture();
        let row = DEFINITIONS + AIR_RANGE as usize * modifier_definition::STRIDE;
        for index in 0..4 {
            memory
                .0
                .remove(&(row + modifier_definition::FEATURE_SELECTOR + index));
        }
        let run = run_selected(&memory, 2, &|_| false);
        assert_eq!(run.walk.command_power, Outcome::Rows { len: 1 });
        assert_eq!(
            (
                run.command_power.maximum,
                run.command_power.maximum_factor,
                run.command_power.ace_effectiveness,
                run.command_power.ace_bonuses
            ),
            (Some(1_000_000), Some(100_000), Some(30_000), Some(7_000))
        );
        assert_eq!(run.walk.countries, Outcome::Rows { len: 0 });
        let run = run_selected(&memory, 2, &|_| true);
        assert_eq!(
            run.walk.command_power,
            Outcome::Unavailable {
                reason: "modifier selector unreadable"
            }
        );
    }

    /// Two words per reservation of every country, from the headers alone:
    /// the arena size a full hour needs to read every country.
    #[test]
    fn the_reservation_demand_sizes_a_full_hour() {
        let memory = fixture();
        let words = required_reservations(&memory, 0, GS).unwrap();
        assert_eq!(words, 2);
        assert_eq!(run(&memory, words).walk.countries, Outcome::Rows { len: 1 });
        assert!(matches!(
            run(&memory, words - 1).walk.countries,
            Outcome::Truncated { .. }
        ));
    }

    #[test]
    fn one_pass_keeps_stored_values_and_getter_results() {
        let run = run(&fixture(), 2);
        assert_eq!(run.walk.countries, Outcome::Rows { len: 1 });
        assert_eq!(run.walk.command_power, Outcome::Rows { len: 1 });
        assert_eq!(
            run.command_power,
            CommandPowerRow {
                canonical_slot: 1,
                allocated_raw: 3_000_000,
                maximum: Some(1_000_000),
                maximum_factor: Some(100_000),
                ace_effectiveness: Some(30_000),
                ace_bonuses: Some(7_000),
            }
        );
        // Present keys pass the getter; the factor clamps; absent keys are
        // zero. One evaluation serves both rows; the conversion publishes the
        // hourly columns only in CountryCommandPower.
        assert_eq!(
            run.country.modifiers,
            [
                Some(20_000),
                Some(0),
                Some(7_000),
                Some(1_000_000),
                Some(100_000),
                Some(30_000)
            ]
        );
        assert_eq!(run.country.autonomous, Some(1));
        assert_eq!(run.pairs, [1, 12]);
        assert_eq!(
            (run.archetype.archetype, run.archetype.quantity_raw),
            (0x3309, 4_000_000)
        );
        assert_eq!(
            (
                run.site.list,
                run.site.base,
                run.site.container_capacity_raw
            ),
            (0, 0x0000_0009_0000_0041, Some(100_000))
        );
        assert_eq!(
            (
                run.ace.wing,
                run.ace.alive,
                run.ace.range,
                run.ace.type_range
            ),
            (0x0000_0005_0000_0045, 1, 10_000, 5_000)
        );
    }

    #[test]
    fn a_short_reservation_arena_withholds_the_country_and_reports_demand() {
        let run = run(&fixture(), 1);
        assert_eq!(
            run.walk.countries,
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
        assert_eq!(run.walk.required_reservations, 2);
    }

    #[test]
    fn a_site_without_a_container_for_the_country_has_no_capacity() {
        let mut memory = fixture();
        memory.u64(CONTAINERS + 8, 0);
        let run = run(&memory, 2);
        assert_eq!(run.walk.sites, Outcome::Rows { len: 1 });
        assert_eq!(run.site.container_capacity_raw, None);
    }

    #[test]
    fn a_foreign_site_vtable_fails_the_sliced_tables_and_keeps_the_stockpile() {
        let mut memory = fixture();
        memory.u64(SITE, 7);
        let run = run(&memory, 2);
        let failed = Outcome::Unavailable {
            reason: "air site vtable mismatch",
        };
        assert_eq!(
            (run.walk.countries, run.walk.sites, run.walk.aces),
            (failed, failed, failed)
        );
        assert_eq!(run.walk.archetypes, Outcome::Rows { len: 1 });
        assert_eq!(run.archetype.quantity_raw, 4_000_000);
    }

    #[test]
    fn a_stockpile_failure_keeps_the_sliced_tables() {
        let mut memory = fixture();
        memory.u64(PRODUCTION + cproduction_status::COUNTRY, 0);
        let run = run(&memory, 2);
        assert_eq!(
            run.walk.archetypes,
            Outcome::Unavailable {
                reason: "production status country mismatch"
            }
        );
        assert_eq!(run.walk.countries, Outcome::Rows { len: 1 });
        assert_eq!(run.walk.sites, Outcome::Rows { len: 1 });
    }

    /// The fixture with readable controller arrays, a player count and a
    /// second site in list 0.
    fn site_fixture() -> Synthetic {
        let mut memory = fixture();
        memory.bytes(HUMANS, &[0, 0]);
        memory.bytes(FLAGS, &[0]);
        memory.u32(GS + cgame_state::PLAYER_RECORDS + 0xc, 1);
        memory.array(STRATEGIC + cstrategic_air::FIRST_AIR_SITES, SITES, 2);
        memory.u64(SITES + 8, SITE_B as u64);
        memory.u64(SITE_B, cair_base::VTABLE_RVA as u64);
        memory.u64(SITE_B + cair_base::HANDLE, 0x0000_000a_0000_0041);
        // Slot 1 has no entry in the second site's container table.
        memory.array(SITE_B + cair_base::COUNTRY_CONTAINERS, CONTAINERS, 1);
        memory
    }

    #[test]
    fn each_site_reports_its_own_failure_in_site_order() {
        // The second site's container table is invalid; the first site's
        // container pointer is unreadable, which a per-site read meets first.
        let mut memory = site_fixture();
        memory.array(SITE_B + cair_base::COUNTRY_CONTAINERS, CONTAINERS, 2);
        memory.u32(SITE_B + cair_base::COUNTRY_CONTAINERS + 12, 9);
        memory.u64(SITE_B, cair_base::VTABLE_RVA as u64);
        memory.u64(SITE_B + cair_base::HANDLE, 0x42);
        let unavailable = |run: &Run| match run.walk.sites {
            Outcome::Unavailable { reason } => reason,
            other => panic!("{other:?}"),
        };
        assert_eq!(unavailable(&run(&memory, 2)), "air site containers invalid");
        memory
            .0
            .retain(|&address, _| !(CONTAINERS + 8..CONTAINERS + 16).contains(&address));
        assert_eq!(
            unavailable(&run(&memory, 2)),
            "air site container unreadable"
        );
    }

    #[test]
    fn an_unselected_country_reads_only_its_command_power() {
        // Its convoys, air sites, aces and stockpile are never read.
        let mut memory = fixture();
        let unread = [
            COUNTRY + ccountry::CONVOY_STATE + cconvoys::RESERVATIONS,
            STRATEGIC,
            COUNTRY + ccountry::ACES,
            COUNTRY + ccountry::PRODUCTION_STATUS,
        ];
        memory
            .0
            .retain(|address, _| !unread.iter().any(|at| (*at..at + 16).contains(address)));
        let run = run_selected(&memory, 2, &|slot| slot != 1);
        assert_eq!(run.walk.countries, Outcome::Rows { len: 0 });
        assert_eq!(run.walk.sites, Outcome::Rows { len: 0 });
        assert_eq!(run.walk.aces, Outcome::Rows { len: 0 });
        assert_eq!(run.walk.archetypes, Outcome::Rows { len: 0 });
        assert_eq!(run.walk.command_power, Outcome::Rows { len: 1 });
        assert_eq!(run.command_power.allocated_raw, 3_000_000);
        // Selected, the same memory fails the sliced tables.
        let run = run_selected(&memory, 2, &|slot| slot == 1);
        assert!(matches!(run.walk.countries, Outcome::Unavailable { .. }));
        assert!(matches!(run.walk.archetypes, Outcome::Unavailable { .. }));
    }

    const SITE_B: usize = 0x2a_000;

    /// Guarded calls and bytes a walk asks of its memory: one call per
    /// native guarded frame (a copy, a scattered batch or a record batch),
    /// forwarded to the inner memory's own batch methods.
    pub(crate) struct Counted<M> {
        pub(crate) inner: M,
        pub(crate) guards: std::cell::Cell<u64>,
        pub(crate) bytes: std::cell::Cell<u64>,
    }

    impl<M: Memory> Counted<M> {
        pub(crate) fn new(inner: M) -> Self {
            Self {
                inner,
                guards: std::cell::Cell::new(0),
                bytes: std::cell::Cell::new(0),
            }
        }
        fn count(&self, bytes: usize) {
            self.guards.set(self.guards.get() + 1);
            self.bytes.set(self.bytes.get() + bytes as u64);
        }
        pub(crate) fn reset(&self) {
            self.guards.set(0);
            self.bytes.set(0);
        }
    }

    impl<M: Memory> Memory for Counted<M> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.count(destination.len());
            self.inner.copy(address, destination)
        }
        fn copy_scattered<const N: usize>(
            &self,
            addresses: [usize; N],
            output: &mut [&mut [u8]; N],
        ) -> usize {
            self.count(output.iter().map(|bytes| bytes.len()).sum());
            self.inner.copy_scattered(addresses, output)
        }
        fn copy_scattered_prefetched<const N: usize>(
            &self,
            addresses: [usize; N],
            output: &mut [&mut [u8]; N],
        ) -> usize {
            self.count(output.iter().map(|bytes| bytes.len()).sum());
            self.inner.copy_scattered_prefetched(addresses, output)
        }
        fn copy_records(
            &self,
            sources: crate::memory::RecordSources<'_>,
            layout: &crate::memory::RecordLayout<'_>,
            output: &mut [u8],
        ) -> usize {
            let completed = self.inner.copy_records(sources, layout, output);
            self.count(layout.completed_bytes(completed));
            completed
        }
    }

    /// A flat, growing byte image at address zero.
    #[derive(Default)]
    pub(crate) struct Flat(pub(crate) Vec<u8>);

    impl Memory for Flat {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            match address
                .checked_add(destination.len())
                .and_then(|end| self.0.get(address..end))
            {
                Some(source) => {
                    destination.copy_from_slice(source);
                    true
                }
                None => false,
            }
        }
    }

    impl Put for Flat {
        fn bytes(&mut self, address: usize, bytes: &[u8]) {
            let end = address + bytes.len();
            if self.0.len() < end {
                self.0.resize(end, 0);
            }
            self.0[address..end].copy_from_slice(bytes);
        }
    }

    /// A [`Flat`] image read through the guarded native copies: every image
    /// address, including those stored in it, is rebased onto the image's
    /// storage at each read, so a bench pays what production pays per
    /// guarded call and per record. An address past the image reads as a
    /// non-canonical one, which faults gracefully; null stays null.
    #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
    pub(crate) struct Rebased {
        image: Flat,
        live: crate::memory::LiveMemory,
    }

    #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
    impl Rebased {
        pub(crate) fn new(image: Flat) -> Self {
            // SAFETY: every rebased read stays in `image`, which this owns
            // and never mutates while it is read.
            let live = unsafe { crate::memory::LiveMemory::new() };
            Self { image, live }
        }

        fn at(&self, address: usize) -> usize {
            match address {
                0 => 0,
                address if address < self.image.0.len() => self.image.0.as_ptr() as usize + address,
                _ => 0x8000_0000_0000_0000,
            }
        }
    }

    #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
    impl Memory for Rebased {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            let end = address.checked_add(destination.len());
            if end.is_none_or(|end| end > self.image.0.len()) {
                return false;
            }
            self.live.copy(self.at(address), destination)
        }
        fn copy_scattered<const N: usize>(
            &self,
            addresses: [usize; N],
            output: &mut [&mut [u8]; N],
        ) -> usize {
            self.live
                .copy_scattered(addresses.map(|address| self.at(address)), output)
        }
        fn copy_scattered_prefetched<const N: usize>(
            &self,
            addresses: [usize; N],
            output: &mut [&mut [u8]; N],
        ) -> usize {
            self.live
                .copy_scattered_prefetched(addresses.map(|address| self.at(address)), output)
        }
        fn copy_records(
            &self,
            sources: RecordSources<'_>,
            layout: &RecordLayout<'_>,
            output: &mut [u8],
        ) -> usize {
            let pointers = match sources {
                RecordSources::Pointers(pointers) => pointers,
                RecordSources::Strided {
                    base,
                    stride,
                    count,
                } => {
                    return self.live.copy_records(
                        RecordSources::Strided {
                            base: self.at(base),
                            stride,
                            count,
                        },
                        layout,
                        output,
                    );
                }
            };
            let (fields, stride) = (layout.field_count(), layout.stride());
            let mut rebased = [0_usize; 256];
            let mut completed = 0;
            for (chunk, sources) in pointers.chunks(rebased.len()).enumerate() {
                for (to, &from) in rebased.iter_mut().zip(sources) {
                    *to = self.at(from);
                }
                let first = chunk * rebased.len();
                let done = self.live.copy_records(
                    RecordSources::Pointers(&rebased[..sources.len()]),
                    layout,
                    &mut output[first * stride..(first + sources.len()) * stride],
                );
                completed += done;
                if done < sources.len() * fields {
                    break;
                }
            }
            completed
        }
    }

    /// The median wall time of `runs` calls, in milliseconds.
    pub(crate) fn median_ms(runs: usize, mut call: impl FnMut()) -> f64 {
        let mut times: Vec<f64> = (0..runs)
            .map(|_| {
                let started = std::time::Instant::now();
                call();
                started.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        times.sort_by(f64::total_cmp);
        times[runs / 2]
    }

    // Per-country block of the scaled world.
    const WORLD: usize = 0x200_000;
    const BLOCK: usize = 0x5000;
    const B_ACCUMULATOR: usize = 0x1400;
    const B_PRODUCTION: usize = 0x2100;
    const B_ALLOCATIONS: usize = 0x2400;
    const B_RESERVATIONS: usize = 0x2800;
    const B_STRATEGIC: usize = 0x2a00;
    const B_SITES: usize = 0x2c00;
    const B_ROSTER: usize = 0x2d00;
    const B_ACES: usize = 0x2e00;
    const ACE_SIZE: usize = 0x180;
    const B_ACE_MODIFIERS: usize = 0x4000;
    const MODIFIER_ENTRIES: usize = 200;
    const ARCHETYPES: usize = 30;
    const RESERVATIONS_PER_COUNTRY: usize = 4;
    const SITES_PER_COUNTRY: usize = 10;
    const ACES_PER_COUNTRY: usize = 5;

    /// The worst plausible hour for `countries` countries: every country
    /// with 200 accumulator entries (the six keys among them), 4 convoy
    /// reservations, 30 stockpile archetypes, 10 air sites where it has a
    /// container, and 5 aces.
    fn scaled(countries: usize) -> Flat {
        let mut memory = Flat::default();
        let slots = countries + 1;
        memory.u64(modifier_definition::TABLE_RVA, DEFINITIONS as u64);
        for key in MODIFIERS {
            let row = DEFINITIONS + key as usize * modifier_definition::STRIDE;
            memory.u32(row + modifier_definition::FEATURE_SELECTOR, 0);
            memory.u32(row + modifier_definition::FLAGS, 0);
        }
        // Globals past the country blocks: tables, archetypes, sites.
        let globals = WORLD + slots * BLOCK;
        let table = globals;
        let links = table + slots * 8;
        let humans = links + slots * 4;
        let flags = humans + slots;
        let air = (flags + slots + 0xf) & !0xf;
        let archetypes = air + slots * 8;
        let sites = archetypes + ARCHETYPES * 0x500;
        let site_count = countries.max(SITES_PER_COUNTRY) * 2;
        let containers = sites + site_count * 0x100;
        let container = containers + slots * 8;
        let ace_type = container + 0x100;
        memory.array(GS + cgame_state::COUNTRIES, table, slots as u32);
        memory.array(GS + cgame_state::COUNTRY_LINKS, links, slots as u32);
        memory.array(GS + cgame_state::HUMAN_STATUS, humans, slots as u32);
        memory.array(GS + cgame_state::AUTONOMOUS_STATUS, flags, slots as u32);
        memory.u64(GS + cgame_state::STRATEGIC_AIR_MANAGER, MANAGER as u64);
        memory.u64(MANAGER, cstrategic_air_manager::VTABLE_RVA as u64);
        memory.array(
            MANAGER + cstrategic_air_manager::COUNTRY_AIR,
            air,
            slots as u32,
        );
        for index in 0..ARCHETYPES {
            let archetype = archetypes + index * 0x500;
            memory.u64(archetype, cequipment_type::VTABLE_RVA as u64);
            memory.u32(archetype + cequipment_type::TOKEN, 0x3000 + index as u32);
            memory.u32(archetype + cequipment_type::ARCHETYPE_ID, index as u32);
        }
        // Every site shares one container table in which every country has
        // a container.
        for slot in 1..slots {
            memory.u64(containers + slot * 8, container as u64);
        }
        memory.u64(container, ccountry_air_container::VTABLE_RVA as u64);
        memory.u64(container + ccountry_air_container::CAPACITY_RAW, 100_000);
        for index in 0..site_count {
            let site = sites + index * 0x100;
            memory.u64(site, cair_base::VTABLE_RVA as u64);
            memory.u64(site + cair_base::HANDLE, ((index as u64 + 1) << 32) | 0x41);
            memory.array(
                site + cair_base::COUNTRY_CONTAINERS,
                containers,
                slots as u32,
            );
        }
        memory.accumulator(
            ace_type + ACE_TYPE_MODIFIERS,
            ace_type + 0x60,
            &[(AIR_RANGE, 5_000)],
        );
        for slot in 1..slots {
            let country = WORLD + slot * BLOCK;
            memory.u64(table + slot * 8, country as u64);
            memory.u32(links + slot * 4, slot as u32);
            memory.bytes(flags + slot, &[1]);
            memory.u64(country, ccountry::VTABLE_RVA as u64);
            memory.u32(country + ccountry::ID, slot as u32);
            memory.u64(country + ccountry::ALLOCATED_COMMAND_POWER, 3_000_000);
            let mut entries: Vec<(u32, i64)> = (1..=MODIFIER_ENTRIES as u32
                + MODIFIERS.len() as u32)
                .filter(|key| !MODIFIERS.contains(key))
                .take(MODIFIER_ENTRIES - MODIFIERS.len())
                .map(|key| (key, 1_000))
                .chain(MODIFIERS.iter().map(|&key| (key, 50_000)))
                .collect();
            entries.sort_unstable();
            memory.accumulator(
                country + ccountry::MODIFIER_ACCUMULATOR,
                country + B_ACCUMULATOR,
                &entries,
            );
            let reservations = country + B_RESERVATIONS;
            memory.array(
                country + ccountry::CONVOY_STATE + cconvoys::RESERVATIONS,
                reservations,
                RESERVATIONS_PER_COUNTRY as u32,
            );
            for index in 0..RESERVATIONS_PER_COUNTRY {
                let entry = reservations + 0x40 + index * 8;
                memory.u64(reservations + index * 8, entry as u64);
                memory.u32(entry, index as u32);
                memory.u32(entry + 4, 5);
            }
            let production = country + B_PRODUCTION;
            memory.u64(country + ccountry::PRODUCTION_STATUS, production as u64);
            memory.u64(production, cproduction_status::VTABLE_RVA as u64);
            memory.u64(production + cproduction_status::COUNTRY, country as u64);
            let pool = production + cproduction_status::STOCKPILE_POOL;
            memory.u64(pool, cequipment_variant_pool::VTABLE_RVA as u64);
            let allocations = country + B_ALLOCATIONS;
            memory.array(
                pool + cequipment_variant_pool::ALLOCATIONS,
                allocations,
                ARCHETYPES as u32,
            );
            for index in 0..ARCHETYPES {
                let row = allocations + index * allocation_equipment_row::STRIDE;
                memory.u64(row, (archetypes + index * 0x500) as u64);
                memory.u64(row + 8, 0);
                memory.u64(row + allocation_equipment_row::AMOUNT, 4_000_000);
            }
            let strategic = country + B_STRATEGIC;
            memory.u64(air + slot * 8, strategic as u64);
            memory.u64(strategic, cstrategic_air::VTABLE_RVA as u64);
            let list = country + B_SITES;
            memory.array(
                strategic + cstrategic_air::FIRST_AIR_SITES,
                list,
                SITES_PER_COUNTRY as u32 - 2,
            );
            memory.array(strategic + cstrategic_air::SECOND_AIR_SITES, list + 0x40, 2);
            for index in 0..SITES_PER_COUNTRY {
                let site = sites + ((slot * 7 + index) % site_count) * 0x100;
                let at = if index < SITES_PER_COUNTRY - 2 {
                    list + index * 8
                } else {
                    list + 0x40 + (index + 2 - SITES_PER_COUNTRY) * 8
                };
                memory.u64(at, site as u64);
            }
            let roster = country + B_ROSTER;
            memory.array(country + ccountry::ACES, roster, ACES_PER_COUNTRY as u32);
            for index in 0..ACES_PER_COUNTRY {
                let ace = country + B_ACES + index * ACE_SIZE;
                memory.u64(roster + index * 8, ace as u64);
                memory.u64(ace, cace::VTABLE_RVA as u64);
                memory.u64(
                    ace + cace::REFERENCE,
                    ((slot * 8 + index) as u64) << 32 | 0x77,
                );
                memory.u64(ace + cace::ASSIGNED_WING_REFERENCE, 0x0000_0005_0000_0045);
                memory.bytes(ace + cace::ALIVE, &[1]);
                memory.accumulator(
                    ace + cace::MODIFIERS,
                    country + B_ACE_MODIFIERS + index * 0x40,
                    &[(AIR_RANGE, 10_000)],
                );
                memory.u64(ace + ACE_TYPE_DEFINITION, ace_type as u64);
            }
        }
        memory
    }

    /// `cargo test --release -p engine raid_launch_inputs_bench -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn raid_launch_inputs_bench() {
        for countries in [1_500, 5_000] {
            let memory = Counted::new(scaled(countries));
            let mut rows = [CountryRow::default()].repeat(countries);
            let mut command_power = [CommandPowerRow::default()].repeat(countries);
            let mut pairs = vec![0; countries * RESERVATIONS_PER_COUNTRY * 2];
            let mut archetypes = [ArchetypeRow::default()].repeat(countries * ARCHETYPES);
            let mut sites = [SiteRow::default()].repeat(countries * SITES_PER_COUNTRY);
            let mut aces = [AceRow::default()].repeat(countries * ACES_PER_COUNTRY);
            let mut last = None;
            let ms = median_ms(20, || {
                memory.reset();
                last = Some(walk(
                    &memory,
                    0,
                    GS,
                    Buffers {
                        memo: &DefinitionMemo::default(),
                        keys: &KEYS,
                        countries: crate::walk::Buffer::Fixed(&mut rows),
                        command_power: crate::walk::Buffer::Fixed(&mut command_power),
                        reservations: crate::walk::Buffer::Fixed(&mut pairs),
                        archetypes: crate::walk::Buffer::Fixed(&mut archetypes),
                        sites: crate::walk::Buffer::Fixed(&mut sites),
                        aces: crate::walk::Buffer::Fixed(&mut aces),
                    },
                    &|_| true,
                ));
            });
            let walk = last.unwrap();
            assert_eq!(walk.countries, Outcome::Rows { len: countries });
            println!(
                "raid_launch_inputs {countries} countries: {ms:.3} ms median of 20, {} guarded calls, {} bytes; rows: RaidCountryInputs {}, StockpileArchetypes {}, RaidAirSites {}, AceRangeInputs {}",
                memory.guards.get(),
                memory.bytes.get(),
                walk.countries.len(),
                walk.archetypes.len(),
                walk.sites.len(),
                walk.aces.len(),
            );
        }
    }

    /// Evicts the caches by streaming a buffer larger than the last level.
    struct Evictor(Vec<u8>);

    impl Evictor {
        fn new() -> Self {
            Self(vec![1; 512 << 20])
        }
        fn flush(&self) {
            let mut sum = 0_u64;
            for line in self.0.chunks(64) {
                sum = sum.wrapping_add(u64::from(line[0]));
            }
            std::hint::black_box(sum);
        }
    }

    /// The median of 20 runs, each after `prepare`, in milliseconds.
    fn median_after(mut prepare: impl FnMut(), mut call: impl FnMut()) -> f64 {
        let mut times: Vec<f64> = (0..20)
            .map(|_| {
                prepare();
                let started = std::time::Instant::now();
                call();
                started.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        times.sort_by(f64::total_cmp);
        times[10]
    }

    /// The walk's rows, sized for the scaled world.
    struct Rows {
        countries: Vec<CountryRow>,
        command_power: Vec<CommandPowerRow>,
        pairs: Vec<u32>,
        archetypes: Vec<ArchetypeRow>,
        sites: Vec<SiteRow>,
        aces: Vec<AceRow>,
    }

    impl Rows {
        fn new(countries: usize) -> Self {
            Self {
                countries: [CountryRow::default()].repeat(countries),
                command_power: [CommandPowerRow::default()].repeat(countries),
                pairs: vec![0; countries * RESERVATIONS_PER_COUNTRY * 2],
                archetypes: [ArchetypeRow::default()].repeat(countries * ARCHETYPES),
                sites: [SiteRow::default()].repeat(countries * SITES_PER_COUNTRY),
                aces: [AceRow::default()].repeat(countries * ACES_PER_COUNTRY),
            }
        }
        fn walk_selected(&mut self, memory: &impl Memory, selected: &dyn Fn(u32) -> bool) -> Walk {
            walk(
                memory,
                0,
                GS,
                Buffers {
                    memo: &DefinitionMemo::default(),
                    keys: &KEYS,
                    countries: crate::walk::Buffer::Fixed(&mut self.countries),
                    command_power: crate::walk::Buffer::Fixed(&mut self.command_power),
                    reservations: crate::walk::Buffer::Fixed(&mut self.pairs),
                    archetypes: crate::walk::Buffer::Fixed(&mut self.archetypes),
                    sites: crate::walk::Buffer::Fixed(&mut self.sites),
                    aces: crate::walk::Buffer::Fixed(&mut self.aces),
                },
                selected,
            )
        }
    }

    /// The callback's authenticated country census over a bench memory, as
    /// production's country memory supplies it to every reader.
    struct Census<'a, M> {
        inner: &'a M,
        slots: Vec<Option<ccountry::Country>>,
        arrays: [Array; 2],
    }

    impl<'a, M: Memory> Census<'a, M> {
        fn new(inner: &'a M) -> Self {
            let reader = ccountry::Reader::new(inner, 0, GS).unwrap();
            let slots = (0..reader.count())
                .map(|slot| {
                    if slot == 0 {
                        None
                    } else {
                        reader.at(slot).unwrap()
                    }
                })
                .collect();
            Self {
                inner,
                slots,
                arrays: reader.arrays(),
            }
        }
    }

    impl<M: Memory> Memory for Census<'_, M> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.inner.copy(address, destination)
        }
        fn copy_scattered<const N: usize>(
            &self,
            addresses: [usize; N],
            output: &mut [&mut [u8]; N],
        ) -> usize {
            self.inner.copy_scattered(addresses, output)
        }
        fn copy_scattered_prefetched<const N: usize>(
            &self,
            addresses: [usize; N],
            output: &mut [&mut [u8]; N],
        ) -> usize {
            self.inner.copy_scattered_prefetched(addresses, output)
        }
        fn copy_records(
            &self,
            sources: RecordSources<'_>,
            layout: &RecordLayout<'_>,
            output: &mut [u8],
        ) -> usize {
            self.inner.copy_records(sources, layout, output)
        }
        fn authenticated_countries(
            &self,
            _base: usize,
            _game_state: usize,
        ) -> Option<ccountry::AuthenticatedRoster<'_>> {
            Some(ccountry::AuthenticatedRoster::new(&self.slots, self.arrays))
        }
    }

    /// A launch hour as the collector runs it: every country's command power
    /// and one daily slice of the sliced tables (the first hour of a session
    /// reads every country).
    /// `cargo test --release -p engine raid_launch_hour_bench -- --ignored --nocapture`
    #[test]
    #[ignore]
    #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
    fn raid_launch_hour_bench() {
        let evictor = Evictor::new();
        for countries in [1_500, 5_000] {
            let counted = Counted::new(Rebased::new(scaled(countries)));
            let memory = Census::new(&counted);
            let mut rows = Rows::new(countries);
            for (label, chosen) in [
                (
                    "first hour (every country)",
                    &(|_: u32| true) as &dyn Fn(u32) -> bool,
                ),
                ("steady hour (one slice)", &|slot: u32| slot % 24 == 7),
            ] {
                counted.reset();
                let walked = rows.walk_selected(&memory, chosen);
                assert_eq!(walked.command_power, Outcome::Rows { len: countries });
                let (calls, bytes) = (counted.guards.get(), counted.bytes.get());
                let mut hour = || {
                    std::hint::black_box(rows.walk_selected(&memory, chosen));
                };
                let warm = median_after(|| {}, &mut hour);
                let cold = median_after(|| evictor.flush(), &mut hour);
                println!(
                    "launch {label} {countries}: {calls} calls, {bytes} bytes, warm {warm:.3} ms, cold {cold:.3} ms"
                );
            }
        }
    }

    #[test]
    fn growing_raid_buffers_cover_5000_countries_and_later_sites_without_a_slot_ratio() {
        const COUNTRIES: usize = 5000;
        let mut memory = scaled(COUNTRIES);
        let mut countries = Vec::new();
        let mut command_power = Vec::new();
        let mut reservations = Vec::new();
        let mut archetypes = Vec::new();
        let mut sites = Vec::new();
        let mut aces = Vec::new();
        for later in [false, true] {
            if later {
                let strategic = WORLD + BLOCK + B_STRATEGIC;
                let first_site_list = WORLD + BLOCK + B_SITES;
                let selected = u64::from_le_bytes(
                    memory.0[first_site_list..first_site_list + 8]
                        .try_into()
                        .unwrap(),
                );
                let list = memory.0.len();
                for i in 0..20000 {
                    memory.u64(list + i * 8, selected);
                }
                memory.array(strategic + cstrategic_air::FIRST_AIR_SITES, list, 20000);
            }
            let demand = storage_demand(&memory, 0, GS).unwrap();
            assert_eq!(demand.countries, COUNTRIES);
            assert_eq!(demand.archetypes, COUNTRIES * ARCHETYPES);
            assert_eq!(demand.aces, COUNTRIES * ACES_PER_COUNTRY);
            assert_eq!(
                demand.reservations,
                COUNTRIES * RESERVATIONS_PER_COUNTRY * 2
            );
            let captured = walk(
                &memory,
                0,
                GS,
                Buffers {
                    memo: &DefinitionMemo::default(),
                    keys: &KEYS,
                    countries: crate::walk::Buffer::Growing(&mut countries),
                    command_power: crate::walk::Buffer::Growing(&mut command_power),
                    reservations: crate::walk::Buffer::Growing(&mut reservations),
                    archetypes: crate::walk::Buffer::Growing(&mut archetypes),
                    sites: crate::walk::Buffer::Growing(&mut sites),
                    aces: crate::walk::Buffer::Growing(&mut aces),
                },
                &|_| true,
            );
            assert_eq!(
                captured.countries,
                Outcome::Rows {
                    len: demand.countries
                }
            );
            assert_eq!(
                captured.command_power,
                Outcome::Rows {
                    len: demand.countries
                }
            );
            assert_eq!(
                captured.archetypes,
                Outcome::Rows {
                    len: demand.archetypes
                }
            );
            assert_eq!(captured.sites, Outcome::Rows { len: demand.sites });
            assert_eq!(captured.aces, Outcome::Rows { len: demand.aces });
            assert_eq!(captured.reservations_len, demand.reservations);
            assert!(demand.sites > 8192);
            if later {
                assert!(demand.sites > COUNTRIES * SITES_PER_COUNTRY);
            }
        }
    }
}

#[cfg(test)]
mod shared_demand_tests {
    use super::*;
    #[test]
    fn shared_raid_sources_retain_five_thousand_original_dependencies() {
        let mut memo = Shared::<u32>::new();
        for key in 1..=5_001 {
            assert_eq!(memo.get_or(key, || Ok(key as u32)), Ok(key as u32));
        }
        for key in 1..=5_001 {
            assert_eq!(
                memo.get_or(key, || Err("source no longer readable")),
                Ok(key as u32)
            );
        }
    }
}
