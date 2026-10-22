//! Production status reached through the country's production pointer.
//! Factory totals are stored caches, rebuilt on controller, idea and decision
//! changes and during pre-hourly updates. Pending refresh bits select factory
//! recalculation; building-level changes can rebuild modifiers and trigger
//! recalculation when the factory modifier changes. Production updates rebuild
//! dirty totals and otherwise only reassign construction. Top-bar presentation,
//! factory tooltips and triggers read these fields directly.
//! `walk_factories` reads each canonical country's three stored group-head
//! levels with independent authentication and failure handling. The controlled
//! factory count uses those levels; fixed5 production totals are separate.

use crate::{
    Memory, ccountry, cequipment_production_license, cequipment_variant, civ_factory_consumer,
    clicensed_production_status,
    cpdx_array::Array,
    factory_group,
    memory::{RecordField, RecordLayout, RecordSources},
    read, read_slice,
    walk::{Buffer, Outcome},
};

pub mod design_templates;

pub use cequipment_production_license::Row as LicenseRecordRow;

pub use crate::cequipment_production_line::Row as LineRow;

/// Licensed-variant entry in stored order. The first published array writes
/// bare typed variant handles; the third array holds licence relations recorded
/// separately as [`LicenseRecordRow`]. This list is the licensee's buildable variants.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LicenseRow {
    /// Raw country reference of the licensee: the production manager's owner.
    pub country: u32,
    /// Stored container position; the save writes the same order.
    pub ordinal: u32,
    /// Authenticated CEquipmentVariant handle, not a process pointer.
    pub variant: u64,
}

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x0295_9b60;

/// Production-line pointer-array data.
pub const LINES: usize = 0x58;
/// Production-line pointer width.
pub const LINE_POINTER_STRIDE: usize = 0x08;
/// Organisation pointer array with data, capacity and count. New-game setup
/// and loading append entries; serialization keeps order keyed by each token.
pub const ORGANISATIONS: usize = 0x130;
/// Cached allowed-policy roster, rebuilt from content by refresh and loading.
pub const POLICIES: usize = 0x148;
/// Policy primary vtable and definition-token layout.
pub const POLICY_VTABLE_RVA: usize = 0x0292_63a0;
/// Lexer token used to resolve the localized policy name.
pub const POLICY_TOKEN: usize = 8;
/// Construction-line pointer-array data.
pub const CONSTRUCTION_LINES: usize = 0x70;
/// Construction-line pointer-array capacity, u32.
pub const CONSTRUCTION_LINES_CAPACITY: usize = 0x78;
/// Construction-line pointer-array count, u32.
pub const CONSTRUCTION_LINES_COUNT: usize = 0x7c;
/// Construction-line pointer width.
pub const CONSTRUCTION_POINTER_STRIDE: usize = 0x08;
/// Licensed equipment-variant pointer array in the container's available list.
pub const LICENSES: usize = 0x198;
/// Owned-licence object array appended on registration and serialized in order.
pub const LICENSE_RECORDS: usize = 0x1c8;
/// Owned-licence signed count bounding traversal.
#[cfg(test)]
pub const LICENSE_RECORDS_COUNT: usize = 0x1d4;
/// Licence-object pointer stride.
pub const LICENSE_RECORD_STRIDE: usize = clicensed_production_status::POINTER_STRIDE;
/// License pointer width.
pub const LICENSE_POINTER_STRIDE: usize = 0x08;
/// Embedded equipment-variant pool.
pub const STOCKPILE_POOL: usize = 0x200;
/// Stockpile entry-array data.
pub const STOCKPILE: usize = 0x220;
/// CProductionStatus owning CCountry pointer; trade factory-header reader validates this backlink.
pub const COUNTRY: usize = 0x290;
/// Military factory group ([`factory_group`]), populated from military-production
/// buildings and assigned to non-naval lines.
pub const MILITARY_FACTORIES: usize = 0x2b0;
/// Naval dockyard group populated from naval-production buildings for naval lines.
pub const NAVAL_FACTORIES: usize = 0x310;
/// Civilian factory group populated from general-production buildings for
/// civilian consumers and construction.
pub const CIVILIAN_FACTORIES: usize = 0x370;
/// Embedded consumer-goods object ([`civ_factory_consumer`]).
pub const CONSUMER_GOODS: usize = 0x430;
/// National projects consumer.
pub const SPECIAL_PROJECTS: usize = 0x448;
/// Licensed production consumer.
pub const LICENSED_PRODUCTION: usize = 0x460;
/// Trade consumer.
pub const TRADE: usize = 0x478;
/// Market contract payment consumer.
pub const CONTRACT_PAYMENTS: usize = 0x490;
/// Bytes from the military group to the end of the last consumer: one span
/// covers every factory field the reader takes.
pub const FACTORY_SPAN: usize = CONTRACT_PAYMENTS + civ_factory_consumer::SIZE - MILITARY_FACTORIES;

/// Authenticates the manager reached from a previously authenticated country.
pub fn manager(
    memory: &impl Memory,
    base: usize,
    country: ccountry::Country,
) -> Result<usize, &'static str> {
    let pointer = country
        .pointer
        .checked_add(ccountry::PRODUCTION_STATUS)
        .and_then(|address| read::<u64>(memory, address))
        .and_then(|pointer| usize::try_from(pointer).ok())
        .filter(|&pointer| pointer != 0)
        .ok_or("production status pointer unreadable or null")?;
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")?;
    let mut vtable = [0; 8];
    let mut owner = [0; 8];
    let copied = memory.copy_scattered(
        [pointer, pointer.checked_add(COUNTRY).unwrap_or(0)],
        &mut [&mut vtable, &mut owner],
    );
    if copied == 0 || u64::from_le_bytes(vtable) != expected as u64 {
        return Err("production status vtable mismatch");
    }
    if copied < 2 || u64::from_le_bytes(owner) != country.pointer as u64 {
        return Err("production status country mismatch");
    }
    Ok(pointer)
}

/// Null production status is native absence for the appended policy roster.
pub fn policies_header(
    memory: &impl Memory,
    base: usize,
    country: ccountry::Country,
) -> Result<Option<Array>, &'static str> {
    let pointer = country
        .pointer
        .checked_add(ccountry::PRODUCTION_STATUS)
        .and_then(|at| read::<u64>(memory, at))
        .and_then(|p| usize::try_from(p).ok())
        .ok_or("MIO policy production status unreadable")?;
    if pointer == 0 {
        return Ok(None);
    }
    if base.checked_add(VTABLE_RVA).map(|p| p as u64) != read::<u64>(memory, pointer) {
        return Err("MIO policy production status vtable mismatch");
    }
    if pointer
        .checked_add(COUNTRY)
        .and_then(|at| read::<u64>(memory, at))
        != Some(country.pointer as u64)
    {
        return Err("MIO policy production status owner mismatch");
    }
    pointer
        .checked_add(POLICIES)
        .and_then(|at| Array::read(memory, at, 8))
        .map(Some)
        .ok_or("MIO policy roster unreadable or invalid")
}

/// Every country's roster names the same few content `CPolicy` objects, so
/// each page of pointers is followed by one guarded batch of their identities
/// rather than a guarded read per policy. Rows, validation and the first
/// fault are those of a read per policy in roster order: a page's identities
/// are checked in order up to the first unreadable one, then a short page.
pub fn read_policies(
    memory: &impl Memory,
    base: usize,
    array: Array,
    output: &mut [u32],
) -> Result<(), &'static str> {
    const BATCH: usize = 64;
    const FIELDS: usize = POLICY_TOKEN + 4;
    if output.len() != array.count as usize {
        return Err("MIO policy roster arena capacity exceeded");
    }
    let vtable = base.checked_add(POLICY_VTABLE_RVA).map(|p| p as u64);
    let mut pointers = [[0_u8; 8]; BATCH];
    let mut fields = [[0_u8; FIELDS]; BATCH];
    for first in (0..array.count).step_by(BATCH) {
        let count = ((array.count - first) as usize).min(BATCH);
        let addresses: [usize; BATCH] = std::array::from_fn(|index| {
            if index < count {
                array.element(first + index as u32, 8).unwrap_or(0)
            } else {
                0
            }
        });
        let mut slots = pointers.iter_mut().enumerate();
        let copied = memory
            .copy_scattered(
                addresses,
                &mut std::array::from_fn(|_| {
                    let (index, bytes) = slots.next().unwrap();
                    &mut bytes[..if index < count { 8 } else { 0 }]
                }),
            )
            .min(count);
        let sources: [usize; BATCH] =
            std::array::from_fn(|index| u64::from_le_bytes(pointers[index]) as usize);
        let mut slots = fields.iter_mut().enumerate();
        let identified = memory
            .copy_scattered_prefetched(
                sources,
                &mut std::array::from_fn(|_| {
                    let (index, bytes) = slots.next().unwrap();
                    &mut bytes[..if index < copied { FIELDS } else { 0 }]
                }),
            )
            .min(copied);
        for index in 0..copied {
            if index == identified {
                return Err("MIO policy identity unreadable");
            }
            let identity = &fields[index];
            if vtable != Some(u64::from_le_bytes(identity[..8].try_into().unwrap())) {
                return Err("MIO policy vtable mismatch");
            }
            output[first as usize + index] =
                u32::from_le_bytes(identity[POLICY_TOKEN..].try_into().unwrap());
        }
        if copied < count {
            return Err("MIO policy pointer unreadable");
        }
    }
    Ok(())
}

/// One factory group as the last factory-total recalculation left it. Field
/// meanings and writers are in [`factory_group`]. Fixed5 fields keep the raw i64.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FactoryGroup {
    pub levels: i32,
    pub total_raw: i64,
    pub controlled_raw: i64,
    pub owned_raw: i64,
    pub damaged_raw: i64,
    pub used: i32,
    pub unavailable: i32,
    pub from_subjects: i32,
    pub to_overlord: i32,
    pub from_others: i32,
    pub to_others: i32,
}

