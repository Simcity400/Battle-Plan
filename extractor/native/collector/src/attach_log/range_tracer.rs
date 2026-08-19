//! thread-local on nest, so the tracer rides that lifetime exactly. A read
//! `cargo build -p collector` carries none of it, and the release/package
//!    cargo build -p collector --release --features range-tracer
//!    cargo build -p recorder_host --release

use std::cell::{Cell, RefCell};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

use crate::range_census::{
    TraceEntry, TraceHeader, TRACE_ENTRY_LEN, TRACE_FLAG_TRUNCATED, TRACE_KIND_POINTER,
    TRACE_KIND_READ, TRACE_VERSION,
};

pub const TRACE_PATH_ENV: &str = "BATTLE_PLAN_RANGE_TRACE_PATH";
pub const TRACE_MAX_ENTRIES_ENV: &str = "BATTLE_PLAN_RANGE_TRACE_MAX_ENTRIES";

pub const DEFAULT_MAX_ENTRIES: usize = 64 * 1024 * 1024;

pub const TAG_UNATTRIBUTED: u16 = u16::MAX;

static ARMED: AtomicBool = AtomicBool::new(false);
static CAPTURING: AtomicBool = AtomicBool::new(false);
static NEXT_SEQ: AtomicU64 = AtomicU64::new(0);
static RECORDED: AtomicUsize = AtomicUsize::new(0);
static BRACKETED_READS: AtomicU64 = AtomicU64::new(0);
static TRUNCATED: AtomicBool = AtomicBool::new(false);
static FRAME_ORDINAL: AtomicU64 = AtomicU64::new(0);
static WINDOW_LO: AtomicUsize = AtomicUsize::new(0);
static WINDOW_HI: AtomicUsize = AtomicUsize::new(0);

pub fn set_heap_window(lo: usize, hi: usize) {
    WINDOW_LO.store(lo, Ordering::SeqCst);
    WINDOW_HI.store(hi, Ordering::SeqCst);
}

struct TraceConfig {
    path: String,
    max_entries: usize,
}

static CONFIG: Mutex<Option<TraceConfig>> = Mutex::new(None);
static SINK: Mutex<Vec<TraceEntry>> = Mutex::new(Vec::new());

thread_local! {
    static TRACE_DOMAIN: Cell<u16> = const { Cell::new(TAG_UNATTRIBUTED) };
    static BUFFER: RefCell<Vec<TraceEntry>> = const { RefCell::new(Vec::new()) };
}

