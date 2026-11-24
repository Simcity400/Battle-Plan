
use engine_native_memory::{Memory, cpdx_array::Array};
use crate::{
    cloop_history,
    loop_history_record_fields as record,
    loop_history_window as window,
};
use engine_native_memory::memory::{RecordField, RecordLayout, RecordSources};

pub const CHANNELS: usize = cloop_history::air_stats::FIELD_COUNT as usize;
pub const LEVELS: usize = 3;
const BATCH: usize = 64;
const VALUE_CHUNK: usize = 256;
const STRIDE: usize = CHANNELS * 8;
const ZERO: [i64; CHANNELS] = [0; CHANNELS];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Window {
    pub capacity: u32,
    pub current: u32,
    pub wrapped: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    pub level: u8,
    pub slot: u32,
    pub values: [i64; CHANNELS],
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct History {
    pub roll_up: i32,
    pub windows: [Window; LEVELS],
    pub records: Vec<Record>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Current {
    pub roll_up: i32,
    pub windows: [Window; LEVELS],
    pub values: [i64; CHANNELS],
}

impl History {
    pub fn allocated_bytes(&self) -> usize {
        self.records.capacity() * size_of::<Record>()
    }

    fn level(&self, level: u8) -> &[Record] {
        let first = self.records.partition_point(|record| record.level < level);
        let end = self.records.partition_point(|record| record.level <= level);
        &self.records[first..end]
    }

    fn at(&self, level: u8, slot: u32) -> Option<&[i64; CHANNELS]> {
        self.records
            .binary_search_by_key(&(level, slot), |record| (record.level, record.slot))
            .ok()
            .map(|at| &self.records[at].values)
    }

    pub fn set(
        &mut self,
        level: u8,
        slot: u32,
        values: [i64; CHANNELS],
    ) -> Result<(), &'static str> {
        let found = self
            .records
            .binary_search_by_key(&(level, slot), |record| (record.level, record.slot));
        match (found, values == ZERO) {
            (Ok(at), true) => {
                self.records.remove(at);
            }
            (Ok(at), false) => self.records[at].values = values,
            (Err(_), true) => {}
            (Err(at), false) => {
                crate::allocation::reserve(&mut self.records, 1)
                    .map_err(|_| "air menu history allocation failed")?;
                self.records.insert(
                    at,
                    Record {
                        level,
                        slot,
                        values,
                    },
                );
            }
        }
        Ok(())
    }

    fn step(&mut self, level: usize) -> bool {
        let window = &mut self.windows[level];
        window.current += 1;
        if window.current >= window.capacity {
            window.current = 0;
            window.wrapped = true;
            true
        } else {
            false
        }
    }

    fn aggregate(&self, level: usize, include_current: bool) -> [i64; CHANNELS] {
        let window = self.windows[level];
        let capacity = window.capacity;
        let (count, start) = if window.wrapped {
            (capacity, window.current)
        } else {
            (window.current, 0)
        };
        let records = self.level(level as u8);
        let position = |slot: u32| (slot + capacity - start) % capacity;
        if self.roll_up == 1 {
            return records
                .iter()
                .filter(|record| position(record.slot) < count)
                .filter(|record| record.values.iter().any(|&value| value > 0))
                .max_by_key(|record| position(record.slot))
                .map_or(ZERO, |record| record.values);
        }
        let mut sum = ZERO;
        let mut add = |values: &[i64; CHANNELS]| {
            for (sum, &value) in sum.iter_mut().zip(values) {
                *sum = sum.wrapping_add(value).clamp(
                    -crate::air_kamikaze::SUM_LIMIT,
                    crate::air_kamikaze::SUM_LIMIT,
                );
            }
        };
        let split = records.partition_point(|record| record.slot < start);
        for record in records[split..].iter().chain(&records[..split]) {
            if position(record.slot) < count {
                add(&record.values);
            }
        }
        if include_current
            && !window.wrapped
            && let Some(values) = self.at(level as u8, window.current)
        {
            add(values);
        }
        sum
    }

    pub fn advance(&mut self, count: u64) -> Result<(), &'static str> {
        if self.windows.iter().any(|window| window.capacity == 0) {
            return Err("air menu history window invalid");
        }
        for _ in 0..count {
            let wrapped = self.step(0);
            let monthly = self.aggregate(0, !wrapped);
            self.set(1, self.windows[1].current, monthly)?;
            if wrapped {
                if self.step(1) {
                    let yearly = self.aggregate(1, false);
                    self.set(2, self.windows[2].current, yearly)?;
                    self.step(2);
                }
                self.set(1, self.windows[1].current, ZERO)?;
            } else {
                let yearly = self.aggregate(1, true);
                self.set(2, self.windows[2].current, yearly)?;
            }
            self.set(0, self.windows[0].current, ZERO)?;
        }
        Ok(())
    }

    pub fn apply_current(&mut self, current: &Current) -> Result<bool, &'static str> {
        if current.roll_up != self.roll_up || current.windows != self.windows {
            return Err("air menu history moved without an observed advance");
        }
        let slot = self.windows[0].current;
        if self.at(0, slot).unwrap_or(&ZERO) == &current.values {
            return Ok(false);
        }
        self.set(0, slot, current.values)?;
        Ok(true)
    }
}

