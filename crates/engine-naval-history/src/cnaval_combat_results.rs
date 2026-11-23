
use engine_sites::Rva;
use engine_native_memory::{Memory, cpdx_array::Array, read, walk::{Buffer, Outcome}};
use engine_native::cgame_state;
use crate::{cnaval_combat_result_side as side, cnaval_combat_ship_entry as member};

pub const SIZE: usize = 0x168;
pub const VTABLE_RVA: Rva = Rva(0x029d_a6c0);
pub const HANDLE: usize = 0x08;
pub const FIRST_SIDE: usize = 0x18;
pub const SECOND_SIDE: usize = 0x88;
pub const WINNING_SIDE: usize = 0xf8;
pub const LOSING_SIDE: usize = 0x100;
pub const PROVINCE: usize = 0x108;
pub const RESULT_HOUR: usize = 0x118;
pub const FLAG_A: usize = 0x128;
pub const FLAG_B: usize = 0x129;
pub const SUBTYPE: usize = 0x12a;
pub const READINESS: usize = 0x12b;
pub const RETIRE_HOUR: usize = 0x138;
pub const CATEGORY: usize = 0x148;
pub const ADMITTED: usize = 0x150;
pub const CONSTRUCTOR_RVA: Rva = Rva(0x00d4_1460);
pub const MERGE_RVA: Rva = Rva(0x00d4_4ea0);
pub const FILL_FROM_COMBAT_RVA: Rva = Rva(0x00d4_5c60);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    pub province: u32,
    pub result_hour: i32,
    pub retire_hour: i32,
    pub category: u32,
    pub flag_a: u8,
    pub flag_b: u8,
    pub subtype: u8,
    pub readiness: u8,
    pub winning_side: u8,
    pub losing_side: u8,
    pub province_ordinal: u32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ResultSlot {
    pointer: usize,
    row: u32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MemberRow {
    pub result: u64,
    pub member: u64,
    pub presence_value: u64,
    pub ordinal: u32,
    pub side: u8,
}

impl MemberRow {
    pub fn key(self) -> u128 {
        (u128::from(self.result) << 64) | u128::from(self.member)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CountryRow {
    pub result: u64,
    pub country: u32,
    pub ordinal: u32,
    pub list: u8,
}

impl CountryRow {
    pub fn key(self) -> u128 {
        (u128::from(self.result) << 64) | (u128::from(self.list) << 32) | u128::from(self.country)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub records: Outcome,
    pub members: Outcome,
    pub countries: Outcome,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Source {
    pub pointer: usize,
    pub id: u64,
}

pub fn visit_sources(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut visit: impl FnMut(Source) -> Result<(), &'static str>,
) -> Result<usize, &'static str> {
    if game_state == 0 {
        return Err("game state is null");
    }
    let list = array(memory, game_state, cgame_state::NAVAL_COMBAT_RESULTS, 8)?;
    for ordinal in 0..list.count {
        let pointer = element::<u64>(memory, list, ordinal, 8)? as usize;
        if pointer == 0 {
            return Err("naval result list contains a null pointer");
        }
        let id = identity(memory, base, pointer)?;
        visit(Source { pointer, id })?;
    }
    Ok(list.count as usize)
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    records: &mut Vec<Row>,
    slots: &mut Vec<ResultSlot>,
    members: &mut Vec<MemberRow>,
    countries: &mut Vec<CountryRow>,
) -> Walk {
    walk_buffer(
        memory,
        base,
        game_state,
        Buffer::Growing(records),
        Buffer::Growing(slots),
        Buffer::Growing(members),
        Buffer::Growing(countries),
    )
}

fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    records: Buffer<'_, Row>,
    mut slots: Buffer<'_, ResultSlot>,
    members: Buffer<'_, MemberRow>,
    countries: Buffer<'_, CountryRow>,
) -> Walk {
    let mut records = Sink::new(records);
    let mut members = Sink::new(members);
    let mut countries = Sink::new(countries);
    let result = (|| {
        if game_state == 0 {
            return Err("game state is null");
        }
        let list = array(memory, game_state, cgame_state::NAVAL_COMBAT_RESULTS, 8)?;
        records.reserve(list.count as usize)?;
        slots.ensure(list.count as usize)?;
        for ordinal in 0..list.count {
            let pointer = element::<u64>(memory, list, ordinal, 8)? as usize;
            if pointer == 0 {
                return Err("naval result list contains a null pointer");
            }
            let id = identity(memory, base, pointer)?;
            if records.wants_row() {
                match record(memory, pointer, id) {
                    Ok(row) => match slots.get_mut(records.len) {
                        Some(slot) => {
                            *slot = ResultSlot {
                                pointer,
                                row: records.len as u32,
                            };
                            records.push(row);
                        }
                        None => records.fail("naval result slot scratch shorter than its rows"),
                    },
                    Err(reason) => records.fail(reason),
                }
            } else if records.error.is_none() {
                records.omit(1);
            }
            for (side_index, offset) in [(0, FIRST_SIDE), (1, SECOND_SIDE)] {
                let side_pointer = pointer
                    .checked_add(offset)
                    .ok_or("naval result side address overflow")?;
                if let Err(reason) = side::authenticate(memory, base, side_pointer) {
                    members.fail(reason);
                    countries.fail(reason);
                    continue;
                }
                if members.error.is_none() {
                    let result =
                        read_members(memory, base, side_pointer, id, side_index, &mut members);
                    if let Err(reason) = result {
                        members.fail(reason);
                    }
                }
                if countries.error.is_none() {
                    let result = read_countries(
                        memory,
                        side_pointer,
                        side::COUNTRIES,
                        id,
                        side_index,
                        &mut countries,
                    );
                    if let Err(reason) = result {
                        countries.fail(reason);
                    }
                }
            }
            if countries.error.is_none() {
                let result = read_countries(memory, pointer, ADMITTED, id, 2, &mut countries);
                if let Err(reason) = result {
                    countries.fail(reason);
                }
            }
        }
        if records.error.is_none()
            && records.len > 0
            && let Err(reason) = province_ordinals(
                memory,
                game_state,
                &mut records.rows[..records.len],
                &mut slots[..records.len],
            )
        {
            records.fail(reason);
        }
        Ok(())
    })();
    if let Err(reason) = result {
        records.fail(reason);
        members.fail(reason);
        countries.fail(reason);
    }
    Walk {
        records: records.outcome(),
        members: members.outcome(),
        countries: countries.outcome(),
    }
}

fn identity(memory: &impl Memory, base: usize, pointer: usize) -> Result<u64, &'static str> {
    let header = read::<[u8; 16]>(memory, pointer).ok_or("naval result identity unreadable")?;
    let vtable = u64::from_le_bytes(header[..8].try_into().unwrap());
    if Some(vtable) != engine_sites::at(base, VTABLE_RVA).map(|p| p as u64) {
        return Err("naval result vtable mismatch");
    }
    let id = u64::from_le_bytes(header[8..].try_into().unwrap());
    if id == 0 {
        return Err("naval result handle is null");
    }
    Ok(id)
}

fn record(memory: &impl Memory, pointer: usize, id: u64) -> Result<Row, &'static str> {
    let tail = pointer
        .checked_add(WINNING_SIDE)
        .and_then(|p| read::<[u8; 84]>(memory, p))
        .ok_or("naval result fields unreadable")?;
    let dword = |offset| u32::from_le_bytes(tail[offset..offset + 4].try_into().unwrap());
    let qword = |offset| u64::from_le_bytes(tail[offset..offset + 8].try_into().unwrap());
    let selector = |value: u64| -> Result<u8, &'static str> {
        if value == 0 {
            Ok(0)
        } else if pointer.checked_add(FIRST_SIDE).map(|p| p as u64) == Some(value) {
            Ok(1)
        } else if pointer.checked_add(SECOND_SIDE).map(|p| p as u64) == Some(value) {
            Ok(2)
        } else {
            Err("naval result winner or loser does not name an embedded side")
        }
    };
    Ok(Row {
        id,
        province: dword(PROVINCE - WINNING_SIDE),
        result_hour: dword(RESULT_HOUR - WINNING_SIDE) as i32,
        retire_hour: dword(RETIRE_HOUR - WINNING_SIDE) as i32,
        category: dword(CATEGORY - WINNING_SIDE),
        flag_a: tail[FLAG_A - WINNING_SIDE],
        flag_b: tail[FLAG_B - WINNING_SIDE],
        subtype: tail[SUBTYPE - WINNING_SIDE],
        readiness: tail[READINESS - WINNING_SIDE],
        winning_side: selector(qword(0))?,
        losing_side: selector(qword(LOSING_SIDE - WINNING_SIDE))?,
        province_ordinal: 0,
    })
}

