use super::*;
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

fn binding() -> FinalizeProgressBinding {
    FinalizeProgressBinding {
        pid: 4321,
        session_id: "11111111-2222-3333-4444-555555555555".to_string(),
        capture_id: "headless-raw-capture-test".to_string(),
    }
}

struct SteppedMeter {
    announce: std::sync::mpsc::Sender<()>,
    answers: std::sync::mpsc::Receiver<Option<u64>>,
    last: u64,
    released: bool,
}

impl WorkMeter for SteppedMeter {
    fn observe(&mut self) -> u64 {
        if self.released {
            return self.last;
        }
        // the recv below then releases us too.
        let _ = self.announce.send(());
        match self.answers.recv() {
            Ok(Some(work)) => {
                self.last = work;
                work
            }
            // `release`, or a dropped control: stop parking so `finish`/`Drop`
            Ok(None) | Err(_) => {
                self.released = true;
                self.last
            }
        }
    }
}

struct MeterControl {
    observed: std::sync::mpsc::Receiver<()>,
    answers: std::sync::mpsc::Sender<Option<u64>>,
}

impl MeterControl {
    fn started(&self) {
        self.observed.recv().expect("start took no observation");
        self.observed
            .recv()
            .expect("the sampling thread took no observation");
    }

    fn step(&self, work: u64) {
        self.answers.send(Some(work)).expect("publisher is gone");
        self.observed.recv().expect("publisher stopped sampling");
    }

    fn step_final(&self, work: u64) {
        self.answers.send(Some(work)).expect("publisher is gone");
        if self.observed.recv().is_ok() {
            self.release();
            panic!("the sampling thread took another observation after it was stopped");
        }
    }

    fn release(&self) {
        let _ = self.answers.send(None);
    }
}

fn stepped_meter(first: u64) -> (SteppedMeter, MeterControl) {
    let (announce, observed) = std::sync::mpsc::channel();
    let (answers_tx, answers) = std::sync::mpsc::channel();
    answers_tx.send(Some(first)).unwrap();
    (
        SteppedMeter {
            announce,
            answers,
            last: first,
            released: false,
        },
        MeterControl {
            observed,
            answers: answers_tx,
        },
    )
}

const STEP_INTERVAL: Duration = Duration::from_millis(2);

struct SteppedPublisher {
    control: Arc<MeterControl>,
    publisher: Option<FinalizeProgressPublisher>,
}

impl SteppedPublisher {
    fn start(out_dir: &Path, first_work: u64) -> Self {
        let (meter, control) = stepped_meter(first_work);
        let publisher = FinalizeProgressPublisher::start(
            out_dir,
            binding(),
            Box::new(meter),
            STEP_INTERVAL,
            PHASE_FINALIZING,
        )
        .unwrap();
        let started = Self {
            control: Arc::new(control),
            publisher: Some(publisher),
        };
        started.control.started();
        started
    }

    fn step(&self, work: u64) {
        self.control.step(work);
    }

    fn publisher_mut(&mut self) -> &mut FinalizeProgressPublisher {
        self.publisher.as_mut().expect("publisher already finished")
    }

    /// Release the parked thread and seal.
    fn finish(mut self) -> Result<(), String> {
        let publisher = self.publisher.take().expect("publisher already finished");
        self.control.release();
        publisher.finish()
    }

    fn finish_released_by_hook(mut self) -> Result<(), String> {
        self.publisher
            .take()
            .expect("publisher already finished")
            .finish()
    }
}

impl Drop for SteppedPublisher {
    fn drop(&mut self) {
        self.control.release();
        drop(self.publisher.take());
    }
}

