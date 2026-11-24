
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources},
    read,
    read_slice,
    walk::Outcome,
};
use engine_native::{ccountry, cgame_state};
use engine_native_parts::{cequipment_database as database, cequipment_type};
use crate::{
    cloop_history,
    loop_history_record_fields as record_fields,
    loop_history_window as window,
};
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x02a2_ec48);
pub const SERIES: usize = 0x08;
pub const SERIES_COUNT: u32 = 19;
pub const COUNTRY: usize = 0x20;
pub const STOCK_SERIES: u32 = 15;
#[cfg(test)]
pub const BALANCE_SERIES: u32 = 16;
pub const STOCK_ROLL_UP: i32 = 0;
pub const BALANCE_ROLL_UP: i32 = 1;

const STATUS_BYTES: usize = COUNTRY + 8;
const HISTORY_BYTES: usize = cloop_history::FIELD_COUNT + 4;
const WINDOW_BYTES: usize = window::HISTORY + 8;
const RECORD_BYTES: usize = record_fields::HISTORY + 8;
const BATCH: usize = 64;

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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub archetype: u32,
    pub token: u32,
    pub balance_raw: i64,
    pub stock_raw: i64,
    pub balance_history: usize,
    pub balance_values: usize,
    pub available_raw: Option<i64>,
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    rows: &mut Vec<Row>,
) -> Outcome {
    walk_buffer(memory, base, game_state, engine_native_memory::walk::Buffer::Growing(rows))
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    rows: engine_native_memory::walk::Buffer<'_, Row>,
) -> Outcome {
    match walk_inner(memory, base, game_state, rows) {
        Ok((len, 0)) => Outcome::Rows { len },
        Ok((len, unread_rows)) => Outcome::Truncated { len, unread_rows },
        Err(reason) => Outcome::Unavailable { reason },
    }
}

pub fn row_demand(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<usize, &'static str> {
    let slots = ccountry::Reader::new(memory, base, game_state)?
        .count()
        .saturating_sub(1) as usize;
    slots
        .checked_mul(read_archetype_list(memory, base)?.count as usize)
        .ok_or("logistics row demand overflow")
}

struct Vtables {
    status: u64,
    history: u64,
    window: u64,
}

struct Archetypes {
    list: Array,
    named: Option<usize>,
}

fn walk_inner(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut rows: engine_native_memory::walk::Buffer<'_, Row>,
) -> Result<(usize, u32), &'static str> {
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    let mut control = cgame_state::AiControl::read(memory, game_state)?;
    let vtable = |key: Rva| {
        engine_sites::at(base, key)
            .map(|address| address as u64)
            .ok_or("logistics module address overflow")
    };
    let vtables = Vtables {
        status: vtable(VTABLE_RVA)?,
        history: vtable(cloop_history::VTABLE_RVA)?,
        window: vtable(window::VTABLE_RVA)?,
    };
    let mut archetypes: Option<Archetypes> = None;
    let mut len = 0_usize;
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
        let records = read_records(memory, &vtables, country)?;
        let [stock, balance] = records.values;
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
        rows.ensure(
            len.checked_add(count)
                .ok_or("logistics row count overflow")?,
        )?;
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
            row.balance_history = records.balance_history;
            row.balance_values = balance.data;
            row.available_raw = None;
        }
        len += count;
    }
    if !found {
        return Err("logistics country table has no countries");
    }
    Ok((len, unread_rows))
}

struct Records {
    values: [Array; 2],
    balance_history: usize,
}

fn read_records(
    memory: &impl Memory,
    vtables: &Vtables,
    country: engine_native_memory::country_identity::Country,
) -> Result<Records, &'static str> {
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
    Ok(Records {
        values: [
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
        ],
        balance_history: histories[1],
    })
}

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
    let database = engine_sites::at(base, database::SINGLETON_RVA)
        .and_then(|global| read::<u64>(memory, global))
        .and_then(|pointer| usize::try_from(pointer).ok())
        .filter(|&pointer| pointer != 0)
        .ok_or("equipment database pointer unreadable or null")?;
    let expected = engine_sites::at(base, database::VTABLE_RVA)
        .ok_or("equipment database vtable address overflow")?;
    if read::<u64>(memory, database) != Some(expected as u64) {
        return Err("equipment database vtable unreadable or mismatched");
    }
    database
        .checked_add(database::ARCHETYPES)
        .and_then(|address| Array::read(memory, address, database::ARCHETYPE_STRIDE))
        .ok_or("equipment archetype list unreadable or invalid")
}