fn province_ordinals(
    memory: &impl Memory,
    game_state: usize,
    rows: &mut [Row],
    slots: &mut [ResultSlot],
) -> Result<(), &'static str> {
    const UNASSIGNED: u32 = u32::MAX;
    const CHUNK: u32 = 64;
    let field = |address: usize| {
        read::<u64>(memory, address)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or("naval result province map unreadable")
    };
    let at = |offset: usize| {
        game_state
            .checked_add(offset)
            .ok_or("naval result province map overflow")
    };
    slots.sort_unstable_by_key(|slot| slot.pointer);
    for row in rows.iter_mut() {
        row.province_ordinal = UNASSIGNED;
    }
    let head = field(at(cgame_state::NAVAL_RESULTS_MAP_LIST_HEAD)?)?;
    let size = field(at(cgame_state::NAVAL_RESULTS_MAP_LIST_SIZE)?)?;
    let mut node = field(head)?;
    let mut assigned = 0usize;
    let mut pointers = [0u8; CHUNK as usize * 8];
    for _ in 0..size {
        if node == head {
            return Err("naval result province list shorter than its size");
        }
        let bytes =
            read::<[u8; 0x28]>(memory, node).ok_or("naval result province node unreadable")?;
        let next = u64::from_le_bytes(bytes[..8].try_into().expect("eight bytes")) as usize;
        let province = u32::from_le_bytes(bytes[0x10..0x14].try_into().expect("four bytes"));
        let entries = Array::from_header(bytes[0x18..0x28].try_into().expect("header"), 8)
            .ok_or("naval result province vector invalid")?;
        let mut first = 0;
        while first < entries.count {
            let n = (entries.count - first).min(CHUNK) as usize;
            let address = entries
                .element(first, 8)
                .ok_or("naval result province vector unreadable")?;
            if !engine_native_memory::read_slice(memory, address, &mut pointers[..n * 8]) {
                return Err("naval result province vector unreadable");
            }
            for (offset, word) in pointers[..n * 8].chunks_exact(8).enumerate() {
                let pointer = u64::from_le_bytes(word.try_into().expect("eight bytes")) as usize;
                let Ok(found) = slots.binary_search_by_key(&pointer, |slot| slot.pointer) else {
                    continue;
                };
                let row = &mut rows[slots[found].row as usize];
                if row.province != province {
                    return Err("naval result filed under another province");
                }
                if row.province_ordinal != UNASSIGNED {
                    return Err("naval result repeated in its province bucket");
                }
                row.province_ordinal = first + offset as u32;
                assigned += 1;
            }
            first += n as u32;
        }
        node = next;
    }
    if node != head {
        return Err("naval result province list longer than its size");
    }
    if assigned != rows.len() {
        return Err("naval result absent from its province bucket");
    }
    Ok(())
}