#[test]
fn work_staleness_separates_an_advancing_finalize_from_a_stalled_one() {
    let dir = TempDir::new("staleness");
    let stepped = SteppedPublisher::start(dir.path(), 0);

    for step in 1..=5u64 {
        stepped.step(step * 4_096);
        let sample = read_published_sample(dir.path()).unwrap().unwrap();
        assert_eq!(sample.sample_ordinal, step);
        assert_eq!(sample.work_bytes, step * 4_096);
        assert_eq!(
            sample.work_changed_ordinal, step,
            "an advancing sample did not name itself as the change"
        );
        assert_eq!(
            sample.work_unchanged_ms, 0,
            "an advancing finalize published staleness against work that had \
             just changed"
        );
    }

    let mut previous_unchanged = 0u64;
    for step in 6..=10u64 {
        stepped.step(5 * 4_096);
        let sample = read_published_sample(dir.path()).unwrap().unwrap();
        assert_eq!(
            sample.sample_ordinal, step,
            "the publisher stopped sampling during the stall"
        );
        assert_eq!(
            sample.work_bytes,
            5 * 4_096,
            "stalled sample reported a work quantity nobody wrote"
        );
        assert_eq!(
            sample.work_changed_ordinal, 5,
            "a stalled sample moved the change ordinal; a supervisor bounding \
             this could never call it wedged"
        );
        assert!(
            sample.work_unchanged_ms >= previous_unchanged,
            "staleness went backwards during an uninterrupted stall: {} !>= {}",
            sample.work_unchanged_ms,
            previous_unchanged
        );
        previous_unchanged = sample.work_unchanged_ms;
        assert_eq!(sample.phase, PHASE_FINALIZING);
    }
    assert!(
        previous_unchanged > 0,
        "five stalled samples, each preceded by a wait of at least {} ms, accumulated no \
         staleness at all; the published field is not measuring the passage of time",
        STEP_INTERVAL.as_millis()
    );

    stepped.finish().unwrap();
}

#[test]
fn resumed_work_clears_the_staleness_a_supervisor_bounds() {
    let dir = TempDir::new("resume");
    let stepped = SteppedPublisher::start(dir.path(), 7);

    for step in 1..=4u64 {
        stepped.step(7);
        let stalled = read_published_sample(dir.path()).unwrap().unwrap();
        assert_eq!(stalled.sample_ordinal, step);
        assert_eq!(
            stalled.work_changed_ordinal, 0,
            "a stalled sample restarted the stall it is supposed to be bounding"
        );
    }
    let stalled = read_published_sample(dir.path()).unwrap().unwrap();
    assert!(
        stalled.work_unchanged_ms > 0,
        "four stalled samples accumulated no staleness at all, so the reset \
         below would prove nothing"
    );

    stepped.step(9_001);
    let resumed = read_published_sample(dir.path()).unwrap().unwrap();
    assert_eq!(resumed.work_bytes, 9_001);
    assert_eq!(
        resumed.work_unchanged_ms, 0,
        "staleness did not reset when work resumed"
    );
    assert_eq!(
        resumed.work_changed_ordinal, resumed.sample_ordinal,
        "resumed work did not move work_changed_ordinal onto the resuming sample"
    );
    assert!(
        resumed.work_changed_ordinal > stalled.work_changed_ordinal,
        "resumed work did not move work_changed_ordinal: {} !> {}",
        resumed.work_changed_ordinal,
        stalled.work_changed_ordinal
    );

    stepped.step(9_001);
    let after = read_published_sample(dir.path()).unwrap().unwrap();
    assert_eq!(
        after.work_changed_ordinal, resumed.sample_ordinal,
        "the next stall was measured from the wrong change"
    );

    stepped.finish().unwrap();
}

#[test]
fn a_stall_accumulates_milliseconds_without_bound() {
    let started = Instant::now();
    let mut tracker = ProgressTracker::new(started);
    let at = |ms: u64| started + Duration::from_millis(ms);
    let sample = |tracker: &mut ProgressTracker, ordinal: u64, ms: u64, work: u64| {
        tracker.sample(
            PHASE_FINALIZING,
            ordinal,
            at(ms),
            SystemTime::now(),
            started,
            work,
        )
    };

    let first = sample(&mut tracker, 0, 0, 4_096);
    assert_eq!(first.work_unchanged_ms, 0);
    assert_eq!(first.work_changed_ordinal, 0);

    assert_eq!(
        sample(&mut tracker, 1, 2_000, 4_096).work_unchanged_ms,
        2_000
    );
    assert_eq!(
        sample(&mut tracker, 2, 300_000, 4_096).work_unchanged_ms,
        300_000
    );
    let deep = sample(&mut tracker, 3, 600_000, 4_096);
    assert_eq!(deep.work_unchanged_ms, 600_000);
    assert_eq!(
        deep.work_changed_ordinal, 0,
        "ten minutes of standing still moved the change ordinal"
    );
    assert_eq!(deep.elapsed_ms, 600_000);

    let resumed = sample(&mut tracker, 4, 600_100, 4_097);
    assert_eq!(resumed.work_unchanged_ms, 0);
    assert_eq!(resumed.work_changed_ordinal, 4);

    let shrunk = sample(&mut tracker, 5, 602_000, 8);
    assert_eq!(shrunk.work_unchanged_ms, 0);
    assert_eq!(shrunk.work_changed_ordinal, 5);

    assert_eq!(
        sample(&mut tracker, 6, 607_000, 8).work_unchanged_ms,
        5_000,
        "the stall after a decrease was measured from the wrong change"
    );
}

