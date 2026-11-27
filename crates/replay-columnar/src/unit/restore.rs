
use super::{FrameEntry, get_status, get_varint, get_wide, put_status, put_varint, put_wide, unzig_wide, zig, zig_wide};
use crate::{
    invalid,
    manifest::Status,
    state::Kind,
    store::{Last, Strings, TableState, Value},
};
use std::io;

pub const CHUNK: usize = 1 << 20;

pub const ROWS_CHUNK: u32 = 1 << 16;

pub const ROWS: u32 = 0;
pub const META: u32 = 1;

pub const fn column_stream(column: usize) -> u32 {
    column as u32 + 2
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub first: u32,
    pub frame: FrameEntry,
    pub prefix: Option<FrameEntry>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RestoreEntry {
    pub whole: bool,
    pub streams: Vec<(u32, Vec<Chunk>)>,
    pub log: bool,
}

fn cell_code(value: Value<'_>) -> u128 {
    match value {
        Value::Unrecorded => 3,
        Value::Missing => 1,
        Value::Number(bits) => u128::from(zig(bits as i64)) << 1,
        Value::Text(id) => u128::from(zig(i64::from(id))) << 1,
        Value::List(_) => 3,
    }
}

pub(crate) fn column_chunk(table: &TableState, column: usize, from: u32, to: u32, out: &mut Vec<u8>) {
    let Some(held) = table.column(column) else { return };
    let list = matches!(held.kind(), Kind::U32List | Kind::U64List | Kind::Opaque);
    for slot in from..to.min(table.slots()) {
        if !table.live().get(slot) {
            continue;
        }
        let value = held.value(slot);
        if list {
            match value {
                Value::List(bytes) => {
                    put_varint(out, (bytes.len() as u64) << 1);
                    out.extend_from_slice(bytes);
                }
                _ => {
                    let held = match held.held(slot) {
                        Value::List(bytes) => bytes,
                        _ => &[],
                    };
                    put_varint(out, (held.len() as u64) << 1 | 1);
                    out.extend_from_slice(held);
                }
            }
        } else {
            put_wide(out, cell_code(value));
        }
    }
}

pub(crate) fn rows_chunk(table: &TableState, from: u32, to: u32, out: &mut Vec<u8>) {
    let to = to.min(table.slots());
    live_bits(table, from, to, out);
    let mut previous = 0_u128;
    for slot in from..to {
        if let Some(key) = table.key(slot) {
            put_wide(out, zig_wide(key.wrapping_sub(previous) as i128));
            previous = key;
        }
    }
}

pub(crate) fn listed_chunk(table: &TableState, out: &mut Vec<u8>) {
    let (Some(count), Some(listed)) = (table.count(), table.listed()) else { return };
    let lowest = listed.lowest().unwrap_or(count);
    put_varint(out, lowest);
    let mut bits = 0_u8;
    for ordinal in lowest..count {
        if listed.get(ordinal) {
            bits |= 1 << ((ordinal - lowest) % 8);
        }
        if (ordinal - lowest) % 8 == 7 || ordinal + 1 == count {
            out.push(bits);
            bits = 0;
        }
    }
}

pub(crate) fn read_listed(bytes: &[u8], count: u64) -> io::Result<crate::store::Listed> {
    let mut at = 0;
    let lowest = Some(get_varint(bytes, &mut at)?).filter(|&lowest| lowest <= count).ok_or_else(|| invalid("A log's live set starts past its count"))?;
    let bits = bytes.get(at..).filter(|bits| bits.len() as u64 == (count - lowest).div_ceil(8)).ok_or_else(|| invalid("A log's live set holds another length than its count"))?;
    let mut listed = crate::store::Listed::default();
    for (index, &byte) in bits.iter().enumerate() {
        let mut rest = byte;
        while rest != 0 {
            let ordinal = lowest + index as u64 * 8 + u64::from(rest.trailing_zeros());
            rest &= rest - 1;
            if ordinal >= count {
                return Err(invalid("A log's live set lists a row past its count"));
            }
            listed.append(ordinal);
        }
    }
    Ok(listed)
}

fn live_bits(table: &TableState, from: u32, to: u32, out: &mut Vec<u8>) {
    let mut bits = 0_u8;
    for slot in from..to {
        if table.live().get(slot) {
            bits |= 1 << ((slot - from) % 8);
        }
        if (slot - from) % 8 == 7 || slot + 1 == to {
            out.push(bits);
            bits = 0;
        }
    }
}

struct WholeRows<'a> {
    bytes: &'a [u8],
    count: u32,
    at: usize,
    key: u128,
}

impl<'a> WholeRows<'a> {
    fn new(bytes: &'a [u8], count: u32) -> io::Result<Self> {
        let at = count.div_ceil(8) as usize;
        if bytes.len() < at {
            return Err(invalid("A whole restore's rows end inside their bits"));
        }
        Ok(Self { bytes, count, at, key: 0 })
    }

