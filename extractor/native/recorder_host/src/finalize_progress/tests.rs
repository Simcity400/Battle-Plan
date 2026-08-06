use super::*;
use std::sync::atomic::AtomicU64;
use std::time::{SystemTime, UNIX_EPOCH};

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "recorder_finalize_progress_{label}_{}_{}",
            std::process::id(),
            nonce
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Meter the test drives directly, so "work advanced" and "work stalled" are
/// facts the test sets rather than filesystem timing it has to hope for.
struct ScriptedMeter(Arc<AtomicU64>);

impl WorkMeter for ScriptedMeter {
    fn observe(&mut self) -> u64 {
        self.0.load(Ordering::Acquire)
    }
}

fn binding() -> FinalizeProgressBinding {
    FinalizeProgressBinding {
        pid: 4321,
        session_id: "11111111-2222-3333-4444-555555555555".to_string(),
        capture_id: "headless-raw-capture-test".to_string(),
    }
}

const TEST_INTERVAL: Duration = Duration::from_millis(20);

/// THE regression test for #116.
///
/// A recorder finalizing a multi-GB stream and a recorder spinning on an
/// unsatisfiable condition are indistinguishable from outside unless something
/// published tells them apart -- which is why four live trials on 2026-08-06
/// were declared unresolved on a fixed 300 s budget while their folds were
/// still writing checkpoint bytes.
///
/// The property: `sample_ordinal` advances in BOTH cases (it is liveness only,
/// and must never be used as a progress signal), while `work_unchanged_ms`
/// stays near zero exactly while work advances and grows without bound once it
/// stops. Reverting the change-tracking in `ProgressTracker::sample` -- for
/// instance by stamping `changed_at` on every sample rather than only on a
/// change -- pins `work_unchanged_ms` at ~0 forever and fails the stalled half
/// of this test, which is precisely the fixed-timer behaviour being replaced.
#[test]
fn work_staleness_separates_an_advancing_finalize_from_a_stalled_one() {
    let dir = TempDir::new("staleness");
    let work = Arc::new(AtomicU64::new(0));
    let publisher = FinalizeProgressPublisher::start(
        dir.path(),
        binding(),
        Box::new(ScriptedMeter(Arc::clone(&work))),
        TEST_INTERVAL,
    )
    .unwrap();

    // Phase A: work advances. Collect what the supervisor would have read.
    let mut advancing = Vec::new();
    for step in 1..=25u64 {
        work.store(step * 4_096, Ordering::Release);
        std::thread::sleep(Duration::from_millis(20));
        if let Some(sample) = read_published_sample(dir.path()).unwrap() {
            advancing.push(sample);
        }
    }
    let fresh = advancing
        .iter()
        .filter(|sample| sample.sample_ordinal > 0 && sample.work_unchanged_ms <= 60)
        .count();
    assert!(
        fresh > 0,
        "an advancing finalize published no sample with fresh work; saw {advancing:?}"
    );
    let advancing_ordinal = advancing
        .last()
        .expect("advancing phase published no sample")
        .sample_ordinal;

    // Phase B: work stops. The publisher keeps sampling; staleness must grow.
    std::thread::sleep(Duration::from_millis(400));
    let stalled = read_published_sample(dir.path()).unwrap().unwrap();
    assert!(
        stalled.sample_ordinal > advancing_ordinal,
        "publisher stopped sampling during the stall: {} !> {}",
        stalled.sample_ordinal,
        advancing_ordinal
    );
    assert_eq!(
        stalled.work_bytes,
        25 * 4_096,
        "stalled sample reported a work quantity nobody wrote"
    );
    assert!(
        stalled.work_unchanged_ms >= 300,
        "a stalled finalize reported only {} ms of unchanged work; \
         a supervisor bounding this field could never call it wedged",
        stalled.work_unchanged_ms
    );
    assert_eq!(stalled.phase, PHASE_FINALIZING);

    publisher.finish().unwrap();
}

