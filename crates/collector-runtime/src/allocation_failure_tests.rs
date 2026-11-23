use collector_memory::terminal::{Code, Context, Phase};
use collector_test_support::with_allocations_denied;
use collector_test_support::{ATTEMPTS, DENIALS, NullGuard, REACHED, TARGET_PHASE};
use replay_data::rows::RowCodec;
use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

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
        .env("BATTLEPLAN_COLLECTOR_ALLOCATION_TEST", mode)
        .env("BATTLEPLAN_COLLECTOR_ALLOCATION_RUN", &run)
        .env("BATTLEPLAN_COLLECTOR_ALLOCATION_PHASE", phase)
        .env(
            "BATTLEPLAN_COLLECTOR_ALLOCATION_ORDINAL",
            ordinal.to_string(),
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
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
#[cfg(feature = "alloc-fault-injection")]
fn allocation_sites(output: &Output) -> Vec<u64> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| {
            line.split_once("collector-allocation-sites=")
                .map(|(_, sites)| {
                    sites
                        .split(',')
                        .map(|site| u64::from_str_radix(site, 16).unwrap())
                        .collect()
                })
        })
        .expect("child allocation site marker")
}
#[cfg(feature = "alloc-fault-injection")]
static SITES: std::sync::Mutex<Vec<u64>> = std::sync::Mutex::new(Vec::new());
#[cfg(feature = "alloc-fault-injection")]
fn record_site() {
    use windows_sys::Win32::System::{
        Diagnostics::Debug::RtlCaptureStackBackTrace,
        LibraryLoader::{
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            GetModuleHandleExW,
        },
    };
    let mut frames = [std::ptr::null_mut(); 62];
    let captured = unsafe {
        RtlCaptureStackBackTrace(
            1,
            frames.len() as u32,
            frames.as_mut_ptr(),
            std::ptr::null_mut(),
        )
    };
    let site =
        frames[..usize::from(captured)]
            .iter()
            .fold(0xcbf2_9ce4_8422_2325_u64, |site, &frame| {
                let mut module = std::ptr::null_mut();
                let found = unsafe {
                    GetModuleHandleExW(
                        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
                            | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                        frame.cast_const().cast(),
                        &mut module,
                    )
                };
                let base = if found != 0 { module as usize } else { 0 };
                (site ^ (frame as usize).wrapping_sub(base) as u64).wrapping_mul(0x0100_0000_01b3)
            });
    SITES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(site);
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
fn a_refused_frame_allocation_names_its_hour_and_stops_recording() {
    let output = child("frame-growth-refused", "callback", usize::MAX);
    successful(&output, "frame-growth-refused", "callback", usize::MAX);
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("collector-frame-growth-refused=ok"),
        "the refused frame allocation did not reach its checks"
    );
}

#[test]
fn every_refused_allocation_of_a_new_frame_names_its_hour_and_never_aborts() {
    let output = child("frame-build-refused", "callback", usize::MAX);
    successful(&output, "frame-build-refused", "callback", usize::MAX);
    assert!(observed_allocations(&output) > 0, "a new frame allocates");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("collector-frame-build-refused=ok"),
        "the refused frame builds did not reach their checks"
    );
}

#[test]
fn every_refused_allocation_of_attach_names_the_failure_and_never_aborts() {
    let output = child("attach-refused", "bootstrap", usize::MAX);
    successful(&output, "attach-refused", "bootstrap", usize::MAX);
    assert!(observed_allocations(&output) > 0, "attach allocates");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("collector-attach-refused=ok"),
        "the refused attach did not reach its checks"
    );
}

#[test]
fn every_refused_allocation_of_the_payload_pools_is_named_and_never_aborts() {
    let output = child("payload-pools-refused", "bootstrap", usize::MAX);
    successful(&output, "payload-pools-refused", "bootstrap", usize::MAX);
    assert!(
        observed_allocations(&output) > 0,
        "the payload pools allocate"
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("collector-payload-pools-refused=ok"),
        "the refused payload pools did not reach their checks"
    );
}

#[cfg(feature = "alloc-fault-injection")]
#[test]
fn a_refused_decode_allocation_costs_its_tables_for_the_hour_and_capture_goes_on() {
    let baseline = child("decode-refused", "decode", usize::MAX);
    successful(&baseline, "decode-refused", "decode", usize::MAX);
    let count = observed_allocations(&baseline);
    assert!(count > 0, "the hour's decodes allocate");
    let mut contained = 0;
    for ordinal in 1..=count {
        let output = child("decode-refused", "decode", ordinal);
        successful(&output, "decode-refused", "decode", ordinal);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("collector-decode-refused="),
            "ordinal {ordinal} did not reach its checks"
        );
        contained += usize::from(stdout.contains("collector-decode-refused=contained"));
    }
    assert!(contained > 0, "no refused decode allocation was contained");
}

#[cfg(feature = "alloc-fault-injection")]
fn decode_refused(ordinal: usize) {
    use engine_native_memory::walk::Outcome;
    let by_name = Outcome::Unavailable {
        reason: "capture buffer allocation failed",
    };
    collector_memory::terminal::reset();
    collector_memory::terminal::install_hooks();
    let decode_work = |ordinal: u64| {
        move || collector_memory::terminal::inject_any(&[Phase::Decode, Phase::Raw], ordinal)
    };
    let shared =
        // SAFETY: This is unpublished attach or isolated fixture state, with no competing observer
        // boundary or detach.
        unsafe { collector_capture::sampling::Shared::with_sizing(0, 0, 1, &minimal_sizing()) }.unwrap();
    let index =
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { shared.take_frame(&mut 0) }.unwrap();
    let baseline =
        // SAFETY: This fixture owns the claimed frame and process-wide refusal state.
        unsafe { collector_capture::decode::refusal_exercise(&shared, index, 20, decode_work(u64::MAX)) };
    assert!(!baseline.terminal && baseline.published, "{baseline:?}");
    assert!(!baseline.projects.contains(&by_name) && baseline.capitulation != by_name);
    let refused =
    // SAFETY: This fixture owns the claimed frame and process-wide refusal state.
    unsafe { collector_capture::decode::refusal_exercise(
        &shared,
        index,
        21,
        decode_work(ordinal as u64),
    ) };
    if ordinal == usize::MAX {
        assert_eq!(refused.projects, baseline.projects);
        assert_eq!(refused.capitulation, baseline.capitulation);
        println!(
            "collector-allocation-count={}",
            collector_memory::terminal::allocation_count()
        );
        return;
    }
    if refused.terminal {
        assert!(!refused.published, "a terminal hour is never published");
        assert!(collector_memory::terminal::failure().is_some());
        println!("collector-decode-refused=terminal");
        return;
    }
    assert!(
        refused.published,
        "a contained refusal publishes its hour: {refused:?}"
    );
    assert!(collector_memory::terminal::failure().is_none());
    let projects = refused.projects.iter().all(|outcome| *outcome == by_name);
    let capitulation = refused.capitulation == by_name;
    assert!(
        projects != capitulation,
        "exactly one decode is refused: {refused:?}"
    );
    if projects {
        assert_eq!(refused.capitulation, baseline.capitulation);
    } else {
        assert_eq!(refused.projects, baseline.projects);
    }
    let next =
        // SAFETY: This fixture owns the claimed frame and process-wide refusal state.
        unsafe { collector_capture::decode::refusal_exercise(&shared, index, 22, decode_work(u64::MAX)) };
    assert!(!next.terminal && next.published, "{next:?}");
    assert_eq!(next.projects, baseline.projects);
    assert_eq!(next.capitulation, baseline.capitulation);
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { shared.recycle_frame(index) };
    println!("collector-decode-refused=contained");
}

