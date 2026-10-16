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
//! load reader and post-load initial passes. The country input family
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
//! pass, a failed read, a poisoned carry) reads every record and holds it.
//! Clocks and the two display arrays are still read whole every hour: the
//! day/night update rewrites every clock hourly.
//!
//! Each window hour also re-reads carried records — every one under
//! `weather_verify`, otherwise the hour's rotating slice of slots — and
//! compares them with the held ones. A difference while the write count
//! moved is a hooked write during the read: the hour reads every record. A
//! difference with the count still is a write no hook saw: it poisons the
//! carry for the attachment.

use super::*;
use crate::country_input_changes::weather_carry::{
    EXPLAINED, FULL_HOURS, MODIFIER_COMPARE_TICKS, MODIFIER_DIFFERENCES, MODIFIER_FULL_HOURS,
    MODIFIER_LOOKUPS, MODIFIER_REFUSALS, MODIFIER_TICKS, MODIFIER_WINDOW_HOURS, POISONED, SAMPLED,
    VERIFY, WINDOW_HOURS, WINDOW_RECORDS,
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
        array: engine::cpdx_array::Array,
        cursor: usize,
        budget: usize,
    ) -> Option<Self> {
        Some(Self {
            data: array.data,
            count: array.count,
            cursor: engine::read::<i32>(memory, manager.checked_add(cursor)?)?,
            budget: engine::read::<i32>(memory, manager.checked_add(budget)?)?,
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
    /// Every province's weather modifier entries of the held read.
    province_modifiers: Vec<Option<ProvinceModifiers>>,
    /// Scratch for the comparison reads.
    scratch: Vec<u8>,
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
    /// A comparison found a write no hook saw: every hour reads every
    /// record for the rest of the attachment.
    poisoned: bool,
    /// The write count now, to tell a difference a hooked write explains.
    observe: fn() -> Option<u64>,
    /// Compare every carried record ([`VERIFY`]), not the hour's slice.
    every: bool,
}

impl Default for WeatherCarry {
    fn default() -> Self {
        Self {
            held: None,
            regions: Vec::new(),
            provinces: Vec::new(),
            combat: Vec::new(),
            province_modifiers: Vec::new(),
            scratch: Vec::new(),
            hour: 0,
            force_full: false,
            writes: None,
            now: None,
            windows: None,
            complete: false,
            followed: 0,
            provinces_followed: false,
            poisoned: false,
            observe: crate::country_input_changes::weather_writes,
            every: VERIFY,
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
    provinces: Option<engine::cpdx_array::Array>,
    regions: Option<engine::cpdx_array::Array>,
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
        flags: engine::read::<[u8; 3]>(
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
        if self.poisoned
            || self.force_full
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
        provinces: Option<engine::cpdx_array::Array>,
        regions: Option<engine::cpdx_array::Array>,
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

    /// A comparison found `mismatches`: explained by a hooked write during
    /// the read, or a write no hook saw (the carry is poisoned).
    fn differed(&mut self) {
        if (self.observe)() != self.writes {
            EXPLAINED.fetch_add(1, Relaxed);
        } else {
            self.poisoned = true;
            POISONED.store(true, Relaxed);
        }
    }
}

/// The slots of `0..count` a comparison reads this hour: `(start, step)`,
/// every slot under `weather_verify`, otherwise those in the hour's phase of
/// the rotation.
fn compared(hour: i32, every: bool) -> (u32, u32) {
    let check = crate::sampled_verify::Check::hourly(hour, every);
    if check == crate::sampled_verify::Check::Every {
        return (0, 1);
    }
    let rotation = crate::sampled_verify::ROTATION;
    let phase = (0..rotation).find(|&phase| check.covers_phase(phase));
    (phase.unwrap_or(0) as u32, rotation as u32)
}

/// Whether `slot` is in the window `(first, visits)` of `count` slots.
fn in_window(slot: u32, (first, visits): (u32, u32), count: u32) -> bool {
    (u64::from(slot) + u64::from(count) - u64::from(first)) % u64::from(count) < u64::from(visits)
}

/// Gathers records `first, first + step, ...` (`count` of them) of `array`
/// into `out`, packed; false when a field did not read.
#[allow(clippy::too_many_arguments)]
fn gather(
    memory: &impl Memory,
    array: engine::cpdx_array::Array,
    stride: usize,
    layout: &RecordLayout<'_>,
    first: u32,
    count: u32,
    step: u32,
    out: &mut [u8],
) -> bool {
    let record_bytes = layout.stride();
    let mut done = 0;
    while done < count {
        let batch = (count - done).min(64);
        let Some(base) = array.element(first + done * step, stride) else {
            return false;
        };
        let sources = RecordSources::Strided {
            base,
            stride: stride * step as usize,
            count: batch as usize,
        };
        let begin = done as usize * record_bytes;
        let completed = memory.copy_records(
            sources,
            layout,
            &mut out[begin..begin + batch as usize * record_bytes],
        );
        if completed != batch as usize * layout.field_count() {
            return false;
        }
        done += batch;
    }
    true
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
    if let (Some(window), Some(array)) = (carry.window(part), payload.array)
        && follow(memory, array, stride, layout, carry, part, window)
    {
        let total = array.count as usize * record_bytes;
        payload.bytes[..total].copy_from_slice(&carry.held_bytes(part)[..total]);
        payload.copied_fields = array.count as usize * fields;
        payload.copied_bytes = layout.completed_bytes(window.1 as usize * fields);
        carry.followed += 1;
        carry.provinces_followed |= part == Part::Provinces;
        WINDOW_RECORDS.fetch_add(u64::from(window.1), Relaxed);
        return;
    }
    // Every record, held when complete.
    payload.capture(memory, stride, layout);
    let total = payload
        .array
        .filter(|array| payload.copied_fields == array.count as usize * fields)
        .map(|array| array.count as usize * record_bytes);
    let held = carry.held_bytes(part);
    match total {
        Some(total)
            if held.len() >= total || engine::allocation::resize(held, total, 0).is_ok() =>
        {
            held[..total].copy_from_slice(&payload.bytes[..total]);
        }
        _ => carry.complete = false,
    }
}

/// Reads `window` of `array` into the held records and compares the hour's
/// carried slots; false when either did not hold (the part reads every
/// record).
fn follow(
    memory: &impl Memory,
    array: engine::cpdx_array::Array,
    stride: usize,
    layout: &RecordLayout<'_>,
    carry: &mut WeatherCarry,
    part: Part,
    (first, visits): (u32, u32),
) -> bool {
    let record_bytes = layout.stride();
    let count = array.count;
    if carry.held_bytes(part).len() < count as usize * record_bytes {
        return false;
    }
    // The comparison first, against the held bytes before the window
    // overwrites them.
    let (start, step) = compared(carry.hour, carry.every);
    if start < count {
        let compared = (count - start).div_ceil(step);
        let needed = compared as usize * record_bytes;
        if carry.scratch.len() < needed
            && engine::allocation::resize(&mut carry.scratch, needed, 0).is_err()
        {
            return false;
        }
        let mut scratch = std::mem::take(&mut carry.scratch);
        let read = gather(
            memory,
            array,
            stride,
            layout,
            start,
            compared,
            step,
            &mut scratch[..needed],
        );
        let (mut verified, mut mismatches) = (0, 0);
        if read {
            let held = carry.held_bytes(part);
            for index in 0..compared {
                let slot = start + index * step;
                if in_window(slot, (first, visits), count) {
                    continue;
                }
                let at = index as usize * record_bytes;
                let fresh = &scratch[at..at + record_bytes];
                let at = slot as usize * record_bytes;
                verified += 1;
                mismatches += u64::from(fresh != &held[at..at + record_bytes]);
            }
        }
        carry.scratch = scratch;
        SAMPLED.record(verified, mismatches, part as usize);
        if mismatches != 0 {
            carry.differed();
            return false;
        }
        if !read {
            return false;
        }
    }
    // The window, wrapping at the count, into the held records.
    let head = visits.min(count - first);
    let held = carry.held_bytes(part);
    let (front, back) =
        held[..count as usize * record_bytes].split_at_mut(first as usize * record_bytes);
    gather(
        memory,
        array,
        stride,
        layout,
        first,
        head,
        1,
        &mut back[..head as usize * record_bytes],
    ) && gather(
        memory,
        array,
        stride,
        layout,
        0,
        visits - head,
        1,
        &mut front[..(visits - head) as usize * record_bytes],
    )
}

/// One region's combat modifier entries.
fn region_combat(
    memory: &impl Memory,
    array: engine::cpdx_array::Array,
    slot: u32,
) -> Option<RegionModifiers> {
    array
        .element(slot, sweather_per_region::STRIDE)
        .and_then(|record| record.checked_add(sweather_per_region::MODIFIERS))
        .and_then(|accumulator| {
            let mut values = [engine::modifier_accumulator::Value::default(); COMBAT_KEYS.len()];
            engine::modifier_accumulator::lookup_many_at(
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
        let _ = engine::allocation::reserve_exact(&mut job.combat, count)
            .map_err(crate::allocation::io);
    }
    let room = count.min(job.combat.capacity());
    if let Some(carry) = job.carry.as_deref_mut()
        && let Some(window) = carry.window(Part::Regions)
        && carry.combat.len() == count
        && follow_combat(memory, array, carry, window)
    {
        job.combat.extend_from_slice(&carry.combat[..room]);
        carry.followed += 1;
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
                || engine::allocation::reserve_exact(&mut carry.combat, count).is_ok())
        {
            carry.combat.clear();
            carry.combat.extend_from_slice(&job.combat[..count]);
        } else {
            carry.combat.clear();
            carry.complete = false;
        }
    }
}

/// The window's combat entries into the carried ones, after comparing the
/// hour's carried slots; false on a difference or a refused read.
fn follow_combat(
    memory: &impl Memory,
    array: engine::cpdx_array::Array,
    carry: &mut WeatherCarry,
    (first, visits): (u32, u32),
) -> bool {
    let count = array.count;
    let (start, step) = compared(carry.hour, carry.every);
    let (mut verified, mut mismatches) = (0, 0);
    let mut slot = start;
    while slot < count {
        if !in_window(slot, (first, visits), count) {
            let Some(fresh) = region_combat(memory, array, slot) else {
                SAMPLED.record(verified, mismatches, 2);
                return false;
            };
            verified += 1;
            mismatches += u64::from(Some(fresh) != carry.combat[slot as usize]);
        }
        slot += step;
    }
    SAMPLED.record(verified, mismatches, 2);
    if mismatches != 0 {
        carry.differed();
        return false;
    }
    for visit in 0..visits {
        let slot = (first + visit) % count;
        let Some(fresh) = region_combat(memory, array, slot) else {
            return false;
        };
        carry.combat[slot as usize] = Some(fresh);
    }
    true
}

/// One province's weather modifier entries.
fn province_modifiers(
    memory: &impl Memory,
    array: engine::cpdx_array::Array,
    slot: u32,
) -> Option<ProvinceModifiers> {
    MODIFIER_LOOKUPS.fetch_add(1, Relaxed);
    array
        .element(slot, sweather_per_province::STRIDE)
        .and_then(|record| record.checked_add(sweather_per_province::MODIFIERS))
        .and_then(|accumulator| {
            let mut values = [engine::modifier_accumulator::Value::default(); 5];
            engine::modifier_accumulator::lookup_many_at(
                memory,
                accumulator,
                &PROVINCE_MODIFIER_KEYS,
                &mut values,
            )
            .ok()
            .map(|()| values.map(|value| value.present.then_some(value.raw)))
        })
}

/// [`WeatherJob::province_modifiers`] of every province record: the window's
/// provinces afresh and the rest carried when the carry allows it, otherwise
/// every province. The weather window pass rebuilds a province record's modifier
/// with the record, so the record carry's writers are the modifier's: an hour whose
/// province records did not follow their window (a record difference among them)
/// reads every province's entries too, and a carried entry difference is explained
/// or poisons the carry as a record difference does, and that hour reads every
/// province. Provinces beyond the hour's storage are refused and counted.
pub(super) fn capture_province_modifiers(memory: &impl Memory, job: &mut WeatherJob) {
    let begin = qpc();
    capture_province_modifiers_timed(memory, job);
    MODIFIER_TICKS.fetch_add(qpc().saturating_sub(begin).max(0) as u64, Relaxed);
}

fn capture_province_modifiers_timed(memory: &impl Memory, job: &mut WeatherJob) {
    let Some(array) = job.provinces.array else {
        return;
    };
    let count = array.count as usize;
    if job.worker_growth && job.province_modifiers.capacity() < count {
        let _ = engine::allocation::reserve_exact(&mut job.province_modifiers, count)
            .map_err(crate::allocation::io);
    }
    let room = count.min(job.province_modifiers.capacity());
    // Provinces beyond the storage are refused, each counted.
    MODIFIER_REFUSALS.fetch_add((count - room) as u64, Relaxed);
    if let Some(carry) = job.carry.as_deref_mut()
        && carry.provinces_followed
        && let Some(window) = carry.window(Part::Provinces)
        && carry.province_modifiers.len() == count
        && follow_province_modifiers(memory, array, carry, window)
    {
        job.province_modifiers
            .extend_from_slice(&carry.province_modifiers[..room]);
        MODIFIER_WINDOW_HOURS.fetch_add(1, Relaxed);
        return;
    }
    MODIFIER_FULL_HOURS.fetch_add(1, Relaxed);
    for slot in 0..room {
        let read = province_modifiers(memory, array, slot as u32);
        if read.is_none() {
            MODIFIER_REFUSALS.fetch_add(1, Relaxed);
        }
        job.province_modifiers.push(read);
    }
    if let Some(carry) = job.carry.as_deref_mut() {
        carry.province_modifiers.clear();
        if room == count
            && job.province_modifiers.iter().all(Option::is_some)
            && (carry.province_modifiers.capacity() >= count
                || engine::allocation::reserve_exact(&mut carry.province_modifiers, count).is_ok())
        {
            carry
                .province_modifiers
                .extend_from_slice(&job.province_modifiers[..count]);
        }
    }
}

/// The window's province modifier entries into the carried ones, after
/// comparing the hour's carried slots; false on a difference or a refusal.
fn follow_province_modifiers(
    memory: &impl Memory,
    array: engine::cpdx_array::Array,
    carry: &mut WeatherCarry,
    (first, visits): (u32, u32),
) -> bool {
    let count = array.count;
    let (start, step) = compared(carry.hour, carry.every);
    let (mut verified, mut mismatches) = (0, 0);
    let mut slot = start;
    let begin = qpc();
    while slot < count {
        if !in_window(slot, (first, visits), count) {
            let Some(fresh) = province_modifiers(memory, array, slot) else {
                SAMPLED.record(verified, mismatches, 3);
                return false;
            };
            verified += 1;
            mismatches += u64::from(Some(fresh) != carry.province_modifiers[slot as usize]);
        }
        slot += step;
    }
    MODIFIER_COMPARE_TICKS.fetch_add(qpc().saturating_sub(begin).max(0) as u64, Relaxed);
    SAMPLED.record(verified, mismatches, 3);
    if mismatches != 0 {
        MODIFIER_DIFFERENCES.fetch_add(mismatches, Relaxed);
        carry.differed();
        return false;
    }
    for visit in 0..visits {
        let slot = (first + visit) % count;
        let Some(fresh) = province_modifiers(memory, array, slot) else {
            return false;
        };
        carry.province_modifiers[slot as usize] = Some(fresh);
    }
    true
}

#[cfg(test)]
mod tests;