/// Complement of the test above: resumed work must clear the staleness, so the
/// field bounds the CURRENT stall rather than accumulating over the run.
#[test]
fn resumed_work_clears_the_staleness_a_supervisor_bounds() {
    let dir = TempDir::new("resume");
    let work = Arc::new(AtomicU64::new(7));
    let publisher = FinalizeProgressPublisher::start(
        dir.path(),
        binding(),
        Box::new(ScriptedMeter(Arc::clone(&work))),
        TEST_INTERVAL,
    )
    .unwrap();

    std::thread::sleep(Duration::from_millis(300));
    let stalled = read_published_sample(dir.path()).unwrap().unwrap();
    assert!(
        stalled.work_unchanged_ms >= 200,
        "expected an accumulated stall, saw {} ms",
        stalled.work_unchanged_ms
    );

    work.store(9_001, Ordering::Release);
    std::thread::sleep(Duration::from_millis(150));
    let resumed = read_published_sample(dir.path()).unwrap().unwrap();
    assert_eq!(resumed.work_bytes, 9_001);
    assert!(
        resumed.work_unchanged_ms < stalled.work_unchanged_ms,
        "staleness did not reset when work resumed: {} !< {}",
        resumed.work_unchanged_ms,
        stalled.work_unchanged_ms
    );
    assert!(
        resumed.work_changed_ordinal > stalled.work_changed_ordinal,
        "resumed work did not move work_changed_ordinal"
    );

    publisher.finish().unwrap();
}

/// The publisher owns a thread that writes into the run directory. `finish`
/// must join it BEFORE publishing the terminal sample, or a late sample could
/// overwrite `complete` and a supervisor would wait for a recorder that had
/// already finished.
#[test]
fn finish_joins_the_publisher_before_sealing_the_receipt() {
    let dir = TempDir::new("finish");
    let work = Arc::new(AtomicU64::new(0));
    let publisher = FinalizeProgressPublisher::start(
        dir.path(),
        binding(),
        Box::new(ScriptedMeter(Arc::clone(&work))),
        TEST_INTERVAL,
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(80));
    publisher.finish().unwrap();

    let sealed = read_published_sample(dir.path()).unwrap().unwrap();
    assert_eq!(sealed.phase, PHASE_COMPLETE);

    // Nothing may republish after the seal.
    work.store(1_000_000, Ordering::Release);
    std::thread::sleep(Duration::from_millis(120));
    let after = read_published_sample(dir.path()).unwrap().unwrap();
    assert_eq!(
        after, sealed,
        "a sample landed after the terminal receipt was published"
    );
}

/// Dropping on a `?` path must also stop and join, never abandon the thread to
/// process exit partway through a rename.
#[test]
fn drop_stops_and_joins_the_publisher() {
    let dir = TempDir::new("drop");
    let work = Arc::new(AtomicU64::new(0));
    {
        let _publisher = FinalizeProgressPublisher::start(
            dir.path(),
            binding(),
            Box::new(ScriptedMeter(Arc::clone(&work))),
            TEST_INTERVAL,
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(80));
    }
    let last = read_published_sample(dir.path()).unwrap().unwrap();
    work.store(555_555, Ordering::Release);
    std::thread::sleep(Duration::from_millis(120));
    let after = read_published_sample(dir.path()).unwrap().unwrap();
    assert_eq!(
        after, last,
        "the publisher thread outlived the dropped publisher"
    );
}

/// Every observation is a complete document: the receipt is rewritten many
/// times, so a supervisor polling it must never read a half-written file.
#[test]
fn every_observation_of_the_receipt_parses() {
    let dir = TempDir::new("atomic");
    let work = Arc::new(AtomicU64::new(0));
    let publisher = FinalizeProgressPublisher::start(
        dir.path(),
        binding(),
        Box::new(ScriptedMeter(Arc::clone(&work))),
        Duration::from_millis(5),
    )
    .unwrap();
    for step in 0..300u64 {
        work.store(step, Ordering::Release);
        // Any torn read surfaces as a parse error from `read_published_sample`.
        read_published_sample(dir.path()).unwrap().unwrap();
    }
    publisher.finish().unwrap();
}