#[test]
fn finish_joins_the_publisher_before_sealing_the_receipt() {
    let dir = TempDir::new("finish");
    let mut stepped = SteppedPublisher::start(dir.path(), 0);
    stepped.step(64);

    let hook_control = Arc::clone(&stepped.control);
    stepped.publisher_mut().on_before_stop_and_join(move || {
        hook_control.step_final(128);
    });
    let control = Arc::clone(&stepped.control);
    stepped.finish_released_by_hook().unwrap();

    assert!(
        control.answers.send(Some(1_000_000)).is_err(),
        "the sampling thread outlived `finish`, so it could still overwrite the seal"
    );
    let sealed = read_published_sample(dir.path()).unwrap().unwrap();
    assert_eq!(
        sealed.phase, PHASE_COMPLETE,
        "a sample published while `finish` was running landed on top of the seal"
    );
    assert_eq!(
        read_published_sample(dir.path()).unwrap().unwrap(),
        sealed,
        "a sample landed after the terminal receipt was published"
    );
}

#[test]
fn drop_stops_and_joins_the_publisher() {
    let dir = TempDir::new("drop");
    let control;
    {
        let stepped = SteppedPublisher::start(dir.path(), 0);
        stepped.step(32);
        control = Arc::clone(&stepped.control);
        // `SteppedPublisher::drop` releases the parked thread and then drops the
    }
    assert!(
        control.answers.send(Some(555_555)).is_err(),
        "the publisher thread outlived the dropped publisher"
    );
    let last = read_published_sample(dir.path()).unwrap().unwrap();
    assert_eq!(last.phase, PHASE_FINALIZING);
    assert_eq!(
        read_published_sample(dir.path()).unwrap().unwrap(),
        last,
        "a sample landed after the publisher was dropped"
    );
}

struct HammeringReader {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<(u64, u64, u64)>>,
}

impl HammeringReader {
    fn start(out_dir: &Path) -> (Self, std::sync::mpsc::Receiver<u64>) {
        let stop = Arc::new(AtomicBool::new(false));
        let reader_stop = Arc::clone(&stop);
        let reader_dir = out_dir.to_path_buf();
        let (seen_tx, seen) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            let (mut parsed, mut absent, mut reads) = (0u64, 0u64, 0u64);
            let mut last = None;
            loop {
                reads += 1;
                match read_published_sample(&reader_dir) {
                    Ok(Some(sample)) => {
                        parsed += 1;
                        if last != Some(sample.sample_ordinal) {
                            last = Some(sample.sample_ordinal);
                            let _ = seen_tx.send(sample.sample_ordinal);
                        }
                    }
                    Ok(None) => absent += 1,
                    Err(error) => panic!("an observation was not a whole document: {error}"),
                }
                if reader_stop.load(Ordering::Acquire) {
                    return (parsed, absent, reads);
                }
            }
        });
        (
            Self {
                stop,
                handle: Some(handle),
            },
            seen,
        )
    }

    fn finish(mut self) -> (u64, u64, u64) {
        self.stop.store(true, Ordering::Release);
        self.handle
            .take()
            .expect("reader already finished")
            .join()
            .expect("the reader observed a torn receipt")
    }
}

