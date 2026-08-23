//! The d13 change-signal gate: the per-manager dirty word the hourly
//! technology bracket reads passively.
//!
//! WHY THIS EXISTS. Domain d13's witness walk (one guarded read per
//! CTechnology instance, ~254k visits per hour) is the bundle's longest
//! sub-read. QN's census (runs/qn_d13_census) proved there is no passive
//! change word for [CTechnology]+0x170: exactly six writer sites exist and
//! none publishes a comparable residue. The owner's ruling of 2026-08-25
//! therefore authorises the hook-based change gate: five entry detours (the
//! five runtime writers; the constructor is dominated, see hooks) each mark
//! ONE manager in this table, and the walk consults it per country.
//!
//! THE STATE MODEL - a two-window dirty pair per known manager:
//!
//! Each slot holds two bits and the module holds PHASE in {0,1}. A mark
//! always sets the CURRENT window's bit; open_hour_window flips the phase
//! once per bracket. Consequences:
//!
//! * probe answers "witnessed unchanged" ONLY on an existing entry with
//!   BOTH bits clear while every global gate stands open. The previous
//!   window's bit set means "a write raced or followed last hour's copy" -
//!   measure now; the current window's bit means "already written this
//!   hour" - measure.
//! * note_witness_captured (successful witness read) consumes ONLY the
//!   previous window's bit: the bytes just captured supersede every mark
//!   outstanding when this hour opened. Marks landing during or after this
//!   walk stay in the current bit and force exactly one more measurement
//!   next hour. Every mark therefore forces at least one post-mark
//!   measurement and then dies - no stickiness, no lost updates, and no
//!   ordering ambiguity between the hook's entry-mark and the body store.
//!
//! Every ambiguous state (miss, foreign key, overflow, unreadable manager,
//! baseline owed, closed hour, hooks absent) degrades to MEASURE - the
//! full-copy fallback the ruling requires to be named rather than silent.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// Table capacity. One tech-status manager exists per country; production
/// rosters are a few hundred entries and the engine's tag space bounds them
/// far below this table. Pressure beyond it is not a correctness risk: the
/// mark path latches OVERFLOW and every probe then answers MEASURE.
pub const SLOTS: usize = 4096;
const MASK: u64 = (SLOTS - 1) as u64;

/// Linear-probe length, matching the house pattern (a15_counter_ui_hook
/// probes 16 of 65 536 slots for the same reason).
pub const PROBES: usize = 16;

const BIT0: u64 = 1;
const BIT1: u64 = 2;
const BOTH_BITS: u64 = BIT0 | BIT1;

struct Slot {
    /// The tech-status manager pointer. Claimed exactly once by a CAS from
    /// zero and never rewritten or removed afterwards.
    key: AtomicU64,
    flags: AtomicU64,
}

impl Slot {
    const fn new() -> Self {
        Self {
            key: AtomicU64::new(0),
            flags: AtomicU64::new(0),
        }
    }
}

static TABLE: [Slot; SLOTS] = [const { Slot::new() }; SLOTS];

/// Which window's bit a mark sets. Flipped once per bracket by
/// open_hour_window.
static PHASE: AtomicU32 = AtomicU32::new(0);

/// Latched when a mark could not claim a slot within PROBES. Cleared by the
/// next open_hour_window, which reports it as that hour's named full-copy
/// reason before resetting it.
static OVERFLOW: AtomicBool = AtomicBool::new(false);

/// Set by reset_for_install and cleared by note_baseline_seeded. While it
/// stands, the delta tracker has no pinned baseline, so a skipped country
/// would publish silence about state the fold does not hold yet; the gate
/// stays closed and every country measures.
static BASELINE_PENDING: AtomicBool = AtomicBool::new(false);

/// HOOK SIDE (W5 only): the owner-pointer load failed. Countdown in window
/// boundaries - 1 = failed this window: window A names it and arms the
/// countdown (the body's store may land after this hour's copy, so one more
/// named full-copy hour is required before skipping is sound again); 2 =
/// armed: window B names it again and clears; 0 = clear. A failure while
/// armed RESTARTS the countdown from 1. An AtomicU32 rather than a bool
/// because a single-window latch has the same unsafe boundary handoff as a
/// mark cleared too early (Codex r3 finding 4).
static MANAGER_READ_FAILED: AtomicU32 = AtomicU32::new(0);

