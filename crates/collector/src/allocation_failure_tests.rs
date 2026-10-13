//! Disposable synthetic processes isolate terminal state and allocator refusal.

use crate::allocation::{self, Code, Context, Phase};
use replay::rows::RowCodec;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    ptr,
    sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicUsize, Ordering},
    time::{Duration, Instant},
};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;

const CHILD: &str = "allocation_failure_tests::allocation_safety_child";
const CAMPAIGN: &[u8; 36] = b"01234567-89ab-cdef-0123-456789abcdef";
#[cfg(feature = "alloc-fault-injection")]
const PHASES: [(Phase, &str); 11] = [
    (Phase::Bootstrap, "bootstrap"),
    (Phase::Callback, "callback"),
    (Phase::Acquisition, "acquisition"),
    (Phase::Raw, "raw"),
    (Phase::Decode, "decode"),
    (Phase::Extended, "extended"),
    (Phase::Spool, "spool"),
    (Phase::Encode, "encode"),
    (Phase::Map, "map"),
    (Phase::Status, "status"),
    (Phase::Teardown, "teardown"),
];

struct FailingAllocator;
#[global_allocator]
static ALLOCATOR: FailingAllocator = FailingAllocator;

// Only the prepared-buffer child interval denies allocations. Fixed controls,
// paths, thread creation and recovery run outside that accepted boundary.
static OWNER: AtomicU32 = AtomicU32::new(0);
static ORDINAL: AtomicUsize = AtomicUsize::new(usize::MAX);
static ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
static DENIALS: AtomicUsize = AtomicUsize::new(0);
static TARGET_PHASE: AtomicU8 = AtomicU8::new(0);
static REACHED: AtomicBool = AtomicBool::new(false);
thread_local! {
    static FAIL_PUBLICATION: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn before_spool_publication() {
    if FAIL_PUBLICATION.replace(false) {
        allocation::fail(Phase::Spool, Code::Reserve);
    }
}
pub(crate) fn phase_enter(phase: Phase) {
    if TARGET_PHASE.load(Ordering::Relaxed) == phase as u8 {
        REACHED.store(true, Ordering::Relaxed);
    }
}
pub(crate) fn with_allocations_denied<T>(run: impl FnOnce() -> T) -> T {
    let guard = NullGuard::new(1);
    let output = run();
    drop(guard);
    output
}
fn deny() -> bool {
    if OWNER.load(Ordering::Relaxed) != unsafe { GetCurrentThreadId() } {
        return false;
    }
    let attempt = ATTEMPTS.fetch_add(1, Ordering::Relaxed) + 1;
    if attempt >= ORDINAL.load(Ordering::Relaxed) {
        DENIALS.fetch_add(1, Ordering::Relaxed);
        return true;
    }
    false
}
unsafe impl GlobalAlloc for FailingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if deny() {
            ptr::null_mut()
        } else {
            unsafe { System.alloc(layout) }
        }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if deny() {
            ptr::null_mut()
        } else {
            unsafe { System.alloc_zeroed(layout) }
        }
    }
    unsafe fn realloc(&self, old: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if deny() {
            ptr::null_mut()
        } else {
            unsafe { System.realloc(old, layout, size) }
        }
    }
    unsafe fn dealloc(&self, allocation: *mut u8, layout: Layout) {
        unsafe { System.dealloc(allocation, layout) };
    }
}
struct NullGuard;
impl NullGuard {
    fn new(ordinal: usize) -> Self {
        ORDINAL.store(ordinal, Ordering::Relaxed);
        ATTEMPTS.store(0, Ordering::Relaxed);
        DENIALS.store(0, Ordering::Relaxed);
        OWNER.store(unsafe { GetCurrentThreadId() }, Ordering::Relaxed);
        Self
    }
}
impl Drop for NullGuard {
    fn drop(&mut self) {
        OWNER.store(0, Ordering::Relaxed);
    }
}

