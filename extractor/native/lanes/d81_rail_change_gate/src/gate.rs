//! d81's RAIL WRITE-TAP: the write witness the hourly rail bracket reads
//! passively, and the three detours that maintain it.
//!
//! WHY IT EXISTS. Lane TD's dossier (`runs/td_rail_change_signal/DOSSIER.md`)
//! searched for a passive change signal for d81's rail geometry and FOUND
//! NONE: `CProvinceRailwayInfo::SetRailway` writes the level IN PLACE at a
//! stable address inside a stable allocation reached through a table whose base
//! never moves (§4, M2a), the engine's own consumer re-compares the pair vector
//! element by element rather than reading a version word (§6.1), and the one
//! rail-specific notification the engine keeps is cleared by its own producer
//! inside a single update, so an hourly out-of-band reader cannot use it
//! (§6.2).
//!
//! TD'S OWN LIMIT, CARRIED RATHER THAN ROUNDED UP (Codex r1 finding 3). §6.6
//! of that dossier deliberately does NOT claim that no such signal exists
//! anywhere in the image, and it withdraws any claim that its writer census is
//! closed: it is a bounded syntactic census over a one-hop reach set. So the
//! justification here is "no signal was found by a whole-image search that
//! states its own gaps", not "none exists". That is what the owner's
//! minimise-hooks ruling (2026-08-24) asks for before a hook may be installed,
//! and the structural-perf ruling of 2026-08-27 approves write-tap hooks on
//! that footing. The residual is why every ambiguity below copies in full.
//!
//! # THE STATE MODEL: a seqlock witness, not a dirty bit
//!
//! Two words. `WRITE_EPOCH` is bumped by every detour on entry AND on exit;
//! `IN_FLIGHT` counts detours currently executing. A [`RailWitness`] is a
//! reading of the pair, and the consumer takes one BEFORE it copies the rail
//! families and one AFTER.
//!
//! The rule the whole design rests on:
//!
//! > A copy taken between witnesses `w0` and `w1` may be REMEMBERED only when
//! > `w0.epoch == w1.epoch` and both are QUIET. It may later be REUSED only
//! > while the epoch still equals `w1.epoch` and nothing is in flight.
//!
//! Why that is sound, case by case. A detour's entry bump happens before every
//! store in its body and its exit bump happens after all of them (locked RMWs,
//! `AcqRel`), so any writer that stored anything contributes at least one epoch
//! bump inside the interval its stores lie in.
//!
//!  * A writer that ran ENTIRELY before the copy: both its bumps precede `w0`,
//!    the epoch is stable across the copy, and its stores are IN the copy.
//!    Remembering is correct.
//!  * A writer that entered DURING the copy: its entry bump lands between `w0`
//!    and `w1`, so the epochs differ and nothing is remembered.
//!  * A writer that entered BEFORE the copy and was still running: `w0` is not
//!    quiet, so nothing is remembered — this is the case a dirty bit cannot
//!    see, because the bit it set may already have been consumed by an earlier
//!    bracket (Codex r2 finding 1).
//!  * A writer that ran ENTIRELY between the remembered copy and a later reuse:
//!    two bumps, so the epoch has moved and the reuse is refused.
//!  * A writer in flight AT the reuse: it bumped on entry, so the epoch has
//!    already moved. The `IN_FLIGHT` half of the reuse check is therefore
//!    redundant — and it is kept anyway, so the invariant is local to
//!    [`still_current`] instead of resting on an argument that spans two
//!    functions.
//!  * A writer that ran entirely WHILE A WITNESS WAS BEING TAKEN: the reading
//!    itself has to be atomic for any of the above to mean anything, and making
//!    it so is [`witness`]'s own problem rather than the state model's. It
//!    reads the epoch, the in-flight count and the epoch again, and only
//!    returns a reading whose two epochs agree (Codex r7). That reading is
//!    truthful for the instant of the MIDDLE load, which is the property the
//!    cases above need; the comment on `witness_probed` works it through,
//!    including why the weaker-looking claim is the right one (Codex r8).
//!
//! WHAT THIS REPLACED, and why. The first form of this gate was lane RO's
//! two-window dirty pair (d13). It is sound for a writer whose entry and exit
//! both fall inside one mark window, and unsound for one that spans two: the
//! bracket that consumed the entry mark could not know the writer was still
//! running, and the bracket after it saw both bits clear and reused a copy the
//! writer had since changed. The epoch witness has no such case, and it is
//! cheaper for the recorder too: a write that completes BETWEEN two brackets
//! costs one full copy, where the two-window model always cost two. A write
//! that SPANS a bracket costs that bracket -- which could not have been
//! remembered under either model -- plus one recovery bracket.
//!
//! Every ambiguous state (hooks absent, baseline owed, epoch moved, a writer in
//! flight) degrades to a full copy, which is the pre-tap behaviour and costs
//! only the guarded copies the tap exists to avoid.

