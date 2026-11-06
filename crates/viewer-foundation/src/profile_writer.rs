//! Shared asynchronous profiling output for viewer workers and GPU passes.

use job_priority::SpawnCarried as _;
use std::{
    io::Write,
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, SyncSender, TrySendError},
    },
};

/// Opt-in window CPU timings; absent from the ordinary repaint path.
pub fn profiling() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("BATTLE_PLAN_VIEWER_PROFILE").or_else(|| std::env::var_os("HOI4_VIEWER_PROFILE")).is_some())
}

/// Lines the writer holds while stderr is behind: seconds of frames at the
/// window's few lines a frame. Past it, new lines are dropped and counted.
const QUEUED_LINES: usize = 4096;

enum ProfileLine {
    Line(String),
    /// This many lines were dropped here because the queue was full.
    Dropped(u64),
    Flush(mpsc::Sender<()>),
}

/// A writer thread fed through a bounded queue.
///
/// A frame never waits on the reader of the output: when the queue is full the
/// line is dropped, and the writer reports how many were dropped at the point
/// they went missing.
struct Sink {
    queue: SyncSender<ProfileLine>,
    dropped: AtomicU64,
}

impl Sink {
    fn start(mut out: impl Write + Send + 'static, capacity: usize) -> Option<Self> {
        let (queue, lines) = mpsc::sync_channel(capacity);
        std::thread::Builder::new()
            .name("profile writer".into())
            .spawn_carried(move || {
                for line in lines {
                    match line {
                        ProfileLine::Line(line) => {
                            let _ = writeln!(out, "{line}");
                        }
                        ProfileLine::Dropped(count) => {
                            let _ = writeln!(
                                out,
                                "profile writer: {count} lines dropped while the output was behind"
                            );
                        }
                        ProfileLine::Flush(done) => {
                            let _ = out.flush();
                            let _ = done.send(());
                        }
                    }
                }
            })
            .ok()
            .map(|_| Self {
                queue,
                dropped: AtomicU64::new(0),
            })
    }

    /// Queues `line` without waiting; gives it back if the writer is gone.
    fn print(&self, line: String) -> Result<(), String> {
        let dropped = self.dropped.swap(0, Ordering::Relaxed);
        if dropped > 0 {
            match self.queue.try_send(ProfileLine::Dropped(dropped)) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    self.dropped.fetch_add(dropped + 1, Ordering::Relaxed);
                    return Ok(());
                }
                Err(TrySendError::Disconnected(_)) => return Err(line),
            }
        }
        match self.queue.try_send(ProfileLine::Line(line)) {
            Err(TrySendError::Full(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Err(TrySendError::Disconnected(ProfileLine::Line(line))) => Err(line),
            _ => Ok(()),
        }
    }

    /// Waits until every line queued so far, and any drop count, is written.
    fn flush(&self) {
        let dropped = self.dropped.swap(0, Ordering::Relaxed);
        if dropped > 0 && self.queue.send(ProfileLine::Dropped(dropped)).is_err() {
            return;
        }
        let (done, written) = mpsc::channel();
        if self.queue.send(ProfileLine::Flush(done)).is_ok() {
            let _ = written.recv();
        }
    }
}

/// The profile writer, started by the first line printed.
static SINK: OnceLock<Option<Sink>> = OnceLock::new();

fn profile_sink() -> Option<&'static Sink> {
    SINK.get_or_init(|| Sink::start(std::io::stderr(), QUEUED_LINES))
        .as_ref()
}

/// Prints a profiling line from a thread of its own.
///
/// Written from the frame, a line waited whenever the reader of stderr fell
/// behind: 7 to 18 ms frames at 60 recorded hours a second whose only cost was
/// the print.
pub fn profile_print(line: String) {
    match profile_sink() {
        Some(sink) => {
            if let Err(line) = sink.print(line) {
                eprintln!("{line}");
            }
        }
        None => eprintln!("{line}"),
    }
}

/// Waits until every profiling line printed so far is written, whichever
/// switch printed it (`BATTLE_PLAN_VIEWER_PROFILE`, `_GPU_PASSES`, `_BENCH_FRAMES`).
pub fn profile_flush() {
    if let Some(Some(sink)) = SINK.get() {
        sink.flush();
    }
}

#[cfg(test)]
mod tests;
