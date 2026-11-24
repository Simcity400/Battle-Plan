use replay_data::{witness::TerminalWitness, witness::WITNESS_BYTES};
use std::{
    cell::{Cell, UnsafeCell},
    fs::File,
    io,
    os::windows::io::AsRawHandle,
    sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, AtomicUsize, Ordering, fence},
};
use windows_sys::Win32::Storage::FileSystem::{FlushFileBuffers, WriteFile};

pub use replay_data::{
    witness::FailureCode as Code, witness::FailurePhase as Phase, witness::UncommittedRange,
};

#[derive(Clone, Copy, Default)]
pub struct Context {
    pub campaign_id: Option<[u8; 36]>,
    pub branch: Option<u64>,
    pub load_signal: Option<i64>,
    pub failed_hour: Option<i32>,
}

#[derive(Clone, Copy)]
pub struct Outstanding {
    pub range: UncommittedRange,
    pub committed_segment: Option<u32>,
    pub committed_bytes: u64,
}

#[derive(Clone, Copy)]
struct SlotSnapshot {
    outstanding: Outstanding,
    committed_hour: Option<i32>,
}

struct AtomicOutstanding {
    sequence: AtomicU64,
    campaign: [AtomicU64; 5],
    branch: AtomicU64,
    signal: AtomicU64,
    hours: AtomicU64,
    flags: AtomicU32,
    segment: AtomicU64,
    bytes: AtomicU64,
    committed_hour: AtomicU32,
}
impl AtomicOutstanding {
    fn new() -> Self {
        Self {
            sequence: AtomicU64::new(0),
            campaign: [const { AtomicU64::new(0) }; 5],
            branch: AtomicU64::new(0),
            signal: AtomicU64::new(0),
            hours: AtomicU64::new(0),
            flags: AtomicU32::new(0),
            segment: AtomicU64::new(u64::MAX),
            bytes: AtomicU64::new(0),
            committed_hour: AtomicU32::new(0),
        }
    }

    fn store(&self, snapshot: Option<SlotSnapshot>) {
        let sequence = self.sequence.load(Ordering::Relaxed);
        self.sequence
            .store(sequence.wrapping_add(1), Ordering::Release);
        fence(Ordering::Release);
        let mut flags = 0;
        if let Some(snapshot) = snapshot {
            let outstanding = snapshot.outstanding;
            let range = outstanding.range;
            for (index, word) in self.campaign.iter().enumerate() {
                word.store(campaign_word(&range.campaign_id, index), Ordering::Relaxed);
            }
            self.branch.store(range.branch, Ordering::Relaxed);
            self.signal
                .store(range.load_signal.unwrap_or(0) as u64, Ordering::Relaxed);
            self.hours.store(
                (u64::from(range.first_hour as u32) << 32) | u64::from(range.last_hour as u32),
                Ordering::Relaxed,
            );
            self.segment.store(
                outstanding.committed_segment.map_or(u64::MAX, u64::from),
                Ordering::Relaxed,
            );
            self.bytes
                .store(outstanding.committed_bytes, Ordering::Relaxed);
            self.committed_hour.store(
                snapshot.committed_hour.unwrap_or(0) as u32,
                Ordering::Relaxed,
            );
            flags = 1
                | (u32::from(range.load_signal.is_some()) << 1)
                | (u32::from(snapshot.committed_hour.is_some()) << 2);
        }
        self.flags.store(flags, Ordering::Relaxed);
        self.sequence
            .store(sequence.wrapping_add(2), Ordering::Release);
    }

    fn load(&self) -> Option<SlotSnapshot> {
        loop {
            let before = self.sequence.load(Ordering::Acquire);
            if before & 1 != 0 {
                std::hint::spin_loop();
                continue;
            }
            let flags = self.flags.load(Ordering::Relaxed);
            let hours = self.hours.load(Ordering::Relaxed);
            let snapshot = (flags & 1 != 0).then(|| SlotSnapshot {
                outstanding: Outstanding {
                    range: UncommittedRange {
                        campaign_id: load_campaign(&self.campaign),
                        branch: self.branch.load(Ordering::Relaxed),
                        load_signal: (flags & 2 != 0)
                            .then(|| self.signal.load(Ordering::Relaxed) as i64),
                        first_hour: (hours >> 32) as u32 as i32,
                        last_hour: hours as u32 as i32,
                    },
                    committed_segment: u32::try_from(self.segment.load(Ordering::Relaxed)).ok(),
                    committed_bytes: self.bytes.load(Ordering::Relaxed),
                },
                committed_hour: (flags & 4 != 0)
                    .then(|| self.committed_hour.load(Ordering::Relaxed) as i32),
            });
            fence(Ordering::Acquire);
            if self.sequence.load(Ordering::Acquire) == before {
                return snapshot;
            }
        }
    }
}

struct OutstandingSlot {
    next: *mut OutstandingSlot,
    published: AtomicOutstanding,
    committed: AtomicOutstanding,
    retained: AtomicBool,
    frozen: UnsafeCell<Option<SlotSnapshot>>,
}
unsafe impl Sync for OutstandingSlot {}
static OUTSTANDING: AtomicPtr<OutstandingSlot> = AtomicPtr::new(std::ptr::null_mut());

static OUTSTANDING_BYTES: AtomicUsize = AtomicUsize::new(0);

pub fn uncommitted_storage_bytes() -> usize {
    OUTSTANDING_BYTES.load(Ordering::Relaxed)
}

fn same_range_context(left: UncommittedRange, right: UncommittedRange) -> bool {
    left.campaign_id == right.campaign_id
        && left.branch == right.branch
        && left.load_signal == right.load_signal
}

fn matches_context(range: UncommittedRange, context: Context) -> bool {
    Some(range.campaign_id) == context.campaign_id
        && Some(range.branch) == context.branch
        && range.load_signal == context.load_signal
}

fn newer_prefix(left: (Option<u32>, u64), right: (Option<u32>, u64)) -> (Option<u32>, u64) {
    left.max(right)
}