/// The production meter must total ONLY the recorder's own artifacts. The
/// harness writes its independent passive clock witness into the same run
/// directory every few seconds, so a directory-wide meter would report
/// progress forever regardless of what the recorder was doing -- the exact
/// failure mode that makes a naive "did anything in the run dir change"
/// watchdog useless here.
#[test]
fn artifact_meter_ignores_files_outside_the_watched_roots() {
    let dir = TempDir::new("roots");
    let watched = dir.path().join("artifacts");
    fs::create_dir_all(watched.join("nested")).unwrap();
    fs::write(watched.join("checkpoints.h4cp"), vec![0u8; 512]).unwrap();
    fs::write(watched.join("nested").join("hour_index.h4ix"), vec![0u8; 8]).unwrap();
    let replay = dir.path().join("native_session.battleplan");
    fs::write(&replay, vec![0u8; 100]).unwrap();

    let mut meter = RecorderArtifactByteMeter::new(vec![watched.clone(), replay.clone()]);
    assert_eq!(meter.observe(), 620);

    // The harness's own witness stream lands in the run directory, NOT under a
    // watched root, and must not move the meter.
    fs::write(dir.path().join("passive_speed5_witness.jsonl"), vec![0u8; 4_096]).unwrap();
    assert_eq!(
        meter.observe(),
        620,
        "the meter counted a file the recorder does not own"
    );

    // A missing root is zero, not an error: the fold's spill directory is
    // removed at the end of finalize.
    fs::remove_dir_all(&watched).unwrap();
    assert_eq!(meter.observe(), 100);
}

/// The run-directory meter must see the finalize phases as they are ACTUALLY
/// written: the container and the manifest live under temporary names for the
/// whole time they are being produced and are renamed into place only at the
/// end. A meter anchored on the final names goes flat across the entire
/// container write -- the fold's spill directories have stopped growing by then
/// -- and the harness would call a working recorder wedged.
#[test]
fn run_directory_meter_tracks_the_temporary_names_finalize_actually_writes() {
    let dir = TempDir::new("rundir");
    let mut meter = RunDirectoryMeter::new(dir.path());
    assert_eq!(meter.observe(), 0);

    // Fold phase: both spill directories, each carrying a per-run nonce.
    let spill = dir
        .path()
        .join(format!("{SNAPSHOT_SPILL_DIR_PREFIX}abc123-0"));
    let artifacts = dir
        .path()
        .join(format!("{REPLAY_ARTIFACT_SPILL_DIR_PREFIX}abc123-0"));
    fs::create_dir_all(spill.join("hours")).unwrap();
    fs::create_dir_all(&artifacts).unwrap();
    fs::write(spill.join("hours").join("0001.json"), vec![0u8; 300]).unwrap();
    fs::write(artifacts.join("checkpoints.h4cp"), vec![0u8; 700]).unwrap();
    assert_eq!(meter.observe(), 1_000);

    // Harness-owned files in the same directory must never move the meter.
    fs::write(dir.path().join("passive_speed5_witness.jsonl"), vec![0u8; 9_999]).unwrap();
    fs::write(dir.path().join("collector_record.stderr.log"), vec![0u8; 512]).unwrap();
    assert_eq!(
        meter.observe(),
        1_000,
        "the meter counted harness-owned files in the run directory"
    );

    // Container phase: the snapshot spill is removed and the container exists
    // only under its temporary name. The meter must still move.
    fs::remove_dir_all(&spill).unwrap();
    let container_temp = dir.path().join("native_session.battleplan.tmp-abc123-0");
    fs::write(&container_temp, vec![0u8; 5_000]).unwrap();
    let with_temp = meter.observe();
    assert_eq!(
        with_temp, 5_700,
        "the container's temporary name was invisible to the meter"
    );

    // Promotion to the final name is a rename, not new work.
    fs::rename(&container_temp, dir.path().join("native_session.battleplan")).unwrap();
    assert_eq!(meter.observe(), with_temp);

    // The manifest's temporary name is metered too.
    fs::write(
        dir.path().join(".native_session_manifest.json.1234.99.tmp"),
        vec![0u8; 42],
    )
    .unwrap();
    assert_eq!(meter.observe(), with_temp + 42);
}

