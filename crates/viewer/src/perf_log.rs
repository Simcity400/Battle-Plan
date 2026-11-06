//! Opt-in window frame timings, one file per viewer process: `<BATTLE_PLAN_PERF_MAP_LOG>.<pid>.jsonl`.

use job_priority::SpawnCarried as _;
use std::{
    cell::Cell,
    fs::File,
    io::{BufWriter, Write},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
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
    queue: SyncSender<Message>,
    writer: Mutex<Option<JoinHandle<()>>>,
    lost: AtomicBool,
    loss: Mutex<Loss>,
}

static LOG: OnceLock<Option<Log>> = OnceLock::new();

fn log() -> Option<&'static Log> {
    LOG.get_or_init(open).as_ref()
}

fn epoch_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |since| since.as_secs_f64() * 1000.0)
}

fn open() -> Option<Log> {
    let prefix = std::env::var_os("BATTLE_PLAN_PERF_MAP_LOG").or_else(|| std::env::var_os("HOI4_PERF_MAP_LOG")).filter(|path| !path.is_empty())?;
    let pid = std::process::id();
    let mut path = prefix;
    path.push(format!(".{pid}.jsonl"));
    let file = File::create(&path).ok()?;
    let role = match std::env::var("BATTLE_PLAN_PERF_MAP_ROLE").or_else(|_| std::env::var("HOI4_PERF_MAP_ROLE")).as_deref() {
        Ok("popout") => "popout",
        _ => "main",
    };
    let opening = format!(
        r#"{{"t":"map_open","at":{:.3},"pid":{pid},"role":"{role}"}}"#,
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
                        owed_loss(&mut out);
                        let _ = writeln!(out, r#"{{"t":"map_idle","at":{:.3},"pid":{pid}}}"#, epoch_ms());
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
                    owed_loss(&mut out);
                    let _ = writeln!(out, r#"{{"t":"map_close","at":{:.3},"pid":{pid}}}"#, epoch_ms());
                }
                // The host may end this process after any frame.
                let _ = out.flush();
                if finished {
                    break;
                }
            }
        })
        .ok()?;
    Some(Log {
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

    fn record(&self) -> String {
        format!(
            r#"{{"t":"lost","at":{:.3},"pid":{},"source":"map","count":{},"bytes":{},"from":{:.3},"to":{:.3}}}"#,
            epoch_ms(),
            std::process::id(),
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
                if self.queue.try_send(Message::Line(loss.record())).is_err() {
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
fn owed_loss(out: &mut impl Write) {
    let Some(log) = LOG.get().and_then(Option::as_ref) else {
        return;
    };
    if log.lost.load(Ordering::Acquire) {
        let mut loss = log.loss.lock().unwrap_or_else(|error| error.into_inner());
        if loss.count > 0 {
            let _ = writeln!(out, "{}", loss.record());
            *loss = Loss::default();
        }
        log.lost.store(false, Ordering::Release);
    }
}

thread_local! {
    static STARTED: Cell<Option<Instant>> = const { Cell::new(None) };
    static PREVIOUS: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// A window frame begins: eframe's `logic` pass.
pub(crate) fn frame_start() {
    if log().is_some() {
        STARTED.with(|started| started.set(Some(Instant::now())));
    }
}

/// A frame's `ui` pass, which begins the frame when no `logic` pass did.
pub(crate) fn frame_ui() {
    if log().is_some() {
        STARTED.with(|started| {
            if started.get().is_none() {
                started.set(Some(Instant::now()));
            }
        });
    }
}

/// Egui's paint and the present fall outside `work_ms`, inside the interval only.
pub(crate) fn frame_end() {
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
            |previous| format!("{:.3}", started.duration_since(previous).as_secs_f64() * 1000.0),
        );
    let at = epoch_ms();
    let pid = std::process::id();
    log.send(
        format!(r#"{{"t":"map","at":{at:.3},"pid":{pid},"work_ms":{work:.3},"dt":{interval}}}"#),
        at,
    );
}

/// Whether this process records its map timings.
pub(crate) fn enabled() -> bool {
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
pub(crate) fn map_hour(view: Option<&str>, built: &crate::hours::Built, clock: i32) {
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
            r#"{{"t":"map_hour","at":{at:.3},"pid":{},"view":{},"epoch":{},"hour":{},"lag":{},"ok":{},"wall":{:.3},"source":{:.3},"parts":{{{parts}}}}}"#,
            std::process::id(),
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
pub(crate) fn map_upload(
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
            r#"{{"t":"map_upload","at":{at:.3},"pid":{},"view":{},"epoch":{epoch},"hour":{hour},"lag":{},"ok":{ok},"upload":{upload_ms:.3}}}"#,
            std::process::id(),
            view_json(view),
            i64::from(clock) - i64::from(hour),
        ),
        at,
    );
}

/// Writes every queued line and any loss still owed, then waits for the writer to end.
pub(crate) fn finish() {
    let Some(log) = LOG.get().and_then(Option::as_ref) else {
        return;
    };
    let loss = std::mem::take(&mut *log.loss.lock().unwrap_or_else(|error| error.into_inner()));
    if loss.count > 0 {
        let _ = log.queue.send(Message::Line(loss.record()));
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
