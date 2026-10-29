//! The window-following province and region read (`weather_changes`).
//!
//! With both suspension bytes and the initial-mode byte clear, the hourly weather update runs
//! exactly one window pass; it is the pass's only caller. The pass visits two rotating windows:
//! Province slots from their cursor , as many as `provinces_per_update` or the whole array,
//! wrapping at the live Count ; and region slots from their cursor, as many as
//! `regions_per_update`, wrapping at the region count ([`window`]). Both budgets are parsed from
//! `common/weather.txt`, so mods set them; each hour sizes its windows from the live words.
//!
//! Every hourly writer of a recorded province or region field is reached
//! only through those windows. Temperature staging has two callers, both
//! over the window's pointer list; publication copies in the pass and its
//! two jobs use that same list. The display writer runs only from the pass.
//! Region updates run from the pass and its range job over the region
//! window's list, and alone among hourly code rebuild region modifiers.
//! Writers outside the windows are the reseed (used by the weather
//! randomization effect, reload command and reset), the reset and reload
//! Commands  (reload initializes before reseeding), snow, mud and region
//! console commands, the console command running hourly game state, the
//! load reader and post-load initial passes, and the province modifier
//! addition and removal and the daily expiry's rebuild of a province whose list
//! dropped a modifier, which rebuild a province's modifier accumulator outside
//! the window. The country input family
//! hooks them and counts each call on entry and return
//! ([`crate::country_input_changes::weather_writes`]). A new game allocates
//! its new manager before freeing the old one, so their addresses differ.
//!
//! So an hour whose read finds the count, the manager, both arrays and both
//! budgets as the held read left them, the gates and the initial mode clear,
//! and both cursors exactly one pass further, reads only the windows'
//! records (and the visited regions' modifier entries) and carries every
//! other record from the held read. Anything else (the first hour, a gap, a
//! forced full hour, a moved count, a cursor that moved by anything but one
//! pass, a failed read) reads every record and holds it. A window read that
//! fails is not repeated: that part is unavailable for the hour, a refused
//! region's combat entries are absent, and the next hour reads every record.
//! Clocks and the two display arrays are still read whole every hour: the
//! day/night update rewrites every clock hourly.

use super::*;
use crate::country_input_changes::weather_carry::{
    FULL_HOURS, MODIFIER_FULL_HOURS, MODIFIER_LOOKUPS, MODIFIER_REFUSALS, MODIFIER_TICKS,
    MODIFIER_WINDOW_HOURS, WINDOW_HOURS, WINDOW_RECORDS,
};
use std::sync::atomic::Ordering::Relaxed;

/// One rotating array's live words.
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
        array: engine_native::cpdx_array::Array,
        cursor: usize,
        budget: usize,
    ) -> Option<Self> {
        Some(Self {
            data: array.data,
            count: array.count,
            cursor: engine_native::read::<i32>(memory, manager.checked_add(cursor)?)?,
            budget: engine_native::read::<i32>(memory, manager.checked_add(budget)?)?,
        })
    }

    /// The slots one ordinary pass visited from this held pass, when `now`
    /// is what it left: `(first, visits)`.
    fn window_to(self, now: Self) -> Option<(u32, u32)> {
        if (self.data, self.count, self.budget) != (now.data, now.count, now.budget) {
            return None;
        }
        let (first, visits) = window(self.cursor, i32::try_from(now.count).ok()?, self.budget)?;
        (visits != 0 && now.cursor >= 0 && advanced(first, visits, now.count) == now.cursor as u32)
            .then_some((first, visits))
    }
}

/// The live words an hour decides on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Words {
    pub manager: usize,
    pub provinces: Pass,
    pub regions: Pass,
    /// The initial mode and the two suspension bytes.
    pub flags: [u8; 3],
}

/// The windows an hour reads: `(first, visits)` of provinces and regions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Windows {
    pub provinces: (u32, u32),
    pub regions: (u32, u32),
}

/// What the held records were read under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Held {
    hour: i32,
    writes: u64,
    words: Words,
}

/// Which carried array.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Regions,
    Provinces,
}

/// The carried province and region records, loaned to each hour's weather
/// job.
#[derive(Debug)]
pub struct WeatherCarry {
    held: Option<Held>,
    /// Every region record of the held read, packed as the job packs them.
    regions: Vec<u8>,
    /// Every province record of the held read, packed as the job packs them.
    provinces: Vec<u8>,
    /// Every region's combat modifier entries of the held read.
    combat: Vec<Option<RegionModifiers>>,
    /// Every province's weather modifier accumulator of the held read.
    province_modifiers: ProvinceModifiers,
    /// This hour, set by the game thread before dispatch.
    hour: i32,
    force_full: bool,
    writes: Option<u64>,
    /// This hour's words and windows ([`WeatherCarry::start`]).
    now: Option<Words>,
    windows: Option<Windows>,
    /// Every part of this hour's read completed.
    complete: bool,
    /// Parts this hour read through their window.
    followed: u8,
    /// This hour's province records followed their window. A province's modifier is
    /// rebuilt with its record, so the modifier entries follow theirs only then.
    provinces_followed: bool,
}