fn child(mode: &str, phase: &str, ordinal: usize) -> Output {
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    let run = std::env::temp_dir().join(format!(
        "hoi4-collector-allocation-{}-{}",
        std::process::id(),
        CALLS.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&run);
    let mut process = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            CHILD,
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("BATTLE_PLAN_COLLECTOR_ALLOCATION_TEST", mode)
        .env("BATTLE_PLAN_COLLECTOR_ALLOCATION_RUN", &run)
        .env("BATTLE_PLAN_COLLECTOR_ALLOCATION_PHASE", phase)
        .env("BATTLE_PLAN_COLLECTOR_ALLOCATION_ORDINAL", ordinal.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Bounds deadlock while allowing filesystem recovery on a shared host.
    let deadline = Instant::now() + Duration::from_secs(120);
    while process.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            process.kill().unwrap();
            let output = process.wait_with_output().unwrap();
            panic!(
                "synthetic collector child timed out for {mode}/{phase}/{ordinal}: {} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = process.wait_with_output().unwrap();
    // A failed child keeps its files for diagnosis.
    if output.status.success() {
        let _ = fs::remove_dir_all(&run);
    }
    output
}
fn successful(output: &Output, mode: &str, phase: &str, ordinal: usize) {
    assert!(
        output.status.success(),
        "collector child failed for {mode}/{phase}/{ordinal}: {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
fn observed_allocations(output: &Output) -> usize {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| {
            line.split_once("collector-allocation-count=")
                .map(|(_, count)| count)
        })
        .expect("child allocation coverage marker")
        .parse()
        .unwrap()
}

/// An unwinding callback drains every job its batch accepted, finished or
/// cancelled before it starts, whether a terminal allocation failure came
/// before the unwind or during the drain
/// (`acquisition_jobs::batch_drain_tests`).
#[test]
fn a_terminal_failure_never_skips_the_callbacks_drain() {
    for mode in ["batch-drain-terminal", "batch-drain-midway"] {
        let output = child(mode, "raw", usize::MAX);
        successful(&output, mode, "raw", usize::MAX);
        let case = mode.trim_start_matches("batch-drain-");
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .contains(&format!("collector-batch-drain-{case}=ok")),
            "{mode} did not reach its drain checks"
        );
    }
}

#[test]
fn persistent_null_allocator_survives_prepared_buffers() {
    for name in ["raw", "decode", "extended", "spool", "encode", "status"] {
        let baseline = child("null", name, usize::MAX);
        successful(&baseline, "null", name, usize::MAX);
        let count = observed_allocations(&baseline);
        assert!(count > 0, "the prepared {name} payload must grow storage");
        println!("null allocation samples {name}: {count} allocator calls");
        let samples = [1, count.div_ceil(2), count];
        for (index, ordinal) in samples.into_iter().enumerate() {
            if !samples[..index].contains(&ordinal) {
                let output = child("null", name, ordinal);
                successful(&output, "null", name, ordinal);
            }
        }
    }
}

#[test]
fn raw_hints_are_advisory_and_capture_failure_keeps_request_details() {
    let output = child("raw-advisory", "raw", 1);
    successful(&output, "raw-advisory", "raw", 1);
}

#[test]
fn fuel_history_growth_writes_the_terminal_witness() {
    let output = child("fuel-history", "extended", 1);
    successful(&output, "fuel-history", "extended", 1);
    assert_eq!(observed_allocations(&output), 1);
}

#[cfg(feature = "alloc-fault-injection")]
#[test]
fn allocation_fault_waits_for_committed_hours() {
    let output = child("deferred", "encode", 1);
    successful(&output, "deferred", "encode", 1);
}

#[test]
fn concurrent_commits_preserve_one_correlated_failure_snapshot() {
    let output = child("concurrent-watermark", "status", 1);
    successful(&output, "concurrent-watermark", "status", 1);
}

#[cfg(feature = "alloc-fault-injection")]
#[test]
fn checked_allocation_sweep_stops_once_and_preserves_prefix() {
    let mut cases = Vec::new();
    for (_, name) in PHASES {
        for changed_loads in [false, true] {
            if changed_loads && !matches!(name, "spool" | "encode" | "map") {
                continue;
            }
            let mode = if changed_loads {
                "checked-load"
            } else {
                "checked"
            };
            let baseline = child(mode, name, usize::MAX);
            successful(&baseline, mode, name, usize::MAX);
            let count = observed_allocations(&baseline);
            println!("{mode} allocation sweep {name}: {count} allocation attempts");
            cases.extend((1..=count).map(|ordinal| (mode, name, ordinal)));
        }
    }
    let next = AtomicUsize::new(0);
    let completed = AtomicUsize::new(0);
    // Each child has its own allocator, terminal state, synthetic memory and
    // PID-qualified files. Only the coordinator shares these work counters.
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&(mode, name, ordinal)) = cases.get(index) else {
                        break;
                    };
                    let output = child(mode, name, ordinal);
                    successful(&output, mode, name, ordinal);
                    let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
                    if done.is_multiple_of(100) {
                        println!("collector-checked-sweep-progress={done}/{}", cases.len());
                    }
                }
            });
        }
    });
    let output = child("pre-publication", "spool", usize::MAX);
    successful(&output, "pre-publication", "spool", usize::MAX);
    assert_eq!(completed.load(Ordering::Relaxed), cases.len());
    println!("collector-checked-sweep-total={}", cases.len());
}