pub fn arm_from_env() {
    if ARMED.load(Ordering::SeqCst) {
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

pub fn on_frame_boundary() {
    static ARM_ONCE: std::sync::Once = std::sync::Once::new();
    ARM_ONCE.call_once(arm_from_env);
    if CAPTURING.load(Ordering::SeqCst) {
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

///    no bracket lifetime (the long-lived scheduler observer) leaves nothing a
fn discard_open_interval() {
    BUFFER.with(|buffer| buffer.borrow_mut().clear());
    #[cfg(test)]
    if let Some(hook) = test_support::take_discard_pre_lock_hook() {
        hook();
    }
    let Ok(mut sink) = SINK.lock() else {
        return;
    };
    sink.clear();
    RECORDED.store(0, Ordering::SeqCst);
    BRACKETED_READS.store(0, Ordering::SeqCst);
    drop(sink);
    FRAME_ORDINAL.fetch_add(1, Ordering::SeqCst);
}

pub fn enter_domain(bracket_tag: u16) -> u16 {
    TRACE_DOMAIN.with(|slot| slot.replace(bracket_tag))
}

pub fn leave_domain(restore: u16) {
    TRACE_DOMAIN.with(|slot| slot.set(restore));
}

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

pub fn note_read(addr: usize, len: usize, bytes: &[u8], heap_lo: usize, heap_hi: usize) {
    if !CAPTURING.load(Ordering::Relaxed) {
        return;
    }
    #[cfg(test)]
    if !test_support::thread_is_enrolled() {
        return;
    }
    if TRUNCATED.load(Ordering::Relaxed) {
        return;
    }
    let bracket_tag = TRACE_DOMAIN.with(Cell::get);
    let pointer_slots = bytes.len() / 8;
    let want = 1 + pointer_slots;
    let max_entries = max_entries();
    let unattributed = bracket_tag == TAG_UNATTRIBUTED;

    let sink_guard = if unattributed {
        match SINK.lock() {
            Ok(guard) => Some(guard),
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

fn write_sidecar() {
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

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::sync::{Mutex as StdMutex, MutexGuard};

    pub static TRACER_LOCK: StdMutex<()> = StdMutex::new(());

    pub fn lock() -> MutexGuard<'static, ()> {
        TRACER_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    static REQUIRED_GENERATION: AtomicU64 = AtomicU64::new(0);

    thread_local! {
        static ENROLLED_GENERATION: Cell<u64> = const { Cell::new(0) };
    }

    pub fn thread_is_enrolled() -> bool {
        let required = REQUIRED_GENERATION.load(Ordering::Relaxed);
        required == 0 || ENROLLED_GENERATION.with(Cell::get) == required
    }

    pub fn enroll_current_thread() {
        let required = REQUIRED_GENERATION.load(Ordering::SeqCst);
        ENROLLED_GENERATION.with(|slot| slot.set(required));
    }

    type Hook = Box<dyn FnOnce() + Send>;

    static PRE_RESERVE_HOOK: Mutex<Option<Hook>> = Mutex::new(None);

    pub fn set_pre_reserve_hook(hook: impl FnOnce() + Send + 'static) {
        if let Ok(mut slot) = PRE_RESERVE_HOOK.lock() {
            *slot = Some(Box::new(hook));
        }
    }

    pub fn take_pre_reserve_hook(unattributed: bool) -> Option<Hook> {
        if !unattributed {
            return None;
        }
        PRE_RESERVE_HOOK
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
    }

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

    pub fn sink_len_and_recorded() -> (usize, usize) {
        let len = SINK.lock().map(|sink| sink.len()).unwrap_or(0);
        (len, RECORDED.load(Ordering::SeqCst))
    }

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

    fn temp_path(name: &str) -> String {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "hoi4_range_tracer_{name}_{}.bin",
            std::process::id()
        ));
        path.to_string_lossy().into_owned()
    }

    const LO: usize = 0x100_0000_0000;
    const HI: usize = 0x7FF0_0000_0000;

    #[test]
    fn a_traced_frame_reproduces_a_known_read_sequence_exactly() {
        let _guard = lock();
        let path = temp_path("known");
        test_support::reset(&path, 4096);

        on_frame_boundary();
        assert!(test_support::capturing(), "the first boundary must arm");

        let object = 0x200_0000_0000u64;
        let restore = enter_domain(47);
        note_read(0x1000, 8, &object.to_le_bytes(), LO, HI);
        note_read(object as usize, 8, &[0u8; 8], LO, HI);
        note_read(object as usize, 8, &[0u8; 8], LO, HI);
        note_read(object as usize + 0x10, 4, &[0u8; 4], LO, HI);
        leave_domain(restore);
        flush_after_bracket();

        let restore = enter_domain(60);
        note_read(object as usize + 0x8, 0x10, &[0u8; 0x10], LO, HI);
        leave_domain(restore);
        flush_after_bracket();

        on_frame_boundary();
        assert!(!test_support::capturing(), "the second boundary must close");

        let mut file = std::fs::File::open(&path).expect("sidecar written");
        let (header, entries) = read_trace(&mut file, 4096).expect("sidecar parses");
        assert!(!header.truncated(), "this trace fits well inside the cap");
        assert_eq!(header.frame_ordinal, 1, "exactly one frame was traced");

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

    #[test]
    fn exactly_one_frame_is_traced_per_arm() {
        let _guard = lock();
        let path = temp_path("one_frame");
        test_support::reset(&path, 4096);
        on_frame_boundary();
        assert!(test_support::capturing());
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

    #[test]
    fn only_a_bracketed_read_lets_a_boundary_close_the_trace() {
        let _guard = lock();
        let path = temp_path("idle_frame");
        let _ = std::fs::remove_file(&path);
        test_support::reset(&path, 4096);

        on_frame_boundary();
        assert!(test_support::capturing());

        on_frame_boundary();
        assert!(
            test_support::capturing(),
            "the trace must stay open across a boundary that saw no read"
        );

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

    #[test]
    fn an_unattributed_read_from_another_thread_is_cleared_by_a_discard() {
        let _guard = lock();
        let path = temp_path("other_thread");
        let _ = std::fs::remove_file(&path);
        test_support::reset(&path, 4096);
        on_frame_boundary();

        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        let observer = std::thread::spawn(move || {
            test_support::enroll_current_thread();
            note_read(0x3_0000, 8, &[0u8; 8], LO, HI);
            done_tx.send(()).expect("observer reported");
            rx.recv().expect("observer released");
        });
        done_rx
            .recv()
            .expect("the observer read before the boundary");

        on_frame_boundary();
        assert!(test_support::capturing(), "no bracketed read, so no close");

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

        test_support::set_discard_pre_lock_hook(move || {
            at_lock_tx
                .send(())
                .expect("discard reached its lock attempt");
        });
        test_support::set_pre_reserve_hook(move || {
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

    #[test]
    fn only_heap_shaped_qwords_become_pointer_entries() {
        let _guard = lock();
        let path = temp_path("heap_window");
        test_support::reset(&path, 4096);
        on_frame_boundary();

        let restore = enter_domain(41);
        note_read(
            0x8000,
            24,
            &{
                let mut bytes = [0u8; 24];
                bytes[0..8].copy_from_slice(&7u64.to_le_bytes());
                bytes[8..16].copy_from_slice(&0xFFFF_0000_0000_0000u64.to_le_bytes());
                bytes[16..24].copy_from_slice(&0x300_0000_0000u64.to_le_bytes());
                bytes
            },
            LO,
            HI,
        );
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