pub const REFUSALS: [&str; 7] = [
    "air menu history header unreadable",
    "air menu history identity invalid",
    "air menu history window unreadable",
    "air menu history window invalid",
    "air menu history record pointer unreadable",
    "air menu history record identity invalid",
    "air menu history channels unreadable",
];

#[derive(Default)]
pub struct Scratch {
    bytes: Vec<u8>,
    pointers: Vec<usize>,
    owners: Vec<(usize, usize, u32)>,
    failures: Vec<Option<&'static str>>,
    windows: Vec<(usize, usize, usize)>,
    heads: Vec<Head>,
}

#[derive(Clone, Copy, Default)]
struct Head {
    roll_up: i32,
    windows: [Window; LEVELS],
    refused: Option<&'static str>,
}

impl Scratch {
    pub const fn new() -> Self {
        Self {
            bytes: Vec::new(),
            pointers: Vec::new(),
            owners: Vec::new(),
            failures: Vec::new(),
            windows: Vec::new(),
            heads: Vec::new(),
        }
    }

    pub fn allocated_bytes(&self) -> usize {
        self.bytes.capacity()
            + self.pointers.capacity() * size_of::<usize>()
            + self.owners.capacity() * size_of::<(usize, usize, u32)>()
            + self.failures.capacity() * size_of::<Option<&'static str>>()
            + self.windows.capacity() * size_of::<(usize, usize, usize)>()
            + self.heads.capacity() * size_of::<Head>()
    }

    fn gather(
        &mut self,
        memory: &impl Memory,
        range: std::ops::Range<usize>,
        fields: &[RecordField],
        stride: usize,
        reason: &'static str,
    ) -> Result<(), &'static str> {
        let len = range
            .len()
            .checked_mul(stride)
            .ok_or("air menu history size overflow")?;
        crate::allocation::resize(&mut self.bytes, len, 0)
            .map_err(|_| "air menu history allocation failed")?;
        crate::allocation::resize(&mut self.failures, range.len(), None)
            .map_err(|_| "air menu history allocation failed")?;
        self.failures[..range.len()].fill(None);
        let layout = RecordLayout::new(fields, stride).ok_or("air menu history layout invalid")?;
        let pointers = &self.pointers[range];
        let mut first = 0;
        while first < pointers.len() {
            let complete = memory.copy_records(
                RecordSources::Pointers(&pointers[first..]),
                &layout,
                &mut self.bytes[first * stride..len],
            );
            first += complete / fields.len();
            if first < pointers.len() {
                self.failures[first] = Some(reason);
                first += 1;
            }
        }
        Ok(())
    }

    fn compact(&mut self) {
        let mut live = 0;
        for at in 0..self.pointers.len() {
            if self.heads[self.owners[at].0].refused.is_none() {
                self.pointers[live] = self.pointers[at];
                self.owners[live] = self.owners[at];
                live += 1;
            }
        }
        self.pointers.truncate(live);
        self.owners.truncate(live);
    }

    fn refuse(&mut self, history: usize, reason: &'static str) {
        self.heads[history].refused.get_or_insert(reason);
    }

    fn each_gathered(
        &mut self,
        memory: &impl Memory,
        fields: &[RecordField],
        stride: usize,
        reason: &'static str,
        chunk: usize,
        mut each: impl FnMut(&mut Self, usize, &[u8]) -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        let total = self.pointers.len();
        let mut first = 0;
        while first < total {
            let end = first.saturating_add(chunk).min(total);
            self.gather(memory, first..end, fields, stride, reason)?;
            for at in first..end {
                let history = self.owners[at].0;
                if let Some(reason) = self.failures[at - first] {
                    self.refuse(history, reason);
                    continue;
                }
                let bytes = std::mem::take(&mut self.bytes);
                let result = each(
                    self,
                    at,
                    &bytes[(at - first) * stride..(at - first + 1) * stride],
                );
                self.bytes = bytes;
                result?;
            }
            first = end;
        }
        Ok(())
    }
}

