//! Status files written on their own thread, so the spool thread never waits
//! on status I/O (#94).
//!
//! The loader owns the [`StatusWriter`] and its thread; the spool thread gets
//! a [`StatusHandle`] and hands each snapshot over with
//! [`StatusHandle::publish`]: a short lock and no I/O. Dropping a handle never
//! waits. The writer thread keeps only the latest unwritten snapshot per path,
//! so a stalled disk or a reader that blocks the replace coalesces snapshots
//! instead of backing up the spool, and through it the capture buffers the
//! game thread waits for. A snapshot published with directories to measure
//! gets their `spool.*` byte total as `spool_bytes`, computed here rather than
//! on the spool thread. A failed write is kept and retried after
//! [`RETRY_DELAY`], unless a newer snapshot for that path replaced it first.
//! The loader [`StatusWriter::finish`]es it (also run on drop) only after the
//! spool thread has returned and every callback is quiescent, then reads the
//! spool's last status.
use serde_json::{Value, json};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, PoisonError},
    thread::JoinHandle,
    time::{Duration, Instant},
};

/// How long a failed write waits before its next attempt.
pub(crate) const RETRY_DELAY: Duration = Duration::from_millis(250);
/// At `finish`, how long pending snapshots keep being retried before the
/// writer gives up on them. A single stuck write cannot be interrupted.
const FINISH_BUDGET: Duration = Duration::from_secs(2);
/// While finishing, the pause between attempts of a failing write.
const FINISH_RETRY_DELAY: Duration = Duration::from_millis(20);

type Write = dyn Fn(&Path, &Value) -> io::Result<()> + Send + Sync;

/// One unwritten snapshot: its file, its value, and the directories whose
/// `spool.*` files it reports as `spool_bytes`, if any.
struct Pending {
    path: PathBuf,
    value: Value,
    measure: Option<Vec<PathBuf>>,
}

#[derive(Default)]
struct State {
    /// The latest unwritten snapshot per path, oldest path first.
    pending: Vec<Pending>,
    stop: bool,
    /// The last write failure not yet taken.
    error: Option<String>,
    writes: u64,
    failures: u64,
    /// Snapshots replaced by a newer one before they were written, including
    /// a failed one a newer snapshot superseded before its retry.
    coalesced: u64,
    write_ns_total: u64,
    write_ns_max: u64,
}

struct Inner {
    state: Mutex<State>,
    wake: Condvar,
}

impl Inner {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The spool thread's side: hand-overs only, never I/O, never a join.
#[derive(Clone)]
pub(crate) struct StatusHandle {
    inner: Arc<Inner>,
}

impl StatusHandle {
    /// Hand a snapshot over without waiting for any I/O. It replaces an
    /// unwritten snapshot of the same path.
    pub(crate) fn publish(&self, path: &Path, value: Value) {
        self.hand_over(path, value, None);
    }

    /// [`Self::publish`], with `spool_bytes` set on the writer thread to the
    /// total size of the `spool.*` files in `directories`.
    pub(crate) fn publish_measured(&self, path: &Path, value: Value, directories: Vec<PathBuf>) {
        self.hand_over(path, value, Some(directories));
    }

    fn hand_over(&self, path: &Path, value: Value, measure: Option<Vec<PathBuf>>) {
        let mut state = self.inner.lock();
        if let Some(entry) = state.pending.iter_mut().find(|entry| entry.path == path) {
            entry.value = value;
            entry.measure = measure;
            state.coalesced += 1;
        } else {
            state.pending.push(Pending {
                path: path.to_path_buf(),
                value,
                measure,
            });
        }
        drop(state);
        self.inner.wake.notify_one();
    }

    /// The last write failure since the previous call, if any.
    pub(crate) fn take_error(&self) -> Option<String> {
        self.inner.lock().error.take()
    }