pub fn prepare_uncommitted_context(context: Context) -> io::Result<()> {
    if failed() {
        return Err(io::ErrorKind::OutOfMemory.into());
    }
    let mut pointer = OUTSTANDING.load(Ordering::Acquire);
    let mut free = false;
    while let Some(slot) = unsafe { pointer.as_ref() } {
        match slot.published.load() {
            Some(snapshot) if matches_context(snapshot.outstanding.range, context) => return Ok(()),
            None => free = true,
            _ => {}
        }
        pointer = slot.next;
    }
    if free {
        return Ok(());
    }
    let slot = engine_allocation::boxed(OutstandingSlot {
        next: OUTSTANDING.load(Ordering::Acquire),
        published: AtomicOutstanding::new(),
        committed: AtomicOutstanding::new(),
        retained: AtomicBool::new(false),
        frozen: UnsafeCell::new(None),
    })?;
    if failed() {
        return Err(io::ErrorKind::OutOfMemory.into());
    }
    OUTSTANDING.store(Box::into_raw(slot), Ordering::Release);
    OUTSTANDING_BYTES.fetch_add(size_of::<OutstandingSlot>(), Ordering::Relaxed);
    Ok(())
}

pub fn publish_uncommitted(ranges: impl Iterator<Item = Outstanding>) {
    if failed() {
        return;
    }
    let head = OUTSTANDING.load(Ordering::Acquire);
    let mut pointer = head;
    while let Some(slot) = unsafe { pointer.as_ref() } {
        slot.retained.store(false, Ordering::Relaxed);
        pointer = slot.next;
    }
    for outstanding in ranges {
        if failed() {
            return;
        }
        let mut pointer = head;
        let mut free = std::ptr::null_mut();
        let mut matching = std::ptr::null_mut();
        while let Some(slot) = unsafe { pointer.as_ref() } {
            match slot.published.load() {
                Some(snapshot)
                    if same_range_context(snapshot.outstanding.range, outstanding.range) =>
                {
                    matching = pointer;
                    break;
                }
                None => free = pointer,
                _ => {}
            }
            pointer = slot.next;
        }
        let pointer = if matching.is_null() { free } else { matching };
        let Some(slot) = (unsafe { pointer.as_ref() }) else {
            let _ = record_failure(phase(), Code::SizeOverflow);
            return;
        };
        if failed() {
            return;
        }
        slot.published.store(Some(SlotSnapshot {
            outstanding,
            committed_hour: None,
        }));
        slot.retained.store(true, Ordering::Relaxed);
    }
    let mut pointer = head;
    while let Some(slot) = unsafe { pointer.as_ref() } {
        if failed() {
            return;
        }
        if !slot.retained.load(Ordering::Relaxed) {
            slot.published.store(None);
        }
        pointer = slot.next;
    }
}

fn publish_slot_commits(context: Context, segment: u32, bytes: u64, complete_hour: bool) {
    let mut pointer = OUTSTANDING.load(Ordering::Acquire);
    while let Some(slot) = unsafe { pointer.as_ref() } {
        if let Some(mut snapshot) = slot.published.load()
            && Some(snapshot.outstanding.range.campaign_id) == context.campaign_id
        {
            let previous = slot.committed.load();
            snapshot.committed_hour = previous
                .filter(|previous| {
                    same_range_context(previous.outstanding.range, snapshot.outstanding.range)
                })
                .and_then(|previous| previous.committed_hour);
            if complete_hour && matches_context(snapshot.outstanding.range, context) {
                snapshot.committed_hour = context.failed_hour.or(snapshot.committed_hour);
            }
            let prefix = newer_prefix(
                (
                    snapshot.outstanding.committed_segment,
                    snapshot.outstanding.committed_bytes,
                ),
                (Some(segment), bytes),
            );
            snapshot.outstanding.committed_segment = prefix.0;
            snapshot.outstanding.committed_bytes = prefix.1;
            slot.committed.store(Some(snapshot));
        }
        pointer = slot.next;
    }
}

fn freeze_outstanding(head: *mut OutstandingSlot) {
    let mut pointer = head;
    while let Some(slot) = unsafe { pointer.as_ref() } {
        let snapshot = slot
            .published
            .load()
            .map(|mut snapshot| {
                if let Some(committed) = slot.committed.load()
                    && committed.outstanding.range.campaign_id
                        == snapshot.outstanding.range.campaign_id
                {
                    let prefix = newer_prefix(
                        (
                            snapshot.outstanding.committed_segment,
                            snapshot.outstanding.committed_bytes,
                        ),
                        (
                            committed.outstanding.committed_segment,
                            committed.outstanding.committed_bytes,
                        ),
                    );
                    snapshot.outstanding.committed_segment = prefix.0;
                    snapshot.outstanding.committed_bytes = prefix.1;
                    if same_range_context(committed.outstanding.range, snapshot.outstanding.range) {
                        snapshot.committed_hour = committed.committed_hour;
                    }
                }
                snapshot
            })
            .filter(|snapshot| snapshot.outstanding.range.is_valid());
        unsafe {
            *slot.frozen.get() = snapshot;
        }
        pointer = slot.next;
    }
}

thread_local! {
    static PHASE: Cell<Option<Phase>> = const { Cell::new(None) };
    static CONTEXT: Cell<Context> = const { Cell::new(Context {
        campaign_id: None, branch: None, load_signal: None, failed_hour: None,
    }) };
    static BORROWED_CONTEXT: Cell<*const Context> = const { Cell::new(std::ptr::null()) };
    static REQUEST: Cell<Option<(&'static str, usize)>> = const { Cell::new(None) };
}

static DETAIL_HANDLE: AtomicUsize = AtomicUsize::new(0);

pub struct Details {
    _file: File,
}
impl Drop for Details {
    fn drop(&mut self) {
        DETAIL_HANDLE.store(0, Ordering::Release);
    }
}

pub fn prepare_details(directory: &std::path::Path) -> Option<Details> {
    let file = File::create(directory.join("collector-failure-detail.json")).ok()?;
    DETAIL_HANDLE.store(file.as_raw_handle() as usize, Ordering::Release);
    Some(Details { _file: file })
}

pub fn reserve_raw<T>(values: &mut Vec<T>, target: usize, family: &'static str) -> io::Result<()> {
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
    engine_allocation::reserve(values, target.saturating_sub(values.len()))
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
            &raw mut written,
            std::ptr::null_mut(),
        );
        FlushFileBuffers(handle as _);
    }
}