use std::sync::atomic::{fence, AtomicBool, AtomicU64, Ordering};

/// Bumped twice by every hooked call: once before the body runs and once after
/// its stores are visible.
///
/// `u64` at two bumps per hooked call, against writers that fire on railway
/// construction: wrapping would need 2^63 railway events between two game
/// hours, so the comparison below is exact rather than probabilistic.
static WRITE_EPOCH: AtomicU64 = AtomicU64::new(0);

/// How many hooked detours are executing right now.
///
/// EVERY WAY THIS CAN GO WRONG IS FAIL-CLOSED, which is why it is a plain
/// counter and not something more careful. A `mark_exit` that never runs --
/// the only shape being an unwind out of the original, which these
/// straight-line `extern "system"` bodies do not otherwise admit -- leaks the
/// count UPWARDS, and a count that never returns to zero makes [`witness`]
/// permanently non-quiet, so the tap refuses forever and d81 runs the pre-tap
/// program. It can never go the other way: `mark_exit` is only ever reached
/// through a `mark_enter` on the same call, so the counter cannot underflow.
///
/// It also survives [`reset_for_install`] deliberately (see there).
static IN_FLIGHT: AtomicU64 = AtomicU64::new(0);

/// Set by [`reset_for_install`] and cleared by [`note_memory_seeded`]. While it
/// stands there is no remembered rail copy this session that the tap can vouch
/// for, so the gate stays shut and the reason is named rather than showing up
/// as an unexplained miss.
static BASELINE_PENDING: AtomicBool = AtomicBool::new(true);

static MARKS_TOTAL: AtomicU64 = AtomicU64::new(0);
static WINDOWS_OPENED: AtomicU64 = AtomicU64::new(0);

/// One reading of the tap: the write epoch, and whether any hooked detour was
/// executing.
///
/// The consumer stores it beside its remembered copy and hands it back; it
/// never reasons about the numbers itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RailWitness {
    pub epoch: u64,
    pub quiet: bool,
}

/// Which reason forces a FULL rail copy this bracket, if any.
///
/// The BIN and the wire name a reason is published under belong to the
/// collector, which matches this enum exhaustively: a reason added here does
/// not compile there until it is given one. This carried its own `as_str`
/// and `ordinal` until lane TV made that match exhaustive, at which point
/// nothing called either.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FullCopyReason {
    None,
    HooksNotInstalled,
    BaselinePending,
    RailWritten,
}


/// Called ONCE per d81 copy bracket, before anything is read. Reports the
/// reason the rail families must be copied in full this bracket if a GLOBAL
/// condition forces it; the per-copy currency question is [`still_current`].
pub fn open_hour_window(hooks_installed: bool) -> FullCopyReason {
    WINDOWS_OPENED.fetch_add(1, Ordering::Relaxed);
    if !hooks_installed {
        return FullCopyReason::HooksNotInstalled;
    }
    if BASELINE_PENDING.load(Ordering::Acquire) {
        return FullCopyReason::BaselinePending;
    }
    FullCopyReason::None
}