    /// Counters for `status.json`.
    pub(crate) fn json(&self) -> Value {
        let state = self.inner.lock();
        json!({
            "writes": state.writes,
            "failures": state.failures,
            "coalesced": state.coalesced,
            "pending": state.pending.len(),
            "write_ns_total": state.write_ns_total,
            "write_ns_max": state.write_ns_max,
            "scope": "status and content files written on their own thread; the spool thread only hands the latest snapshot over; spool_bytes is measured on this thread; a failed write is retried later unless a newer snapshot replaced it (coalesced); write time includes that measurement; cumulative since attach",
        })
    }
}

/// The loader's side: owns the writer thread.
pub(crate) struct StatusWriter {
    handle: StatusHandle,
    thread: Option<JoinHandle<()>>,
}

impl StatusWriter {
    /// Start the writer thread; `write` writes one file (production:
    /// `runtime::write_status`).
    pub(crate) fn start(
        write: impl Fn(&Path, &Value) -> io::Result<()> + Send + Sync + 'static,
    ) -> io::Result<Self> {
        let inner = Arc::new(Inner {
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
        });
        let worker = Arc::clone(&inner);
        let write: Box<Write> = Box::new(write);
        let thread = std::thread::Builder::new()
            .name("hoi4-status".into())
            .spawn(move || run(&worker, &*write))?;
        Ok(Self {
            handle: StatusHandle { inner },
            thread: Some(thread),
        })
    }

    pub(crate) fn handle(&self) -> StatusHandle {
        self.handle.clone()
    }

    pub(crate) fn take_error(&self) -> Option<String> {
        self.handle.take_error()
    }

    /// Write what is pending (retrying failures for a short budget) and join
    /// the thread. Call it only where waiting on disk I/O blocks no game
    /// thread: after the spool thread has returned and callbacks are quiescent.
    pub(crate) fn finish(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        self.handle.inner.lock().stop = true;
        self.handle.inner.wake.notify_one();
        let _ = thread.join();
    }
}

impl Drop for StatusWriter {
    fn drop(&mut self) {
        self.finish();
    }
}

/// Total size of the `spool.*` files in `directories`.
fn spool_bytes(directories: &[PathBuf]) -> u64 {
    directories
        .iter()
        .filter_map(|directory| fs::read_dir(directory).ok())
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("spool."))
        .filter_map(|entry| entry.metadata().ok())
        .map(|metadata| metadata.len())
        .sum()
}