fn payload_pools_refused() {
    collector_memory::terminal::reset();
    collector_memory::terminal::install_hooks();
    drop(
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { collector_capture::decode::prepare_payload_pools(true) }.unwrap(),
    );
    let prepare = |ordinal: usize| {
        let scope = collector_memory::terminal::enter(Phase::Bootstrap);
        let guard = NullGuard::new(ordinal);
        let prepared =
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { collector_capture::decode::prepare_payload_pools(true) };
        let attempts = ATTEMPTS.load(Ordering::Relaxed);
        let denials = DENIALS.load(Ordering::Relaxed);
        drop(guard);
        drop(scope);
        (
            prepared.map(drop).map_err(|error| error.kind()),
            attempts,
            denials,
        )
    };
    let (prepared, total, _) = prepare(usize::MAX);
    assert_eq!(prepared, Ok(()));
    assert!(!collector_memory::terminal::failed());
    let first = total.div_ceil(3);
    let spread = (first + 1..=total)
        .step_by(total / 64 + 1)
        .chain(Some(total));
    for ordinal in (1..=first).chain(spread) {
        collector_memory::terminal::reset();
        collector_memory::terminal::install_hooks();
        let (prepared, _, denials) = prepare(ordinal);
        assert_eq!(
            prepared,
            Err(io::ErrorKind::OutOfMemory),
            "allocation {ordinal} of {total}"
        );
        assert!(
            denials > 0,
            "allocation {ordinal} of {total} was never asked for"
        );
        let witness = collector_memory::terminal::failure()
            .unwrap_or_else(|| panic!("allocation {ordinal} of {total} named no failure"));
        assert_eq!(
            (witness.phase, witness.code),
            (Phase::Bootstrap as u8, Code::Reserve as u8),
            "allocation {ordinal} of {total}"
        );
    }
    collector_memory::terminal::clear_handle();
    println!("collector-allocation-count={total}");
    println!("collector-payload-pools-refused=ok");
}

fn frame_build_refused() {
    collector_memory::terminal::reset();
    collector_memory::terminal::install_hooks();
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { collector_capture::sampling::Shared::new(0, 0, 1) };
    let held: Vec<_> = (0..collector_capture::frame_pool::STEADY)
        .map(|_|
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { shared.take_frame(&mut 0) }.unwrap())
        .collect();
    let extra =
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { shared.take_frame(&mut 0) }.unwrap();
    drop(
        // SAFETY: The caller owns the installed slot and all other frame users are finished.
        unsafe { shared.frames().vacate(extra) },
    );
    let context = Context {
        campaign_id: Some(*CAMPAIGN),
        branch: None,
        load_signal: Some(7),
        failed_hour: Some(41),
    };
    let built = {
        let _scope = collector_memory::terminal::enter(Phase::Callback);
        collector_memory::terminal::set_context(context);
        let guard = NullGuard::new(usize::MAX);
        let built =
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { shared.take_frame(&mut 0) };
        let count = ATTEMPTS.load(Ordering::Relaxed);
        drop(guard);
        drop(
            // SAFETY: The caller owns the installed slot and all other frame users are finished.
            unsafe { shared.frames().vacate(built.expect("an unrefused frame")) },
        );
        count
    };
    assert!(!collector_memory::terminal::failed());
    let live =
        // SAFETY: This is the runtime's frame pool for the current session.
        unsafe { shared.frames() }.live();
    let grown =
        // SAFETY: This is the runtime's frame pool for the current session.
        unsafe { shared.frames() }.gauge().grown;
    for ordinal in 1..=built {
        collector_memory::terminal::reset();
        collector_memory::terminal::install_hooks();
        let scope = collector_memory::terminal::enter(Phase::Callback);
        collector_memory::terminal::set_context(context);
        let mut grow_ns = u64::MAX;
        let guard = NullGuard::new(ordinal);
        let refused =
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { shared.take_frame(&mut grow_ns) };
        let denials = DENIALS.load(Ordering::Relaxed);
        drop(guard);
        drop(scope);
        assert_eq!(
            (refused, grow_ns),
            (None, 0),
            "allocation {ordinal} of {built}"
        );
        assert!(
            denials > 0,
            "allocation {ordinal} of {built} was never asked for"
        );
        let witness = collector_memory::terminal::failure()
            .unwrap_or_else(|| panic!("allocation {ordinal} of {built} named no failure"));
        assert_eq!(
            (witness.phase, witness.code, witness.failed_hour),
            (Phase::Callback as u8, Code::Reserve as u8, Some(41)),
            "allocation {ordinal} of {built}"
        );
        assert_eq!(
            // SAFETY: This code uses the frame pool belonging to its own recording or fixture.
            unsafe { shared.frames() }.live(),
            live
        );
        assert_eq!(
            // SAFETY: This code uses the frame pool belonging to its own recording or fixture.
            unsafe { shared.frames() }.gauge().grown,
            grown
        );
    }
    let complete = unsafe {
        collector_capture::sampling::synthetic_callback(
            &shared,
            None,
            || panic!("no acquisition after a refused frame"),
            || {},
        )
    };
    assert!(!complete);
    assert!(!
        // SAFETY: This is the owning runtime's stop or an isolated fixture's recording flag.
        unsafe { shared.recording() }.load(Ordering::Acquire));
    assert_eq!(
        // SAFETY: The spool or fixture reads the count maintained by its own capture.
        unsafe { shared.dropped() }.load(Ordering::Relaxed),
        0
    );
    assert!(
        // SAFETY: The isolated fixture owns this loss state and no other thread can access it.
        unsafe { &*shared.pending().get() }.iter().next().is_none()
    );
    for index in held {
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { shared.recycle_frame(index) };
    }
    collector_memory::terminal::clear_handle();
    println!("collector-allocation-count={built}");
    println!("collector-frame-build-refused=ok");
}