impl Default for WeatherCarry {
    fn default() -> Self {
        Self {
            held: None,
            regions: Vec::new(),
            provinces: Vec::new(),
            combat: Vec::new(),
            province_modifiers: ProvinceModifiers::default(),
            hour: 0,
            force_full: false,
            writes: None,
            now: None,
            windows: None,
            complete: false,
            followed: 0,
            provinces_followed: false,
        }
    }
}

/// Slots an ordinary weather pass visits from `cursor`: `(first, visits)`,
/// wrapping at `count`. A signed comparison admits a cursor below the count.
/// The loop visits its slot, advances and wraps, then decrements the budget;
/// it continues while the budget is positive and the cursor has not returned
/// to its start. It visits at least one slot, at most the budget or the array
/// count. A cursor at or past the count visits nothing; a negative cursor
/// Would index before the array and is refused  (`None`).
pub fn window(cursor: i32, count: i32, budget: i32) -> Option<(u32, u32)> {
    if cursor < 0 || count <= 0 {
        return None;
    }
    if cursor >= count {
        return Some((cursor as u32, 0));
    }
    Some((cursor as u32, budget.max(1).min(count) as u32))
}

/// The cursor one ordinary pass leaves.
fn advanced(first: u32, visits: u32, count: u32) -> u32 {
    ((u64::from(first) + u64::from(visits)) % u64::from(count)) as u32
}

/// Reads the words an hour decides on; `None` when any does not read.
pub fn read_words(
    memory: &impl Memory,
    manager: usize,
    provinces: Option<engine_native::cpdx_array::Array>,
    regions: Option<engine_native::cpdx_array::Array>,
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
        flags: engine_native::read::<[u8; 3]>(
            memory,
            manager.checked_add(cweather_manager::INITIAL_MODE)?,
        )?,
    })
}

impl WeatherCarry {
    /// Game thread, before the hour's job is dispatched.
    pub fn begin(&mut self, hour: i32, force_full: bool, writes: Option<u64>) {
        self.hour = hour;
        self.force_full = force_full;
        self.writes = writes;
    }

    /// The job did not complete: the next hour reads every record.
    pub fn forget(&mut self) {
        self.held = None;
    }

    /// Whether this hour may read only the windows, and which.
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

    /// The hour's words and windows, before any record is read.
    fn start(
        &mut self,
        memory: &impl Memory,
        manager: usize,
        provinces: Option<engine_native::cpdx_array::Array>,
        regions: Option<engine_native::cpdx_array::Array>,
    ) {
        self.now = read_words(memory, manager, provinces, regions);
        self.windows = self.now.and_then(|now| self.windows_of(now));
        self.complete = true;
        self.followed = 0;
        self.provinces_followed = false;
    }

    /// After every part: holds what was read, or forgets it.
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
}

/// Whether `slot` is in the window `(first, visits)` of `count` slots.
fn in_window(slot: u32, (first, visits): (u32, u32), count: u32) -> bool {
    (u64::from(slot) + u64::from(count) - u64::from(first)) % u64::from(count) < u64::from(visits)
}

/// Gathers records `first..first + count` of `array` into `out`, packed;
/// `Err` with the fields that read when one did not.
fn gather(
    memory: &impl Memory,
    array: engine_native::cpdx_array::Array,
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

/// The hour's words and windows (`carry` loaned to the job), before any
/// record is read.
pub(super) fn start(job: &mut WeatherJob, memory: &impl Memory) {
    let (manager, provinces, regions) = (job.manager, job.provinces.array, job.regions.array);
    if let Some(carry) = job.carry.as_deref_mut() {
        carry.start(memory, manager, provinces, regions);
    }
}

/// After every part: holds what was read, or forgets it.
pub(super) fn finish(job: &mut WeatherJob) {
    if let Some(carry) = job.carry.as_deref_mut() {
        carry.finish();
    }
}

/// The hour's region records into the job.
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

/// The hour's province records into the job.
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
}

/// One part's records into `payload`, following its window when the carry
/// allows it.
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
                WINDOW_RECORDS.fetch_add(u64::from(window.1), Relaxed);
                return;
            }
            Followed::Failed { current, read } => {
                // A failed source is not read again: the part keeps the fields
                // read this hour, and the next hour reads every record.
                payload.bytes[..total].copy_from_slice(&carry.held_bytes(part)[..total]);
                payload.copied_fields = current;
                payload.copied_bytes = layout.completed_bytes(read);
                carry.complete = false;
                return;
            }
        }
    }
    // Every record, held when complete.
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