impl Drop for HammeringReader {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[test]
fn every_observation_of_the_receipt_parses() {
    const REPUBLISHES: u64 = 60;

    let dir = TempDir::new("atomic");
    let stepped = SteppedPublisher::start(dir.path(), 0);

    let (reader, seen) = HammeringReader::start(dir.path());

    for step in 1..=REPUBLISHES {
        stepped.step(step);
        loop {
            match seen.recv() {
                Ok(ordinal) if ordinal == step => break,
                Ok(_) => {}
                Err(_) => panic!("the reader stopped before it observed sample {step}"),
            }
        }
    }

    let (parsed, absent, reads) = reader.finish();
    let final_sample = read_published_sample(dir.path()).unwrap().unwrap();
    assert_eq!(
        final_sample.sample_ordinal, REPUBLISHES,
        "the publisher did not complete the republishes the reader was racing"
    );
    assert!(
        parsed >= REPUBLISHES,
        "the reader was observed catching {REPUBLISHES} samples but counted only {parsed} whole \
         reads"
    );
    assert_eq!(
        parsed + absent,
        reads,
        "an observation was neither whole nor absent"
    );

    if let Err(error) = stepped.finish() {
        assert!(
            error.contains(RECEIPT_FILE_NAME),
            "a failed seal must name the receipt it could not publish: {error}"
        );
    }
}

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

    fs::write(
        dir.path().join("passive_speed5_witness.jsonl"),
        vec![0u8; 4_096],
    )
    .unwrap();
    assert_eq!(
        meter.observe(),
        620,
        "the meter counted a file the recorder does not own"
    );

    portable_test_fs_retry::remove_dir_all(&watched).unwrap();
    assert_eq!(meter.observe(), 100);
}

#[test]
fn run_directory_meter_tracks_the_temporary_names_finalize_actually_writes() {
    let dir = TempDir::new("rundir");
    let mut meter = RunDirectoryMeter::new(dir.path());
    assert_eq!(meter.observe(), 0);

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

    fs::write(
        dir.path().join("passive_speed5_witness.jsonl"),
        vec![0u8; 9_999],
    )
    .unwrap();
    fs::write(
        dir.path().join("collector_record.stderr.log"),
        vec![0u8; 512],
    )
    .unwrap();
    assert_eq!(
        meter.observe(),
        1_000,
        "the meter counted harness-owned files in the run directory"
    );

    portable_test_fs_retry::remove_dir_all(&spill).unwrap();
    let container_temp = dir.path().join("native_session.battleplan.tmp-abc123-0");
    fs::write(&container_temp, vec![0u8; 5_000]).unwrap();
    let with_temp = meter.observe();
    assert_eq!(
        with_temp, 5_700,
        "the container's temporary name was invisible to the meter"
    );

    fs::rename(
        &container_temp,
        dir.path().join("native_session.battleplan"),
    )
    .unwrap();
    assert_eq!(meter.observe(), with_temp);

    fs::write(
        dir.path().join(".native_session_manifest.json.1234.99.tmp"),
        vec![0u8; 42],
    )
    .unwrap();
    assert_eq!(meter.observe(), with_temp + 42);
}

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
    fs::write(
        member.join("native_session.battleplan.tmp-x-0"),
        vec![0u8; 2_048],
    )
    .unwrap();
    assert_eq!(
        meter.observe(),
        2_048,
        "nested rollover member work was invisible to the meter"
    );
}

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
    assert!(
        sample.elapsed_ms >= 50,
        "elapsed was not measured from start"
    );
}

#[test]
fn a_foreign_schema_is_refused_not_read_as_zero_progress() {
    let dir = TempDir::new("schema");
    fs::write(
        dir.path().join(RECEIPT_FILE_NAME),
        br#"{"schema":"battle-plan-recorder-finalize-progress-v0","phase":"finalizing"}"#,
    )
    .unwrap();
    let error = read_published_sample(dir.path()).unwrap_err();
    assert!(
        error.contains("unsupported finalize progress schema"),
        "{error}"
    );
}

#[test]
fn no_receipt_is_absence_not_an_error() {
    let dir = TempDir::new("absent");
    assert!(read_published_sample(dir.path()).unwrap().is_none());
}