fn run(inner: &Inner, write: &Write) {
    let mut stopping_since: Option<Instant> = None;
    loop {
        let mut state = inner.lock();
        while state.pending.is_empty() && !state.stop {
            state = inner
                .wake
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
        if state.stop {
            let since = *stopping_since.get_or_insert_with(Instant::now);
            if state.pending.is_empty() || since.elapsed() >= FINISH_BUDGET {
                return;
            }
        }
        let batch = std::mem::take(&mut state.pending);
        drop(state);
        let mut failed = Vec::new();
        for mut pending in batch {
            let started = Instant::now();
            if let Some(directories) = &pending.measure {
                pending.value["spool_bytes"] = json!(spool_bytes(directories));
            }
            let result = write(&pending.path, &pending.value);
            let ns = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
            let mut state = inner.lock();
            state.write_ns_total = state.write_ns_total.saturating_add(ns);
            state.write_ns_max = state.write_ns_max.max(ns);
            match result {
                Ok(()) => state.writes += 1,
                Err(error) => {
                    state.failures += 1;
                    state.error = Some(error.to_string());
                    failed.push(pending);
                }
            }
        }
        if failed.is_empty() {
            continue;
        }
        // Keep a failed snapshot unless a newer one for its path arrived.
        let mut state = inner.lock();
        for pending in failed {
            if state.pending.iter().any(|entry| entry.path == pending.path) {
                state.coalesced += 1;
            } else {
                state.pending.push(pending);
            }
        }
        // Wait before the retry. A stop request cuts a normal wait short to
        // the shorter finishing pause.
        let stopping = state.stop;
        let deadline = Instant::now()
            + if stopping {
                FINISH_RETRY_DELAY
            } else {
                RETRY_DELAY
            };
        loop {
            let now = Instant::now();
            if now >= deadline || (state.stop && !stopping) {
                break;
            }
            state = inner
                .wake
                .wait_timeout(state, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    /// Generous: every wait in these tests ends in a failure, never a hang.
    const TIMEOUT: Duration = Duration::from_secs(30);

    /// Releases a stalled fake write when dropped, so a failing assertion
    /// unwinds without hanging the writer's join. Declared after the writer,
    /// it drops first.
    struct Release(mpsc::Sender<()>);
    impl Drop for Release {
        fn drop(&mut self) {
            let _ = self.0.send(());
        }
    }

    /// A fake write whose first call reports that it entered, then stalls
    /// until released: the release guard sends, or drops its sender on
    /// unwinding. Every call records what it wrote.
    #[allow(clippy::type_complexity)]
    fn stalling(
        entered: mpsc::Sender<()>,
        released: mpsc::Receiver<()>,
        written: Arc<Mutex<Vec<(PathBuf, Value)>>>,
    ) -> impl Fn(&Path, &Value) -> io::Result<()> + Send + Sync + 'static {
        let entered = Mutex::new(entered);
        let released = Mutex::new(released);
        move |path, value| {
            let first = written.lock().unwrap().is_empty();
            if first {
                let _ = entered.lock().unwrap().send(());
                let _ = released.lock().unwrap().recv();
            }
            written
                .lock()
                .unwrap()
                .push((path.to_path_buf(), value.clone()));
            Ok(())
        }
    }

    /// While a write is stuck, the spool thread's hand-overs return at once
    /// (on another thread, observed with a timeout), and the snapshots
    /// coalesce to the latest per path.
    #[test]
    fn a_stalled_write_never_blocks_publish_and_writes_the_latest_snapshot() {
        let (entered_tx, entered) = mpsc::channel();
        let (release_tx, released) = mpsc::channel();
        let written = Arc::new(Mutex::new(Vec::new()));
        let mut writer =
            StatusWriter::start(stalling(entered_tx, released, Arc::clone(&written))).unwrap();
        let release = Release(release_tx);
        let status = Path::new("status.json");
        let content = Path::new("content.json");
        let handle = writer.handle();
        handle.publish(status, json!({ "hour": 0 }));
        entered
            .recv_timeout(TIMEOUT)
            .expect("the first write stalls");
        let (done_tx, done) = mpsc::channel();
        let publisher = handle.clone();
        std::thread::spawn(move || {
            for hour in 1..=1000 {
                publisher.publish(status, json!({ "hour": hour }));
                publisher.publish(content, json!({ "hour": hour }));
            }
            let _ = done_tx.send(());
        });
        done.recv_timeout(TIMEOUT)
            .expect("hand-overs finish while the write is stuck");
        assert_eq!(
            handle.inner.lock().pending.len(),
            2,
            "one snapshot per path"
        );
        drop(release);
        writer.finish();
        let written = written.lock().unwrap();
        assert_eq!(
            *written,
            [
                (status.to_path_buf(), json!({ "hour": 0 })),
                (status.to_path_buf(), json!({ "hour": 1000 })),
                (content.to_path_buf(), json!({ "hour": 1000 })),
            ]
        );
        assert_eq!(handle.json()["writes"], 3);
        assert_eq!(handle.json()["coalesced"], 1998);
    }

    /// The spool thread ending early (an error return drops its handle) never
    /// waits for a stuck write; the loader's `finish` does, later.
    #[test]
    fn dropping_the_spool_handle_never_waits_for_a_stuck_write() {
        let (entered_tx, entered) = mpsc::channel();
        let (release_tx, released) = mpsc::channel();
        let written = Arc::new(Mutex::new(Vec::new()));
        let mut writer =
            StatusWriter::start(stalling(entered_tx, released, Arc::clone(&written))).unwrap();
        let release = Release(release_tx);
        let spool = writer.handle();
        spool.publish(Path::new("status.json"), json!({ "hour": 1 }));
        entered.recv_timeout(TIMEOUT).expect("the write stalls");
        // Only now, with the write stuck, does the spool side end early.
        let (done_tx, done) = mpsc::channel();
        std::thread::spawn(move || {
            spool.publish(Path::new("status.json"), json!({ "state": "failing" }));
            drop(spool);
            let _ = done_tx.send(());
        });
        done.recv_timeout(TIMEOUT)
            .expect("the spool side returned while the write is stuck");
        drop(release);
        writer.finish();
        assert_eq!(written.lock().unwrap().len(), 2);
    }

    /// A write refused by a reader (sharing violation 32) is retried in the
    /// background, while the writer keeps running, and reported.
    #[test]
    fn a_refused_write_is_retried_in_the_background_and_reported() {
        let (wrote_tx, wrote) = mpsc::channel();
        let wrote_tx = Mutex::new(wrote_tx);
        let attempts = Arc::new(Mutex::new(0));
        let count = Arc::clone(&attempts);
        let mut writer = StatusWriter::start(move |_, _| {
            let mut attempts = count.lock().unwrap();
            *attempts += 1;
            if *attempts <= 2 {
                return Err(io::Error::from_raw_os_error(32));
            }
            let _ = wrote_tx.lock().unwrap().send(());
            Ok(())
        })
        .unwrap();
        let handle = writer.handle();
        handle.publish(Path::new("status.json"), json!({ "hour": 1 }));
        // The retry succeeded while the writer was running; finishing only
        // lets its counters settle before they are read.
        wrote.recv_timeout(TIMEOUT).expect("the retry succeeds");
        writer.finish();
        assert_eq!(*attempts.lock().unwrap(), 3);
        assert_eq!(handle.json()["failures"], 2);
        assert_eq!(handle.json()["writes"], 1);
        let error = handle.take_error().expect("the failure is reported");
        assert!(
            error.contains("32") || error.contains("another process"),
            "{error}"
        );
    }

    /// A newer snapshot published while a write is failing supersedes it: the
    /// failed one is not retried, and it counts as coalesced.
    #[test]
    fn a_newer_snapshot_supersedes_a_failed_write() {
        let (entered_tx, entered) = mpsc::channel();
        let (proceed_tx, proceed) = mpsc::channel::<()>();
        let entered_tx = Mutex::new(entered_tx);
        let proceed = Mutex::new(proceed);
        let (wrote_tx, wrote) = mpsc::channel();
        let wrote_tx = Mutex::new(wrote_tx);
        let attempts = Arc::new(Mutex::new(Vec::<Value>::new()));
        let record = Arc::clone(&attempts);
        let mut writer = StatusWriter::start(move |_, value| {
            let first = record.lock().unwrap().is_empty();
            record.lock().unwrap().push(value.clone());
            if first {
                // Fail only after the test has published the newer snapshot.
                let _ = entered_tx.lock().unwrap().send(());
                let _ = proceed.lock().unwrap().recv_timeout(TIMEOUT);
                return Err(io::Error::from_raw_os_error(32));
            }
            let _ = wrote_tx.lock().unwrap().send(());
            Ok(())
        })
        .unwrap();
        let release = Release(proceed_tx);
        let handle = writer.handle();
        let path = Path::new("status.json");
        handle.publish(path, json!({ "hour": 1 }));
        entered
            .recv_timeout(TIMEOUT)
            .expect("the first write is in flight");
        handle.publish(path, json!({ "hour": 2 }));
        drop(release);
        wrote
            .recv_timeout(TIMEOUT)
            .expect("the newer snapshot is written");
        writer.finish();
        assert_eq!(
            *attempts.lock().unwrap(),
            [json!({ "hour": 1 }), json!({ "hour": 2 })]
        );
        assert_eq!(handle.json()["coalesced"], 1);
        assert_eq!(handle.json()["failures"], 1);
    }

    /// `publish_measured` reports the `spool.*` bytes of its directories,
    /// measured on the writer thread.
    #[test]
    fn measured_snapshots_carry_their_spool_bytes() {
        let directory = std::env::temp_dir().join(format!(
            "hoi4-status-writer-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("spool.0001"), [0_u8; 100]).unwrap();
        fs::write(directory.join("spool.0002"), [0_u8; 23]).unwrap();
        fs::write(directory.join("other.bin"), [0_u8; 7]).unwrap();
        let written = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&written);
        let mut writer = StatusWriter::start(move |_, value| {
            record.lock().unwrap().push(value.clone());
            Ok(())
        })
        .unwrap();
        writer.handle().publish_measured(
            Path::new("status.json"),
            json!({ "hour": 1 }),
            vec![directory.clone()],
        );
        writer.finish();
        let _ = fs::remove_dir_all(&directory);
        assert_eq!(
            *written.lock().unwrap(),
            [json!({ "hour": 1, "spool_bytes": 123 })]
        );
    }
}
