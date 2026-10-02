//! CLogisticsStatus, each country's logistics history, reached through CCountry+0xf98.
//!
//! Its constructor (CLogisticsStatus::CLogisticsStatus@14135aa20) builds 19
//! CLoopHistory series whose records hold one fixed5 value per equipment
//! archetype, at the archetype's CEquipmentType+0x538: its position in
//! CEquipmentDatabase+0x80. Once a day CGameState::DailyUpdate@1401d4390 runs
//! CCountry::PostDailyUpdate@1406f8a10 for every country (its
//! country.post_daily_parallel phase), and CLogisticsStatus::PostDailyUpdate
//! @14135ffa0 advances every series to a new record, which
//! CLoopHistoryContainerQueue::Advance@1415050b0 zeroes, then writes two of them:
//!
//! - series 15, the stock: each archetype total of CProductionStatus+0x208;
//! - series 16, the balance, but only when CGameState::IsCountryAIControlled
//!   @1401e1fe0 is false for the country (the check the engine names
//!   `naval_result_country_excluded`@1406f3120): per archetype,
//!   `Avail − clamp(Demand + PoolDifference)`. Avail is
//!   Logistics_GetAvailableEquipmentStockpile@14135dbc0: series 15, or for
//!   archetypes with effective category bit 0 the country's live pool cache
//!   (GetArchetypeTotal@14135c3f0), plus the supply system's truck and train
//!   terms, clamped at zero. Demand is ComputeArchetypeDemandAdjustment
//!   @14135de20 without a training list (training lines' missing equipment,
//!   the army's reinforcement requests, garrisons, the cached air requirement,
//!   operations, trucks and trains); PoolDifference is
//!   GetArchetypePoolDifference@14135c650 (outgoing minus incoming pools of
//!   owner+0x11f8's +0x758 entries). Both sums saturate at ±0x53e2d620fb40.
//!
//! Series 16 is the Logistics screen's "Stored" value (LOGISTICS_IN_STOCK,
//! "Total amount of Equipment surplus/deficit"):
//! CLogisticsOverviewEquipmentItem::Refresh@141d87030 shows Avail −
//! GetAdjustedArchetypePoolDifference@14135dda0, which is the same demand
//! (R9 zeroed at 14135ddaf) plus the same pool difference, clamped the same
//! way. The screen recomputes it at each refresh, so while the day passes it
//! can drift from the stored value as demand changes; the record keeps the
//! value the day's update computed. CLogisticsStatus::RefreshCurrentStockAndBalance
//! @1413607e0 rewrites both series in the current record, with the same
//! AI-control check, when equipment leaves or enters the stockpile outside the
//! daily update (stockpile deletion, send-equipment effects, market transfers
//! the view adds after each section's archetype rows
//! (CCountryLogisticsView::vt30_vf9@14171ea90 appends one
//! CLogisticsOverviewTypeItem per section, grouping the archetypes of its +0x540
//! list under GetLocalizedEquipmentCategoryName) show, in
//! CLogisticsOverviewTypeItem::Refresh@141d882d0, the sum of those archetypes'
//! values plus their incoming lend-lease
//! (GetIncomingLendLeaseArchetypeQuantity@14135dc90), which is not recorded.
//! A row's text is LOGISTICS_IN_STOCK with a suffix Logistics_SelectAmountTextKey
//! @141d81c80 picks by whether the value lies strictly within ±1000 units
//! (`_LOW_VALUE`, `$AMOUNT|^+=1$`, and `_HIGH_VALUE`, `$AMOUNT|^+=0$`; which
//! global holds which suffix was not traced).
//!
//! The reader copies the current daily record of both series only for
//! countries the predicate calls not AI-controlled at the read: for the others
//! the game never writes series 16, and series 15 repeats the stockpile the
//! Stockpiles table records. The old garrison-alert reader read the same
//! series 15 record through this object.

use crate::{
    Memory, ccountry, cequipment_database as database, cequipment_type, cgame_state, cloop_history,
    cpdx_array::Array,
    loop_history_record_fields as record_fields, loop_history_window as window,
    memory::{RecordField, RecordLayout, RecordSources},
    read, read_slice,
    walk::Outcome,
};