/// One country's stored factory totals: the three factory groups, the
/// civilian licence and trade exports, and each civilian consumer's allocated
/// count ([`civ_factory_consumer::ALLOCATED`]). These are the game's own
/// numbers, not a derivation from state buildings.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FactoryRow {
    /// Raw country reference: the canonical country's native id.
    pub country: u32,
    pub military: FactoryGroup,
    pub naval: FactoryGroup,
    pub civilian: FactoryGroup,
    pub civilian_license_exports: i32,
    pub civilian_trade_exports: i32,
    pub consumer_goods: i32,
    pub special_projects: i32,
    pub license_imports: i32,
    pub trade_imports: i32,
    pub contract_payments: i32,
}

/// The five civilian consumers in their constructor order with their vtables.
const CONSUMERS: [(usize, usize); 5] = [
    (
        CONSUMER_GOODS,
        civ_factory_consumer::CONSUMER_GOODS_VTABLE_RVA,
    ),
    (
        SPECIAL_PROJECTS,
        civ_factory_consumer::SPECIAL_PROJECTS_VTABLE_RVA,
    ),
    (
        LICENSED_PRODUCTION,
        civ_factory_consumer::LICENSED_PRODUCTION_VTABLE_RVA,
    ),
    (TRADE, civ_factory_consumer::TRADE_VTABLE_RVA),
    (
        CONTRACT_PAYMENTS,
        civ_factory_consumer::CONTRACT_PAYMENTS_VTABLE_RVA,
    ),
];

/// Reads every canonical country's stored factory totals, one row per
/// country in country-table order. Each country costs the production status
/// pointer read and one guarded scatter of its vtable, owner and the factory
/// span (the three groups and the five consumers). The status and each
/// consumer are authenticated by vtable and the status by its owner, so a
/// moved layout fails rather than yielding plausible counts.
///
/// Any country whose status cannot be read or authenticated makes the table
/// unavailable: a missing country is not a country without factories. Rows
/// past capacity are counted unread without reading their statuses.
pub fn walk_factories(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [FactoryRow],
) -> Outcome {
    walk_factories_buffer(memory, base, game_state, Buffer::Fixed(output))
}

/// Worker acquisition grows from the original country census; canonical
/// countries own the cached totals rebuilt on update.
pub fn walk_factories_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<FactoryRow>,
) -> Outcome {
    walk_factories_buffer(memory, base, game_state, Buffer::Growing(output))
}

fn walk_factories_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: Buffer<'_, FactoryRow>,
) -> Outcome {
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        output.ensure(countries.count().saturating_sub(1) as usize)?;
        let mut vtables = [0; 6];
        for (vtable, rva) in vtables
            .iter_mut()
            .zip(std::iter::once(VTABLE_RVA).chain(CONSUMERS.iter().map(|&(_, rva)| rva)))
        {
            *vtable = base.checked_add(rva).ok_or("module address overflow")?;
        }
        let mut len = 0_usize;
        let mut unread_rows = 0_u32;
        let mut found_country = false;
        // The countries whose rows the next batch reads, in slot order.
        let mut batch = [None::<ccountry::Country>; BATCH];
        let mut pending = 0;
        for slot in 1..countries.count() {
            let country = match countries.at(slot) {
                Ok(Some(country)) => country,
                Ok(None) => continue,
                Err(reason) => {
                    // The serial walk read the earlier countries' statuses
                    // first: their failure comes first.
                    read_batch(memory, &vtables, &batch[..pending], &mut output, &mut len)?;
                    return Err(reason);
                }
            };
            found_country = true;
            if len + pending >= output.len() {
                unread_rows = unread_rows
                    .checked_add(1)
                    .ok_or("factory unread row count overflow")?;
                continue;
            }
            batch[pending] = Some(country);
            pending += 1;
            if pending == BATCH {
                read_batch(memory, &vtables, &batch[..pending], &mut output, &mut len)?;
                pending = 0;
            }
        }
        read_batch(memory, &vtables, &batch[..pending], &mut output, &mut len)?;
        if !found_country {
            Err("country table has no readable countries")
        } else if unread_rows == 0 {
            Ok(Outcome::Rows { len })
        } else {
            Ok(Outcome::Truncated { len, unread_rows })
        }
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

/// Countries whose status pointers, then status spans, one batch gathers.
const BATCH: usize = 32;
/// A country's production status pointer.
const POINTER_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: ccountry::PRODUCTION_STATUS,
    destination_offset: 0,
    len: 8,
}];
const POINTER_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&POINTER_FIELDS, 8).expect("valid production status pointer layout");
/// A status's identity, owner and factory span, as [`read_factories`]
/// copies them.
const STATUS_FIELDS: [RecordField; 3] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: COUNTRY,
        destination_offset: 8,
        len: 8,
    },
    RecordField {
        source_offset: MILITARY_FACTORIES,
        destination_offset: 16,
        len: FACTORY_SPAN,
    },
];
const STATUS_BYTES: usize = 16 + FACTORY_SPAN;
const STATUS_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&STATUS_FIELDS, STATUS_BYTES).expect("valid production status layout");

/// The rows of `batch` (consecutive countries the walk reads, in slot
/// order) into `output[*len..]`: every status pointer in one guarded
/// gather, then every status's identity, owner and factory span in another,
/// each decoded with [`read_factories`]' checks in order. Healthy memory
/// gives the serial walk's rows and first failure. When a gather does not
/// complete, or a status pointer is null, the batch is read again country
/// by country, once ([`crate::batch_fallback`]), which gives the serial
/// walk's result for a fault that persists and never refuses more than it
/// for one that does not.
fn read_batch(
    memory: &impl Memory,
    vtables: &[usize; 6],
    batch: &[Option<ccountry::Country>],
    output: &mut Buffer<'_, FactoryRow>,
    len: &mut usize,
) -> Result<(), &'static str> {
    let count = batch.len();
    if count == 0 {
        return Ok(());
    }
    let mut countries = [0_usize; BATCH];
    for (pointer, country) in countries.iter_mut().zip(batch) {
        *pointer = country.expect("batched country").pointer;
    }
    let mut words = [0_u8; BATCH * 8];
    let mut statuses = [0_usize; BATCH];
    let gathered = memory.copy_records(
        RecordSources::Pointers(&countries[..count]),
        &POINTER_LAYOUT,
        &mut words[..count * 8],
    ) == count
        && statuses[..count]
            .iter_mut()
            .zip(words[..count * 8].chunks_exact(8))
            .all(|(status, word)| {
                *status =
                    usize::try_from(u64::from_le_bytes(word.try_into().unwrap())).unwrap_or(0);
                *status != 0
            });
    let mut records = [0_u8; BATCH * STATUS_BYTES];
    if gathered
        && memory.copy_records(
            RecordSources::Pointers(&statuses[..count]),
            &STATUS_LAYOUT,
            &mut records[..count * STATUS_BYTES],
        ) == count * STATUS_FIELDS.len()
    {
        for (country, record) in batch.iter().zip(records.chunks_exact(STATUS_BYTES)) {
            let country = country.expect("batched country");
            output[*len] = decode_factories(
                vtables,
                country,
                record[..8].try_into().unwrap(),
                record[8..16].try_into().unwrap(),
                &record[16..],
                3,
            )?;
            *len += 1;
        }
        return Ok(());
    }
    crate::batch_fallback::record(crate::batch_fallback::Batch::CountryFactories);
    for country in batch {
        output[*len] = read_factories(memory, vtables, country.expect("batched country"))?;
        *len += 1;
    }
    Ok(())
}

/// `vtables` holds the status vtable, then the consumers' in [`CONSUMERS`] order.
fn read_factories(
    memory: &impl Memory,
    vtables: &[usize; 6],
    country: ccountry::Country,
) -> Result<FactoryRow, &'static str> {
    let status = country
        .pointer
        .checked_add(ccountry::PRODUCTION_STATUS)
        .and_then(|address| read::<u64>(memory, address))
        .and_then(|pointer| usize::try_from(pointer).ok())
        .filter(|&pointer| pointer != 0)
        .ok_or("production status pointer unreadable or null")?;
    let mut vtable = [0; 8];
    let mut owner = [0; 8];
    let mut span = [0; FACTORY_SPAN];
    // The three independent spans, hinted together before the same ordered
    // copy.
    let copied = memory.copy_scattered_prefetched(
        [
            status,
            status.checked_add(COUNTRY).unwrap_or(0),
            status.checked_add(MILITARY_FACTORIES).unwrap_or(0),
        ],
        &mut [&mut vtable, &mut owner, &mut span],
    );
    decode_factories(vtables, country, vtable, owner, &span, copied)
}

