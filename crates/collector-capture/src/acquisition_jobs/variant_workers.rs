use super::*;
use std::sync::Weak;

pub struct Request {
    pub base: usize,
    pub(crate) part: Work,
}
pub enum Work {
    Variant(production::VariantPart),
    #[cfg(test)]
    Probe(Arc<std::sync::atomic::AtomicUsize>),
    Context(production::DesignContextPart),
    Setup(production::ColdSetupPart),
    Snapshots(production::SnapshotPart),
    Storage {
        part: production::ColdStorage,
        workers: Dispatch,
    },
    Technology(governance::TechnologyPart),
    Projects {
        part: projects::ProjectPart,
        game_state: usize,
    },
    TemplateWalk(collector_division_templates::extended::division_templates::WalkPart),
    ProjectsRest(Box<projects::Rest>),
    Buildings(Box<super::BuildingsPart>),
}
impl Work {
    pub(super) fn role(&self) -> Role {
        match self {
            Self::Technology(_) => Role::Governance,
            Self::Projects { .. } => Role::GatedProjects,
            Self::TemplateWalk(_) => Role::Templates,
            Self::ProjectsRest(_) => Role::GatedProjects,
            Self::Buildings(_) => Role::Governance,
            _ => Role::Variants,
        }
    }
    pub(super) fn capture(&mut self, memory: &impl engine_native_memory::Memory, base: usize) {
        match self {
            Self::Variant(part) => part.capture(memory, base),
            #[cfg(test)]
            Self::Probe(calls) => {
                calls.fetch_add(1, Ordering::Relaxed);
            }
            Self::Context(part) => part.capture(memory, base),
            Self::Setup(part) => part.capture(),
            Self::Snapshots(part) => part.capture(),
            Self::Storage { part, workers } => {
                part.capture(&mut |parts| workers.setup(parts, memory, base));
            }
            Self::Technology(part) => part.capture(memory, base),
            Self::Projects { part, game_state } => part.capture(memory, base, *game_state),
            Self::TemplateWalk(part) => part.capture(
                memory,
                base,
                &collector_division_templates::template_changes::Observer,
            ),
            Self::ProjectsRest(rest) => rest.capture(memory, base),
            Self::Buildings(part) => part.capture(memory),
        }
    }
}
#[derive(Default)]
pub struct Dispatch {
    executor: Weak<collector_workers::acquisition::Pool<Job, Report>>,
    calibration: Option<Arc<SharedCalibration>>,
    workers: usize,
    priority: u8,
}
impl Dispatch {
    pub(super) fn arm(&mut self, pool: &Pool) {
        self.executor = Arc::downgrade(&pool.executor);
        self.calibration = Some(Arc::clone(&pool.calibration));
        self.workers = pool.worker_count;
    }
    pub(super) fn arm_at(&mut self, pool: &Pool, priority: u8) {
        self.arm(pool);
        self.priority = priority;
    }
    fn fork(&self) -> Self {
        Self {
            executor: self.executor.clone(),
            calibration: self.calibration.clone(),
            workers: self.workers,
            priority: 2,
            ..Default::default()
        }
    }
    pub(super) fn storage_during(
        &self,
        part: production::ColdStorage,
        memory: &impl engine_native_memory::Memory,
        base: usize,
        run: &mut dyn FnMut(),
    ) {
        self.during(
            Work::Storage {
                part,
                workers: self.fork(),
            },
            memory,
            base,
            run,
        );
    }
    fn during(
        &self,
        mut work: Work,
        memory: &impl engine_native_memory::Memory,
        base: usize,
        run: &mut dyn FnMut(),
    ) {
        let executor = self.executor.upgrade().filter(|_| self.workers > 1);
        let fanout = self
            .calibration
            .as_ref()
            .and_then(|calibration| Fanout::enter(&calibration.cold_parents, self.workers));
        let tickets = engine_allocation::advisory(|| engine_allocation::vec(1).ok());
        if let (Some(executor), Some(_fanout), Some(tickets)) = (executor, fanout, tickets) {
            let mut pending = Children {
                executor: &executor,
                calibration: self.calibration.as_deref().unwrap(),
                tickets,
            };
            let mut request = Some(Job::VariantPart(Request { base, part: work }));
            // SAFETY: this parent drains the storage child before returning its live lease.
            match unsafe { executor.submit_with_priority(&mut request, 2, true, true) } {
                Ok(ticket) => {
                    pending.tickets.push(Some(ticket));
                    run();
                    let mut completed = None;
                    executor.finish_worker_into(pending.tickets[0].take().unwrap(), &mut completed);
                    if let Some(completed) = completed {
                        account_completion(pending.calibration, &completed);
                    }
                    return;
                }
                Err(_) => {
                    let Some(Job::VariantPart(request)) = request else {
                        unreachable!()
                    };
                    work = request.part;
                }
            }
        }
        work.capture(memory, base);
        run();
    }
    pub(super) fn beside(
        &self,
        mut work: Work,
        memory: &impl engine_native_memory::Memory,
        base: usize,
        wait: collector_workers::hour_stages::Part,
        run: &mut dyn FnMut(),
    ) -> Option<Work> {
        let executor = self.executor.upgrade().filter(|_| self.workers > 1);
        let fanout = self
            .calibration
            .as_ref()
            .and_then(|calibration| Fanout::enter(&calibration.cold_parents, self.workers));
        let tickets = engine_allocation::advisory(|| engine_allocation::vec(1).ok());
        if let (Some(executor), Some(_fanout), Some(tickets)) = (executor, fanout, tickets) {
            let mut pending = Children {
                executor: &executor,
                calibration: self.calibration.as_deref().unwrap(),
                tickets,
            };
            let role = work.role();
            let mut request = Some(Job::VariantPart(Request { base, part: work }));
            // SAFETY: this parent drains the child before returning its live lease.
            let submitted =
                unsafe { executor.submit_with_priority(&mut request, self.priority, true, true) };
            note_dispatch(&pending.calibration.roles[role as usize], submitted.is_ok());
            match submitted {
                Ok(ticket) => {
                    pending.tickets.push(Some(ticket));
                    run();
                    let waited = collector_primitives::clock::qpc();
                    executor.run_unclaimed_child(ticket);
                    let mut completed = None;
                    executor.finish_worker_into(pending.tickets[0].take().unwrap(), &mut completed);
                    collector_workers::hour_stages::record_part(
                        wait,
                        waited,
                        collector_primitives::clock::qpc(),
                    );
                    let completed = completed?;
                    account_completion(pending.calibration, &completed);
                    return match (completed.report, completed.request) {
                        (Ok(Report::VariantPart), Job::VariantPart(request)) => Some(request.part),
                        _ => None,
                    };
                }
                Err(_) => {
                    let Some(Job::VariantPart(request)) = request else {
                        unreachable!()
                    };
                    work = request.part;
                }
            }
        }
        work.capture(memory, base);
        run();
        Some(work)
    }
    pub fn lanes(&self) -> usize {
        self.workers.max(1)
    }
    #[cfg(test)]
    pub fn priority(&self) -> u8 {
        self.priority
    }
    pub(super) fn acquire(
        &self,
        parts: Vec<production::VariantPart>,
        memory: &impl engine_native_memory::Memory,
        base: usize,
    ) -> Result<Vec<production::VariantPart>, &'static str> {
        let parts = engine_allocation::collect_vec(parts.into_iter().map(Work::Variant))
            .map_err(|_| "variant worker request allocation failed")?;
        let parts = self.acquire_work(parts, memory, base)?;
        let mut output = engine_allocation::vec(parts.len())
            .map_err(|_| "variant worker result allocation failed")?;
        for part in parts {
            let Work::Variant(part) = part else {
                return Err("variant worker returned another family");
            };
            output.push(part);
        }
        Ok(output)
    }
    pub(super) fn context(
        &self,
        parts: Vec<production::DesignContextPart>,
        memory: &impl engine_native_memory::Memory,
        base: usize,
    ) -> Result<Vec<production::DesignContextPart>, &'static str> {
        let parts = engine_allocation::collect_vec(parts.into_iter().map(Work::Context))
            .map_err(|_| "design context worker request allocation failed")?;
        let parts = self.acquire_work(parts, memory, base)?;
        let mut output = engine_allocation::vec(parts.len())
            .map_err(|_| "design context worker result allocation failed")?;
        for part in parts {
            let Work::Context(part) = part else {
                return Err("design context worker returned another family");
            };
            output.push(part);
        }
        Ok(output)
    }
    pub(super) fn technologies(
        &self,
        parts: Vec<governance::TechnologyPart>,
        memory: &impl engine_native_memory::Memory,
        base: usize,
    ) -> Result<Vec<governance::TechnologyPart>, &'static str> {
        let parts = engine_allocation::collect_vec(parts.into_iter().map(Work::Technology))
            .map_err(|_| "technology worker allocation failed")?;
        let parts = self.acquire_work(parts, memory, base)?;
        engine_allocation::collect_vec(parts.into_iter().map(|part| {
            let Work::Technology(part) = part else {
                unreachable!()
            };
            part
        }))
        .map_err(|_| "technology worker result allocation failed")
    }
    pub(super) fn projects(
        &self,
        parts: Vec<projects::ProjectPart>,
        memory: &impl engine_native_memory::Memory,
        base: usize,
        game_state: usize,
    ) -> Result<Vec<projects::ProjectPart>, &'static str> {
        let parts = engine_allocation::collect_vec(
            parts
                .into_iter()
                .map(|part| Work::Projects { part, game_state }),
        )
        .map_err(|_| "project worker allocation failed")?;
        let parts = self.acquire_work(parts, memory, base)?;
        engine_allocation::collect_vec(parts.into_iter().map(|part| {
            let Work::Projects { part, .. } = part else {
                unreachable!()
            };
            part
        }))
        .map_err(|_| "project worker result allocation failed")
    }
    pub(crate) fn template_walk(
        &self,
        parts: Vec<collector_division_templates::extended::division_templates::WalkPart>,
        memory: &impl engine_native_memory::Memory,
        base: usize,
    ) -> Result<
        Vec<collector_division_templates::extended::division_templates::WalkPart>,
        &'static str,
    > {
        let parts = engine_allocation::collect_vec(parts.into_iter().map(Work::TemplateWalk))
            .map_err(|_| "template walk worker allocation failed")?;
        let parts = self.acquire_work(parts, memory, base)?;
        engine_allocation::collect_vec(parts.into_iter().map(|part| {
            let Work::TemplateWalk(part) = part else {
                unreachable!()
            };
            part
        }))
        .map_err(|_| "template walk worker result allocation failed")
    }
    pub(super) fn snapshots(
        &self,
        parts: Vec<production::SnapshotPart>,
        memory: &impl engine_native_memory::Memory,
        base: usize,
    ) -> Result<Vec<production::SnapshotPart>, &'static str> {
        if self.workers < 2 || self.executor.upgrade().is_none() {
            return Err("variant snapshot lanes unavailable");
        }
        let parts = engine_allocation::collect_vec(parts.into_iter().map(Work::Snapshots))
            .map_err(|_| "variant snapshot worker allocation failed")?;
        let parts = self.acquire_work(parts, memory, base)?;
        engine_allocation::collect_vec(parts.into_iter().map(|part| {
            let Work::Snapshots(part) = part else {
                unreachable!()
            };
            part
        }))
        .map_err(|_| "variant snapshot worker result allocation failed")
    }
    pub(super) fn setup(
        &self,
        parts: Vec<production::ColdSetupPart>,
        memory: &impl engine_native_memory::Memory,
        base: usize,
    ) -> Result<Vec<production::ColdSetupPart>, &'static str> {
        let parts = engine_allocation::collect_vec(parts.into_iter().map(Work::Setup))
            .map_err(|_| "cold setup worker allocation failed")?;
        let parts = self.acquire_work(parts, memory, base)?;
        engine_allocation::collect_vec(parts.into_iter().map(|part| {
            let Work::Setup(part) = part else {
                unreachable!()
            };
            part
        }))
        .map_err(|_| "cold setup worker result allocation failed")
    }
    fn acquire_work(
        &self,
        parts: Vec<Work>,
        memory: &impl engine_native_memory::Memory,
        base: usize,
    ) -> Result<Vec<Work>, &'static str> {
        let Some(executor) = self.executor.upgrade().filter(|_| self.workers > 1) else {
            let mut parts = parts;
            for part in &mut parts {
                part.capture(memory, base);
            }
            return Ok(parts);
        };
        let calibration = self.calibration.as_deref().unwrap();
        let Some(_fanout) = Fanout::enter(&calibration.cold_parents, self.workers) else {
            let mut parts = parts;
            for part in &mut parts {
                part.capture(memory, base);
            }
            return Ok(parts);
        };
        let mut pending = Children {
            executor: &executor,
            calibration: self.calibration.as_deref().unwrap(),
            tickets: engine_allocation::vec(parts.len())
                .map_err(|_| "variant worker ticket allocation failed")?,
        };
        let mut output = engine_allocation::vec(parts.len())
            .map_err(|_| "variant worker result allocation failed")?;
        let mut parts = parts.into_iter();
        let local = parts.next();
        let mut failure = None;
        for part in parts {
            let role = part.role();
            let mut request = Some(Job::VariantPart(Request { base, part }));
            // SAFETY: the worker parent drains every child before returning its live lease.
            let submitted =
                unsafe { executor.submit_with_priority(&mut request, self.priority, true, true) };
            note_dispatch(&pending.calibration.roles[role as usize], submitted.is_ok());
            match submitted {
                Ok(ticket) => pending.tickets.push(Some(ticket)),
                Err(SubmitError::Full) => {
                    let Some(Job::VariantPart(mut request)) = request else {
                        unreachable!()
                    };
                    request.part.capture(memory, base);
                    output.push(request.part);
                }
                Err(_) => failure = Some("variant partition admission stopped before reading"),
            }
        }
        if failure.is_none()
            && let Some(mut local) = local
        {
            local.capture(memory, base);
            output.push(local);
        }
        for ticket in pending.tickets.iter().flatten() {
            executor.run_unclaimed_child(*ticket);
        }
        for index in 0..pending.tickets.len() {
            let ticket = pending.tickets[index].take().unwrap();
            let mut completed = None;
            executor.finish_worker_into(ticket, &mut completed);
            let Some(completed) = completed else {
                failure = Some("variant partition completion missing");
                continue;
            };
            account_completion(pending.calibration, &completed);
            match completed.report {
                Ok(Report::VariantPart) => {
                    let Job::VariantPart(request) = completed.request else {
                        unreachable!()
                    };
                    output.push(request.part);
                }
                Err(Failure::Panicked) => failure = Some("variant partition acquisition panicked"),
                _ => failure = Some("variant partition acquisition stopped before reading"),
            }
        }
        failure.map_or(Ok(output), Err)
    }
}
struct Fanout<'a>(&'a std::sync::atomic::AtomicUsize);
impl<'a> Fanout<'a> {
    fn enter(active: &'a std::sync::atomic::AtomicUsize, workers: usize) -> Option<Self> {
        active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < workers.saturating_sub(1)).then_some(count + 1)
            })
            .ok()
            .map(|_| Self(active))
    }
}
impl Drop for Fanout<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Release);
    }
}
struct Children<'a> {
    executor: &'a collector_workers::acquisition::Pool<Job, Report>,
    calibration: &'a SharedCalibration,
    tickets: Vec<Option<Ticket>>,
}
impl Drop for Children<'_> {
    fn drop(&mut self) {
        for ticket in self.tickets.iter_mut().filter_map(Option::take) {
            contain(|| {
                let mut completed = None;
                self.executor.finish_worker_into(ticket, &mut completed);
                if let Some(completed) = completed {
                    account_completion(self.calibration, &completed);
                }
            });
        }
    }
}