/// CLogisticsStatus primary vtable 0x14299cc88; the constructor@14135aa20 installs it first.
pub const VTABLE_RVA: usize = 0x0299_cc88;
/// CLogisticsStatus series CPdxArray of `CLoopHistory*` (data, capacity +0x10, count +0x14); the
/// constructor@14135aa20 grows it to 19 and PostDailyUpdate@14135ffa0 advances `count` series.
pub const SERIES: usize = 0x08;
/// Series the constructor@14135aa20 builds (it stores 0x13 in the count); the save writer
/// CLogisticsStatus::vf2@1413609d0 loops 0..0x13.
pub const SERIES_COUNT: u32 = 19;
/// CLogisticsStatus owning `CCountry*`; the constructor@14135aa20 stores its argument here and
/// every logistics getter reads the country through it.
pub const COUNTRY: usize = 0x20;
/// The stock series; PostDailyUpdate@14135ffa0 and RefreshCurrentStockAndBalance@1413607e0 write
/// CProductionStatus+0x208's archetype totals into series array +0x78.
pub const STOCK_SERIES: u32 = 15;
/// The balance series; PostDailyUpdate@14135ffa0 and RefreshCurrentStockAndBalance@1413607e0
/// write `Avail − (Demand + PoolDifference)` into series array +0x80.
pub const BALANCE_SERIES: u32 = 16;
/// Roll-up mode the constructor@14135aa20 gives the stock series (`series == 0x10` is false).
pub const STOCK_ROLL_UP: i32 = 0;
/// Roll-up mode the constructor@14135aa20 gives the balance series alone (`series == 0x10`).
pub const BALANCE_ROLL_UP: i32 = 1;

/// Bytes of the status copied: vtable, series header and owner.
const STATUS_BYTES: usize = COUNTRY + 8;
/// Bytes of a CLoopHistory copied: through the field count at +0x34.
const HISTORY_BYTES: usize = cloop_history::FIELD_COUNT + 4;
/// Bytes of a CLoopHistoryContainer copied: through the owning history at +0x38.
const WINDOW_BYTES: usize = window::HISTORY + 8;
/// Bytes of a record copied: its value header and owning history.
const RECORD_BYTES: usize = record_fields::HISTORY + 8;
/// Record values and archetype identities read per guarded batch.
const BATCH: usize = 64;

/// The fields of an archetype read to name a record index: vtable, script
/// token, index and archetype flag.
const IDENTITY_FIELDS: [RecordField; 4] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: cequipment_type::TOKEN,
        destination_offset: 8,
        len: 4,
    },
    RecordField {
        source_offset: cequipment_type::STOCKPILE_INDEX,
        destination_offset: 12,
        len: 4,
    },
    RecordField {
        source_offset: cequipment_type::IS_ARCHETYPE,
        destination_offset: 16,
        len: 1,
    },
];
const IDENTITY_STRIDE: usize = 24;
const IDENTITY_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&IDENTITY_FIELDS, IDENTITY_STRIDE).expect("valid record layout");

/// One archetype of a country the game computes the balance for: the value
/// of each series in the current daily record, fixed5.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// The country's own raw tag reference (CCountry+8).
    pub country: u32,
    /// The record index: the archetype's CEquipmentType+0x538 and its position
    /// in CEquipmentDatabase+0x80.
    pub archetype: u32,
    /// The archetype's script token (CEquipmentType+0x08).
    pub token: u32,
    /// Series 16: the Logistics screen's surplus (positive) or deficit.
    pub balance_raw: i64,
    /// Series 15: the stock the day's update counted.
    pub stock_raw: i64,
}

/// Reads the current daily record of the stock and balance series of every
/// authenticated canonical country IsCountryAIControlled calls not
/// AI-controlled, one row per archetype in record order, and names each
/// index through the equipment database's archetype list. AI-controlled
/// countries have no rows and none of their logistics memory is read.
///
/// The layout is authenticated from the status to each value: vtables, the
/// constructed 19 series, the owner and self references, each series' roll-up
/// mode, the 30-record daily window, the current slot's record, the two
/// series' agreement on that slot, and records sized by the database's
/// archetype count; every archetype must be one at its own index. Any other
/// form, and any failed read, makes the table unavailable. A country whose rows
/// do not fit is counted unread and its values are not read.
pub fn walk(memory: &impl Memory, base: usize, game_state: usize, rows: &mut [Row]) -> Outcome {
    match walk_inner(memory, base, game_state, rows) {
        Ok((len, 0)) => Outcome::Rows { len },
        Ok((len, unread_rows)) => Outcome::Truncated { len, unread_rows },
        Err(reason) => Outcome::Unavailable { reason },
    }
}

/// Module vtable addresses the layout is authenticated against.
struct Vtables {
    status: u64,
    history: u64,
    window: u64,
}

/// The equipment database's archetype list, read at the first country that
/// needs it, and where the rows already naming its indices start.
struct Archetypes {
    list: Array,
    named: Option<usize>,
}

