use super::*;
use collector_observers::country_input_changes::{Changes, Table};
use engine_native::{Memory, walk::Outcome};
use std::sync::{Condvar, atomic::AtomicUsize, mpsc};

#[test]
fn callback_help_keeps_nested_acquisition_and_coordinator_jobs_on_workers() {
    for role in [
        Role::Variants,
        Role::Governance,
        Role::GatedProjects,
        Role::PresentationPrepare,
        Role::PresentationSettle,
        Role::Presentation,
        Role::FinalInputs,
        Role::ScriptVariables,
        Role::Research,
        Role::Lexical,
    ] {
        assert!(!role.callback_help());
        assert!(role.worker_only());
    }
    for role in [
        Role::Countries,
        Role::DivisionInputs,
        Role::AirWing,
        Role::Characters,
    ] {
        assert!(role.callback_help());
        assert!(role.worker_only());
    }
}

struct Unreadable;
impl Memory for Unreadable {
    fn copy(&self, _: usize, _: &mut [u8]) -> bool {
        false
    }
}
#[derive(Default)]
struct Marks(AtomicUsize);
impl Changes for Marks {
    fn take(&self, table: Table, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        assert_eq!(table, Table::Projects);
        self.0.fetch_add(1, Ordering::Relaxed);
        visit(1);
        Some(0)
    }
}
fn gated(hour: i32, full: bool, raw: bool, mode: replay_data::scope::CaptureMode) -> GatedRequest {
    let mut captured = crate::extended::Captured::with_capacities(&Default::default());
    captured.set_capture_mode(mode);
    captured.prepare_hour(hour, full);
    captured.radar_pilot().radar_raw_mode = raw;
    captured.radar_pilot().intel_raw_mode = true;
    captured.radar_pilot().registrations_raw_mode = true;
    captured.projects_pilot().raw_mode = raw;
    captured.construction_rest_pilot().rest_raw_mode = raw;
    captured.take_gated_request(
        11,
        0x1000,
        hour,
        full,
        Box::new(collector_certificates::slicing::SliceFingerprints::new()),
        Some(
            engine_allocation::shared(
                engine_native::country_memory::Cache::try_with_capacity(0).unwrap(),
            )
            .unwrap(),
        ),
    )
}
fn old_gated(request: &mut GatedRequest, marks: &Marks) {
    request.packs.fog.capture(
        &Unreadable,
        request.base,
        request.game_state,
        request.hour,
        request.force_full,
    );
    request.packs.construction.capture(
        &Unreadable,
        request.base,
        request.game_state,
        request.hour,
        request.force_full,
    );
    if request.packs.projects_recorded {
        request.packs.projects.capture_selected(
            &Unreadable,
            &Unreadable,
            request.base,
            request.game_state,
            request.hour,
            request.force_full,
            request.project_fingerprints.as_deref_mut().unwrap(),
            marks,
        );
    } else {
        request.packs.projects.fail_gated_capture(
            request.hour,
            request.force_full,
            collector_extended_support::outcomes::NOT_RECORDED,
        );
    }
}
#[test]
fn split_gated_matches_serial_tables_raw_admission_and_single_mark_consumption() {
    use replay_data::scope::CaptureMode;
    for (hour, full) in [
        (0, false),
        (1, false),
        (9, false),
        (23, false),
        (24, false),
        (25, true),
    ] {
        for raw in [false, true] {
            for mode in [CaptureMode::Full, CaptureMode::Map] {
                let mut reference = gated(hour, full, raw, mode);
                let reference_marks = Marks::default();
                old_gated(&mut reference, &reference_marks);
                let mut split = gated(hour, full, raw, mode);
                let marks = Marks::default();
                let mut projects = split.take_projects();
                assert!(split.countries.is_some());
                assert!(projects.countries.is_some());
                projects.capture(&Unreadable, &Unreadable, &marks);
                split
                    .packs
                    .capture(&Unreadable, split.base, split.game_state, hour, full);
                split.restore_projects(projects);
                assert_eq!(
                    split.packs.projects.tables(),
                    reference.packs.projects.tables()
                );
                assert_eq!(split.packs.fog.tables(), reference.packs.fog.tables());
                assert_eq!(
                    split.packs.construction.tables(),
                    reference.packs.construction.tables()
                );
                assert_eq!(split.packs.projects.raw_mode, raw);
                assert_eq!(
                    marks.0.load(Ordering::Relaxed),
                    usize::from(split.packs.projects_recorded)
                );
                assert_eq!(
                    marks.0.load(Ordering::Relaxed),
                    reference_marks.0.load(Ordering::Relaxed)
                );
            }
        }
    }
}
#[test]
fn either_split_gated_failure_keeps_the_combined_failure() {
    for (rest_ok, projects_ok) in [(true, false), (false, true), (false, false)] {
        let mut frame = crate::extended::Captured::with_capacities(&Default::default());
        let mut request = frame.take_gated_request(
            11,
            0x1000,
            24,
            false,
            Box::new(collector_certificates::slicing::SliceFingerprints::new()),
            None,
        );
        let mut projects = request.take_projects();
        projects.capture(&Unreadable, &Unreadable, &Marks::default());
        request.packs.capture(&Unreadable, 11, 0x1000, 24, false);
        request.restore_projects(projects);
        frame.restore_gated_request(request, rest_ok && projects_ok);
        let actual = frame.outcomes();
        let mut reference = crate::extended::Captured::with_capacities(&Default::default());
        let request = reference.take_gated_request(
            11,
            0x1000,
            24,
            false,
            Box::new(collector_certificates::slicing::SliceFingerprints::new()),
            None,
        );
        reference.restore_gated_request(request, false);
        assert_eq!(actual, reference.outcomes());
        assert_eq!(
            actual[replay_data::rows::TableId::Projects.index()],
            Outcome::Unavailable {
                reason: "gated acquisition failed"
            }
        );
    }
}
#[derive(Default)]
struct Gate(Mutex<bool>, Condvar);
impl Gate {
    fn open(&self) {
        *self.0.lock().unwrap() = true;
        self.1.notify_all();
    }
    fn wait(&self) {
        let (open, _) = self
            .1
            .wait_timeout_while(
                self.0.lock().unwrap(),
                collector_test_support::TEST_HANG_GUARD,
                |open| !*open,
            )
            .unwrap();
        assert!(*open, "gated fixture was not released");
    }
}
struct Release(Arc<Gate>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.open();
    }
}
#[test]
fn projects_run_while_division_inputs_fog_and_construction_are_held() {
    let rest = Arc::new(Gate::default());
    let (started, running) = mpsc::channel();
    let pool = Pool::with_test_runner(3, {
        let rest = Arc::clone(&rest);
        move |job| {
            let role = job.role();
            match job {
                Job::DivisionInputs(_) => {
                    started.send(role).unwrap();
                    rest.wait();
                    Report::DivisionInputs(false)
                }
                Job::Gated(_) => {
                    started.send(role).unwrap();
                    rest.wait();
                    run_for_test(job, &Unreadable)
                }
                Job::GatedProjects(request) => {
                    request.capture(&Unreadable, &Unreadable, &Marks::default());
                    started.send(role).unwrap();
                    Report::GatedProjects(())
                }
                _ => unreachable!(),
            }
        }
    });
    let mut batch = Batch::new(&pool);
    let _release = Release(rest);
    let mut request = gated(24, false, true, replay_data::scope::CaptureMode::Full);
    let projects = request.take_projects();
    batch.hold();
    let input = unsafe {
        batch.submit(&mut Some(Job::DivisionInputs(RawRequest {
            countries: engine_allocation::shared(
                engine_native::country_memory::Cache::try_with_capacity(0).unwrap(),
            )
            .unwrap(),
            context: crate::sampling::DivisionsCopyContext::default(),
            payload: crate::sampling::DivisionsJob::new_worker(),
        })))
    }
    .unwrap();
    unsafe { batch.submit(&mut Some(Job::Gated(request))) }.unwrap();
    unsafe { batch.submit(&mut Some(Job::GatedProjects(projects))) }.unwrap();
    batch.release();
    let mut roles = std::array::from_fn::<_, 3, _>(|_| {
        running
            .recv_timeout(collector_test_support::TEST_HANG_GUARD)
            .unwrap()
    });
    assert!(
        !batch.is_complete(input),
        "Projects finished before the Inputs handler could run"
    );
    roles.sort_by_key(|role| *role as usize);
    let mut expected = [Role::DivisionInputs, Role::Gated, Role::GatedProjects];
    expected.sort_by_key(|role| *role as usize);
    assert_eq!(roles, expected);
}