const fn field(source_offset: usize, destination_offset: usize, len: usize) -> RecordField {
    RecordField {
        source_offset,
        destination_offset,
        len,
    }
}
fn word(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}
fn pointer(bytes: &[u8], at: usize) -> Result<usize, &'static str> {
    usize::try_from(u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()))
        .map_err(|_| "air menu history pointer outside address space")
}
fn values(bytes: &[u8]) -> [i64; CHANNELS] {
    std::array::from_fn(|channel| {
        i64::from_le_bytes(bytes[channel * 8..channel * 8 + 8].try_into().unwrap())
    })
}

fn headers(
    memory: &impl Memory,
    base: usize,
    histories: &[usize],
    scratch: &mut Scratch,
    whole: bool,
) -> Result<(), &'static str> {
    let history_vtable = engine_sites::at(base, cloop_history::VTABLE_RVA)
        .ok_or("air menu history address overflow")?;
    let window_vtable =
        engine_sites::at(base, window::VTABLE_RVA).ok_or("air menu history address overflow")?;
    scratch.heads.clear();
    crate::allocation::resize(&mut scratch.heads, histories.len(), Head::default())
        .map_err(|_| "air menu history allocation failed")?;
    scratch.pointers.clear();
    crate::allocation::reserve(&mut scratch.pointers, histories.len() * LEVELS)
        .map_err(|_| "air menu history allocation failed")?;
    scratch.pointers.extend_from_slice(histories);
    scratch.gather(
        memory,
        0..histories.len(),
        &[
            field(0, 0, 8),
            field(cloop_history::DAILY_WINDOW, 8, 24),
            field(cloop_history::SELF_REFERENCE, 32, 8),
            field(cloop_history::ROLL_UP, 40, 8),
        ],
        48,
        REFUSALS[0],
    )?;
    scratch.windows.clear();
    crate::allocation::reserve(&mut scratch.windows, histories.len() * LEVELS)
        .map_err(|_| "air menu history allocation failed")?;
    for at in 0..histories.len() {
        if let Some(reason) = scratch.failures[at] {
            scratch.refuse(at, reason);
            continue;
        }
        let bytes: [u8; 48] = scratch.bytes[at * 48..at * 48 + 48].try_into().unwrap();
        let bytes = &bytes[..];
        let roll_up = word(bytes, 40) as i32;
        if pointer(bytes, 0)? != history_vtable
            || pointer(bytes, 32)? != histories[at]
            || word(bytes, 44) as usize != CHANNELS
            || !matches!(roll_up, 0 | 1)
        {
            scratch.refuse(at, REFUSALS[1]);
            continue;
        }
        scratch.heads[at].roll_up = roll_up;
        for level in 0..LEVELS {
            let window = pointer(bytes, 8 + level * 8)?;
            scratch.windows.push((at, level, window));
        }
    }
    scratch.pointers.clear();
    scratch
        .pointers
        .extend(scratch.windows.iter().map(|entry| entry.2));
    scratch.gather(
        memory,
        0..scratch.pointers.len(),
        &[
            field(0, 0, 8),
            field(window::RECORDS, 8, 16),
            field(window::CAPACITY, 24, 9),
            field(window::HISTORY, 33, 8),
        ],
        41,
        REFUSALS[2],
    )?;
    let windows = std::mem::take(&mut scratch.windows);
    let mut slots = std::mem::take(&mut scratch.pointers);
    slots.clear();
    scratch.owners.clear();
    let result = (|| {
        for (at, &(history, level, _)) in windows.iter().enumerate() {
            if let Some(reason) = scratch.failures[at] {
                scratch.refuse(history, reason);
                continue;
            }
            let bytes: [u8; 41] = scratch.bytes[at * 41..at * 41 + 41].try_into().unwrap();
            let bytes = &bytes[..];
            let capacity = word(bytes, 24) as i32;
            let current = word(bytes, 28);
            let Some(array) =
                Array::from_header(bytes[8..24].try_into().unwrap(), 8).filter(|array| {
                    capacity > 0 && array.count >= capacity as u32 && current < capacity as u32
                })
            else {
                scratch.refuse(history, REFUSALS[3]);
                continue;
            };
            if pointer(bytes, 0)? != window_vtable
                || pointer(bytes, 33)? != histories[history]
                || bytes[32] > 1
            {
                scratch.refuse(history, REFUSALS[3]);
                continue;
            }
            scratch.heads[history].windows[level] = Window {
                capacity: capacity as u32,
                current,
                wrapped: bytes[32] != 0,
            };
            let wanted = if whole {
                0..capacity as u32
            } else if level == 0 {
                current..current + 1
            } else {
                0..0
            };
            crate::allocation::reserve(&mut slots, wanted.len())
                .map_err(|_| "air menu history allocation failed")?;
            crate::allocation::reserve(&mut scratch.owners, wanted.len())
                .map_err(|_| "air menu history allocation failed")?;
            for slot in wanted {
                slots.push(
                    array
                        .element(slot, 8)
                        .ok_or("air menu history slot overflow")?,
                );
                scratch.owners.push((history, level, slot));
            }
        }
        Ok(())
    })();
    scratch.windows = windows;
    scratch.pointers = slots;
    result?;
    scratch.compact();
    Ok(())
}