/// HOOK BODY, ENTRY. Bumped before the original runs, so every store the body
/// makes lies after this bump in the total order.
///
/// Three locked read-modify-writes (the `Relaxed` census still compiles to a
/// locked add on x86-64), and the caller adds its own removal-barrier count for
/// four at this boundary. No key, no memory read, no branch on a pointer, no
/// allocation, no logging, no lock: this runs on the game's own threads and
/// must never make the engine wait (zero-loss ruling).
///
/// [`marks_total`] counts MARK OPERATIONS, not calls: every detour marks at
/// entry and again at exit, so a completed hooked call contributes two.
///
/// ITS PARITY MEANS NOTHING and must not be read as an in-flight predicate
/// (Codex r3 finding 2): [`mark_exit`] lowers `IN_FLIGHT` BEFORE it bumps this
/// census, so a sample taken between those two reads odd while the gate is
/// perfectly quiet, and two concurrent callbacks read even while both are
/// inside. It is a `> 0` liveness sample -- "did the detours fire at all" --
/// and nothing in the production path reads it at all.
#[inline]
pub fn mark_enter() {
    IN_FLIGHT.fetch_add(1, Ordering::AcqRel);
    WRITE_EPOCH.fetch_add(1, Ordering::AcqRel);
    MARKS_TOTAL.fetch_add(1, Ordering::Relaxed);
}

/// HOOK BODY, EXIT. Bumped after the original returned, so every store the
/// body made lies before this bump.
///
/// THE ORDER IS THE MIRROR OF THE ENTRY'S and that is deliberate: the epoch
/// moves while the callback is still counted as in flight, so no reader can
/// ever observe "epoch settled AND nothing in flight" for an interval a writer
/// was still inside.
#[inline]
pub fn mark_exit() {
    WRITE_EPOCH.fetch_add(1, Ordering::AcqRel);
    IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
    MARKS_TOTAL.fetch_add(1, Ordering::Relaxed);
}

/// COLLECTOR SIDE. Read the tap.
///
/// SEQUENTIALLY CONSISTENT, and fenced, because the thing being bracketed is a
/// plain copy of engine memory that the compiler and the CPU are otherwise free
/// to move across an ordinary atomic load. The reader needs the copy's loads to
/// lie strictly between two readings of the epoch, which is the classic seqlock
/// reader shape. It runs twice per game hour, so being unambiguous here costs
/// nothing worth measuring.
pub fn witness() -> RailWitness {
    witness_probed(|| {})
}

/// How many times [`witness_probed`] will re-read a torn pair before it gives
/// up and reports a reading that can never be current.
///
/// A retry needs a hooked call to land inside the four instructions between the
/// two epoch loads. Eight of those in a row, with writers that fire on railway
/// construction, is not a thing that happens; the constant exists so the reader
/// has NO unbounded loop in it at all rather than because the retries are
/// expected to be used.
const WITNESS_ATTEMPTS: usize = 8;

