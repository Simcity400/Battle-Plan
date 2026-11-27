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

const QUEUE_LINES: usize = 1024;
const IDLE: Duration = Duration::from_millis(500);

enum Message {
    Line(String),
    Finish,
}

#[derive(Default)]
struct Loss {
    count: u64,
    bytes: u64,
    from: f64,
    to: f64,
}

struct Log {
    map: u64,
    queue: SyncSender<Message>,
    writer: Mutex<Option<JoinHandle<()>>>,
    lost: AtomicBool,
    loss: Mutex<Loss>,
}

thread_local! {
    static LOG: std::cell::OnceCell<Option<&'static Log>> = const { std::cell::OnceCell::new() };
    static ROLE: Cell<&'static str> = const { Cell::new("main") };
}

static OPENED: Mutex<Vec<&'static Log>> = Mutex::new(Vec::new());
static MAPS: AtomicU64 = AtomicU64::new(1);

static STARTUP: Mutex<Vec<(String, f64)>> = Mutex::new(Vec::new());


pub fn first_complete() {
    let Some(path) = std::env::var_os("BATTLEPLAN_FIRST_FRAME_FILE").or_else(|| std::env::var_os("HOI4_FIRST_FRAME_FILE")) else { return; };
    let event = serde_json::json!({"stage": "complete first hour", "at": epoch_ms()});
    if let Ok(bytes) = serde_json::to_vec(&event) {
        let _ = std::fs::write(path, bytes);
    }
}

pub fn source_admission_probe(source: &dashboard_base::Source, options: &content_reader_layers::ResolveOptions) {
    let Some(path) = std::env::var_os("BATTLEPLAN_SOURCE_ADMISSION_FILE").or_else(|| std::env::var_os("HOI4_SOURCE_ADMISSION_FILE")) else { return; };
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
    loaders: Option<content_reader_layers::parallel::cpu::Measurement>,
    source: Option<String>,
    peak_before: Option<u64>,
}

fn requested() -> bool {
    static REQUESTED: OnceLock<bool> = OnceLock::new();
    *REQUESTED.get_or_init(|| {
        let requested =
            std::env::var_os("BATTLEPLAN_PERF_MAP_LOG").or_else(|| std::env::var_os("HOI4_PERF_MAP_LOG")).is_some_and(|path| !path.is_empty());
        if requested {
            account();
        }
        requested
    })
}

fn account() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(replay_frame::cpu::enable);
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
            self.loaders = Some(content_reader_layers::parallel::cpu::Measurement::start());
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
                (&raw mut memory).cast(),
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

static PREFIX: OnceLock<std::ffi::OsString> = OnceLock::new();

pub fn log_to(prefix: &std::path::Path) {
    let _ = PREFIX.set(prefix.as_os_str().to_owned());
    viewer_gpu::gpu_frame_time::request();
    account();
}

pub fn enter(role: &'static str) {
    ROLE.with(|named| named.set(role));
}

pub fn epoch_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |since| since.as_secs_f64() * 1000.0)
}

fn open() -> Option<Log> {
    account();
    let prefix = PREFIX
        .get()
        .cloned()
        .or_else(|| std::env::var_os("BATTLEPLAN_PERF_MAP_LOG").or_else(|| std::env::var_os("HOI4_PERF_MAP_LOG")))
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

static GPU_TIMER: OnceLock<Result<(), String>> = OnceLock::new();

fn json_string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "null".to_owned())
}

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

const SCOPE_FRAMES: u64 = 30;

struct Scopes {
    before: std::collections::HashMap<String, [Option<u64>; 3]>,
    reads: u64,
    frames: u64,
    from: f64,
    work_ms: f64,
}

thread_local! {
    static VIEW: Cell<Option<(bool, f32)>> = const { Cell::new(None) };
    static SCOPES: RefCell<Option<Scopes>> = const { RefCell::new(None) };
    static FRAME: Cell<u64> = const { Cell::new(0) };
    static GPU: RefCell<VecDeque<(u64, Result<f64, String>)>> = const { RefCell::new(VecDeque::new()) };
    static STARTED: Cell<Option<Instant>> = const { Cell::new(None) };
    static PREVIOUS: Cell<Option<Instant>> = const { Cell::new(None) };
    static PAINTED: Cell<Option<(f64, f64, f64)>> = const { Cell::new(None) };
}