/// Set for the remainder of an hour whose opening conditions already forbid
/// skipping (hooks absent, baseline owed, overflow latched, unreadable
/// managers latched, or the producer's baseline-withholding status flags
/// standing). Cleared by the next open_hour_window. One chokepoint so the
/// walk cannot disagree with the reason the hour was opened under.
static HOUR_CLOSED: AtomicBool = AtomicBool::new(false);

static MARKS_TOTAL: AtomicU64 = AtomicU64::new(0);
static OVERFLOW_EVENTS: AtomicU64 = AtomicU64::new(0);

fn hash(mgr: u64) -> u64 {
    (mgr.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 24) & MASK
}

fn slot_at(mgr: u64, probe: usize) -> &'static Slot {
    &TABLE[((hash(mgr) + probe as u64) & MASK) as usize]
}

fn current_bit() -> u64 {
    match PHASE.load(Ordering::Acquire) & 1 {
        0 => BIT0,
        _ => BIT1,
    }
}

fn previous_bit() -> u64 {
    match PHASE.load(Ordering::Acquire) & 1 {
        0 => BIT1,
        _ => BIT0,
    }
}

/// Which reason forces a FULL copy this hour, if any. Named on the
/// diagnostics channel rather than silently absorbed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FullCopyReason {
    None,
    HooksNotInstalled,
    BaselinePending,
    SlotOverflow,
    ManagerUnreadable,
}

impl FullCopyReason {
    pub fn as_str(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::HooksNotInstalled => Some("d13_change_gate_hooks_not_installed"),
            Self::BaselinePending => Some("d13_change_gate_baseline_pending"),
            Self::SlotOverflow => Some("d13_change_gate_slot_overflow"),
            Self::ManagerUnreadable => Some("d13_change_gate_manager_unreadable"),
        }
    }

    pub fn gate_open(self) -> bool {
        self == Self::None
    }
}

/// Called ONCE per d13 bracket, before the country walk. Flips the mark
/// window, reports why the whole domain must be measured this hour if
/// anything forces it, clears the previous hour's close override, and resets
/// the transient latches so pressure that has passed stops costing hours.
pub fn open_hour_window(hooks_installed: bool) -> FullCopyReason {
    HOUR_CLOSED.store(false, Ordering::Release);
    PHASE.fetch_xor(1, Ordering::AcqRel);
    let overflow = OVERFLOW.swap(false, Ordering::AcqRel);
    if overflow {
        OVERFLOW_EVENTS.fetch_add(1, Ordering::Relaxed);
    }
    // Two-window countdown: 1 -> 2 (named this window), 2 -> 0 (sound to
    // skip again). Advanced via CAS so a concurrent W5 restart (which stores
    // 1) can never be erased by an opener's blind store: the opener either
    // advances the exact value it saw or loops and names the restarted
    // countdown instead (Codex r4/r7).
    let unreadable = loop {
        match MANAGER_READ_FAILED.load(Ordering::Acquire) {
            0 => {
                // A restart may land between load and CAS; the CAS failing
                // means the latch is live again - loop and name it.
                if MANAGER_READ_FAILED
                    .compare_exchange(0, 0, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    break false;
                }
            }
            1 => {
                if MANAGER_READ_FAILED
                    .compare_exchange(1, 2, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    break true;
                }
            }
            _ => {
                if MANAGER_READ_FAILED
                    .compare_exchange(2, 0, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    break true;
                }
            }
        }
    };
    let reason = if !hooks_installed {
        FullCopyReason::HooksNotInstalled
    } else if BASELINE_PENDING.load(Ordering::Acquire) {
        FullCopyReason::BaselinePending
    } else if unreadable {
        FullCopyReason::ManagerUnreadable
    } else if overflow {
        FullCopyReason::SlotOverflow
    } else {
        FullCopyReason::None
    };
    if !reason.gate_open() {
        HOUR_CLOSED.store(true, Ordering::Release);
    }
    reason
}

/// The tech child closes the hour by name when a condition the probe cannot
/// see on its own stands - today: the producer's baseline-withholding status
/// flags, which force the packer down a whole-payload arm no matter what the
/// walk measured. A closed hour answers MEASURE everywhere.
pub fn close_hour() {
    HOUR_CLOSED.store(true, Ordering::Release);
}

