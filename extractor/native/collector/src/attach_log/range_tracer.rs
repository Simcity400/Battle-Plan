//! ONE FRAME'S READS, RECORDED. The recording half of the capture-floor
//! range/byte/depth instrument; [`crate::range_census`] is the reduction half
//! and owns the sidecar format.
//!
//! # Why this shape and no other
//!
//! Every guarded read this collector makes funnels through exactly three
//! functions in `attach_log.rs` — `read_scalar::<N>`, `read_bytes` and
//! `read_into` — because all three, and nothing else on the frame path, call
//! `copy_selected`. (`read_bytes_scan_reporting` is the fourth copy site and is
//! deliberately NOT traced: it is the full-heap discovery sweep, which walks
//! gigabytes of address space looking for candidates rather than observing a
//! domain's data, so folding it into a per-domain range census would drown the
//! measurement in the search that precedes it. The census reports that
//! exclusion rather than hiding it.)
//!
//! Three call sites is therefore the whole instrument, and it is the reason a
//! tracer is cheaper here than any per-reader instrumentation would be: no
//! reader changes, no reader can forget to report, and a reader added tomorrow
//! is traced the day it is written.
//!
//! Attribution comes free from the same place the clock bracket comes from:
//! `ReaderClockBracketGuard` already knows its tag and already saves/restores a
//! thread-local on nest, so the tracer rides that lifetime exactly. A read
//! taken outside any bracket is attributed to [`TAG_UNATTRIBUTED`] rather than
//! to whichever bracket read last — an unattributed read is a fact about the
//! frame, not a rounding error.
//!
//! THE TAG IS NOT ALWAYS A DOMAIN ID. Brackets are per WORKER: `LW_TAG_COUNTRY_
//! BUNDLE` (200) is one worker covering domains 10..=17, and some workers cover
//! a neighbour (domain 49's also covers 50). The census reports rows per
//! bracket tag and says so; see `crate::range_census`'s module header for what
//! that costs and what it cannot affect.
//!
//! # Zero production cost, and why that is structural rather than measured
//!
//! Every call site is written `#[cfg(feature = "range-tracer")]`. With the
//! feature off there is no call, no branch, no atomic load and no symbol: the
//! compiler is not asked to optimise anything away because nothing was emitted.
//! `attach_log.rs` does not `mod range_tracer;` without the feature either, so
//! this file is not compiled into a production collector at all. The default
//! `cargo build -p collector` carries none of it, and the release/package
//! receipt path already enumerates forbidden features by name.
//!
//! # Read semantics are untouched
//!
//! The tracer runs strictly AFTER a read has produced its result, observes the
//! bytes the caller is about to receive, and returns the caller's value
//! unchanged. It never gates, retries, widens or reorders a read. It cannot
//! change what the frame publishes; it can only change how long the frame took,
//! which is why it is a diagnostic and why a traced frame's timings must not be
//! compared with an untraced one's.
//!
//! # Arming
//!
//! `BATTLE_PLAN_RANGE_TRACE_PATH` names the sidecar. When it is set at attach the
//! tracer arms; the NEXT extraction-frame boundary starts recording, the one
//! after it stops and writes the file, and the tracer disarms. Exactly one
//! frame is ever traced per process. `BATTLE_PLAN_RANGE_TRACE_MAX_ENTRIES` bounds the
//! in-memory buffer (default [`DEFAULT_MAX_ENTRIES`]); reaching it stops
//! recording and sets the sidecar's truncated flag, so a capped trace reports
//! itself as a lower bound instead of as a total.
//!
//! # RUNBOOK: taking the whole-roster census during a capture window
//!
//! 1. Build the collector WITH the feature. Both the DLL and the recorder host
//!    must come from this build — a host paired with a stale `collector.dll`
//!    reports itself as a hook-install failure, not as a missing tracer:
//!
//!    ```text
//!    cargo build -p collector --release --features range-tracer
//!    cargo build -p recorder_host --release
//!    ```
//!
//! 2. Set `BATTLE_PLAN_RANGE_TRACE_PATH` in the environment the GAME process
//!    inherits, before injection, to a path outside `runs/`. The DLL reads it
//!    on its first frame boundary; a variable set only in the host's shell
//!    after injection is not seen.
//!
//! 3. Run the capture as normal. Trace the frame you want to be
//!    REPRESENTATIVE: arm late enough that discovery has warmed (the first
//!    frames of a capture read far less than a steady-state one, and several
//!    domains are still cold), which in practice means setting the variable
//!    for a capture that has already settled rather than for the very first
//!    frame after attach. The tracer records one frame and then costs nothing.
//!
//! 4. Reduce the sidecar. The tool is pure formatting over
//!    [`crate::range_census`], which the library gate covers:
//!
//!    ```text
//!    cargo run -p collector --release --bin range_census -- \
//!        <sidecar.bin> --span 0x10000 --json census.json
//!    ```
//!
//! 5. Read the verdict off two lines: `FULL SET / unique bytes` against the
//!    plan's 70 MiB rejection threshold, and `FULL SET / distinct ranges`
//!    against its 450k. If the header says TRUNCATED, both are lower bounds and
//!    the run must be repeated with a larger
//!    `BATTLE_PLAN_RANGE_TRACE_MAX_ENTRIES` before either can reject anything —
//!    a truncated trace can only ever CONFIRM an over-threshold verdict, never
//!    an under-threshold one.
//!
//! # What a live trace costs, and why it must not be a capture that counts
//!
//! Every traced read appends to a per-thread buffer and, for a bulk read, scans
//! the result for heap-shaped qwords. That is small per read and enormous per
//! frame at ~1e6 reads. A traced frame will overrun its hour and the scheduler
//! will evict hours behind it. That is expected and harmless — the eviction is
//! named on the wire like any other backpressure — but it means a capture taken
//! with the tracer armed is EVIDENCE ABOUT READS, never a data capture, and its
//! timings must never be compared with an untraced run's.

use std::cell::{Cell, RefCell};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

use crate::range_census::{
    TraceEntry, TraceHeader, TRACE_ENTRY_LEN, TRACE_FLAG_TRUNCATED, TRACE_KIND_POINTER,
    TRACE_KIND_READ, TRACE_VERSION,
};

/// Environment variable naming the sidecar to write. Absent = tracer inert.
pub const TRACE_PATH_ENV: &str = "BATTLE_PLAN_RANGE_TRACE_PATH";
/// Environment variable overriding [`DEFAULT_MAX_ENTRIES`].
pub const TRACE_MAX_ENTRIES_ENV: &str = "BATTLE_PLAN_RANGE_TRACE_MAX_ENTRIES";