fn read_members(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    result: u64,
    side: u8,
    sink: &mut Sink<'_, MemberRow>,
) -> Result<(), &'static str> {
    let entries = array(memory, pointer, side::MEMBERS, 8)?;
    sink.reserve(entries.count as usize)?;
    for ordinal in 0..entries.count {
        if !sink.wants_row() {
            sink.omit(entries.count - ordinal);
            break;
        }
        let pointer = element::<u64>(memory, entries, ordinal, 8)? as usize;
        if pointer == 0 {
            return Err("naval result member pointer is null");
        }
        let (member, presence_value) = member::values(memory, base, pointer)?;
        sink.push(MemberRow {
            result,
            member,
            presence_value,
            ordinal,
            side,
        });
    }
    Ok(())
}

fn read_countries(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
    result: u64,
    list: u8,
    sink: &mut Sink<'_, CountryRow>,
) -> Result<(), &'static str> {
    let entries = array(memory, pointer, offset, 4)?;
    sink.reserve(entries.count as usize)?;
    for ordinal in 0..entries.count {
        if !sink.wants_row() {
            sink.omit(entries.count - ordinal);
            break;
        }
        let country = element::<u32>(memory, entries, ordinal, 4)?;
        if list == 2 && country == 0 {
            return Err("naval result admitted country is zero");
        }
        sink.push(CountryRow {
            result,
            country,
            ordinal,
            list,
        });
    }
    Ok(())
}

fn array(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
    stride: usize,
) -> Result<Array, &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|p| Array::read(memory, p, stride))
        .ok_or("naval result array unreadable or invalid")
}

fn element<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    array: Array,
    index: u32,
    stride: usize,
) -> Result<T, &'static str> {
    array
        .element(index, stride)
        .and_then(|p| read(memory, p))
        .ok_or("naval result array element unreadable")
}

struct Sink<'a, R> {
    rows: Buffer<'a, R>,
    len: usize,
    unread: u32,
    error: Option<&'static str>,
}

impl<'a, R: Default> Sink<'a, R> {
    fn new(rows: Buffer<'a, R>) -> Self {
        Self {
            rows,
            len: 0,
            unread: 0,
            error: None,
        }
    }

    fn reserve(&mut self, count: usize) -> Result<(), &'static str> {
        let required = self
            .len
            .checked_add(count)
            .ok_or("naval result row count overflow")?;
        self.rows.ensure(required)?;
        Ok(())
    }

    fn wants_row(&self) -> bool {
        self.error.is_none() && self.len < self.rows.len()
    }

    fn omit(&mut self, count: u32) {
        match self.unread.checked_add(count) {
            Some(count) => self.unread = count,
            None => self.fail("naval result omitted row count overflow"),
        }
    }

    fn push(&mut self, row: R) {
        self.rows[self.len] = row;
        self.len += 1;
    }

    fn fail(&mut self, reason: &'static str) {
        self.error.get_or_insert(reason);
    }

    fn outcome(self) -> Outcome {
        if let Some(reason) = self.error {
            Outcome::Unavailable { reason }
        } else if self.unread != 0 {
            Outcome::Truncated {
                len: self.len,
                unread_rows: self.unread,
            }
        } else {
            Outcome::Rows { len: self.len }
        }
    }
}

#[cfg(test)]
mod tests;