fn walk_inner(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    rows: &mut [Row],
) -> Result<(usize, u32), &'static str> {
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    let mut control = cgame_state::AiControl::read(memory, game_state)?;
    let vtable = |rva: usize| {
        base.checked_add(rva)
            .map(|address| address as u64)
            .ok_or("logistics module address overflow")
    };
    let vtables = Vtables {
        status: vtable(VTABLE_RVA)?,
        history: vtable(cloop_history::VTABLE_RVA)?,
        window: vtable(window::VTABLE_RVA)?,
    };
    let mut archetypes: Option<Archetypes> = None;
    let mut len = 0;
    let mut unread_rows = 0_u32;
    let mut found = false;
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        found = true;
        if control.autonomous(memory, base, &countries, country.id, country.canonical_slot)? {
            continue;
        }
        let [stock, balance] = read_records(memory, &vtables, country)?;
        let list = match &archetypes {
            Some(archetypes) => archetypes.list,
            None => {
                let list = read_archetype_list(memory, base)?;
                archetypes = Some(Archetypes { list, named: None });
                list
            }
        };
        if stock.count != list.count || balance.count != list.count {
            return Err("logistics record size differs from the equipment archetype count");
        }
        let count = list.count as usize;
        if count > rows.len() - len {
            unread_rows = unread_rows
                .checked_add(list.count)
                .ok_or("logistics unread row count exceeds u32")?;
            continue;
        }
        let (done, rest) = rows.split_at_mut(len);
        let block = &mut rest[..count];
        let archetypes = archetypes.as_mut().expect("archetype list read above");
        match archetypes.named {
            Some(start) => {
                for (row, named) in block.iter_mut().zip(&done[start..start + count]) {
                    row.token = named.token;
                }
            }
            None => {
                read_identities(memory, base, list, block)?;
                archetypes.named = Some(len);
            }
        }
        read_values(memory, stock, balance, block)?;
        for (archetype, row) in (0..).zip(block.iter_mut()) {
            row.country = country.id;
            row.archetype = archetype;
        }
        len += count;
    }
    if !found {
        return Err("logistics country table has no countries");
    }
    Ok((len, unread_rows))
}

/// Follows one country's status to the current daily record of the stock
/// and balance series, returning each record's value array.
fn read_records(
    memory: &impl Memory,
    vtables: &Vtables,
    country: ccountry::Country,
) -> Result<[Array; 2], &'static str> {
    let status = pointer_at(memory, country.pointer, ccountry::LOGISTICS_STATUS)
        .filter(|&status| status != 0)
        .ok_or("logistics status pointer unreadable or null")?;
    let mut head = [0; STATUS_BYTES];
    if !read_slice(memory, status, &mut head) {
        return Err("logistics status unreadable");
    }
    if u64_at(&head, 0) != vtables.status {
        return Err("logistics status vtable mismatch");
    }
    if u64_at(&head, COUNTRY) != country.pointer as u64 {
        return Err("logistics status country mismatch");
    }
    let series = Array::from_header(head[SERIES..SERIES + 16].try_into().unwrap(), 8)
        .filter(|series| series.count == SERIES_COUNT)
        .ok_or("logistics series array is not the constructed 19 series")?;
    // The two series pointers are adjacent in the series array.
    let pair = series
        .element(STOCK_SERIES, 8)
        .and_then(|address| read::<[u8; 16]>(memory, address))
        .ok_or("logistics series pointers unreadable")?;
    let histories = [0, 8].map(|offset| u64_at(&pair, offset) as usize);
    if histories.contains(&0) {
        return Err("logistics series pointer null");
    }
    let mut stock_history = [0; HISTORY_BYTES];
    let mut balance_history = [0; HISTORY_BYTES];
    if memory.copy_scattered(histories, &mut [&mut stock_history, &mut balance_history]) != 2 {
        return Err("logistics series unreadable");
    }
    let windows = [
        check_history(vtables, histories[0], &stock_history, STOCK_ROLL_UP)?,
        check_history(vtables, histories[1], &balance_history, BALANCE_ROLL_UP)?,
    ];
    let mut stock_window = [0; WINDOW_BYTES];
    let mut balance_window = [0; WINDOW_BYTES];
    if memory.copy_scattered(windows, &mut [&mut stock_window, &mut balance_window]) != 2 {
        return Err("logistics daily window unreadable");
    }
    let stock = check_window(vtables, histories[0], &stock_window)?;
    let balance = check_window(vtables, histories[1], &balance_window)?;
    if stock.slot != balance.slot {
        return Err("logistics stock and balance daily slots disagree");
    }
    // Each window's slot entry and its current record, in one guarded copy.
    let mut stock_slot = [0; 8];
    let mut balance_slot = [0; 8];
    let mut stock_record = [0; RECORD_BYTES];
    let mut balance_record = [0; RECORD_BYTES];
    if memory.copy_scattered(
        [
            stock.slot_entry,
            balance.slot_entry,
            stock.record,
            balance.record,
        ],
        &mut [
            &mut stock_slot,
            &mut balance_slot,
            &mut stock_record,
            &mut balance_record,
        ],
    ) != 4
    {
        return Err("logistics current record unreadable");
    }
    Ok([
        check_record(
            histories[0],
            &stock,
            &stock_slot,
            &stock_record,
            &stock_history,
        )?,
        check_record(
            histories[1],
            &balance,
            &balance_slot,
            &balance_record,
            &balance_history,
        )?,
    ])
}