/// A status's row from its `copied` leading spans (identity, owner, factory
/// span), with [`read_factories`]' checks in their order.
fn decode_factories(
    vtables: &[usize; 6],
    country: ccountry::Country,
    vtable: [u8; 8],
    owner: [u8; 8],
    span: &[u8],
    copied: usize,
) -> Result<FactoryRow, &'static str> {
    if copied == 0 || u64::from_le_bytes(vtable) != vtables[0] as u64 {
        return Err("production status vtable mismatch");
    }
    if copied < 2 || u64::from_le_bytes(owner) != country.pointer as u64 {
        return Err("production status country mismatch");
    }
    if copied < 3 {
        return Err("production status factory totals unreadable");
    }
    let at = |offset: usize| offset - MILITARY_FACTORIES;
    for (&(offset, _), &expected) in CONSUMERS.iter().zip(&vtables[1..]) {
        if i64_at(span, at(offset)) as u64 != expected as u64 {
            return Err("civilian factory consumer vtable mismatch");
        }
    }
    let group = |start: usize| {
        let field = |offset: usize| at(start) + offset;
        FactoryGroup {
            levels: i32_at(span, field(factory_group::LEVELS)),
            total_raw: i64_at(span, field(factory_group::TOTAL)),
            controlled_raw: i64_at(span, field(factory_group::CONTROLLED)),
            owned_raw: i64_at(span, field(factory_group::OWNED)),
            damaged_raw: i64_at(span, field(factory_group::DAMAGED)),
            used: i32_at(span, field(factory_group::USED)),
            unavailable: i32_at(span, field(factory_group::UNAVAILABLE)),
            from_subjects: i32_at(span, field(factory_group::FROM_SUBJECTS)),
            to_overlord: i32_at(span, field(factory_group::TO_OVERLORD)),
            from_others: i32_at(span, field(factory_group::FROM_OTHERS)),
            to_others: i32_at(span, field(factory_group::TO_OTHERS)),
        }
    };
    let civilian = at(CIVILIAN_FACTORIES);
    let allocated = |consumer: usize| i32_at(span, at(consumer) + civ_factory_consumer::ALLOCATED);
    Ok(FactoryRow {
        country: country.id,
        military: group(MILITARY_FACTORIES),
        naval: group(NAVAL_FACTORIES),
        civilian: group(CIVILIAN_FACTORIES),
        civilian_license_exports: i32_at(span, civilian + factory_group::LICENSE_EXPORTS),
        civilian_trade_exports: i32_at(span, civilian + factory_group::TRADE_EXPORTS),
        consumer_goods: allocated(CONSUMER_GOODS),
        special_projects: allocated(SPECIAL_PROJECTS),
        license_imports: allocated(LICENSED_PRODUCTION),
        trade_imports: allocated(TRADE),
        contract_payments: allocated(CONTRACT_PAYMENTS),
    })
}

#[cfg(test)]
mod factory_tests;

fn i32_at(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn i64_at(bytes: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

/// Equipment-roster entry retaining order and repeats. Serialization writes
/// primary and foreign rosters separately; land-model fallback appends foreign
/// entries after primary ones. The third roster stores best-variant candidates
/// for archetype lookup and is mutated by additions/removals, never serialized.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AvailableEquipmentRow {
    /// Raw country reference of the roster's production manager owner.
    pub country: u32,
    /// 0 for the primary roster, 1 for the foreign one, 2 for the best-variant candidates.
    pub roster_kind: u8,
    /// Position within its own roster.
    pub ordinal: u32,
    /// Authenticated CEquipmentVariant handle.
    pub variant: u64,
}

/// CProductionStatus primary equipment roster, 8-byte CEquipmentVariant pointers.
pub const AVAILABLE_EQUIPMENT: usize = 0xa0;
/// CProductionStatus foreign (licensed or lent) equipment roster.
pub const FOREIGN_EQUIPMENT: usize = 0xe8;
/// CProductionStatus best-variant candidate list, with its count.
pub const BEST_VARIANT_CANDIDATES: usize = 0xb8;

/// Every country's three equipment rosters. Rows past capacity are counted.
/// Each roster's pointers are copied in spans, not one read per entry.
pub fn walk_available_equipment(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [AvailableEquipmentRow],
) -> Outcome {
    walk_available_equipment_buffer(memory, base, game_state, Buffer::Fixed(output))
}

/// Worker-only capture. Primary and foreign rosters are serialized; the
/// best-variant roster is maintained by equipment updates. Grow from each
/// original header before ordered entries, retaining repeated variant rows.
pub fn walk_available_equipment_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<AvailableEquipmentRow>,
) -> Outcome {
    walk_available_equipment_buffer(memory, base, game_state, Buffer::Growing(output))
}

fn walk_available_equipment_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: Buffer<'_, AvailableEquipmentRow>,
) -> Outcome {
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let mut len = 0usize;
        let mut unread_rows = 0_u32;
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            let status = manager(memory, base, country)?;
            for (roster_kind, offset) in [
                (0, AVAILABLE_EQUIPMENT),
                (1, FOREIGN_EQUIPMENT),
                (2, BEST_VARIANT_CANDIDATES),
            ] {
                let roster = status
                    .checked_add(offset)
                    .and_then(|at| Array::read(memory, at, 8))
                    .ok_or("equipment roster invalid")?;
                output.ensure(
                    len.checked_add(roster.count as usize)
                        .ok_or("equipment roster sizing overflow")?,
                )?;
                let kept = (roster.count as usize).min(output.len() - len);
                unread_rows = unread_rows
                    .checked_add(roster.count - kept as u32)
                    .ok_or("equipment roster count overflow")?;
                const SPAN: usize = 64;
                let mut pointers = [0_u8; SPAN * 8];
                for first in (0..kept).step_by(SPAN) {
                    let count = (kept - first).min(SPAN);
                    let at = roster
                        .element(first as u32, 8)
                        .ok_or("equipment roster index invalid")?;
                    if !read_slice(memory, at, &mut pointers[..count * 8]) {
                        return Err("equipment roster pointers unreadable");
                    }
                    for (index, bytes) in pointers[..count * 8].chunks_exact(8).enumerate() {
                        let pointer =
                            usize::try_from(u64::from_le_bytes(bytes.try_into().unwrap()))
                                .ok()
                                .filter(|pointer| *pointer != 0)
                                .ok_or("equipment roster pointer null")?;
                        output[len] = AvailableEquipmentRow {
                            country: country.id,
                            roster_kind,
                            ordinal: (first + index) as u32,
                            variant: cequipment_variant::identity(memory, base, pointer)?,
                        };
                        len += 1;
                    }
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

/// Reads each canonical country's ordered equipment-production roster once.
/// Construction uses its independent roster and is not silently filtered here.
/// Once capacity is reached, complete remaining roster lengths are counted;
/// line payloads are deferred to the next enlarged buffer.
pub fn walk_lines(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [LineRow],
) -> Outcome {
    walk_lines_buffer(memory, base, game_state, crate::walk::Buffer::Fixed(output))
}

pub fn walk_lines_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: crate::walk::Buffer<'_, LineRow>,
) -> Outcome {
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let mut len = 0_usize;
        let mut unread_rows = 0_u32;
        let mut found_country = false;
        let mut defines = crate::cequipment_production_line::Defines::default();
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            found_country = true;
            let status = manager(memory, base, country)?;
            let array = status
                .checked_add(LINES)
                .and_then(|address| Array::read(memory, address, LINE_POINTER_STRIDE))
                .ok_or("production line array unreadable or invalid")?;
            output.ensure(
                len.checked_add(array.count as usize)
                    .ok_or("production row count overflow")?,
            )?;
            for index in 0..array.count {
                if len == output.len() {
                    unread_rows = unread_rows
                        .checked_add(array.count - index)
                        .ok_or("production line unread row count overflow")?;
                    break;
                }
                let pointer = array
                    .element(index, LINE_POINTER_STRIDE)
                    .and_then(|address| read::<u64>(memory, address))
                    .and_then(|pointer| usize::try_from(pointer).ok())
                    .filter(|&pointer| pointer != 0)
                    .ok_or("production line pointer unreadable or null")?;
                output[len] = crate::cequipment_production_line::read_row(
                    memory,
                    base,
                    status,
                    country.id,
                    pointer,
                    &mut defines,
                )?;
                len += 1;
            }
        }
        if !found_country {
            Err("country table has no readable countries")
        } else if unread_rows == 0 {
            Ok(Outcome::Rows { len })
        } else {
            Ok(Outcome::Truncated { len, unread_rows })
        }
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

/// Reads every canonical country's licensed-variant container in stored order.
/// An unreadable header or a null entry inside the counted span fails the whole
/// table: a shortened licence list is indistinguishable from a genuine one.
pub fn walk_licenses(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [LicenseRow],
) -> Outcome {
    walk_licenses_buffer(memory, base, game_state, crate::walk::Buffer::Fixed(output))
}

pub fn walk_licenses_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: crate::walk::Buffer<'_, LicenseRow>,
) -> Outcome {
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let mut len = 0_usize;
        let mut unread_rows = 0_u32;
        let mut found_country = false;
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            found_country = true;
            let status = manager(memory, base, country)?;
            let array = status
                .checked_add(LICENSES)
                .and_then(|address| Array::read(memory, address, LICENSE_POINTER_STRIDE))
                .ok_or("production licence array unreadable or invalid")?;
            output.ensure(
                len.checked_add(array.count as usize)
                    .ok_or("production row count overflow")?,
            )?;
            for ordinal in 0..array.count {
                if len == output.len() {
                    unread_rows = unread_rows
                        .checked_add(array.count - ordinal)
                        .ok_or("production licence unread row count overflow")?;
                    break;
                }
                let pointer = array
                    .element(ordinal, LICENSE_POINTER_STRIDE)
                    .and_then(|address| read::<u64>(memory, address))
                    .and_then(|pointer| usize::try_from(pointer).ok())
                    .filter(|pointer| *pointer != 0)
                    .ok_or("production licence pointer unreadable or null")?;
                output[len] = LicenseRow {
                    country: country.id,
                    ordinal,
                    variant: cequipment_variant::identity(memory, base, pointer)?,
                };
                len += 1;
            }
        }
        if !found_country {
            Err("country table has no readable countries")
        } else if unread_rows == 0 {
            Ok(Outcome::Rows { len })
        } else {
            Ok(Outcome::Truncated { len, unread_rows })
        }
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

/// One licence-record walk: its outcome and the handle-arena use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LicenseRecordOutcome {
    pub outcome: Outcome,
    pub handles_len: usize,
    /// Full produced-handle need of the records this pass actually reached.
    pub required_handles: usize,
}

/// Reads every canonical country's owned-licence relations in stored order.
/// The container's granted-licence array is the same objects seen from the
/// licensor and is a reverse index, so it is not walked a second time.
pub fn walk_license_records(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [LicenseRecordRow],
    handles: &mut [u64],
) -> LicenseRecordOutcome {
    walk_license_records_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Fixed(output),
        crate::walk::Buffer::Fixed(handles),
    )
}

