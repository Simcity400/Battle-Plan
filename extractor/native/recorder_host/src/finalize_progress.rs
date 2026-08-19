use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const RECEIPT_FILE_NAME: &str = "RECORDER_FINALIZE_PROGRESS_V1.json";

pub const SCHEMA: &str = "battle-plan-recorder-finalize-progress-v1";

pub const PHASE_FINALIZING: &str = "finalizing";

pub const PHASE_COMPLETE: &str = "complete";

pub const DEFAULT_SAMPLE_INTERVAL: Duration = Duration::from_millis(2_000);

pub const SNAPSHOT_SPILL_DIR_PREFIX: &str = ".native_session_snapshots-";

pub const REPLAY_ARTIFACT_SPILL_DIR_PREFIX: &str = ".replay_decoder_artifacts-";

pub const ROLLOVER_LOGICAL_STREAMS_DIR: &str = "logical-streams";

#[derive(Clone, Debug)]
pub struct FinalizeProgressBinding {
    pub pid: u32,
    pub session_id: String,
    pub capture_id: String,
}

pub trait WorkMeter: Send + 'static {
    fn observe(&mut self) -> u64;
}

pub struct RecorderArtifactByteMeter {
    roots: Vec<PathBuf>,
}

impl RecorderArtifactByteMeter {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self { roots }
    }

    fn total_bytes(path: &Path, depth: u32) -> u64 {
        const MAX_DEPTH: u32 = 8;
        let Ok(metadata) = fs::symlink_metadata(path) else {
            return 0;
        };
        if metadata.is_file() {
            return metadata.len();
        }
        if !metadata.is_dir() || depth >= MAX_DEPTH {
            return 0;
        }
        let Ok(entries) = fs::read_dir(path) else {
            return 0;
        };
        entries
            .flatten()
            .map(|entry| Self::total_bytes(&entry.path(), depth + 1))
            .fold(0u64, |total, bytes| total.saturating_add(bytes))
    }
}

impl WorkMeter for RecorderArtifactByteMeter {
    fn observe(&mut self) -> u64 {
        self.roots
            .iter()
            .map(|root| Self::total_bytes(root, 0))
            .fold(0u64, |total, bytes| total.saturating_add(bytes))
    }
}

pub struct RunDirectoryMeter {
    out_dir: PathBuf,
}

impl RunDirectoryMeter {
    pub fn new(out_dir: &Path) -> Self {
        Self {
            out_dir: out_dir.to_path_buf(),
        }
    }

    fn is_recorder_finalize_output(name: &str) -> bool {
        name.starts_with(SNAPSHOT_SPILL_DIR_PREFIX)
            || name.starts_with(REPLAY_ARTIFACT_SPILL_DIR_PREFIX)
            || name == ROLLOVER_LOGICAL_STREAMS_DIR
            || name.contains(".battleplan")
            || name.contains("native_session_manifest.json")
    }
}

impl WorkMeter for RunDirectoryMeter {
    fn observe(&mut self) -> u64 {
        let Ok(entries) = fs::read_dir(&self.out_dir) else {
            return 0;
        };
        entries
            .flatten()
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(Self::is_recorder_finalize_output)
            })
            .map(|entry| RecorderArtifactByteMeter::total_bytes(&entry.path(), 0))
            .fold(0u64, |total, bytes| total.saturating_add(bytes))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalizeProgressSample {
    pub phase: String,
    pub sample_ordinal: u64,
    pub elapsed_ms: u64,
    pub work_bytes: u64,
    pub work_changed_ordinal: u64,
    pub work_unchanged_ms: u64,
    pub sample_unix_ms: u64,
}

impl FinalizeProgressSample {
    pub fn from_json(value: &serde_json::Value) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or("finalize progress receipt is not a JSON object")?;
        let schema = object
            .get("schema")
            .and_then(|value| value.as_str())
            .ok_or("finalize progress receipt has no schema")?;
        if schema != SCHEMA {
            return Err(format!(
                "unsupported finalize progress schema {schema:?}, expected {SCHEMA:?}"
            ));
        }
        let number = |key: &str| -> Result<u64, String> {
            object
                .get(key)
                .and_then(|value| value.as_u64())
                .ok_or_else(|| format!("finalize progress receipt has no unsigned {key}"))
        };
        Ok(Self {
            phase: object
                .get("phase")
                .and_then(|value| value.as_str())
                .ok_or("finalize progress receipt has no phase")?
                .to_string(),
            sample_ordinal: number("sample_ordinal")?,
            elapsed_ms: number("elapsed_ms")?,
            work_bytes: number("work_bytes")?,
            work_changed_ordinal: number("work_changed_ordinal")?,
            work_unchanged_ms: number("work_unchanged_ms")?,
            sample_unix_ms: number("sample_unix_ms")?,
        })
    }
}