/// Default in-memory entry cap: 64M entries, i.e. 1.5 GiB of sidecar at
/// [`TRACE_ENTRY_LEN`] bytes each. Sized from the largest per-frame read count
/// this collector has ever published (domain 53's ~660k reads/frame, times a
/// roster of 68) with an order of magnitude of headroom, so the expected case
/// is not truncated and the pathological one still cannot exhaust the game's
/// address space.
pub const DEFAULT_MAX_ENTRIES: usize = 64 * 1024 * 1024;

/// Attribution for a read taken while no `ReaderClockBracketGuard` is held.
/// Deliberately a reserved id and not "the last domain that read": a read with
/// no bracket belongs to no domain's hour, and the census must be able to say
/// so.
pub const TAG_UNATTRIBUTED: u16 = u16::MAX;

/// The tracer is armed and waiting for a frame boundary.
static ARMED: AtomicBool = AtomicBool::new(false);
/// The tracer is recording the current frame. Loaded on every traced read, so
/// it is the one atomic on the instrumented path.
static CAPTURING: AtomicBool = AtomicBool::new(false);
/// Global entry order. The census needs a causal order across threads to
/// reconstruct pointer provenance; a relaxed `fetch_add` gives a total order
/// over the atomic's own modifications, which is exactly what "this pointer was
/// recorded before that read" requires.
static NEXT_SEQ: AtomicU64 = AtomicU64::new(0);
/// Entries recorded so far, against the cap.
static RECORDED: AtomicUsize = AtomicUsize::new(0);
/// Reads recorded so far that were taken INSIDE a `ReaderClockBracketGuard`,
/// i.e. by a domain reader rather than by the capture loop's own clock sample.
/// This, and not the entry count, is what tells an extracting frame from an
/// idle one; see [`on_frame_boundary`].
static BRACKETED_READS: AtomicU64 = AtomicU64::new(0);
/// Set when the cap stopped the recording.
static TRUNCATED: AtomicBool = AtomicBool::new(false);
/// Extraction frames seen since arming; published in the sidecar header so a
/// census names the frame it covers.
static FRAME_ORDINAL: AtomicU64 = AtomicU64::new(0);
/// Optional heap-window override for the pointer scan. Both zero = use the
/// window the read primitive passed in (the live game's). Set only by offline
/// fixtures, whose arenas are allocated wherever the OS put them.
static WINDOW_LO: AtomicUsize = AtomicUsize::new(0);
static WINDOW_HI: AtomicUsize = AtomicUsize::new(0);

/// Point the pointer scan at an arbitrary window. `(0, 0)` restores the
/// caller-supplied one.
pub fn set_heap_window(lo: usize, hi: usize) {
    WINDOW_LO.store(lo, Ordering::SeqCst);
    WINDOW_HI.store(hi, Ordering::SeqCst);
}

struct TraceConfig {
    path: String,
    max_entries: usize,
}

static CONFIG: Mutex<Option<TraceConfig>> = Mutex::new(None);
/// Entries flushed out of the per-thread buffers. Contended only at bracket
/// close, once per domain per frame.
static SINK: Mutex<Vec<TraceEntry>> = Mutex::new(Vec::new());

thread_local! {
    /// Domain of the innermost `ReaderClockBracketGuard` on this thread.
    static TRACE_DOMAIN: Cell<u16> = const { Cell::new(TAG_UNATTRIBUTED) };
    /// This thread's unflushed entries. Per-thread so the instrumented read
    /// path takes no lock.
    static BUFFER: RefCell<Vec<TraceEntry>> = const { RefCell::new(Vec::new()) };
}

/// Arm the tracer from the environment. Run once, on the first frame boundary,
/// so the instrument needs no edit to the attach path. A missing
/// [`TRACE_PATH_ENV`] leaves the tracer inert for the life of the process.
pub fn arm_from_env() {
    if ARMED.load(Ordering::SeqCst) {
        // Already armed by a caller that supplied its own configuration; the
        // environment must not overwrite it.
        return;
    }
    let Ok(path) = std::env::var(TRACE_PATH_ENV) else {
        return;
    };
    if path.is_empty() {
        return;
    }
    let max_entries = std::env::var(TRACE_MAX_ENTRIES_ENV)
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MAX_ENTRIES);
    if let Ok(mut config) = CONFIG.lock() {
        *config = Some(TraceConfig { path, max_entries });
    }
    ARMED.store(true, Ordering::SeqCst);
}

/// Called from `page_cache_new_frame`, i.e. once at the start of every
/// extraction frame. Armed -> recording on the first boundary; recording ->
/// written and disarmed on the second. Exactly one frame is covered.
pub fn on_frame_boundary() {
    static ARM_ONCE: std::sync::Once = std::sync::Once::new();
    ARM_ONCE.call_once(arm_from_env);
    if CAPTURING.load(Ordering::SeqCst) {
        // AN INTERVAL WITH NO BRACKETED READ IS DISCARDED, NOT CARRIED.
        //
        // Why an interval can be barren: the call site sits past every early
        // `continue` above it, but still runs BEFORE the scheduler decides
        // whether this hour is swept. An idle or awaiting scheduler yields no
        // sweep and the loop comes round again having extracted nothing — yet
        // it is not read-free, because the capture loop's own clock sample goes
        // through `read_scalar` like everything else, and domain 47's serial
        // prewarm reads before any bracket exists. So neither "no entries" nor
        // "some entries" distinguishes an extracting interval; only a read
        // carrying a real bracket tag does.
        //
        // Why DISCARD rather than wait. An earlier version simply declined to
        // close and left the trace open. That is worse than the problem it
        // solved: the recorded entries keep accumulating, so the sidecar a
        // later boundary finally writes spans SEVERAL intervals while claiming
        // to be one frame (Codex review 019fd9e8-d714-7071-8698-8454baa2f288).
        // Resetting instead keeps the invariant that matters — a census covers
        // exactly one inter-boundary interval — and simply retries on the next
        // one.
        //
        // TRUNCATION CLOSES REGARDLESS, and that is what makes this terminate.
        // Once the cap is hit no further read is recorded, so `BRACKETED_READS`
        // can never rise; discarding and retrying would truncate again the same
        // way, forever, and no sidecar would ever be written. A short file
        // carrying the truncated flag says exactly what it is.
        if BRACKETED_READS.load(Ordering::SeqCst) == 0 && !TRUNCATED.load(Ordering::SeqCst) {
            discard_open_interval();
            return;
        }
        CAPTURING.store(false, Ordering::SeqCst);
        ARMED.store(false, Ordering::SeqCst);
        write_sidecar();
        return;
    }
    if ARMED.load(Ordering::SeqCst) {
        FRAME_ORDINAL.fetch_add(1, Ordering::SeqCst);
        CAPTURING.store(true, Ordering::SeqCst);
    }
}

