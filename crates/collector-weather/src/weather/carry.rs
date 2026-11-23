use super::*;
use crate::country_input_changes::weather_carry::{
    CUSTOM_SEA_REFUSALS, FULL_HOURS, MODIFIER_FULL_HOURS, MODIFIER_LOOKUPS, MODIFIER_REFUSALS,
    MODIFIER_TICKS, MODIFIER_WINDOW_HOURS, WINDOW_HOURS, WINDOW_RECORDS,
};
use engine_allocation::SharedBox;
use std::sync::atomic::Ordering::Relaxed;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pass {
    pub data: usize,
    pub count: u32,
    pub cursor: i32,
    pub budget: i32,
}

impl Pass {
    fn read(
        memory: &impl Memory,
        manager: usize,
        array: engine_native_memory::cpdx_array::Array,
        cursor: usize,
        budget: usize,
    ) -> Option<Self> {
        Some(Self {
            data: array.data,
            count: array.count,
            cursor: engine_native_memory::read::<i32>(memory, manager.checked_add(cursor)?)?,
            budget: engine_native_memory::read::<i32>(memory, manager.checked_add(budget)?)?,
        })
    }

    fn window_to(self, now: Self) -> Option<(u32, u32)> {
        if (self.data, self.count, self.budget) != (now.data, now.count, now.budget) {
            return None;
        }
        let (first, visits) = window(self.cursor, i32::try_from(now.count).ok()?, self.budget)?;
        (visits != 0 && now.cursor >= 0 && advanced(first, visits, now.count) == now.cursor as u32)
            .then_some((first, visits))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Words {
    pub manager: usize,
    pub provinces: Pass,
    pub regions: Pass,
    pub flags: [u8; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Windows {
    pub provinces: (u32, u32),
    pub regions: (u32, u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Held {
    hour: i32,
    writes: u64,
    words: Words,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Regions,
    Provinces,
}

#[derive(Debug)]
pub struct WeatherCarry {
    held: Option<Held>,
    regions: Vec<u8>,
    provinces: Vec<u8>,
    combat: Vec<Option<RegionModifiers>>,
    pub(super) air_efficiency: Vec<air_efficiency::Tree>,
    province_modifiers: ProvinceModifiers,
    province_statics: Option<SharedBox<ProvinceStatics>>,
    custom_sea: super::entries::CustomSea,
    hour: i32,
    force_full: bool,
    writes: Option<u64>,
    now: Option<Words>,
    windows: Option<Windows>,
    complete: bool,
    followed: u8,
    provinces_followed: bool,
    regions_followed: bool,
}

impl Default for WeatherCarry {
    fn default() -> Self {
        Self {
            held: None,
            regions: Vec::new(),
            provinces: Vec::new(),
            combat: Vec::new(),
            air_efficiency: Vec::new(),
            province_modifiers: ProvinceModifiers::default(),
            province_statics: None,
            custom_sea: Default::default(),
            hour: 0,
            force_full: false,
            writes: None,
            now: None,
            windows: None,
            complete: false,
            followed: 0,
            provinces_followed: false,
            regions_followed: false,
        }
    }
}

pub fn window(cursor: i32, count: i32, budget: i32) -> Option<(u32, u32)> {
    if cursor < 0 || count <= 0 {
        return None;
    }
    if cursor >= count {
        return Some((cursor as u32, 0));
    }
    Some((cursor as u32, budget.max(1).min(count) as u32))
}

fn advanced(first: u32, visits: u32, count: u32) -> u32 {
    ((u64::from(first) + u64::from(visits)) % u64::from(count)) as u32
}

pub fn read_words(
    memory: &impl Memory,
    manager: usize,
    provinces: Option<engine_native_memory::cpdx_array::Array>,
    regions: Option<engine_native_memory::cpdx_array::Array>,
) -> Option<Words> {
    Some(Words {
        manager,
        provinces: Pass::read(
            memory,
            manager,
            provinces?,
            cweather_manager::UPDATE_CURSOR,
            cweather_manager::PROVINCES_PER_UPDATE,
        )?,
        regions: Pass::read(
            memory,
            manager,
            regions?,
            cweather_manager::REGION_CURSOR,
            cweather_manager::REGIONS_PER_UPDATE,
        )?,
        flags: engine_native_memory::read::<[u8; 3]>(
            memory,
            manager.checked_add(cweather_manager::INITIAL_MODE)?,
        )?,
    })
}

impl WeatherCarry {
    pub fn begin(&mut self, hour: i32, force_full: bool, writes: Option<u64>) {
        self.hour = hour;
        self.force_full = force_full;
        self.writes = writes;
    }

    pub fn forget(&mut self) {
        self.held = None;
    }

    fn windows_of(&self, now: Words) -> Option<Windows> {
        let held = self.held?;
        let writes = self.writes?;
        if self.force_full
            || held.hour.checked_add(1) != Some(self.hour)
            || held.writes != writes
            || now.flags != [0; 3]
            || now.manager != held.words.manager
        {
            return None;
        }
        Some(Windows {
            provinces: held.words.provinces.window_to(now.provinces)?,
            regions: held.words.regions.window_to(now.regions)?,
        })
    }

    fn start(
        &mut self,
        memory: &impl Memory,
        manager: usize,
        provinces: Option<engine_native_memory::cpdx_array::Array>,
        regions: Option<engine_native_memory::cpdx_array::Array>,
    ) {
        self.now = read_words(memory, manager, provinces, regions);
        self.windows = self.now.and_then(|now| self.windows_of(now));
        self.complete = true;
        self.followed = 0;
        self.provinces_followed = false;
        self.regions_followed = false;
    }

    fn finish(&mut self) {
        match (self.complete, self.now, self.writes) {
            (true, Some(words), Some(writes)) => {
                self.held = Some(Held {
                    hour: self.hour,
                    writes,
                    words,
                });
            }
            _ => self.held = None,
        }
        if self.followed == 3 {
            WINDOW_HOURS.fetch_add(1, Relaxed);
        } else {
            FULL_HOURS.fetch_add(1, Relaxed);
        }
    }

    fn held_bytes(&mut self, part: Part) -> &mut Vec<u8> {
        match part {
            Part::Regions => &mut self.regions,
            Part::Provinces => &mut self.provinces,
        }
    }

    fn window(&self, part: Part) -> Option<(u32, u32)> {
        self.windows.map(|windows| match part {
            Part::Regions => windows.regions,
            Part::Provinces => windows.provinces,
        })
    }

    pub(super) fn region_window(&self) -> Option<(u32, u32)> {
        self.window(Part::Regions)
    }

    fn statics_held(&self) -> Option<&SharedBox<ProvinceStatics>> {
        let (held, now, statics) = (self.held?, self.now?, self.province_statics.as_ref()?);
        (!self.force_full
            && held.hour.checked_add(1) == Some(self.hour)
            && self.hour.rem_euclid(replay_data::cadence::STAGGER_PERIOD) != 0
            && self.writes == Some(held.writes)
            && now.manager == held.words.manager
            && (now.provinces.data, now.provinces.count)
                == (held.words.provinces.data, held.words.provinces.count)
            && statics.len() == now.provinces.count as usize)
            .then_some(statics)
    }
}

fn in_window(slot: u32, (first, visits): (u32, u32), count: u32) -> bool {
    (u64::from(slot) + u64::from(count) - u64::from(first)) % u64::from(count) < u64::from(visits)
}

fn gather(
    memory: &impl Memory,
    array: engine_native_memory::cpdx_array::Array,
    stride: usize,
    layout: &RecordLayout<'_>,
    first: u32,
    count: u32,
    out: &mut [u8],
) -> Result<(), usize> {
    let record_bytes = layout.stride();
    let fields = layout.field_count();
    let mut done = 0;
    while done < count {
        let batch = (count - done).min(64);
        let Some(base) = array.element(first + done, stride) else {
            return Err(done as usize * fields);
        };
        let sources = RecordSources::Strided {
            base,
            stride,
            count: batch as usize,
        };
        let begin = done as usize * record_bytes;
        let completed = memory.copy_records(
            sources,
            layout,
            &mut out[begin..begin + batch as usize * record_bytes],
        );
        if completed != batch as usize * fields {
            return Err(done as usize * fields + completed);
        }
        done += batch;
    }
    Ok(())
}

pub(super) fn start(job: &mut WeatherJob, memory: &impl Memory) {
    let (manager, provinces, regions) = (job.manager, job.provinces.array, job.regions.array);
    if let Some(carry) = job.carry.as_deref_mut() {
        carry.start(memory, manager, provinces, regions);
    }
}

pub(super) fn finish(job: &mut WeatherJob) {
    if let Some(carry) = job.carry.as_deref_mut() {
        carry.finish();
    }
}

pub(super) fn capture_regions(
    memory: &impl Memory,
    job: &mut WeatherJob,
    layout: &RecordLayout<'_>,
) {
    capture(
        memory,
        &mut job.regions,
        job.carry.as_deref_mut(),
        Part::Regions,
        sweather_per_region::STRIDE,
        layout,
    );
}

pub(super) fn capture_provinces(
    memory: &impl Memory,
    job: &mut WeatherJob,
    layout: &RecordLayout<'_>,
) {
    capture(
        memory,
        &mut job.provinces,
        job.carry.as_deref_mut(),
        Part::Provinces,
        sweather_per_province::STRIDE,
        layout,
    );
    job.province_window = job
        .carry
        .as_deref()
        .filter(|carry| carry.provinces_followed)
        .and_then(|carry| carry.window(Part::Provinces));
}

fn capture(
    memory: &impl Memory,
    payload: &mut DensePayload,
    carry: Option<&mut WeatherCarry>,
    part: Part,
    stride: usize,
    layout: &RecordLayout<'_>,
) {
    let Some(carry) = carry else {
        payload.capture(memory, stride, layout);
        return;
    };
    let record_bytes = layout.stride();
    let fields = layout.field_count();
    if let (Some(window), Some(array)) = (carry.window(part), payload.array) {
        let followed = follow(memory, array, stride, layout, carry, part, window);
        let total = array.count as usize * record_bytes;
        match followed {
            Followed::Ineligible => {}
            Followed::Read => {
                payload.bytes[..total].copy_from_slice(&carry.held_bytes(part)[..total]);
                payload.copied_fields = array.count as usize * fields;
                payload.copied_bytes = layout.completed_bytes(window.1 as usize * fields);
                carry.followed += 1;
                carry.provinces_followed |= part == Part::Provinces;
                carry.regions_followed |= part == Part::Regions;
                WINDOW_RECORDS.fetch_add(u64::from(window.1), Relaxed);
                return;
            }
            Followed::Failed { current, read } => {
                payload.bytes[..total].copy_from_slice(&carry.held_bytes(part)[..total]);
                payload.copied_fields = current;
                payload.copied_bytes = layout.completed_bytes(read);
                carry.complete = false;
                return;
            }
        }
    }
    payload.capture(memory, stride, layout);
    let total = payload
        .array
        .filter(|array| payload.copied_fields == array.count as usize * fields)
        .map(|array| array.count as usize * record_bytes);
    let held = carry.held_bytes(part);
    match total {
        Some(total) if held.len() >= total || engine_allocation::resize(held, total, 0).is_ok() => {
            held[..total].copy_from_slice(&payload.bytes[..total]);
        }
        _ => carry.complete = false,
    }
}

enum Followed {
    Ineligible,
    Read,
    Failed {
        current: usize,
        read: usize,
    },
}

fn follow(
    memory: &impl Memory,
    array: engine_native_memory::cpdx_array::Array,
    stride: usize,
    layout: &RecordLayout<'_>,
    carry: &mut WeatherCarry,
    part: Part,
    (first, visits): (u32, u32),
) -> Followed {
    let record_bytes = layout.stride();
    let fields = layout.field_count();
    let count = array.count;
    if carry.held_bytes(part).len() < count as usize * record_bytes {
        return Followed::Ineligible;
    }
    let head = visits.min(count - first);
    let tail = visits - head;
    let held = carry.held_bytes(part);
    let (front, back) =
        held[..count as usize * record_bytes].split_at_mut(first as usize * record_bytes);
    if let Err(read) = gather(
        memory,
        array,
        stride,
        layout,
        first,
        head,
        &mut back[..head as usize * record_bytes],
    ) {
        let current = if tail == 0 {
            first as usize * fields + read
        } else {
            0
        };
        return Followed::Failed { current, read };
    }
    match gather(
        memory,
        array,
        stride,
        layout,
        0,
        tail,
        &mut front[..tail as usize * record_bytes],
    ) {
        Ok(()) => Followed::Read,
        Err(read) => Followed::Failed {
            current: read,
            read: head as usize * fields + read,
        },
    }
}

fn region_combat(
    memory: &impl Memory,
    array: engine_native_memory::cpdx_array::Array,
    slot: u32,
) -> Option<RegionModifiers> {
    array
        .element(slot, sweather_per_region::STRIDE)
        .and_then(|record| record.checked_add(sweather_per_region::MODIFIERS))
        .and_then(|accumulator| {
            let mut values =
                [engine_native_memory::modifier_cache::Value::default(); COMBAT_KEYS.len()];
            engine_native_parts::modifier_accumulator::lookup_many_at(
                memory,
                accumulator,
                &COMBAT_KEYS,
                &mut values,
            )
            .ok()
            .map(|()| values.map(|value| value.present.then_some(value.raw)))
        })
}

pub(super) fn capture_combat(memory: &impl Memory, job: &mut WeatherJob) {
    let Some(array) = job.regions.array else {
        return;
    };
    let count = array.count as usize;
    if engine_allocation::reserve_exact(&mut job.combat, count)
        .map_err(crate::allocation::io)
        .is_err()
    {
        return;
    }
    if let Some(carry) = job.carry.as_deref_mut()
        && let Some(window) = carry.window(Part::Regions)
        && carry.combat.len() == count
    {
        let complete = follow_combat(memory, array, carry, window);
        job.combat.extend_from_slice(&carry.combat);
        if complete {
            carry.followed += 1;
        } else {
            carry.combat.clear();
            carry.complete = false;
        }
        return;
    }
    for slot in 0..array.count {
        job.combat.push(region_combat(memory, array, slot));
    }
    if let Some(carry) = job.carry.as_deref_mut() {
        if job.combat.iter().all(Option::is_some)
            && (carry.combat.capacity() >= count
                || engine_allocation::reserve_exact(&mut carry.combat, count).is_ok())
        {
            carry.combat.clear();
            carry.combat.extend_from_slice(&job.combat[..count]);
        } else {
            carry.combat.clear();
            carry.complete = false;
        }
    }
}

fn follow_combat(
    memory: &impl Memory,
    array: engine_native_memory::cpdx_array::Array,
    carry: &mut WeatherCarry,
    (first, visits): (u32, u32),
) -> bool {
    let count = array.count;
    let mut complete = true;
    for visit in 0..visits {
        let slot = (first + visit) % count;
        let fresh = region_combat(memory, array, slot);
        complete &= fresh.is_some();
        carry.combat[slot as usize] = fresh;
    }
    complete
}

fn read_custom_sea(
    memory: &impl Memory,
    record: &[u8],
    bytes: &mut Vec<u8>,
    values: &mut Vec<i16>,
) -> bool {
    values.clear();
    let Some(array) = sweather_per_region::custom_sea(record) else {
        return false;
    };
    let Some(len) = (array.count as usize).checked_mul(2) else {
        return false;
    };
    if len == 0 {
        return true;
    }
    if (bytes.len() < len && engine_allocation::resize(bytes, len, 0).is_err())
        || engine_allocation::reserve(values, array.count as usize).is_err()
        || !memory.copy(array.data, &mut bytes[..len])
    {
        return false;
    }
    values.extend(
        bytes[..len]
            .chunks_exact(2)
            .map(|word| i16::from_le_bytes([word[0], word[1]])),
    );
    true
}

pub(super) fn capture_custom_sea(memory: &impl Memory, job: &mut WeatherJob) {
    job.custom_sea.clear();
    let Some(array) = job.regions.array else {
        return;
    };
    let count = array.count;
    if job.regions.faulted(sweather_per_region::FIELDS.len()) {
        if let Some(carry) = job.carry.as_deref_mut() {
            carry.custom_sea.clear();
        }
        return;
    }
    job.custom_sea.reserve_slots(count as usize);
    let window = match job.carry.as_deref() {
        Some(carry) if carry.regions_followed && carry.custom_sea.len() == count as usize => {
            carry.window(Part::Regions)
        }
        _ => None,
    };
    let carried = job.carry.as_deref().map(|carry| &carry.custom_sea);
    let record_bytes = sweather_per_region::RECORD_BYTES;
    for slot in 0..count {
        let values = match (window, carried) {
            (Some(window), Some(carried)) if !in_window(slot, window, count) => {
                carried.get(slot as usize)
            }
            _ => job
                .regions
                .bytes
                .get(slot as usize * record_bytes..(slot as usize + 1) * record_bytes)
                .filter(|record| {
                    read_custom_sea(memory, record, &mut job.custom_bytes, &mut job.custom_values)
                })
                .map(|_| {
                    job.log.raw_bytes[0] += job.custom_values.len() as u64 * 2;
                    &job.custom_values[..]
                }),
        };
        if values.is_none() {
            CUSTOM_SEA_REFUSALS.fetch_add(1, Relaxed);
        }
        if !job.custom_sea.push(values) {
            CUSTOM_SEA_REFUSALS.fetch_add(u64::from(count - slot), Relaxed);
            break;
        }
    }
    if let Some(carry) = job.carry.as_deref_mut() {
        let whole = job.custom_sea.len() == count as usize && job.custom_sea.complete();
        if !(whole && carry.custom_sea.copy_from(&job.custom_sea)) {
            carry.custom_sea.clear();
        }
    }
}

fn read_province(
    memory: &impl Memory,
    array: engine_native_memory::cpdx_array::Array,
    slot: u32,
    out: &mut Vec<super::entries::Entry>,
) -> bool {
    MODIFIER_LOOKUPS.fetch_add(1, Relaxed);
    super::entries::read(memory, array, slot, out)
}

pub(super) fn capture_province_modifiers(memory: &impl Memory, job: &mut WeatherJob) {
    let begin = qpc();
    capture_province_modifiers_timed(memory, job);
    MODIFIER_TICKS.fetch_add(qpc().saturating_sub(begin).max(0) as u64, Relaxed);
}

fn capture_province_modifiers_timed(memory: &impl Memory, job: &mut WeatherJob) {
    let Some(array) = job.provinces.array else {
        return;
    };
    let count = array.count;
    job.province_modifiers.clear();
    job.province_modifiers.reserve_slots(count as usize);
    let mut scratch = std::mem::take(&mut job.province_scratch);
    let window = match job.carry.as_deref() {
        Some(carry)
            if carry.provinces_followed && carry.province_modifiers.len() == count as usize =>
        {
            carry.window(Part::Provinces)
        }
        _ => None,
    };
    job.modifiers_windowed = window.is_some();
    if window.is_some() {
        MODIFIER_WINDOW_HOURS.fetch_add(1, Relaxed);
    } else {
        MODIFIER_FULL_HOURS.fetch_add(1, Relaxed);
    }
    let carried = job.carry.as_deref().map(|carry| &carry.province_modifiers);
    for slot in 0..count {
        let entries = match (window, carried) {
            (Some(window), Some(carried)) if !in_window(slot, window, count) => {
                carried.get(slot as usize)
            }
            _ => read_province(memory, array, slot, &mut scratch).then_some(&scratch[..]),
        };
        if entries.is_none() {
            MODIFIER_REFUSALS.fetch_add(1, Relaxed);
        }
        if !job.province_modifiers.push(entries) {
            MODIFIER_REFUSALS.fetch_add(u64::from(count - slot), Relaxed);
            break;
        }
    }
    job.province_scratch = scratch;
    if let Some(carry) = job.carry.as_deref_mut() {
        let whole =
            job.province_modifiers.len() == count as usize && job.province_modifiers.complete();
        if !(whole && carry.province_modifiers.copy_from(&job.province_modifiers)) {
            carry.province_modifiers.clear();
        }
    }
}

pub(super) fn capture_province_statics(memory: &impl Memory, job: &mut WeatherJob) {
    job.province_statics = None;
    if let Some(held) = job.carry.as_deref().and_then(WeatherCarry::statics_held) {
        job.province_statics = Some(held.clone());
        job.statics_held = true;
        return;
    }
    job.statics_held = false;
    let previous = job
        .carry
        .as_deref_mut()
        .and_then(|carry| carry.province_statics.take());
    let read = job.provinces.array.and_then(|array| {
        read_province_statics(
            memory,
            array,
            job.static_vtable,
            &mut job.static_headers,
            &mut job.static_scratch,
            previous,
        )
    });
    if let Some(carry) = job.carry.as_deref_mut() {
        let count = job.provinces.array.map(|array| array.count as usize);
        carry.province_statics = read
            .as_ref()
            .filter(|statics| Some(statics.len()) == count && statics.complete())
            .cloned();
    }
    job.province_statics = read;
}

fn read_province_statics(
    memory: &impl Memory,
    array: engine_native_memory::cpdx_array::Array,
    definition_vtable: Option<usize>,
    headers: &mut Vec<u8>,
    list: &mut Vec<u8>,
    previous: Option<SharedBox<ProvinceStatics>>,
) -> Option<SharedBox<ProvinceStatics>> {
    const HEADER: RecordLayout<'static> =
        RecordLayout::new(&sweather_per_province::STATIC_MODIFIER_HEADER, 16)
            .expect("valid static modifier header layout");
    let count = array.count as usize;
    let bytes = count.checked_mul(16)?;
    if headers.len() < bytes && engine_allocation::resize(headers, bytes, 0).is_err() {
        return None;
    }
    let copied = match gather(
        memory,
        array,
        sweather_per_province::STRIDE,
        &HEADER,
        0,
        array.count,
        &mut headers[..bytes],
    ) {
        Ok(()) => count,
        Err(copied) => copied,
    };
    let mut shared = match previous {
        Some(previous) if SharedBox::strong_count(&previous) == 1 => previous,
        _ => engine_allocation::shared(ProvinceStatics::default()).ok()?,
    };
    let statics = SharedBox::get_mut(&mut shared)?;
    statics.clear();
    statics.reserve_slots(count);
    for (slot, header) in headers[..bytes].chunks_exact(16).enumerate() {
        let read = slot < copied
            && super::entries::read_statics(
                memory,
                header.try_into().expect("a 16-byte header"),
                definition_vtable,
                list,
            );
        if !statics.push(read.then_some(&list[..])) {
            break;
        }
    }
    Some(shared)
}

#[cfg(test)]
mod tests;