pub fn receipt_temp_name(publisher_pid: u32, ordinal: u64) -> String {
    format!(".{RECEIPT_FILE_NAME}.{publisher_pid}.{ordinal}.tmp")
}

pub fn publisher_pid_of_receipt_temp(name: &str) -> Option<u32> {
    let rest = name.strip_prefix(&format!(".{RECEIPT_FILE_NAME}."))?;
    let rest = rest.strip_suffix(".tmp")?;
    let (pid, ordinal) = rest.split_once('.')?;
    ordinal.parse::<u64>().ok()?;
    pid.parse::<u32>().ok()
}

pub fn read_published_sample(out_dir: &Path) -> Result<Option<FinalizeProgressSample>, String> {
    let path = out_dir.join(RECEIPT_FILE_NAME);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("read {} failed: {error}", path.display())),
    };
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse {} failed: {error}", path.display()))?;
    FinalizeProgressSample::from_json(&value).map(Some)
}

fn publish_sample(
    out_dir: &Path,
    binding: &FinalizeProgressBinding,
    sample: &FinalizeProgressSample,
    temp_ordinal: u64,
) -> Result<(), String> {
    let value = serde_json::json!({
        "schema": SCHEMA,
        // TARGET GAME, which is identity, not ownership: a recovery asking
        "publisher_pid": std::process::id(),
        "publisher_creation_filetime": crate::modset::query_process_creation_filetime(
            std::process::id(),
        )
        .ok(),
        "pid": binding.pid,
        "session_id": binding.session_id,
        "capture_id": binding.capture_id,
        "phase": sample.phase,
        "sample_ordinal": sample.sample_ordinal,
        "elapsed_ms": sample.elapsed_ms,
        "work_bytes": sample.work_bytes,
        "work_changed_ordinal": sample.work_changed_ordinal,
        "work_unchanged_ms": sample.work_unchanged_ms,
        "sample_unix_ms": sample.sample_unix_ms,
    });
    let mut bytes = serde_json::to_vec_pretty(&value)
        .map_err(|error| format!("serialize {RECEIPT_FILE_NAME} failed: {error}"))?;
    bytes.push(b'\n');
    let temp = out_dir.join(receipt_temp_name(std::process::id(), temp_ordinal));
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temp)
        .map_err(|error| format!("create {} failed: {error}", temp.display()))?;
    if let Err(error) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(&temp);
        return Err(format!("durably write {} failed: {error}", temp.display()));
    }
    drop(file);
    if let Err(error) = fs::rename(&temp, out_dir.join(RECEIPT_FILE_NAME)) {
        let _ = fs::remove_file(&temp);
        return Err(format!(
            "atomically publish {RECEIPT_FILE_NAME} failed: {error}"
        ));
    }
    Ok(())
}

#[derive(Debug)]
struct ProgressTracker {
    last_work: Option<u64>,
    changed_ordinal: u64,
    changed_at: Instant,
}

impl ProgressTracker {
    fn new(started: Instant) -> Self {
        Self {
            last_work: None,
            changed_ordinal: 0,
            changed_at: started,
        }
    }

    fn sample(
        &mut self,
        phase: &str,
        ordinal: u64,
        now: Instant,
        wall: SystemTime,
        started: Instant,
        work: u64,
    ) -> FinalizeProgressSample {
        if self.last_work != Some(work) {
            self.last_work = Some(work);
            self.changed_ordinal = ordinal;
            self.changed_at = now;
        }
        FinalizeProgressSample {
            phase: phase.to_string(),
            sample_ordinal: ordinal,
            elapsed_ms: now.saturating_duration_since(started).as_millis() as u64,
            work_bytes: work,
            work_changed_ordinal: self.changed_ordinal,
            work_unchanged_ms: now.saturating_duration_since(self.changed_at).as_millis() as u64,
            sample_unix_ms: wall
                .duration_since(UNIX_EPOCH)
                .map(|since| since.as_millis().min(u128::from(u64::MAX)) as u64)
                .unwrap_or_default(),
        }
    }
}