pub fn walk_license_records_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: crate::walk::Buffer<'_, LicenseRecordRow>,
    mut handles: crate::walk::Buffer<'_, u64>,
) -> LicenseRecordOutcome {
    let mut len = 0_usize;
    let mut used = 0;
    let mut required = 0_usize;
    let mut unread_rows = 0_u32;
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let mut found_country = false;
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            found_country = true;
            let status = manager(memory, base, country)?;
            let array = status
                .checked_add(LICENSE_RECORDS)
                .and_then(|address| Array::read(memory, address, LICENSE_RECORD_STRIDE))
                .ok_or("production licence record array unreadable or invalid")?;
            output.ensure(
                len.checked_add(array.count as usize)
                    .ok_or("production row count overflow")?,
            )?;
            for ordinal in 0..array.count {
                let pointer = array
                    .element(ordinal, LICENSE_RECORD_STRIDE)
                    .and_then(|address| read::<u64>(memory, address))
                    .and_then(|pointer| usize::try_from(pointer).ok())
                    .filter(|pointer| *pointer != 0)
                    .ok_or("production licence record pointer unreadable or null")?;
                if len == output.len() {
                    // A record's produced-handle count is only knowable by
                    // reading it, so stop counting handles once rows stop.
                    unread_rows = unread_rows
                        .checked_add(array.count - ordinal)
                        .ok_or("production licence unread row count overflow")?;
                    break;
                }
                let (row, next, wanted) = cequipment_production_license::read_row_buffer(
                    memory,
                    base,
                    country.id,
                    ordinal,
                    pointer,
                    &mut handles,
                    used,
                )?;
                required = required.saturating_add(wanted);
                match row {
                    Some(row) => {
                        output[len] = row;
                        len += 1;
                        used = next;
                    }
                    None => unread_rows += 1,
                }
            }
        }
        if found_country {
            Ok(())
        } else {
            Err("country table has no readable countries")
        }
    })();
    match result {
        Err(reason) => LicenseRecordOutcome {
            outcome: Outcome::Unavailable { reason },
            handles_len: 0,
            required_handles: 0,
        },
        Ok(()) => LicenseRecordOutcome {
            outcome: if unread_rows == 0 {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated { len, unread_rows }
            },
            handles_len: used,
            required_handles: required,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cequipment_production_line as equipment, cgame_state, cmilitary_production_line,
        cnaval_production_line, cproduction_line, memory::SliceMemory,
    };

    const BASE: usize = 0x1000;
    const COUNTRY_POINTER: usize = 0x3000;
    const STATUS: usize = 0x5000;
    const FIRST_LINE: usize = 0x7000;
    const SECOND_LINE: usize = 0x7200;
    const VARIANT: usize = 0x8000;
    const RECORD: usize = 0x6400;
    /// The variant's equipment type; its archetype link points to itself, as
    /// the constructor leaves it.
    const EQUIPMENT: usize = 0x8800;
    const FIRST_EFFICIENCIES: usize = 0x7400;
    const SECOND_EFFICIENCIES: usize = 0x7600;
    const EFFICIENCY_COUNT: u32 = 20;
    const Q15: i64 = 32_768;

    fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
        bytes[address - BASE..address - BASE + value.len()].copy_from_slice(value);
    }

    fn array(bytes: &mut [u8], header: usize, data: usize, count: u32) {
        put(bytes, header, &(data as u64).to_le_bytes());
        put(bytes, header + 8, &count.to_le_bytes());
        put(bytes, header + 12, &count.to_le_bytes());
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x8000];
        array(&mut bytes, BASE + cgame_state::COUNTRIES, 0x2000, 3);
        array(&mut bytes, BASE + cgame_state::COUNTRY_LINKS, 0x2100, 3);
        // Current id 2 aliases canonical slot 1; both table slots are occupied.
        for slot in [1, 2] {
            put(
                &mut bytes,
                0x2000 + slot * 8,
                &(COUNTRY_POINTER as u64).to_le_bytes(),
            );
            put(&mut bytes, 0x2100 + slot * 4, &1_u32.to_le_bytes());
        }
        for (address, value) in [
            (COUNTRY_POINTER, ccountry::VTABLE_RVA as u64),
            (COUNTRY_POINTER + ccountry::PRODUCTION_STATUS, STATUS as u64),
            (STATUS, VTABLE_RVA as u64),
            (STATUS + COUNTRY, COUNTRY_POINTER as u64),
            (0x6000, FIRST_LINE as u64),
            (0x6008, SECOND_LINE as u64),
            (VARIANT, cequipment_variant::VTABLE_RVA as u64),
            (VARIANT + cequipment_variant::TYPE, 0xfedc_ba98_0000_0046),
        ] {
            put(&mut bytes, address, &value.to_le_bytes());
        }
        put(
            &mut bytes,
            COUNTRY_POINTER + ccountry::ID,
            &2_u32.to_le_bytes(),
        );
        array(&mut bytes, STATUS + LINES, 0x6000, 2);
        array(&mut bytes, STATUS + LICENSES, 0x6100, 2);
        array(&mut bytes, STATUS + LICENSE_RECORDS, 0x6200, 1);
        put(&mut bytes, 0x6200, &(RECORD as u64).to_le_bytes());
        put(
            &mut bytes,
            RECORD,
            &(cequipment_production_license::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            RECORD + cequipment_production_license::OWNER,
            &2_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            RECORD + cequipment_production_license::UNSERIALIZED_OPERAND,
            &100_000_i64.to_le_bytes(),
        );
        for (offset, value) in [
            (cequipment_production_license::LENDED_CIC, i32::MIN),
            (cequipment_production_license::REQUIRED_CIC, i32::MAX),
            (cequipment_production_license::GIVER, -1),
            (cequipment_production_license::START_DATE, i32::MIN),
        ] {
            put(&mut bytes, RECORD + offset, &value.to_le_bytes());
        }
        put(
            &mut bytes,
            RECORD + cequipment_production_license::PARENT,
            &(VARIANT as u64).to_le_bytes(),
        );
        array(
            &mut bytes,
            RECORD + cequipment_production_license::EQUIPMENT,
            0x6300,
            2,
        );
        for slot in 0..2 {
            put(
                &mut bytes,
                0x6300 + slot * 8,
                &(VARIANT as u64).to_le_bytes(),
            );
        }
        for slot in 0..2 {
            put(
                &mut bytes,
                0x6100 + slot * 8,
                &(VARIANT as u64).to_le_bytes(),
            );
        }
        for (pointer, vtable, handle) in [
            (
                FIRST_LINE,
                cmilitary_production_line::VTABLE_RVA,
                0x8000_1234_0000_0038_u64,
            ),
            (
                SECOND_LINE,
                cnaval_production_line::VTABLE_RVA,
                0x1234_0000_0039,
            ),
        ] {
            for (offset, value) in [
                (0, vtable as u64),
                (cproduction_line::REFERENCE_TYPE, handle),
                (cproduction_line::PRODUCTION_STATUS, STATUS as u64),
                (cproduction_line::COST, 9_876_543_210),
                (cproduction_line::SPEED, (-4_294_967_297_i64) as u64),
                (cproduction_line::PRODUCED, 5_123_456_789),
                (equipment::EQUIPMENT_VARIANT, VARIANT as u64),
                (
                    equipment::INDUSTRIAL_ORGANISATION_CID,
                    0x1234_5678_0000_0095,
                ),
            ] {
                put(&mut bytes, pointer + offset, &value.to_le_bytes());
            }
            for (offset, value) in [
                (cproduction_line::ACTIVE_FACTORIES, 12_i32),
                (cproduction_line::PRIORITY, 17),
                (cproduction_line::AMOUNT, -1),
                (equipment::REQUESTED_FACTORIES, 14),
            ] {
                put(&mut bytes, pointer + offset, &value.to_le_bytes());
            }
        }
        // Ten factories at 53% and the remaining factories at 10%.
        for (line, data) in [
            (FIRST_LINE, FIRST_EFFICIENCIES),
            (SECOND_LINE, SECOND_EFFICIENCIES),
        ] {
            array(
                &mut bytes,
                line + equipment::FACTORY_EFFICIENCIES,
                data,
                EFFICIENCY_COUNT,
            );
            for index in 0..EFFICIENCY_COUNT as usize {
                let value = if index < 10 { 53 * Q15 } else { 10 * Q15 };
                put(&mut bytes, data + index * 8, &value.to_le_bytes());
            }
        }
        put(
            &mut bytes,
            VARIANT + cequipment_variant::DEFINITION,
            &(EQUIPMENT as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            EQUIPMENT,
            &(crate::cequipment_type::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            EQUIPMENT + crate::cequipment_type::ARCHETYPE,
            &(EQUIPMENT as u64).to_le_bytes(),
        );
        // The type sets both optionals, so no define is needed: 150 military
        // and 15 naval factories per line.
        for (offset, value) in [
            (crate::cequipment_type::MAX_MIL_FACTORIES, 150_i32),
            (crate::cequipment_type::MAX_NAV_FACTORIES, 15),
        ] {
            put(&mut bytes, EQUIPMENT + offset, &value.to_le_bytes());
            put(&mut bytes, EQUIPMENT + offset + 4, &[1]);
        }
        bytes
    }

    /// The fixture's memory plus the six maximum-factory defines at their
    /// module addresses (module base zero), each read as its own span.
    struct WithDefines<'a> {
        memory: SliceMemory<'a>,
        defines: Option<[i32; 6]>,
    }

    impl Memory for WithDefines<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            match equipment::MAX_FACTORY_DEFINE_RVAS
                .iter()
                .position(|&rva| rva == address)
            {
                Some(index) if destination.len() == 4 => match self.defines {
                    Some(values) => {
                        destination.copy_from_slice(&values[index].to_le_bytes());
                        true
                    }
                    None => false,
                },
                _ => self.memory.copy(address, destination),
            }
        }
    }

    const DEFINES: [i32; 6] = [150, 5, 3, 10, 2, 1];

    fn walk_first(bytes: &[u8], defines: Option<[i32; 6]>) -> Result<LineRow, Outcome> {
        let memory = WithDefines {
            memory: SliceMemory { base: BASE, bytes },
            defines,
        };
        let mut rows = [LineRow::default(); 2];
        match walk_lines(&memory, 0, BASE, &mut rows) {
            Outcome::Rows { len: 2 } => Ok(rows[0]),
            outcome => Err(outcome),
        }
    }

    fn set_line_kind(bytes: &mut [u8], vtable: usize) {
        put(bytes, FIRST_LINE, &(vtable as u64).to_le_bytes());
    }

    fn clear_type_maximum(bytes: &mut [u8], offset: usize) {
        put(bytes, EQUIPMENT + offset + 4, &[0]);
    }

    #[test]
    fn efficiency_averages_the_clamped_factory_prefix_in_q15() {
        let bytes = fixture();
        let row = walk_first(&bytes, None).unwrap();
        // (10 * 53 + 2 * 10) / 12 percent, truncated in Q15.
        assert_eq!(
            (row.max_factories, row.efficiency_raw),
            (150, 550 * Q15 / 12)
        );
        let mut bytes = fixture();
        put(
            &mut bytes,
            FIRST_LINE + cproduction_line::ACTIVE_FACTORIES,
            &200_i32.to_le_bytes(),
        );
        // 200 active clamp to the type's 150, beyond the array's 20 entries:
        // the reader refuses rather than reading past the array.
        assert_eq!(
            walk_first(&bytes, None),
            Err(Outcome::Unavailable {
                reason: "production line efficiency array shorter than its factories"
            })
        );
        put(
            &mut bytes,
            EQUIPMENT + crate::cequipment_type::MAX_MIL_FACTORIES,
            &20_i32.to_le_bytes(),
        );
        let row = walk_first(&bytes, None).unwrap();
        assert_eq!(row.max_factories, 20);
        assert_eq!(row.efficiency_raw, (10 * 53 + 10 * 10) * Q15 / 20);
        for active in [0_i32, -3] {
            put(
                &mut bytes,
                FIRST_LINE + cproduction_line::ACTIVE_FACTORIES,
                &active.to_le_bytes(),
            );
            // No entry is read without factories, not even a broken header.
            put(
                &mut bytes,
                FIRST_LINE + equipment::FACTORY_EFFICIENCIES,
                &0_u64.to_le_bytes(),
            );
            assert_eq!(walk_first(&bytes, None).unwrap().efficiency_raw, 0);
        }
    }

    #[test]
    fn maximum_factories_follow_the_type_archetype_and_kind_defines() {
        let mut bytes = fixture();
        let archetype = EQUIPMENT + 0x200;
        put(
            &mut bytes,
            EQUIPMENT + crate::cequipment_type::ARCHETYPE,
            &(archetype as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            archetype,
            &(crate::cequipment_type::VTABLE_RVA as u64).to_le_bytes(),
        );
        // The type's own optional wins over the archetype's.
        put(
            &mut bytes,
            archetype + crate::cequipment_type::MAX_MIL_FACTORIES,
            &[99, 0, 0, 0, 1],
        );
        assert_eq!(walk_first(&bytes, None).unwrap().max_factories, 150);
        clear_type_maximum(&mut bytes, crate::cequipment_type::MAX_MIL_FACTORIES);
        assert_eq!(walk_first(&bytes, None).unwrap().max_factories, 99);
        put(
            &mut bytes,
            archetype + crate::cequipment_type::MAX_MIL_FACTORIES + 4,
            &[0],
        );
        // Without either optional the defines are read, once per walk.
        assert_eq!(
            walk_first(&bytes, None),
            Err(Outcome::Unavailable {
                reason: "production line factory defines unreadable"
            })
        );
        assert_eq!(
            walk_first(&bytes, Some(DEFINES)).unwrap().max_factories,
            150
        );
        set_line_kind(&mut bytes, crate::crailway_gun_production_line::VTABLE_RVA);
        let row = walk_first(&bytes, Some(DEFINES)).unwrap();
        assert_eq!(row.max_factories, 1);
        // One factory of the twelve: 53%.
        assert_eq!(row.efficiency_raw, 53 * Q15);
        for vtable in [
            cnaval_production_line::VTABLE_RVA,
            crate::cship_refit_production_line::VTABLE_RVA,
        ] {
            set_line_kind(&mut bytes, vtable);
            // The naval optional is still set on the type.
            assert_eq!(walk_first(&bytes, Some(DEFINES)).unwrap().max_factories, 15);
            clear_type_maximum(&mut bytes, crate::cequipment_type::MAX_NAV_FACTORIES);
            for (flags, expected) in [
                (0_u64, 10),
                (cequipment_variant::CAPITAL_SHIP_CLASSIFIER_MASK, 2),
                (
                    cequipment_variant::FLOATING_HARBOR_CLASSIFIER_MASK
                        | cequipment_variant::CAPITAL_SHIP_CLASSIFIER_MASK,
                    3,
                ),
                (
                    cequipment_variant::CONVOY_CLASSIFIER_MASK
                        | cequipment_variant::FLOATING_HARBOR_CLASSIFIER_MASK,
                    5,
                ),
            ] {
                put(
                    &mut bytes,
                    VARIANT + cequipment_variant::CLASSIFIER_FLAGS,
                    &flags.to_le_bytes(),
                );
                assert_eq!(
                    walk_first(&bytes, Some(DEFINES)).unwrap().max_factories,
                    expected,
                    "{flags:#x}"
                );
            }
            put(
                &mut bytes,
                EQUIPMENT + crate::cequipment_type::MAX_NAV_FACTORIES + 4,
                &[1],
            );
        }
    }

    #[test]
    fn efficiency_and_type_failures_withhold_the_table() {
        for (address, value, reason) in [
            (
                FIRST_LINE + equipment::FACTORY_EFFICIENCIES + 12,
                EFFICIENCY_COUNT + 1,
                "production line efficiency array invalid",
            ),
            (
                FIRST_LINE + equipment::FACTORY_EFFICIENCIES + 12,
                11,
                "production line efficiency array shorter than its factories",
            ),
            (
                VARIANT + cequipment_variant::DEFINITION,
                0,
                "production line equipment type unreadable or null",
            ),
            (
                EQUIPMENT,
                0x1234,
                "production line equipment type vtable mismatch",
            ),
        ] {
            let mut bytes = fixture();
            put(&mut bytes, address, &value.to_le_bytes());
            assert_eq!(
                walk_first(&bytes, None),
                Err(Outcome::Unavailable { reason }),
                "{address:#x}"
            );
        }
        // The entries lie outside the readable memory.
        let mut bytes = fixture();
        put(
            &mut bytes,
            FIRST_LINE + equipment::FACTORY_EFFICIENCIES,
            &0x8ff8_u64.to_le_bytes(),
        );
        assert_eq!(
            walk_first(&bytes, None),
            Err(Outcome::Unavailable {
                reason: "production line efficiency entries unreadable"
            })
        );
        // An unreadable efficiency header, the last span of the line's
        // scatter, is reported after every other field was copied.
        struct Blocked<'a>(SliceMemory<'a>);
        impl Memory for Blocked<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                address != FIRST_LINE + equipment::FACTORY_EFFICIENCIES
                    && self.0.copy(address, destination)
            }
        }
        let bytes = fixture();
        assert_eq!(
            walk_lines(
                &Blocked(SliceMemory {
                    base: BASE,
                    bytes: &bytes
                }),
                0,
                BASE,
                &mut [LineRow::default(); 2]
            ),
            Outcome::Unavailable {
                reason: "production line efficiency header unreadable"
            }
        );
    }

    #[test]
    fn lines_preserve_native_widths_handles_aliases_and_capacity() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let mut rows = [LineRow::default(); 4];
        assert_eq!(
            walk_lines(&memory, 0, BASE, &mut rows),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(
            rows[0],
            LineRow {
                handle: 0x8000_1234_0000_0038,
                country: 2,
                kind: equipment::KIND_MILITARY,
                variant: 0xfedc_ba98_0000_0046,
                mio: 0x1234_5678_0000_0095,
                active_factories: 12,
                requested_factories: 14,
                cost_raw: 9_876_543_210,
                speed_raw: -4_294_967_297,
                produced_raw: 5_123_456_789,
                priority: 17,
                amount: -1,
                max_factories: 150,
                efficiency_raw: 550 * Q15 / 12,
            }
        );
        assert_eq!(rows[1].kind, equipment::KIND_NAVAL);
        assert_eq!(
            walk_lines(&memory, 0, BASE, &mut rows[..1]),
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
        assert_eq!(
            walk_lines(&memory, 0, BASE, &mut []),
            Outcome::Truncated {
                len: 0,
                unread_rows: 2
            }
        );
    }

    #[test]
    fn lines_distinguish_proven_empty_from_broken_roots_and_rows() {
        let mut bytes = fixture();
        array(&mut bytes, STATUS + LINES, 0, 0);
        assert_eq!(
            walk_lines(
                &SliceMemory {
                    base: BASE,
                    bytes: &bytes
                },
                0,
                BASE,
                &mut []
            ),
            Outcome::Rows { len: 0 }
        );
        for address in [
            COUNTRY_POINTER + ccountry::PRODUCTION_STATUS,
            STATUS,
            STATUS + COUNTRY,
            STATUS + LINES,
            0x6008,
            SECOND_LINE,
            SECOND_LINE + cproduction_line::PRODUCTION_STATUS,
            SECOND_LINE + equipment::EQUIPMENT_VARIANT,
            VARIANT,
        ] {
            let mut bytes = fixture();
            put(&mut bytes, address, &0_u64.to_le_bytes());
            let mut output = [LineRow::default(); 4];
            let outcome = walk_lines(
                &SliceMemory {
                    base: BASE,
                    bytes: &bytes,
                },
                0,
                BASE,
                &mut output,
            );
            assert!(
                matches!(outcome, Outcome::Unavailable { .. }),
                "{address:#x}: {outcome:?}"
            );
        }
    }

    #[test]
    fn unknown_line_subtype_is_unavailable_not_silently_omitted() {
        let mut bytes = fixture();
        put(&mut bytes, SECOND_LINE, &0x1234_u64.to_le_bytes());
        let mut rows = [LineRow::default(); 2];
        assert_eq!(
            walk_lines(
                &SliceMemory {
                    base: BASE,
                    bytes: &bytes
                },
                0,
                BASE,
                &mut rows
            ),
            Outcome::Unavailable {
                reason: "production line vtable mismatch"
            }
        );
    }

    #[test]
    fn equipment_rosters_keep_all_three_rosters_in_order_with_repeats() {
        let mut bytes = fixture();
        // The licence list's two entries double as the primary roster; the
        // foreign roster and the best-variant candidates repeat it once each.
        array(&mut bytes, STATUS + AVAILABLE_EQUIPMENT, 0x6100, 2);
        array(&mut bytes, STATUS + FOREIGN_EQUIPMENT, 0x6100, 1);
        array(&mut bytes, STATUS + BEST_VARIANT_CANDIDATES, 0x6108, 1);
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let mut rows = [AvailableEquipmentRow::default(); 4];
        assert_eq!(
            walk_available_equipment(&memory, 0, BASE, &mut rows),
            Outcome::Rows { len: 4 }
        );
        let entry = |roster_kind, ordinal| AvailableEquipmentRow {
            country: 2,
            roster_kind,
            ordinal,
            variant: 0xfedc_ba98_0000_0046,
        };
        assert_eq!(rows, [entry(0, 0), entry(0, 1), entry(1, 0), entry(2, 0)]);
        assert_eq!(
            walk_available_equipment(&memory, 0, BASE, &mut rows[..1]),
            Outcome::Truncated {
                len: 1,
                unread_rows: 3
            }
        );
    }

    #[test]
    fn licences_keep_container_order_full_handles_and_repeated_variants() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let mut rows = [LicenseRow::default(); 4];
        assert_eq!(
            walk_licenses(&memory, 0, BASE, &mut rows),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(
            &rows[..2],
            &[
                LicenseRow {
                    country: 2,
                    ordinal: 0,
                    variant: 0xfedc_ba98_0000_0046,
                },
                LicenseRow {
                    country: 2,
                    ordinal: 1,
                    variant: 0xfedc_ba98_0000_0046,
                },
            ]
        );
        assert_eq!(
            walk_licenses(&memory, 0, BASE, &mut rows[..1]),
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
        assert_eq!(
            walk_licenses(&memory, 0, BASE, &mut []),
            Outcome::Truncated {
                len: 0,
                unread_rows: 2
            }
        );
    }

    #[test]
    fn licence_records_keep_both_countries_signed_words_and_ordered_equipment() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let mut rows = [LicenseRecordRow::default(); 4];
        let mut handles = [0_u64; 8];
        let result = walk_license_records(&memory, 0, BASE, &mut rows, &mut handles);
        assert_eq!(
            result,
            LicenseRecordOutcome {
                outcome: Outcome::Rows { len: 1 },
                handles_len: 2,
                required_handles: 2,
            }
        );
        assert_eq!(
            rows[0],
            LicenseRecordRow {
                owner: 2,
                ordinal: 0,
                giver: u32::MAX,
                lended_cic: i32::MIN,
                required_cic: i32::MAX,
                start_date: i32::MIN,
                parent: 0xfedc_ba98_0000_0046,
                operand_raw: 100_000,
                equipment_start: 0,
                equipment_len: 2,
            }
        );
        assert_eq!(&handles[..2], &[0xfedc_ba98_0000_0046; 2]);
        // A full row buffer and a full handle arena both truncate exactly.
        assert_eq!(
            walk_license_records(&memory, 0, BASE, &mut [], &mut handles).outcome,
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
        let short = walk_license_records(&memory, 0, BASE, &mut rows, &mut [0; 1]);
        assert_eq!(
            short,
            LicenseRecordOutcome {
                outcome: Outcome::Truncated {
                    len: 0,
                    unread_rows: 1
                },
                handles_len: 0,
                required_handles: 2,
            }
        );
    }

    #[test]
    fn broken_licence_records_are_unavailable_and_an_empty_array_is_rows() {
        let mut bytes = fixture();
        array(&mut bytes, STATUS + LICENSE_RECORDS, 0, 0);
        assert_eq!(
            walk_license_records(
                &SliceMemory {
                    base: BASE,
                    bytes: &bytes
                },
                0,
                BASE,
                &mut [],
                &mut []
            )
            .outcome,
            Outcome::Rows { len: 0 }
        );
        for (address, reason) in [
            (
                0x6200,
                "production licence record pointer unreadable or null",
            ),
            (RECORD, "production licence vtable mismatch"),
            (
                RECORD + cequipment_production_license::OWNER,
                "production licence owner differs from its roster country",
            ),
            (
                RECORD + cequipment_production_license::PARENT,
                "production licence parent pointer unreadable or null",
            ),
        ] {
            let mut bytes = fixture();
            put(&mut bytes, address, &0_u64.to_le_bytes());
            assert_eq!(
                walk_license_records(
                    &SliceMemory {
                        base: BASE,
                        bytes: &bytes
                    },
                    0,
                    BASE,
                    &mut [LicenseRecordRow::default(); 4],
                    &mut [0; 8]
                )
                .outcome,
                Outcome::Unavailable { reason },
                "{address:#x}"
            );
        }
        let mut bytes = fixture();
        put(
            &mut bytes,
            STATUS + LICENSE_RECORDS_COUNT,
            &9_u32.to_le_bytes(),
        );
        assert_eq!(
            walk_license_records(
                &SliceMemory {
                    base: BASE,
                    bytes: &bytes
                },
                0,
                BASE,
                &mut [LicenseRecordRow::default(); 16],
                &mut [0; 64]
            )
            .outcome,
            Outcome::Unavailable {
                reason: "production licence record array unreadable or invalid"
            }
        );
    }

    #[test]
    fn a_proven_empty_licence_container_differs_from_a_broken_one() {
        let mut bytes = fixture();
        array(&mut bytes, STATUS + LICENSES, 0, 0);
        assert_eq!(
            walk_licenses(
                &SliceMemory {
                    base: BASE,
                    bytes: &bytes
                },
                0,
                BASE,
                &mut []
            ),
            Outcome::Rows { len: 0 }
        );
        for (address, reason) in [
            (0x6100, "production licence pointer unreadable or null"),
            (VARIANT, "variant vtable or reference type mismatch"),
        ] {
            let mut bytes = fixture();
            put(&mut bytes, address, &0_u64.to_le_bytes());
            assert_eq!(
                walk_licenses(
                    &SliceMemory {
                        base: BASE,
                        bytes: &bytes
                    },
                    0,
                    BASE,
                    &mut [LicenseRow::default(); 4]
                ),
                Outcome::Unavailable { reason },
                "{address:#x}"
            );
        }
        let mut bytes = fixture();
        put(&mut bytes, STATUS + LICENSES + 12, &9_u32.to_le_bytes());
        assert_eq!(
            walk_licenses(
                &SliceMemory {
                    base: BASE,
                    bytes: &bytes
                },
                0,
                BASE,
                &mut [LicenseRow::default(); 16]
            ),
            Outcome::Unavailable {
                reason: "production licence array unreadable or invalid"
            }
        );
    }
}
/// Slots of [`WarFactoryTemplates`]; a full table only reads again.
const WAR_FACTORY_TEMPLATE_SLOTS: usize = 1024;

/// Building templates already authenticated in one quiescent war overview pass,
/// with whether the war overview counts their buildings (the signed 32-bit sum
/// of the four production words is positive). The game waits on the pass, so a
/// template read again in it returns the same bytes: each is read once per pass
/// instead of once per building. Only a template whose record was read and
/// whose vtable matched is kept; nothing is kept across passes.
pub struct WarFactoryTemplates {
    keys: [usize; WAR_FACTORY_TEMPLATE_SLOTS],
    counted: [bool; WAR_FACTORY_TEMPLATE_SLOTS],
}

impl Default for WarFactoryTemplates {
    fn default() -> Self {
        Self::new()
    }
}

impl WarFactoryTemplates {
    pub fn new() -> Self {
        Self {
            keys: [0; WAR_FACTORY_TEMPLATE_SLOTS],
            counted: [false; WAR_FACTORY_TEMPLATE_SLOTS],
        }
    }

    /// The template's slot when kept, else the empty slot it would take
    /// (`None` when the table is full).
    fn slot(&self, template: usize) -> Result<usize, Option<usize>> {
        let start = ((template as u64 >> 3).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 54) as usize;
        for step in 0..WAR_FACTORY_TEMPLATE_SLOTS {
            let slot = (start + step) % WAR_FACTORY_TEMPLATE_SLOTS;
            match self.keys[slot] {
                key if key == template => return Ok(slot),
                0 => return Err(Some(slot)),
                _ => {}
            }
        }
        Err(None)
    }

    fn get(&self, template: usize) -> Option<bool> {
        if template == 0 {
            return None;
        }
        self.slot(template).ok().map(|slot| self.counted[slot])
    }

    fn insert(&mut self, template: usize, counted: bool) {
        if template == 0 {
            return;
        }
        if let Err(Some(slot)) = self.slot(template) {
            self.keys[slot] = template;
            self.counted[slot] = counted;
        }
    }
}

/// War-overview factory counts, rather than cached factory group totals. Core
/// occurrences, including nulls and repeats, and the terminal offsite pass stay
/// in native order. No native functions are called. `templates` holds the
/// pass's authenticated building templates.
pub fn war_overview_factories(
    memory: &impl crate::Memory,
    base: usize,
    countries: &crate::ccountry::Reader<'_, impl crate::Memory>,
    country: crate::ccountry::Country,
    templates: &mut WarFactoryTemplates,
) -> Result<i32, &'static str> {
    use crate::{
        cbuilding, cbuilding_status, cbuilding_template, ccountry, cpdx_array::Array, cstate, read,
    };
    fn field<T: crate::memory::Raw>(
        memory: &impl crate::Memory,
        pointer: usize,
        offset: usize,
    ) -> Result<T, &'static str> {
        pointer
            .checked_add(offset)
            .and_then(|p| read(memory, p))
            .ok_or("war factory field unreadable or address overflow")
    }
    fn authenticate(
        memory: &impl crate::Memory,
        base: usize,
        pointer: usize,
        rva: usize,
    ) -> Result<(), &'static str> {
        if pointer == 0 || read::<u64>(memory, pointer) != base.checked_add(rva).map(|p| p as u64) {
            Err("war factory vtable mismatch")
        } else {
            Ok(())
        }
    }
    let production = field::<u64>(memory, country.pointer, ccountry::PRODUCTION_STATUS)? as usize;
    authenticate(memory, base, production, VTABLE_RVA)?;
    if field::<u64>(memory, production, COUNTRY)? != country.pointer as u64 {
        return Err("war factory production owner mismatch");
    }
    let core = country
        .pointer
        .checked_add(ccountry::WAR_OVERVIEW_CORE_STATES)
        .and_then(|p| Array::read(memory, p, 8))
        .ok_or("war factory core array invalid or unreadable")?;
    let mut total = 0_i32;
    for index in 0..=core.count {
        let state = if index == core.count {
            0
        } else {
            core.element(index, 8)
                .and_then(|p| read::<u64>(memory, p))
                .ok_or("war factory core pointer unreadable")? as usize
        };
        let buildings = if state == 0 {
            let status =
                field::<u64>(memory, country.pointer, ccountry::WAR_OVERVIEW_OFFSITE)? as usize;
            authenticate(memory, base, status, cbuilding_status::VTABLE_RVA)?;
            status.checked_add(cbuilding_status::ACTIVE_BUILDINGS)
        } else {
            authenticate(memory, base, state, cstate::VTABLE_RVA)?;
            let controller = field::<u32>(memory, state, cstate::CONTROLLER)?;
            if controller == 0
                || countries.resolve(controller)?.canonical_slot != country.canonical_slot
            {
                continue;
            }
            state.checked_add(cstate::BUILDING_STATUS + cbuilding_status::ACTIVE_BUILDINGS)
        };
        let buildings = buildings
            .and_then(|p| Array::read(memory, p, 8))
            .ok_or("war factory building array invalid or unreadable")?;
        use crate::memory::{RecordLayout, RecordSources, packed_fields};
        const BUILDING_FIELDS: [crate::memory::RecordField; 2] =
            packed_fields([(0, 8), (cbuilding::TEMPLATE, 8)]);
        const BUILDING_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&BUILDING_FIELDS, 16).unwrap();
        const TEMPLATE_FIELDS: [crate::memory::RecordField; 5] = packed_fields([
            (0, 8),
            (cbuilding_template::WAR_OVERVIEW_PRODUCTION[0], 4),
            (cbuilding_template::WAR_OVERVIEW_PRODUCTION[1], 4),
            (cbuilding_template::WAR_OVERVIEW_PRODUCTION[2], 4),
            (cbuilding_template::WAR_OVERVIEW_PRODUCTION[3], 4),
        ]);
        const TEMPLATE_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&TEMPLATE_FIELDS, 24).unwrap();
        const LEVEL_FIELDS: [crate::memory::RecordField; 1] =
            packed_fields([(cbuilding::LEVEL, 2)]);
        const LEVEL_LAYOUT: RecordLayout<'static> = RecordLayout::new(&LEVEL_FIELDS, 2).unwrap();
        for first in (0..buildings.count).step_by(64) {
            let count = (buildings.count - first).min(64) as usize;
            let mut pointers = [0; 64 * 8];
            if !crate::read_slice(
                memory,
                buildings
                    .element(first, 8)
                    .ok_or("war factory building address overflow")?,
                &mut pointers[..count * 8],
            ) {
                return Err("war factory building pointer unreadable");
            }
            let pointers: [usize; 64] = std::array::from_fn(|i| {
                u64::from_le_bytes(pointers[i * 8..i * 8 + 8].try_into().unwrap()) as usize
            });
            let mut records = [0; 64 * 16];
            if memory.copy_records(
                RecordSources::Pointers(&pointers[..count]),
                &BUILDING_LAYOUT,
                &mut records,
            ) != count * 2
            {
                return Err("war factory building record unreadable");
            }
            let mut sources = [0; 64];
            for index in 0..count {
                let record = &records[index * 16..];
                if u64::from_le_bytes(record[..8].try_into().unwrap())
                    != base
                        .checked_add(cbuilding::VTABLE_RVA)
                        .ok_or("war factory vtable address overflow")? as u64
                {
                    return Err("war factory vtable mismatch");
                }
                sources[index] = u64::from_le_bytes(record[8..16].try_into().unwrap()) as usize;
            }
            // The page's templates this pass has not authenticated yet, once
            // each. Every kept template was readable with its vtable, so this
            // page fails exactly when reading all of its templates would, with
            // the same reason.
            let mut unread = [0; 64];
            let mut unread_len = 0;
            for &template in &sources[..count] {
                if templates.get(template).is_none() && !unread[..unread_len].contains(&template) {
                    unread[unread_len] = template;
                    unread_len += 1;
                }
            }
            let mut counted = [false; 64];
            if unread_len != 0 {
                let mut definitions = [0; 64 * 24];
                if memory.copy_records(
                    RecordSources::Pointers(&unread[..unread_len]),
                    &TEMPLATE_LAYOUT,
                    &mut definitions,
                ) != unread_len * 5
                {
                    return Err("war factory template record unreadable");
                }
                for index in 0..unread_len {
                    let record = &definitions[index * 24..(index + 1) * 24];
                    if u64::from_le_bytes(record[..8].try_into().unwrap())
                        != base
                            .checked_add(cbuilding_template::VTABLE_RVA)
                            .ok_or("war factory template vtable address overflow")?
                            as u64
                    {
                        return Err("war factory vtable mismatch");
                    }
                    let sum = record[8..].chunks_exact(4).fold(0_i32, |sum, bytes| {
                        sum.wrapping_add(i32::from_le_bytes(bytes.try_into().unwrap()))
                    });
                    counted[index] = sum > 0;
                }
                for (&template, &counts) in unread[..unread_len].iter().zip(&counted) {
                    templates.insert(template, counts);
                }
            }
            let mut selected = [0; 64];
            let mut n = 0;
            for (&template, &pointer) in sources[..count].iter().zip(&pointers) {
                // A full pass table leaves this page's reads only in `counted`.
                let counts = match unread[..unread_len].iter().position(|&t| t == template) {
                    Some(position) => counted[position],
                    None => templates.get(template).expect("kept template"),
                };
                if counts {
                    selected[n] = pointer;
                    n += 1;
                }
            }
            if n != 0 {
                let mut levels = [0; 64 * 2];
                if memory.copy_records(
                    RecordSources::Pointers(&selected[..n]),
                    &LEVEL_LAYOUT,
                    &mut levels,
                ) != n
                {
                    return Err("war factory level record unreadable");
                }
                for bytes in levels[..n * 2].chunks_exact(2) {
                    total = total
                        .wrapping_add(i32::from(i16::from_le_bytes(bytes.try_into().unwrap())));
                }
            }
        }
    }
    Ok(total)
}