/// Whether the witness gate may skip at all right now. Kept for callers that
/// want the predicate without a probe; the authoritative chokepoint is
/// probe_witness_clean itself, which consults every gate below.
pub fn gate_open(hooks_installed: bool) -> bool {
    hooks_installed
        && !BASELINE_PENDING.load(Ordering::Acquire)
        && !HOUR_CLOSED.load(Ordering::Acquire)
}

/// HOOK BODY. Records that some writer of [CTechnology]+0x170 fired for this
/// manager. Lock-free, allocation-free, logging-free: one acquire load and
/// one locked OR on the hot path (entry already present), a CAS claim on
/// first touch.
pub fn mark(mgr: u64) {
    if mgr == 0 {
        return;
    }
    let bit = current_bit();
    for probe in 0..PROBES {
        let slot = slot_at(mgr, probe);
        if slot.key.load(Ordering::Acquire) == mgr {
            slot.flags.fetch_or(bit, Ordering::AcqRel);
            MARKS_TOTAL.fetch_add(1, Ordering::Relaxed);
            return;
        }
    }
    for probe in 0..PROBES {
        let slot = slot_at(mgr, probe);
        match slot
            .key
            .compare_exchange(0, mgr, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => {
                slot.flags.fetch_or(bit, Ordering::AcqRel);
                MARKS_TOTAL.fetch_add(1, Ordering::Relaxed);
                return;
            }
            Err(current) if current == mgr => {
                slot.flags.fetch_or(bit, Ordering::AcqRel);
                MARKS_TOTAL.fetch_add(1, Ordering::Relaxed);
                return;
            }
            Err(_) => continue,
        }
    }
    // No free slot within the probe window. This is table pressure, never a
    // silently lost mark that matters: probes answer MEASURE while the latch
    // stands, so the failure direction is extra copying, not missed change.
    OVERFLOW.store(true, Ordering::Release);
}

/// COLLECTOR SIDE. True ONLY when this manager provably has no hooked store
/// since its last committed publication AND every global gate stands open
/// (hooks installed, pin trusted, hour not closed, no overflow). Any miss,
/// any ambiguity, any closed gate answers false, which means: run the full
/// witness read.
pub fn probe_witness_clean(mgr: u64) -> bool {
    if mgr == 0
        || HOUR_CLOSED.load(Ordering::Acquire)
        || MANAGER_READ_FAILED.load(Ordering::Acquire) != 0
        || OVERFLOW.load(Ordering::Acquire)
        || BASELINE_PENDING.load(Ordering::Acquire)
    {
        return false;
    }
    for probe in 0..PROBES {
        let slot = slot_at(mgr, probe);
        match slot.key.load(Ordering::Acquire) {
            0 => {}
            key if key == mgr => {
                return slot.flags.load(Ordering::Acquire) & BOTH_BITS == 0;
            }
            _ => {}
        }
    }
    false
}

/// COLLECTOR SIDE, after a successful full witness read. Seeds the entry on
/// first sighting and consumes ONLY the previous window's bit: the bytes
/// just captured supersede every mark outstanding when this hour opened,
/// while marks landing during or after this walk keep the current bit set
/// and force exactly one more measurement next hour. Never blocks anything.
pub fn note_witness_captured(mgr: u64) {
    if mgr == 0 {
        return;
    }
    let consume = previous_bit();
    for probe in 0..PROBES {
        let slot = slot_at(mgr, probe);
        if slot.key.load(Ordering::Acquire) == mgr {
            loop {
                let flags = slot.flags.load(Ordering::Acquire);
                if flags & consume == 0 {
                    return;
                }
                match slot.flags.compare_exchange(
                    flags,
                    flags & !consume,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => return,
                    Err(_) => continue,
                }
            }
        }
    }
    for probe in 0..PROBES {
        let slot = slot_at(mgr, probe);
        match slot.key.compare_exchange(0, mgr, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return,
            Err(current) if current == mgr => return,
            Err(_) => continue,
        }
    }
    OVERFLOW.store(true, Ordering::Release);
}

/// The collector acknowledges a successfully emitted payload whose pin the
/// fold can trust: the first payload ever (the baseline), and every patch
/// after it. Before the first, the fold holds no d13 state at all, so
/// skipping would be silence about state nobody holds.
pub fn note_baseline_seeded() {
    BASELINE_PENDING.store(false, Ordering::Release);
}

