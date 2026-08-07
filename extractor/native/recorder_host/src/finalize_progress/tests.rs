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

/// Meter that turns the publisher's sampling thread into a STEPPED machine.
///
/// `observe` is called exactly once per sample and nothing else in the loop is
/// reachable from a test, so it is the one place a test can stand INSIDE the
/// publisher. This meter announces every call and then parks until the test
/// hands back the quantity to report, which buys two things no wall clock can:
///
///   * the test decides exactly which work quantity each sample sees, and
///   * between the announcement and the answer the publisher is parked in
///     `observe` -- it is not metering, not serialising and not renaming -- so
///     the receipt on disk is the previous sample, whole, and stays that way
///     for as long as the test needs.
///
/// Every assertion downstream of it is therefore about a KNOWN sample rather
/// than about whatever the publisher managed to produce inside some number of
/// milliseconds. Most are equalities; the two that are not are inequalities
/// against the sampling interval, which load can only make more true. Load can
/// make a step take longer; it cannot change which sample the test is reading.
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
        // Ignore a send failure: it only means the test is already gone, and
        // the recv below then releases us too.
        let _ = self.announce.send(());
        match self.answers.recv() {
            Ok(Some(work)) => {
                self.last = work;
                work
            }
            // `release`, or a dropped control: stop parking so `finish`/`Drop`
            // can stop and join the thread.
            Ok(None) | Err(_) => {
                self.released = true;
                self.last
            }
        }
    }
}

/// The test's end of a `SteppedMeter`.
struct MeterControl {
    observed: std::sync::mpsc::Receiver<()>,
    answers: std::sync::mpsc::Sender<Option<u64>>,
}

impl MeterControl {
    /// Consume the first observation -- which `start` takes on the CALLER's
    /// thread, before the sampling thread exists -- and then wait until the
    /// sampling thread is parked on its own first observation.
    ///
    /// On return: sample 0 is published and the publisher is holding still.
    fn started(&self) {
        self.observed.recv().expect("start took no observation");
        self.observed
            .recv()
            .expect("the sampling thread took no observation");
    }

    /// Answer the observation the publisher is parked on with `work`, then wait
    /// until it parks on the NEXT one.
    ///
    /// Reaching the next observation is only possible after the sample built
    /// from `work` has been published, so on return the receipt holds exactly
    /// that sample and nothing is writing it.
    fn step(&self, work: u64) {
        self.answers.send(Some(work)).expect("publisher is gone");
        self.observed.recv().expect("publisher stopped sampling");
    }

    /// Answer the parked observation and then wait for the sampling thread to
    /// DIE, rather than for the next observation.
    ///
    /// Only valid once the stop flag is already set -- which is exactly the
    /// state `on_before_stop_and_join` fires in. The thread publishes this
    /// sample, returns to the top of its loop, sees the stop and exits, which
    /// drops the meter it owns and disconnects this channel. A disconnect is
    /// therefore proof that the sample is on disk AND that no further sample
    /// can ever be published -- both facts, neither a duration.
    fn step_final(&self, work: u64) {
        self.answers.send(Some(work)).expect("publisher is gone");
        if self.observed.recv().is_ok() {
            // Failing here means the thread is parked on that observation, and
            // the panic about to be raised unwinds straight into the
            // publisher's join. Let it go FIRST: a test that hangs on a real
            // regression reports nothing, which is worse than the flake this
            // machinery replaced.
            self.release();
            panic!("the sampling thread took another observation after it was stopped");
        }
    }

    /// Let the publisher run unattended so `finish`/`Drop` can join it.
    fn release(&self) {
        let _ = self.answers.send(None);
    }
}

/// The publisher takes its FIRST observation on the calling thread inside
/// `start`, so that answer has to be queued before `start` runs or the test
/// would park itself.
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

/// Interval for the stepped tests. The rendezvous, not this, paces them.
///
/// It does carry one load-bearing guarantee: the sampling loop waits it out
/// before every observation, so consecutive samples are separated by at least
/// this much MONOTONIC time. That is what lets a stalled sample assert it
/// accumulated staleness at all. It is a floor, and load only raises it, so no
/// assertion built on it can be made false by a busy machine.
const STEP_INTERVAL: Duration = Duration::from_millis(2);

