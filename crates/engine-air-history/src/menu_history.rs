use crate::{
    Memory, cloop_history, cpdx_array::Array, loop_history_record_fields as record,
    loop_history_window as window,
};
use engine_native::memory::{RecordField, RecordLayout, RecordSources};

pub const CHANNELS: usize = cloop_history::air_stats::FIELD_COUNT as usize;
const BATCH: usize = 64;
const STRIDE: usize = CHANNELS * 8;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Window {
    pub current: u32,
    pub wrapped: bool,
    pub records: Vec<[i64; CHANNELS]>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct History {
    pub roll_up: i32,
    pub windows: Vec<Window>,
}

impl Window {
    fn step(&mut self) -> bool {
        self.current += 1;
        if self.current as usize == self.records.len() {
            self.current = 0;
            self.wrapped = true;
            true
        } else {
            false
        }
    }

    fn aggregate(&self, roll_up: i32, include_current: bool) -> [i64; CHANNELS] {
        let count = if self.wrapped {
            self.records.len()
        } else {
            self.current as usize
        };
        let start = if self.wrapped {
            self.current as usize
        } else {
            0
        };
        if roll_up == 1 {
            return (0..count)
                .rev()
                .map(|at| self.records[(start + at) % self.records.len()])
                .find(|record| record.iter().any(|&value| value > 0))
                .unwrap_or([0; CHANNELS]);
        }
        let mut sum = [0_i64; CHANNELS];
        let mut add = |record: &[i64; CHANNELS]| {
            for (sum, &value) in sum.iter_mut().zip(record) {
                *sum = sum.wrapping_add(value).clamp(
                    -crate::air_kamikaze::SUM_LIMIT,
                    crate::air_kamikaze::SUM_LIMIT,
                );
            }
        };
        for at in 0..count {
            add(&self.records[(start + at) % self.records.len()]);
        }
        if include_current && !self.wrapped {
            add(&self.records[self.current as usize]);
        }
        sum
    }
}

impl History {
    pub fn advance(&mut self, count: u64) {
        let [daily, monthly, yearly] = self.windows.as_mut_slice() else {
            return;
        };
        for _ in 0..count {
            let wrapped = daily.step();
            monthly.records[monthly.current as usize] = daily.aggregate(self.roll_up, !wrapped);
            if wrapped {
                let wrapped = monthly.step();
                if wrapped {
                    yearly.records[yearly.current as usize] =
                        monthly.aggregate(self.roll_up, false);
                    yearly.step();
                }
                monthly.records[monthly.current as usize] = [0; CHANNELS];
            } else {
                yearly.records[yearly.current as usize] = monthly.aggregate(self.roll_up, true);
            }
            daily.records[daily.current as usize] = [0; CHANNELS];
        }
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
    owners: Vec<(usize, usize, usize)>,
    failures: Vec<Option<&'static str>>,
}

impl Scratch {
    pub const fn new() -> Self {
        Self {
            bytes: Vec::new(),
            pointers: Vec::new(),
            owners: Vec::new(),
            failures: Vec::new(),
        }
    }

    pub fn allocated_bytes(&self) -> usize {
        self.bytes.capacity()
            + self.pointers.capacity() * size_of::<usize>()
            + self.owners.capacity() * size_of::<(usize, usize, usize)>()
            + self.failures.capacity() * size_of::<Option<&'static str>>()
    }

    fn gather(
        &mut self,
        memory: &impl Memory,
        fields: &[RecordField],
        stride: usize,
        reason: &'static str,
    ) -> Result<(), &'static str> {
        let len = self
            .pointers
            .len()
            .checked_mul(stride)
            .ok_or("air menu history size overflow")?;
        crate::allocation::resize(&mut self.bytes, len, 0)
            .map_err(|_| "air menu history allocation failed")?;
        crate::allocation::resize(&mut self.failures, self.pointers.len(), None)
            .map_err(|_| "air menu history allocation failed")?;
        self.failures.fill(None);
        let layout = RecordLayout::new(fields, stride).ok_or("air menu history layout invalid")?;
        let mut first = 0;
        while first < self.pointers.len() {
            let complete = memory.copy_records(
                RecordSources::Pointers(&self.pointers[first..]),
                &layout,
                &mut self.bytes[first * stride..],
            );
            first += complete / fields.len();
            if first < self.pointers.len() {
                self.failures[first] = Some(reason);
                first += 1;
            }
        }
        Ok(())
    }

    fn compact(&mut self, refused: &[Option<&'static str>]) {
        let mut live = 0;
        for at in 0..self.pointers.len() {
            if refused[self.owners[at].0].is_none() {
                self.pointers[live] = self.pointers[at];
                self.owners[live] = self.owners[at];
                live += 1;
            }
        }
        self.pointers.truncate(live);
        self.owners.truncate(live);
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

pub fn capture(
    memory: &impl Memory,
    base: usize,
    histories: &[usize],
    scratch: &mut Scratch,
    mut each: impl FnMut(usize, Result<History, &'static str>) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let history_vtable = base
        .checked_add(cloop_history::VTABLE_RVA)
        .ok_or("air menu history address overflow")?;
    let window_vtable = base
        .checked_add(window::VTABLE_RVA)
        .ok_or("air menu history address overflow")?;
    for (first, histories) in (0..).step_by(BATCH).zip(histories.chunks(BATCH)) {
        let mut results = Vec::new();
        crate::allocation::reserve_exact(&mut results, histories.len())
            .map_err(|_| "air menu history allocation failed")?;
        results.resize_with(histories.len(), || None::<History>);
        let mut refused = [None; BATCH];
        scratch.pointers.clear();
        crate::allocation::reserve_exact(&mut scratch.pointers, histories.len() * 3)
            .map_err(|_| "air menu history allocation failed")?;
        scratch.pointers.extend_from_slice(histories);
        scratch.gather(
            memory,
            &[
                field(0, 0, 8),
                field(cloop_history::DAILY_WINDOW, 8, 24),
                field(cloop_history::SELF_REFERENCE, 32, 8),
                field(cloop_history::ROLL_UP, 40, 8),
            ],
            48,
            REFUSALS[0],
        )?;
        let mut windows = Vec::new();
        crate::allocation::reserve_exact(&mut windows, histories.len() * 3)
            .map_err(|_| "air menu history allocation failed")?;
        for (at, bytes) in scratch.bytes.chunks_exact(48).enumerate() {
            if let Some(reason) = scratch.failures[at] {
                refused[at] = Some(reason);
                continue;
            }
            let roll_up = word(bytes, 40) as i32;
            if pointer(bytes, 0)? != history_vtable
                || pointer(bytes, 32)? != histories[at]
                || word(bytes, 44) as usize != CHANNELS
                || !matches!(roll_up, 0 | 1)
            {
                refused[at] = Some(REFUSALS[1]);
                continue;
            }
            let mut history = History {
                roll_up,
                windows: Vec::new(),
            };
            crate::allocation::reserve_exact(&mut history.windows, 3)
                .map_err(|_| "air menu history allocation failed")?;
            history.windows.resize_with(3, Window::default);
            for level in 0..3 {
                windows.push((at, level, pointer(bytes, 8 + level * 8)?));
            }
            results[at] = Some(history);
        }
        scratch.pointers.clear();
        scratch.pointers.extend(windows.iter().map(|entry| entry.2));
        scratch.gather(
            memory,
            &[
                field(0, 0, 8),
                field(window::RECORDS, 8, 16),
                field(window::CAPACITY, 24, 9),
                field(window::HISTORY, 33, 8),
            ],
            41,
            REFUSALS[2],
        )?;
        scratch.owners.clear();
        let mut slots = Vec::new();
        for (at, (&(history, level, _), bytes)) in windows
            .iter()
            .zip(scratch.bytes.chunks_exact(41))
            .enumerate()
        {
            if let Some(reason) = scratch.failures[at] {
                refused[history].get_or_insert(reason);
                continue;
            }
            let capacity = word(bytes, 24) as i32;
            let current = word(bytes, 28);
            let Some(array) =
                Array::from_header(bytes[8..24].try_into().unwrap(), 8).filter(|array| {
                    capacity > 0 && array.count >= capacity as u32 && current < capacity as u32
                })
            else {
                refused[history].get_or_insert(REFUSALS[3]);
                continue;
            };
            if pointer(bytes, 0)? != window_vtable
                || pointer(bytes, 33)? != histories[history]
                || bytes[32] > 1
            {
                refused[history].get_or_insert(REFUSALS[3]);
                continue;
            }
            let target = &mut results[history].as_mut().unwrap().windows[level];
            target.current = current;
            target.wrapped = bytes[32] != 0;
            crate::allocation::resize(&mut target.records, capacity as usize, [0; CHANNELS])
                .map_err(|_| "air menu history allocation failed")?;
            crate::allocation::reserve(&mut slots, capacity as usize)
                .map_err(|_| "air menu history allocation failed")?;
            crate::allocation::reserve(&mut scratch.owners, capacity as usize)
                .map_err(|_| "air menu history allocation failed")?;
            for slot in 0..capacity as u32 {
                slots.push(
                    array
                        .element(slot, 8)
                        .ok_or("air menu history slot overflow")?,
                );
                scratch.owners.push((history, level, slot as usize));
            }
        }
        scratch.pointers.clear();
        crate::allocation::reserve_exact(&mut scratch.pointers, slots.len())
            .map_err(|_| "air menu history allocation failed")?;
        scratch.pointers.extend_from_slice(&slots);
        scratch.compact(&refused);
        scratch.gather(memory, &[field(0, 0, 8)], 8, REFUSALS[4])?;
        for (at, bytes) in scratch.bytes.chunks_exact(8).enumerate() {
            if let Some(reason) = scratch.failures[at] {
                refused[scratch.owners[at].0].get_or_insert(reason);
                continue;
            }
            scratch.pointers[at] = pointer(bytes, 0)?;
        }
        scratch.compact(&refused);
        scratch.gather(
            memory,
            &[field(record::VALUES, 0, 16), field(record::HISTORY, 16, 8)],
            24,
            REFUSALS[5],
        )?;
        for (at, bytes) in scratch.bytes.chunks_exact(24).enumerate() {
            let history = scratch.owners[at].0;
            if let Some(reason) = scratch.failures[at] {
                refused[history].get_or_insert(reason);
                continue;
            }
            let Some(array) = Array::from_header(bytes[..16].try_into().unwrap(), 8)
                .filter(|array| array.count as usize == CHANNELS)
            else {
                refused[history].get_or_insert(REFUSALS[5]);
                continue;
            };
            if pointer(bytes, 16)? != histories[history] {
                refused[history].get_or_insert(REFUSALS[5]);
                continue;
            }
            scratch.pointers[at] = array.data;
        }
        scratch.compact(&refused);
        scratch.gather(memory, &[field(0, 0, STRIDE)], STRIDE, REFUSALS[6])?;
        for (at, bytes) in scratch.bytes.chunks_exact(STRIDE).enumerate() {
            let (history, level, slot) = scratch.owners[at];
            if let Some(reason) = scratch.failures[at] {
                refused[history].get_or_insert(reason);
                continue;
            }
            results[history].as_mut().unwrap().windows[level].records[slot] =
                std::array::from_fn(|channel| {
                    i64::from_le_bytes(bytes[channel * 8..channel * 8 + 8].try_into().unwrap())
                });
        }
        for (at, result) in results.into_iter().enumerate() {
            each(
                first + at,
                match refused[at] {
                    Some(reason) => Err(reason),
                    None => result.ok_or(REFUSALS[1]),
                },
            )?;
        }
    }
    Ok(())
}