/// The packer took a WHOLE-payload arm (withheld baseline, repeated key,
/// pinned key off the roster) and rebuilt its pin from this hour's measured
/// set alone. Skipped countries' bytes are in NO pin now, so their
/// witnessed-unchanged answers would reference state the producer no longer
/// holds: re-open the baseline obligation and make the next hour measure
/// everyone until a successful payload re-seeds it.
pub fn note_pin_lost() {
    BASELINE_PENDING.store(true, Ordering::Release);
}

/// HOOK SIDE (W5 only): the owner-pointer load failed. Never stall the game;
/// latch the named ambiguity so every probe answers MEASURE for the next two
/// windows (a failure while already counting restarts both windows).
pub fn latch_manager_read_failed() {
    MANAGER_READ_FAILED.fetch_max(1, Ordering::Release);
}

/// Re-arm the countdown even while it is mid-countdown (overlapping W5
/// failures): the newest failure deserves its own two named windows.
pub fn restart_manager_read_failed() {
    MANAGER_READ_FAILED.store(1, Ordering::Release);
}

/// Install/uninstall reset. A fresh capture starts with a fresh table and a
/// fresh baseline obligation; stale entries from a previous capture in the
/// same process must never answer probes for the new one.
pub fn reset_for_install() {
    for slot in TABLE.iter() {
        slot.key.store(0, Ordering::Release);
        slot.flags.store(0, Ordering::Release);
    }
    OVERFLOW.store(false, Ordering::Release);
    MANAGER_READ_FAILED.store(0, Ordering::Release);
    BASELINE_PENDING.store(true, Ordering::Release);
    PHASE.store(0, Ordering::Release);
    HOUR_CLOSED.store(false, Ordering::Release);
    MARKS_TOTAL.store(0, Ordering::Relaxed);
    OVERFLOW_EVENTS.store(0, Ordering::Relaxed);
}

pub fn marks_total() -> u64 {
    MARKS_TOTAL.load(Ordering::Relaxed)
}