pub fn frame_painted(elapsed: Duration, acquire_secs: f32, presented: bool) {
    if let Some(log) = log() {
        let at = epoch_ms();
        PAINTED.with(|painted| {
            painted.set(Some((
                elapsed.as_secs_f64() * 1000.0,
                f64::from(acquire_secs) * 1000.0,
                at,
            )));
        });
        if presented && let Some(frame) = frame_number().checked_sub(1) {
            log.send(format!(r#"{{"t":"map_present","at":{at:.3},"map":{},"frame":{frame}}}"#, log.map), at);
        }
    }
}

pub fn frame_start() {
    if log().is_some() {
        STARTED.with(|started| started.set(Some(Instant::now())));
    }
}

pub fn frame_ui() {
    if log().is_some() {
        STARTED.with(|started| {
            if started.get().is_none() {
                started.set(Some(Instant::now()));
            }
        });
    }
}

pub fn frame_id() -> Option<u64> {
    log()?;
    STARTED.with(Cell::get).map(|_| FRAME.with(Cell::get))
}

pub fn frame_number() -> u64 {
    FRAME.with(Cell::get)
}

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

pub fn mark(name: &str, fields: serde_json::Value) {
    let Some(log) = log() else {
        return;
    };
    let at = epoch_ms();
    log.send(format!(r#"{{"t":"map_mark","at":{at:.3},"map":{},"name":{},"fields":{fields}}}"#, log.map, json_string(name)), at);
}

pub fn frame_view(free: bool, height: f32) {
    if log().is_some() {
        VIEW.with(|view| view.set(Some((free, height))));
    }
}

pub fn gpu_times(times: Vec<(u64, Result<f64, String>)>) {
    if !times.is_empty() && log().is_some() {
        GPU.with(|gpu| gpu.borrow_mut().extend(times));
    }
}

pub fn frame_end(drawn: Option<(replay_data::BranchId, replay_data::Hour, f64)>) {
    let frame = FRAME.with(|frame| frame.replace(frame.get() + 1));
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
    let (gpu_frame, gpu_ms, gpu_error) = match GPU.with(|gpu| gpu.borrow_mut().pop_front()) {
        None => ("null".to_owned(), "null".to_owned(), "null".to_owned()),
        Some((frame, Ok(ms))) => (frame.to_string(), format!("{ms:.3}"), "null".to_owned()),
        Some((frame, Err(why))) => (frame.to_string(), "null".to_owned(), json_string(&why)),
    };
    let at = epoch_ms();
    let map = log.map;
    let drawn = drawn.map_or_else(String::new, |(branch, hour, at)| format!(r#","branch":"{branch}","hour":{hour},"drawn_at":{at:.3}"#));
    log.send(
        format!(
            r#"{{"t":"map","at":{at:.3},"map":{map},"frame":{frame},"work_ms":{work:.3},"dt":{interval},"paint_ms":{paint},"acquire_ms":{acquire},"gpu_frame":{gpu_frame},"gpu_ms":{gpu_ms},"gpu_error":{gpu_error},"painted_at":{painted}{drawn}}}"#
        ),
        at,
    );
    scope_window(log, work, at);
}

fn scope_window(log: &Log, work: f64, at: f64) {
    SCOPES.with(|held| {
        let mut held = held.borrow_mut();
        let window = held.get_or_insert_with(|| Scopes {
            before: replay_frame::cpu::own_totals(),
            reads: replay_frame::cpu::cycle_reads(),
            frames: 0,
            from: at,
            work_ms: 0.0,
        });
        window.frames += 1;
        window.work_ms += work;
        if window.frames < SCOPE_FRAMES {
            return;
        }
        let _own = replay_frame::cpu::scope("frame.perf-log", Some("other"));
        let after = replay_frame::cpu::own_totals();
        let mut scopes = String::from("{");
        let mut walls = String::from("{");
        let mut calls = String::from("{");
        for (label, [ns, wall, entered]) in &after {
            let [was_ns, was_wall, was_entered] =
                window.before.get(label).copied().unwrap_or([None, None, None]);
            let spent = ns.unwrap_or(0).saturating_sub(was_ns.unwrap_or(0));
            if spent > 0 {
                append(&mut scopes, label, spent / 1000);
            }
            if let Some(wall) = wall {
                let waited = wall.saturating_sub(was_wall.unwrap_or(0));
                if waited > 0 {
                    append(&mut walls, label, waited / 1000);
                }
            }
            let entered = entered.unwrap_or(0).saturating_sub(was_entered.unwrap_or(0));
            if entered > 0 {
                append(&mut calls, label, entered);
            }
        }
        scopes.push('}');
        walls.push('}');
        calls.push('}');
        let reads = replay_frame::cpu::cycle_reads();
        let camera = VIEW.with(Cell::get).map_or_else(
            || "null".to_owned(),
            |(free, height)| {
                let pose = if free { "free" } else { "map" };
                format!(r#"{{"pose":"{pose}","height":{height:.1}}}"#)
            },
        );
        let line = format!(
            r#"{{"t":"map_scopes","at":{at:.3},"map":{},"from":{:.3},"to":{at:.3},"frames":{},"work_ms":{:.3},"cycle_reads":{},"camera":{camera},"scopes_us":{scopes},"scopes_wall_us":{walls},"scopes_calls":{calls}}}"#,
            log.map,
            window.from,
            window.frames,
            window.work_ms,
            reads.saturating_sub(window.reads),
        );
        *window = Scopes { before: after, reads, frames: 0, from: at, work_ms: 0.0 };
        drop(held);
        log.send(line, at);
    });
}

fn append(out: &mut String, label: &str, value: u64) {
    if out.len() > 1 {
        out.push(',');
    }
    out.push_str(&json_string(label));
    out.push(':');
    out.push_str(&value.to_string());
}

pub fn enabled() -> bool {
    log().is_some()
}

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
