use crate::hook::{Hook, Threads};
use crossbeam_queue::ArrayQueue;
use engine::{
    PINNED_BUILD,
    build::hash_executable,
    ccountry,
    cdlc_manager::{self, Descriptor},
    cgame_state::{self, Clock},
    cprovince,
    memory::LiveMemory,
    read,
    walk::Outcome,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    cell::UnsafeCell,
    ffi::{OsString, c_void},
    fs::{self, File, OpenOptions},
    io::{self, Write},
    mem::size_of,
    os::windows::ffi::OsStringExt,
    path::PathBuf,
    ptr,
    sync::atomic::{AtomicBool, AtomicI64, AtomicPtr, AtomicU64, AtomicUsize, Ordering},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, HMODULE, WAIT_OBJECT_0},
    System::{
        LibraryLoader::{FreeLibraryAndExitThread, GetModuleFileNameW, GetModuleHandleW},
        Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
        Threading::*,
    },
};

const BUFFERS: usize = 256;
const ROSTER_CAPACITY: usize = 64;
static SHARED: AtomicPtr<Shared> = AtomicPtr::new(ptr::null_mut());
static ACTIVE: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Site {
    Tick,
    Daily,
    CountryEvents,
}

impl Site {
    fn address(self) -> (usize, [u8; 16], usize) {
        match self {
            Self::Tick => (
                cgame_state::TICK_CANDIDATE_RVA,
                cgame_state::TICK_CANDIDATE_SENTINEL,
                tick as *const () as usize,
            ),
            Self::Daily => (
                cgame_state::DAILY_CANDIDATE_RVA,
                cgame_state::DAILY_CANDIDATE_SENTINEL,
                daily as *const () as usize,
            ),
            Self::CountryEvents => (
                ccountry::DELAYED_EVENTS_RVA,
                ccountry::DELAYED_EVENTS_SENTINEL,
                country_events as *const () as usize,
            ),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    session_id: String,
    output_dir: PathBuf,
    site: Site,
}

struct Frame {
    entry: Option<Clock>,
    before_walk: Option<Clock>,
    after_walk: Option<Clock>,
    thread: u32,
    argument: u64,
    qpc: i64,
    original_ticks: i64,
    clock_ticks: [i64; 3],
    table_ticks: [i64; 4],
    comparison_ticks: i64,
    roster_ticks: i64,
    roster: Option<Box<Roster>>,
    province_results: [Outcome; 2],
    country_results: [Outcome; 2],
    province_mismatches: usize,
    country_mismatches: usize,
    provinces: [Vec<cprovince::Row>; 2],
    countries: [Vec<ccountry::Row>; 2],
}

impl Frame {
    fn new(tables: bool) -> Self {
        let unavailable = Outcome::Unavailable {
            reason: "not sampled at this site",
        };
        Self {
            entry: None,
            before_walk: None,
            after_walk: None,
            thread: 0,
            argument: 0,
            qpc: 0,
            original_ticks: 0,
            clock_ticks: [0; 3],
            table_ticks: [0; 4],
            comparison_ticks: 0,
            roster_ticks: 0,
            roster: None,
            province_results: [unavailable; 2],
            country_results: [unavailable; 2],
            province_mismatches: 0,
            country_mismatches: 0,
            provinces: std::array::from_fn(|_| {
                vec![cprovince::Row::default(); if tables { 20_000 } else { 0 }]
            }),
            countries: std::array::from_fn(|_| {
                vec![ccountry::Row::default(); if tables { 512 } else { 0 }]
            }),
        }
    }

    fn allocated_bytes(&self) -> usize {
        size_of::<Self>()
            + self
                .provinces
                .iter()
                .map(|v| v.capacity() * size_of::<cprovince::Row>())
                .sum::<usize>()
            + self
                .countries
                .iter()
                .map(|v| v.capacity() * size_of::<ccountry::Row>())
                .sum::<usize>()
            + self
                .roster
                .as_ref()
                .map_or(0, |roster| roster.allocated_bytes())
    }
}

struct RosterPass {
    dlc_filter_strings: Vec<Descriptor>,
    requested_mods: Vec<Descriptor>,
    dlcs: Vec<cdlc_manager::Entry>,
    mods: Vec<cdlc_manager::Entry>,
    observation: cdlc_manager::Observation,
}

impl RosterPass {
    fn new() -> Self {
        let unavailable = Outcome::Unavailable {
            reason: "not sampled",
        };
        Self {
            dlc_filter_strings: vec![Descriptor::default(); ROSTER_CAPACITY],
            requested_mods: vec![Descriptor::default(); ROSTER_CAPACITY],
            dlcs: vec![cdlc_manager::Entry::default(); ROSTER_CAPACITY],
            mods: vec![cdlc_manager::Entry::default(); ROSTER_CAPACITY],
            observation: cdlc_manager::Observation {
                dlc_filter_strings: unavailable,
                requested_mods: unavailable,
                dlcs: unavailable,
                mods: unavailable,
            },
        }
    }

    fn allocated_bytes(&self) -> usize {
        (self.dlc_filter_strings.capacity() + self.requested_mods.capacity())
            * size_of::<Descriptor>()
            + (self.dlcs.capacity() + self.mods.capacity()) * size_of::<cdlc_manager::Entry>()
    }
}

struct Roster {
    passes: [RosterPass; 2],
    sampled: bool,
    qpc: i64,
    hour: Option<i32>,
    ticks: i64,
    mismatches: [usize; 4],
}

impl Roster {
    fn new() -> Self {
        Self {
            passes: std::array::from_fn(|_| RosterPass::new()),
            sampled: false,
            qpc: 0,
            hour: None,
            ticks: 0,
            mismatches: [0; 4],
        }
    }

    fn allocated_bytes(&self) -> usize {
        size_of::<Self>()
            + self
                .passes
                .iter()
                .map(RosterPass::allocated_bytes)
                .sum::<usize>()
    }

    fn sample(&mut self, memory: &LiveMemory, base: usize, hour: Option<i32>) {
        self.qpc = qpc();
        self.hour = hour;
        for pass in &mut self.passes {
            pass.observation = cdlc_manager::observe(
                memory,
                base,
                cdlc_manager::Buffers {
                    dlc_filter_strings: &mut pass.dlc_filter_strings,
                    requested_mods: &mut pass.requested_mods,
                    dlcs: &mut pass.dlcs,
                    mods: &mut pass.mods,
                },
            );
        }
        let [left, right] = &self.passes;
        self.mismatches = [
            row_differences(
                &left.dlc_filter_strings,
                &right.dlc_filter_strings,
                [
                    left.observation.dlc_filter_strings,
                    right.observation.dlc_filter_strings,
                ],
            ),
            row_differences(
                &left.requested_mods,
                &right.requested_mods,
                [
                    left.observation.requested_mods,
                    right.observation.requested_mods,
                ],
            ),
            row_differences(
                &left.dlcs,
                &right.dlcs,
                [left.observation.dlcs, right.observation.dlcs],
            ),
            row_differences(
                &left.mods,
                &right.mods,
                [left.observation.mods, right.observation.mods],
            ),
        ];
        self.sampled = true;
        self.ticks = qpc() - self.qpc;
    }
}

struct Shared {
    base: usize,
    trampoline: usize,
    site: Site,
    frames: Vec<UnsafeCell<Frame>>,
    completed: Vec<AtomicI64>,
    free: ArrayQueue<usize>,
    ready: ArrayQueue<usize>,
    dropped: AtomicU64,
    recording: AtomicBool,
}

// Each index has exactly one owner: free queue, callback, ready queue, or worker.
// Frame access requires that index; publication is completed before worker access.
unsafe impl Sync for Shared {}

fn qpc() -> i64 {
    let mut ticks = 0;
    unsafe {
        QueryPerformanceCounter(&mut ticks);
    }
    ticks
}

unsafe extern "system" fn tick(state: usize, commands: usize) {
    unsafe {
        callback(state, commands);
    }
}
unsafe extern "system" fn daily(state: usize) {
    unsafe {
        callback(state, 0);
    }
}
unsafe extern "system" fn country_events(country: usize) {
    unsafe {
        callback(country, 0);
    }
}

unsafe fn callback(argument: usize, commands: usize) {
    ACTIVE.fetch_add(1, Ordering::SeqCst);
    let entered = qpc();
    // Published before the patch is enabled; freed only after restore and drain.
    let shared = unsafe { &*SHARED.load(Ordering::Acquire) };
    let index = if shared.recording.load(Ordering::Relaxed) {
        shared.free.pop()
    } else {
        None
    };
    let memory = unsafe { LiveMemory::new() };
    if let Some(index) = index {
        let frame = unsafe { &mut *shared.frames[index].get() };
        let clock_begin = qpc();
        frame.entry = cgame_state::read_clock(&memory, shared.base);
        frame.clock_ticks[0] = qpc() - clock_begin;
        frame.thread = unsafe { GetCurrentThreadId() };
        frame.argument = argument as u64;
        frame.qpc = entered;
    }
    let original_begin = qpc();
    if shared.site == Site::Tick {
        let original: unsafe extern "system" fn(usize, usize) =
            unsafe { std::mem::transmute(shared.trampoline) };
        unsafe {
            original(argument, commands);
        }
    } else {
        let original: unsafe extern "system" fn(usize) =
            unsafe { std::mem::transmute(shared.trampoline) };
        unsafe {
            original(argument);
        }
    }
    let original_end = qpc();
    if let Some(index) = index {
        let frame = unsafe { &mut *shared.frames[index].get() };
        frame.original_ticks = original_end - original_begin;
        let clock_begin = qpc();
        frame.before_walk = cgame_state::read_clock(&memory, shared.base);
        frame.clock_ticks[1] = qpc() - clock_begin;
        let unavailable = Outcome::Unavailable {
            reason: if shared.site == Site::Tick {
                "clock unavailable"
            } else {
                "not sampled at this site"
            },
        };
        frame.province_results = [unavailable; 2];
        frame.country_results = [unavailable; 2];
        frame.table_ticks = [0; 4];
        if shared.site == Site::Tick
            && let Some(clock) = frame.before_walk
        {
            let state = clock.game_state as usize;
            for pass in 0..2 {
                let begin = qpc();
                frame.province_results[pass] =
                    cprovince::walk(&memory, shared.base, state, &mut frame.provinces[pass]);
                let middle = qpc();
                frame.country_results[pass] =
                    ccountry::walk(&memory, shared.base, state, &mut frame.countries[pass]);
                frame.table_ticks[pass * 2] = middle - begin;
                frame.table_ticks[pass * 2 + 1] = qpc() - middle;
            }
        }
        let compare_begin = qpc();
        frame.province_mismatches = differences(&frame.provinces, frame.province_results);
        frame.country_mismatches = differences(&frame.countries, frame.country_results);
        frame.comparison_ticks = qpc() - compare_begin;
        frame.roster_ticks = 0;
        if shared.site == Site::Tick
            && let Some(roster) = frame.roster.as_mut()
            && !roster.sampled
        {
            roster.sample(
                &memory,
                shared.base,
                frame.before_walk.map(|clock| clock.hour),
            );
            frame.roster_ticks = roster.ticks;
        }
        let clock_begin = qpc();
        frame.after_walk = cgame_state::read_clock(&memory, shared.base);
        frame.clock_ticks[2] = qpc() - clock_begin;
        // No frame mutation after publication. Completion includes queue-push cost.
        shared.completed[index].store(0, Ordering::Relaxed);
        if shared.ready.push(index).is_err() {
            shared.dropped.fetch_add(1, Ordering::Relaxed);
            let _ = shared.free.push(index);
        } else {
            shared.completed[index].store(
                (qpc() - entered - (original_end - original_begin)).max(1),
                Ordering::Release,
            );
        }
    } else if shared.recording.load(Ordering::Relaxed) {
        shared.dropped.fetch_add(1, Ordering::Relaxed);
    }
    // No calls after this decrement; unload also checks suspended instruction pointers.
    ACTIVE.fetch_sub(1, Ordering::SeqCst);
}

fn differences<T: PartialEq>(rows: &[Vec<T>; 2], results: [Outcome; 2]) -> usize {
    row_differences(&rows[0], &rows[1], results)
}

fn row_differences<T: PartialEq>(left: &[T], right: &[T], results: [Outcome; 2]) -> usize {
    if results[0] != results[1] {
        return 1;
    }
    let len = results[0].len();
    left[..len]
        .iter()
        .zip(&right[..len])
        .filter(|(a, b)| a != b)
        .count()
}

struct Event(HANDLE);
impl Event {
    fn create(name: &str) -> Result<Self, String> {
        let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let handle = unsafe { CreateEventW(ptr::null(), 1, 0, name.as_ptr()) };
        if handle.is_null() {
            Err(io::Error::last_os_error().to_string())
        } else {
            Ok(Self(handle))
        }
    }
    fn set(&self) {
        unsafe {
            SetEvent(self.0);
        }
    }
    fn signaled(&self) -> bool {
        unsafe { WaitForSingleObject(self.0, 0) == WAIT_OBJECT_0 }
    }
}
impl Drop for Event {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn module_path(module: HMODULE) -> Result<PathBuf, String> {
    let mut buffer = vec![0; 32768];
    let len =
        unsafe { GetModuleFileNameW(module, buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    if len == 0 || len == buffer.len() {
        return Err("GetModuleFileNameW failed or truncated".into());
    }
    Ok(PathBuf::from(OsString::from_wide(&buffer[..len])))
}

pub unsafe extern "system" fn worker(module: *mut c_void) -> u32 {
    let result = run(module);
    if let Err(error) = result
        && let Ok(path) = module_path(module)
    {
        let _ = fs::write(path.with_file_name("collector.error.txt"), error);
    }
    // A failed patch publication can retain a live trampoline and its state.
    // Returning ends this worker but leaves LoadLibrary's reference in place.
    if !SHARED.load(Ordering::Acquire).is_null() {
        return 1;
    }
    unsafe {
        FreeLibraryAndExitThread(module, 0);
    }
}

fn run(module: HMODULE) -> Result<(), String> {
    let dll_path = module_path(module)?;
    let config: Config = serde_json::from_slice(
        &fs::read(dll_path.with_file_name("collector.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if config.session_id.is_empty()
        || !config
            .session_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err("invalid probe session id".into());
    }
    fs::create_dir_all(&config.output_dir).map_err(|e| e.to_string())?;
    let prefix = format!("Local\\HOI4Replay.Probe.{}", config.session_id);
    let ready = Event::create(&format!("{prefix}.ready"))?;
    let detach = Event::create(&format!("{prefix}.detach"))?;
    let done = Event::create(&format!("{prefix}.done"))?;
    let mut log = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(config.output_dir.join("probe.jsonl"))
        .map_err(|e| e.to_string())?;
    let exe =
        hash_executable(File::open(module_path(ptr::null_mut())?).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    if !PINNED_BUILD.matches(&exe) {
        return Err(format!(
            "unsupported game build; wanted {} ({} bytes, {:02x?}), got {} bytes, {:02x?}",
            PINNED_BUILD.version,
            PINNED_BUILD.exe_size,
            PINNED_BUILD.exe_sha256,
            exe.size,
            exe.sha256
        ));
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) } as usize;
    let memory = unsafe { LiveMemory::new() };
    let (rva, sentinel, callback) = config.site.address();
    if read::<[u8; 16]>(&memory, base + rva) != Some(sentinel) {
        return Err(format!("candidate sentinel differs at RVA {rva:#x}"));
    }
    let mut hook = unsafe { Hook::new(base + rva, callback, sentinel) }?;
    let mut shared = Box::new(Shared {
        base,
        trampoline: hook.trampoline(),
        site: config.site,
        frames: (0..BUFFERS)
            .map(|_| UnsafeCell::new(Frame::new(config.site == Site::Tick)))
            .collect(),
        completed: (0..BUFFERS).map(|_| AtomicI64::new(0)).collect(),
        free: ArrayQueue::new(BUFFERS),
        ready: ArrayQueue::new(BUFFERS),
        dropped: AtomicU64::new(0),
        recording: AtomicBool::new(true),
    });
    // The first free index carries the only roster allocation. The worker takes
    // and drops it after logging; returning that frame never resamples the roster.
    if config.site == Site::Tick {
        unsafe { &mut *shared.frames[0].get() }.roster = Some(Box::new(Roster::new()));
    }
    for index in 0..BUFFERS {
        shared
            .free
            .push(index)
            .map_err(|_| "pool initialization failed")?;
    }
    let mut frequency = 0;
    if unsafe { QueryPerformanceFrequency(&mut frequency) } == 0 || frequency <= 0 {
        return Err("QPC unavailable".into());
    }
    let pool_bytes: usize = shared
        .frames
        .iter()
        .map(|f| unsafe { &*f.get() }.allocated_bytes())
        .sum();
    line(&mut log, &json!({"record":"start", "session":config.session_id, "pid":unsafe { GetCurrentProcessId() }, "site_rva":rva, "qpc_frequency":frequency, "pool_bytes":pool_bytes, "queue_capacity":BUFFERS, "roster_capacity":ROSTER_CAPACITY, "exe_sha256":format!("{:02x?}", exe.sha256), "cost_scope":"entry reads and post-return double walks, comparisons, clock, one-time roster double read and queue publication; excludes original engine call and final atomic decrement/epilogue"})).map_err(|e| e.to_string())?;
    log.sync_all().map_err(|e| e.to_string())?;
    // A timed-out loader can start us after its recorder exited. The marker is
    // durable even if nobody still holds the named detach event at that point.
    if dll_path.with_file_name("collector.cancel").exists() || detach.signaled() {
        return Err("probe cancelled before hook installation".into());
    }
    SHARED.store(&mut *shared, Ordering::Release);
    let install_deadline = Instant::now() + Duration::from_secs(15);
    let mut installation_error = None;
    loop {
        match hook.set_enabled(true) {
            Ok(true) => break,
            Ok(false) if Instant::now() < install_deadline => {
                std::thread::sleep(Duration::from_millis(1))
            }
            error => {
                if hook.enabled() {
                    // Keep this worker alive to undo a partially published hook.
                    installation_error = Some(format!(
                        "hook installation failed after writing bytes: {error:?}"
                    ));
                    break;
                }
                SHARED.store(ptr::null_mut(), Ordering::Release);
                return Err(format!("hook installation failed: {error:?}"));
            }
        }
    }
    if installation_error.is_none() {
        ready.set();
    }
    let mut io_failure = installation_error;
    let mut synced = Instant::now();
    while !detach.signaled() && io_failure.is_none() {
        if let Err(error) = drain(&shared, &mut log, frequency) {
            io_failure = Some(error.to_string());
            break;
        }
        if synced.elapsed() >= Duration::from_secs(5) {
            if let Err(error) = log.sync_all() {
                io_failure = Some(error.to_string());
                break;
            }
            synced = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    shared.recording.store(false, Ordering::Release);
    // Errors after installation cannot unwind through an enabled hook. Keep trying
    // restoration while the game continues; a failed detach never unloads code.
    loop {
        match hook.set_enabled(false) {
            Ok(true) => break,
            result => {
                let _ = fs::write(
                    dll_path.with_file_name("collector.error.txt"),
                    format!("detach pending: {result:?}"),
                );
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    let pe_offset = read::<u32>(&memory, module as usize + 0x3c).unwrap_or(0) as usize;
    let image_size = read::<u32>(&memory, module as usize + pe_offset + 0x50).unwrap_or(0) as usize;
    let [relay, trampoline] = hook.executable_ranges();
    let ranges = [
        module as usize..module as usize + image_size,
        relay,
        trampoline,
    ];
    let _ = line(
        &mut log,
        &json!({"record":"restored", "qpc":qpc(), "active_callbacks":ACTIVE.load(Ordering::SeqCst), "original_bytes_verified":true}),
    );
    let mut reported = Instant::now();
    loop {
        let outcome = if image_size == 0 {
            Err("collector image size unreadable".to_string())
        } else if ACTIVE.load(Ordering::SeqCst) != 0 {
            Ok(false)
        } else {
            Threads::open().and_then(|mut threads| {
                let quiet = threads.suspend().and_then(|()| {
                    if ACTIVE.load(Ordering::SeqCst) != 0 {
                        Ok(false)
                    } else {
                        threads.contains_ip(&ranges).map(|present| !present)
                    }
                });
                let resumed = threads.resume();
                // Formatting and I/O must follow resuming every suspended thread.
                drop(threads);
                resumed.map_err(|code| format!("ResumeThread failed: {code}"))?;
                quiet.map_err(|code| format!("thread context check failed: {code}"))
            })
        };
        if outcome == Ok(true) {
            break;
        }
        if reported.elapsed() >= Duration::from_secs(1) {
            let _ = line(
                &mut log,
                &json!({"record":"unload_pending", "active_callbacks":ACTIVE.load(Ordering::SeqCst), "error":outcome.err()}),
            );
            reported = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    if io_failure.is_none() {
        io_failure = drain(&shared, &mut log, frequency)
            .err()
            .map(|e| e.to_string());
    }
    let _ = line(
        &mut log,
        &json!({"record":"detach", "dropped_calls":shared.dropped.load(Ordering::Relaxed), "io_error":io_failure, "original_bytes_verified":true, "active_callbacks":0}),
    );
    let flush_result = log.sync_all().map_err(|e| e.to_string());
    SHARED.store(ptr::null_mut(), Ordering::Release);
    drop(hook);
    drop(shared);
    drop(log);
    done.set();
    flush_result?;
    if let Some(error) = io_failure {
        return Err(error);
    }
    Ok(())
}

fn drain(shared: &Shared, log: &mut File, frequency: i64) -> io::Result<()> {
    while let Some(index) = shared.ready.pop() {
        let mut total = shared.completed[index].load(Ordering::Acquire);
        while total == 0 {
            std::thread::yield_now();
            total = shared.completed[index].load(Ordering::Acquire);
        }
        let frame = unsafe { &mut *shared.frames[index].get() };
        let ms = |ticks: i64| ticks as f64 * 1000.0 / frequency as f64;
        let roster = frame.roster.take();
        let result = line(
            log,
            &json!({
                "record":"call", "qpc":frame.qpc, "thread_id":frame.thread, "argument":frame.argument,
                "entry":clock_json(frame.entry), "before_walk":clock_json(frame.before_walk), "after_walk":clock_json(frame.after_walk),
                "total_ms":ms(total), "original_ms":ms(frame.original_ticks), "clock_ms":frame.clock_ticks.map(ms), "table_ms":frame.table_ticks.map(ms), "comparison_ms":ms(frame.comparison_ticks),
                "roster_ms":ms(frame.roster_ticks), "roster":roster.as_deref().map(|roster| roster_json(roster, frequency)),
                "provinces":frame.province_results.map(outcome_json), "countries":frame.country_results.map(outcome_json),
                "province_mismatches":frame.province_mismatches, "country_mismatches":frame.country_mismatches,
                "dropped_calls":shared.dropped.load(Ordering::Relaxed)
            }),
        );
        drop(roster);
        for pass in 0..2 {
            if let Outcome::Truncated { len, unread_rows } = frame.province_results[pass] {
                frame.provinces[pass].resize(len + unread_rows as usize, cprovince::Row::default());
            }
            if let Outcome::Truncated { len, unread_rows } = frame.country_results[pass] {
                frame.countries[pass].resize(len + unread_rows as usize, ccountry::Row::default());
            }
        }
        let _ = shared.free.push(index);
        result?;
    }
    Ok(())
}

fn line(file: &mut File, value: &Value) -> io::Result<()> {
    serde_json::to_writer(&mut *file, value)?;
    file.write_all(b"\n")?;
    file.flush()
}

fn clock_json(clock: Option<Clock>) -> Value {
    clock.map_or(Value::Null, |clock| json!({"game_state":clock.game_state, "hour":clock.hour, "play_time":clock.play_time, "play_session_started_at":clock.play_session_started_at, "seed":clock.seed, "campaign_id":clock.campaign_id.as_str(), "campaign_bytes":clock.campaign_id.as_bytes(), "speed":clock.speed, "tick_busy":clock.tick_busy}))
}

fn outcome_json(outcome: Outcome) -> Value {
    match outcome {
        Outcome::Rows { len } => json!({"kind":"rows", "rows":len}),
        Outcome::Truncated { len, unread_rows } => {
            json!({"kind":"truncated", "rows":len, "unread_rows":unread_rows})
        }
        Outcome::Unavailable { reason } => json!({"kind":"unavailable", "reason":reason}),
    }
}

fn roster_json(roster: &Roster, frequency: i64) -> Value {
    let passes = roster.passes.iter().map(|pass| {
        let observation = pass.observation;
        json!({
            "dlc_filter_strings": {
                "outcome":outcome_json(observation.dlc_filter_strings),
                "rows":pass.dlc_filter_strings[..observation.dlc_filter_strings.len()]
                    .iter().map(text_json).collect::<Vec<_>>()
            },
            "requested_mods": {
                "outcome":outcome_json(observation.requested_mods),
                "rows":pass.requested_mods[..observation.requested_mods.len()]
                    .iter().map(text_json).collect::<Vec<_>>()
            },
            "dlcs": {
                "outcome":outcome_json(observation.dlcs),
                "entries":pass.dlcs[..observation.dlcs.len()].iter().map(content_entry_json).collect::<Vec<_>>()
            },
            "mods": {
                "outcome":outcome_json(observation.mods),
                "entries":pass.mods[..observation.mods.len()].iter().map(content_entry_json).collect::<Vec<_>>()
            }
        })
    }).collect::<Vec<_>>();
    let complete = roster.passes.iter().all(|pass| {
        let observation = pass.observation;
        [
            observation.dlc_filter_strings,
            observation.requested_mods,
            observation.dlcs,
            observation.mods,
        ]
        .iter()
        .all(|outcome| matches!(outcome, Outcome::Rows { .. }))
    });
    json!({
        "qpc":roster.qpc, "hour":roster.hour, "sampled":roster.sampled,
        "roster_ms":roster.ticks as f64 * 1000.0 / frequency as f64,
        "passes":passes, "all_lists_complete":complete,
        "matching_observations":roster.mismatches == [0;4],
        "mismatches": {
            "dlc_filter_strings":roster.mismatches[0], "requested_mods":roster.mismatches[1],
            "dlcs":roster.mismatches[2], "mods":roster.mismatches[3]
        },
        "membership":"reported_active reproduces the engine's Active DLC/Mod diagnostics",
        "dlc_filter_strings_meaning":"mutable manager values mixing initial descriptor requests and runtime display names; not the launcher disabled-DLC list",
        "enumeration_order":"discovered entries retain engine map order; requested descriptors retain request order",
        "vfs_mount_order":"unknown; diagnostic enumeration is not VFS load order",
        "content_verification":"unchecked; no layer hash or launcher comparison performed"
    })
}

fn text_json<const N: usize>(text: &engine::pdx::Text<N>) -> Value {
    json!({"text":text.as_str(), "bytes":text.as_bytes()})
}

fn content_entry_json(entry: &cdlc_manager::Entry) -> Value {
    json!({"name":text_json(&entry.name), "descriptor":text_json(&entry.descriptor), "reported_active":entry.reported_active})
}