    fn at(&mut self, offset: u32) -> io::Result<Option<u128>> {
        if offset >= self.count || self.bytes[(offset / 8) as usize] >> (offset % 8) & 1 == 0 {
            return Ok(None);
        }
        self.key = self.key.wrapping_add(unzig_wide(get_wide(self.bytes, &mut self.at)?) as u128);
        Ok(Some(self.key))
    }

    fn done(&self) -> io::Result<()> {
        if self.at != self.bytes.len() {
            return Err(invalid("A whole restore's rows hold bytes past their keys"));
        }
        Ok(())
    }
}

pub(crate) fn changed_rows_chunk(table: &TableState, from: u32, to: u32, whole: &[u8], whole_to: u32, out: &mut Vec<u8>) -> io::Result<()> {
    let to = to.min(table.slots()).max(from);
    live_bits(table, from, to, out);
    let mut held = WholeRows::new(whole, whole_to.saturating_sub(from))?;
    let (mut last, mut previous) = (from, 0_u128);
    for slot in from..to.max(whole_to) {
        let was = held.at(slot - from)?;
        if let Some(key) = table.key(slot).filter(|_| slot < to)
            && was != Some(key)
        {
            put_varint(out, u64::from(slot - last));
            put_wide(out, zig_wide(key.wrapping_sub(previous) as i128));
            (last, previous) = (slot, key);
        }
    }
    held.done()
}

fn put_sample(out: &mut Vec<u8>, sampled: Option<i32>) {
    put_varint(out, sampled.map_or(0, |hour| u64::from(((hour << 1) ^ (hour >> 31)) as u32) + 1));
}

fn get_sample(bytes: &[u8], at: &mut usize) -> io::Result<Option<i32>> {
    Ok(match get_varint(bytes, at)? {
        0 => None,
        value => {
            let value = (value - 1) as u32;
            Some(((value >> 1) as i32) ^ -((value & 1) as i32))
        }
    })
}

fn whole_table(table: &TableState, out: &mut Vec<u8>) {
    put_varint(out, u64::from(table.slots()));
    let mut rows = Vec::new();
    rows_chunk(table, 0, table.slots(), &mut rows);
    put_varint(out, rows.len() as u64);
    out.extend_from_slice(&rows);
    for column in 0..table.columns().len() {
        let mut cells = Vec::new();
        column_chunk(table, column, 0, table.slots(), &mut cells);
        put_varint(out, cells.len() as u64);
        out.extend_from_slice(&cells);
    }
}

