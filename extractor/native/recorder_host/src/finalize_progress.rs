//! Post-capture finalize progress receipt.
//!
//! `CAPTURE_TEARDOWN_V1.json` proves the collector is detached and the target's
//! bytes are restored. It does NOT mean the recorder is finished: everything
//! expensive -- the streaming world fold, the sealed replay container, the
//! finalize telemetry rewrite of `native_session_manifest.json` -- happens
//! AFTER it, on the main thread, and is minutes of work.
//!
//! Measured on the 2026-08-06 live_unlock trials. t30 completed and reported
//! `host_finalize.finalize_seconds = 315.7` for a 341 MB / 129-hour stream.
//! The three runs beside it were killed with their folds provably INCOMPLETE:
//! counting records in each `checkpoints.h4cp` (every record is one folded
//! hour) against the hour count in the same run's manifest gives t27 443/542,
//! t28 149/577, t29 194/459, versus t30's 129/129, and none of the three had
//! reached its `hour_index.h4ix`. What that establishes is that they were
//! killed with real work outstanding, and that t27 was still writing records
//! eight minutes past the harness's verdict; it does NOT establish that any of
//! them was advancing at the exact instant it was killed. The harness bound
//! them with a FIXED 300 s headroom and declared them unresolved.
//!
//! Nothing the recorder published let a supervisor tell "slow" from "wedged",
//! so the only available bound was a wall clock unrelated to the amount of
//! work -- and a wall clock cannot be made correct, only made longer. This
//! receipt supplies a better fact. It carries a WORK quantity, not a liveness
//! tick: `sample_ordinal` advances on every sample and proves only that the
//! publisher thread runs, while `work_unchanged_ms` is how long the measured
//! work quantity has stood still and is the only field a supervisor may bound.
//!
//! What that quantity does and does not prove, stated plainly because the
//! supervisor's bound is only as good as this: it is a byte total over the
//! recorder's own finalize outputs, so it evidences that finalization is
//! PRODUCING something. Advancing bytes are therefore strong evidence of
//! progress, but standing-still bytes are only evidence of standing-still
//! bytes -- a long read-only verify pass legitimately looks the same as a
//! stall. The no-progress window is sized for that, and the receipt narrows
//! the question rather than settling it.
//!
//! A consumer must also treat a receipt that has STOPPED being rewritten as no
//! evidence at all. Publication failures are swallowed here so they cannot fail
//! a good capture, which means the file can freeze mid-run; a frozen sample
//! still reading "work advancing" would otherwise buy time forever.
//!
//! The publisher owns a thread that WRITES EVIDENCE, so it is stopped and
//! joined before the process exits (`finish`, with `Drop` as the backstop on
//! error paths). It is never abandoned to process teardown.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Published beside the run's other durable receipts.
pub const RECEIPT_FILE_NAME: &str = "RECORDER_FINALIZE_PROGRESS_V1.json";

pub const SCHEMA: &str = "battle-plan-recorder-finalize-progress-v1";

/// Phase while the finalize is still running.
pub const PHASE_FINALIZING: &str = "finalizing";

/// Terminal phase. `finish` publishes exactly one sample carrying it.
pub const PHASE_COMPLETE: &str = "complete";

/// How often the publisher samples and republishes. Small enough that a
/// supervisor polling every couple of seconds always reads a fresh sample,
/// large enough that directory metering never competes with the fold.
pub const DEFAULT_SAMPLE_INTERVAL: Duration = Duration::from_millis(2_000);

/// Prefix of the fold's per-hour snapshot spill directory. Declared here, and
/// used by the code that CREATES the directory, so the meter and the producer
/// cannot drift apart -- a renamed spill directory that silently stopped being
/// metered would report a busy fold as a stalled one.
pub const SNAPSHOT_SPILL_DIR_PREFIX: &str = ".native_session_snapshots-";

/// Prefix of the replay decoder's checkpoint-cache/hour-index spill directory.
/// This is where the bulk of finalize's bytes land: on t30 it held 300 MB of
/// `checkpoints.h4cp`, and on t27 it was still growing past 1.35 GB when the
/// harness declared the attempt unresolved.
pub const REPLAY_ARTIFACT_SPILL_DIR_PREFIX: &str = ".replay_decoder_artifacts-";

/// Root of the rollover package-set path. A rollover run does ALL of its
/// member fold, verify and packaging work nested under here, so without it the
/// meter would sit perfectly still through the entire operation however long it
/// took -- the same false "wedged" verdict, just on a different route.
pub const ROLLOVER_LOGICAL_STREAMS_DIR: &str = "logical-streams";