fn channel_pointers(
    memory: &impl Memory,
    histories: &[usize],
    scratch: &mut Scratch,
) -> Result<(), &'static str> {
    scratch.each_gathered(
        memory,
        &[field(0, 0, 8)],
        8,
        REFUSALS[4],
        usize::MAX,
        |scratch, at, bytes| {
            scratch.pointers[at] = pointer(bytes, 0)?;
            Ok(())
        },
    )?;
    scratch.compact();
    scratch.each_gathered(
        memory,
        &[field(record::VALUES, 0, 16), field(record::HISTORY, 16, 8)],
        24,
        REFUSALS[5],
        usize::MAX,
        |scratch, at, bytes| {
            let history = scratch.owners[at].0;
            let array = Array::from_header(bytes[..16].try_into().unwrap(), 8)
                .filter(|array| array.count as usize == CHANNELS);
            match array {
                Some(array) if pointer(bytes, 16)? == histories[history] => {
                    scratch.pointers[at] = array.data;
                }
                _ => scratch.refuse(history, REFUSALS[5]),
            }
            Ok(())
        },
    )?;
    scratch.compact();
    Ok(())
}

pub fn capture(
    memory: &impl Memory,
    base: usize,
    histories: &[usize],
    scratch: &mut Scratch,
    mut each: impl FnMut(usize, Result<History, &'static str>) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    for (first, batch) in (0..).step_by(BATCH).zip(histories.chunks(BATCH)) {
        headers(memory, base, batch, scratch, true)?;
        channel_pointers(memory, batch, scratch)?;
        let mut copies: [Option<History>; BATCH] = std::array::from_fn(|_| None);
        for (at, head) in scratch.heads.iter().enumerate() {
            if head.refused.is_none() {
                copies[at] = Some(History {
                    roll_up: head.roll_up,
                    windows: head.windows,
                    records: Vec::new(),
                });
            }
        }
        scratch.each_gathered(
            memory,
            &[field(0, 0, STRIDE)],
            STRIDE,
            REFUSALS[6],
            VALUE_CHUNK,
            |scratch, at, bytes| {
                let (history, level, slot) = scratch.owners[at];
                let values = values(bytes);
                if values != ZERO
                    && scratch.heads[history].refused.is_none()
                    && let Some(copy) = &mut copies[history]
                {
                    crate::allocation::reserve(&mut copy.records, 1)
                        .map_err(|_| "air menu history allocation failed")?;
                    copy.records.push(Record {
                        level: level as u8,
                        slot,
                        values,
                    });
                }
                Ok(())
            },
        )?;
        for (at, copy) in copies.iter_mut().take(batch.len()).enumerate() {
            let result = match scratch.heads[at].refused {
                Some(reason) => Err(reason),
                None => copy.take().ok_or(REFUSALS[1]),
            };
            each(first + at, result)?;
        }
    }
    Ok(())
}