pub struct FinalizeProgressPublisher {
    out_dir: PathBuf,
    binding: FinalizeProgressBinding,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    started: Instant,
    ordinal: Arc<std::sync::atomic::AtomicU64>,
    #[cfg(test)]
    before_stop_and_join: Option<Box<dyn FnMut()>>,
}

impl FinalizeProgressPublisher {
    pub fn start(
        out_dir: &Path,
        binding: FinalizeProgressBinding,
        meter: Box<dyn WorkMeter>,
        interval: Duration,
        phase: &'static str,
    ) -> Result<Self, String> {
        let started = Instant::now();
        let mut tracker = ProgressTracker::new(started);
        let mut meter = meter;
        let first_work = meter.observe();
        let first = tracker.sample(
            phase,
            0,
            Instant::now(),
            SystemTime::now(),
            started,
            first_work,
        );
        publish_sample(out_dir, &binding, &first, 0)?;

        let stop = Arc::new(AtomicBool::new(false));
        let ordinal = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let thread_stop = Arc::clone(&stop);
        let thread_ordinal = Arc::clone(&ordinal);
        let thread_dir = out_dir.to_path_buf();
        let thread_binding = binding.clone();
        let handle = match std::thread::Builder::new()
            .name("recorder-finalize-progress".to_string())
            .spawn(move || {
                const TICK: Duration = Duration::from_millis(50);
                let mut next_sample = Instant::now() + interval;
                while !thread_stop.load(Ordering::Acquire) {
                    let now = Instant::now();
                    if now < next_sample {
                        std::thread::sleep(TICK.min(next_sample.saturating_duration_since(now)));
                        continue;
                    }
                    next_sample = now + interval;
                    let next = thread_ordinal.fetch_add(1, Ordering::AcqRel) + 1;
                    let work = meter.observe();
                    let sample = tracker.sample(
                        phase,
                        next,
                        Instant::now(),
                        SystemTime::now(),
                        started,
                        work,
                    );
                    let _ = publish_sample(&thread_dir, &thread_binding, &sample, next);
                }
            }) {
            Ok(handle) => handle,
            Err(error) => {
                let _ = fs::remove_file(out_dir.join(RECEIPT_FILE_NAME));
                return Err(format!("spawn finalize progress publisher failed: {error}"));
            }
        };

        Ok(Self {
            out_dir: out_dir.to_path_buf(),
            binding,
            stop,
            handle: Some(handle),
            started,
            ordinal,
            #[cfg(test)]
            before_stop_and_join: None,
        })
    }

    #[cfg(test)]
    pub(crate) fn on_before_stop_and_join(&mut self, hook: impl FnMut() + 'static) {
        self.before_stop_and_join = Some(Box::new(hook));
    }

    pub fn finish(mut self) -> Result<(), String> {
        let joined = self.stop_and_join();
        let ordinal = self.ordinal.load(Ordering::Acquire) + 1;
        let elapsed = self.started.elapsed().as_millis() as u64;
        let sample = FinalizeProgressSample {
            phase: PHASE_COMPLETE.to_string(),
            sample_ordinal: ordinal,
            elapsed_ms: elapsed,
            work_bytes: 0,
            work_changed_ordinal: ordinal,
            work_unchanged_ms: 0,
            sample_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|since| since.as_millis().min(u128::from(u64::MAX)) as u64)
                .unwrap_or_default(),
        };
        publish_sample(&self.out_dir, &self.binding, &sample, ordinal)?;
        joined
    }

    fn stop_and_join(&mut self) -> Result<(), String> {
        self.stop.store(true, Ordering::Release);
        #[cfg(test)]
        if let Some(mut hook) = self.before_stop_and_join.take() {
            hook();
        }
        match self.handle.take() {
            Some(handle) => handle
                .join()
                .map_err(|_| "finalize progress publisher thread panicked".to_string()),
            None => Ok(()),
        }
    }
}

impl Drop for FinalizeProgressPublisher {
    fn drop(&mut self) {
        let _ = self.stop_and_join();
    }
}

#[cfg(test)]
mod tests;