#[cfg(test)]
mod war_overview_tests {
    use super::*;
    use crate::{
        cbuilding, cbuilding_status, cbuilding_template, ccountry::synthetic::World, cstate,
    };

    #[test]
    fn core_control_null_offsite_occurrences_and_signed_wrapping_are_native() {
        let mut w = World::new(0, 2, 0x1600);
        let production = w.alloc(0x300);
        w.u64(production, VTABLE_RVA as u64);
        w.u64(production + COUNTRY, w.object(1) as u64);
        w.u64(w.object(1) + ccountry::PRODUCTION_STATUS, production as u64);
        let state = w.alloc(0x200);
        let foreign = w.alloc(0x200);
        let offsite = w.alloc(0x80);
        let building = w.alloc(0x200);
        let template = w.alloc(0x320);
        let cores = w.alloc(24);
        let vector = w.alloc(8);
        w.u64(state, cstate::VTABLE_RVA as u64);
        w.u32(state + cstate::CONTROLLER, 1);
        w.u64(foreign, cstate::VTABLE_RVA as u64);
        w.u32(foreign + cstate::CONTROLLER, 2);
        w.u64(offsite, cbuilding_status::VTABLE_RVA as u64);
        w.u64(w.object(1) + ccountry::WAR_OVERVIEW_OFFSITE, offsite as u64);
        w.u64(building, cbuilding::VTABLE_RVA as u64);
        w.u64(building + cbuilding::TEMPLATE, template as u64);
        w.put(building + cbuilding::LEVEL, &3_i16.to_le_bytes());
        w.u64(template, cbuilding_template::VTABLE_RVA as u64);
        w.u32(template + cbuilding_template::WAR_OVERVIEW_PRODUCTION[3], 1);
        w.u64(vector, building as u64);
        for status in [
            state + cstate::BUILDING_STATUS,
            foreign + cstate::BUILDING_STATUS,
            offsite,
        ] {
            w.array(status + cbuilding_status::ACTIVE_BUILDINGS, vector, 1);
        }
        w.u64(cores, state as u64);
        w.u64(cores + 8, foreign as u64);
        w.array(w.object(1) + ccountry::WAR_OVERVIEW_CORE_STATES, cores, 3);
        let count = |w: &World| {
            let memory = w.memory();
            let countries = ccountry::Reader::new(&memory, 0, w.game_state()).unwrap();
            war_overview_factories(
                &memory,
                0,
                &countries,
                countries.resolve(1).unwrap(),
                &mut WarFactoryTemplates::new(),
            )
        };
        // One controlled core, a null-core offsite pass and the terminal pass.
        assert_eq!(count(&w), Ok(9));
        w.u64(production, 0);
        assert!(count(&w).is_err());
        w.u64(production, VTABLE_RVA as u64);
        w.u64(production + COUNTRY, w.object(2) as u64);
        assert_eq!(count(&w), Err("war factory production owner mismatch"));
        w.u64(production + COUNTRY, w.object(1) as u64);
        w.put(building + cbuilding::LEVEL, &(-3_i16).to_le_bytes());
        assert_eq!(count(&w), Ok(-9));
        w.u32(
            template + cbuilding_template::WAR_OVERVIEW_PRODUCTION[0],
            i32::MAX as u32,
        );
        assert_eq!(count(&w), Ok(0)); // positive words wrap to i32::MIN.
        w.u32(template + cbuilding_template::WAR_OVERVIEW_PRODUCTION[0], 0);
        w.array(w.object(1) + ccountry::WAR_OVERVIEW_CORE_STATES, 0, 0);
        let many = w.alloc(65_539 * 8);
        for i in 0..65_539 {
            w.u64(many + i * 8, building as u64);
        }
        w.array(offsite + cbuilding_status::ACTIVE_BUILDINGS, many, 65_539);
        w.put(building + cbuilding::LEVEL, &i16::MAX.to_le_bytes());
        assert_eq!(count(&w), Ok(65_539_i32.wrapping_mul(i32::from(i16::MAX))));
        w.u64(template, 0);
        assert!(count(&w).is_err());
    }