/// A rollover run performs ALL of its member fold, verify and packaging work
/// nested under `logical-streams/`. Without that root the meter stands
/// perfectly still for the whole operation however long it takes, which is the
/// same false "wedged" verdict on a different route.
#[test]
fn run_directory_meter_covers_nested_rollover_member_work() {
    let dir = TempDir::new("rollover");
    let mut meter = RunDirectoryMeter::new(dir.path());
    let member = dir
        .path()
        .join(ROLLOVER_LOGICAL_STREAMS_DIR)
        .join("packages")
        .join("member-0");
    fs::create_dir_all(&member).unwrap();
    assert_eq!(meter.observe(), 0);
    fs::write(member.join("native_session.battleplan.tmp-x-0"), vec![0u8; 2_048]).unwrap();
    assert_eq!(
        meter.observe(),
        2_048,
        "nested rollover member work was invisible to the meter"
    );
}

/// The meter must never count the progress receipt or its temporary. The
/// receipt is rewritten on EVERY sample, so metering it would make the work
/// quantity change every time by construction -- `work_unchanged_ms` would sit
/// at zero forever and the bound it feeds would be unfalsifiable, which is a
/// worse failure than the fixed timer it replaces.
#[test]
fn the_meter_does_not_count_its_own_receipt() {
    let dir = TempDir::new("selfref");
    let mut meter = RunDirectoryMeter::new(dir.path());
    let artifacts = dir
        .path()
        .join(format!("{REPLAY_ARTIFACT_SPILL_DIR_PREFIX}nonce-0"));
    fs::create_dir_all(&artifacts).unwrap();
    fs::write(artifacts.join("checkpoints.h4cp"), vec![0u8; 64]).unwrap();
    assert_eq!(meter.observe(), 64);

    fs::write(dir.path().join(RECEIPT_FILE_NAME), vec![0u8; 4_096]).unwrap();
    fs::write(
        dir.path().join(format!(".{RECEIPT_FILE_NAME}.1234.7.tmp")),
        vec![0u8; 4_096],
    )
    .unwrap();
    assert_eq!(
        meter.observe(),
        64,
        "the meter counted its own receipt, making progress self-fulfilling"
    );
}

/// The first observation has no predecessor, so it is a change by definition.
/// Publishing it as "unchanged since start" would report a staleness that had
/// never been measured, and on a slow first sample that is a false wedge.
#[test]
fn the_first_observation_counts_as_a_change() {
    let started = Instant::now();
    let mut tracker = ProgressTracker::new(started);
    std::thread::sleep(Duration::from_millis(60));
    let now = Instant::now();
    let sample = tracker.sample(PHASE_FINALIZING, 0, now, SystemTime::now(), started, 0);
    assert_eq!(sample.work_changed_ordinal, 0);
    assert!(
        sample.sample_unix_ms > 1_700_000_000_000,
        "the sample carried no usable wall-clock anchor"
    );
    assert_eq!(
        sample.work_unchanged_ms, 0,
        "the first sample reported staleness against a predecessor it never had"
    );
    assert!(sample.elapsed_ms >= 50, "elapsed was not measured from start");
}

/// A receipt from a different schema is refused rather than silently read as
/// zero progress, which would read as an instantly-wedged recorder.
#[test]
fn a_foreign_schema_is_refused_not_read_as_zero_progress() {
    let dir = TempDir::new("schema");
    fs::write(
        dir.path().join(RECEIPT_FILE_NAME),
        br#"{"schema":"battle-plan-recorder-finalize-progress-v0","phase":"finalizing"}"#,
    )
    .unwrap();
    let error = read_published_sample(dir.path()).unwrap_err();
    assert!(error.contains("unsupported finalize progress schema"), "{error}");
}

#[test]
fn no_receipt_is_absence_not_an_error() {
    let dir = TempDir::new("absent");
    assert!(read_published_sample(dir.path()).unwrap().is_none());
}