/// Authenticates a series and returns its daily window.
fn check_history(
    vtables: &Vtables,
    history: usize,
    bytes: &[u8; HISTORY_BYTES],
    roll_up: i32,
) -> Result<usize, &'static str> {
    if u64_at(bytes, 0) != vtables.history {
        return Err("logistics series vtable mismatch");
    }
    if u64_at(bytes, cloop_history::SELF_REFERENCE) != history as u64 {
        return Err("logistics series self reference mismatch");
    }
    if i32_at(bytes, cloop_history::ROLL_UP) != roll_up {
        return Err("logistics series roll-up mode differs from its constructed series");
    }
    match u64_at(bytes, cloop_history::DAILY_WINDOW) as usize {
        0 => Err("logistics daily window null"),
        window => Ok(window),
    }
}

/// A daily window's current slot, the address of its slot entry and the
/// record it names.
struct Current {
    slot: u32,
    slot_entry: usize,
    record: usize,
}

fn check_window(
    vtables: &Vtables,
    history: usize,
    bytes: &[u8; WINDOW_BYTES],
) -> Result<Current, &'static str> {
    if u64_at(bytes, 0) != vtables.window {
        return Err("logistics daily window vtable mismatch");
    }
    if u64_at(bytes, window::HISTORY) != history as u64 {
        return Err("logistics daily window owner mismatch");
    }
    let records = Array::from_header(
        bytes[window::RECORDS..window::RECORDS + 16]
            .try_into()
            .unwrap(),
        8,
    );
    let capacity = u32::from_le_bytes(
        bytes[window::CAPACITY..window::CAPACITY + 4]
            .try_into()
            .unwrap(),
    );
    let slot = u32::from_le_bytes(
        bytes[window::CURRENT_SLOT..window::CURRENT_SLOT + 4]
            .try_into()
            .unwrap(),
    );
    let wrapped = bytes[window::WRAPPED];
    let Some(records) = records.filter(|records| {
        capacity == cloop_history::DAILY_WINDOW_CAPACITY
            && records.count == capacity
            && slot < capacity
            && wrapped <= 1
    }) else {
        return Err("logistics daily window is not the constructed 30-record window");
    };
    let record = u64_at(bytes, window::RECORD_FIELDS) as usize;
    if record == 0 {
        return Err("logistics current record null");
    }
    Ok(Current {
        slot,
        slot_entry: records
            .element(slot, 8)
            .ok_or("logistics daily slot address overflow")?,
        record,
    })
}

/// Checks that the window's slot names its current record, that the record
/// belongs to the series and that it holds the series' field count.
fn check_record(
    history: usize,
    current: &Current,
    slot: &[u8; 8],
    record: &[u8; RECORD_BYTES],
    history_bytes: &[u8; HISTORY_BYTES],
) -> Result<Array, &'static str> {
    if u64::from_le_bytes(*slot) != current.record as u64 {
        return Err("logistics current record disagrees with its daily slot");
    }
    if u64_at(record, record_fields::HISTORY) != history as u64 {
        return Err("logistics record owner mismatch");
    }
    let values = Array::from_header(
        record[record_fields::VALUES..record_fields::VALUES + 16]
            .try_into()
            .unwrap(),
        record_fields::VALUE_STRIDE,
    )
    .ok_or("logistics record value array invalid")?;
    let fields = u32::from_le_bytes(
        history_bytes[cloop_history::FIELD_COUNT..cloop_history::FIELD_COUNT + 4]
            .try_into()
            .unwrap(),
    );
    if values.count != fields {
        return Err("logistics record size differs from its series field count");
    }
    Ok(values)
}