fn attach_refused() {
    collector_memory::terminal::reset();
    collector_memory::terminal::install_hooks();
    let sizing = collector_capture::country_scale::Sizing::for_slots(0);
    let unbind = || unsafe {
        collector_observers::doctrine_changes::bind(0, collector_slow_tables::doctrines::observe);
    };
    drop(
        // SAFETY: This is unpublished attach or isolated fixture state, with no competing observer
        // boundary or detach.
        unsafe { collector_capture::sampling::Shared::with_sizing(0, 0, 1, &sizing) }.unwrap(),
    );
    let stores = {
        let guard = NullGuard::new(usize::MAX);
        drop(
            // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
            unsafe { collector_capture::sampling::Shared::without_frames(0, 0, 1, &sizing) }
                .unwrap(),
        );
        let stores = ATTEMPTS.load(Ordering::Relaxed);
        drop(guard);
        stores
    };
    let attach = |ordinal: usize| {
        unbind();
        let scope = collector_memory::terminal::enter(Phase::Bootstrap);
        let guard = NullGuard::new(ordinal);
        let attached =
        // SAFETY: This is unpublished attach or isolated fixture state, with no competing observer
        // boundary or detach.
        unsafe { collector_capture::sampling::Shared::with_sizing(0, 0, 1, &sizing) };
        let attempts = ATTEMPTS.load(Ordering::Relaxed);
        let denials = DENIALS.load(Ordering::Relaxed);
        drop(guard);
        drop(scope);
        let bound = collector_observers::doctrine_changes::bound_context();
        (
            attached.map(drop).map_err(|error| error.kind()),
            attempts,
            denials,
            bound,
        )
    };
    let (attached, total, _, bound) = attach(usize::MAX);
    assert_eq!(attached, Ok(()));
    assert!(!collector_memory::terminal::failed());
    assert_ne!(bound, 0, "a whole attach binds the doctrine observer");
    let first_frame = stores + (total - stores) / collector_capture::frame_pool::STEADY;
    let spread = (first_frame + 1..=total)
        .step_by(total / 64 + 1)
        .chain(Some(total));
    for ordinal in (1..=first_frame).chain(spread) {
        collector_memory::terminal::reset();
        collector_memory::terminal::install_hooks();
        let (attached, _, denials, bound) = attach(ordinal);
        assert_eq!(
            attached,
            Err(io::ErrorKind::OutOfMemory),
            "allocation {ordinal} of {total}"
        );
        assert!(
            denials > 0,
            "allocation {ordinal} of {total} was never asked for"
        );
        let witness = collector_memory::terminal::failure()
            .unwrap_or_else(|| panic!("allocation {ordinal} of {total} named no failure"));
        assert_eq!(
            (witness.phase, witness.code),
            (Phase::Bootstrap as u8, Code::Reserve as u8),
            "allocation {ordinal} of {total}"
        );
        assert_eq!(
            bound, 0,
            "allocation {ordinal} of {total} left the observer bound"
        );
    }
    collector_memory::terminal::clear_handle();
    println!("collector-allocation-count={total}");
    println!("collector-attach-refused=ok");
}