#[test]
#[ignore = "launched only by allocation-failure parent tests"]
fn allocation_safety_child() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mode = std::env::var("BATTLE_PLAN_COLLECTOR_ALLOCATION_TEST").or_else(|_| std::env::var("HOI4_COLLECTOR_ALLOCATION_TEST")).unwrap();
    let phase = Phase::parse(&std::env::var("BATTLE_PLAN_COLLECTOR_ALLOCATION_PHASE").or_else(|_| std::env::var("HOI4_COLLECTOR_ALLOCATION_PHASE")).unwrap()).unwrap();
    let ordinal = std::env::var("BATTLE_PLAN_COLLECTOR_ALLOCATION_ORDINAL").or_else(|_| std::env::var("HOI4_COLLECTOR_ALLOCATION_ORDINAL"))
        .unwrap()
        .parse::<usize>()
        .unwrap();
    match mode.as_str() {
        "fuel-history" => {
            allocation::reset();
            allocation::install_hooks();
            prepared_null(
                phase,
                ordinal,
                crate::extended::fuel_awards::allocation_failure_history_exercise,
            )
            .unwrap();
        }
        "raw-advisory" => raw_advisory_and_details().unwrap(),
        "batch-drain-terminal" => crate::acquisition_jobs::batch_drain_tests::exercise("terminal"),
        "batch-drain-midway" => crate::acquisition_jobs::batch_drain_tests::exercise("midway"),
        "null" => prepared_buffers_null(phase, ordinal).unwrap(),
        "concurrent-watermark" => allocation::concurrent_watermark_exercise().unwrap(),
        #[cfg(feature = "alloc-fault-injection")]
        "deferred" => deferred_failure().unwrap(),
        "pre-publication" => validate(
            &exercise_pipeline(phase, ordinal, false, true).unwrap(),
            true,
        ),
        #[cfg(feature = "alloc-fault-injection")]
        "checked" | "checked-load" => {
            let outcome = exercise_pipeline(phase, ordinal, mode == "checked-load", false).unwrap();
            validate(&outcome, ordinal != usize::MAX);
            println!("collector-allocation-count={}", outcome.checked);
        }
        _ => panic!("unknown allocation-test mode"),
    }
}

#[cfg(feature = "alloc-fault-injection")]
fn deferred_failure() -> io::Result<()> {
    allocation::reset();
    allocation::install_hooks();
    let root = case_root(Phase::Encode, 1, true).join("deferred");
    fs::create_dir_all(&root)?;
    let witness_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(root.join(replay::witness::WITNESS_FILE_NAME))?;
    allocation::prepare(&witness_file, 1, 1000);
    allocation::inject_after(Phase::Encode, 1, 3);
    let _scope = allocation::enter(Phase::Encode);
    for hour in 19..=21 {
        context(Some(hour));
        engine::allocation::vec::<u8>(64)?;
        assert!(!allocation::failed());
        assert_eq!(allocation::allocation_count(), 0);
        allocation::committed(0, (hour as u64) * 100);
        allocation::committed_hour();
    }
    context(Some(22));
    assert_eq!(
        engine::allocation::vec::<u8>(64).unwrap_err().kind(),
        io::ErrorKind::OutOfMemory
    );
    assert_eq!(allocation::allocation_count(), 1);
    let witness = allocation::failure().unwrap();
    assert_eq!(witness.phase, Phase::Encode as u8);
    assert_eq!(witness.failed_hour, Some(22));
    assert_eq!(witness.committed_bytes, 2100);
    assert_eq!(Some(witness), read_witness(&witness_file)?);
    allocation::clear_handle();
    Ok(())
}

fn raw_advisory_and_details() -> io::Result<()> {
    allocation::reset();
    allocation::install_hooks();
    let root = case_root(Phase::Raw, 1, false).join("raw-details");
    fs::create_dir_all(&root)?;
    let witness_file = File::create(root.join(replay::witness::WITNESS_FILE_NAME))?;
    allocation::prepare(&witness_file, 1, 1000);
    let _details = allocation::prepare_details(&root).unwrap();
    context(Some(21));
    let mut raw = crate::raw::RawLog::<1>::default();
    {
        let _scope = allocation::enter(Phase::Raw);
        with_allocations_denied(|| {
            raw.presize(64, 4096);
            raw.initialize_read_plan(64, 16, 4096);
        });
        assert!(
            !allocation::failed(),
            "optional sizing must not stop capture"
        );
        assert_eq!((raw.spans.capacity(), raw.bytes.capacity()), (0, 0));
        raw.enable_worker_growth();
        with_allocations_denied(|| raw.reserve(4096));
    }
    let witness = allocation::failure().unwrap();
    assert_eq!(
        (witness.phase, witness.code, witness.failed_hour),
        (Phase::Raw as u8, Code::Reserve as u8, Some(21))
    );
    let detail: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("collector-failure-detail.json"))?).unwrap();
    assert_eq!(detail["session"], 1);
    assert_eq!(detail["process_start_time"], 1000);
    assert_eq!(detail["family"], "raw bytes");
    assert_eq!(detail["requested_bytes"], 4096);
    allocation::clear_handle();
    Ok(())
}