pub(crate) fn meta(table: &TableState, strings: &Strings, listed: Option<&std::collections::HashSet<u32, foldhash::fast::FixedState>>, out: &mut Vec<u8>) -> io::Result<Vec<u32>> {
    put_varint(out, table.count().unwrap_or(u64::from(table.slots())));
    put_status(out, table.status());
    put_sample(out, table.sampled_at());
    match table.view() {
        None => out.push(0),
        Some(view) => {
            out.push(1);
            put_varint(out, view.len() as u64);
            let mut previous = 0_u32;
            for &slot in view {
                put_varint(out, u64::from(slot - previous));
                previous = slot;
            }
        }
    }
    let mut ids = Vec::new();
    let mut named = |held: &TableState| {
        for column in held.columns() {
            if column.kind() == Kind::Bytes {
                for slot in held.rows() {
                    if let Value::Text(id) = column.value(slot) {
                        ids.push(id);
                    }
                }
            }
        }
    };
    named(table);
    if let Some(Last::Apart(last, _)) = table.last() {
        named(last);
    }
    if let Status::Unavailable { reason } = table.status() {
        ids.push(reason);
    }
    ids.sort_unstable();
    ids.dedup();
    if let Some(listed) = listed {
        ids.retain(|id| !listed.contains(id));
    }
    put_varint(out, ids.len() as u64);
    let mut previous = 0_u32;
    for &id in &ids {
        let text = strings.get(id).ok_or_else(|| invalid(format!("String {id} a restore names is not held")))?;
        put_varint(out, u64::from(id - previous));
        previous = id;
        put_varint(out, text.len() as u64);
        out.extend_from_slice(text);
    }
    match table.last() {
        None => out.push(0),
        Some(Last::Held(read)) => {
            out.push(1);
            put_sample(out, *read);
        }
        Some(Last::Apart(last, read)) => {
            out.push(2);
            put_sample(out, *read);
            whole_table(last, out);
        }
    }
    Ok(ids)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Meta {
    pub slots: u32,
    pub count: u64,
    pub strings: Vec<(u32, Vec<u8>)>,
}

#[derive(Debug, Default)]
pub struct StringTable {
    bytes: Vec<u8>,
    at: Vec<(u32, u32, u32)>,
}

impl StringTable {
    pub fn get(&self, id: u32) -> Option<&[u8]> {
        let (_, from, length) = self.at[self.at.binary_search_by_key(&id, |&(held, ..)| held).ok()?];
        self.bytes.get(from as usize..from as usize + length as usize)
    }

    pub fn iter(&self) -> impl Iterator<Item = (u32, &[u8])> {
        self.at.iter().map(|&(id, from, length)| (id, &self.bytes[from as usize..from as usize + length as usize]))
    }

    pub fn len(&self) -> usize {
        self.at.len()
    }

    pub fn is_empty(&self) -> bool {
        self.at.is_empty()
    }

    pub fn held_bytes(&self) -> usize {
        self.bytes.capacity() + self.at.capacity() * size_of::<(u32, u32, u32)>()
    }
}

fn read_whole_table(bytes: &[u8], at: &mut usize, kinds: &[Kind]) -> io::Result<TableState> {
    let mut table = TableState::new(kinds);
    let slots = get_varint(bytes, at)? as u32;
    let length = get_varint(bytes, at)? as usize;
    table.set_slots(slots);
    let passes = || invalid("A last complete read passes its stream");
    super::reader::bulk::rows(bytes.get(*at..*at + length).ok_or_else(passes)?, 0, slots, &mut table, &mut Vec::new())?;
    *at += length;
    let (mut values, mut cells) = (Vec::new(), Vec::new());
    for (column, &kind) in kinds.iter().enumerate() {
        let length = get_varint(bytes, at)? as usize;
        let held = bytes.get(*at..*at + length).ok_or_else(passes)?;
        if matches!(kind, Kind::U32List | Kind::U64List | Kind::Opaque) {
            super::reader::bulk::lists(held, length, 0, slots, column, &mut table, &mut cells)?;
        } else {
            super::reader::bulk::numbers(held, length, 0, slots, column, &mut table, &mut values)?;
        }
        *at += length;
    }
    Ok(table)
}

pub(crate) fn read_meta(bytes: &[u8], kinds: &[Kind], table: &mut TableState) -> io::Result<Meta> {
    let mut strings = Vec::new();
    let meta = read_meta_with(bytes, kinds, table, &mut |id, text, _| strings.push((id, text.to_vec())))?;
    Ok(Meta { strings, ..meta })
}

pub(crate) fn read_meta_lazily(bytes: Vec<u8>, length: usize, kinds: &[Kind], table: &mut TableState) -> io::Result<(Meta, StringTable)> {
    let mut at = Vec::new();
    let meta = read_meta_with(bytes.get(..length).ok_or_else(|| invalid("A restore's meta passes its chunk"))?, kinds, table, &mut |id, text, from| {
        at.push((id, from as u32, text.len() as u32));
    })?;
    at.sort_by_key(|&(id, ..)| id);
    Ok((meta, StringTable { bytes, at }))
}

fn read_meta_with(bytes: &[u8], kinds: &[Kind], table: &mut TableState, each: &mut dyn FnMut(u32, &[u8], usize)) -> io::Result<Meta> {
    let mut at = 0;
    let count = get_varint(bytes, &mut at)?;
    let slots = if table.is_log() { 0 } else { u32::try_from(count).map_err(|_| invalid("A restore's slots pass 32 bits"))? };
    let status = get_status(bytes, &mut at)?;
    let sampled = get_sample(bytes, &mut at)?;
    let view = match *bytes.get(at).ok_or_else(|| invalid("A restore's meta ends early"))? {
        0 => {
            at += 1;
            None
        }
        _ => {
            at += 1;
            let count = get_varint(bytes, &mut at)? as usize;
            let mut view = Vec::with_capacity(count.min(bytes.len()));
            let mut slot = 0_u32;
            for _ in 0..count {
                slot += get_varint(bytes, &mut at)? as u32;
                view.push(slot);
            }
            Some(view)
        }
    };
    let mut id = 0_u32;
    for _ in 0..get_varint(bytes, &mut at)? {
        id += get_varint(bytes, &mut at)? as u32;
        let length = get_varint(bytes, &mut at)? as usize;
        each(id, bytes.get(at..at + length).ok_or_else(|| invalid("A restore's string passes its stream"))?, at);
        at += length;
    }
    let last = match *bytes.get(at).ok_or_else(|| invalid("A restore's meta ends early"))? {
        0 => {
            at += 1;
            None
        }
        1 => {
            at += 1;
            Some(Last::Held(get_sample(bytes, &mut at)?))
        }
        _ => {
            at += 1;
            let read = get_sample(bytes, &mut at)?;
            Some(Last::Apart(Box::new(read_whole_table(bytes, &mut at, kinds)?), read))
        }
    };
    if at != bytes.len() {
        return Err(invalid("A restore's meta holds bytes past its end"));
    }
    table.set_meta(status, sampled, view, last);
    Ok(Meta { slots, count, strings: Vec::new() })
}
