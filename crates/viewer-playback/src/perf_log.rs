//! Opt-in window frame timings, one file per map: `<BATTLE_PLAN_PERF_MAP_LOG>.<map>.jsonl`, each map of
//! the process numbered from 1 as it first records.

use job_priority::SpawnCarried as _;
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    fs::File,
    io::{BufWriter, Write},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, RecvTimeoutError, SyncSender},
    },
    thread::JoinHandle,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Lines waiting for the writer; a full queue drops new lines rather than hold a frame.
const QUEUE_LINES: usize = 1024;
/// How long the writer waits on an empty queue before marking it idle.
const IDLE: Duration = Duration::from_millis(500);

enum Message {
    Line(String),
    Finish,
}

/// Lines dropped since the last loss record.
#[derive(Default)]
struct Loss {
    count: u64,
    bytes: u64,
    from: f64,
    to: f64,
}

struct Log {
    /// This map's number in the process, which its records carry.
    map: u64,
    queue: SyncSender<Message>,
    writer: Mutex<Option<JoinHandle<()>>>,
    lost: AtomicBool,
    loss: Mutex<Loss>,
}

thread_local! {
    /// This map thread's log, opened at its first record.
    static LOG: std::cell::OnceCell<Option<&'static Log>> = const { std::cell::OnceCell::new() };
    /// The host's name for this thread's map ([`enter`]).
    static ROLE: Cell<&'static str> = const { Cell::new("main") };
}

/// Every map's log, which [`finish`] closes.
static OPENED: Mutex<Vec<&'static Log>> = Mutex::new(Vec::new());
static MAPS: AtomicU64 = AtomicU64::new(1);

static STARTUP: Mutex<Vec<(String, f64)>> = Mutex::new(Vec::new());


pub fn first_complete() {
    let Some(path) = std::env::var_os("BATTLE_PLAN_FIRST_FRAME_FILE").or_else(|| std::env::var_os("HOI4_FIRST_FRAME_FILE")) else { return; };
    let event = serde_json::json!({"stage": "complete first hour", "at": epoch_ms()});
    if let Ok(bytes) = serde_json::to_vec(&event) {
        let _ = std::fs::write(path, bytes);
    }
}

pub fn source_admission_probe(source: &dashboard_base::Source, options: &installed_content_core::ResolveOptions) {
    let Some(path) = std::env::var_os("BATTLE_PLAN_SOURCE_ADMISSION_FILE").or_else(|| std::env::var_os("HOI4_SOURCE_ADMISSION_FILE")) else { return; };
    let measure = || -> std::io::Result<serde_json::Value> {
        let content = source.content(options)?;
        let before = content.layers.content_counts().generation_validations;
        let mut sequential_us = Vec::new();
        for _ in 0..100 {
            let started = Instant::now();
            let current = source.content(options)?;
            sequential_us.push(started.elapsed().as_secs_f64() * 1e6);
            if !std::sync::Arc::ptr_eq(&current, &content) {
                return Err(std::io::Error::other("admission probe changed content generation"));
            }
        }
        let started = Instant::now();
        std::thread::scope(|scope| -> std::io::Result<()> {
            let jobs: Vec<_> = (0..20).map(|_| {
                let expected = &content;
                scope.spawn(move || {
                    let current = source.content(options)?;
                    if !std::sync::Arc::ptr_eq(&current, expected) {
                        return Err(std::io::Error::other("concurrent admission changed content generation"));
                    }
                    Ok(())
                })
            }).collect();
            for job in jobs {
                job.join().map_err(|_| std::io::Error::other("admission probe worker panicked"))??;
            }
            Ok(())
        })?;
        let concurrent_us = started.elapsed().as_secs_f64() * 1e6;
        let after = content.layers.content_counts().generation_validations;
        Ok(serde_json::json!({
            "source_id": source.key().source_id, "revision": source.key().revision,
            "pid": std::process::id(),
            "sequential_us": sequential_us, "concurrent_us": concurrent_us,
            "validations_before": before, "validations_after": after,
            "scope": "Map Source after its complete frame; 100 cached sequential admissions and 20 concurrent admissions, including thread creation; no transport"
        }))
    };
    let result = measure().unwrap_or_else(|error| serde_json::json!({"error": error.to_string()}));
    if let Ok(bytes) = serde_json::to_vec(&result) {
        let _ = std::fs::write(path, bytes);
    }
}

pub struct StartupSpan {
    name: &'static str,
    started: Option<(Instant, Option<u64>)>,
    loaders: Option<installed_content_core::parallel::cpu::Measurement>,
    source: Option<String>,
    peak_before: Option<u64>,
}