fn prepared_buffers_null(phase: Phase, ordinal: usize) -> io::Result<()> {
    use replay::rows::{StateCoreRow, TableRows};
    allocation::reset();
    allocation::install_hooks();
    match phase {
        Phase::Raw => {
            let mut raw = crate::raw::RawLog::<1>::new(4, 64);
            raw.reserve(4096);
            prepared_null(phase, ordinal, || {
                raw.enable_worker_growth();
                raw.reserve(4096);
                verdict(true)
            })
        }
        Phase::Decode => {
            let decode =
                crate::extended::lexical::lexical_test_kit::allocation_failure_decode_exercise(
                    0, 0,
                );
            prepared_null(phase, ordinal, || verdict(decode()))
        }
        Phase::Extended => {
            let previous = TableRows::StateCores(
                (1..=128)
                    .map(|state| StateCoreRow {
                        state,
                        cores: vec![1, 2, 3, 4],
                    })
                    .collect(),
            );
            let fresh = TableRows::StateCores(vec![StateCoreRow {
                state: 129,
                cores: vec![5, 6],
            }]);
            prepared_null(phase, ordinal, || {
                let merged =
                    replay::sliced::merge_slice(&previous, fresh, |state| state == 129, |_| true)?;
                verdict(merged.len() == 129)
            })
        }
        Phase::Spool => {
            let convert = crate::extended::governance::allocation_failure_fixture();
            prepared_null(phase, ordinal, || verdict(convert()))
        }
        Phase::Encode => prepared_encoder_null(ordinal),
        Phase::Status => {
            crate::status::allocation_failure_exercise()?;
            let rows: Vec<_> = (0..128).map(|row| (row, "borrowed \"status\"\n")).collect();
            let mut bytes = Vec::new();
            prepared_null(phase, ordinal, || {
                crate::status::serialize_into(&mut bytes, &rows)
            })
        }
        _ => unreachable!("unprepared null-allocation boundary"),
    }
}

fn verdict(valid: bool) -> io::Result<()> {
    if allocation::failed() {
        Err(io::ErrorKind::OutOfMemory.into())
    } else if valid {
        Ok(())
    } else {
        Err(io::ErrorKind::InvalidData.into())
    }
}

fn prepared_null(
    phase: Phase,
    ordinal: usize,
    run: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    let root = case_root(phase, ordinal, false);
    fs::create_dir_all(&root)?;
    let witness_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(root.join(replay::witness::WITNESS_FILE_NAME))?;
    allocation::prepare(&witness_file, 1, 1000);
    context((phase != Phase::Status).then_some(21));
    #[cfg(feature = "alloc-fault-injection")]
    allocation::inject(phase, u64::MAX);
    let guard = NullGuard::new(ordinal);
    let scope = allocation::enter(phase);
    let result = run();
    let first = allocation::failure();
    let denials = DENIALS.load(Ordering::Relaxed);
    let attempts = ATTEMPTS.load(Ordering::Relaxed);
    if first.is_some() {
        let _ = engine::allocation::vec::<u8>(1);
        allocation::fail(Phase::Teardown, Code::Thread);
    }
    let unchanged = first == allocation::failure();
    drop(scope);
    drop(guard);
    assert!(
        unchanged,
        "the first failure wins during persistent refusal"
    );
    assert_eq!(result.is_err(), denials > 0);
    assert_eq!(first, read_witness(&witness_file)?);
    if let Some(witness) = first {
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::OutOfMemory);
        assert_eq!(witness.phase, phase as u8);
        assert_eq!(witness.code, Code::Reserve as u8);
        assert_eq!(witness.failed_hour, (phase != Phase::Status).then_some(21));
    } else {
        result?;
    }
    allocation::clear_handle();
    println!("collector-allocation-count={attempts}");
    Ok(())
}

fn prepared_encoder_null(ordinal: usize) -> io::Result<()> {
    use replay::{
        frame::{FrameEncoder, FrameInput, Metrics, TableInput, TableView},
        rows::{FrontRow, TableId, TableRows, tables},
    };
    fn source(hour: i32) -> FrameInput {
        let mut tables = std::array::from_fn(|_| {
            TableInput::Sampled(TableView::Unavailable {
                reason: "synthetic unavailable".into(),
            })
        });
        let rows = (1..=128)
            .map(|front| FrontRow {
                front,
                section_ids: vec![7, 8],
                section_ends: vec![2, 4],
                province_pairs: vec![3, 4, 5, hour as u32],
            })
            .collect();
        tables[TableId::Fronts.index()] =
            TableInput::Sampled(TableView::Complete(TableRows::Fronts(rows)));
        FrameInput {
            branch: 0,
            hour,
            tables,
            metrics: Metrics::default(),
        }
    }
    let mut encoder = FrameEncoder::new();
    encoder.encode_bytes(source(19), true)?;
    encoder.encode_bytes(source(20), false)?;
    let baseline = encoder
        .comparison_baseline::<tables::Fronts>(0, 21, false)
        .map(|(token, _)| token);
    let input = source(21);
    prepared_null(Phase::Encode, ordinal, || {
        encoder.prepare_bytes(input, false).map(drop)
    })?;
    assert_eq!(
        baseline,
        encoder
            .comparison_baseline::<tables::Fronts>(0, 21, false)
            .map(|(token, _)| token),
        "failed or uncommitted encoding preserves the baseline"
    );
    Ok(())
}