pub fn overflow_events() -> u64 {
    OVERFLOW_EVENTS.load(Ordering::Relaxed)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialises every unit test in this module: the table and latches are
    /// process globals and the default harness runs tests concurrently, so
    /// one test's reset could erase another test's marks mid-flight (Codex
    /// r2 finding 5). Hold the guard for the whole test body.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    /// Fresh state, then open window 1 (the phase flips to 1).
    fn fresh() {
        reset_for_install();
        note_baseline_seeded();
        hour();
    }

    fn hour() -> FullCopyReason {
        open_hour_window(true)
    }

    #[test]
    fn a_fresh_table_answers_measure_for_every_manager() {
        // kills: a probe treating absence-of-entry as witnessed-clean.
        let _g = lock();
        fresh();
        assert!(!probe_witness_clean(0x1000));
    }

    #[test]
    fn a_marked_manager_measures_then_settles_after_one_extra_hour() {
        // kills: lost-mark AND sticky-dirty. One mark costs its own hour's
        // read plus exactly one conservative re-read, then settles clean.
        let _g = lock();
        fresh();
        mark(0x2000);
        assert!(!probe_witness_clean(0x2000), "this hour must measure");
        note_witness_captured(0x2000); // consumes LAST window's bit only
        assert!(!probe_witness_clean(0x2000), "same-window mark survives");
        hour();
        assert!(!probe_witness_clean(0x2000), "next hour re-measures once");
        note_witness_captured(0x2000);
        hour();
        assert!(probe_witness_clean(0x2000), "then it settles clean");
    }

    #[test]
    fn a_write_landing_in_a_skip_hour_is_measured_the_next_hour() {
        // kills: THE hard stop - a write racing a SKIP decision surfaces as
        // the very next hour's measurement.
        let _g = lock();
        fresh();
        note_witness_captured(0x3000); // measured clean in window 1
        hour(); // window 2: clean -> SKIPPED
        assert!(probe_witness_clean(0x3000));
        mark(0x3000); // engine writes during this skipped hour
        hour(); // window 3: measures (the write lands here)
        assert!(
            !probe_witness_clean(0x3000),
            "the skipped-hour write lands now"
        );
        note_witness_captured(0x3000);
        hour(); // window 4: one measurement suffices
        assert!(probe_witness_clean(0x3000));
    }

    #[test]
    fn a_refused_read_keeps_its_marks() {
        // kills: clearing marks on paths that never published. A refused
        // witness read performs NO capture; bits stand until success, then
        // the usual conservative re-check runs once before settling.
        let _g = lock();
        fresh();
        mark(0x4000);
        hour();
        assert!(!probe_witness_clean(0x4000));
        hour();
        assert!(!probe_witness_clean(0x4000));
        note_witness_captured(0x4000);
        hour();
        assert!(!probe_witness_clean(0x4000));
        note_witness_captured(0x4000);
        hour();
        assert!(probe_witness_clean(0x4000));
    }

    #[test]
    fn baseline_pending_keeps_the_gate_shut() {
        // kills: skipping during the seeding hour.
        let _g = lock();
        reset_for_install();
        assert_eq!(open_hour_window(true), FullCopyReason::BaselinePending);
        assert!(!gate_open(true));
        assert!(!probe_witness_clean(0x5000));
        note_baseline_seeded();
        assert_eq!(open_hour_window(true), FullCopyReason::None);
        assert!(gate_open(true));
    }

    #[test]
    fn uninstalled_hooks_keep_the_gate_shut() {
        // kills: probing with detours absent.
        let _g = lock();
        fresh();
        assert_eq!(
            open_hour_window(false),
            FullCopyReason::HooksNotInstalled
        );
        assert!(!gate_open(false));
        assert!(!probe_witness_clean(0x6000));
    }

    #[test]
    fn slot_pressure_degrades_to_a_named_full_copy() {
        // kills: an overflow quietly dropping marks.
        let _g = lock();
        fresh();
        for index in 0..PROBES as u64 {
            let slot = slot_at(0x7000, index as usize);
            assert!(slot
                .key
                .compare_exchange(0, 0x9000 + index, Ordering::AcqRel, Ordering::Acquire)
                .is_ok());
        }
        mark(0x7000);
        assert!(OVERFLOW.load(Ordering::Acquire));
        assert!(!probe_witness_clean(0x7000));
        assert_eq!(open_hour_window(true), FullCopyReason::SlotOverflow);
        assert!(!OVERFLOW.load(Ordering::Acquire));
    }

    #[test]
    fn claims_never_steal_another_manager_key() {
        // kills: claim stealing across probe windows.
        let _g = lock();
        fresh();
        slot_at(0xB000, 3).key.store(0xC000, Ordering::Release);
        mark(0xB000);
        assert_eq!(slot_at(0xB000, 3).key.load(Ordering::Acquire), 0xC000);
        let claimed = (0..PROBES)
            .map(|p| slot_at(0xB000, p).key.load(Ordering::Acquire))
            .position(|k| k == 0xB000);
        assert_eq!(claimed, Some(0));
    }

    #[test]
    fn zero_pointers_are_ignored_everywhere() {
        let _g = lock();
        fresh();
        mark(0);
        assert!(!probe_witness_clean(0));
        note_witness_captured(0);
    }

    #[test]
    fn losing_the_pin_reopens_the_baseline_obligation() {
        // kills: skipping against a pin nobody holds.
        let _g = lock();
        fresh();
        note_pin_lost();
        assert!(!probe_witness_clean(0xD000));
        assert_eq!(open_hour_window(true), FullCopyReason::BaselinePending);
        note_baseline_seeded();
        assert_eq!(open_hour_window(true), FullCopyReason::None);
        assert!(gate_open(true));
    }

    #[test]
    fn an_unreadable_manager_pointer_covers_two_windows() {
        // kills: clearing the ambiguity latch in the very next window. The
        // write whose manager was unreadable lands AFTER the latch is set;
        // window A names it and keeps full-copy, window B still measures
        // (the store may land during A's copy), and only after B does the
        // countdown reach zero (Codex r3 finding 4).
        let _g = lock();
        reset_for_install();
        note_baseline_seeded();
        latch_manager_read_failed();
        assert!(!probe_witness_clean(0xA000));
        // Window A opens: the countdown 1 is named and advances to 2.
        assert_eq!(hour(), FullCopyReason::ManagerUnreadable);
        assert!(!probe_witness_clean(0xA000), "window B still measures");
        // Window B opens: the countdown 2 is named again and clears.
        assert_eq!(hour(), FullCopyReason::ManagerUnreadable);
        assert!(
            !probe_witness_clean(0xA000),
            "window C: countdown already consumed, but this window still              measures (it opened while the latch stood)"
        );
        note_witness_captured(0xA000); // window C's quiet read
        // Window D opens: countdown consumed AND a successful capture has
        // superseded it - only now is a skip sound.
        assert_eq!(hour(), FullCopyReason::None);
        assert!(probe_witness_clean(0xA000));
    }
}