/// Throw away everything recorded since the last boundary and start the
/// interval again, still capturing. Called when an interval turned out not to
/// contain an extraction; see [`on_frame_boundary`].
///
/// THIS CLEARS EVERYTHING THE INTERVAL RECORDED, and the argument has two
/// halves because there are two kinds of recording thread:
///
///  * BRACKETED reads live in a per-thread buffer, drained to the sink by
///    `flush_after_bracket` when the bracket closes. Domain workers are scoped
///    threads that `thread::scope` joins before the frame builder returns, so
///    by the time the sweep thread reaches a boundary every one of them has
///    closed its bracket and flushed. No worker buffer can still hold anything.
///  * UNATTRIBUTED reads never enter a per-thread buffer at all — see
///    `note_read`. They go straight to the sink precisely so that a thread with
///    no bracket lifetime (the long-lived scheduler observer) leaves nothing a
///    discard cannot reach.
///
/// So the sink, plus this thread's own buffer, is the whole of it.
fn discard_open_interval() {
    BUFFER.with(|buffer| buffer.borrow_mut().clear());
    // TEST-BUILD ONLY. Fires immediately before the lock attempt and AFTER
    // every pre-lock statement, so a test that receives this signal knows the
    // discard has finished whatever it does outside the lock and is about to
    // block on it. That is the one observation needed to force the ordering the
    // race turned on, and it is why the regression test needs no timing
    // assumption (Codex review 019fda22-fabd-7121-8629-c58c4d792e08).
    #[cfg(test)]
    if let Some(hook) = test_support::take_discard_pre_lock_hook() {
        hook();
    }
    // The clear AND the counter resets happen under the sink lock, because
    // `note_read`'s unattributed path reserves and appends under that same
    // lock. Resetting outside it would let an in-flight append land in the new
    // interval unaccounted for; see the note there.
    let Ok(mut sink) = SINK.lock() else {
        return;
    };
    sink.clear();
    RECORDED.store(0, Ordering::SeqCst);
    BRACKETED_READS.store(0, Ordering::SeqCst);
    drop(sink);
    // The ordinal counts intervals the tracer opened, so a census names which
    // one it finally covered rather than always claiming the first.
    FRAME_ORDINAL.fetch_add(1, Ordering::SeqCst);
}

/// Enter a domain's bracket. Returns the enclosing domain, which the caller
/// MUST hand back to [`leave_domain`] on drop — the same save/restore the
/// bracket guard already does for its clock observation, for the same reason:
/// brackets nest, and an inner one's exit must return the thread to its
/// enclosing domain rather than to "no domain".
pub fn enter_domain(bracket_tag: u16) -> u16 {
    TRACE_DOMAIN.with(|slot| slot.replace(bracket_tag))
}

/// Leave a bracket, restoring what [`enter_domain`] returned. Cheap: one
/// thread-local store and nothing else.
///
/// THE FLUSH IS NOT DONE HERE, and that separation is required rather than
/// tidy. `ReaderClockBracketGuard::drop` takes the bracket's CLOSING clock
/// observation, and the interval between the reader's last read and that
/// observation is what decides whether the domain's hour is coherent. Taking a
/// global mutex and appending a buffer of up to a million entries inside that
/// interval can push the closing read past a game-hour boundary and turn a
/// perfectly good domain-hour into a torn or mislabelled one — a tracer
/// changing what a domain publishes, which is exactly what a diagnostic may
/// never do. So the guard restores attribution here, closes its bracket, and
/// calls [`flush_after_bracket`] only once the clock has been read.
pub fn leave_domain(restore: u16) {
    TRACE_DOMAIN.with(|slot| slot.set(restore));
}

/// Move this thread's recorded entries to the shared sink. MUST be called only
/// after the enclosing clock bracket has taken its closing observation; see
/// [`leave_domain`].
///
/// Flushing per bracket rather than at frame end keeps the per-thread buffer
/// bounded by one bracket's reads and needs no thread-exit hook, which scoped
/// worker threads do not offer.
pub fn flush_after_bracket() {
    flush_thread_buffer();
}

fn flush_thread_buffer() {
    BUFFER.with(|buffer| {
        let mut buffer = buffer.borrow_mut();
        if buffer.is_empty() {
            return;
        }
        if let Ok(mut sink) = SINK.lock() {
            sink.append(&mut buffer);
        } else {
            buffer.clear();
        }
    });
}