struct Exercise {
    witness: Option<replay::witness::TerminalWitness>,
    disk_witness: Option<replay::witness::TerminalWitness>,
    phase: Phase,
    #[cfg(feature = "alloc-fault-injection")]
    checked: u64,
    original_calls: usize,
    callback_calls: usize,
    last_attempted_hour: i32,
    attempted_hour_committed: bool,
    conversion_matched: bool,
    publication_intact: bool,
    admitted_after_failure: bool,
    incomplete_published: bool,
    bootstrap_refused: bool,
    second_failure_unchanged: bool,
    prefix_hours: usize,
    reached: bool,
}

fn case_root(phase: Phase, ordinal: usize, checked: bool) -> PathBuf {
    // Windows reuses child PIDs across runs and case modes.
    PathBuf::from(std::env::var_os("BATTLE_PLAN_COLLECTOR_ALLOCATION_RUN").or_else(|| std::env::var_os("HOI4_COLLECTOR_ALLOCATION_RUN")).unwrap())
        .join(std::env::var_os("BATTLE_PLAN_COLLECTOR_ALLOCATION_TEST").or_else(|| std::env::var_os("HOI4_COLLECTOR_ALLOCATION_TEST")).unwrap())
        .join(format!(
            "{}-{phase:?}-{ordinal}-{checked}",
            std::process::id()
        ))
}

fn context(hour: Option<i32>) {
    allocation::set_context(Context {
        campaign_id: hour.map(|_| *CAMPAIGN),
        branch: hour.map(|_| 0),
        load_signal: hour.map(|_| 1234),
        failed_hour: hour,
    });
}

fn read_witness(file: &File) -> io::Result<Option<replay::witness::TerminalWitness>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = file;
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = [0; replay::witness::WITNESS_BYTES];
    let mut len = 0;
    while len < bytes.len() {
        let read = file.read(&mut bytes[len..])?;
        if read == 0 {
            break;
        }
        len += read;
    }
    if len == 0 {
        return Ok(None);
    }
    if replay::witness::TerminalWitness::decode_prepared(&bytes[..len]).is_ok() {
        return Ok(None);
    }
    replay::witness::TerminalWitness::decode(&bytes[..len]).map(Some)
}

fn validate(outcome: &Exercise, failed: bool) {
    assert_eq!(outcome.witness.is_some(), failed);
    assert_eq!(outcome.witness, outcome.disk_witness);
    assert_eq!(
        outcome.original_calls,
        outcome.callback_calls + 1,
        "the original game tick always runs"
    );
    assert!(!outcome.admitted_after_failure);
    assert!(
        !outcome.incomplete_published,
        "failed decode did not publish a complete frame"
    );
    assert!(
        outcome.conversion_matched,
        "populated codec conversions retain exact values"
    );
    assert!(
        outcome.publication_intact,
        "terminal failure preserves the encoder and committed watermark"
    );
    assert!(outcome.second_failure_unchanged);
    assert!(
        outcome.reached,
        "synthetic pipeline did not enter the selected phase"
    );
    assert!(
        outcome.prefix_hours >= 2,
        "completed prefix remains recoverable"
    );
    if outcome.phase == Phase::Bootstrap {
        assert_eq!(outcome.bootstrap_refused, failed);
    }
    if let Some(witness) = outcome.witness {
        assert_eq!(witness.phase, outcome.phase as u8);
        assert_eq!(witness.code, Code::Reserve as u8);
        assert_eq!(witness.session, 1);
        assert_eq!(witness.process_start_time, 1000);
        assert_eq!(witness.committed_campaign_id, Some(*CAMPAIGN));
        assert!(witness.committed_segment.is_some());
        assert!(witness.committed_bytes > 0);
        match outcome.phase {
            Phase::Bootstrap | Phase::Status | Phase::Teardown => {
                assert_eq!(witness.failed_hour, None)
            }
            _ => assert_eq!(
                witness.failed_hour,
                (!outcome.attempted_hour_committed).then_some(outcome.last_attempted_hour)
            ),
        }
    }
}

fn minimal_sizing() -> crate::country_scale::Sizing {
    // Sizing contains only integer capacity fields. The synthetic source has
    // no content roster; zero demand keeps repeated fault sweeps small.
    unsafe { std::mem::zeroed() }
}