/// What following a window read.
enum Followed {
    /// Nothing was read: the part reads every record.
    Ineligible,
    Read,
    /// A field did not read. `current` fields of the held records, from slot
    /// zero, are this hour's; `read` fields were read.
    Failed {
        current: usize,
        read: usize,
    },
}

/// Reads `window` of `array` into the held records.
fn follow(
    memory: &impl Memory,
    array: engine_native::cpdx_array::Array,
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
    // The window, wrapping at the count, into the held records.
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
        // A wrapped window has not yet read its records from slot zero.
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

/// One region's combat modifier entries.
fn region_combat(
    memory: &impl Memory,
    array: engine_native::cpdx_array::Array,
    slot: u32,
) -> Option<RegionModifiers> {
    array
        .element(slot, sweather_per_region::STRIDE)
        .and_then(|record| record.checked_add(sweather_per_region::MODIFIERS))
        .and_then(|accumulator| {
            let mut values =
                [engine_native::modifier_accumulator::Value::default(); COMBAT_KEYS.len()];
            engine_native::modifier_accumulator::lookup_many_at(
                memory,
                accumulator,
                &COMBAT_KEYS,
                &mut values,
            )
            .ok()
            .map(|()| values.map(|value| value.present.then_some(value.raw)))
        })
}

/// [`WeatherJob::combat`] of every region record the dense gather located:
/// the window's regions afresh and the rest carried when the carry allows
/// it, otherwise every region; a worker grows its storage, a fixed fixture
/// refuses the regions past it.
pub(super) fn capture_combat(memory: &impl Memory, job: &mut WeatherJob) {
    let Some(array) = job.regions.array else {
        return;
    };
    let count = array.count as usize;
    if job.worker_growth && job.combat.capacity() < count {
        let _ =
            engine_allocation::reserve_exact(&mut job.combat, count).map_err(crate::allocation::io);
    }
    let room = count.min(job.combat.capacity());
    if let Some(carry) = job.carry.as_deref_mut()
        && let Some(window) = carry.window(Part::Regions)
        && carry.combat.len() == count
    {
        let complete = follow_combat(memory, array, carry, window);
        job.combat.extend_from_slice(&carry.combat[..room]);
        if complete {
            carry.followed += 1;
        } else {
            // A refused region is read again next hour, with every other.
            carry.combat.clear();
            carry.complete = false;
        }
        return;
    }
    for slot in 0..room {
        job.combat.push(region_combat(memory, array, slot as u32));
    }
    if let Some(carry) = job.carry.as_deref_mut() {
        // A refused region is read again next hour, not carried.
        if room == count
            && job.combat.iter().all(Option::is_some)
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

/// The window's combat entries into the carried ones, each region read
/// once; false when one was refused.
fn follow_combat(
    memory: &impl Memory,
    array: engine_native::cpdx_array::Array,
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

/// Reads one province's accumulator into `out`, counting the lookup; false refuses it.
fn read_province(
    memory: &impl Memory,
    array: engine_native::cpdx_array::Array,
    slot: u32,
    out: &mut Vec<super::entries::Entry>,
    grow: bool,
) -> bool {
    MODIFIER_LOOKUPS.fetch_add(1, Relaxed);
    super::entries::read(memory, array, slot, out, grow)
}

/// [`WeatherJob::province_modifiers`] of every province record: the window's
/// provinces afresh and the rest carried when the carry allows it, otherwise
/// every province. The weather window pass rebuilds a province record's modifier
/// with the record, so the record carry's writers are the modifier's: an hour whose
/// province records did not follow their window reads every province's entries too.
/// Provinces beyond the hour's storage are refused and counted.
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
    let grow = job.worker_growth;
    job.province_modifiers.clear();
    job.province_modifiers
        .reserve_provinces(count as usize, grow);
    let mut scratch = std::mem::take(&mut job.province_scratch);
    let window = match job.carry.as_deref() {
        Some(carry)
            if carry.provinces_followed && carry.province_modifiers.len() == count as usize =>
        {
            carry.window(Part::Provinces)
        }
        _ => None,
    };
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
            _ => read_province(memory, array, slot, &mut scratch, grow).then_some(&scratch[..]),
        };
        if entries.is_none() {
            MODIFIER_REFUSALS.fetch_add(1, Relaxed);
        }
        if !job.province_modifiers.push(entries, grow) {
            // This province and those after it, beyond the storage, are refused.
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

#[cfg(test)]
mod tests;