fn frame_growth_refused() {
    collector_memory::terminal::reset();
    collector_memory::terminal::install_hooks();
    let shared =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { collector_capture::sampling::Shared::new(0, 0, 1) };
    let held: Vec<_> = (0..collector_capture::frame_pool::STEADY)
        .map(|_|
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { shared.take_frame(&mut 0) }.unwrap())
        .collect();
    let scope = collector_memory::terminal::enter(Phase::Callback);
    collector_memory::terminal::set_context(Context {
        campaign_id: Some(*CAMPAIGN),
        branch: None,
        load_signal: Some(7),
        failed_hour: Some(41),
    });
    let mut grow_ns = u64::MAX;
    let refused = with_allocations_denied(||
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { shared.take_frame(&mut grow_ns) });
    drop(scope);
    assert_eq!(refused, None);
    assert_eq!(grow_ns, 0);
    let witness =
        collector_memory::terminal::failure().expect("a refused frame is the allocation failure");
    assert_eq!(witness.phase, Phase::Callback as u8);
    assert_eq!(witness.code, Code::Reserve as u8);
    assert_eq!(witness.failed_hour, Some(41));
    assert_eq!(
        // SAFETY: This code uses the frame pool belonging to its own recording or fixture.
        unsafe { shared.frames() }.live(),
        collector_capture::frame_pool::STEADY
    );
    assert_eq!(
        // SAFETY: This code uses the frame pool belonging to its own recording or fixture.
        unsafe { shared.frames() }.gauge().grown,
        0
    );
    let complete = unsafe {
        collector_capture::sampling::synthetic_callback(
            &shared,
            None,
            || panic!("no acquisition after a refused frame"),
            || {},
        )
    };
    assert!(!complete);
    assert!(!
        // SAFETY: This is the owning runtime's stop or an isolated fixture's recording flag.
        unsafe { shared.recording() }.load(Ordering::Acquire));
    assert_eq!(
        // SAFETY: The spool or fixture reads the count maintained by its own capture.
        unsafe { shared.dropped() }.load(Ordering::Relaxed),
        0
    );
    assert!(
        // SAFETY: The isolated fixture owns this loss state and no other thread can access it.
        unsafe { &*shared.pending().get() }.iter().next().is_none()
    );
    for index in held {
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { shared.recycle_frame(index) };
    }
    collector_memory::terminal::clear_handle();
    println!("collector-frame-growth-refused=ok");
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
fn cpu_set_first_use_failures_are_advisory_and_keep_other_policy() {
    let baseline = child("cpu-sets", "bootstrap", usize::MAX);
    successful(&baseline, "cpu-sets", "bootstrap", usize::MAX);
    let count = observed_allocations(&baseline);
    assert_eq!(
        count, 4,
        "query buffer, parsed sets, core ranks and selected ids"
    );
    for ordinal in 1..=count {
        let output = child("cpu-sets", "bootstrap", ordinal);
        successful(&output, "cpu-sets", "bootstrap", ordinal);
        assert_eq!(observed_allocations(&output), ordinal);
    }
    for mode in ["cpu-sets-size-failed", "cpu-sets-read-failed"] {
        let output = child(mode, "bootstrap", usize::MAX);
        successful(&output, mode, "bootstrap", usize::MAX);
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

#[cfg(feature = "alloc-fault-injection")]
#[test]
fn the_spool_sweep_counts_the_same_allocations_in_the_same_order_every_run() {
    for mode in ["sites", "sites-load"] {
        let [first, second] = [0, 1].map(|_| {
            let output = child(mode, "spool", usize::MAX);
            successful(&output, mode, "spool", usize::MAX);
            allocation_sites(&output)
        });
        if let Some(index) = first.iter().zip(&second).position(|(a, b)| a != b) {
            panic!(
                "{mode}: spool ordinal {} is another allocation on a second run",
                index + 1
            );
        }
        assert_eq!(
            first.len(),
            second.len(),
            "{mode}: the runs counted differently"
        );
    }
}

#[test]
fn a_refused_allocation_on_a_collector_thread_names_its_own_phase() {
    for phase in ["spool-helper", "spool", "journal", "hook"] {
        if phase == "journal" && !cfg!(feature = "alloc-fault-injection") {
            continue;
        }
        let output = child("thread-label", phase, 1);
        successful(&output, "thread-label", phase, 1);
    }
}

#[test]
fn a_campaign_set_aside_with_a_pending_gap_commits_under_its_own_name() {
    let output = child("set-aside", "spool", 0);
    successful(&output, "set-aside", "spool", 0);
}

#[test]
#[ignore = "launched only by allocation-failure parent tests"]
fn allocation_safety_child() {
    synthetic_game::Executable::fixed_layout(engine_fixed_layout::fixed_layout::TOKEN_IDS).enter();
    let mode = std::env::var("BATTLEPLAN_COLLECTOR_ALLOCATION_TEST")
        .or_else(|_| std::env::var("HOI4_COLLECTOR_ALLOCATION_TEST"))
        .unwrap();
    let phase = Phase::parse(
        &std::env::var("BATTLEPLAN_COLLECTOR_ALLOCATION_PHASE")
            .or_else(|_| std::env::var("HOI4_COLLECTOR_ALLOCATION_PHASE"))
            .unwrap(),
    )
    .unwrap();
    let ordinal = std::env::var("BATTLEPLAN_COLLECTOR_ALLOCATION_ORDINAL")
        .or_else(|_| std::env::var("HOI4_COLLECTOR_ALLOCATION_ORDINAL"))
        .unwrap()
        .parse::<usize>()
        .unwrap();
    match mode.as_str() {
        "fuel-history" => {
            collector_memory::terminal::reset();
            collector_memory::terminal::install_hooks();
            prepared_null(
                phase,
                ordinal,
                collector_fuel_awards::tables::allocation_failure_history_exercise,
            )
            .unwrap();
        }
        "raw-advisory" => raw_advisory_and_details().unwrap(),
        "set-aside" => set_aside_witness().unwrap(),
        "thread-label" => thread_label(phase).unwrap(),
        "cpu-sets" | "cpu-sets-size-failed" | "cpu-sets-read-failed" => {
            let query_failure = match mode.as_str() {
                "cpu-sets-size-failed" => Some(1),
                "cpu-sets-read-failed" => Some(2),
                _ => None,
            };
            cpu_sets_null(phase, ordinal, query_failure).unwrap();
        }
        "frame-growth-refused" => frame_growth_refused(),
        "frame-build-refused" => frame_build_refused(),
        "attach-refused" => attach_refused(),
        "payload-pools-refused" => payload_pools_refused(),
        #[cfg(feature = "alloc-fault-injection")]
        "decode-refused" => decode_refused(ordinal),
        "batch-drain-terminal" => {
            // SAFETY: This test fixture runs in isolation and owns the process-wide failure state.
            unsafe { collector_capture::acquisition_jobs::batch_drain_tests::exercise("terminal") }
        }
        "batch-drain-midway" => {
            // SAFETY: This test fixture runs in isolation and owns the process-wide failure state.
            unsafe { collector_capture::acquisition_jobs::batch_drain_tests::exercise("midway") }
        }
        "null" => prepared_buffers_null(phase, ordinal).unwrap(),
        "concurrent-watermark" => {
            crate::allocation_tests_support::concurrent_watermark_exercise().unwrap()
        }
        #[cfg(feature = "alloc-fault-injection")]
        "deferred" => deferred_failure().unwrap(),
        "pre-publication" => validate(
            &exercise_pipeline(phase, ordinal, false, true).unwrap(),
            true,
        ),
        #[cfg(feature = "alloc-fault-injection")]
        "sites" | "sites-load" => {
            collector_memory::terminal::observe_counted(Some(record_site));
            let outcome = exercise_pipeline(phase, ordinal, mode == "sites-load", false).unwrap();
            collector_memory::terminal::observe_counted(None);
            validate(&outcome, false);
            let sites = SITES.lock().unwrap();
            assert_eq!(sites.len() as u64, outcome.checked);
            let sites = sites.iter().map(|site| format!("{site:016x}"));
            println!(
                "collector-allocation-sites={}",
                sites.collect::<Vec<_>>().join(",")
            );
        }
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
    collector_memory::terminal::reset();
    collector_memory::terminal::install_hooks();
    let root = case_root(Phase::Encode, 1, true).join("deferred");
    fs::create_dir_all(&root)?;
    let witness_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(root.join(replay_data::witness::WITNESS_FILE_NAME))?;
    collector_memory::terminal::prepare(&witness_file, 1, 1000);
    collector_memory::terminal::inject_after(Phase::Encode, 1, 3);
    let _scope = collector_memory::terminal::enter(Phase::Encode);
    for hour in 19..=21 {
        context(Some(hour));
        engine_allocation::vec::<u8>(64)?;
        assert!(!collector_memory::terminal::failed());
        assert_eq!(collector_memory::terminal::allocation_count(), 0);
        collector_memory::terminal::committed(0, (hour as u64) * 100);
        collector_memory::terminal::committed_hour();
    }
    context(Some(22));
    assert_eq!(
        engine_allocation::vec::<u8>(64).unwrap_err().kind(),
        io::ErrorKind::OutOfMemory
    );
    assert_eq!(collector_memory::terminal::allocation_count(), 1);
    let witness = collector_memory::terminal::failure().unwrap();
    assert_eq!(witness.phase, Phase::Encode as u8);
    assert_eq!(witness.failed_hour, Some(22));
    assert_eq!(witness.committed_bytes, 2100);
    assert_eq!(Some(witness), read_witness(&witness_file)?);
    collector_memory::terminal::clear_handle();
    Ok(())
}

fn thread_label(phase: Phase) -> io::Result<()> {
    collector_memory::terminal::reset();
    collector_memory::terminal::install_hooks();
    let root = case_root(phase, 1, false);
    fs::create_dir_all(&root)?;
    let witness_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(root.join(replay_data::witness::WITNESS_FILE_NAME))?;
    collector_memory::terminal::prepare(&witness_file, 1, 1000);
    match phase {
        Phase::SpoolHelper => {
            let helper =
            // SAFETY: This fixture owns its frame and worker state and runs without a live
            // attachment.
            unsafe { collector_workers::spool_helper::SpoolHelper::start(None) }?;
            let job = || drop(engine_allocation::vec::<u64>(usize::MAX));
            // SAFETY: The spool exclusively uses this helper until the job joins.
            unsafe { helper.join(false, &job, || ()) };
        }
        Phase::Spool =>
        // SAFETY: This allocation-failure exercise runs in its own fixture process.
        unsafe { collector_spool::spooling::write_behind_tests::idle_sync_refused() },
        #[cfg(feature = "alloc-fault-injection")]
        Phase::Journal => {
            collector_memory::terminal::inject(Phase::Journal, 1);
            let mut processor =
            // SAFETY: The isolated test owns the spool's failure state.
            unsafe { collector_spool::spooling::spool_failure_fixture(&root) }?;
            let mut frame = collector_spool::spooling::allocation_failure_frame(19);
            {
                let _spool = collector_memory::terminal::enter(Phase::Spool);
                for hour in 19..=20 {
                    if let Some(clock) = &mut frame.clock {
                        clock.hour = hour;
                    }
                    let _ =
                    // SAFETY: This fixture owns its frame and worker state and runs without a live
                    // attachment.
                    unsafe { processor
                        .prepare(&frame) }
                        .and_then(|()|
                        // SAFETY: This fixture owns its frame and worker state and runs without a
                        // live attachment.
                        unsafe { processor.frame(&frame) });
                }
            }
            drop(processor);
        }
        Phase::Hook => {
            let _hook =
                collector_timing::hook_timing::enter(collector_timing::hook_timing::Family::Hourly);
            drop(engine_allocation::vec::<u64>(usize::MAX));
        }
        _ => panic!("{phase:?} has no labelled-thread case"),
    }
    let witness = collector_memory::terminal::failure().expect("the thread's refusal is terminal");
    assert_eq!(witness.phase, phase as u8, "{phase:?}");
    assert_eq!(Some(witness), read_witness(&witness_file)?);
    collector_memory::terminal::clear_handle();
    Ok(())
}

fn set_aside_witness() -> io::Result<()> {
    const OTHER: &[u8; 36] = b"01234567-89ab-cdef-0123-456789abcdee";
    collector_memory::terminal::reset();
    collector_memory::terminal::install_hooks();
    let root = case_root(Phase::Spool, 0, false);
    fs::create_dir_all(&root)?;
    let witness_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(root.join(replay_data::witness::WITNESS_FILE_NAME))?;
    collector_memory::terminal::prepare(&witness_file, 1, 1000);
    let mut processor =
    // SAFETY: The isolated test owns the spool's failure state.
    unsafe { collector_spool::spooling::spool_failure_fixture(&root) }?;
    {
        let _scope = collector_memory::terminal::enter(Phase::Spool);
        for hour in 10..=11 {
            // SAFETY: This fixture owns its frame and worker state and runs without a live
            // attachment.
            unsafe {
                processor.frame(&collector_spool::spooling::campaign_frame(
                    CAMPAIGN, 1, hour,
                ))
            }?;
        }
        // SAFETY: The originating spool is submitting a loss from its own capture or isolated
        // fixture.
        unsafe {
            processor.lost(&collector_capture::sampling::LostObservation {
                campaign_id: engine_native_memory::capture_text::Owned {
                    bytes: CAMPAIGN.to_vec(),
                    len: CAMPAIGN.len(),
                },
                first_hour: 12,
                last_hour: 12,
                load_signal: Some(1),
                count: 1,
                ..Default::default()
            })
        }?;
        // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
        unsafe { processor.frame(&collector_spool::spooling::campaign_frame(OTHER, 2, 30)) }?;
        collector_memory::terminal::fail(Phase::Spool, Code::Reserve);
    }
    let witness = collector_memory::terminal::failure().unwrap();
    assert_eq!(witness.committed_campaign_id, Some(*OTHER));
    assert_eq!(witness.campaign_id, Some(*OTHER));
    assert_eq!(Some(witness), read_witness(&witness_file)?);
    collector_memory::terminal::clear_handle();
    drop(processor);
    collector_memory::terminal::reset();
    let finalized =
        recorder_replay::replay_io::finalize_session_outcome(&root, &root.join("recordings"))
            .map_err(io::Error::other)?;
    assert!(finalized.errors.is_empty(), "{:?}", finalized.errors);
    assert_eq!(finalized.replays.len(), 2);
    let first = finalized
        .replays
        .iter()
        .find_map(|path| {
            let replay = replay::campaign::Replay::open_path(path).unwrap();
            (replay.campaign_id.as_slice() == CAMPAIGN).then_some(replay)
        })
        .unwrap();
    assert_eq!(
        first
            .gaps
            .iter()
            .map(|gap| (gap.branch, gap.first_hour, gap.last_hour))
            .collect::<Vec<_>>(),
        [(first.branches[0].id, 12, 12)]
    );
    Ok(())
}

fn cpu_sets_null(phase: Phase, ordinal: usize, query_failure: Option<usize>) -> io::Result<()> {
    collector_memory::terminal::reset();
    collector_memory::terminal::install_hooks();
    let root = case_root(phase, ordinal, false);
    fs::create_dir_all(&root)?;
    let witness_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(root.join(replay_data::witness::WITNESS_FILE_NAME))?;
    collector_memory::terminal::prepare(&witness_file, 1, 1000);
    context(Some(21));
    let _scope = collector_memory::terminal::enter(phase);
    let attempts =
        // SAFETY: This isolated fixture owns the affinity failure injection.
        unsafe { collector_workers::priority::background_cpu_sets_failure_exercise(ordinal, query_failure) };
    assert!(
        !collector_memory::terminal::failed(),
        "optional placement must not stop capture"
    );
    assert_eq!(collector_memory::terminal::failure(), None);
    assert_eq!(read_witness(&witness_file)?, None);
    collector_memory::terminal::clear_handle();
    println!("collector-allocation-count={attempts}");
    Ok(())
}

fn raw_advisory_and_details() -> io::Result<()> {
    collector_memory::terminal::reset();
    collector_memory::terminal::install_hooks();
    let root = case_root(Phase::Raw, 1, false).join("raw-details");
    fs::create_dir_all(&root)?;
    let witness_file = File::create(root.join(replay_data::witness::WITNESS_FILE_NAME))?;
    collector_memory::terminal::prepare(&witness_file, 1, 1000);
    let _details = collector_memory::terminal::prepare_details(&root).unwrap();
    context(Some(21));
    let mut raw = collector_raw::raw::RawLog::<1>::default();
    {
        let _scope = collector_memory::terminal::enter(Phase::Raw);
        with_allocations_denied(|| {
            raw.presize(64, 4096);
            raw.initialize_read_plan(64, 16, 4096);
        });
        assert!(
            !collector_memory::terminal::failed(),
            "optional sizing must not stop capture"
        );
        assert_eq!((raw.spans.capacity(), raw.bytes.capacity()), (0, 0));
        with_allocations_denied(|| raw.reserve(4096));
    }
    let witness = collector_memory::terminal::failure().unwrap();
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
    collector_memory::terminal::clear_handle();
    Ok(())
}

fn prepared_buffers_null(phase: Phase, ordinal: usize) -> io::Result<()> {
    use replay_data::{rows::StateCoreRow, rows::TableRows};
    collector_memory::terminal::reset();
    collector_memory::terminal::install_hooks();
    match phase {
        Phase::Raw => {
            let mut raw = collector_raw::raw::RawLog::<1>::new(4, 64);
            prepared_null(phase, ordinal, || {
                raw.reserve(4096);
                verdict(true)
            })
        }
        Phase::Decode => {
            let decode =
                collector_lexical::lexical::lexical_test_kit::allocation_failure_decode_exercise(
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
                        ..Default::default()
                    })
                    .collect(),
            );
            let fresh = TableRows::StateCores(vec![StateCoreRow {
                state: 129,
                cores: vec![5, 6],
                ..Default::default()
            }]);
            prepared_null(phase, ordinal, || {
                let merged = replay_data::sliced::merge_slice(
                    &previous,
                    fresh,
                    |state| state == 129,
                    |_| true,
                )?;
                verdict(merged.len() == 129)
            })
        }
        Phase::Spool => {
            let convert = collector_governance::governance::allocation_failure_fixture();
            prepared_null(phase, ordinal, || verdict(convert()))
        }
        Phase::Encode => prepared_encoder_null(ordinal),
        Phase::Status => {
            crate::status_tests::allocation_failure_exercise()?;
            let rows: Vec<_> = (0..128).map(|row| (row, "borrowed \"status\"\n")).collect();
            let mut bytes = Vec::new();
            prepared_null(phase, ordinal, || {
                collector_status::status::serialize_into(&mut bytes, &rows)
            })
        }
        _ => unreachable!("unprepared null-allocation boundary"),
    }
}

fn verdict(valid: bool) -> io::Result<()> {
    if collector_memory::terminal::failed() {
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
        .open(root.join(replay_data::witness::WITNESS_FILE_NAME))?;
    collector_memory::terminal::prepare(&witness_file, 1, 1000);
    context((phase != Phase::Status).then_some(21));
    #[cfg(feature = "alloc-fault-injection")]
    collector_memory::terminal::inject(phase, u64::MAX);
    let guard = NullGuard::new(ordinal);
    let scope = collector_memory::terminal::enter(phase);
    let result = run();
    let first = collector_memory::terminal::failure();
    let denials = DENIALS.load(Ordering::Relaxed);
    let attempts = ATTEMPTS.load(Ordering::Relaxed);
    if first.is_some() {
        let _ = engine_allocation::vec::<u8>(1);
        collector_memory::terminal::fail(Phase::Teardown, Code::Thread);
    }
    let unchanged = first == collector_memory::terminal::failure();
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
    collector_memory::terminal::clear_handle();
    println!("collector-allocation-count={attempts}");
    Ok(())
}

fn prepared_encoder_null(ordinal: usize) -> io::Result<()> {
    use replay_data::{rows::FrontRow, rows::TableId, rows::TableRows, rows::tables};
    use replay_frame::{
        frame::FrameEncoder, frame::FrameInput, frame::Metrics, frame::TableInput, frame::TableView,
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
    witness: Option<replay_data::witness::TerminalWitness>,
    disk_witness: Option<replay_data::witness::TerminalWitness>,
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
    contained: bool,
}

fn case_root(phase: Phase, ordinal: usize, checked: bool) -> PathBuf {
    PathBuf::from(
        std::env::var_os("BATTLEPLAN_COLLECTOR_ALLOCATION_RUN")
            .or_else(|| std::env::var_os("HOI4_COLLECTOR_ALLOCATION_RUN"))
            .unwrap(),
    )
    .join(
        std::env::var_os("BATTLEPLAN_COLLECTOR_ALLOCATION_TEST")
            .or_else(|| std::env::var_os("HOI4_COLLECTOR_ALLOCATION_TEST"))
            .unwrap(),
    )
    .join(format!(
        "{}-{phase:?}-{ordinal}-{checked}",
        std::process::id()
    ))
}

fn context(hour: Option<i32>) {
    collector_memory::terminal::set_context(Context {
        campaign_id: hour.map(|_| *CAMPAIGN),
        branch: hour.map(|_| 0),
        load_signal: hour.map(|_| 1234),
        failed_hour: hour,
    });
}

fn read_witness(file: &File) -> io::Result<Option<replay_data::witness::TerminalWitness>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = file;
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    if bytes.is_empty() {
        return Ok(None);
    }
    if replay_data::witness::TerminalWitness::decode_prepared(&bytes).is_ok() {
        return Ok(None);
    }
    replay_data::witness::decode_bundle(&bytes).map(|records| records.first().copied())
}

fn validate(outcome: &Exercise, refused: bool) {
    let failed = refused && !outcome.contained;
    if outcome.contained {
        assert_eq!(outcome.phase, Phase::Decode);
    }
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

fn minimal_sizing() -> collector_capture::country_scale::Sizing {
    unsafe { std::mem::zeroed() }
}

fn exercise_pipeline(
    phase: Phase,
    ordinal: usize,
    changed_loads: bool,
    fail_publication: bool,
) -> io::Result<Exercise> {
    collector_memory::terminal::reset();
    collector_memory::terminal::install_hooks();
    let root = case_root(phase, ordinal, true);
    fs::create_dir_all(&root)?;
    let witness_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(root.join(replay_data::witness::WITNESS_FILE_NAME))?;
    let shared =
    // SAFETY: This is unpublished attach or isolated fixture state, with no competing observer
    // boundary or detach.
    unsafe { collector_capture::sampling::Shared::with_sizing(0, 0, 1, &minimal_sizing()) }?;
    let mut processor =
    // SAFETY: The isolated test owns the spool's failure state.
    unsafe { collector_spool::spooling::spool_allocation_fixture(&root, phase == Phase::Spool) }?;
    let mut frame = collector_spool::spooling::allocation_failure_frame(19);
    if changed_loads {
        frame.clock.as_mut().unwrap().play_session_started_at = 19;
    }
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { processor.prepare(&frame) }?;
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { processor.frame(&frame) }?;
    if let Some(clock) = &mut frame.clock {
        clock.hour = 20;
        if changed_loads {
            clock.play_session_started_at = 20;
        }
    }
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { processor.prepare(&frame) }?;
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { processor.frame(&frame) }?;
    let preceding_publication = processor.allocation_failure_publication_state(20);
    let bytes = collector_divisions::census_test_fixture::fixture();
    let mut divisions = collector_capture::sampling::DivisionsJob::new();
    if let Some(clock) = &mut frame.clock {
        clock.hour = 21;
    }
    let index =
    // SAFETY: This fixture owns its frame and worker state and runs without a live attachment.
    unsafe { shared.take_frame(&mut 0) }.ok_or(io::ErrorKind::Other)?;
    let mut status_probe = if phase == Phase::Status {
        Some(crate::status_writer::AllocationProbe::prepare(&root)?)
    } else {
        None
    };
    let mut lexical_zero = (phase == Phase::Decode).then(|| {
        collector_lexical::lexical::lexical_test_kit::allocation_failure_decode_exercise(0, 0)
    });
    let mut lexical_short = (phase == Phase::Decode).then(|| {
        collector_lexical::lexical::lexical_test_kit::allocation_failure_decode_exercise(1, 2)
    });
    let mut roster = if matches!(phase, Phase::Spool | Phase::Encode | Phase::Map) {
        Some(collector_metadata::roster::allocation_failure_fixture())
    } else {
        None
    };
    let mut governance = if matches!(phase, Phase::Spool | Phase::Encode | Phase::Map) {
        Some(collector_governance::governance::allocation_failure_fixture())
    } else {
        None
    };
    let front = replay_data::rows::FrontRow {
        front: 1,
        section_ids: vec![7, 8],
        section_ends: vec![2, 4],
        province_pairs: vec![3, 4, 5, 6],
    };
    let mut front_expected = Vec::new();
    front.encode_change(None, 0, &mut front_expected)?;
    collector_memory::terminal::prepare(&witness_file, 1, 1000);
    let idle = matches!(phase, Phase::Bootstrap | Phase::Status | Phase::Teardown);
    context((!idle).then_some(21));
    #[cfg(feature = "alloc-fault-injection")]
    collector_memory::terminal::inject(phase, ordinal as u64);
    TARGET_PHASE.store(phase as u8, Ordering::Relaxed);
    REACHED.store(false, Ordering::Relaxed);
    let mut fail_next_publication = fail_publication;
    let mut original_calls = 0;
    let mut conversion_matched = true;
    let mut callback_calls = 0;
    let mut last_attempted_hour = 21;
    let mut bootstrap_refused = false;
    let mut contained = false;
    if phase == Phase::Bootstrap {
        let _scope = collector_memory::terminal::enter(Phase::Bootstrap);
        let before = collector_capture::sampling::SHARED.identity_for_test();
        bootstrap_refused =
            // SAFETY: This is unpublished attach or isolated fixture state, with no competing
            // observer boundary or detach.
            unsafe { collector_capture::sampling::Shared::with_sizing(0, 0, 1, &minimal_sizing()) }.is_err();
        assert!(before.is_null());
        assert_eq!(
            before,
            collector_capture::sampling::SHARED.identity_for_test(),
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
        let entry = if idle {
            None
        } else {
            frame
                .clock
                .as_ref()
                .map(|clock| clock.with_campaign_id(CAMPAIGN.as_slice()))
        };
        // SAFETY: This isolated fixture owns the callback state and the Divisions read accounting.
        unsafe {
            collector_capture::sampling::synthetic_callback(
                &shared,
                entry,
                || match phase {
                    Phase::Bootstrap => {}
                    Phase::Callback | Phase::Acquisition => {
                        let _scope = collector_memory::terminal::enter(phase);
                        collector_capture::sampling::copy_divisions_refresh(
                            &collector_divisions::census_test_fixture::memory(&bytes),
                            collector_capture::sampling::DivisionsCopyContext {
                                base: collector_capture::sampling::divisions_test_kit::BASE,
                                game_state:
                                    collector_capture::sampling::divisions_test_kit::GAME_STATE,
                                ..Default::default()
                            },
                            &mut divisions,
                        );
                    }
                    Phase::Raw => {
                        let _scope = collector_memory::terminal::enter(Phase::Raw);
                        collector_raw::raw::allocation_exercise();
                        if !collector_memory::terminal::failed() {
                            crate::snapshot_integration_tests::allocation_failure_exercise();
                        }
                    }
                    Phase::Decode => {
                        let _scope = collector_memory::terminal::enter(Phase::Decode);
                        if let Some(decode) = lexical_zero.take() {
                            conversion_matched &= decode();
                        }
                        if !collector_memory::terminal::failed()
                            && let Some(decode) = lexical_short.take()
                        {
                            conversion_matched &= decode();
                        }
                        if collector_memory::terminal::failed() {
                            return;
                        }
                        divisions.grow();
                        if !collector_memory::terminal::failed() {
                            collector_capture::sampling::decode_divisions_payload(
                                &mut frame,
                                0,
                                &mut divisions,
                                &mut [0; 6],
                            );
                            contained |= collector_capture::decode::allocation_exercise(
                                &shared, index, hour,
                            );
                        }
                    }
                    Phase::Extended => {
                        collector_combat::combat_details::allocation_failure_exercise(|| {}, || {});
                        let _scope = collector_memory::terminal::enter(Phase::Extended);
                        if !collector_memory::terminal::failed() {
                            collector_air_histories::air_histories::allocation_failure_exercise();
                        }
                        if !collector_memory::terminal::failed() {
                            collector_air_relations::air_relations::allocation_failure_exercise();
                        }
                        if !collector_memory::terminal::failed() {
                            collector_characters::extended::character_inputs::allocation_failure_exercise();
                        }
                        let registry = collector_certificates::watch_registry::WatchRegistry::new();
                        if !collector_memory::terminal::failed() {
                            registry.offer(32);
                        }
                        if !collector_memory::terminal::failed() {
                            let mut capacity = collector_capture::extended::Capacities::default();
                            frame.extended.grow(&mut capacity);
                        }
                        registry.release();
                    }
                    Phase::Spool | Phase::Encode | Phase::Map => {
                        collector_capture::extended::output_allocation_failure_exercise(
                            || {},
                            || {},
                        );
                        let _scope = collector_memory::terminal::enter(Phase::Spool);
                        if !collector_memory::terminal::failed()
                            && let Some(verify) = roster.take()
                        {
                            let _ = verify();
                        }
                        if !collector_memory::terminal::failed()
                            && let Some(convert) = governance.take()
                        {
                            conversion_matched &= convert();
                        }
                        if !collector_memory::terminal::failed() {
                            let _ = collector_metadata::fixed_layout::allocation_failure_exercise();
                        }
                        if !collector_memory::terminal::failed() {
                            let _ = front.check_key_fields();
                        }
                        if !collector_memory::terminal::failed() {
                            let _encode = collector_memory::terminal::enter(Phase::Encode);
                            let mut encoded = Vec::new();
                            if front.encode_change(None, 0, &mut encoded).is_ok() {
                                conversion_matched &= encoded == front_expected;
                            }
                        }
                        if !collector_memory::terminal::failed() {
                            let _ = processor
                                .prepare(&frame)
                                .and_then(|()| processor.frame_before_publication(&frame, || {
                                    if std::mem::take(&mut fail_next_publication) {
                                        collector_memory::terminal::fail(Phase::Spool, Code::Reserve);
                                    }
                                }));
                        }
                    }
                    Phase::Status => {
                        let _scope = collector_memory::terminal::enter(Phase::Status);
                        collector_memory::terminal::attempted_hour(None);
                        if let Ok(status) = processor.status() {
                            let _ = collector_status::status::serialize(&status);
                        }
                        if !collector_memory::terminal::failed()
                            && let Some(mut probe) = status_probe.take()
                        {
                            let _ = probe.run();
                        }
                    }
                    Phase::Teardown => {
                        let _scope = collector_memory::terminal::enter(Phase::Teardown);
                        collector_memory::terminal::attempted_hour(None);
                        let _ = processor.detach();
                    }
                    Phase::Journal
                    | Phase::SpoolHelper
                    | Phase::History
                    | Phase::Unphased
                    | Phase::Hook => {
                        panic!("{phase:?} has no deterministic sweep")
                    }
                },
                || {
                    original_calls += 1;
                },
            );
        }
        if collector_memory::terminal::failed() {
            break;
        }
    }
    let first = collector_memory::terminal::failure();
    let failed = collector_memory::terminal::failed();
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
        && (
            // SAFETY: This is the runtime's frame pool for the current session.
            unsafe { shared.frames() }
                .slot(index)
                .completed
                .load(Ordering::Acquire)
                != 0
                || !
                // SAFETY: This is the runtime's frame pool for the current session.
                unsafe { shared.frames() }.ready_is_empty()
        );
    let mut admitted_after_failure = false;
    unsafe {
        collector_capture::sampling::synthetic_callback(
            &shared,
            frame
                .clock
                .as_ref()
                .map(engine_native::cgame_state::Clock::borrowed)
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
    let allocations_before_cleanup = collector_memory::terminal::allocation_count();
    if failed {
        let _scope = collector_memory::terminal::enter(Phase::Teardown);
        let _ = engine_allocation::vec::<u8>(1);
        collector_memory::terminal::fail(Phase::Teardown, Code::Thread);
    }
    {
        let _scope = collector_memory::terminal::enter(Phase::Teardown);
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
        collector_memory::terminal::allocation_count()
    };
    let second_failure_unchanged = first == collector_memory::terminal::failure();
    let reached = REACHED.load(Ordering::Relaxed);
    let disk_witness = read_witness(&witness_file)?;
    collector_memory::terminal::clear_handle();
    #[cfg(feature = "alloc-fault-injection")]
    collector_memory::terminal::inject(Phase::Bootstrap, u64::MAX);
    collector_memory::terminal::reset();
    let (prefix_hours, attempted_hour_committed) =
        recovered_hours(&root, last_attempted_hour, first.as_ref())?;
    drop(witness_file);
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
        contained,
    })
}

fn recovered_hours(
    root: &Path,
    attempted_hour: i32,
    witness: Option<&replay_data::witness::TerminalWitness>,
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
        let recovered = replay_capture_spool::spool::recover_prefix(prefix);
        if exact_cap && recovered.valid_bytes != prefix.len() {
            return Err(io::ErrorKind::InvalidData.into());
        }
        for record in recovered.records {
            if let replay_capture_spool::spool::Record::Frame(frame) = record {
                hours += 1;
                attempted_committed |= frame.hour == attempted_hour;
            }
        }
    }
    Ok((hours, attempted_committed))
}