fn read_identities(
    memory: &impl Memory,
    base: usize,
    list: Array,
    block: &mut [Row],
) -> Result<(), &'static str> {
    let expected = engine_sites::at(base, cequipment_type::VTABLE_RVA)
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BalanceOwner {
    pub country: u32,
    pub history: usize,
    pub fields: u32,
}

pub fn owner_capacity(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<usize, &'static str> {
    Ok(ccountry::Reader::new(memory, base, game_state)?.count() as usize)
}

pub fn walk_balance_owners(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    owners: &mut [BalanceOwner],
) -> Outcome {
    let mut walk = || -> Result<(usize, u32), &'static str> {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let mut control = cgame_state::AiControl::read(memory, game_state)?;
        let address = |key| {
            engine_sites::at(base, key)
                .map(|a| a as u64)
                .ok_or("logistics module address overflow")
        };
        let vtables = Vtables {
            status: address(VTABLE_RVA)?,
            history: address(cloop_history::VTABLE_RVA)?,
            window: address(window::VTABLE_RVA)?,
        };
        let mut len = 0_usize;
        let mut unread_rows = 0_u32;
        let mut archetypes = None;
        let mut found = false;
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            found = true;
            if control.autonomous(memory, base, &countries, country.id, country.canonical_slot)? {
                continue;
            }
            let records = read_records(memory, &vtables, country)?;
            let archetypes = match archetypes {
                Some(count) => count,
                None => {
                    let count = read_archetype_list(memory, base)?.count;
                    archetypes = Some(count);
                    count
                }
            };
            if records.values.iter().any(|array| array.count != archetypes) {
                return Err("logistics record size differs from the equipment archetype count");
            }
            if let Some(owner) = owners.get_mut(len) {
                *owner = BalanceOwner {
                    country: country.id,
                    history: records.balance_history,
                    fields: archetypes,
                };
                len += 1;
            } else {
                unread_rows = unread_rows
                    .checked_add(1)
                    .ok_or("logistics owner count exceeds u32")?;
            }
        }
        if !found {
            return Err("logistics country table has no countries");
        }
        Ok((len, unread_rows))
    };
    match walk() {
        Ok((len, 0)) => Outcome::Rows { len },
        Ok((len, unread_rows)) => Outcome::Truncated { len, unread_rows },
        Err(reason) => Outcome::Unavailable { reason },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BalanceWrite {
    pub history: usize,
    pub values: usize,
    pub archetype: u32,
    pub balance_raw: i64,
    pub available_raw: i64,
}

pub const BALANCE_WRITE_RVA: Rva = Rva(0x015b_3220);
pub const BALANCE_CALLS: [(Rva, usize); 2] = [(Rva(0x0140_bc04), 0x90), (Rva(0x0140_c402), 0x80)];

pub fn balance_write_stores(memory: &impl Memory, record: usize, archetype: u32) -> Option<bool> {
    let count = read::<i32>(memory, record.checked_add(0xc)?)?;
    Some((archetype as i32) < count)
}

pub fn balance_identity(
    memory: &impl Memory,
    record: usize,
    archetype: u32,
) -> Result<(usize, usize), &'static str> {
    let mut bytes = [0; RECORD_BYTES];
    if !read_slice(memory, record, &mut bytes) {
        return Err("logistics balance write record unreadable");
    }
    let values = Array::from_header(bytes[..16].try_into().unwrap(), 8)
        .ok_or("logistics balance write values invalid")?;
    if archetype > i32::MAX as u32 || values.element(archetype, 8).is_none() {
        return Err("logistics balance write index outside record");
    }
    let history = usize::try_from(u64_at(&bytes, record_fields::HISTORY))
        .map_err(|_| "logistics history pointer exceeds usize")?;
    Ok((history, values.data))
}

