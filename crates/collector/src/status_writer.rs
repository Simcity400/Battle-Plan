//! Coalesced checked status snapshots with reusable byte buffers.
use crate::{allocation, status::status};
use std::{
    io,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, PoisonError},
    thread::JoinHandle,
    time::{Duration, Instant},
};

pub(crate) const RETRY_DELAY: Duration = Duration::from_millis(250);
const FINISH_BUDGET: Duration = Duration::from_secs(2);
const FINISH_RETRY_DELAY: Duration = Duration::from_millis(20);
struct Pending {
    path: PathBuf,
    bytes: Vec<u8>,
    measure: Option<Vec<PathBuf>>,
}
#[derive(Default)]
struct State {
    pending: Vec<Pending>,
    recycled: Vec<Pending>,
    stop: bool,
    error: Option<&'static str>,
    writes: u64,
    failures: u64,
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
#[derive(Clone)]
pub(crate) struct StatusHandle {
    inner: Arc<Inner>,
}
impl StatusHandle {
    pub(crate) fn publish(&self, path: &Path, value: impl serde::Serialize) -> io::Result<()> {
        self.publish_snapshot(path, value, None)
    }
    pub(crate) fn publish_measured(
        &self,
        path: &Path,
        value: impl serde::Serialize,
        directories: Vec<PathBuf>,
    ) -> io::Result<()> {
        self.publish_snapshot(path, value, Some(directories))
    }
    fn publish_snapshot(
        &self,
        path: &Path,
        value: impl serde::Serialize,
        measure: Option<Vec<PathBuf>>,
    ) -> io::Result<()> {
        if allocation::failed() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        let mut state = self.inner.lock();
        let position = state.pending.iter().position(|p| p.path == path);
        let mut pending = if let Some(index) = position {
            state.coalesced += 1;
            state.pending.remove(index)
        } else if let Some(index) = state.recycled.iter().position(|p| p.path == path) {
            state.recycled.swap_remove(index)
        } else {
            Pending {
                path: path.to_path_buf(),
                bytes: Vec::new(),
                measure: None,
            }
        };
        drop(state);
        crate::status::serialize_into(&mut pending.bytes, &value)?;
        pending.measure = measure;
        let mut state = self.inner.lock();
        if allocation::failed() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        engine::allocation::reserve(&mut state.pending, 1)?;
        if let Some(index) = position {
            let index = index.min(state.pending.len());
            state.pending.insert(index, pending);
        } else {
            state.pending.push(pending);
        }
        drop(state);
        self.inner.wake.notify_one();
        Ok(())
    }
    pub(crate) fn take_error(&self) -> Option<&'static str> {
        self.inner.lock().error.take()
    }
    pub(crate) fn status(&self) -> impl serde::Serialize {
        let state = self.inner.lock();
        status!({
            "writes":state.writes,"failures":state.failures,"coalesced":state.coalesced,
            "pending":state.pending.len(),"write_ns_total":state.write_ns_total,"write_ns_max":state.write_ns_max,
            "scope":"status and content files written on their own thread; the spool thread only hands the latest snapshot over; spool_bytes is measured on this thread; a failed write is retried later unless a newer snapshot replaced it (coalesced); write time includes that measurement; cumulative since attach",
        })
    }
    #[cfg(test)]
    fn json(&self) -> serde_json::Value {
        serde_json::to_value(self.status()).unwrap()
    }
}
pub(crate) struct StatusWriter {
    handle: StatusHandle,
    thread: Option<JoinHandle<()>>,
}
impl StatusWriter {
    pub(crate) fn start(
        write: impl Fn(&Path, &[u8]) -> io::Result<()> + Send + Sync + 'static,
    ) -> io::Result<Self> {
        let inner = Arc::new(Inner {
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
        });
        let worker = inner.clone();
        let thread = std::thread::Builder::new()
            .name("collector-status".into())
            .spawn(move || run(&worker, &write))?;
        Ok(Self {
            handle: StatusHandle { inner },
            thread: Some(thread),
        })
    }
    pub(crate) fn handle(&self) -> StatusHandle {
        self.handle.clone()
    }
    #[cfg(test)]
    pub(crate) fn take_error(&self) -> Option<&'static str> {
        self.handle.take_error()
    }
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
fn spool_bytes(directories: &[PathBuf]) -> io::Result<u64> {
    let mut total = 0u64;
    for directory in directories {
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().starts_with("spool.") {
                total = total.saturating_add(entry.metadata()?.len());
            }
        }
    }
    Ok(total)
}
fn measure(pending: &mut Pending) -> io::Result<()> {
    let Some(directories) = &pending.measure else {
        return Ok(());
    };
    measure_bytes(&mut pending.bytes, directories)?;
    pending.measure = None;
    Ok(())
}
pub(crate) fn measure_bytes(bytes: &mut Vec<u8>, directories: &[PathBuf]) -> io::Result<()> {
    let value = spool_bytes(directories)?;
    // Comma, key, maximum u64 decimal digits, and the closing brace.
    engine::allocation::reserve(bytes, 1 + b"\"spool_bytes\":".len() + 20 + 1)?;
    if bytes.last() != Some(&b'}') {
        return Err(io::ErrorKind::InvalidData.into());
    }
    bytes.pop();
    if bytes.len() > 1 {
        bytes.push(b',');
    }
    bytes.extend_from_slice(b"\"spool_bytes\":");
    let mut digits = [0u8; 20];
    let mut index = digits.len();
    let mut number = value;
    loop {
        index -= 1;
        digits[index] = b'0' + (number % 10) as u8;
        number /= 10;
        if number == 0 {
            break;
        }
    }
    bytes.extend_from_slice(&digits[index..]);
    bytes.push(b'}');
    Ok(())
}
fn run(inner: &Inner, write: &impl Fn(&Path, &[u8]) -> io::Result<()>) {
    let _phase = allocation::enter(allocation::Phase::Status);
    allocation::attempted_hour(None);
    let mut stopping_since: Option<Instant> = None;
    loop {
        let mut state = inner.lock();
        while state.pending.is_empty() && !state.stop && !allocation::failed() {
            state = inner
                .wake
                .wait_timeout(state, RETRY_DELAY)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        if allocation::failed() {
            state.pending.clear();
            return;
        }
        if state.stop {
            let since = *stopping_since.get_or_insert_with(Instant::now);
            if state.pending.is_empty() || since.elapsed() >= FINISH_BUDGET {
                return;
            }
        }
        let mut pending = state.pending.remove(0);
        drop(state);
        let started = Instant::now();
        let result = measure(&mut pending).and_then(|()| write(&pending.path, &pending.bytes));
        let ns = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
        let mut state = inner.lock();
        state.write_ns_total = state.write_ns_total.saturating_add(ns);
        state.write_ns_max = state.write_ns_max.max(ns);
        match result {
            Ok(()) => {
                state.writes += 1;
                pending.bytes.clear();
                if let Some(old) = state.recycled.iter_mut().find(|p| p.path == pending.path) {
                    if pending.bytes.capacity() > old.bytes.capacity() {
                        *old = pending;
                    }
                } else if engine::allocation::reserve(&mut state.recycled, 1).is_ok() {
                    state.recycled.push(pending);
                }
                continue;
            }
            Err(error) => {
                allocation::observe(&error);
                state.failures += 1;
                state.error = Some("status write failed");
                if state.pending.iter().any(|entry| entry.path == pending.path) {
                    state.coalesced += 1;
                } else {
                    if engine::allocation::reserve(&mut state.pending, 1).is_err() {
                        return;
                    }
                    state.pending.push(pending);
                }
            }
        }
        let stopping = state.stop;
        let deadline = Instant::now()
            + if stopping {
                FINISH_RETRY_DELAY
            } else {
                RETRY_DELAY
            };
        loop {
            let now = Instant::now();
            if now >= deadline || (state.stop && !stopping) || allocation::failed() {
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
pub(crate) struct AllocationProbe {
    writer: StatusWriter,
    status: PathBuf,
    directory: PathBuf,
}
#[cfg(test)]
impl AllocationProbe {
    pub(crate) fn prepare(directory: &Path) -> io::Result<Self> {
        let directory = directory.join("status-writer");
        std::fs::create_dir_all(&directory)?;
        std::fs::write(directory.join("spool.0001"), [0; 23])?;
        let status = directory.join("status.json");
        Ok(Self {
            writer: StatusWriter::start(crate::runtime::write_status_bytes)?,
            status,
            directory,
        })
    }
    pub(crate) fn run(&mut self) -> io::Result<()> {
        let _phase = allocation::enter(allocation::Phase::Status);
        allocation::attempted_hour(None);
        let mut directories = Vec::new();
        engine::allocation::reserve_exact(&mut directories, 1)?;
        directories.push(self.directory.clone());
        let published = self.writer.handle().publish_measured(
            &self.status,
            status!({"state":"recording","hours":2u64}),
            directories,
        );
        self.writer.finish();
        published?;
        if allocation::failed() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        if let Some(_error) = self.writer.take_error() {
            return Err(io::ErrorKind::Other.into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::{
        fs,
        sync::{Arc, mpsc},
    };

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
    ) -> impl Fn(&Path, &[u8]) -> io::Result<()> + Send + Sync + 'static {
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
                .push((path.to_path_buf(), serde_json::from_slice(value).unwrap()));
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
        handle.publish(status, json!({ "hour": 0 })).unwrap();
        entered
            .recv_timeout(TIMEOUT)
            .expect("the first write stalls");
        let (done_tx, done) = mpsc::channel();
        let publisher = handle.clone();
        std::thread::spawn(move || {
            for hour in 1..=1000 {
                publisher.publish(status, json!({ "hour": hour })).unwrap();
                publisher.publish(content, json!({ "hour": hour })).unwrap();
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
        spool
            .publish(Path::new("status.json"), json!({ "hour": 1 }))
            .unwrap();
        entered.recv_timeout(TIMEOUT).expect("the write stalls");
        // Only now, with the write stuck, does the spool side end early.
        let (done_tx, done) = mpsc::channel();
        std::thread::spawn(move || {
            spool
                .publish(Path::new("status.json"), json!({ "state": "failing" }))
                .unwrap();
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
        handle
            .publish(Path::new("status.json"), json!({ "hour": 1 }))
            .unwrap();
        // The retry succeeded while the writer was running; finishing only
        // lets its counters settle before they are read.
        wrote.recv_timeout(TIMEOUT).expect("the retry succeeds");
        writer.finish();
        assert_eq!(*attempts.lock().unwrap(), 3);
        assert_eq!(handle.json()["failures"], 2);
        assert_eq!(handle.json()["writes"], 1);
        let error = handle.take_error().expect("the failure is reported");
        assert!(error == "status write failed", "{error}");
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
            record
                .lock()
                .unwrap()
                .push(serde_json::from_slice(value).unwrap());
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
        handle.publish(path, json!({ "hour": 1 })).unwrap();
        entered
            .recv_timeout(TIMEOUT)
            .expect("the first write is in flight");
        handle.publish(path, json!({ "hour": 2 })).unwrap();
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
        let written = Arc::new(Mutex::new(Vec::<Value>::new()));
        let record = Arc::clone(&written);
        let mut writer = StatusWriter::start(move |_, value| {
            record
                .lock()
                .unwrap()
                .push(serde_json::from_slice(value).unwrap());
            Ok(())
        })
        .unwrap();
        writer
            .handle()
            .publish_measured(
                Path::new("status.json"),
                json!({ "hour": 1 }),
                vec![directory.clone()],
            )
            .unwrap();
        writer.finish();
        let _ = fs::remove_dir_all(&directory);
        assert_eq!(
            *written.lock().unwrap(),
            [json!({ "hour": 1, "spool_bytes": 123 })]
        );
    }
}