fn exercise_pipeline(
    phase: Phase,
    ordinal: usize,
    changed_loads: bool,
    fail_publication: bool,
) -> io::Result<Exercise> {
    allocation::reset();
    allocation::install_hooks();
    let root = case_root(phase, ordinal, true);
    fs::create_dir_all(&root)?;
    let witness_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(root.join(replay::witness::WITNESS_FILE_NAME))?;
    let shared = crate::sampling::Shared::with_sizing(0, 0, 1, &minimal_sizing())?;
    let mut processor = crate::spooling::spool_failure_fixture(&root)?;
    let mut frame = crate::spooling::allocation_failure_frame(19);
    if changed_loads {
        frame.clock.as_mut().unwrap().play_session_started_at = 19;
    }
    processor.prepare(&frame)?;
    processor.frame(&frame)?;
    if let Some(clock) = &mut frame.clock {
        clock.hour = 20;
        if changed_loads {
            clock.play_session_started_at = 20;
        }
    }
    processor.prepare(&frame)?;
    processor.frame(&frame)?;
    let preceding_publication = processor.allocation_failure_publication_state(20);
    let bytes = crate::sampling::divisions_test_kit::fixture();
    let mut divisions = crate::sampling::DivisionsJob::new();
    if let Some(clock) = &mut frame.clock {
        clock.hour = 21;
    }
    let index = shared.take_frame(&mut 0).ok_or(io::ErrorKind::Other)?;
    let mut status_probe = if phase == Phase::Status {
        Some(crate::status_writer::AllocationProbe::prepare(&root)?)
    } else {
        None
    };
    let mut lexical_zero = (phase == Phase::Decode).then(|| {
        crate::extended::lexical::lexical_test_kit::allocation_failure_decode_exercise(0, 0)
    });
    let mut lexical_short = (phase == Phase::Decode).then(|| {
        crate::extended::lexical::lexical_test_kit::allocation_failure_decode_exercise(1, 2)
    });
    let mut roster = if matches!(phase, Phase::Spool | Phase::Encode | Phase::Map) {
        Some(crate::roster::allocation_failure_fixture())
    } else {
        None
    };
    let mut governance = if matches!(phase, Phase::Spool | Phase::Encode | Phase::Map) {
        Some(crate::extended::governance::allocation_failure_fixture())
    } else {
        None
    };
    let front = replay::rows::FrontRow {
        front: 1,
        section_ids: vec![7, 8],
        section_ends: vec![2, 4],
        province_pairs: vec![3, 4, 5, 6],
    };
    let mut front_expected = Vec::new();
    front.encode_change(None, 0, &mut front_expected)?;
    allocation::prepare(&witness_file, 1, 1000);
    let idle = matches!(phase, Phase::Bootstrap | Phase::Status | Phase::Teardown);
    context((!idle).then_some(21));
    #[cfg(feature = "alloc-fault-injection")]
    allocation::inject(phase, ordinal as u64);
    TARGET_PHASE.store(phase as u8, Ordering::Relaxed);
    REACHED.store(false, Ordering::Relaxed);
    FAIL_PUBLICATION.set(fail_publication);
    let mut original_calls = 0;
    let mut conversion_matched = true;
    let mut callback_calls = 0;
    let mut last_attempted_hour = 21;
    let mut bootstrap_refused = false;
    if phase == Phase::Bootstrap {
        let _scope = allocation::enter(Phase::Bootstrap);
        let before = crate::sampling::SHARED.load(Ordering::Acquire);
        bootstrap_refused =
            crate::sampling::Shared::with_sizing(0, 0, 1, &minimal_sizing()).is_err();
        assert!(before.is_null());
        assert_eq!(
            before,
            crate::sampling::SHARED.load(Ordering::Acquire),
            "failed bootstrap must not publish hook state"
        );
    }
    let final_hour = if matches!(phase, Phase::Spool | Phase::Encode | Phase::Map) {
        23
    } else {
        21
    };
    for hour in 21..=final_hour {
        if let Some(clock) = &mut frame.clock {
            clock.hour = hour;
            if changed_loads {
                clock.play_session_started_at = i64::from(hour);
            }
        }
        last_attempted_hour = hour;
        callback_calls += 1;
        // The fixture campaign is fixed; copy only clock scalars before the mutable frame lease.
        let entry = if idle {
            None
        } else {
            frame
                .clock
                .as_ref()
                .map(|clock| clock.with_campaign_id(CAMPAIGN.as_slice()))
        };
        unsafe {
            crate::sampling::synthetic_callback(
                &shared,
                entry,
                || {
                    match phase {
                        Phase::Bootstrap => {}
                        Phase::Callback | Phase::Acquisition => {
                            let _scope = allocation::enter(phase);
                            // These use the same retained synthetic native census as
                            // existing primary-copy/decode equivalence tests.
                            crate::sampling::copy_divisions_refresh(
                                &crate::sampling::divisions_test_kit::memory(&bytes),
                                crate::sampling::DivisionsCopyContext {
                                    base: crate::sampling::divisions_test_kit::BASE,
                                    game_state: crate::sampling::divisions_test_kit::GAME_STATE,
                                    ..Default::default()
                                },
                                &mut divisions,
                            );
                        }
                        Phase::Raw => {
                            let _scope = allocation::enter(Phase::Raw);
                            crate::raw::allocation_exercise();
                            if !allocation::failed() {
                                crate::snapshot_plan::allocation_failure_exercise();
                            }
                        }
                        Phase::Decode => {
                            let _scope = allocation::enter(Phase::Decode);
                            if let Some(decode) = lexical_zero.take() {
                                conversion_matched &= decode();
                            }
                            if !allocation::failed()
                                && let Some(decode) = lexical_short.take()
                            {
                                conversion_matched &= decode();
                            }
                            if allocation::failed() {
                                return;
                            }
                            divisions.grow();
                            if !allocation::failed() {
                                crate::sampling::decode_divisions_payload(
                                    &mut frame,
                                    0,
                                    &mut divisions,
                                    &mut [0; 6],
                                );
                                crate::decode::allocation_exercise(&shared, index, hour);
                            }
                        }
                        Phase::Extended => {
                            crate::extended::combat_details::allocation_failure_exercise(
                                || {},
                                || {},
                            );
                            let _scope = allocation::enter(Phase::Extended);
                            if !allocation::failed() {
                                crate::extended::air_histories::allocation_failure_exercise();
                            }
                            if !allocation::failed() {
                                crate::extended::air_relations::allocation_failure_exercise();
                            }
                            if !allocation::failed() {
                                crate::extended::character_inputs::allocation_failure_exercise();
                            }
                            let registry = crate::watch_registry::WatchRegistry::new();
                            if !allocation::failed() {
                                registry.offer(32);
                            }
                            if !allocation::failed() {
                                let mut capacity = crate::extended::Capacities::default();
                                frame.extended.grow(&mut capacity);
                            }
                            registry.release();
                        }
                        Phase::Spool | Phase::Encode | Phase::Map => {
                            crate::extended::output_allocation_failure_exercise(|| {}, || {});
                            let _scope = allocation::enter(Phase::Spool);
                            if !allocation::failed()
                                && let Some(verify) = roster.take()
                            {
                                let _ = verify();
                            }
                            if !allocation::failed()
                                && let Some(convert) = governance.take()
                            {
                                conversion_matched &= convert();
                            }
                            if !allocation::failed() {
                                let _ = crate::fixed_layout::allocation_failure_exercise();
                            }
                            if !allocation::failed() {
                                let _ = front.check_key_fields();
                            }
                            if !allocation::failed() {
                                let _encode = allocation::enter(Phase::Encode);
                                let mut encoded = Vec::new();
                                if front.encode_change(None, 0, &mut encoded).is_ok() {
                                    conversion_matched &= encoded == front_expected;
                                }
                            }
                            if !allocation::failed() {
                                let _ = processor
                                    .prepare(&frame)
                                    .and_then(|()| processor.frame(&frame));
                            }
                        }
                        Phase::Status => {
                            let _scope = allocation::enter(Phase::Status);
                            allocation::attempted_hour(None);
                            if let Ok(status) = processor.status() {
                                let _ = crate::status::serialize(&status);
                            }
                            if !allocation::failed()
                                && let Some(mut probe) = status_probe.take()
                            {
                                let _ = probe.run();
                            }
                        }
                        Phase::Teardown => {
                            let _scope = allocation::enter(Phase::Teardown);
                            allocation::attempted_hour(None);
                            let _ = processor.detach();
                        }
                    }
                },
                || {
                    original_calls += 1;
                },
            );
        }
        if allocation::failed() {
            break;
        }
    }
    let first = allocation::failure();
    let failed = allocation::failed();
    let publication_intact = !fail_publication
        || (preceding_publication.is_some()
            && processor.allocation_failure_publication_state(20) == preceding_publication
            && first.is_some_and(|witness| {
                preceding_publication.is_some_and(|(_, (segment, bytes))| {
                    witness.committed_segment == Some(segment) && witness.committed_bytes == bytes
                })
            }));
    let incomplete_published = failed
        && phase == Phase::Decode
        && (shared.completed[index].load(Ordering::Acquire) != 0 || !shared.ready.is_empty());
    let mut admitted_after_failure = false;
    unsafe {
        crate::sampling::synthetic_callback(
            &shared,
            frame
                .clock
                .as_ref()
                .map(engine::cgame_state::Clock::borrowed)
                .map(|mut clock| {
                    clock.hour = last_attempted_hour + 1;
                    clock
                }),
            || {
                if failed {
                    admitted_after_failure = true;
                }
            },
            || {
                original_calls += 1;
            },
        );
    }
    #[cfg(feature = "alloc-fault-injection")]
    let allocations_before_cleanup = allocation::allocation_count();
    if failed {
        let _scope = allocation::enter(Phase::Teardown);
        // Teardown refuses checked growth after failure, and a competing
        // terminal reason cannot replace the already published witness.
        let _ = engine::allocation::vec::<u8>(1);
        allocation::fail(Phase::Teardown, Code::Thread);
    }
    {
        let _scope = allocation::enter(Phase::Teardown);
        drop(status_probe);
        drop(lexical_zero);
        drop(lexical_short);
        drop(roster);
        drop(governance);
        drop(processor);
        drop(shared);
        drop(divisions);
        drop(frame);
        drop(front);
        drop(front_expected);
    }
    #[cfg(feature = "alloc-fault-injection")]
    let allocations = if failed {
        allocations_before_cleanup
    } else {
        allocation::allocation_count()
    };
    let second_failure_unchanged = first == allocation::failure();
    let reached = REACHED.load(Ordering::Relaxed);
    let disk_witness = read_witness(&witness_file)?;
    allocation::clear_handle();
    #[cfg(feature = "alloc-fault-injection")]
    allocation::inject(Phase::Bootstrap, u64::MAX);
    // Recovery runs outside the simulated game-owned allocation interval.
    // Reader allocations must no longer observe the stopped collector state.
    allocation::reset();
    let (prefix_hours, attempted_hour_committed) =
        recovered_hours(&root, last_attempted_hour, first.as_ref())?;
    drop(witness_file);
    if failed {
        finalize_with_recorder(&root)?;
    }
    Ok(Exercise {
        witness: first,
        disk_witness,
        phase,
        #[cfg(feature = "alloc-fault-injection")]
        checked: allocations,
        original_calls,
        callback_calls,
        last_attempted_hour,
        attempted_hour_committed,
        conversion_matched,
        publication_intact,
        admitted_after_failure,
        incomplete_published,
        bootstrap_refused,
        second_failure_unchanged,
        prefix_hours,
        reached,
    })
}