#[cfg(test)]
mod fanout_tests {
    use super::*;
    #[test]
    fn nonblocking_parent_admission_always_leaves_a_worker_for_children_and_releases_on_unwind() {
        for workers in [1, 2, 3, 4, collector_workers::acquisition::MAX_WORKERS] {
            let active = std::sync::atomic::AtomicUsize::new(0);
            let mut parents: Vec<_> = (0..workers.saturating_sub(1))
                .map(|_| Fanout::enter(&active, workers).unwrap())
                .collect();
            assert!(Fanout::enter(&active, workers).is_none());
            assert_eq!(active.load(Ordering::Acquire), workers - 1);
            if !parents.is_empty() {
                drop(parents.pop());
                let unwind = std::panic::catch_unwind(|| {
                    let _parent = Fanout::enter(&active, workers).unwrap();
                    assert!(Fanout::enter(&active, workers).is_none());
                    panic!("synthetic cold parent unwind");
                });
                assert!(unwind.is_err());
                assert!(Fanout::enter(&active, workers).is_some());
            }
            drop(parents);
            assert_eq!(active.load(Ordering::Acquire), 0);
        }
    }
}

#[cfg(test)]
mod overlap_tests {
    use super::*;
    use std::sync::{atomic::AtomicUsize, mpsc};
    struct NoReads;
    impl engine_native_memory::Memory for NoReads {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("owned setup read native memory")
        }
    }
    #[test]
    fn cold_setup_overlaps_the_parent_and_drains_before_normal_or_unwound_return() {
        for unwind in [false, true] {
            let (started, start) = mpsc::channel();
            let (released, release) = mpsc::channel();
            let release = std::sync::Mutex::new(release);
            let pool = Pool::with_test_runner(3, move |job| {
                let Job::VariantPart(Request {
                    part: Work::Probe(calls),
                    ..
                }) = job
                else {
                    unreachable!()
                };
                started.send(()).unwrap();
                release
                    .lock()
                    .unwrap()
                    .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                    .unwrap();
                calls.fetch_add(1, Ordering::Release);
                Report::VariantPart
            });
            let mut dispatch = Dispatch::default();
            dispatch.arm(&pool);
            let calls = Arc::new(AtomicUsize::new(0));
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                dispatch.during(Work::Probe(Arc::clone(&calls)), &NoReads, 0, &mut || {
                    start
                        .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                        .unwrap();
                    assert_eq!(calls.load(Ordering::Acquire), 0);
                    released.send(()).unwrap();
                    assert!(!unwind, "synthetic context unwind");
                });
            }));
            assert_eq!(result.is_err(), unwind);
            assert_eq!(calls.load(Ordering::Acquire), 1);
            assert_eq!(pool.calibration().executor.uncollected, 0);
            assert_eq!(pool.calibration.cold_parents.load(Ordering::Acquire), 0);
        }
    }
    #[test]
    fn work_beside_the_parent_runs_on_a_worker_meanwhile_and_comes_back() {
        let (started, start) = mpsc::channel();
        let (released, release) = mpsc::channel();
        let release = std::sync::Mutex::new(release);
        let pool = Pool::with_test_runner(3, move |job| {
            let Job::VariantPart(Request {
                part: Work::Probe(calls),
                ..
            }) = job
            else {
                unreachable!()
            };
            started.send(()).unwrap();
            release
                .lock()
                .unwrap()
                .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                .unwrap();
            calls.fetch_add(1, Ordering::Release);
            Report::VariantPart
        });
        let mut dispatch = Dispatch::default();
        dispatch.arm(&pool);
        let calls = Arc::new(AtomicUsize::new(0));
        let mut parent = 0;
        let returned = dispatch.beside(
            Work::Probe(Arc::clone(&calls)),
            &NoReads,
            0,
            collector_workers::hour_stages::Part::ProjectsRestWait,
            &mut || {
                start
                    .recv_timeout(collector_test_support::TEST_HANG_GUARD)
                    .unwrap();
                assert_eq!(calls.load(Ordering::Acquire), 0);
                parent += 1;
                released.send(()).unwrap();
            },
        );
        assert!(matches!(returned, Some(Work::Probe(_))));
        assert_eq!((parent, calls.load(Ordering::Acquire)), (1, 1));
        assert_eq!(pool.calibration().executor.uncollected, 0);
        assert_eq!(pool.calibration.cold_parents.load(Ordering::Acquire), 0);
        let single = Pool::with_test_runner(1, |_| unreachable!());
        let mut alone = Dispatch::default();
        alone.arm(&single);
        let calls = Arc::new(AtomicUsize::new(0));
        let returned = alone.beside(
            Work::Probe(Arc::clone(&calls)),
            &NoReads,
            0,
            collector_workers::hour_stages::Part::ProjectsRestWait,
            &mut || assert_eq!(calls.load(Ordering::Acquire), 1),
        );
        assert!(matches!(returned, Some(Work::Probe(_))));
    }

    #[test]
    fn optional_setup_ticket_denial_keeps_local_setup_and_parent_work_nonterminal() {
        let pool = Pool::with_test_runner(3, |_| unreachable!());
        let mut dispatch = Dispatch::default();
        dispatch.arm(&pool);
        let calls = Arc::new(AtomicUsize::new(0));
        collector_test_support::with_allocations_denied(|| {
            dispatch.during(Work::Probe(Arc::clone(&calls)), &NoReads, 0, &mut || {
                assert_eq!(calls.load(Ordering::Acquire), 1);
            });
        });
        assert_eq!(pool.calibration().executor.uncollected, 0);
        assert_eq!(pool.calibration.cold_parents.load(Ordering::Acquire), 0);
        assert_eq!(
            super::jobs_per_hour(Role::Variants),
            2 * collector_workers::acquisition::MAX_WORKERS
        );
    }
}