/// A stepped publisher that cannot hang the test that owns it.
///
/// The sampling thread parks inside `observe` waiting on the control, and the
/// publisher's `Drop` joins that thread. Held as two plain locals, the
/// publisher would be declared last and so drop FIRST -- joining a thread that
/// is still waiting for a control that has not dropped yet, which never
/// returns. A failing assertion would then HANG the test instead of failing it,
/// and a test that hangs on failure is worse than the flake it replaced.
/// Releasing before dropping the publisher is what makes failure terminate.
struct SteppedPublisher {
    control: Arc<MeterControl>,
    publisher: Option<FinalizeProgressPublisher>,
}

impl SteppedPublisher {
    fn start(out_dir: &Path, first_work: u64) -> Self {
        let (meter, control) = stepped_meter(first_work);
        let publisher =
            FinalizeProgressPublisher::start(out_dir, binding(), Box::new(meter), STEP_INTERVAL)
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

    /// Seal WITHOUT releasing first: the caller has installed a hook that does
    /// the releasing at the exact moment `finish` is about to join.
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

/// THE regression test for #116.
///
/// A recorder finalizing a multi-GB stream and a recorder spinning on an
/// unsatisfiable condition are indistinguishable from outside unless something
/// published tells them apart -- which is why four live trials on 2026-08-06
/// were declared unresolved on a fixed 300 s budget while their folds were
/// still writing checkpoint bytes.
///
/// The property: `sample_ordinal` advances in BOTH cases (it is liveness only,
/// and must never be used as a progress signal), while the change-tracking
/// fields separate them -- `work_changed_ordinal` follows the sample that
/// changed and freezes through a stall, and `work_unchanged_ms` is zero exactly
/// at a change. Reverting the change-tracking in `ProgressTracker::sample` --
/// for instance by stamping `changed_at`/`changed_ordinal` on every sample
/// rather than only on a change -- makes `work_changed_ordinal` track
/// `sample_ordinal` through the stall and fails the stalled half of this test,
/// which is precisely the fixed-timer behaviour being replaced.
///
/// How long a stall MEASURES in milliseconds is proven separately, on a
/// synthetic timeline, by `a_stall_accumulates_milliseconds_without_bound` --
/// that is a property of the tracker, and asserting it through a real thread
/// would only be asserting that the machine was not busy.
#[test]
fn work_staleness_separates_an_advancing_finalize_from_a_stalled_one() {
    let dir = TempDir::new("staleness");
    let stepped = SteppedPublisher::start(dir.path(), 0);

    // Phase A: work advances. Every sample is a change, so every sample must
    // name ITSELF as the one that changed and report no staleness at all.
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

    // Phase B: work stops at the value phase A left. The publisher keeps
    // sampling -- liveness -- but nothing may look like progress again.
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
    // And it must have accumulated SOMETHING. Without this the publisher could
    // be feeding the tracker a frozen instant -- `started` instead of
    // `Instant::now()` -- and every stalled sample would report 0 ms forever
    // while still satisfying every equality above. That is the bound the
    // supervisor applies, silently pinned off.
    //
    // Not a wall-clock margin: the sampling loop waits STEP_INTERVAL before
    // each observation, so five stalled samples are separated by at least
    // 5 * STEP_INTERVAL of monotonic time, and a loaded machine only makes that
    // larger. The assertion is that the number is not zero, not that it is big.
    assert!(
        previous_unchanged > 0,
        "five stalled samples, each preceded by a wait of at least {} ms, accumulated no \
         staleness at all; the published field is not measuring the passage of time",
        STEP_INTERVAL.as_millis()
    );

    stepped.finish().unwrap();
}

/// Complement of the test above: resumed work must clear the staleness, so the
/// field bounds the CURRENT stall rather than accumulating over the run.
///
/// `work_unchanged_ms` at a resuming sample is exactly zero, not "small": the
/// tracker stamps `changed_at` with the same instant it then measures against.
/// Asserting the exact value is what makes this independent of how loaded the
/// machine is -- the previous form asserted that the stall had accumulated at
/// least 200 ms of REAL time by the time the test looked, and an I/O-loaded
/// machine that had published only sample 0 by then reported 0 ms and failed.
#[test]
fn resumed_work_clears_the_staleness_a_supervisor_bounds() {
    let dir = TempDir::new("resume");
    let stepped = SteppedPublisher::start(dir.path(), 7);

    // Stall: the quantity never moves off the value the first sample saw, so
    // every sample must keep pointing back at sample 0.
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
    // There has to BE a stall to clear. See the note in the test above: this is
    // a floor the sampling interval guarantees, not a margin load can erase.
    assert!(
        stalled.work_unchanged_ms > 0,
        "four stalled samples accumulated no staleness at all, so the reset \
         below would prove nothing"
    );

    // Resume.
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

    // And the stall starts over from the resuming sample, not from the run.
    stepped.step(9_001);
    let after = read_published_sample(dir.path()).unwrap().unwrap();
    assert_eq!(
        after.work_changed_ordinal, resumed.sample_ordinal,
        "the next stall was measured from the wrong change"
    );

    stepped.finish().unwrap();
}

/// The millisecond half of the staleness contract, on a synthetic timeline so
/// it states exact numbers instead of hoping a thread got scheduled.
///
/// `ProgressTracker` takes `now` as an argument precisely so this is possible:
/// no thread, no clock, no filesystem. A supervisor bounds `work_unchanged_ms`,
/// so what has to be true is that an uninterrupted stall accumulates it without
/// bound and that a change zeroes it -- both stated here as equalities.
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

    // A stall of ten minutes, sampled every two seconds: the field is the
    // distance back to the change, never the distance to the previous sample.
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

    // One byte of progress ends the stall outright -- not gradually.
    let resumed = sample(&mut tracker, 4, 600_100, 4_097);
    assert_eq!(resumed.work_unchanged_ms, 0);
    assert_eq!(resumed.work_changed_ordinal, 4);

    // The work quantity legitimately DECREASES once per run (the fold's
    // snapshot spill is deleted), and a decrease is progress like any other
    // change -- treating only increases as progress would call the container
    // phase wedged.
    let shrunk = sample(&mut tracker, 5, 602_000, 8);
    assert_eq!(shrunk.work_unchanged_ms, 0);
    assert_eq!(shrunk.work_changed_ordinal, 5);

    // And the new stall is measured from the shrink, not from the run.
    assert_eq!(
        sample(&mut tracker, 6, 607_000, 8).work_unchanged_ms,
        5_000,
        "the stall after a decrease was measured from the wrong change"
    );
}

/// The publisher owns a thread that writes into the run directory. `finish`
/// must join it BEFORE publishing the terminal sample, or a late sample could
/// overwrite `complete` and a supervisor would wait for a recorder that had
/// already finished.
///
/// Forced, not raced. The sampling thread is parked inside its meter for the
/// whole test, and the ONE extra sample it is allowed is let through from
/// `on_before_stop_and_join` -- which fires after the stop flag is set and
/// before the join. `step_final` returns only when the sampling thread has
/// DIED, so at that instant a `finalizing` sample newer than anything `finish`
/// has written is on disk and no further sample can ever be published.
///
/// That makes both outcomes deterministic rather than scheduling-dependent: an
/// implementation that sealed before this point has had its seal overwritten,
/// every run; this one seals after the join and cannot be overwritten, because
/// the only thread that could is provably gone.
#[test]
fn finish_joins_the_publisher_before_sealing_the_receipt() {
    let dir = TempDir::new("finish");
    let mut stepped = SteppedPublisher::start(dir.path(), 0);
    stepped.step(64);

    let hook_control = Arc::clone(&stepped.control);
    stepped.publisher_mut().on_before_stop_and_join(move || {
        // Publishes sample 2 and returns only once the thread that wrote it has
        // exited.
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

/// Dropping on a `?` path must also stop and join, never abandon the thread to
/// process exit partway through a rename.
#[test]
fn drop_stops_and_joins_the_publisher() {
    let dir = TempDir::new("drop");
    let control;
    {
        let stepped = SteppedPublisher::start(dir.path(), 0);
        stepped.step(32);
        control = Arc::clone(&stepped.control);
        // `SteppedPublisher::drop` releases the parked thread and then drops the
        // publisher, whose own `Drop` is what must stop and join it.
    }
    // Same argument as above: the dropped publisher joined the thread, and the
    // thread owned the meter.
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

/// A thread that reads the receipt as fast as it can and reports each new
/// sample ordinal it sees, so a test can WAIT for it to have observed one.
///
/// Owned by a guard whose `Drop` stops and joins it, because it is detached
/// otherwise: a panic anywhere in the stepping loop would leave it spinning on
/// a directory the `TempDir` is about to delete, for the rest of the test
/// binary's life.
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
                            // A closed channel only means the test has stopped
                            // waiting on us.
                            let _ = seen_tx.send(sample.sample_ordinal);
                        }
                    }
                    // A momentary absence is legal; see below.
                    Ok(None) => absent += 1,
                    // A torn or truncated observation surfaces here as a parse
                    // error, and there is no reading of the publish sequence
                    // under which that is allowed.
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

/// Every observation THAT FINDS THE RECEIPT is a complete document: the file is
/// rewritten many times, so a supervisor polling it must never read a
/// half-written one.
///
/// The republishes are stepped from the test thread while a reader thread
/// hammers the file, and the stepping does not advance until the reader has
/// REPORTED seeing the sample just published. Spawning a reader and hoping it
/// gets scheduled would prove nothing -- it could first run after the last
/// republish, read once, and let the test pass having raced nothing. Because
/// the reader has to be observed catching every one of the samples, it was
/// reading continuously across every rename between them.
///
/// What this test may NOT claim is that the receipt is always there. Windows
/// replaces a file by renaming over it, and that replace is not atomic on this
/// project's disks: probing the exact temp-write/fsync/rename sequence
/// `publish_sample` uses, a concurrent reader saw `RECORDER_FINALIZE_PROGRESS_
/// V1.json` reported ERROR_FILE_NOT_FOUND while a directory listing taken at
/// the same instant contained only the publisher's `.tmp` -- the old name
/// already unlinked, the new one not yet in place. The windows measured were
/// microseconds most of the time and 7-16 ms when a filesystem filter got
/// involved. So a momentary ABSENCE is a legal observation and a TORN one is
/// not, which is what is asserted here. The consumer consequence of that
/// absence is recorded on `require_no_live_finalizer` in `main.rs`.
#[test]
fn every_observation_of_the_receipt_parses() {
    const REPUBLISHES: u64 = 60;

    let dir = TempDir::new("atomic");
    let stepped = SteppedPublisher::start(dir.path(), 0);

    let (reader, seen) = HammeringReader::start(dir.path());

    for step in 1..=REPUBLISHES {
        stepped.step(step);
        // Do not advance until the reader has been SEEN observing this sample.
        // Guaranteed to arrive rather than hoped for: the publisher is parked
        // inside its meter, so the receipt holds this sample and stops changing
        // until the next step, and the reader is looping on it.
        loop {
            match seen.recv() {
                Ok(ordinal) if ordinal == step => break,
                // An older sample it was still catching up on.
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

    // Sealing is deliberately NOT unwrapped here, and this is the one test in
    // the module where that is right. Hammering the receipt with reads is what
    // provokes the other half of the same Windows behaviour: replacing a name a
    // reader still has open leaves it delete-pending, and the next rename onto
    // it fails with ERROR_ACCESS_DENIED (observed under concurrent build load).
    // Production tolerates exactly that -- the sampling thread swallows publish
    // failures, and `main.rs` downgrades a failed seal to a warning rather than
    // fail a capture whose container and manifest are already sealed. Asserting
    // more than production requires is what made this test load-sensitive. The
    // seal itself is asserted strictly, with no reader racing it, in
    // `finish_joins_the_publisher_before_sealing_the_receipt`.
    if let Err(error) = stepped.finish() {
        assert!(
            error.contains(RECEIPT_FILE_NAME),
            "a failed seal must name the receipt it could not publish: {error}"
        );
    }
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
