//! Allocation-free, first-failure terminal witness and fault injection.

use replay::witness::{TerminalWitness, WITNESS_BYTES};
use std::{
    cell::{Cell, UnsafeCell},
    fs::File,
    io,
    os::windows::io::AsRawHandle,
    sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering, fence},
};
use windows_sys::Win32::Storage::FileSystem::{FlushFileBuffers, WriteFile};

pub(crate) use replay::witness::{FailureCode as Code, FailurePhase as Phase};

#[derive(Clone, Copy, Default)]
pub(crate) struct Context {
    pub(crate) campaign_id: Option<[u8; 36]>,
    pub(crate) branch: Option<u64>,
    pub(crate) load_signal: Option<i64>,
    pub(crate) failed_hour: Option<i32>,
}

thread_local! {
    static PHASE: Cell<Phase> = const { Cell::new(Phase::Bootstrap) };
    static CONTEXT: Cell<Context> = const { Cell::new(Context {
        campaign_id: None, branch: None, load_signal: None, failed_hour: None,
    }) };
    static BORROWED_CONTEXT: Cell<*const Context> = const { Cell::new(std::ptr::null()) };
    static REQUEST: Cell<Option<(&'static str, usize)>> = const { Cell::new(None) };
}

static DETAIL_HANDLE: AtomicUsize = AtomicUsize::new(0);

pub(crate) struct Details {
    _file: File,
}
impl Drop for Details {
    fn drop(&mut self) {
        DETAIL_HANDLE.store(0, Ordering::Release);
    }
}

/// Best-effort diagnostic storage prepared before worker admission.
pub(crate) fn prepare_details(directory: &std::path::Path) -> Option<Details> {
    let file = File::create(directory.join("collector-failure-detail.json")).ok()?;
    DETAIL_HANDLE.store(file.as_raw_handle() as usize, Ordering::Release);
    Some(Details { _file: file })
}

/// The request is scoped to the allocation, so another thread's terminal
/// failure cannot inherit this worker's family or requested byte count.
pub(crate) fn reserve_raw<T>(
    values: &mut Vec<T>,
    target: usize,
    family: &'static str,
) -> io::Result<()> {
    struct Restore(Option<(&'static str, usize)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            REQUEST.set(self.0);
        }
    }
    let _request = Restore(REQUEST.replace(Some((
        family,
        target.saturating_mul(std::mem::size_of::<T>()),
    ))));
    engine::allocation::reserve(values, target.saturating_sub(values.len()))
}

fn write_details(record: &TerminalWitness) {
    use std::fmt::Write;
    let handle = DETAIL_HANDLE.load(Ordering::Acquire);
    let Some((family, requested_bytes)) = REQUEST.get() else {
        return;
    };
    if handle == 0 {
        return;
    }
    struct Buffer {
        bytes: [u8; 256],
        len: usize,
    }
    impl std::fmt::Write for Buffer {
        fn write_str(&mut self, value: &str) -> std::fmt::Result {
            let end = self.len.checked_add(value.len()).ok_or(std::fmt::Error)?;
            self.bytes
                .get_mut(self.len..end)
                .ok_or(std::fmt::Error)?
                .copy_from_slice(value.as_bytes());
            self.len = end;
            Ok(())
        }
    }
    let mut buffer = Buffer {
        bytes: [0; 256],
        len: 0,
    };
    if writeln!(
        buffer,
        "{{\"session\":{},\"process_start_time\":{},\"family\":\"{}\",\"requested_bytes\":{}}}",
        record.session, record.process_start_time, family, requested_bytes
    )
    .is_err()
    {
        return;
    }
    let mut written = 0;
    unsafe {
        WriteFile(
            handle as _,
            buffer.bytes.as_ptr(),
            buffer.len as u32,
            &mut written,
            std::ptr::null_mut(),
        );
        FlushFileBuffers(handle as _);
    }
}

pub(crate) struct Scope(Phase);
impl Drop for Scope {
    fn drop(&mut self) {
        PHASE.set(self.0);
        #[cfg(test)]
        crate::allocation_failure_tests::phase_enter(self.0);
    }
}
pub(crate) fn enter(phase: Phase) -> Scope {
    #[cfg(test)]
    crate::allocation_failure_tests::phase_enter(phase);
    Scope(PHASE.replace(phase))
}
pub(crate) fn set_context(context: Context) {
    CONTEXT.set(context);
}
pub(crate) fn context() -> Context {
    let borrowed = BORROWED_CONTEXT.get();
    if borrowed.is_null() {
        CONTEXT.get()
    } else {
        unsafe { *borrowed }
    }
}
fn with_context<R>(read: impl FnOnce(&Context) -> R) -> R {
    let borrowed = BORROWED_CONTEXT.get();
    if borrowed.is_null() {
        CONTEXT.with(|context| read(unsafe { &*context.as_ptr() }))
    } else {
        read(unsafe { &*borrowed })
    }
}
pub(crate) struct BorrowedScope(*const Context);
impl Drop for BorrowedScope {
    fn drop(&mut self) {
        BORROWED_CONTEXT.set(self.0);
    }
}
/// The caller keeps this immutable context live until the returned guard drops.
pub(crate) unsafe fn borrow_context(pointer: *const Context) -> BorrowedScope {
    BorrowedScope(BORROWED_CONTEXT.replace(pointer))
}
pub(crate) fn attempted_hour(hour: Option<i32>) {
    CONTEXT.with(|context| {
        let mut value = context.get();
        value.failed_hour = hour;
        context.set(value);
    });
}

// 0 healthy, 1 claimed, 2 fully published. The witness writer is the only
// writer of Terminal::record, and publication follows the complete write.
struct Terminal {
    state: AtomicU32,
    record: UnsafeCell<Option<TerminalWitness>>,
    buffer: UnsafeCell<[u8; WITNESS_BYTES]>,
    handle: AtomicUsize,
    session: AtomicU64,
    process_start: AtomicU64,
    committed_segment: AtomicU64,
    committed_bytes: AtomicU64,
    committed_sequence: AtomicU64,
    committed_campaign: [AtomicU64; 5],
    committed_branch: AtomicU64,
    committed_signal: AtomicU64,
    committed_flags: AtomicU32,
    pending_campaign: [AtomicU64; 5],
    pending_segment: AtomicU64,
    pending_bytes: AtomicU64,
    pending_flags: AtomicU32,
}
unsafe impl Sync for Terminal {}
static TERMINAL: Terminal = Terminal {
    state: AtomicU32::new(0),
    record: UnsafeCell::new(None),
    buffer: UnsafeCell::new([0; WITNESS_BYTES]),
    handle: AtomicUsize::new(0),
    session: AtomicU64::new(0),
    process_start: AtomicU64::new(0),
    committed_segment: AtomicU64::new(u64::MAX),
    committed_bytes: AtomicU64::new(0),
    committed_sequence: AtomicU64::new(0),
    committed_campaign: [const { AtomicU64::new(0) }; 5],
    committed_branch: AtomicU64::new(0),
    committed_signal: AtomicU64::new(0),
    committed_flags: AtomicU32::new(0),
    pending_campaign: [const { AtomicU64::new(0) }; 5],
    pending_segment: AtomicU64::new(u64::MAX),
    pending_bytes: AtomicU64::new(0),
    pending_flags: AtomicU32::new(0),
};

/// Called while bootstrap owns the file, before hooks or workers exist.
pub(crate) fn prepare(file: &File, session: u64, process_start_time: u64) {
    unsafe {
        (*TERMINAL.buffer.get()).fill(0);
    }
    TERMINAL.session.store(session, Ordering::Relaxed);
    TERMINAL
        .process_start
        .store(process_start_time, Ordering::Relaxed);
    TERMINAL
        .handle
        .store(file.as_raw_handle() as usize, Ordering::Release);
}
pub(crate) fn clear_handle() {
    TERMINAL.handle.store(0, Ordering::Release);
}
pub(crate) fn failed() -> bool {
    TERMINAL.state.load(Ordering::Acquire) != 0
}

pub(crate) fn committed(segment: u32, bytes: u64) {
    if failed() {
        return;
    }
    let sequence = TERMINAL.committed_sequence.load(Ordering::Relaxed);
    TERMINAL
        .committed_sequence
        .store(sequence.wrapping_add(1), Ordering::Release);
    fence(Ordering::Release);
    with_context(|context| {
        let flags = u32::from(context.campaign_id.is_some())
            | (u32::from(context.branch.is_some()) << 1)
            | (u32::from(context.load_signal.is_some()) << 2);
        let word = |index: usize| {
            context.campaign_id.as_ref().map_or(0, |id| {
                if index == 4 {
                    u64::from(u32::from_le_bytes(id[32..36].try_into().unwrap()))
                } else {
                    u64::from_le_bytes(id[index * 8..index * 8 + 8].try_into().unwrap())
                }
            })
        };
        let changed = TERMINAL.committed_flags.load(Ordering::Relaxed) != flags
            || TERMINAL.committed_branch.load(Ordering::Relaxed) != context.branch.unwrap_or(0)
            || TERMINAL.committed_signal.load(Ordering::Relaxed)
                != context.load_signal.unwrap_or(0) as u64
            || TERMINAL
                .committed_campaign
                .iter()
                .enumerate()
                .any(|(index, slot)| slot.load(Ordering::Relaxed) != word(index));
        if changed {
            for (index, slot) in TERMINAL.committed_campaign.iter().enumerate() {
                slot.store(word(index), Ordering::Relaxed);
            }
            TERMINAL
                .committed_branch
                .store(context.branch.unwrap_or(0), Ordering::Relaxed);
            TERMINAL
                .committed_signal
                .store(context.load_signal.unwrap_or(0) as u64, Ordering::Relaxed);
            TERMINAL.committed_flags.store(flags, Ordering::Relaxed);
        }
    });
    TERMINAL.committed_bytes.store(bytes, Ordering::Relaxed);
    TERMINAL
        .committed_segment
        .store(u64::from(segment), Ordering::Relaxed);
    TERMINAL.pending_flags.store(0, Ordering::Relaxed);
    TERMINAL
        .committed_sequence
        .store(sequence.wrapping_add(2), Ordering::Release);
}

/// The sole spool writer advertises its old prefix before an append can write.
pub(crate) fn record_start(previous: Option<(u32, u64)>) {
    if failed() {
        return;
    }
    let sequence = TERMINAL.committed_sequence.load(Ordering::Relaxed);
    TERMINAL
        .committed_sequence
        .store(sequence.wrapping_add(1), Ordering::Release);
    fence(Ordering::Release);
    with_context(|context| {
        for (index, slot) in TERMINAL.pending_campaign.iter().enumerate() {
            let word = context.campaign_id.as_ref().map_or(0, |id| {
                if index == 4 {
                    u64::from(u32::from_le_bytes(id[32..36].try_into().unwrap()))
                } else {
                    u64::from_le_bytes(id[index * 8..index * 8 + 8].try_into().unwrap())
                }
            });
            if slot.load(Ordering::Relaxed) != word {
                slot.store(word, Ordering::Relaxed);
            }
        }
        TERMINAL
            .pending_flags
            .store(u32::from(context.campaign_id.is_some()), Ordering::Relaxed);
    });
    TERMINAL.pending_segment.store(
        previous.map_or(u64::MAX, |(segment, _)| u64::from(segment)),
        Ordering::Relaxed,
    );
    TERMINAL
        .pending_bytes
        .store(previous.map_or(0, |(_, bytes)| bytes), Ordering::Relaxed);
    TERMINAL
        .committed_sequence
        .store(sequence.wrapping_add(2), Ordering::Release);
}

#[cfg(test)]
pub(crate) fn failure() -> Option<TerminalWitness> {
    if TERMINAL.state.load(Ordering::Acquire) != 2 {
        return None;
    }
    // The first-failure record is immutable after release publication.
    unsafe { *TERMINAL.record.get() }
}

pub(crate) fn fail(phase: Phase, code: Code) {
    // Bootstrap through teardown shares the installed capture's terminal state.
    if !engine::allocation::capture_active() {
        return;
    }
    if TERMINAL
        .state
        .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    let mut context = context();
    if context
        .campaign_id
        .is_some_and(|id| !replay::identity::is_campaign_uuid(&id))
    {
        context.campaign_id = None;
    }
    let (
        segment,
        committed_bytes,
        committed_context,
        pending_campaign_id,
        pending_segment,
        pending_bytes,
    ) = loop {
        let before = TERMINAL.committed_sequence.load(Ordering::Acquire);
        if before & 1 != 0 {
            std::hint::spin_loop();
            continue;
        }
        let segment = TERMINAL.committed_segment.load(Ordering::Relaxed);
        let bytes = TERMINAL.committed_bytes.load(Ordering::Relaxed);
        let flags = TERMINAL.committed_flags.load(Ordering::Relaxed);
        let mut campaign = [0u8; 40];
        for (output, word) in campaign
            .chunks_exact_mut(8)
            .zip(&TERMINAL.committed_campaign)
        {
            output.copy_from_slice(&word.load(Ordering::Relaxed).to_le_bytes());
        }
        let committed_context = Context {
            campaign_id: (flags & 1 != 0).then(|| campaign[..36].try_into().unwrap()),
            branch: (flags & 2 != 0).then(|| TERMINAL.committed_branch.load(Ordering::Relaxed)),
            load_signal: (flags & 4 != 0)
                .then(|| TERMINAL.committed_signal.load(Ordering::Relaxed) as i64),
            failed_hour: None,
        };
        let mut pending_campaign = [0u8; 40];
        for (output, word) in pending_campaign
            .chunks_exact_mut(8)
            .zip(&TERMINAL.pending_campaign)
        {
            output.copy_from_slice(&word.load(Ordering::Relaxed).to_le_bytes());
        }
        let pending_campaign_id = (TERMINAL.pending_flags.load(Ordering::Relaxed) != 0)
            .then(|| pending_campaign[..36].try_into().unwrap());
        let pending_segment = u32::try_from(TERMINAL.pending_segment.load(Ordering::Relaxed)).ok();
        let pending_bytes = TERMINAL.pending_bytes.load(Ordering::Relaxed);
        fence(Ordering::Acquire);
        if TERMINAL.committed_sequence.load(Ordering::Acquire) == before {
            break (
                segment,
                bytes,
                committed_context,
                pending_campaign_id,
                pending_segment,
                pending_bytes,
            );
        }
    };
    if context.campaign_id.is_none()
        && context.failed_hour.is_none()
        && context.load_signal.is_none()
        && context.branch.is_none()
    {
        context = committed_context;
    } else if context.campaign_id == committed_context.campaign_id
        && context.load_signal == committed_context.load_signal
        && context.failed_hour.is_none()
        && context.branch.is_none()
    {
        context.branch = committed_context.branch;
    }
    let record = TerminalWitness {
        session: TERMINAL.session.load(Ordering::Relaxed),
        process_start_time: TERMINAL.process_start.load(Ordering::Relaxed),
        campaign_id: context.campaign_id,
        branch: context.branch,
        load_signal: context.load_signal,
        failed_hour: context.failed_hour,
        committed_segment: u32::try_from(segment).ok(),
        committed_bytes,
        committed_campaign_id: committed_context.campaign_id,
        pending_campaign_id,
        pending_segment: pending_campaign_id.and(pending_segment),
        pending_bytes: if pending_campaign_id.is_some() {
            pending_bytes
        } else {
            0
        },
        phase: phase as u8,
        code: code as u8,
        drain_complete: false,
    };
    // Only the first-failure winner writes this already committed buffer.
    write_details(&record);
    let bytes = unsafe { &mut *TERMINAL.buffer.get() };
    record.encode_into(bytes);
    let handle = TERMINAL.handle.load(Ordering::Acquire);
    if handle != 0 {
        let mut written = 0;
        unsafe {
            WriteFile(
                handle as _,
                bytes.as_ptr(),
                bytes.len() as u32,
                &mut written,
                std::ptr::null_mut(),
            );
            FlushFileBuffers(handle as _);
        }
    }
    unsafe {
        *TERMINAL.record.get() = Some(record);
    }
    TERMINAL.state.store(2, Ordering::Release);
    // A frame waiter must release the game callback immediately.
    let shared = crate::sampling::SHARED.load(Ordering::Acquire);
    if !shared.is_null() {
        unsafe {
            (*shared).cancel_buffer_wait.store(true, Ordering::Release);
        }
    }
}

pub(crate) fn oom() -> io::Error {
    engine::allocation::oom()
}
pub(crate) fn install_hooks() {
    engine::allocation::set_hooks(Some(should_fail), Some(failure_hook));
    engine::allocation::set_stop_hook(Some(failed));
}
fn failure_hook() {
    fail(PHASE.get(), Code::Reserve);
}
pub(crate) fn observe(error: &io::Error) {
    if engine::allocation::is_allocation_error(error) {
        let _ = engine::allocation::oom();
    }
}
pub(crate) fn native_error(error: &io::Error) {
    if engine::allocation::is_allocation_error(error) {
        fail(PHASE.get(), Code::Native);
    }
}
pub(crate) fn io(error: io::Error) -> io::Error {
    observe(&error);
    error
}
#[cfg(feature = "alloc-fault-injection")]
mod injection {
    use super::*;
    static PHASE: AtomicU32 = AtomicU32::new(0);
    static ORDINAL: AtomicU64 = AtomicU64::new(0);
    static COUNT: AtomicU64 = AtomicU64::new(0);
    static HOURS: AtomicU64 = AtomicU64::new(0);
    static AFTER_HOURS: AtomicU64 = AtomicU64::new(0);
    pub(super) fn configure(phase: super::Phase, ordinal: u64, after_hours: u64) {
        PHASE.store(phase as u32, Ordering::Relaxed);
        ORDINAL.store(ordinal, Ordering::Relaxed);
        COUNT.store(0, Ordering::Relaxed);
        HOURS.store(0, Ordering::Relaxed);
        AFTER_HOURS.store(after_hours, Ordering::Relaxed);
    }
    pub(super) fn should_fail() -> bool {
        HOURS.load(Ordering::Relaxed) >= AFTER_HOURS.load(Ordering::Relaxed)
            && PHASE.load(Ordering::Relaxed) == super::PHASE.get() as u32
            && COUNT.fetch_add(1, Ordering::Relaxed) + 1 == ORDINAL.load(Ordering::Relaxed)
    }
    pub(super) fn committed_hour() {
        HOURS.fetch_add(1, Ordering::Relaxed);
    }
    #[cfg(test)]
    pub(super) fn count() -> u64 {
        COUNT.load(Ordering::Relaxed)
    }
}
#[inline]
pub(crate) fn should_fail() -> bool {
    #[cfg(feature = "alloc-fault-injection")]
    {
        injection::should_fail()
    }
    #[cfg(not(feature = "alloc-fault-injection"))]
    {
        false
    }
}
#[cfg(all(test, feature = "alloc-fault-injection"))]
pub(crate) fn inject(phase: Phase, ordinal: u64) {
    injection::configure(phase, ordinal, 0);
}
#[cfg(feature = "alloc-fault-injection")]
pub(crate) fn inject_after(phase: Phase, ordinal: u64, after_hours: u64) {
    injection::configure(phase, ordinal, after_hours);
}
#[cfg(feature = "alloc-fault-injection")]
pub(crate) fn committed_hour() {
    injection::committed_hour();
}
#[cfg(all(test, feature = "alloc-fault-injection"))]
pub(crate) fn allocation_count() -> u64 {
    injection::count()
}
#[cfg(test)]
pub(crate) fn reset() {
    TERMINAL.state.store(0, Ordering::Release);
    unsafe {
        *TERMINAL.record.get() = None;
    }
    TERMINAL
        .committed_segment
        .store(u64::MAX, Ordering::Relaxed);
    TERMINAL.committed_bytes.store(0, Ordering::Relaxed);
    TERMINAL.committed_sequence.store(0, Ordering::Relaxed);
    TERMINAL.committed_flags.store(0, Ordering::Relaxed);
    TERMINAL.pending_flags.store(0, Ordering::Relaxed);
    TERMINAL.pending_segment.store(u64::MAX, Ordering::Relaxed);
    TERMINAL.pending_bytes.store(0, Ordering::Relaxed);
    CONTEXT.set(Context::default());
}

#[cfg(test)]
pub(crate) fn concurrent_watermark_exercise() -> io::Result<()> {
    use std::sync::{Arc, atomic::AtomicBool};
    install_hooks();
    for _ in 0..64 {
        reset();
        clear_handle();
        let started = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let writer_started = Arc::clone(&started);
        let writer_stop = Arc::clone(&stop);
        let writer = std::thread::spawn(move || {
            for segment in 1..100_000u32 {
                set_context(Context {
                    campaign_id: Some(*b"21b7CC84-3028-4f65-8E0d-aDf2f37Ca1a6"),
                    branch: Some(u64::from(segment)),
                    load_signal: Some(i64::from(segment)),
                    failed_hour: None,
                });
                committed(segment, u64::from(segment) * 37);
                set_context(Context {
                    campaign_id: Some(*b"22b7cc84-3028-4f65-8e0d-adf2f37ca1a6"),
                    ..Context::default()
                });
                record_start(Some((segment, u64::from(segment) * 17)));
                writer_started.store(true, Ordering::Release);
                if writer_stop.load(Ordering::Acquire) {
                    break;
                }
            }
        });
        while !started.load(Ordering::Acquire) {
            std::hint::spin_loop();
        }
        fail(Phase::Status, Code::Reserve);
        let first = failure().ok_or(io::ErrorKind::Other)?;
        stop.store(true, Ordering::Release);
        writer
            .join()
            .map_err(|_| io::Error::from(io::ErrorKind::Other))?;
        let segment = first.committed_segment.ok_or(io::ErrorKind::Other)?;
        if first.committed_bytes != u64::from(segment) * 37
            || first.committed_campaign_id != Some(*b"21b7CC84-3028-4f65-8E0d-aDf2f37Ca1a6")
            || first.branch != Some(u64::from(segment))
            || first.load_signal != Some(i64::from(segment))
            || first.failed_hour.is_some()
        {
            return Err(io::ErrorKind::InvalidData.into());
        }
        if let Some(pending) = first.pending_campaign_id {
            if pending != *b"22b7cc84-3028-4f65-8e0d-adf2f37ca1a6"
                || first.pending_segment != Some(segment)
                || first.pending_bytes != u64::from(segment) * 17
            {
                return Err(io::ErrorKind::InvalidData.into());
            }
        } else if first.pending_segment.is_some() || first.pending_bytes != 0 {
            return Err(io::ErrorKind::InvalidData.into());
        }
        fail(Phase::Teardown, Code::Thread);
        if failure() != Some(first) {
            return Err(io::ErrorKind::InvalidData.into());
        }
    }
    reset();
    Ok(())
}