pub struct Scope(Option<Phase>);
impl Drop for Scope {
    #[inline]
    fn drop(&mut self) {
        PHASE.set(self.0);
        #[cfg(any(test, feature = "test-support"))]
        crate::test_support::phase_enter(self.0.unwrap_or(Phase::Unphased) as u8);
    }
}
#[inline]
pub fn enter(phase: Phase) -> Scope {
    #[cfg(any(test, feature = "test-support"))]
    crate::test_support::phase_enter(phase as u8);
    Scope(PHASE.replace(Some(phase)))
}
pub fn set_context(context: Context) {
    CONTEXT.set(context);
}
pub fn context() -> Context {
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
pub struct BorrowedScope(*const Context);
impl Drop for BorrowedScope {
    fn drop(&mut self) {
        BORROWED_CONTEXT.set(self.0);
    }
}
/// The caller keeps this immutable context live until the returned guard drops.
/// # Safety
/// The pointer must refer to an initialized context that stays alive and is not
/// mutated while this thread holds the returned scope. Nested scopes drop in reverse order, so
/// each restores a context that is still alive.
pub unsafe fn borrow_context(pointer: *const Context) -> BorrowedScope {
    BorrowedScope(BORROWED_CONTEXT.replace(pointer))
}
pub fn attempted_hour(hour: Option<i32>) {
    CONTEXT.with(|context| {
        let mut value = context.get();
        value.failed_hour = hour;
        context.set(value);
    });
}

const FRAME_HOUR: u32 = 8;

fn campaign_word(id: &[u8; 36], index: usize) -> u64 {
    if index == 4 {
        u64::from(u32::from_le_bytes(id[32..36].try_into().unwrap()))
    } else {
        u64::from_le_bytes(id[index * 8..index * 8 + 8].try_into().unwrap())
    }
}

fn load_campaign(slots: &[AtomicU64; 5]) -> [u8; 36] {
    let mut campaign = [0u8; 40];
    for (output, word) in campaign.chunks_exact_mut(8).zip(slots) {
        output.copy_from_slice(&word.load(Ordering::Relaxed).to_le_bytes());
    }
    campaign[..36].try_into().unwrap()
}

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
    committed_hour: AtomicU64,
    pending_campaign: [AtomicU64; 5],
    pending_segment: AtomicU64,
    pending_bytes: AtomicU64,
    pending_flags: AtomicU32,
    range_sequence: AtomicU64,
    range_campaign: [AtomicU64; 5],
    range_branch: AtomicU64,
    range_signal: AtomicU64,
    range_hours: AtomicU64,
    range_flags: AtomicU32,
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
    committed_hour: AtomicU64::new(0),
    pending_campaign: [const { AtomicU64::new(0) }; 5],
    pending_segment: AtomicU64::new(u64::MAX),
    pending_bytes: AtomicU64::new(0),
    pending_flags: AtomicU32::new(0),
    range_sequence: AtomicU64::new(0),
    range_campaign: [const { AtomicU64::new(0) }; 5],
    range_branch: AtomicU64::new(0),
    range_signal: AtomicU64::new(0),
    range_hours: AtomicU64::new(0),
    range_flags: AtomicU32::new(0),
};