fn finalize_with_recorder(root: &Path) -> io::Result<()> {
    let executable = std::env::var_os("battle-plan-recorder_TEST_EXE").expect(
        "set battle-plan-recorder_TEST_EXE to the exact recorder test executable built for this head",
    );
    let output = Command::new(executable)
        .args([
            "--ignored",
            "--exact",
            "replay_io::tests::allocation_fixture_finalization_child",
            "--test-threads=1",
        ])
        .env("BATTLE_PLAN_ALLOCATION_FIXTURE", root)
        .output()?;
    successful(&output, "recorder-finalization", "verified-prefix", 0);
    Ok(())
}

fn recovered_hours(
    root: &Path,
    attempted_hour: i32,
    witness: Option<&replay::witness::TerminalWitness>,
) -> io::Result<(usize, bool)> {
    let campaign = root.join(std::str::from_utf8(CAMPAIGN).unwrap());
    let mut hours = 0;
    let mut attempted_committed = false;
    for entry in fs::read_dir(campaign)? {
        let entry = entry?;
        let path = entry.path();
        if !path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("spool."))
        {
            continue;
        }
        let bytes = fs::read(path)?;
        let mut prefix = bytes.as_slice();
        let mut exact_cap = false;
        if let Some(witness) = witness {
            let number = entry
                .file_name()
                .to_str()
                .and_then(|name| name.strip_prefix("spool."))
                .and_then(|name| name.strip_suffix(".h4s"))
                .and_then(|number| number.parse::<u32>().ok())
                .ok_or(io::ErrorKind::InvalidData)?;
            for (id, segment, committed_bytes) in [
                (
                    witness.committed_campaign_id,
                    witness.committed_segment,
                    witness.committed_bytes,
                ),
                (
                    witness.pending_campaign_id,
                    witness.pending_segment,
                    witness.pending_bytes,
                ),
            ] {
                if id != Some(*CAMPAIGN) {
                    continue;
                }
                match segment {
                    Some(segment) if number < segment => {}
                    Some(segment) if number == segment => {
                        let end = usize::try_from(committed_bytes)
                            .map_err(|_| io::ErrorKind::InvalidData)?;
                        if end > bytes.len() {
                            return Err(io::ErrorKind::InvalidData.into());
                        }
                        prefix = &prefix[..prefix.len().min(end)];
                        exact_cap = true;
                    }
                    _ => prefix = &[],
                }
            }
        }
        let recovered = replay::spool::recover_prefix(prefix);
        if exact_cap && recovered.valid_bytes != prefix.len() {
            return Err(io::ErrorKind::InvalidData.into());
        }
        for record in recovered.records {
            if let replay::spool::Record::Frame(frame) = record {
                hours += 1;
                attempted_committed |= frame.hour == attempted_hour;
            }
        }
    }
    Ok((hours, attempted_committed))
}