/// Identity carried in every sample so a receipt can never be mistaken for one
/// left behind by an earlier attempt in a reused output directory.
#[derive(Clone, Debug)]
pub struct FinalizeProgressBinding {
    pub pid: u32,
    pub session_id: String,
    pub capture_id: String,
}

/// Source of the monotone work quantity a supervisor bounds.
///
/// Deliberately a trait: the production meter reads the filesystem, and the
/// tests need a meter whose value they control exactly, without sleeping on
/// real I/O.
pub trait WorkMeter: Send + 'static {
    /// Current work quantity. Callers treat only CHANGE as progress, never the
    /// absolute value or its direction -- the fold's snapshot spill directory
    /// is deleted at the end of finalize, so the production quantity legitimately
    /// decreases at least once per run.
    fn observe(&mut self) -> u64;
}

/// Total bytes of every regular file under the watched roots.
///
/// The roots are the recorder's OWN finalize outputs. The run directory as a
/// whole would be wrong: the harness writes its independent passive clock
/// witness into that same directory every few seconds, so a directory-wide
/// meter would report progress forever no matter what the recorder was doing.
pub struct RecorderArtifactByteMeter {
    roots: Vec<PathBuf>,
}

impl RecorderArtifactByteMeter {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self { roots }
    }

    fn total_bytes(path: &Path, depth: u32) -> u64 {
        // Bounded so a symlink/junction cycle under a spill root cannot turn a
        // metering sample into an unbounded walk.
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

/// Production meter: totals the recorder's finalize outputs inside the run
/// directory, resolved on every sample.
///
/// Resolved per sample rather than fixed at construction because the two spill
/// directories carry a per-run nonce and are created AFTER finalize begins --
/// a root list captured at start would be permanently empty. Entries are
/// admitted by an allow-list, never by excluding known harness files: the
/// harness owns this directory too, and its passive clock witness appends
/// every few seconds, so "something in the run directory changed" is true
/// forever and means nothing.
pub struct RunDirectoryMeter {
    out_dir: PathBuf,
}

impl RunDirectoryMeter {
    pub fn new(out_dir: &Path) -> Self {
        Self {
            out_dir: out_dir.to_path_buf(),
        }
    }

    /// Is this a direct child of the run directory that the recorder writes
    /// during finalize?
    ///
    /// Matched with `contains`, not `ends_with`/equality, because the container
    /// and the manifest are both written under a TEMPORARY name and renamed
    /// into place at the very end -- `native_session.battleplan.tmp-<token>-<n>`
    /// and `.native_session_manifest.json.<pid>.<nanos>.tmp`. Anchoring on the
    /// final names would leave the whole container-write phase invisible: the
    /// fold's spill directories have stopped growing (and the snapshot spill is
    /// deleted outright) while the only file being written is one that has not
    /// been renamed yet. On t30 that phase ran 34 s; on a multi-GB stream it is
    /// minutes, and a frozen meter across it is exactly the false "wedged"
    /// verdict this receipt exists to prevent.
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

/// One published sample, as it appears on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalizeProgressSample {
    pub phase: String,
    pub sample_ordinal: u64,
    pub elapsed_ms: u64,
    pub work_bytes: u64,
    /// Sample at which `work_bytes` last differed from its predecessor. Sample
    /// zero always counts as a change, because the first observation has no
    /// predecessor to be unchanged against.
    pub work_changed_ordinal: u64,
    /// Milliseconds since that change. THE bound a supervisor applies.
    pub work_unchanged_ms: u64,
    /// Wall-clock time of THIS sample, captured next to the monotonic reading
    /// that produced `work_unchanged_ms`.
    ///
    /// A supervisor reconstructing when progress last happened must subtract
    /// `work_unchanged_ms` from this, never from the receipt file's modified
    /// time. The two are not the same instant: metering walks directories and
    /// publication serializes, writes, syncs and renames, so the file's stamp
    /// trails the measurement by however long all of that took. Anchoring on
    /// the file would silently push the reconstructed last-progress instant
    /// forward by that latency and hand out time the recorder never earned.
    /// The file's timestamp stays useful for exactly one thing -- proving the
    /// publisher is still alive -- and that is a different question.
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

/// Name of the same-directory temporary that a sample is written to before it
/// is renamed onto `RECEIPT_FILE_NAME`.
///
/// Declared here, beside the code that CREATES it, for the same reason the
/// spill-directory prefixes are: a consumer that has to recognise a publish in
/// flight must not carry its own copy of this spelling and drift from it.
pub fn receipt_temp_name(publisher_pid: u32, ordinal: u64) -> String {
    format!(".{RECEIPT_FILE_NAME}.{publisher_pid}.{ordinal}.tmp")
}

/// Pid of the process publishing `name`, if `name` is one of those temporaries.
///
/// This exists because replacing a file on Windows is NOT atomic: probing the
/// exact write/fsync/rename sequence `publish_sample` uses, a concurrent reader
/// saw `RECEIPT_FILE_NAME` reported ERROR_FILE_NOT_FOUND while a directory
/// listing taken at the same instant held only this temporary -- the old name
/// unlinked, the new one not yet in place. Measured windows were microseconds
/// most of the time and 7-16 ms when a filesystem filter got involved.
///
/// So "the receipt is absent" is not the same fact as "no finalizer published
/// one", and a consumer that must not race a live finalizer needs evidence that
/// tells the two apart. The temporary is that evidence, and its exact strength
/// is worth stating: it is normally created and consumed within one publish, so
/// finding one says a publish is in flight -- but it also survives a finalizer
/// killed between the write and the rename, and the pid it carries can have
/// been reissued. It is therefore evidence a publisher EXISTS OR RECENTLY DIED
/// here, which is why the only sound thing to do with it is refuse.
pub fn publisher_pid_of_receipt_temp(name: &str) -> Option<u32> {
    let rest = name.strip_prefix(&format!(".{RECEIPT_FILE_NAME}."))?;
    let rest = rest.strip_suffix(".tmp")?;
    let (pid, ordinal) = rest.split_once('.')?;
    // Both fields must parse, so an unrelated file that merely looks similar is
    // not read as a publish in flight.
    ordinal.parse::<u64>().ok()?;
    pid.parse::<u32>().ok()
}

/// Read the currently published sample, if any.
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

/// Republish over the previous sample. Unlike the once-only durable receipts,
/// this file is BY DESIGN rewritten many times, so `create_new` on the final
/// name is wrong; the same-directory temporary plus rename is what keeps every
/// observation a complete JSON document.
fn publish_sample(
    out_dir: &Path,
    binding: &FinalizeProgressBinding,
    sample: &FinalizeProgressSample,
    temp_ordinal: u64,
) -> Result<(), String> {
    let value = serde_json::json!({
        "schema": SCHEMA,
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

/// Tracks change in the measured quantity and turns it into the two published
/// staleness fields. Split out from the thread so the state machine is testable
/// without any thread, clock, or filesystem at all.
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
        // The first observation has no predecessor, so it is a change by
        // definition -- treating it as "unchanged since start" would publish a
        // staleness that had never been measured.
        //
        // A change is stamped at the instant it was OBSERVED, not at the
        // earliest instant it could have happened. All this ever knows is that
        // the quantity moved somewhere inside the gap since the previous
        // observation, and the two ends of that gap are not equally safe. This
        // field exists because four live trials were killed on a fixed timer
        // while their folds were still writing bytes, so the failure that
        // matters is calling a working recorder wedged. Stamping the near end
        // can only under-report a stall -- more patience, never a false kill --
        // and by at most one OBSERVATION GAP, which is the sampling interval
        // plus however long metering and publication took, not the interval
        // alone. Stamping the far end would report a starved PUBLISHER as
        // a stalled FINALIZE and spend the supervisor's budget on evidence of
        // progress, which is the original sin. The residual is real and stated
        // plainly: work that moved early in a long publisher gap and then
        // wedged buys one extra no-progress window.
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

/// Owns the sampling thread. Stopped and JOINED by `finish`; `Drop` is the
/// backstop for the `?` paths through finalize.
pub struct FinalizeProgressPublisher {
    out_dir: PathBuf,
    binding: FinalizeProgressBinding,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    started: Instant,
    /// Handed back by the thread so `finish` can publish the terminal sample
    /// with a continuous ordinal instead of restarting the count.
    ordinal: Arc<std::sync::atomic::AtomicU64>,
    /// Test-only ordering hook; see `on_before_stop_and_join`.
    #[cfg(test)]
    before_stop_and_join: Option<Box<dyn FnMut()>>,
}

impl FinalizeProgressPublisher {
    /// Begin publishing. Fails only if the very first sample cannot be written:
    /// a receipt whose absence is ambiguous is worse than none at all, so the
    /// caller learns immediately rather than at the first supervisor poll.
    pub fn start(
        out_dir: &Path,
        binding: FinalizeProgressBinding,
        meter: Box<dyn WorkMeter>,
        interval: Duration,
    ) -> Result<Self, String> {
        let started = Instant::now();
        let mut tracker = ProgressTracker::new(started);
        let mut meter = meter;
        // Meter FIRST, then read the clocks. Reversing these stamps the sample
        // with a time from before a directory walk that can take a while on a
        // large run, which biases the reconstructed last-progress instant
        // forward by exactly that walk.
        let first_work = meter.observe();
        let first = tracker.sample(
            PHASE_FINALIZING,
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
                // Poll the stop flag on a short tick rather than sleeping the
                // whole interval: `finish` must not have to wait out a full
                // sampling period before the process can exit.
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
                    // Same ordering rule as the first sample: measure, then
                    // timestamp.
                    let work = meter.observe();
                    let sample = tracker.sample(
                        PHASE_FINALIZING,
                        next,
                        Instant::now(),
                        SystemTime::now(),
                        started,
                        work,
                    );
                    // A metering or publish failure must never take down a
                    // capture that is otherwise succeeding. The receipt then
                    // stops advancing, and a supervisor MUST treat a receipt
                    // that has stopped advancing as no evidence -- a frozen
                    // sample carrying a low `work_unchanged_ms` would otherwise
                    // keep buying time forever.
                    let _ = publish_sample(&thread_dir, &thread_binding, &sample, next);
                }
            }) {
            Ok(handle) => handle,
            Err(error) => {
                // The first sample is already on disk. Leaving it there after
                // the thread failed to start would publish a receipt that can
                // never change while still looking valid -- exactly the frozen
                // receipt described above. Withdraw it so absence, which every
                // supervisor already treats as "no evidence", is what remains.
                //
                // BEST-EFFORT: on Windows the delete can fail transiently while
                // another process holds the file open without delete sharing.
                // A receipt that survives is bounded by the consumer's
                // freshness rule -- it stops being rewritten, so it stops
                // counting -- which is why this does not need to be reliable.
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

    /// Install a hook fired ONCE, after the sampling thread has been told to
    /// stop and before it is joined.
    ///
    /// It exists so "the join happens BEFORE the terminal publish" is a fact a
    /// test can force rather than a race it hopes to lose. A test parks the
    /// sampling thread inside its own `WorkMeter` and, from here, lets exactly
    /// one more sample through. Because the stop flag is ALREADY set when the
    /// hook runs, that sample is the thread's last: it publishes, sees the
    /// stop, and exits. The test can therefore wait for the thread to die --
    /// not for a duration -- and know the sample is on disk.
    ///
    /// The ordering that buys: at the moment the hook returns, a `finalizing`
    /// sample newer than anything `finish` has written is on disk. An
    /// implementation that published `complete` before this point has just had
    /// its seal overwritten, deterministically; one that publishes after the
    /// join cannot be overwritten, because the only thread that could is gone.
    #[cfg(test)]
    pub(crate) fn on_before_stop_and_join(&mut self, hook: impl FnMut() + 'static) {
        self.before_stop_and_join = Some(Box::new(hook));
    }

    /// Stop, join, and publish the terminal sample.
    ///
    /// The join happens BEFORE the terminal publish so the sampling thread can
    /// never overwrite it: `complete` is the last thing written, and a
    /// supervisor that reads it knows no further sample is coming.
    pub fn finish(mut self) -> Result<(), String> {
        let joined = self.stop_and_join();
        let ordinal = self.ordinal.load(Ordering::Acquire) + 1;
        let elapsed = self.started.elapsed().as_millis() as u64;
        let sample = FinalizeProgressSample {
            phase: PHASE_COMPLETE.to_string(),
            sample_ordinal: ordinal,
            elapsed_ms: elapsed,
            // The terminal sample states completion, not a fresh measurement:
            // re-metering here would walk directories the finalize has already
            // torn down and publish a drop that reads like lost work.
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
        // Test-only; see `on_before_stop_and_join`. Taken rather than borrowed
        // so a later `Drop` cannot fire it a second time.
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
        // The thread writes evidence into the run directory, so it is joined
        // even on the error paths -- never left to be killed by process exit
        // partway through a rename.
        let _ = self.stop_and_join();
    }
}

#[cfg(test)]
mod tests;