/// [`witness`] with a probe that runs BETWEEN the two field reads.
///
/// The probe is `|| {}` in production and inlines away; it exists because the
/// interleaving this function has to survive cannot be reached from the outside
/// (see `a_writer_completing_between_the_two_field_reads_is_never_quiet_at_the_old_epoch`).
/// There is deliberately only ONE body, so the test and the recorder read the
/// tap through the same code.
#[inline]
fn witness_probed(mut between: impl FnMut()) -> RailWitness {
    // TWO EPOCH READS AROUND THE IN-FLIGHT READ, and this is a BLOCKING finding
    // paid for (Codex round 7). The earlier form read the epoch once and then
    // `IN_FLIGHT` once, and a whole writer fits between them: read epoch E,
    // writer enters and stores and exits (epoch now E+2), read in-flight 0. The
    // reading was `{E, quiet}` -- exactly what a genuinely idle moment at E
    // looks like -- so `still_current` accepted a remembered witness whose bytes
    // the engine had since changed. Two `SeqCst` loads with fences around them
    // are each atomic; the PAIR was not, and no amount of fencing makes two
    // reads one.
    //
    // The repair is the seqlock reader's own shape, and the argument for it is
    // a LINEARISATION at the middle load rather than the stronger claim an
    // earlier draft made (Codex round 8 finding 1). That draft said a stable
    // epoch proves `IN_FLIGHT` did not change across the span, and that is
    // FALSE, because the two words are not written together: `mark_exit` bumps
    // the epoch and only then lowers the count, so an exit can bump before the
    // opening load and lower inside the span; `mark_enter` raises the count and
    // only then bumps, so an entry can raise inside the span and bump after the
    // closing load. Both leave the epoch stable while the count moves.
    //
    // What the accepted reading DOES mean is that it is a truthful reading of
    // the instant of the middle load:
    //
    //  * An exiting writer whose count-decrement lands in the span bumped the
    //    epoch BEFORE the opening load, so its stores -- all of which precede
    //    that bump -- are already accounted for by the epoch this reading
    //    carries. The reading is `{E, quiet}` for an `E` that includes it, and
    //    a memory remembered at an older epoch does not match `E`.
    //  * An entering writer whose count-increment lands after the middle load
    //    has not run its body: its raise precedes its epoch bump which precedes
    //    every store. The reading is truthful for the instant it names, and the
    //    writer's later bump moves the epoch, so the next `still_current`
    //    refuses. If its raise lands BEFORE the middle load, the reading is not
    //    quiet and nothing is remembered or reused.
    //  * A writer wholly inside the span moves the epoch twice, the two epoch
    //    reads disagree, and the reading is retaken.
    //
    // So no accepted reading can be `{E, quiet}` for an `E` a later
    // `still_current` would take while a body completed inside the validated
    // span -- which is the property the reuse rests on. `copy_was_quiet` and
    // `still_current` check quiet explicitly at both ends rather than inferring
    // it, which is what makes the middle-load argument enough.
    //
    // The two epoch loads cannot be merged back into one: a `SeqCst` load may
    // not be reordered with, or elided across, another `SeqCst` operation, and
    // the in-flight load between them is one.
    for _ in 0..WITNESS_ATTEMPTS {
        fence(Ordering::SeqCst);
        let opening = WRITE_EPOCH.load(Ordering::SeqCst);
        between();
        let quiet = IN_FLIGHT.load(Ordering::SeqCst) == 0;
        let closing = WRITE_EPOCH.load(Ordering::SeqCst);
        fence(Ordering::SeqCst);
        if opening == closing {
            return RailWitness {
                epoch: opening,
                quiet,
            };
        }
    }
    // FAIL CLOSED rather than spin. `quiet: false` cannot pass `copy_was_quiet`
    // and cannot pass `still_current`, whatever epoch rides with it, so the
    // frame takes the full copy the tap exists to avoid and the next one tries
    // again.
    RailWitness {
        epoch: WRITE_EPOCH.load(Ordering::SeqCst),
        quiet: false,
    }
}

/// COLLECTOR SIDE. May a copy taken between `before` and `after` be
/// remembered?
///
/// Both readings must be QUIET and the epoch must not have moved between them.
/// `before.quiet` with a stable epoch already implies `after.quiet` (a writer
/// entering would have bumped), and both are still checked so the rule is one
/// expression rather than an inference.
pub fn copy_was_quiet(before: RailWitness, after: RailWitness) -> bool {
    before.quiet && after.quiet && before.epoch == after.epoch
}

/// COLLECTOR SIDE. Is a remembered copy still the engine's current state?
///
/// True only when no hooked writer has bumped the epoch since `remembered` was
/// taken, nothing is executing now, and every global gate stands open.
pub fn still_current(hooks_installed: bool, remembered: RailWitness) -> bool {
    if !hooks_installed || BASELINE_PENDING.load(Ordering::Acquire) {
        return false;
    }
    let now = witness();
    now.quiet && now.epoch == remembered.epoch
}

/// COLLECTOR SIDE. A rail copy was taken AND remembered with its witness. The
/// baseline obligation is discharged.
pub fn note_memory_seeded() {
    BASELINE_PENDING.store(false, Ordering::Release);
}

/// COLLECTOR SIDE. The remembered rail copy is gone (a refused or defective
/// copy, a changed root, a domain reset). Re-open the baseline obligation so
/// the next bracket copies under a NAMED reason instead of finding no memory
/// and reporting nothing.
pub fn note_memory_lost() {
    BASELINE_PENDING.store(true, Ordering::Release);
}

/// Install/uninstall reset. A fresh capture starts with a fresh epoch and a
/// fresh baseline obligation; a previous capture's witness must never answer
/// this one's currency question. The epoch restarts at zero, which is a value
/// an old witness can hold, so the BASELINE latch is what closes that — and it
/// is tested inside [`still_current`], not only at bracket open.
///
/// `IN_FLIGHT` IS DELIBERATELY NOT RESET. It is owned by the detours, and the
/// lane's own removal barrier already waits for every callback to leave before
/// a detour is unpatched; zeroing it here could only ever LOSE an in-flight
/// callback, which is the one direction that is unsafe.
pub fn reset_for_install() {
    WRITE_EPOCH.store(0, Ordering::Release);
    BASELINE_PENDING.store(true, Ordering::Release);
    MARKS_TOTAL.store(0, Ordering::Relaxed);
    WINDOWS_OPENED.store(0, Ordering::Relaxed);
}

