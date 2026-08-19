//! Same reason the release ledger is here. Freezes are taken from seven entry
//! suspended thread may already own with nobody left to release it"*. The
//! The ordering argument is then only about publication, not about deadlock:
//! each slot's `ready` flag is stored with `Release` after its fields and read
//! with `Acquire` before them, so a reader either sees a whole row or no row.

use std::sync::atomic::{AtomicPtr, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

/// the successful call graph, and teardown releases each installed group again
pub const CENSUS_CAPACITY: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum FreezeEntryPoint {
    Direct = 0,
    Enable = 1,
    ApplyQueued = 2,
    EnableBatch = 3,
    Disable = 4,
    DisableBatch = 5,
    Remove = 6,
    RemoveBatch = 7,
}

impl FreezeEntryPoint {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "freeze_process",
            Self::Enable => "enable",
            Self::ApplyQueued => "apply_queued",
            Self::EnableBatch => "enable_batch",
            Self::Disable => "disable",
            Self::DisableBatch => "disable_batch",
            Self::Remove => "remove",
            Self::RemoveBatch => "remove_batch",
        }
    }

    const fn code(self) -> u32 {
        self as u32
    }

    fn from_code(code: u32) -> Option<Self> {
        Some(match code {
            0 => Self::Direct,
            1 => Self::Enable,
            2 => Self::ApplyQueued,
            3 => Self::EnableBatch,
            4 => Self::Disable,
            5 => Self::DisableBatch,
            6 => Self::Remove,
            7 => Self::RemoveBatch,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FreezeWindow {
    pub ordinal: u64,
    /// A refused row carries no `minhook_us` and no `acquire_us`: there was no
    pub refused: bool,
    pub entry_point: Option<FreezeEntryPoint>,
    pub label: &'static str,
    pub target_count: u32,
    pub window_us: u32,
    pub acquire_us: u32,
    pub minhook_us: u32,
    pub passes: u32,
}

impl FreezeWindow {
    pub fn entry_point_name(&self) -> &'static str {
        self.entry_point
            .map(FreezeEntryPoint::as_str)
            .unwrap_or("unknown")
    }
}

struct Slot {
    /// Zero until every field below has been stored. Written with `Release` and
    /// read with `Acquire`, which is the whole of the publication protocol.
    ready: AtomicU32,
    refused: AtomicU32,
    ordinal: AtomicU64,
    entry_point: AtomicU32,
    label_ptr: AtomicPtr<u8>,
    label_len: AtomicUsize,
    target_count: AtomicU32,
    window_us: AtomicU32,
    acquire_us: AtomicU32,
    minhook_us: AtomicU32,
    passes: AtomicU32,
}

impl Slot {
    const NEW: Self = Self {
        ready: AtomicU32::new(0),
        refused: AtomicU32::new(0),
        ordinal: AtomicU64::new(0),
        entry_point: AtomicU32::new(0),
        label_ptr: AtomicPtr::new(core::ptr::null_mut()),
        label_len: AtomicUsize::new(0),
        target_count: AtomicU32::new(0),
        window_us: AtomicU32::new(0),
        acquire_us: AtomicU32::new(0),
        minhook_us: AtomicU32::new(0),
        passes: AtomicU32::new(0),
    };
}

static SLOTS: [Slot; CENSUS_CAPACITY] = [Slot::NEW; CENSUS_CAPACITY];
static CURSOR: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn record(window: FreezeWindow) {
    let index = CURSOR.fetch_add(1, Ordering::AcqRel);
    let Some(slot) = SLOTS.get(index) else {
        return;
    };
    slot.ordinal.store(window.ordinal, Ordering::Relaxed);
    slot.refused
        .store(u32::from(window.refused), Ordering::Relaxed);
    slot.entry_point.store(
        window
            .entry_point
            .map(FreezeEntryPoint::code)
            .unwrap_or(u32::MAX),
        Ordering::Relaxed,
    );
    slot.label_ptr
        .store(window.label.as_ptr().cast_mut(), Ordering::Relaxed);
    slot.label_len.store(window.label.len(), Ordering::Relaxed);
    slot.target_count
        .store(window.target_count, Ordering::Relaxed);
    slot.window_us.store(window.window_us, Ordering::Relaxed);
    slot.acquire_us.store(window.acquire_us, Ordering::Relaxed);
    slot.minhook_us.store(window.minhook_us, Ordering::Relaxed);
    slot.passes.store(window.passes, Ordering::Relaxed);
    slot.ready.store(1, Ordering::Release);
}

pub fn freeze_windows() -> Vec<FreezeWindow> {
    let claimed = CURSOR.load(Ordering::Acquire).min(CENSUS_CAPACITY);
    let mut rows = Vec::with_capacity(claimed);
    for slot in &SLOTS[..claimed] {
        if slot.ready.load(Ordering::Acquire) == 0 {
            continue;
        }
        let label_ptr = slot.label_ptr.load(Ordering::Relaxed);
        let label_len = slot.label_len.load(Ordering::Relaxed);
        let label: &'static str = if label_ptr.is_null() || label_len == 0 {
            ""
        } else {
            unsafe {
                core::str::from_utf8_unchecked(core::slice::from_raw_parts(label_ptr, label_len))
            }
        };
        rows.push(FreezeWindow {
            ordinal: slot.ordinal.load(Ordering::Relaxed),
            refused: slot.refused.load(Ordering::Relaxed) != 0,
            entry_point: FreezeEntryPoint::from_code(slot.entry_point.load(Ordering::Relaxed)),
            label,
            target_count: slot.target_count.load(Ordering::Relaxed),
            window_us: slot.window_us.load(Ordering::Relaxed),
            acquire_us: slot.acquire_us.load(Ordering::Relaxed),
            minhook_us: slot.minhook_us.load(Ordering::Relaxed),
            passes: slot.passes.load(Ordering::Relaxed),
        });
    }
    rows
}

pub fn freeze_windows_dropped() -> u64 {
    windows_dropped_from_cursor(CURSOR.load(Ordering::Acquire))
}

const fn windows_dropped_from_cursor(claimed: usize) -> u64 {
    claimed.saturating_sub(CENSUS_CAPACITY) as u64
}

pub fn freeze_windows_recorded() -> u64 {
    CURSOR.load(Ordering::Acquire) as u64
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct FreezeTiming {
    ordinal: u64,
    started: Instant,
    acquired: Instant,
    entry_point: FreezeEntryPoint,
    label: &'static str,
    target_count: u32,
    minhook_us: u32,
}

impl FreezeTiming {
    /// `acquired` at the moment the freeze is established.
    pub(crate) fn new(ordinal: u64, started: Instant, acquired: Instant) -> Self {
        Self {
            ordinal,
            started,
            acquired,
            entry_point: FreezeEntryPoint::Direct,
            label: "",
            target_count: 0,
            minhook_us: 0,
        }
    }

    pub(crate) fn note_entry(
        &mut self,
        entry_point: FreezeEntryPoint,
        label: &'static str,
        target_count: u32,
    ) {
        self.entry_point = entry_point;
        self.label = label;
        self.target_count = target_count;
    }

    pub(crate) fn note_minhook(&mut self, span: std::time::Duration) {
        self.minhook_us = saturating_micros(span);
    }

    pub(crate) fn close(self, resumed: Instant, passes: u32) -> FreezeWindow {
        let window_us = saturating_micros(resumed.saturating_duration_since(self.started));
        let acquire_us = saturating_micros(self.acquired.saturating_duration_since(self.started));
        FreezeWindow {
            ordinal: self.ordinal,
            refused: false,
            entry_point: Some(self.entry_point),
            label: self.label,
            target_count: self.target_count,
            window_us,
            // saturate `window_us` can carry an `acquire_us` that also saturates
            acquire_us: acquire_us.min(window_us),
            minhook_us: self
                .minhook_us
                .min(window_us.saturating_sub(acquire_us.min(window_us))),
            passes,
        }
    }
}

pub(crate) fn refused_window(ordinal: u64, started: Instant, ended: Instant) -> FreezeWindow {
    FreezeWindow {
        ordinal,
        refused: true,
        entry_point: None,
        label: "",
        target_count: 0,
        window_us: saturating_micros(ended.saturating_duration_since(started)),
        acquire_us: 0,
        minhook_us: 0,
        passes: 0,
    }
}

fn saturating_micros(span: std::time::Duration) -> u32 {
    u32::try_from(span.as_micros()).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absurd_span_saturates_rather_than_wrapping() {
        assert_eq!(
            saturating_micros(std::time::Duration::from_secs(u64::from(u32::MAX))),
            u32::MAX
        );
        assert_eq!(saturating_micros(std::time::Duration::from_micros(7)), 7);
    }

    #[test]
    fn every_entry_point_survives_the_round_trip_through_a_slot() {
        for entry in [
            FreezeEntryPoint::Direct,
            FreezeEntryPoint::Enable,
            FreezeEntryPoint::ApplyQueued,
            FreezeEntryPoint::EnableBatch,
            FreezeEntryPoint::Disable,
            FreezeEntryPoint::DisableBatch,
            FreezeEntryPoint::Remove,
            FreezeEntryPoint::RemoveBatch,
        ] {
            assert_eq!(FreezeEntryPoint::from_code(entry.code()), Some(entry));
        }
        assert_eq!(FreezeEntryPoint::from_code(u32::MAX), None);
    }

    #[test]
    fn the_dropped_count_is_the_overflow_of_the_cursor() {
        assert_eq!(windows_dropped_from_cursor(0), 0);
        assert_eq!(windows_dropped_from_cursor(CENSUS_CAPACITY - 1), 0);
        assert_eq!(windows_dropped_from_cursor(CENSUS_CAPACITY), 0);
        assert_eq!(windows_dropped_from_cursor(CENSUS_CAPACITY + 1), 1);
        assert_eq!(
            windows_dropped_from_cursor(CENSUS_CAPACITY + 9_999),
            9_999,
            "a census that overflowed by ten thousand must say ten thousand"
        );
        assert_eq!(
            freeze_windows_dropped(),
            windows_dropped_from_cursor(freeze_windows_recorded() as usize)
        );
        assert!(freeze_windows().len() <= CENSUS_CAPACITY);
    }

    #[test]
    fn a_saturated_row_never_reports_parts_larger_than_its_whole() {
        let start = Instant::now();
        let mut timing = FreezeTiming::new(1, start, start + std::time::Duration::from_secs(9_000));
        timing.note_minhook(std::time::Duration::from_secs(9_000));
        let row = timing.close(start + std::time::Duration::from_secs(1), 2);
        assert!(
            row.acquire_us <= row.window_us,
            "acquire {} exceeds window {}",
            row.acquire_us,
            row.window_us
        );
        assert!(
            row.acquire_us.saturating_add(row.minhook_us) <= row.window_us,
            "acquire {} + minhook {} exceeds window {}",
            row.acquire_us,
            row.minhook_us,
            row.window_us
        );
    }

    #[test]
    fn a_refused_attempt_still_produces_a_row_that_says_it_was_refused() {
        let start = Instant::now();
        let row = refused_window(7, start, start + std::time::Duration::from_millis(3));
        assert!(row.refused);
        assert_eq!(row.ordinal, 7);
        assert_eq!(row.window_us, 3_000);
        assert_eq!(row.acquire_us, 0, "a refusal never acquired anything");
        assert_eq!(row.minhook_us, 0, "a refusal never reached MinHook");
        assert_eq!(row.entry_point, None);
    }
}