    /// A pass reads each building template once, not once per building:
    /// the same count comes from fewer source requests, across countries too.
    #[test]
    fn a_pass_reads_each_building_template_once() {
        use crate::memory::{CountedMemory, ReadCounts};
        let mut w = World::new(0, 2, 0x1600);
        let counted = w.alloc(0x320);
        let skipped = w.alloc(0x320);
        w.u64(counted, cbuilding_template::VTABLE_RVA as u64);
        w.u32(counted + cbuilding_template::WAR_OVERVIEW_PRODUCTION[1], 2);
        w.u64(skipped, cbuilding_template::VTABLE_RVA as u64);
        let vector = w.alloc(6 * 8);
        for index in 0..6_usize {
            let building = w.alloc(0x200);
            w.u64(building, cbuilding::VTABLE_RVA as u64);
            let template = if index % 3 == 2 { skipped } else { counted };
            w.u64(building + cbuilding::TEMPLATE, template as u64);
            w.put(
                building + cbuilding::LEVEL,
                &(index as i16 + 1).to_le_bytes(),
            );
            w.u64(vector + index * 8, building as u64);
        }
        for id in [1, 2] {
            let production = w.alloc(0x300);
            w.u64(production, VTABLE_RVA as u64);
            w.u64(production + COUNTRY, w.object(id) as u64);
            w.u64(
                w.object(id) + ccountry::PRODUCTION_STATUS,
                production as u64,
            );
            let offsite = w.alloc(0x80);
            w.u64(offsite, cbuilding_status::VTABLE_RVA as u64);
            w.array(offsite + cbuilding_status::ACTIVE_BUILDINGS, vector, 6);
            w.u64(
                w.object(id) + ccountry::WAR_OVERVIEW_OFFSITE,
                offsite as u64,
            );
        }
        let plain = w.memory();
        let countries = ccountry::Reader::new(&plain, 0, w.game_state()).unwrap();
        let memory = CountedMemory::new(w.memory());
        let mut templates = WarFactoryTemplates::new();
        let prior = ReadCounts::take_current();
        let mut spans = Vec::new();
        for id in [1, 2] {
            let country = countries.resolve(id).unwrap();
            // Levels 1, 2, 4 and 5 count; 3 and 6 have a zero sum.
            let result = war_overview_factories(&memory, 0, &countries, country, &mut templates);
            spans.push(ReadCounts::take_current().spans);
            assert_eq!(result, Ok(12));
        }
        prior.add_current();
        // The first country reads its two templates once each (five fields
        // apiece) rather than once per building; the second reads neither.
        assert_eq!(spans[0] - spans[1], 2 * 5, "{spans:?}");
        // Six buildings: a vtable and template each, four levels, and the
        // fixed production, core and offsite reads besides.
        assert!(spans[1] < 6 * 5, "{spans:?}");
    }
}