fn read_archetype_list(memory: &impl Memory, base: usize) -> Result<Array, &'static str> {
    let database = base
        .checked_add(database::SINGLETON_RVA)
        .and_then(|global| read::<u64>(memory, global))
        .and_then(|pointer| usize::try_from(pointer).ok())
        .filter(|&pointer| pointer != 0)
        .ok_or("equipment database pointer unreadable or null")?;
    let expected = base
        .checked_add(database::VTABLE_RVA)
        .ok_or("equipment database vtable address overflow")?;
    if read::<u64>(memory, database) != Some(expected as u64) {
        return Err("equipment database vtable unreadable or mismatched");
    }
    database
        .checked_add(database::ARCHETYPES)
        .and_then(|address| Array::read(memory, address, database::ARCHETYPE_STRIDE))
        .ok_or("equipment archetype list unreadable or invalid")
}

/// Names each record index with the token of the archetype at that position,
/// requiring it to be an archetype whose own index is that position.
fn read_identities(
    memory: &impl Memory,
    base: usize,
    list: Array,
    block: &mut [Row],
) -> Result<(), &'static str> {
    let expected = base
        .checked_add(cequipment_type::VTABLE_RVA)
        .ok_or("equipment type vtable address overflow")? as u64;
    let mut pointer_bytes = [0; BATCH * 8];
    let mut pointers = [0_usize; BATCH];
    let mut identities = [0; BATCH * IDENTITY_STRIDE];
    for (first, rows) in (0..).step_by(BATCH).zip(block.chunks_mut(BATCH)) {
        let count = rows.len();
        let span = list
            .slice(first, count as u32, database::ARCHETYPE_STRIDE)
            .map_err(|_| "equipment archetype list window invalid")?;
        if !read_slice(memory, span.data, &mut pointer_bytes[..count * 8]) {
            return Err("equipment archetype list unreadable");
        }
        for (pointer, bytes) in pointers.iter_mut().zip(pointer_bytes.chunks_exact(8)) {
            *pointer = u64::from_le_bytes(bytes.try_into().unwrap()) as usize;
        }
        let copied = memory.copy_records(
            RecordSources::Pointers(&pointers[..count]),
            &IDENTITY_LAYOUT,
            &mut identities[..count * IDENTITY_STRIDE],
        );
        for (index, row) in rows.iter_mut().enumerate() {
            if copied < (index + 1) * IDENTITY_FIELDS.len() {
                return Err("equipment archetype unreadable");
            }
            let bytes = &identities[index * IDENTITY_STRIDE..(index + 1) * IDENTITY_STRIDE];
            if u64_at(bytes, 0) != expected {
                return Err("equipment archetype vtable mismatch");
            }
            if bytes[16] != 1 {
                return Err("equipment archetype list holds a type that is not an archetype");
            }
            if i32_at(bytes, 12) != first as i32 + index as i32 {
                return Err("equipment archetype index differs from its list position");
            }
            row.token = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        }
    }
    Ok(())
}

/// Copies both records' values, a batch of each per guarded call.
fn read_values(
    memory: &impl Memory,
    stock: Array,
    balance: Array,
    block: &mut [Row],
) -> Result<(), &'static str> {
    let mut stock_bytes = [0; BATCH * 8];
    let mut balance_bytes = [0; BATCH * 8];
    for (first, rows) in (0..).step_by(BATCH).zip(block.chunks_mut(BATCH)) {
        let count = rows.len();
        let address = |values: Array| {
            values
                .slice(first, count as u32, record_fields::VALUE_STRIDE)
                .map(|window| window.data)
                .map_err(|_| "logistics record value window invalid")
        };
        if memory.copy_scattered(
            [address(stock)?, address(balance)?],
            &mut [
                &mut stock_bytes[..count * 8],
                &mut balance_bytes[..count * 8],
            ],
        ) != 2
        {
            return Err("logistics record values unreadable");
        }
        for (index, row) in rows.iter_mut().enumerate() {
            row.stock_raw = i64_at(&stock_bytes, index * 8);
            row.balance_raw = i64_at(&balance_bytes, index * 8);
        }
    }
    Ok(())
}

fn pointer_at(memory: &impl Memory, object: usize, offset: usize) -> Option<usize> {
    object
        .checked_add(offset)
        .and_then(|address| read::<u64>(memory, address))
        .and_then(|pointer| usize::try_from(pointer).ok())
}

fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn i64_at(bytes: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn i32_at(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

#[cfg(test)]
mod tests;