pub fn balance_write(
    memory: &impl Memory,
    record: usize,
    balance: i64,
    archetype: u32,
    stack: usize,
    available_offset: usize,
) -> Result<BalanceWrite, &'static str> {
    let (history, values) = balance_identity(memory, record, archetype)?;
    let actual = values
        .checked_add(archetype as usize * 8)
        .and_then(|address| read::<i64>(memory, address));
    if actual != Some(balance) {
        return Err("logistics balance write did not store its operand");
    }
    let available_raw = stack
        .checked_add(available_offset)
        .and_then(|address| read::<i64>(memory, address))
        .ok_or("logistics available operand unreadable")?;
    if available_raw < 0 {
        return Err("logistics native available operand is negative");
    }
    Ok(BalanceWrite {
        history,
        values,
        archetype,
        balance_raw: balance,
        available_raw,
    })
}

pub fn balance_write_with_signal(
    memory: &impl Memory,
    base: usize,
    record: usize,
    balance: i64,
    archetype: u32,
    stack_operand: (usize, usize),
    expected_values: Option<usize>,
) -> (Option<i64>, Result<BalanceWrite, &'static str>) {
    let (stack, available_offset) = stack_operand;
    let signal_addr = engine_sites::at(base, cgame_state::PLAY_SESSION_STARTED_AT_RVA);
    let avail_addr = stack.checked_add(available_offset);
    let actual_addr = expected_values.and_then(|v| v.checked_add(archetype as usize * 8));
    if let (Some(sig_at), Some(av_at), Some(act_at)) = (signal_addr, avail_addr, actual_addr) {
        let mut sig_bytes = [0_u8; 8];
        let mut rec_bytes = [0_u8; RECORD_BYTES];
        let mut av_bytes = [0_u8; 8];
        let mut act_bytes = [0_u8; 8];
        let copied = memory.copy_scattered(
            [sig_at, record, av_at, act_at],
            &mut [
                &mut sig_bytes[..],
                &mut rec_bytes[..],
                &mut av_bytes[..],
                &mut act_bytes[..],
            ],
        );
        if copied == 4 {
            let signal = Some(i64::from_le_bytes(sig_bytes));
            let result = (|| {
                let values = Array::from_header(rec_bytes[..16].try_into().unwrap(), 8)
                    .ok_or("logistics balance write values invalid")?;
                if archetype > i32::MAX as u32 || values.element(archetype, 8).is_none() {
                    return Err("logistics balance write index outside record");
                }
                let history = usize::try_from(u64_at(&rec_bytes, record_fields::HISTORY))
                    .map_err(|_| "logistics history pointer exceeds usize")?;
                let actual = if Some(values.data) == expected_values {
                    Some(i64::from_le_bytes(act_bytes))
                } else {
                    values
                        .data
                        .checked_add(archetype as usize * 8)
                        .and_then(|address| read::<i64>(memory, address))
                };
                if actual != Some(balance) {
                    return Err("logistics balance write did not store its operand");
                }
                let available_raw = i64::from_le_bytes(av_bytes);
                if available_raw < 0 {
                    return Err("logistics native available operand is negative");
                }
                Ok(BalanceWrite {
                    history,
                    values: values.data,
                    archetype,
                    balance_raw: balance,
                    available_raw,
                })
            })();
            return (signal, result);
        }
    }
    let signal = signal_addr.and_then(|at| read::<i64>(memory, at));
    let result = balance_write(memory, record, balance, archetype, stack, available_offset);
    (signal, result)
}

pub const HISTORY_CALLS: [(Rva, Rva); 3] = [
    (Rva(0x0140_ba58), Rva(0x015b_1de0)),
    (Rva(0x0140_7d68), Rva(0x015b_2420)),
    (Rva(0x0140_be38), Rva(0x015b_2420)),
];