pub fn prepare(file: &File, session: u64, process_start_time: u64) {
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
pub fn clear_handle() {
    TERMINAL.handle.store(0, Ordering::Release);
}
pub fn failed() -> bool {
    TERMINAL.state.load(Ordering::Acquire) != 0
}

pub fn committed(segment: u32, bytes: u64) {
    committed_record(segment, bytes, false);
}

pub fn committed_record(segment: u32, bytes: u64, complete_hour: bool) {
    if failed() {
        return;
    }
    let sequence = TERMINAL.committed_sequence.load(Ordering::Relaxed);
    TERMINAL
        .committed_sequence
        .store(sequence.wrapping_add(1), Ordering::Release);
    fence(Ordering::Release);
    with_context(|context| {
        let mut flags = u32::from(context.campaign_id.is_some())
            | (u32::from(context.branch.is_some()) << 1)
            | (u32::from(context.load_signal.is_some()) << 2);
        let word = |index: usize| {
            context
                .campaign_id
                .as_ref()
                .map_or(0, |id| campaign_word(id, index))
        };
        let previous = TERMINAL.committed_flags.load(Ordering::Relaxed);
        let same_branch = previous & 3 == flags & 3
            && TERMINAL.committed_branch.load(Ordering::Relaxed) == context.branch.unwrap_or(0)
            && TERMINAL
                .committed_campaign
                .iter()
                .enumerate()
                .all(|(index, slot)| slot.load(Ordering::Relaxed) == word(index));
        let same_signal = previous & 4 == flags & 4
            && TERMINAL.committed_signal.load(Ordering::Relaxed)
                == context.load_signal.unwrap_or(0) as u64;
        if !same_branch || !same_signal {
            for (index, slot) in TERMINAL.committed_campaign.iter().enumerate() {
                slot.store(word(index), Ordering::Relaxed);
            }
            TERMINAL
                .committed_branch
                .store(context.branch.unwrap_or(0), Ordering::Relaxed);
            TERMINAL
                .committed_signal
                .store(context.load_signal.unwrap_or(0) as u64, Ordering::Relaxed);
        }
        let frame_hour = context
            .failed_hour
            .filter(|_| complete_hour && flags & 3 == 3);
        if let Some(hour) = frame_hour {
            TERMINAL
                .committed_hour
                .store(u64::from(hour as u32), Ordering::Relaxed);
            flags |= FRAME_HOUR;
        } else if same_branch && same_signal {
            flags |= previous & FRAME_HOUR;
        }
        TERMINAL.committed_flags.store(flags, Ordering::Relaxed);
    });
    TERMINAL.committed_bytes.store(bytes, Ordering::Relaxed);
    TERMINAL
        .committed_segment
        .store(u64::from(segment), Ordering::Relaxed);
    TERMINAL.pending_flags.store(0, Ordering::Relaxed);
    with_context(|context| publish_slot_commits(*context, segment, bytes, complete_hour));
    TERMINAL
        .committed_sequence
        .store(sequence.wrapping_add(2), Ordering::Release);
}

pub fn record_start(previous: Option<(u32, u64)>) {
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
            let word = context
                .campaign_id
                .as_ref()
                .map_or(0, |id| campaign_word(id, index));
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

pub fn set_uncommitted_range(range: Option<UncommittedRange>) {
    if failed() {
        return;
    }
    let sequence = TERMINAL.range_sequence.load(Ordering::Relaxed);
    TERMINAL
        .range_sequence
        .store(sequence.wrapping_add(1), Ordering::Release);
    fence(Ordering::Release);
    if let Some(range) = range {
        for (index, slot) in TERMINAL.range_campaign.iter().enumerate() {
            slot.store(campaign_word(&range.campaign_id, index), Ordering::Relaxed);
        }
        TERMINAL.range_branch.store(range.branch, Ordering::Relaxed);
        TERMINAL
            .range_signal
            .store(range.load_signal.unwrap_or(0) as u64, Ordering::Relaxed);
        TERMINAL.range_hours.store(
            (u64::from(range.first_hour as u32) << 32) | u64::from(range.last_hour as u32),
            Ordering::Relaxed,
        );
    }
    TERMINAL.range_flags.store(
        range.map_or(0, |range| 1 | (u32::from(range.load_signal.is_some()) << 1)),
        Ordering::Relaxed,
    );
    TERMINAL
        .range_sequence
        .store(sequence.wrapping_add(2), Ordering::Release);
}

fn uncommitted_range() -> Option<UncommittedRange> {
    loop {
        let before = TERMINAL.range_sequence.load(Ordering::Acquire);
        if before & 1 != 0 {
            std::hint::spin_loop();
            continue;
        }
        let flags = TERMINAL.range_flags.load(Ordering::Relaxed);
        let hours = TERMINAL.range_hours.load(Ordering::Relaxed);
        let range = (flags & 1 != 0).then(|| UncommittedRange {
            campaign_id: load_campaign(&TERMINAL.range_campaign),
            branch: TERMINAL.range_branch.load(Ordering::Relaxed),
            load_signal: (flags & 2 != 0)
                .then(|| TERMINAL.range_signal.load(Ordering::Relaxed) as i64),
            first_hour: (hours >> 32) as u32 as i32,
            last_hour: hours as u32 as i32,
        });
        fence(Ordering::Acquire);
        if TERMINAL.range_sequence.load(Ordering::Acquire) == before {
            return range;
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn failure() -> Option<TerminalWitness> {
    if TERMINAL.state.load(Ordering::Acquire) != 2 {
        return None;
    }
    unsafe { *TERMINAL.record.get() }
}

pub fn record_failure(phase: Phase, code: Code) -> bool {
    if !engine_allocation::capture_active() {
        return false;
    }
    if TERMINAL
        .state
        .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return false;
    }
    let mut context = context();
    if context
        .campaign_id
        .is_some_and(|id| !replay_data::identity::is_campaign_uuid(&id))
    {
        context.campaign_id = None;
    }
    let head = OUTSTANDING.load(Ordering::Acquire);
    freeze_outstanding(head);
    let uncommitted = uncommitted_range();
    let (
        segment,
        committed_bytes,
        committed_context,
        committed_frame_hour,
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
        let committed_context = Context {
            campaign_id: (flags & 1 != 0).then(|| load_campaign(&TERMINAL.committed_campaign)),
            branch: (flags & 2 != 0).then(|| TERMINAL.committed_branch.load(Ordering::Relaxed)),
            load_signal: (flags & 4 != 0)
                .then(|| TERMINAL.committed_signal.load(Ordering::Relaxed) as i64),
            failed_hour: None,
        };
        let committed_frame_hour = (flags & FRAME_HOUR != 0)
            .then(|| TERMINAL.committed_hour.load(Ordering::Relaxed) as u32 as i32);
        let pending_campaign_id = (TERMINAL.pending_flags.load(Ordering::Relaxed) != 0)
            .then(|| load_campaign(&TERMINAL.pending_campaign));
        let pending_segment = u32::try_from(TERMINAL.pending_segment.load(Ordering::Relaxed)).ok();
        let pending_bytes = TERMINAL.pending_bytes.load(Ordering::Relaxed);
        fence(Ordering::Acquire);
        if TERMINAL.committed_sequence.load(Ordering::Acquire) == before {
            break (
                segment,
                bytes,
                committed_context,
                committed_frame_hour,
                pending_campaign_id,
                pending_segment,
                pending_bytes,
            );
        }
    };
    let committed_through = |campaign_id: [u8; 36], branch: u64, load_signal: Option<i64>| {
        let mut through = committed_frame_hour.filter(|_| {
            committed_context.campaign_id == Some(campaign_id)
                && committed_context.branch == Some(branch)
                && committed_context.load_signal == load_signal
        });
        let mut pointer = head;
        while let Some(slot) = unsafe { pointer.as_ref() } {
            if let Some(snapshot) = unsafe { *slot.frozen.get() }
                && snapshot.outstanding.range.campaign_id == campaign_id
                && snapshot.outstanding.range.branch == branch
                && snapshot.outstanding.range.load_signal == load_signal
            {
                through = through.max(snapshot.committed_hour);
            }
            pointer = slot.next;
        }
        through
    };
    if let (Some(campaign_id), Some(branch), Some(hour)) =
        (context.campaign_id, context.branch, context.failed_hour)
        && committed_through(campaign_id, branch, context.load_signal)
            .is_some_and(|committed| hour <= committed)
    {
        context.failed_hour = None;
    }
    let uncommitted = uncommitted
        .filter(UncommittedRange::is_valid)
        .and_then(|mut range| {
            if let Some(committed) =
                committed_through(range.campaign_id, range.branch, range.load_signal)
                && committed >= range.first_hour
            {
                if committed >= range.last_hour {
                    return None;
                }
                range.first_hour = committed + 1;
            }
            Some(range)
        });
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
        uncommitted,
        phase: phase as u8,
        code: code as u8,
        drain_complete: false,
    };
    write_details(&record);
    let mut count = 1u32;
    let mut pointer = head;
    while let Some(slot) = unsafe { pointer.as_ref() } {
        if unsafe { (*slot.frozen.get()).is_some() } {
            let Some(next) = count.checked_add(1) else {
                TERMINAL.state.store(2, Ordering::Release);
                return true;
            };
            count = next;
        }
        pointer = slot.next;
    }
    let bytes = unsafe { &mut *TERMINAL.buffer.get() };
    let handle = TERMINAL.handle.load(Ordering::Acquire);
    record.encode_bundle_record(bytes, 0, count);
    let mut writable = write_witness(handle, bytes);
    let mut index = 1;
    let mut pointer = head;
    while let Some(slot) = unsafe { pointer.as_ref() } {
        if let Some(snapshot) = unsafe { *slot.frozen.get() } {
            let extension =
                extension_record(&record, snapshot, committed_context, committed_frame_hour);
            extension.encode_bundle_record(bytes, index, count);
            if writable {
                writable = write_witness(handle, bytes);
            }
            index += 1;
        }
        pointer = slot.next;
    }
    if handle != 0 {
        unsafe {
            FlushFileBuffers(handle as _);
        }
    }
    unsafe {
        *TERMINAL.record.get() = Some(record);
    }
    TERMINAL.state.store(2, Ordering::Release);
    true
}

fn extension_record(
    primary: &TerminalWitness,
    snapshot: SlotSnapshot,
    committed_context: Context,
    committed_frame_hour: Option<i32>,
) -> TerminalWitness {
    let mut outstanding = snapshot.outstanding;
    let mut through = snapshot.committed_hour;
    if committed_context.campaign_id == Some(outstanding.range.campaign_id) {
        let prefix = newer_prefix(
            (outstanding.committed_segment, outstanding.committed_bytes),
            (primary.committed_segment, primary.committed_bytes),
        );
        outstanding.committed_segment = prefix.0;
        outstanding.committed_bytes = prefix.1;
        if matches_context(outstanding.range, committed_context) {
            through = through.max(committed_frame_hour);
        }
    }
    if primary.pending_campaign_id == Some(outstanding.range.campaign_id) {
        let prefix = newer_prefix(
            (outstanding.committed_segment, outstanding.committed_bytes),
            (primary.pending_segment, primary.pending_bytes),
        );
        outstanding.committed_segment = prefix.0;
        outstanding.committed_bytes = prefix.1;
    }
    let uncommitted = if through.is_some_and(|hour| hour >= outstanding.range.last_hour) {
        None
    } else {
        if let Some(hour) = through
            && hour >= outstanding.range.first_hour
        {
            outstanding.range.first_hour = hour + 1;
        }
        Some(outstanding.range)
    };
    TerminalWitness {
        campaign_id: Some(outstanding.range.campaign_id),
        branch: None,
        load_signal: None,
        failed_hour: None,
        committed_campaign_id: outstanding
            .committed_segment
            .map(|_| outstanding.range.campaign_id),
        committed_segment: outstanding.committed_segment,
        committed_bytes: if outstanding.committed_segment.is_some() {
            outstanding.committed_bytes
        } else {
            0
        },
        pending_campaign_id: None,
        pending_segment: None,
        pending_bytes: 0,
        uncommitted,
        ..*primary
    }
}

fn write_witness(handle: usize, bytes: &[u8; WITNESS_BYTES]) -> bool {
    if handle == 0 {
        return false;
    }
    let mut offset = 0;
    while offset < bytes.len() {
        let mut written = 0;
        let success = unsafe {
            WriteFile(
                handle as _,
                bytes[offset..].as_ptr(),
                (bytes.len() - offset) as u32,
                &raw mut written,
                std::ptr::null_mut(),
            )
        };
        if success == 0 || written == 0 {
            return false;
        }
        offset += written as usize;
    }
    true
}

pub fn oom() -> io::Error {
    engine_allocation::oom()
}
pub fn observe(error: &io::Error) {
    if engine_allocation::is_allocation_error(error) {
        let _ = engine_allocation::oom();
    }
}
pub fn io(error: io::Error) -> io::Error {
    observe(&error);
    error
}
#[cfg(feature = "alloc-fault-injection")]
mod injection {
    use super::*;
    static PHASES: AtomicU32 = AtomicU32::new(0);
    static ORDINAL: AtomicU64 = AtomicU64::new(0);
    static COUNT: AtomicU64 = AtomicU64::new(0);
    static HOURS: AtomicU64 = AtomicU64::new(0);
    static AFTER_HOURS: AtomicU64 = AtomicU64::new(0);
    #[cfg(any(test, feature = "test-support"))]
    static OBSERVER: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());
    pub(super) fn configure(phases: &[super::Phase], ordinal: u64, after_hours: u64) {
        let mask = phases.iter().fold(0, |mask, phase| mask | (1 << *phase as u32));
        PHASES.store(mask, Ordering::Relaxed);
        ORDINAL.store(ordinal, Ordering::Relaxed);
        COUNT.store(0, Ordering::Relaxed);
        HOURS.store(0, Ordering::Relaxed);
        AFTER_HOURS.store(after_hours, Ordering::Relaxed);
    }
    pub(super) fn should_fail() -> bool {
        if HOURS.load(Ordering::Relaxed) < AFTER_HOURS.load(Ordering::Relaxed)
            || !super::PHASE
                .get()
                .is_some_and(|phase| PHASES.load(Ordering::Relaxed) & (1 << phase as u32) != 0)
        {
            return false;
        }
        let ordinal = COUNT.fetch_add(1, Ordering::Relaxed) + 1;
        #[cfg(any(test, feature = "test-support"))]
        {
            let observer = OBSERVER.load(Ordering::Acquire);
            if !observer.is_null() {
                unsafe { std::mem::transmute::<*mut (), fn()>(observer)() };
            }
        }
        ordinal == ORDINAL.load(Ordering::Relaxed)
    }
    pub(super) fn committed_hour() {
        HOURS.fetch_add(1, Ordering::Relaxed);
    }
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn count() -> u64 {
        COUNT.load(Ordering::Relaxed)
    }
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn observe(observer: Option<fn()>) {
        OBSERVER.store(
            observer.map_or(std::ptr::null_mut(), |observer| observer as *mut ()),
            Ordering::Release,
        );
    }
}
#[inline]
pub fn should_fail() -> bool {
    #[cfg(feature = "alloc-fault-injection")]
    {
        injection::should_fail()
    }
    #[cfg(not(feature = "alloc-fault-injection"))]
    {
        false
    }
}
#[cfg(all(any(test, feature = "test-support"), feature = "alloc-fault-injection"))]
pub fn inject(phase: Phase, ordinal: u64) {
    injection::configure(&[phase], ordinal, 0);
}
#[cfg(all(any(test, feature = "test-support"), feature = "alloc-fault-injection"))]
pub fn inject_any(phases: &[Phase], ordinal: u64) {
    injection::configure(phases, ordinal, 0);
}
#[cfg(feature = "alloc-fault-injection")]
pub fn inject_after(phase: Phase, ordinal: u64, after_hours: u64) {
    injection::configure(&[phase], ordinal, after_hours);
}
#[cfg(feature = "alloc-fault-injection")]
pub fn committed_hour() {
    injection::committed_hour();
}
#[cfg(all(any(test, feature = "test-support"), feature = "alloc-fault-injection"))]
pub fn allocation_count() -> u64 {
    injection::count()
}
#[cfg(all(any(test, feature = "test-support"), feature = "alloc-fault-injection"))]
pub fn observe_counted(observer: Option<fn()>) {
    injection::observe(observer);
}
#[cfg(any(test, feature = "test-support"))]
pub fn reset() {
    OUTSTANDING_BYTES.store(0, Ordering::Relaxed);
    let mut pointer = OUTSTANDING.swap(std::ptr::null_mut(), Ordering::AcqRel);
    while !pointer.is_null() {
        let slot = unsafe { Box::from_raw(pointer) };
        pointer = slot.next;
    }
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
    TERMINAL.range_sequence.store(0, Ordering::Relaxed);
    TERMINAL.range_flags.store(0, Ordering::Relaxed);
    CONTEXT.set(Context::default());
}

pub fn phase() -> Phase {
    PHASE.get().unwrap_or(Phase::Unphased)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAMPAIGN: [u8; 36] = *b"21b7cc84-3028-4f65-8e0d-adf2f37ca1a6";
    const OTHER: [u8; 36] = *b"22b7cc84-3028-4f65-8e0d-adf2f37ca1a6";

    fn writing(branch: u64, load_signal: Option<i64>, hour: Option<i32>) -> Context {
        Context {
            campaign_id: Some(CAMPAIGN),
            branch: Some(branch),
            load_signal,
            failed_hour: hour,
        }
    }

    fn range(branch: u64, load_signal: Option<i64>, hours: (i32, i32)) -> UncommittedRange {
        UncommittedRange {
            campaign_id: CAMPAIGN,
            branch,
            load_signal,
            first_hour: hours.0,
            last_hour: hours.1,
        }
    }

    fn fail_after(
        range: Option<UncommittedRange>,
        writer: impl FnOnce(),
        failing: Context,
    ) -> TerminalWitness {
        reset();
        clear_handle();
        set_uncommitted_range(range);
        writer();
        set_context(failing);
        assert!(record_failure(Phase::Spool, Code::Reserve));
        failure().unwrap()
    }

    #[test]
    fn terminal_clips_uncommitted_range_and_attempt_by_committed_frame_hour_only() {
        fn failure_hook() {}
        engine_allocation::set_hooks(None, Some(failure_hook));
        let frame = |hour| {
            move || {
                set_context(writing(9, Some(19), Some(hour)));
                committed_record(0, 100 + hour as u64, true);
            }
        };

        let witness = fail_after(
            Some(range(9, Some(19), (5, 7))),
            frame(5),
            writing(9, Some(19), Some(5)),
        );
        assert_eq!(witness.failed_hour, None);
        assert_eq!(witness.uncommitted, Some(range(9, Some(19), (6, 7))));
        assert_eq!(witness.committed_bytes, 105);
        assert_eq!(witness.committed_campaign_id, Some(CAMPAIGN));

        let witness = fail_after(
            Some(range(9, Some(19), (5, 5))),
            frame(5),
            writing(9, Some(19), Some(5)),
        );
        assert_eq!(witness.uncommitted, None);
        assert_eq!(witness.failed_hour, None);

        let witness = fail_after(
            Some(range(9, Some(19), (5, 7))),
            frame(5),
            writing(9, Some(19), Some(8)),
        );
        assert_eq!(witness.failed_hour, Some(8));
        assert_eq!(witness.uncommitted, Some(range(9, Some(19), (6, 7))));

        let witness = fail_after(
            Some(range(9, Some(19), (5, 7))),
            || {
                set_context(writing(9, Some(19), Some(4)));
                committed_record(0, 104, true);
                set_context(writing(9, Some(19), Some(5)));
            },
            writing(9, Some(19), Some(5)),
        );
        assert_eq!(witness.failed_hour, Some(5));
        assert_eq!(witness.uncommitted, Some(range(9, Some(19), (5, 7))));
        assert_eq!(witness.committed_bytes, 104);

        for (other, attempt) in [
            (range(10, Some(19), (5, 7)), writing(10, Some(19), Some(5))),
            (range(9, Some(20), (5, 7)), writing(9, Some(20), Some(5))),
            (range(9, None, (5, 7)), writing(9, None, Some(5))),
            (
                UncommittedRange {
                    campaign_id: OTHER,
                    ..range(9, Some(19), (5, 7))
                },
                Context {
                    campaign_id: Some(OTHER),
                    ..writing(9, Some(19), Some(5))
                },
            ),
        ] {
            let witness = fail_after(Some(other), frame(5), attempt);
            assert_eq!(witness.uncommitted, Some(other));
            assert_eq!(witness.failed_hour, Some(5));
        }

        let witness = fail_after(
            Some(range(9, Some(19), (5, 7))),
            || {
                frame(5)();
                set_context(writing(9, Some(19), None));
                committed(0, 120);
            },
            Context::default(),
        );
        assert_eq!(witness.uncommitted, Some(range(9, Some(19), (6, 7))));
        assert_eq!(witness.committed_bytes, 120);
        let witness = fail_after(
            Some(range(9, Some(19), (5, 7))),
            || {
                frame(5)();
                set_context(writing(10, Some(23), None));
                committed(0, 130);
            },
            Context::default(),
        );
        assert_eq!(witness.uncommitted, Some(range(9, Some(19), (5, 7))));

        let witness = fail_after(
            Some(range(9, Some(19), (5, 7))),
            || {
                set_context(writing(9, Some(19), None));
                committed_record(0, 140, true);
            },
            Context::default(),
        );
        assert_eq!(witness.uncommitted, Some(range(9, Some(19), (5, 7))));

        let witness = fail_after(None, frame(5), writing(9, Some(19), Some(6)));
        assert_eq!(witness.uncommitted, None);
        assert_eq!(witness.failed_hour, Some(6));
        let witness = fail_after(
            Some(range(9, Some(19), (8, 7))),
            || {},
            writing(9, Some(19), Some(6)),
        );
        assert_eq!(witness.uncommitted, None);
        set_uncommitted_range(Some(range(9, Some(19), (1, 2))));
        assert_eq!(uncommitted_range(), Some(range(9, Some(19), (8, 7))));
        assert_eq!(failure(), Some(witness));

        fn registry_len() -> usize {
            let mut count = 0;
            let mut pointer = OUTSTANDING.load(Ordering::Acquire);
            while let Some(slot) = unsafe { pointer.as_ref() } {
                count += 1;
                pointer = slot.next;
            }
            count
        }
        fn outstanding(range: UncommittedRange, prefix: Option<(u32, u64)>) -> Outstanding {
            Outstanding {
                range,
                committed_segment: prefix.map(|prefix| prefix.0),
                committed_bytes: prefix.map_or(0, |prefix| prefix.1),
            }
        }
        fn admit(ranges: &[Outstanding]) {
            let range = ranges.last().unwrap().range;
            prepare_uncommitted_context(Context {
                campaign_id: Some(range.campaign_id),
                branch: Some(range.branch),
                load_signal: range.load_signal,
                failed_hour: Some(range.last_hour),
            })
            .unwrap();
            publish_uncommitted(ranges.iter().copied());
        }

        reset();
        let a = outstanding(range(9, Some(19), (11, 12)), Some((0, 100)));
        let b = outstanding(
            UncommittedRange {
                campaign_id: OTHER,
                ..range(3, Some(20), (20, 21))
            },
            None,
        );
        let branch = outstanding(range(10, Some(19), (30, 31)), Some((0, 100)));
        let load = outstanding(range(9, Some(22), (40, 41)), Some((0, 100)));
        admit(&[a]);
        admit(&[a, b]);
        admit(&[a, b, branch]);
        admit(&[a, b, branch, load]);
        assert_eq!(registry_len(), 4);
        let head = OUTSTANDING.load(Ordering::Acquire);
        let growths = engine_allocation::growths();
        for _ in 0..3 {
            prepare_uncommitted_context(writing(9, Some(19), Some(13))).unwrap();
        }
        assert_eq!(engine_allocation::growths(), growths);
        set_context(writing(9, Some(19), Some(11)));
        committed_record(0, 111, true);
        set_context(Context {
            campaign_id: Some(OTHER),
            branch: Some(3),
            load_signal: Some(20),
            failed_hour: Some(20),
        });
        committed_record(2, 220, true);
        let directory = std::env::temp_dir().join("hoi4-spool-writer");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join(format!("terminal-bundle-{}.bin", std::process::id()));
        let file = File::create(&path).unwrap();
        prepare(&file, 17, 123);
        set_context(writing(9, Some(22), Some(42)));
        assert!(record_failure(Phase::Spool, Code::Reserve));
        assert!(!record_failure(Phase::Encode, Code::Box));
        clear_handle();
        drop(file);
        let bytes = std::fs::read(&path).unwrap();
        let bundle = replay_data::witness::decode_bundle(&bytes).unwrap();
        assert_eq!(bundle.len(), 5);
        assert_eq!(bundle[0], failure().unwrap());
        assert_eq!(bundle[0].uncommitted, None);
        assert_eq!(bundle[0].failed_hour, Some(42));
        assert!(
            bundle[1..]
                .iter()
                .all(|record| record.failed_hour.is_none())
        );
        let a_record = bundle
            .iter()
            .find(|record| {
                record.uncommitted.is_some_and(|range| {
                    range.campaign_id == CAMPAIGN
                        && range.branch == 9
                        && range.load_signal == Some(19)
                })
            })
            .unwrap();
        assert_eq!(a_record.uncommitted, Some(range(9, Some(19), (12, 12))));
        assert_eq!(
            (a_record.committed_segment, a_record.committed_bytes),
            (Some(0), 111)
        );
        let b_record = bundle
            .iter()
            .find(|record| {
                record
                    .uncommitted
                    .is_some_and(|range| range.campaign_id == OTHER)
            })
            .unwrap();
        assert_eq!(b_record.uncommitted.unwrap().first_hour, 21);
        assert_eq!(
            (b_record.committed_segment, b_record.committed_bytes),
            (Some(2), 220)
        );
        for record in bundle.iter().filter(|record| {
            record.uncommitted.is_some_and(|range| {
                range.campaign_id == CAMPAIGN
                    && (range.branch == 10 || range.load_signal == Some(22))
            })
        }) {
            assert_eq!(
                record.uncommitted.unwrap().first_hour,
                if record.uncommitted.unwrap().branch == 10 {
                    30
                } else {
                    40
                }
            );
            assert_eq!(record.committed_bytes, 111);
        }
        assert_eq!(OUTSTANDING.load(Ordering::Acquire), head);
        for missing in 1..bundle.len() {
            assert!(
                replay_data::witness::decode_bundle(&bytes[..missing * WITNESS_BYTES]).is_err()
            );
        }

        reset();
        admit(&[a]);
        let head = OUTSTANDING.load(Ordering::Acquire);
        freeze_outstanding(head);
        let stale = unsafe { (*(*head).frozen.get()).unwrap() };
        set_context(writing(9, Some(19), Some(11)));
        committed_record(0, 111, true);
        assert!(record_failure(Phase::Spool, Code::Reserve));
        let primary = failure().unwrap();
        let extension = extension_record(&primary, stale, writing(9, Some(19), None), Some(11));
        assert_eq!(extension.uncommitted, Some(range(9, Some(19), (12, 12))));
        assert_eq!(extension.committed_bytes, 111);
        assert_eq!(extension.failed_hour, None);

        reset();
        admit(&[a]);
        admit(&[a, b]);
        set_context(writing(9, Some(19), Some(11)));
        committed_record(0, 111, true);
        set_context(Context {
            campaign_id: Some(OTHER),
            branch: Some(3),
            load_signal: Some(20),
            failed_hour: Some(20),
        });
        committed_record(2, 220, true);
        set_context(writing(9, Some(19), Some(11)));
        assert!(record_failure(Phase::Spool, Code::Reserve));
        assert_eq!(failure().unwrap().failed_hour, None);

        reset();
        admit(&[a]);
        admit(&[a, b]);
        let head = OUTSTANDING.load(Ordering::Acquire);
        let growths = engine_allocation::growths();
        publish_uncommitted([b].into_iter());
        for branch in 100..228 {
            let next = outstanding(range(branch, Some(19), (50, 51)), None);
            admit(&[b, next]);
            publish_uncommitted([b].into_iter());
        }
        assert_eq!(registry_len(), 2);
        assert_eq!(OUTSTANDING.load(Ordering::Acquire), head);
        assert_eq!(engine_allocation::growths(), growths);
        publish_uncommitted(std::iter::empty());
        freeze_outstanding(head);
        let mut pointer = head;
        while let Some(slot) = unsafe { pointer.as_ref() } {
            assert!(slot.published.load().is_none());
            assert!(unsafe { (*slot.frozen.get()).is_none() });
            pointer = slot.next;
        }

        admit(&[a]);
        admit(&[a, b]);
        let mut emitted = false;
        publish_uncommitted(std::iter::from_fn(|| {
            if emitted {
                assert!(record_failure(Phase::Spool, Code::Reserve));
                None
            } else {
                emitted = true;
                Some(a)
            }
        }));
        let mut pointer = OUTSTANDING.load(Ordering::Acquire);
        let mut frozen = 0;
        while let Some(slot) = unsafe { pointer.as_ref() } {
            frozen += usize::from(unsafe { (*slot.frozen.get()).is_some() });
            pointer = slot.next;
        }
        assert_eq!(frozen, 2);

        reset();
        admit(&[outstanding(a.range, None)]);
        set_uncommitted_range(Some(range(15, None, (60, 61))));
        set_context(writing(15, None, Some(62)));
        assert!(record_failure(Phase::Spool, Code::Reserve));
        assert_eq!(
            failure().unwrap().uncommitted,
            Some(range(15, None, (60, 61)))
        );
        let primary = failure().unwrap();
        let head = OUTSTANDING.load(Ordering::Acquire);
        let snapshot = unsafe { (*(*head).frozen.get()).unwrap() };
        let extension = extension_record(&primary, snapshot, Context::default(), None);
        assert_eq!(extension.campaign_id, Some(CAMPAIGN));
        assert_eq!(extension.committed_campaign_id, None);
        assert_eq!(extension.committed_segment, None);
        assert_eq!(extension.committed_bytes, 0);
        assert_eq!(extension.uncommitted, Some(a.range));

        reset();
        engine_allocation::set_hooks(None, None);
    }

    #[cfg(feature = "alloc-fault-injection")]
    #[test]
    fn an_injected_phase_counts_only_threads_that_entered_it() {
        static OBSERVED: AtomicUsize = AtomicUsize::new(0);
        inject(Phase::Bootstrap, 1);
        observe_counted(Some(|| {
            OBSERVED.fetch_add(1, Ordering::Relaxed);
        }));
        std::thread::spawn(|| {
            assert_eq!(phase(), Phase::Unphased);
            assert!(!should_fail(), "an unphased thread is refused");
        })
        .join()
        .unwrap();
        assert_eq!(allocation_count(), 0);
        assert_eq!(OBSERVED.load(Ordering::Relaxed), 0);
        let _bootstrap = enter(Phase::Bootstrap);
        assert!(should_fail());
        assert!(!should_fail());
        observe_counted(None);
        assert_eq!(allocation_count(), 2);
        assert_eq!(OBSERVED.load(Ordering::Relaxed), 2);
    }
}

pub fn fail(phase: Phase, code: Code) {
    record_failure(phase, code);
}

pub fn install_hooks() {
    engine_allocation::set_hooks(Some(should_fail), Some(failure_hook));
    engine_allocation::set_stop_hook(Some(failed));
}
fn failure_hook() {
    fail(phase(), Code::Reserve);
}
pub fn native_error(error: &io::Error) {
    if engine_allocation::is_allocation_error(error) {
        fail(phase(), Code::Native);
    }
}