/// MARK OPERATIONS, not hooked calls: two per completed call. See
/// [`mark_enter`].
pub fn marks_total() -> u64 {
    MARKS_TOTAL.load(Ordering::Relaxed)
}

pub fn windows_opened() -> u64 {
    WINDOWS_OPENED.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// WHAT THESE TESTS DO NOT COVER, stated rather than implied (Codex r3
    /// finding 3). They drive the state machine, and every one of them would
    /// still pass with the `AcqRel` operations weakened to `Relaxed` and both
    /// `SeqCst` fences deleted -- a single process cannot distinguish those
    /// orderings, and a racing empirical test would be the flaky
    /// non-proof this tree bans.
    ///
    /// The orderings are therefore a REVIEWED INVARIANT, and here is what
    /// reviewing them means. The writer's entry bump must not sink below the
    /// body's first store and its exit bump must not rise above the body's
    /// last, or a store lands outside the interval the epoch is vouching for;
    /// locked RMWs give that on x86-64 and `AcqRel` gives it in the Rust
    /// model. The reader's two [`witness`] calls must not be reordered with the
    /// copy's plain loads between them, which is why they are `SeqCst` and
    /// fenced on both sides rather than `Acquire` -- an acquire load stops
    /// LATER loads rising above it and does nothing about earlier ones sinking
    /// below, which is exactly the direction the closing witness needs.
    ///
    /// `a_writer_on_another_thread_is_visible_while_it_runs` is the one test
    /// that at least crosses a thread boundary; it pins the state machine's
    /// cross-thread visibility, not the fences. The two `witness_probed` tests
    /// below pin the one piece of the reader's internal ordering that IS
    /// reachable deterministically -- that its two field reads are one
    /// snapshot -- by running a writer from inside the reading itself.
    ///
    /// Serialises every unit test in this module: the two words and the latch
    /// are process globals and the default harness runs tests concurrently, so
    /// one test's reset would erase another test's epoch mid-flight.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    /// One complete hooked call, entry and exit.
    fn writer_call() {
        mark_enter();
        mark_exit();
    }

    /// Fresh state with a baseline already seeded.
    fn fresh() {
        reset_for_install();
        note_memory_seeded();
    }

    /// The consumer's own sequence: open, witness, copy, witness, remember.
    /// Returns the witness the memory would be stored with, or `None` when the
    /// copy may not be remembered.
    fn copy_and_remember(during: impl FnOnce()) -> Option<RailWitness> {
        open_hour_window(true);
        let before = witness();
        during();
        let after = witness();
        copy_was_quiet(before, after).then(|| {
            note_memory_seeded();
            after
        })
    }

    #[test]
    fn a_fresh_install_owes_a_baseline_and_the_gate_stays_shut() {
        // kills: reusing rail bytes nobody has copied yet this session.
        let _g = lock();
        reset_for_install();
        assert_eq!(open_hour_window(true), FullCopyReason::BaselinePending);
        assert!(!still_current(true, witness()));
        note_memory_seeded();
        assert_eq!(open_hour_window(true), FullCopyReason::None);
    }

    #[test]
    fn uninstalled_hooks_keep_the_gate_shut() {
        // kills: trusting a witness nothing maintains.
        let _g = lock();
        fresh();
        let held = witness();
        assert_eq!(open_hour_window(false), FullCopyReason::HooksNotInstalled);
        assert!(!still_current(false, held));
    }

    #[test]
    fn a_quiet_copy_is_remembered_and_stays_current_until_a_writer_runs() {
        // kills: a gate that never opens (the memory would be dead weight),
        // and a gate that stays open across a completed write.
        let _g = lock();
        fresh();
        let held = copy_and_remember(|| {}).expect("a quiet copy is rememberable");
        assert!(still_current(true, held), "nothing has run since");
        writer_call();
        assert!(!still_current(true, held), "a completed write invalidates it");
    }

    #[test]
    fn a_writer_that_runs_during_the_copy_is_never_remembered() {
        // kills: remembering bytes a writer was changing while they were read.
        let _g = lock();
        fresh();
        assert!(
            copy_and_remember(writer_call).is_none(),
            "the epoch moved across the copy",
        );
    }

    #[test]
    fn a_writer_still_running_when_the_copy_starts_is_never_remembered() {
        // kills: THE round-2 defect. Under the two-window dirty pair this lane
        // shipped first, a writer that entered in one window, had its entry bit
        // consumed by the bracket that copied, and only exited two brackets
        // later left BOTH bits clear -- and the bracket in between reused a
        // copy taken while the writer was mid-body.
        //
        // Here the copy is simply not rememberable: `before` is not quiet. The
        // writer is then walked out and the NEXT copy is, so the gate recovers
        // rather than wedging shut.
        let _g = lock();
        fresh();
        mark_enter(); // a writer enters and does not leave
        assert!(
            copy_and_remember(|| {}).is_none(),
            "a copy taken while a writer is inside the engine is not rememberable",
        );
        // A second bracket, still inside the writer: still refused. Under the
        // dirty-pair model this was exactly the bracket that reused.
        assert!(copy_and_remember(|| {}).is_none(), "still inside the writer");
        mark_exit(); // the writer finally leaves
        let held = copy_and_remember(|| {}).expect("now the copy is quiet");
        assert!(still_current(true, held));
    }

    #[test]
    fn a_writer_between_the_copy_and_the_reuse_is_seen_by_the_reuse() {
        // kills: THE hard stop -- a rail write racing a REUSE decision must
        // surface as that frame's full copy, never as a stale reuse.
        let _g = lock();
        fresh();
        let held = copy_and_remember(|| {}).expect("quiet");
        open_hour_window(true);
        writer_call(); // the engine lays a railway before this bracket probes
        assert!(!still_current(true, held));
        // The bracket that copies re-establishes currency for the next one.
        let held = copy_and_remember(|| {}).expect("quiet again");
        open_hour_window(true);
        assert!(still_current(true, held));
    }

    #[test]
    fn a_write_completing_between_brackets_costs_exactly_one_full_copy() {
        // kills: a conservatism regression, for the case this names and no
        // other (Codex r4 finding 3). The dirty-pair model this replaced always
        // cost TWO full copies for a write that completed between brackets;
        // the witness costs one. A write that SPANS a bracket is a different
        // case with a different cost, covered by
        // `a_writer_still_running_when_the_copy_starts_is_never_remembered`.
        let _g = lock();
        fresh();
        let held = copy_and_remember(|| {}).expect("quiet");
        writer_call();
        assert!(!still_current(true, held), "frame 1 copies");
        let held = copy_and_remember(|| {}).expect("quiet");
        assert!(still_current(true, held), "frame 2 reuses");
    }

    #[test]
    fn losing_the_remembered_copy_reopens_the_baseline_obligation() {
        // kills: reusing against bytes the collector no longer holds.
        let _g = lock();
        fresh();
        let held = witness();
        note_memory_lost();
        assert!(!still_current(true, held));
        assert_eq!(open_hour_window(true), FullCopyReason::BaselinePending);
        note_memory_seeded();
        assert_eq!(open_hour_window(true), FullCopyReason::None);
        assert!(still_current(true, held));
    }

    #[test]
    fn an_install_reset_cannot_be_answered_by_the_previous_captures_witness() {
        // kills: a second capture in one process inheriting the first one's
        // currency. The epoch restarts at zero, which is a value an old witness
        // can hold, so the BASELINE latch is what has to close this -- and it
        // is checked inside `still_current`, not only at bracket open.
        let _g = lock();
        fresh();
        let stale = copy_and_remember(|| {}).expect("quiet");
        assert_eq!(stale.epoch, 0, "the fixture must exercise the zero case");
        reset_for_install();
        assert!(
            !still_current(true, stale),
            "a witness from before the reset must not answer",
        );
    }

    #[test]
    fn a_writer_on_another_thread_is_visible_while_it_runs() {
        // kills: a witness that only ever sees its own thread. Every other test
        // here runs the writer inline, which cannot tell an `IN_FLIGHT` the
        // reader genuinely observes from one it merely remembers setting.
        //
        // Deterministic, not timing-based: two channels hand the turn back and
        // forth, so there is no sleep and no poll and nothing to be flaky
        // about.
        use std::sync::mpsc;

        let _g = lock();
        fresh();
        let before = witness();
        assert!(before.quiet);

        let (entered_tx, entered_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let writer = std::thread::spawn(move || {
            mark_enter();
            entered_tx.send(()).expect("the reader is waiting");
            release_rx.recv().expect("the reader releases us");
            mark_exit();
        });

        entered_rx.recv().expect("the writer entered");
        let inside = witness();
        assert!(!inside.quiet, "a writer on another thread must read as noisy");
        assert_ne!(inside.epoch, before.epoch, "its entry bumped the epoch");
        assert!(
            !copy_was_quiet(before, inside),
            "a copy spanning that entry is not rememberable",
        );

        release_tx.send(()).expect("the writer is waiting");
        writer.join().expect("the writer finished");

        let after = witness();
        assert!(after.quiet, "and it is quiet again once the writer left");
        assert_ne!(after.epoch, inside.epoch, "its exit bumped the epoch too");
        assert!(copy_was_quiet(after, witness()), "a later copy may be taken");
    }

    #[test]
    fn a_writer_completing_between_the_two_field_reads_is_never_quiet_at_the_old_epoch() {
        // kills: reading `WRITE_EPOCH` once and `IN_FLIGHT` once and calling
        // the pair a snapshot. THIS IS THE ROUND-7 BLOCKING INTERLEAVING, and
        // it is reachable only from inside the reading, which is why
        // `witness_probed` takes a probe: read epoch E, a whole writer enters,
        // stores and exits, read in-flight 0. The old form returned
        // `{E, quiet}` -- indistinguishable from a genuinely idle moment at E
        // -- and `still_current` handed the bracket a memory the engine had
        // since changed.
        //
        // Deterministic: the probe fires exactly one writer, on the first
        // attempt, so the retaken reading is the true state and there is
        // nothing to be flaky about.
        let _g = lock();
        fresh();
        let held = copy_and_remember(|| {}).expect("a quiet copy is remembered");
        assert!(still_current(true, held), "current until something writes");

        let mut fired = false;
        let torn = witness_probed(|| {
            if !fired {
                fired = true;
                writer_call();
            }
        });
        assert!(fired, "the probe must have run inside the reading");
        assert!(
            !(torn.quiet && torn.epoch == held.epoch),
            "a reading taken across a completed write must not read as the remembered one",
        );
        assert!(
            !still_current(true, held),
            "and the remembered copy must not answer for the new bytes",
        );
        // What it reports instead is the truth: idle again, two bumps on.
        assert!(torn.quiet, "the writer had finished by the retake");
        assert_eq!(torn.epoch, held.epoch + 2, "its entry and exit both bumped");
    }

    #[test]
    fn a_reading_that_never_settles_gives_up_closed() {
        // kills: an UNBOUNDED retry in the reader, and a give-up that reports
        // quiet. A tap hammered hard enough that no reading ever settles must
        // cost full copies, not a spin inside the hourly bracket and not a
        // reading the bracket would trust.
        let _g = lock();
        fresh();
        let held = copy_and_remember(|| {}).expect("a quiet copy is remembered");
        let mut probes = 0usize;
        let torn = witness_probed(|| {
            probes += 1;
            writer_call();
        });
        assert_eq!(
            probes, WITNESS_ATTEMPTS,
            "the reader must try a bounded number of times and then stop",
        );
        assert!(!torn.quiet, "an unsettled reading is never quiet");
        assert!(
            !copy_was_quiet(torn, torn),
            "and so it can never be remembered",
        );
        assert!(!still_current(true, held));
        // The next reading, with nothing in the way, is clean again: giving up
        // is per-reading and latches nothing.
        assert!(witness().quiet);
    }

    #[test]
    fn marks_and_windows_are_counted_for_the_capture() {
        // kills: shipping a tap nobody can price. Without these two the next
        // capture cannot tell "the rails never moved" from "the detour never
        // fired". Marks are OPERATIONS: two per completed call.
        let _g = lock();
        reset_for_install();
        assert_eq!(marks_total(), 0);
        assert_eq!(windows_opened(), 0);
        writer_call();
        open_hour_window(true);
        assert_eq!(marks_total(), 2, "one completed call is two mark operations");
        assert_eq!(windows_opened(), 1);
    }
}