pub fn capture_current(
    memory: &impl Memory,
    base: usize,
    histories: &[usize],
    scratch: &mut Scratch,
    mut each: impl FnMut(usize, Result<Current, &'static str>) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    for (first, batch) in (0..).step_by(BATCH).zip(histories.chunks(BATCH)) {
        headers(memory, base, batch, scratch, false)?;
        channel_pointers(memory, batch, scratch)?;
        let mut read: [Option<[i64; CHANNELS]>; BATCH] = [None; BATCH];
        scratch.each_gathered(
            memory,
            &[field(0, 0, STRIDE)],
            STRIDE,
            REFUSALS[6],
            VALUE_CHUNK,
            |scratch, at, bytes| {
                read[scratch.owners[at].0] = Some(values(bytes));
                Ok(())
            },
        )?;
        for (at, values) in read.iter().take(batch.len()).enumerate() {
            let head = scratch.heads[at];
            let result = match (head.refused, values) {
                (Some(reason), _) => Err(reason),
                (None, Some(values)) => Ok(Current {
                    roll_up: head.roll_up,
                    windows: head.windows,
                    values: *values,
                }),
                (None, None) => Err(REFUSALS[4]),
            };
            each(first + at, result)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history(roll_up: i32, capacities: [u32; LEVELS]) -> History {
        History {
            roll_up,
            windows: capacities.map(|capacity| Window {
                capacity,
                current: 0,
                wrapped: false,
            }),
            records: Vec::new(),
        }
    }

    fn value(amount: i64) -> [i64; CHANNELS] {
        let mut values = ZERO;
        values[3] = amount;
        values
    }

    #[test]
    fn a_sparse_advance_rolls_days_into_months_and_years_like_the_queue() {
        let mut held = history(0, [3, 2, 2]);
        for day in 0..12_i64 {
            held.set(0, held.windows[0].current, value(day + 1))
                .unwrap();
            held.advance(1).unwrap();
        }
        assert_eq!(
            held.windows[0],
            Window {
                capacity: 3,
                current: 0,
                wrapped: true
            }
        );
        assert!(held.records.iter().all(|record| record.values != ZERO));
        assert!(
            held.records
                .windows(2)
                .all(|pair| (pair[0].level, pair[0].slot) < (pair[1].level, pair[1].slot))
        );
        assert_eq!(held.at(0, 0), None);
        let mut sum = held.clone();
        sum.advance(0).unwrap();
        assert_eq!(sum, held);
    }

    #[test]
    fn roll_up_one_keeps_the_newest_positive_record() {
        let mut held = history(1, [3, 2, 2]);
        held.set(0, 0, value(5)).unwrap();
        held.advance(1).unwrap();
        held.set(0, 1, value(-2)).unwrap();
        held.advance(1).unwrap();
        assert_eq!(held.at(1, 0), Some(&value(5)));
    }

    #[test]
    fn a_current_read_applies_only_where_every_window_stands() {
        let mut held = history(0, [3, 2, 2]);
        let current = Current {
            roll_up: 0,
            windows: held.windows,
            values: value(7),
        };
        assert_eq!(held.apply_current(&current), Ok(true));
        assert_eq!(held.apply_current(&current), Ok(false));
        let mut moved = current;
        moved.windows[0].current = 1;
        assert!(held.apply_current(&moved).is_err());
        let zero = Current {
            values: ZERO,
            ..current
        };
        assert_eq!(held.apply_current(&zero), Ok(true));
        assert!(held.records.is_empty());
    }
}