#[cfg(test)]
mod policy_tests {
    use super::*;
    use crate::memory::SliceMemory;
    use std::cell::Cell;

    const BASE: usize = 0x10_0000;
    /// Four shared policies, then the roster at the end of readable memory.
    const POLICIES: usize = 0x1000;
    const STRIDE: usize = 0x20;
    const ROSTER: usize = 0x2000;

    /// Counts guarded calls: one per copy and one per scattered batch.
    struct Guarded<'a> {
        bytes: &'a [u8],
        guards: Cell<usize>,
    }

    impl<'a> Guarded<'a> {
        fn new(bytes: &'a [u8]) -> Self {
            Self {
                bytes,
                guards: Cell::new(0),
            }
        }

        fn slice(&self) -> SliceMemory<'a> {
            SliceMemory {
                base: 0,
                bytes: self.bytes,
            }
        }
    }

    impl Memory for Guarded<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.guards.set(self.guards.get() + 1);
            self.slice().copy(address, destination)
        }

        fn copy_scattered<const N: usize>(
            &self,
            addresses: [usize; N],
            destinations: &mut [&mut [u8]; N],
        ) -> usize {
            self.guards.set(self.guards.get() + 1);
            let memory = self.slice();
            for (index, (address, destination)) in
                addresses.into_iter().zip(destinations).enumerate()
            {
                if !destination.is_empty() && (address == 0 || !memory.copy(address, destination)) {
                    return index;
                }
            }
            N
        }
    }

    /// A roster of `count` entries naming the four policies in turn; policy
    /// `index` has token 500 + index.
    fn world(count: usize) -> Vec<u8> {
        let mut bytes = vec![0; ROSTER + count * 8];
        for index in 0..4 {
            let policy = POLICIES + index * STRIDE;
            let vtable = (BASE + POLICY_VTABLE_RVA) as u64;
            bytes[policy..policy + 8].copy_from_slice(&vtable.to_le_bytes());
            let token = 500 + index as u32;
            bytes[policy + POLICY_TOKEN..policy + POLICY_TOKEN + 4]
                .copy_from_slice(&token.to_le_bytes());
        }
        for slot in 0..count {
            let policy = (POLICIES + slot % 4 * STRIDE) as u64;
            bytes[ROSTER + slot * 8..ROSTER + slot * 8 + 8].copy_from_slice(&policy.to_le_bytes());
        }
        bytes
    }

    /// A roster header of `count` entries at [`ROSTER`].
    fn roster(count: u32) -> Array {
        let mut header = [0_u8; 16];
        header[..8].copy_from_slice(&(ROSTER as u64).to_le_bytes());
        header[8..12].copy_from_slice(&count.to_le_bytes());
        header[12..].copy_from_slice(&count.to_le_bytes());
        Array::from_header(header, 8).unwrap()
    }

    fn read(bytes: &[u8], count: u32) -> Result<Vec<u32>, &'static str> {
        let mut output = vec![0; count as usize];
        read_policies(&Guarded::new(bytes), BASE, roster(count), &mut output).map(|()| output)
    }

    /// A page of policies costs two guarded calls, its pointers and then their
    /// identities, where a read per policy took one more call per policy.
    #[test]
    fn a_page_of_policies_is_read_in_two_guarded_batches() {
        for (count, guards) in [(20, 2), (64, 2), (130, 6)] {
            let bytes = world(count);
            let memory = Guarded::new(&bytes);
            let mut output = vec![0; count];
            assert_eq!(
                read_policies(&memory, BASE, roster(count as u32), &mut output),
                Ok(())
            );
            let expected: Vec<u32> = (0..count as u32).map(|slot| 500 + slot % 4).collect();
            assert_eq!(output, expected);
            assert_eq!(memory.guards.get(), guards, "{count} policies");
        }
    }

    /// The first fault in roster order stands, as in a read per policy.
    #[test]
    fn the_first_fault_in_roster_order_is_reported() {
        let foreign = |bytes: &mut [u8], policy: usize| {
            let at = POLICIES + policy * STRIDE;
            bytes[at..at + 8].copy_from_slice(&1_u64.to_le_bytes());
        };
        let null = |bytes: &mut [u8], slot: usize| bytes[ROSTER + slot * 8..][..8].fill(0);
        // A foreign policy (slot 1) before a null entry (slot 2).
        let mut bytes = world(8);
        foreign(&mut bytes, 1);
        null(&mut bytes, 2);
        assert_eq!(read(&bytes, 8), Err("MIO policy vtable mismatch"));
        // A null entry (slot 1) before a foreign policy (slot 2).
        let mut bytes = world(8);
        null(&mut bytes, 1);
        foreign(&mut bytes, 2);
        assert_eq!(read(&bytes, 8), Err("MIO policy identity unreadable"));
        // A header longer than the readable roster: its readable entries all
        // check, then the pointers fault.
        let bytes = world(8);
        assert_eq!(read(&bytes, 13), Err("MIO policy pointer unreadable"));
        // A foreign policy before the unreadable pointers still comes first.
        let mut bytes = world(8);
        foreign(&mut bytes, 3);
        assert_eq!(read(&bytes, 13), Err("MIO policy vtable mismatch"));
    }
}