/// Record one guarded read and the heap-shaped qwords it returned.
///
/// `bytes` is the caller's result buffer, read-only and already complete. It is
/// scanned at 8-byte alignment for values inside the collector's heap window;
/// each hit becomes a pointer entry attributed to this read, which is what
/// lets the census reconstruct discovery depth offline.
pub fn note_read(addr: usize, len: usize, bytes: &[u8], heap_lo: usize, heap_hi: usize) {
    if !CAPTURING.load(Ordering::Relaxed) {
        return;
    }
    // TEST-BUILD ONLY, and absent from every other build including a
    // feature-enabled production one. The tracer records every read in the
    // process, which is correct for a collector living inside a game process
    // but means `cargo test` — where ~1500 other tests run concurrently on
    // other threads, some of them opening real `ReaderClockBracketGuard`s —
    // would mix their reads into the tracer's own tests. Enrolment lets a test
    // name the threads whose reads it is measuring; see `test_support`.
    #[cfg(test)]
    if !test_support::thread_is_enrolled() {
        return;
    }
    // Deliberately checked BEFORE the reservation, and deliberately NOT
    // implemented by clearing `CAPTURING`: the frame boundary uses `CAPTURING`
    // to decide that a trace is open and must be WRITTEN, so cancelling it here
    // would turn "the trace filled up" into "no sidecar at all" — the failure
    // mode that has to produce a marked, short file instead of silence.
    if TRUNCATED.load(Ordering::Relaxed) {
        return;
    }
    let bracket_tag = TRACE_DOMAIN.with(Cell::get);
    // One reservation for the read plus, at most, one per aligned qword.
    let pointer_slots = bytes.len() / 8;
    let want = 1 + pointer_slots;
    let max_entries = max_entries();
    let unattributed = bracket_tag == TAG_UNATTRIBUTED;

    // THE UNATTRIBUTED PATH RESERVES AND APPENDS UNDER ONE LOCK.
    //
    // Reserving against `RECORDED` and then appending to `SINK` as two separate
    // steps is a race with `discard_open_interval`, which clears the sink and
    // resets the counter: an observer that reserved before the discard and
    // appended after it leaves an entry in the new interval that the reset
    // counter does not account for, so repeating it around successive discards
    // escapes the memory cap without bound (Codex review
    // 019fda0b-af46-7ca3-a8e6-99dc5e9845a6). The scheduler observer really does
    // run concurrently with frame boundaries, so this interleaving is reachable.
    //
    // Holding the sink lock across both makes them one critical section, and
    // `discard_open_interval` takes the same lock while it clears and resets.
    // A discard therefore observes the reservation and the entry together or
    // neither, and the counter always describes the sink.
    //
    // The BRACKETED path keeps its unlocked reservation and its per-thread
    // buffer, which is the whole reason the buffer exists — and it is sound for
    // a different reason, not a weaker one: every bracketed read is made by a
    // scoped worker that `thread::scope` joins before the frame builder
    // returns, so no bracketed read is ever in flight when the sweep thread
    // reaches a boundary and discards.
    let sink_guard = if unattributed {
        match SINK.lock() {
            Ok(guard) => Some(guard),
            // A poisoned sink means a recorder panicked; recording nothing
            // further is the fail-closed answer.
            Err(_) => return,
        }
    } else {
        None
    };

    #[cfg(test)]
    if let Some(hook) = test_support::take_pre_reserve_hook(unattributed) {
        hook();
    }

    let claimed = RECORDED.fetch_add(want, Ordering::Relaxed);
    if claimed.saturating_add(want) > max_entries {
        TRUNCATED.store(true, Ordering::Relaxed);
        return;
    }

    if !unattributed {
        BRACKETED_READS.fetch_add(1, Ordering::Relaxed);
    }
    let mut entries: Vec<TraceEntry> = Vec::with_capacity(want);
    entries.push(TraceEntry {
        kind: TRACE_KIND_READ,
        bracket_tag,
        len: u32::try_from(len).unwrap_or(u32::MAX),
        seq: NEXT_SEQ.fetch_add(1, Ordering::Relaxed),
        value: addr as u64,
    });
    // An offline fixture's arena is `VirtualAlloc`ed and need not land inside
    // the live game's heap window, so the window is overridable. Unset (both
    // zero) means "use the caller's", which is what a live capture always does.
    let (heap_lo, heap_hi) = match (
        WINDOW_LO.load(Ordering::Relaxed),
        WINDOW_HI.load(Ordering::Relaxed),
    ) {
        (0, 0) => (heap_lo, heap_hi),
        (lo, hi) => (lo, hi),
    };
    for chunk in bytes.chunks_exact(8) {
        let value = u64::from_le_bytes(chunk.try_into().unwrap_or([0u8; 8]));
        let as_usize = value as usize;
        if as_usize >= heap_lo && as_usize < heap_hi {
            entries.push(TraceEntry {
                kind: TRACE_KIND_POINTER,
                bracket_tag,
                len: 0,
                seq: NEXT_SEQ.fetch_add(1, Ordering::Relaxed),
                value,
            });
        }
    }
    // UNATTRIBUTED READS BYPASS THE PER-THREAD BUFFER, still holding the lock
    // taken above.
    //
    // The buffer exists so a domain worker's hot loop takes no lock, and it is
    // drained by `flush_after_bracket` when that worker's bracket closes. A
    // thread that never opens a bracket has no such drain — and the long-lived
    // scheduler observer is exactly that: started before capture, sampling the
    // authenticated clock through these same primitives at poll rate, joined
    // only at scheduler shutdown. Its entries would sit in its TLS buffer
    // indefinitely, survive every `discard_open_interval` (which cannot reach
    // another thread's TLS), and escape the cap accounting. A paused capture,
    // which discards at poll rate, would grow it without limit (Codex review
    // 019fd9f7-b331-7001-9180-5e9f31c720ad).
    //
    // Sending them straight to the sink makes the sink the single place a
    // discard has to clear. The cost is a lock per unattributed read, which is
    // affordable because these are the clock samples and domain 47's serial
    // prewarm, not a per-object walk — and this is a diagnostic build.
    if let Some(mut sink) = sink_guard {
        sink.extend_from_slice(&entries);
        return;
    }
    BUFFER.with(|buffer| buffer.borrow_mut().extend_from_slice(&entries));
}

fn max_entries() -> usize {
    CONFIG
        .lock()
        .ok()
        .and_then(|config| config.as_ref().map(|config| config.max_entries))
        .unwrap_or(DEFAULT_MAX_ENTRIES)
}

/// Write the sidecar. Failures are reported on stderr and never propagate: a
/// diagnostic that could abort a capture would be a production hazard even
/// behind a feature gate.
fn write_sidecar() {
    // Any thread that never closed a bracket still holds entries; the frame
    // boundary runs on the sweep thread, so only its own buffer is reachable.
    // Worker buffers were flushed by `flush_after_bracket` when their brackets closed.
    flush_thread_buffer();

    let path = match CONFIG.lock() {
        Ok(config) => match config.as_ref() {
            Some(config) => config.path.clone(),
            None => return,
        },
        Err(_) => return,
    };
    let entries = match SINK.lock() {
        Ok(mut sink) => std::mem::take(&mut *sink),
        Err(_) => return,
    };
    let header = TraceHeader {
        version: TRACE_VERSION,
        entry_len: TRACE_ENTRY_LEN as u32,
        frame_ordinal: FRAME_ORDINAL.load(Ordering::SeqCst),
        flags: if TRUNCATED.load(Ordering::Relaxed) {
            TRACE_FLAG_TRUNCATED
        } else {
            0
        },
    };
    let result = (|| -> std::io::Result<()> {
        let mut out = BufWriter::new(File::create(&path)?);
        out.write_all(&header.encode())?;
        for entry in &entries {
            out.write_all(&entry.encode())?;
        }
        out.flush()
    })();
    match result {
        Ok(()) => eprintln!(
            "range_tracer: wrote {} entries for frame {} to {path}",
            entries.len(),
            header.frame_ordinal
        ),
        Err(error) => eprintln!("range_tracer: could not write {path}: {error}"),
    }
}