fn requested() -> bool {
    static REQUESTED: OnceLock<bool> = OnceLock::new();
    *REQUESTED
        .get_or_init(|| std::env::var_os("BATTLE_PLAN_PERF_MAP_LOG").or_else(|| std::env::var_os("HOI4_PERF_MAP_LOG")).is_some_and(|path| !path.is_empty()))
}

pub fn hour_view(epoch: u64, branch: replay_data::BranchId, hour: replay_data::Hour, view: crate::hours::View) {
    let Some(log) = log() else { return; };
    let at = epoch_ms();
    log.send(serde_json::json!({
        "t": "map_view", "at": at, "pid": std::process::id(),
        "epoch": epoch, "branch": branch, "hour": hour,
        "viewport": view.viewport, "ui_scale": view.ui_scale,
    }).to_string(), at);
}

impl StartupSpan {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            started: requested().then(|| (Instant::now(), replay_frame::cpu::process_cpu_ns())),
            loaders: None,
            source: None,
            peak_before: None,
        }
    }

    pub fn with_loaders(mut self, path: &std::path::Path) -> Self {
        if self.started.is_some() {
            self.loaders = Some(installed_content_core::parallel::cpu::Measurement::start());
            self.source = Some(path.display().to_string());
            self.peak_before = startup_memory().map(|memory| memory.0);
        }
        self
    }
}

impl Drop for StartupSpan {
    fn drop(&mut self) {
        let Some((started, cpu)) = self.started.take() else {
            return;
        };
        let loaders = self.loaders.take().map(|measurement| measurement.finish());
        let at = epoch_ms();
        let cpu = cpu
            .zip(replay_frame::cpu::process_cpu_ns())
            .map(|(from, to)| to.saturating_sub(from) as f64 / 1e6);
        let memory = startup_memory();
        let line = serde_json::json!({
            "t": "map_stage", "at": at, "pid": std::process::id(), "name": self.name,
            "ms": started.elapsed().as_secs_f64() * 1000.0, "process_cpu_ms": cpu,
            "loader_cpu_ms": loaders.as_ref().and_then(|sample| sample.ns).map(|ns| ns as f64 / 1e6),
            "loader_threads": loaders.as_ref().map(|sample| sample.threads), "source": self.source,
            "process_peak_commit_before_bytes": self.peak_before,
            "process_peak_commit_bytes": memory.map(|memory| memory.0),
            "process_private_bytes": memory.map(|memory| memory.1),
            "process_peak_working_set_bytes": memory.map(|memory| memory.2),
        }).to_string();
        let mut pending = STARTUP
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(log) = LOG.with(|log| log.get().copied().flatten()) {
            drop(pending);
            log.send(line, at);
        } else {
            pending.push((line, at));
        }
    }
}

fn startup_memory() -> Option<(u64, u64, u64)> {
    #[cfg(windows)]
    {
        #[repr(C)]
        #[derive(Default)]
        struct Memory {
            size: u32,
            faults: u32,
            peak_working: usize,
            working: usize,
            peak_paged: usize,
            paged: usize,
            peak_nonpaged: usize,
            nonpaged: usize,
            pagefile: usize,
            peak_pagefile: usize,
            private: usize,
        }
        #[link(name = "psapi")]
        unsafe extern "system" {
            fn GetProcessMemoryInfo(
                process: windows_sys::Win32::Foundation::HANDLE,
                counters: *mut std::ffi::c_void,
                size: u32,
            ) -> i32;
        }
        let mut memory = Memory {
            size: std::mem::size_of::<Memory>() as u32,
            ..Default::default()
        };
        let ok = unsafe {
            GetProcessMemoryInfo(
                windows_sys::Win32::System::Threading::GetCurrentProcess(),
                (&mut memory as *mut Memory).cast(),
                memory.size,
            )
        };
        (ok != 0).then_some((
            memory.peak_pagefile as u64,
            memory.private as u64,
            memory.peak_working as u64,
        ))
    }
    #[cfg(not(windows))]
    None
}

fn log() -> Option<&'static Log> {
    let log = LOG.with(|log| {
        *log.get_or_init(|| {
            let log: &'static Log = Box::leak(Box::new(open()?));
            OPENED.lock().unwrap_or_else(|error| error.into_inner()).push(log);
            Some(log)
        })
    })?;
    let pending = std::mem::take(
        &mut *STARTUP
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    );
    for (line, at) in pending {
        log.send(line, at);
    }
    Some(log)
}

/// Where the maps' files go: the host's log beside its own, else `BATTLE_PLAN_PERF_MAP_LOG`.
static PREFIX: OnceLock<std::ffi::OsString> = OnceLock::new();

pub fn log_to(prefix: &std::path::Path) {
    let _ = PREFIX.set(prefix.as_os_str().to_owned());
    viewer_foundation::map::gpu_frame_time::request();
}

/// Names the map this thread runs, before it records.
pub fn enter(role: &'static str) {
    ROLE.with(|named| named.set(role));
}

/// Milliseconds since the Unix epoch, the clock the host and the page measure with too.
pub fn epoch_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |since| since.as_secs_f64() * 1000.0)
}

fn open() -> Option<Log> {
    let prefix = PREFIX
        .get()
        .cloned()
        .or_else(|| std::env::var_os("BATTLE_PLAN_PERF_MAP_LOG").or_else(|| std::env::var_os("HOI4_PERF_MAP_LOG")))
        .filter(|path| !path.is_empty())?;
    let pid = std::process::id();
    let map = MAPS.fetch_add(1, Ordering::Relaxed);
    let mut path = prefix;
    path.push(format!(".{map}.jsonl"));
    let file = File::create(&path).ok()?;
    let role = ROLE.with(Cell::get);
    let (timer, reason) = match GPU_TIMER.get() {
        Some(Ok(())) => ("timestamps", "null".to_owned()),
        Some(Err(reason)) => ("unsupported", json_string(reason)),
        None => (
            "unsupported",
            json_string("the log opened before the map device existed"),
        ),
    };
    let opening = format!(
        r#"{{"t":"map_open","at":{:.3},"map":{map},"pid":{pid},"role":"{role}","gpu_timer":"{timer}","gpu_timer_reason":{reason}}}"#,
        epoch_ms()
    );
    let (queue, lines) = mpsc::sync_channel::<Message>(QUEUE_LINES);
    let writer = std::thread::Builder::new()
        .name("perf map log".into())
        .spawn_carried(move || {
            let mut out = BufWriter::new(file);
            let _ = writeln!(out, "{opening}");
            let _ = out.flush();
            loop {
                let first = match lines.recv_timeout(IDLE) {
                    Ok(first) => first,
                    // An idle mark tells the host this queue held nothing older.
                    Err(RecvTimeoutError::Timeout) => {
                        owed_loss(&mut out, map);
                        let _ = writeln!(out, r#"{{"t":"map_idle","at":{:.3},"map":{map}}}"#, epoch_ms());
                        let _ = out.flush();
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                };
                let mut finished = false;
                for message in std::iter::once(first).chain(lines.try_iter()) {
                    match message {
                        Message::Line(line) => {
                            let _ = writeln!(out, "{line}");
                        }
                        Message::Finish => finished = true,
                    }
                }
                if finished {
                    owed_loss(&mut out, map);
                    let _ = writeln!(out, r#"{{"t":"map_close","at":{:.3},"map":{map}}}"#, epoch_ms());
                }
                // The host may end after any frame.
                let _ = out.flush();
                if finished {
                    break;
                }
            }
        })
        .ok()?;
    Some(Log {
        map,
        queue,
        writer: Mutex::new(Some(writer)),
        lost: AtomicBool::new(false),
        loss: Mutex::new(Loss::default()),
    })
}

impl Loss {
    fn add(&mut self, bytes: usize, at: f64) {
        if self.count == 0 {
            self.from = at;
        }
        self.count += 1;
        self.bytes += bytes as u64 + 1;
        self.to = at;
    }

    fn record(&self, map: u64) -> String {
        format!(
            r#"{{"t":"lost","at":{:.3},"map":{map},"source":"map","count":{},"bytes":{},"from":{:.3},"to":{:.3}}}"#,
            epoch_ms(),
            self.count,
            self.bytes,
            self.from,
            self.to
        )
    }
}

impl Log {
    /// Queues `line` without waiting, after the loss record any earlier drop owes.
    fn send(&self, line: String, at: f64) {
        if self.lost.load(Ordering::Acquire) {
            let mut loss = self.loss.lock().unwrap_or_else(|error| error.into_inner());
            if loss.count > 0 {
                if self.queue.try_send(Message::Line(loss.record(self.map))).is_err() {
                    loss.add(line.len(), at);
                    return;
                }
                *loss = Loss::default();
            }
            self.lost.store(false, Ordering::Release);
        }
        let bytes = line.len();
        if self.queue.try_send(Message::Line(line)).is_err() {
            self.loss
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .add(bytes, at);
            self.lost.store(true, Ordering::Release);
        }
    }
}

/// Writes the loss record a full queue still owes, so no idle or close mark claims a drained queue.
fn owed_loss(out: &mut impl Write, map: u64) {
    let opened = OPENED.lock().unwrap_or_else(|error| error.into_inner());
    let Some(log) = opened.iter().find(|log| log.map == map) else {
        return;
    };
    if log.lost.load(Ordering::Acquire) {
        let mut loss = log.loss.lock().unwrap_or_else(|error| error.into_inner());
        if loss.count > 0 {
            let _ = writeln!(out, "{}", loss.record(map));
            *loss = Loss::default();
        }
        log.lost.store(false, Ordering::Release);
    }
}

/// Whether the map device can time its frames, set once it exists ([`gpu_timer`]).
static GPU_TIMER: OnceLock<Result<(), String>> = OnceLock::new();

fn json_string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "null".to_owned())
}

/// Records whether the map device can time its frames, before the log opens.
pub fn gpu_timer(support: Result<(), String>) {
    let _ = GPU_TIMER.set(support);
}

pub fn since(name: &str, started: Instant) {
    let Some(log) = log() else {
        return;
    };
    let at = epoch_ms();
    log.send(
        format!(
            r#"{{"t":"map_stage","at":{at:.3},"map":{},"name":{},"ms":{:.3},"builds_total":{}}}"#,
            log.map,
            json_string(name),
            started.elapsed().as_secs_f64() * 1000.0,
            shader_pipeline::gpu::stats::builds_total(),
        ),
        at,
    );
}

thread_local! {
    /// The number the next `map` record carries.
    static FRAME: Cell<u64> = const { Cell::new(0) };
    /// Frames' GPU milliseconds, or why they have none, not yet written, oldest first.
    static GPU: RefCell<VecDeque<(u64, Result<f64, String>)>> = const { RefCell::new(VecDeque::new()) };
    static STARTED: Cell<Option<Instant>> = const { Cell::new(None) };
    static PREVIOUS: Cell<Option<Instant>> = const { Cell::new(None) };
    /// The last paint's milliseconds, whole and waiting for a swapchain image, and the epoch
    /// milliseconds it ended.
    static PAINTED: Cell<Option<(f64, f64, f64)>> = const { Cell::new(None) };
}

/// eframe painted and presented a frame after its `ui` pass, in `elapsed`, `acquire_secs` of it
/// waiting for a swapchain image.
pub fn frame_painted(elapsed: Duration, acquire_secs: f32) {
    if log().is_some() {
        PAINTED.with(|painted| {
            painted.set(Some((
                elapsed.as_secs_f64() * 1000.0,
                f64::from(acquire_secs) * 1000.0,
                epoch_ms(),
            )));
        });
    }
}

/// A window frame begins: eframe's `logic` pass.
pub fn frame_start() {
    if log().is_some() {
        STARTED.with(|started| started.set(Some(Instant::now())));
    }
}

/// A frame's `ui` pass, which begins the frame when no `logic` pass did.
pub fn frame_ui() {
    if log().is_some() {
        STARTED.with(|started| {
            if started.get().is_none() {
                started.set(Some(Instant::now()));
            }
        });
    }
}

/// The number of the window frame under way, for timing its GPU work; none when no frame is
/// under way or the log is off.
pub fn frame_id() -> Option<u64> {
    log()?;
    STARTED.with(Cell::get).map(|_| FRAME.with(Cell::get))
}

/// The rendered frame's opt-in profile, keyed with its window and GPU records.
pub fn profile_frame(line: &str) {
    let Some(frame) = frame_id() else {
        return;
    };
    let log = log().unwrap();
    let at = epoch_ms();
    log.send(
        format!(
            r#"{{"t":"map_profile","at":{at:.3},"map":{},"frame":{frame},"line":{}}}"#,
            log.map,
            json_string(line),
        ),
        at,
    );
}

/// Frames' GPU milliseconds just settled; each later `map` record carries one.
pub fn gpu_times(times: Vec<(u64, Result<f64, String>)>) {
    if !times.is_empty() && log().is_some() {
        GPU.with(|gpu| gpu.borrow_mut().extend(times));
    }
}

/// Egui's output, tessellation, paint and present fall outside `work_ms`, inside the interval
/// only; the record carries the paint that ended the previous frame (`paint_ms`, `acquire_ms`,
/// and `painted_at`, when it was presented), null when none did.
pub fn frame_end() {
    let Some(log) = log() else {
        return;
    };
    let Some(started) = STARTED.with(Cell::take) else {
        return;
    };
    let work = started.elapsed().as_secs_f64() * 1000.0;
    let interval = PREVIOUS
        .with(|previous| previous.replace(Some(started)))
        .map_or_else(
            || "null".to_owned(),
            |previous| {
                format!(
                    "{:.3}",
                    started.duration_since(previous).as_secs_f64() * 1000.0
                )
            },
        );
    let (paint, acquire, painted) = PAINTED.with(Cell::take).map_or_else(
        || ("null".to_owned(), "null".to_owned(), "null".to_owned()),
        |(paint, acquire, painted)| {
            (
                format!("{paint:.3}"),
                format!("{acquire:.3}"),
                format!("{painted:.3}"),
            )
        },
    );
    let frame = FRAME.with(|frame| frame.replace(frame.get() + 1));
    // The GPU time of an earlier frame, read back since; `gpu_frame` names it.
    let (gpu_frame, gpu_ms, gpu_error) = match GPU.with(|gpu| gpu.borrow_mut().pop_front()) {
        None => ("null".to_owned(), "null".to_owned(), "null".to_owned()),
        Some((frame, Ok(ms))) => (frame.to_string(), format!("{ms:.3}"), "null".to_owned()),
        Some((frame, Err(why))) => (frame.to_string(), "null".to_owned(), json_string(&why)),
    };
    let at = epoch_ms();
    let map = log.map;
    log.send(
        format!(
            r#"{{"t":"map","at":{at:.3},"map":{map},"frame":{frame},"work_ms":{work:.3},"dt":{interval},"paint_ms":{paint},"acquire_ms":{acquire},"gpu_frame":{gpu_frame},"gpu_ms":{gpu_ms},"gpu_error":{gpu_error},"painted_at":{painted}}}"#
        ),
        at,
    );
}

/// Whether this map records its timings.
pub fn enabled() -> bool {
    log().is_some()
}

fn view_json(view: Option<&str>) -> String {
    view.map_or_else(
        || "null".to_owned(),
        |view| serde_json::to_string(view).unwrap_or_else(|_| "null".to_owned()),
    )
}

/// A built map hour reaching the window: where its build time went, and how
/// far behind the clock it arrived. `parts` holds each lap, decode first.
pub fn map_hour(view: Option<&str>, built: &crate::hours::Built, clock: i32) {
    use std::fmt::Write as _;
    let Some(log) = log() else {
        return;
    };
    let at = epoch_ms();
    let mut parts = String::new();
    for (name, timing) in &built.laps {
        if !parts.is_empty() {
            parts.push(',');
        }
        let _ = write!(parts, r#""{name}":{:.3}"#, timing.ms);
    }
    log.send(
        format!(
            r#"{{"t":"map_hour","at":{at:.3},"map":{},"view":{},"epoch":{},"hour":{},"lag":{},"ok":{},"wall":{:.3},"source":{:.3},"parts":{{{parts}}}}}"#,
            log.map,
            view_json(view),
            built.epoch,
            built.hour,
            i64::from(clock) - i64::from(built.hour),
            built.layers.is_ok(),
            built.wall,
            built.waits.source_ms,
        ),
        at,
    );
}

/// A built map hour installed on screen: its upload time and lag behind the clock.
pub fn map_upload(
    view: Option<&str>,
    epoch: u64,
    hour: i32,
    clock: i32,
    upload_ms: f64,
    ok: bool,
) {
    let Some(log) = log() else {
        return;
    };
    let at = epoch_ms();
    log.send(
        format!(
            r#"{{"t":"map_upload","at":{at:.3},"map":{},"view":{},"epoch":{epoch},"hour":{hour},"lag":{},"ok":{ok},"upload":{upload_ms:.3}}}"#,
            log.map,
            view_json(view),
            i64::from(clock) - i64::from(hour),
        ),
        at,
    );
}

/// Writes every queued line and any loss still owed, then waits for the writer to end.
pub fn finish() {
    let opened = OPENED.lock().unwrap_or_else(|error| error.into_inner()).clone();
    for log in opened {
        let loss = std::mem::take(&mut *log.loss.lock().unwrap_or_else(|error| error.into_inner()));
        if loss.count > 0 {
            let _ = log.queue.send(Message::Line(loss.record(log.map)));
        }
        if log.queue.send(Message::Finish).is_ok()
            && let Some(writer) = log
                .writer
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take()
        {
            let _ = writer.join();
        }
    }
}