/// Test-only: drive the tracer through one frame without a game attached.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::sync::{Mutex as StdMutex, MutexGuard};

    /// The tracer's state is process-global, so every instrument that drives it
    /// — its own tests and the offline per-domain censuses — serialises here.
    pub static TRACER_LOCK: StdMutex<()> = StdMutex::new(());

    pub fn lock() -> MutexGuard<'static, ()> {
        TRACER_LOCK.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// Enrolment generation a thread must carry for its reads to be recorded.
    /// Zero means "no restriction", which is what a non-test build always has
    /// because nothing outside this module ever raises it.
    static REQUIRED_GENERATION: AtomicU64 = AtomicU64::new(0);

    thread_local! {
        /// The generation this thread was enrolled in.
        static ENROLLED_GENERATION: Cell<u64> = const { Cell::new(0) };
    }

    /// May this thread's reads be recorded?
    ///
    /// WHY THIS EXISTS. `cargo test` runs the whole crate's tests concurrently
    /// in one process, and the tracer's state is process-global by design.
    /// While a trace is open, another test's guarded read — including one taken
    /// under a real `ReaderClockBracketGuard`, which several frame-builder
    /// tests open — is recorded exactly like a domain's, because from the
    /// tracer's point of view that is what it is. `TRACER_LOCK` serialises the
    /// tracer's OWN tests but cannot stop the rest of the suite from reading.
    ///
    /// So a test enrols the threads it is measuring and the tracer ignores
    /// every other one. This is not a weakening of the instrument: production
    /// never raises the generation, so `thread_is_enrolled` is compiled out of
    /// every non-test build and every thread is recorded there.
    pub fn thread_is_enrolled() -> bool {
        let required = REQUIRED_GENERATION.load(Ordering::Relaxed);
        required == 0 || ENROLLED_GENERATION.with(Cell::get) == required
    }

    /// Enrol the calling thread in the current generation. A thread spawned by
    /// a test must call this itself; enrolment is per thread.
    pub fn enroll_current_thread() {
        let required = REQUIRED_GENERATION.load(Ordering::SeqCst);
        ENROLLED_GENERATION.with(|slot| slot.set(required));
    }

    type Hook = Box<dyn FnOnce() + Send>;

    /// A one-shot callback `note_read` runs on the UNATTRIBUTED path after it
    /// has taken the sink lock and before it reserves against the cap. Exists
    /// so a test can drive the exact interleaving that made the reserve/append
    /// pair racy: hold a recorder at that point and try to discard underneath
    /// it. Without an injection point the window is unobservable and the
    /// invariant could only be argued, not tested.
    static PRE_RESERVE_HOOK: Mutex<Option<Hook>> = Mutex::new(None);

    pub fn set_pre_reserve_hook(hook: impl FnOnce() + Send + 'static) {
        if let Ok(mut slot) = PRE_RESERVE_HOOK.lock() {
            *slot = Some(Box::new(hook));
        }
    }

    /// Take the hook, if one is armed and this read is on the path it targets.
    pub fn take_pre_reserve_hook(unattributed: bool) -> Option<Hook> {
        if !unattributed {
            return None;
        }
        PRE_RESERVE_HOOK.lock().ok().and_then(|mut slot| slot.take())
    }

    /// A one-shot callback `discard_open_interval` runs immediately before it
    /// attempts the sink lock, after every statement that precedes that lock.
    /// Receiving its signal is proof that the discard has completed its
    /// pre-lock work and is about to block — which is what lets the regression
    /// test force an ordering instead of hoping for one.
    static DISCARD_PRE_LOCK_HOOK: Mutex<Option<Hook>> = Mutex::new(None);

    pub fn set_discard_pre_lock_hook(hook: impl FnOnce() + Send + 'static) {
        if let Ok(mut slot) = DISCARD_PRE_LOCK_HOOK.lock() {
            *slot = Some(Box::new(hook));
        }
    }

    pub fn take_discard_pre_lock_hook() -> Option<Hook> {
        DISCARD_PRE_LOCK_HOOK
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
    }

    /// How many entries the shared sink currently holds, and what the cap
    /// counter believes was recorded. The two must agree; see
    /// `the_cap_counter_and_the_sink_never_disagree_across_a_discard`.
    pub fn sink_len_and_recorded() -> (usize, usize) {
        let len = SINK.lock().map(|sink| sink.len()).unwrap_or(0);
        (len, RECORDED.load(Ordering::SeqCst))
    }

    /// Reset every global so drivers do not inherit each other's state, and
    /// enrol the calling thread in a fresh generation so no other test's reads
    /// are recorded. Callers must hold [`TRACER_LOCK`].
    pub fn reset(path: &str, max_entries: usize) {
        REQUIRED_GENERATION.fetch_add(1, Ordering::SeqCst);
        enroll_current_thread();
        ARMED.store(false, Ordering::SeqCst);
        CAPTURING.store(false, Ordering::SeqCst);
        TRUNCATED.store(false, Ordering::SeqCst);
        RECORDED.store(0, Ordering::SeqCst);
        BRACKETED_READS.store(0, Ordering::SeqCst);
        NEXT_SEQ.store(0, Ordering::SeqCst);
        FRAME_ORDINAL.store(0, Ordering::SeqCst);
        if let Ok(mut sink) = SINK.lock() {
            sink.clear();
        }
        BUFFER.with(|buffer| buffer.borrow_mut().clear());
        TRACE_DOMAIN.with(|slot| slot.set(TAG_UNATTRIBUTED));
        WINDOW_LO.store(0, Ordering::SeqCst);
        WINDOW_HI.store(0, Ordering::SeqCst);
        if let Ok(mut config) = CONFIG.lock() {
            *config = Some(TraceConfig {
                path: path.to_string(),
                max_entries,
            });
        }
        ARMED.store(true, Ordering::SeqCst);
    }

    pub fn capturing() -> bool {
        CAPTURING.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::range_census::{read_trace, Census, DerivationSpan, TraceEntry};
    use test_support::lock;

    /// THE TRACER IS PROCESS-GLOBAL, AND THESE TESTS DO NOT RUN ALONE.
    ///
    /// While a trace is open, EVERY guarded read anywhere in the process is
    /// recorded — that is the design, and in production it is exactly right:
    /// the collector lives inside a game process whose reads all belong to the
    /// frame, including the scheduler observer's clock samples on its own
    /// long-lived thread. Under `cargo test` it would mean the other ~1500
    /// tests, running concurrently and reading real memory through the same
    /// primitives (several of them under real `ReaderClockBracketGuard`s),
    /// landing in the same sidecar.
    ///
    /// `test_support::reset` therefore opens a fresh enrolment generation and
    /// enrols the calling thread, and `note_read` ignores unenrolled threads in
    /// test builds only. That makes the sidecar contain exactly what the test
    /// performed, so these tests can assert on WHOLE-FILE contents — which is a
    /// far stronger statement than filtering to the entries they expected and
    /// asserting those are present.
    fn temp_path(name: &str) -> String {
        let mut path = std::env::temp_dir();
        path.push(format!("hoi4_range_tracer_{name}_{}.bin", std::process::id()));
        path.to_string_lossy().into_owned()
    }

    const LO: usize = 0x100_0000_0000;
    const HI: usize = 0x7FF0_0000_0000;

    /// GROUND TRUTH BY CONSTRUCTION: a hand-built read sequence whose ranges,
    /// unique bytes and pointer chain are known before the tracer runs, driven
    /// through the real recording path and reduced by the real census. This is
    /// the test that says the instrument counts what it claims.
    ///
    /// Production change that fails this: recording the read's LENGTH as its
    /// address (or vice versa), dropping the pointer scan, attributing reads to
    /// the wrong domain, or writing entries the census cannot pair back up.
    #[test]
    fn a_traced_frame_reproduces_a_known_read_sequence_exactly() {
        let _guard = lock();
        let path = temp_path("known");
        test_support::reset(&path, 4096);

        // Frame boundary 1: start recording.
        on_frame_boundary();
        assert!(test_support::capturing(), "the first boundary must arm");

        // Domain 47 reads a root, gets a pointer, follows it, and re-reads the
        // same eight bytes twice.
        let object = 0x200_0000_0000u64;
        let restore = enter_domain(47);
        note_read(0x1000, 8, &object.to_le_bytes(), LO, HI);
        note_read(object as usize, 8, &[0u8; 8], LO, HI);
        note_read(object as usize, 8, &[0u8; 8], LO, HI);
        // A field 0x10 into the same object: a second distinct range.
        note_read(object as usize + 0x10, 4, &[0u8; 4], LO, HI);
        leave_domain(restore);
        flush_after_bracket();

        // Domain 60 reads bytes that OVERLAP domain 47's object.
        let restore = enter_domain(60);
        note_read(object as usize + 0x8, 0x10, &[0u8; 0x10], LO, HI);
        leave_domain(restore);
        flush_after_bracket();

        // Frame boundary 2: stop and write.
        on_frame_boundary();
        assert!(!test_support::capturing(), "the second boundary must close");

        let mut file = std::fs::File::open(&path).expect("sidecar written");
        let (header, entries) = read_trace(&mut file, 4096).expect("sidecar parses");
        assert!(!header.truncated(), "this trace fits well inside the cap");
        assert_eq!(header.frame_ordinal, 1, "exactly one frame was traced");

        // This test's own reads only; see the module-level note on why a total
        // is not a property of the tracer.
        let census = Census::build(header, &entries, DerivationSpan(0x1000));
        let d47 = census
            .brackets
            .iter()
            .find(|row| row.bracket_tag == 47)
            .expect("domain 47 row");
        assert_eq!(d47.read_count, 4, "four reads were issued by domain 47");
        assert_eq!(
            d47.bytes_moved,
            8 + 8 + 8 + 4,
            "the reader moved 28 bytes across those four reads"
        );
        assert_eq!(
            d47.distinct_ranges, 3,
            "(0x1000,8), (object,8) twice = one, and (object+0x10,4)"
        );
        assert_eq!(
            d47.unique_bytes,
            8 + 8 + 4,
            "0x1000..0x1008, object..object+8 and object+0x10..object+0x14 \
             are disjoint: 20 bytes"
        );
        assert_eq!(
            d47.max_depth, 1,
            "the object reads are one hop from the 0x1000 root; nothing they \
             returned was heap-shaped, so no chain continues"
        );

        let d60 = census
            .brackets
            .iter()
            .find(|row| row.bracket_tag == 60)
            .expect("domain 60 row");
        assert_eq!(d60.unique_bytes, 0x10);

        // object..object+8 (d47) and object+8..object+0x18 (d60) are adjacent,
        // and object+0x10..object+0x14 lies inside d60's span.
        assert_eq!(
            census.full_set_unique_bytes,
            8 + 0x18,
            "0x1000..0x1008 plus the merged object..object+0x18"
        );
        assert_eq!(
            census.shared_bytes,
            (d47.unique_bytes + d60.unique_bytes) - census.full_set_unique_bytes,
            "the 4 bytes at object+0x10 are read by both domains and must be \
             counted once in the full set"
        );
        assert_eq!(census.shared_bytes, 4);

        let _ = std::fs::remove_file(&path);
    }

    /// A read taken with no bracket held is attributed to nobody, not to
    /// whichever domain happened to read last.
    ///
    /// Production change that fails this: making `leave_domain` clear the slot
    /// to a real domain id, or having `note_read` fall back to a "last domain"
    /// cache.
    #[test]
    fn an_unbracketed_read_is_attributed_to_no_domain() {
        let _guard = lock();
        let path = temp_path("unattributed");
        test_support::reset(&path, 4096);
        on_frame_boundary();

        let restore = enter_domain(53);
        note_read(0x2000, 8, &[0u8; 8], LO, HI);
        leave_domain(restore);
        flush_after_bracket();
        // Outside every bracket.
        note_read(0x3000, 8, &[0u8; 8], LO, HI);

        on_frame_boundary();
        let mut file = std::fs::File::open(&path).expect("sidecar written");
        let (header, entries) = read_trace(&mut file, 4096).expect("sidecar parses");
        let census = Census::build(header, &entries, DerivationSpan(0x1000));
        let orphan = census
            .brackets
            .iter()
            .find(|row| row.bracket_tag == TAG_UNATTRIBUTED)
            .expect("the unbracketed read has its own row");
        assert_eq!(
            orphan.read_count, 1,
            "the 0x3000 read, and only it, is unattributed"
        );
        assert_eq!(
            census
                .brackets
                .iter()
                .find(|row| row.bracket_tag == 53)
                .map(|row| row.read_count),
            Some(1),
            "the bracketed read stays with domain 53"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// Nested brackets restore the ENCLOSING domain, so the outer reader's
    /// reads after an inner bracket closes are still its own.
    ///
    /// Production change that fails this: `leave_domain` resetting to
    /// `TAG_UNATTRIBUTED` instead of to the saved value.
    #[test]
    fn a_nested_bracket_hands_the_thread_back_to_its_enclosing_domain() {
        let _guard = lock();
        let path = temp_path("nested");
        test_support::reset(&path, 4096);
        on_frame_boundary();

        let outer = enter_domain(86);
        note_read(0x4000, 8, &[0u8; 8], LO, HI);
        let inner = enter_domain(90);
        note_read(0x5000, 8, &[0u8; 8], LO, HI);
        leave_domain(inner);
        flush_after_bracket();
        note_read(0x6000, 8, &[0u8; 8], LO, HI);
        leave_domain(outer);
        flush_after_bracket();

        on_frame_boundary();
        let mut file = std::fs::File::open(&path).expect("sidecar written");
        let (header, entries) = read_trace(&mut file, 4096).expect("sidecar parses");
        let census = Census::build(header, &entries, DerivationSpan(0x1000));
        assert_eq!(
            census
                .brackets
                .iter()
                .find(|row| row.bracket_tag == 86)
                .map(|row| row.read_count),
            Some(2),
            "0x4000 and 0x6000 both belong to the outer bracket"
        );
        assert_eq!(
            census
                .brackets
                .iter()
                .find(|row| row.bracket_tag == 90)
                .map(|row| row.read_count),
            Some(1)
        );
        let _ = std::fs::remove_file(&path);
    }

    /// Hitting the cap STOPS the trace and marks it, rather than silently
    /// producing a short file that reads like a complete census.
    ///
    /// Production change that fails this: dropping the `TRUNCATED` flag, or
    /// letting the buffer grow past the cap.
    #[test]
    fn exceeding_the_entry_cap_marks_the_sidecar_truncated() {
        let _guard = lock();
        let path = temp_path("capped");
        test_support::reset(&path, 4);
        on_frame_boundary();

        let restore = enter_domain(47);
        for i in 0..32u64 {
            note_read(0x7000 + (i as usize) * 8, 8, &[0u8; 8], LO, HI);
        }
        leave_domain(restore);
        flush_after_bracket();

        on_frame_boundary();
        let mut file = std::fs::File::open(&path).expect("sidecar written");
        let (header, entries) = read_trace(&mut file, 4096).expect("sidecar parses");
        assert!(
            header.truncated(),
            "the cap was hit, so the sidecar must say it is a lower bound"
        );
        assert!(
            entries.len() <= 4,
            "recording stopped at the cap; got {} entries",
            entries.len()
        );
        assert!(
            !entries.is_empty(),
            "a capped trace must still deliver what it did record; an empty \
             file would be indistinguishable from a frame that read nothing"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// Only ONE frame is traced: the boundary after the write does not restart
    /// recording, so a long capture cannot fill a disk.
    ///
    /// Production change that fails this: leaving `ARMED` set after the write.
    #[test]
    fn exactly_one_frame_is_traced_per_arm() {
        let _guard = lock();
        let path = temp_path("one_frame");
        test_support::reset(&path, 4096);
        on_frame_boundary();
        assert!(test_support::capturing());
        // A read must exist for the trace to be closeable; see
        // `two_boundaries_with_no_read_between_them_do_not_close_a_trace`.
        let restore = enter_domain(11);
        note_read(0x1000, 8, &[0u8; 8], LO, HI);
        leave_domain(restore);
        flush_after_bracket();
        on_frame_boundary();
        assert!(!test_support::capturing());
        on_frame_boundary();
        assert!(
            !test_support::capturing(),
            "a third boundary must NOT start a second trace"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// A trace only closes over an iteration in which a DOMAIN actually read.
    ///
    /// WHY IT MATTERS, and why "no read at all" is the wrong condition. The
    /// boundary runs before the scheduler decides whether this hour is swept.
    /// An idle scheduler yields no sweep and the loop comes round again having
    /// extracted nothing — but it is NOT a read-free iteration, because the
    /// capture loop samples the game clock through the same `read_scalar` the
    /// tracer instruments. A guard counting all reads is satisfied by that
    /// clock sample alone and closes a trace over an hour that censused
    /// nothing. Only reads taken inside a bracket are a domain's, so only they
    /// may close a trace.
    ///
    /// This walks the exact sequence: an unbracketed clock read (which must NOT
    /// close), then a real bracketed read (which must).
    ///
    /// Production change that fails this: counting `RECORDED` instead of
    /// `BRACKETED_READS` in `on_frame_boundary`, or dropping the guard and
    /// closing on any second boundary.
    #[test]
    fn only_a_bracketed_read_lets_a_boundary_close_the_trace() {
        let _guard = lock();
        let path = temp_path("idle_frame");
        let _ = std::fs::remove_file(&path);
        test_support::reset(&path, 4096);

        on_frame_boundary();
        assert!(test_support::capturing());

        // Iteration 1: literally nothing read.
        on_frame_boundary();
        assert!(
            test_support::capturing(),
            "the trace must stay open across a boundary that saw no read"
        );

        // Iteration 2: the capture loop's own clock sample and domain 47's
        // serial prewarm — both taken OUTSIDE any bracket. This is the case a
        // naive read-count guard gets wrong.
        note_read(0x2_0000, 8, &[0u8; 8], LO, HI);
        note_read(0x2_1000, 8, &[0u8; 8], LO, HI);
        on_frame_boundary();
        assert!(
            test_support::capturing(),
            "an unbracketed clock read is not a domain extraction and must not \
             close the trace"
        );
        assert!(
            !std::path::Path::new(&path).exists(),
            "and nothing may be written yet"
        );

        // Iteration 3: a domain actually reads. NOW it closes.
        let restore = enter_domain(47);
        note_read(0xA000, 8, &[0u8; 8], LO, HI);
        leave_domain(restore);
        flush_after_bracket();
        on_frame_boundary();
        assert!(!test_support::capturing());

        let mut file = std::fs::File::open(&path).expect("now the sidecar exists");
        let (header, entries) = read_trace(&mut file, 4096).expect("sidecar parses");
        let census = Census::build(header, &entries, DerivationSpan(0x1000));
        assert_eq!(
            census
                .brackets
                .iter()
                .find(|row| row.bracket_tag == 47)
                .map(|row| row.read_count),
            Some(1),
            "the census covers the frame in which domain 47 read"
        );

        // THE INVARIANT THE DISCARD EXISTS FOR: the sidecar covers ONE
        // interval. The two unbracketed reads of iteration 2 belong to a
        // discarded interval and must not be in it — a guard that merely
        // WAITED instead of discarding would carry them here and publish a
        // multi-interval census under a single frame ordinal.
        assert_eq!(
            entries.len(),
            1,
            "of this test's three reads only the closing interval's survives; \
             got {entries:?}"
        );
        assert!(
            !entries
                .iter()
                .any(|entry| entry.value == 0x2_0000 || entry.value == 0x2_1000),
            "no read from the discarded interval may survive anywhere in the file"
        );
        assert_eq!(
            header.frame_ordinal, 3,
            "the ordinal names which interval the census covers, not the first"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// An unattributed read taken on a DIFFERENT, still-running thread is
    /// reachable by a discard and never leaks into the next interval.
    ///
    /// WHY A SEPARATE THREAD IS THE POINT. The long-lived scheduler observer
    /// samples the clock through the instrumented primitives, never opens a
    /// bracket, and is joined only at shutdown, so it has no flush point. If
    /// unattributed reads went into a per-thread buffer, its entries would be
    /// unreachable by `discard_open_interval` (which cannot touch another
    /// thread's TLS), would survive into the interval that eventually closes,
    /// and — because the discard resets `RECORDED` — would escape the cap that
    /// bounds the tracer's memory. A single-threaded test cannot see any of
    /// that.
    ///
    /// Production change that fails this: routing unattributed reads through
    /// `BUFFER` instead of straight to `SINK`.
    #[test]
    fn an_unattributed_read_from_another_thread_is_cleared_by_a_discard() {
        let _guard = lock();
        let path = temp_path("other_thread");
        let _ = std::fs::remove_file(&path);
        test_support::reset(&path, 4096);
        on_frame_boundary();

        // A thread that never opens a bracket and never flushes, exactly like
        // the scheduler observer. It stays alive past the boundary below.
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        let observer = std::thread::spawn(move || {
            test_support::enroll_current_thread();
            note_read(0x3_0000, 8, &[0u8; 8], LO, HI);
            done_tx.send(()).expect("observer reported");
            rx.recv().expect("observer released");
        });
        done_rx.recv().expect("the observer read before the boundary");

        // Barren interval: the only read was that unattributed one.
        on_frame_boundary();
        assert!(test_support::capturing(), "no bracketed read, so no close");

        // Now a real domain frame, on this thread.
        let restore = enter_domain(47);
        note_read(0xB000, 8, &[0u8; 8], LO, HI);
        leave_domain(restore);
        flush_after_bracket();
        on_frame_boundary();
        assert!(!test_support::capturing());

        tx.send(()).expect("release the observer");
        observer.join().expect("observer joins");

        let mut file = std::fs::File::open(&path).expect("sidecar written");
        let (_header, entries) = read_trace(&mut file, 4096).expect("sidecar parses");
        assert!(
            !entries.iter().any(|entry| entry.value == 0x3_0000),
            "the other thread's read belongs to a discarded interval and must \
             not survive into the file"
        );
        assert_eq!(
            entries.len(),
            1,
            "only the closing interval's read; got {entries:?}"
        );
        assert_eq!(entries[0].bracket_tag, 47);
        assert_eq!(entries[0].value, 0xB000);
        let _ = std::fs::remove_file(&path);
    }

    /// THE RACE, DRIVEN DETERMINISTICALLY. A discard cannot slip between an
    /// unattributed read's cap reservation and its append.
    ///
    /// WHY THIS NEEDED AN INJECTION POINT. `note_read` reserved against
    /// `RECORDED` and then appended to `SINK`; `discard_open_interval` cleared
    /// the sink and reset the counter. An observer that reserved before a
    /// discard and appended after it left an entry the reset counter did not
    /// account for — and since the scheduler observer runs concurrently with
    /// frame boundaries for the whole capture, repeating that around successive
    /// discards escapes the memory cap without bound. The window is a few
    /// instructions wide, so a timing test would not find it; this holds a
    /// recorder inside the window by force.
    ///
    /// THE ORDERING IT FORCES, and the one thing it cannot. The recorder is
    /// held inside the critical section until a SEPARATE thread has entered
    /// `discard_open_interval`. Under the correct code that discard blocks on
    /// the sink lock and can only observe the reservation and the entry
    /// together. Under the faulty ordering — counters reset before the lock is
    /// taken — the reset lands first, the recorder then reserves and appends,
    /// and the discard finally clears the sink, leaving a nonzero counter over
    /// an empty sink: the accounting leak itself, and what the assertions below
    /// catch.
    ///
    /// NO TIMING ASSUMPTION, and that took a second signal to achieve. Knowing
    /// the discard has "started" is not enough — it can be descheduled between
    /// starting and reaching the lock, and if it resumes after the recorder has
    /// already appended and unlocked, the faulty implementation also produces
    /// the passing result (Codex review 019fda22-fabd-7121-8629-c58c4d792e08).
    /// So `discard_open_interval` carries its own `#[cfg(test)]` signal fired
    /// immediately before its lock attempt, AFTER every pre-lock statement.
    /// Receiving it means the discard has done whatever it does outside the
    /// lock — which, in the faulty ordering, is the counter reset — and is
    /// about to block. Only then does the recorder proceed. Both orderings are
    /// forced, not raced.
    #[test]
    fn the_cap_counter_and_the_sink_never_disagree_across_a_discard() {
        use std::sync::mpsc::{channel, RecvTimeoutError};
        use std::time::Duration;

        let _guard = lock();
        let path = temp_path("reserve_race");
        let _ = std::fs::remove_file(&path);
        test_support::reset(&path, 4096);
        on_frame_boundary();

        let (at_hook_tx, at_hook_rx) = channel::<()>();
        let (at_lock_tx, at_lock_rx) = channel::<()>();

        // Armed before the discard thread exists, so it cannot be missed.
        test_support::set_discard_pre_lock_hook(move || {
            at_lock_tx.send(()).expect("discard reached its lock attempt");
        });
        test_support::set_pre_reserve_hook(move || {
            // We hold the sink lock here. Announce that, then stay until the
            // discard is at its own lock attempt with its pre-lock work done.
            at_hook_tx.send(()).expect("recorder reported");
            match at_lock_rx.recv_timeout(Duration::from_secs(10)) {
                Ok(()) | Err(RecvTimeoutError::Disconnected) => {}
                Err(RecvTimeoutError::Timeout) => {
                    panic!("the discard never reached its lock attempt")
                }
            }
        });

        let recorder = std::thread::spawn(move || {
            test_support::enroll_current_thread();
            note_read(0xC000, 8, &[0u8; 8], LO, HI);
        });
        at_hook_rx.recv().expect("the recorder reached the window");

        let discarder = std::thread::spawn(on_frame_boundary);

        recorder.join().expect("recorder joins");
        discarder.join().expect("discarder joins");

        let (sink_len, recorded) = test_support::sink_len_and_recorded();
        assert_eq!(
            sink_len, 0,
            "the discard ran after the append completed, so it cleared it"
        );
        assert_eq!(
            recorded, 0,
            "and the counter was reset in the same critical section; a nonzero \
             counter with an empty sink is exactly the accounting leak that lets \
             the memory cap be escaped"
        );
        assert!(
            test_support::capturing(),
            "no bracketed read happened, so the interval was discarded rather \
             than closed"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// A qword that is not heap-shaped is not recorded as a pointer, so a
    /// counter or a float cannot manufacture a discovery chain.
    ///
    /// Production change that fails this: dropping the heap-window test in the
    /// pointer scan.
    #[test]
    fn only_heap_shaped_qwords_become_pointer_entries() {
        let _guard = lock();
        let path = temp_path("heap_window");
        test_support::reset(&path, 4096);
        on_frame_boundary();

        let restore = enter_domain(41);
        // A small integer and a value above the heap window: neither is a
        // pointer. One value inside the window is.
        note_read(0x8000, 24, &{
            let mut bytes = [0u8; 24];
            bytes[0..8].copy_from_slice(&7u64.to_le_bytes());
            bytes[8..16].copy_from_slice(&0xFFFF_0000_0000_0000u64.to_le_bytes());
            bytes[16..24].copy_from_slice(&0x300_0000_0000u64.to_le_bytes());
            bytes
        }, LO, HI);
        leave_domain(restore);
        flush_after_bracket();

        on_frame_boundary();
        let mut file = std::fs::File::open(&path).expect("sidecar written");
        let (_header, entries) = read_trace(&mut file, 4096).expect("sidecar parses");
        let pointers: Vec<u64> = entries
            .iter()
            .filter(|entry| entry.kind == TRACE_KIND_POINTER)
            .map(|entry| entry.value)
            .collect();
        assert_eq!(
            pointers,
            vec![0x300_0000_0000u64],
            "only the value inside [{LO:#x}, {HI:#x}) is a pointer candidate"
        );
        let _ = std::fs::remove_file(&path);
    }
}
