use super::*;

pub struct ProjectDispatch<'a> {
    pub lanes: usize,
    pub acquire: &'a mut dyn FnMut(Vec<ProjectPart>) -> Result<Vec<ProjectPart>, &'static str>,
}
pub struct ProjectPart {
    ordinal: usize,
    owners: Vec<u32>,
    projects: Vec<nproject_cproject::Row>,
    history: Vec<nproject_sproject_history::Row>,
    outcomes: (Outcome, Outcome),
    census_error: Option<&'static str>,
    runtime: runtime::Runtime,
    acquire_runtime: bool,
    runtime_phase: bool,
    acquired: bool,
    runtime_done: bool,
}
impl ProjectPart {
    pub fn capture(&mut self, memory: &impl Memory, base: usize, game_state: usize) {
        let _reads = read_accounting::table(TABLES[1]);
        if self.runtime_phase {
            self.runtime_done = false;
            self.runtime.acquire(
                memory,
                base,
                &self.projects[..self.outcomes.0.len()],
                true,
                false,
                None,
            );
            self.runtime_done = true;
            return;
        }
        self.acquired = false;
        let report = nproject_cproject::walk_partition(
            memory,
            base,
            game_state,
            engine_native_memory::walk::Buffer::Growing(&mut self.projects),
            engine_native_memory::walk::Buffer::Growing(&mut self.history),
            &mut |id| self.owners.binary_search(&id).is_ok(),
        );
        self.outcomes = report.outcomes;
        self.census_error = report.census_error;
        self.acquired = true;
    }
    pub fn runtime_phase(&self) -> bool {
        self.runtime_phase
    }

    fn complete(&self) -> bool {
        self.acquired
            && (!self.runtime_phase || self.runtime_done)
            && matches!(self.outcomes, (Outcome::Rows { .. }, Outcome::Rows { .. }))
    }
}
pub(super) struct Acquired {
    parts: Vec<ProjectPart>,
    failure: Option<&'static str>,
}
impl Acquired {
    fn refused(reason: &'static str) -> Self {
        Self {
            parts: Vec::new(),
            failure: Some(reason),
        }
    }
    pub(super) fn runtime_acquired(&self) -> bool {
        self.parts.first().is_some_and(|part| part.acquire_runtime)
    }
    pub(super) fn join(
        mut self,
        projects: &mut Vec<nproject_cproject::Row>,
        history: &mut Vec<nproject_sproject_history::Row>,
        runtime: &mut runtime::Runtime,
    ) -> (Outcome, Outcome) {
        projects.clear();
        history.clear();
        let census_error = self
            .failure
            .or_else(|| self.parts.iter().find_map(|part| part.census_error));
        if let Some(reason) = census_error {
            return (
                Outcome::Unavailable { reason },
                Outcome::Unavailable { reason },
            );
        }
        let first_error = |table: usize| {
            self.parts.iter().find_map(|part| {
                let outcome = if table == 0 {
                    part.outcomes.0
                } else {
                    part.outcomes.1
                };
                match outcome {
                    Outcome::Unavailable { reason } => Some(reason),
                    _ => None,
                }
            })
        };
        let mut project_error = first_error(0);
        let mut history_error = first_error(1);
        let result = (|| {
            let project_count = self.parts.iter().map(|part| part.outcomes.0.len()).sum();
            let history_count = self.parts.iter().map(|part| part.outcomes.1.len()).sum();
            engine_allocation::reserve(projects, project_count)
                .map_err(|_| "project row join allocation failed")?;
            engine_allocation::reserve(history, history_count)
                .map_err(|_| "project history join allocation failed")?;
            let (held, pending) = self
                .parts
                .iter()
                .filter(|part| part.acquire_runtime)
                .try_fold((0usize, 0usize), |(held, pending), part| {
                    let (more_held, more_pending) = part.runtime.cold_counts();
                    Ok::<_, &'static str>((
                        held.checked_add(more_held)
                            .ok_or("project runtime join count overflow")?,
                        pending
                            .checked_add(more_pending)
                            .ok_or("project runtime join count overflow")?,
                    ))
                })?;
            runtime.reserve_cold(held, pending)?;
            for part in &mut self.parts {
                projects.extend(part.projects.drain(..part.outcomes.0.len()));
                history.extend(part.history.drain(..part.outcomes.1.len()));
                if part.acquire_runtime {
                    runtime.append_cold(&mut part.runtime)?;
                }
            }
            projects.sort_unstable_by_key(|row| row.key());
            history.sort_unstable_by_key(|row| (row.owner, row.project, row.ordinal));
            runtime.order_pending();
            if project_error.is_none()
                && projects
                    .windows(2)
                    .any(|pair| pair[0].key() == pair[1].key())
            {
                project_error = Some("duplicate country-local project id");
            }
            if history_error.is_none()
                && history.windows(2).any(|pair| {
                    (pair[0].owner, pair[0].project, pair[0].ordinal)
                        == (pair[1].owner, pair[1].project, pair[1].ordinal)
                })
            {
                history_error = Some("duplicate country-local project history row");
            }
            Ok(())
        })();
        match result {
            Ok(()) => (
                project_error.map_or(
                    Outcome::Rows {
                        len: projects.len(),
                    },
                    |reason| Outcome::Unavailable { reason },
                ),
                history_error.map_or(Outcome::Rows { len: history.len() }, |reason| {
                    Outcome::Unavailable { reason }
                }),
            ),
            Err(reason) => (
                Outcome::Unavailable { reason },
                Outcome::Unavailable { reason },
            ),
        }
    }
}
pub(super) fn capture(
    _memory: &impl Memory,
    _base: usize,
    _state: usize,
    read: &[(u32, u32)],
    empty_runtime: bool,
    dispatch: Option<ProjectDispatch<'_>>,
) -> Option<Acquired> {
    let dispatch = dispatch.filter(|dispatch| dispatch.lanes > 1 && read.len() >= 32)?;
    let parts = engine_allocation::advisory(|| {
        let owners = engine_allocation::collect_vec(
            read.chunk_by(|a, b| a.0 == b.0).map(|group| group[0].0),
        )
        .ok()?;
        if owners.len() < 2 {
            return None;
        }
        let width = owners.len().div_ceil(dispatch.lanes);
        let mut parts = engine_allocation::vec(owners.len().div_ceil(width)).ok()?;
        for (ordinal, owners) in owners.chunks(width).enumerate() {
            let mut owners = copy_vec(owners).ok()?;
            owners.sort_unstable();
            owners.dedup();
            parts.push(ProjectPart {
                ordinal,
                owners,
                projects: Vec::new(),
                history: Vec::new(),
                outcomes: (Outcome::Rows { len: 0 }, Outcome::Rows { len: 0 }),
                census_error: None,
                runtime: runtime::Runtime::default(),
                acquire_runtime: false,
                runtime_phase: false,
                acquired: false,
                runtime_done: false,
            });
        }
        Some(parts)
    })?;
    let count = parts.len();
    let mut parts = match (dispatch.acquire)(parts) {
        Ok(parts) => parts,
        Err(reason) => return Some(Acquired::refused(reason)),
    };
    parts.sort_unstable_by_key(|part| part.ordinal);
    if parts.len() != count
        || parts
            .iter()
            .enumerate()
            .any(|(i, part)| i != part.ordinal || !part.acquired)
    {
        return Some(Acquired::refused(
            "project partition acquisition incomplete",
        ));
    }
    if parts.iter().any(|part| !part.complete()) {
        return Some(Acquired {
            parts,
            failure: None,
        });
    }
    let valid = engine_allocation::advisory(|| {
        let mut keys = Vec::new();
        for part in &parts {
            engine_allocation::reserve(&mut keys, part.outcomes.0.len()).ok()?;
            keys.extend(
                part.projects[..part.outcomes.0.len()]
                    .iter()
                    .map(|row| (row.key(), row.source_pointer)),
            );
        }
        keys.sort_unstable();
        if keys.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return None;
        }
        keys.sort_unstable_by_key(|entry| entry.1);
        Some(!keys.windows(2).any(|pair| pair[0].1 == pair[1].1))
    });
    let unique_stores = valid.unwrap_or(false);
    if empty_runtime && unique_stores && prepare_runtime(&mut parts) {
        parts = match (dispatch.acquire)(parts) {
            Ok(parts) => parts,
            Err(reason) => return Some(Acquired::refused(reason)),
        };
        parts.sort_unstable_by_key(|part| part.ordinal);
        if parts.len() != count
            || parts
                .iter()
                .enumerate()
                .any(|(i, part)| part.ordinal != i || !part.complete() || !part.runtime_phase)
        {
            return Some(Acquired::refused(
                "project runtime partition acquisition incomplete",
            ));
        }
    }
    Some(Acquired {
        parts,
        failure: None,
    })
}
fn prepare_runtime(parts: &mut [ProjectPart]) -> bool {
    let Some(mut rows) = engine_allocation::advisory(|| {
        engine_allocation::collect_vec(
            parts
                .iter()
                .flat_map(|part| &part.projects[..part.outcomes.0.len()]),
        )
        .ok()
    }) else {
        return false;
    };
    rows.sort_unstable_by_key(|row| row.key());
    for row in rows {
        let Some(store) = row.source_pointer.checked_add(nproject_cproject::VARIABLES) else {
            return false;
        };
        if collector_observers::project_variable_changes::register(store, row.owner, row.local_id)
            .is_none()
        {
            return false;
        }
    }
    for part in parts {
        part.acquire_runtime = true;
        part.runtime_phase = true;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::super::{projects_test_kit as kit, scale_tests::space};
    use super::*;
    fn capture(
        memory: &(impl Memory + Sync),
        state: usize,
        lanes: usize,
    ) -> Vec<([TableView; 2], Vec<(bool, bool)>, bool)> {
        let mut captured = Captured::vacant();
        let mut store = SliceFingerprints::new();
        let changes = crate::country_input_changes::Domain::enabled(100_000);
        let mut output = Vec::new();
        for hour in [1, 2, 24] {
            let mut acquire = |parts: Vec<ProjectPart>| {
                Ok(std::thread::scope(|scope| {
                    let jobs: Vec<_> = parts
                        .into_iter()
                        .map(|mut part| {
                            scope.spawn(move || {
                                part.capture(memory, kit::BASE, state);
                                part
                            })
                        })
                        .collect();
                    jobs.into_iter()
                        .rev()
                        .map(|job| job.join().unwrap())
                        .collect()
                }))
            };
            captured.capture_project_family(
                memory,
                kit::BASE,
                state,
                hour,
                hour == 1,
                &mut store,
                &changes,
                true,
                (lanes > 1).then_some(ProjectDispatch {
                    lanes,
                    acquire: &mut acquire,
                }),
            );
            captured.decode_owned();
            let [_, projects, history, ..] = captured.tables();
            output.push((
                [projects, history],
                (1..100_000)
                    .map(|id| {
                        (
                            captured.project_selection.live(id),
                            captured.project_selection.contains(id),
                        )
                    })
                    .collect(),
                captured.project_changed.certified,
            ));
        }
        output
    }
    #[test]
    fn uneven_and_fifteen_hundred_country_parts_match_serial_aliases_rows_and_certificates() {
        for count in [35, 67, 1500] {
            let world = space(count, 5);
            let expected = capture(&world.memory(), world.game_state, 1);
            assert!(matches!(expected[0].0[0], TableView::Complete(_)));
            for lanes in [2, 3, 15] {
                assert_eq!(capture(&world.memory(), world.game_state, lanes), expected);
            }
        }
    }
    #[test]
    fn refused_country_pool_or_history_uses_the_serial_failure_precedence() {
        struct Fault<'a> {
            inner: engine_native_memory::memory::SliceMemory<'a>,
            at: usize,
        }
        impl Memory for Fault<'_> {
            fn copy(&self, at: usize, out: &mut [u8]) -> bool {
                !(at <= self.at && self.at < at.saturating_add(out.len()))
                    && self.inner.copy(at, out)
            }
        }
        let world = space(67, 5);
        let mut rows = Vec::new();
        let mut history = Vec::new();
        let (outcome, _) = nproject_cproject::walk_with_history_selected(
            &world.memory(),
            kit::BASE,
            world.game_state,
            engine_native_memory::walk::Buffer::Growing(&mut rows),
            engine_native_memory::walk::Buffer::Growing(&mut history),
            &mut |_| true,
        );
        rows.truncate(outcome.len());
        for at in [
            rows[2].source_pointer,
            rows[rows.len() / 2].source_pointer + nproject_cproject::HISTORY,
            rows.last().unwrap().source_pointer + nproject_cproject::STARTED,
        ] {
            let memory = Fault {
                inner: world.memory(),
                at,
            };
            assert_eq!(
                capture(&memory, world.game_state, 15),
                capture(&memory, world.game_state, 1)
            );
        }
    }
    #[test]
    fn one_shot_lifecycle_and_history_failures_never_retry_and_keep_independent_tables() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct OneShot<'a> {
            inner: engine_native_memory::memory::SliceMemory<'a>,
            at: usize,
            reads: AtomicUsize,
        }
        impl Memory for OneShot<'_> {
            fn copy(&self, at: usize, out: &mut [u8]) -> bool {
                if at <= self.at
                    && self.at < at.saturating_add(out.len())
                    && self.reads.fetch_add(1, Ordering::Relaxed) == 0
                {
                    return false;
                }
                self.inner.copy(at, out)
            }
        }
        let world = space(67, 5);
        let mut rows = Vec::new();
        let mut history = Vec::new();
        let (outcome, _) = nproject_cproject::walk_with_history_selected(
            &world.memory(),
            kit::BASE,
            world.game_state,
            engine_native_memory::walk::Buffer::Growing(&mut rows),
            engine_native_memory::walk::Buffer::Growing(&mut history),
            &mut |_| true,
        );
        rows.truncate(outcome.len());
        let history_pointer = rows
            .iter()
            .find_map(|row| {
                let count = engine_native_memory::read::<u32>(
                    &world.memory(),
                    row.source_pointer + nproject_cproject::HISTORY_COUNT,
                )?;
                (count != 0)
                    .then(|| {
                        engine_native_memory::read::<u64>(
                            &world.memory(),
                            row.source_pointer + nproject_cproject::HISTORY,
                        )
                    })
                    .flatten()
            })
            .unwrap() as usize;
        for (at, table) in [
            (
                rows[rows.len() / 2].source_pointer + nproject_cproject::STARTED,
                0,
            ),
            (history_pointer, 1),
        ] {
            let serial = OneShot {
                inner: world.memory(),
                at,
                reads: AtomicUsize::new(0),
            };
            let parallel = OneShot {
                inner: world.memory(),
                at,
                reads: AtomicUsize::new(0),
            };
            let expected = capture(&serial, world.game_state, 1);
            let actual = capture(&parallel, world.game_state, 15);
            assert_eq!(actual, expected);
            assert!(matches!(actual[0].0[table], TableView::Unavailable { .. }));
            assert!(matches!(actual[0].0[1 - table], TableView::Complete(_)));
            assert_eq!(
                parallel.reads.load(Ordering::Relaxed),
                serial.reads.load(Ordering::Relaxed)
            );
        }
    }
    #[test]
    fn a_later_one_shot_native_failure_precedes_global_duplicate_validation() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct OneShot<'a> {
            inner: engine_native_memory::memory::SliceMemory<'a>,
            at: usize,
            reads: AtomicUsize,
        }
        impl Memory for OneShot<'_> {
            fn copy(&self, at: usize, out: &mut [u8]) -> bool {
                if at <= self.at
                    && self.at < at.saturating_add(out.len())
                    && self.reads.fetch_add(1, Ordering::Relaxed) == 0
                {
                    return false;
                }
                self.inner.copy(at, out)
            }
        }
        let mut world = space(67, 5);
        let mut projects = Vec::new();
        let (outcome, _) = nproject_cproject::walk_with_history_selected(
            &world.memory(),
            kit::BASE,
            world.game_state,
            engine_native_memory::walk::Buffer::Growing(&mut projects),
            engine_native_memory::walk::Buffer::Growing(&mut Vec::new()),
            &mut |_| true,
        );
        projects.truncate(outcome.len());
        let mut ordered: Vec<_> = {
            let memory = world.memory();
            let countries = ccountry::Reader::new(&memory, kit::BASE, world.game_state).unwrap();
            projects
                .into_iter()
                .map(|row| (countries.resolve(row.owner).unwrap().canonical_slot, row))
                .collect()
        };
        ordered.sort_unstable_by_key(|(slot, row)| (*slot, row.local_id));
        let pair = ordered
            .windows(2)
            .find(|pair| pair[0].0 == pair[1].0)
            .unwrap();
        let at = world.offset(pair[1].1.source_pointer + nproject_cproject::ID);
        world.bytes[at..at + 4].copy_from_slice(&pair[0].1.local_id.to_le_bytes());
        let fault = ordered.last().unwrap().1.source_pointer + nproject_cproject::STARTED;
        let memory = || OneShot {
            inner: world.memory(),
            at: fault,
            reads: AtomicUsize::new(0),
        };
        let expected = capture(&memory(), world.game_state, 1);
        assert!(
            matches!(&expected[0].0[0], TableView::Unavailable { reason } if reason == "project lifecycle unreadable")
        );
        assert_eq!(capture(&memory(), world.game_state, 15), expected);
    }
    #[test]
    fn missing_or_unrun_project_parts_refuse_without_a_serial_retry() {
        let world = space(67, 5);
        let memory = world.memory();
        let countries = ccountry::Reader::new(&memory, kit::BASE, world.game_state).unwrap();
        let read: Vec<_> = (1..countries.count())
            .filter_map(|slot| countries.at(slot).unwrap())
            .map(|c| (c.id, c.id))
            .collect();
        for unrun in [false, true] {
            let mut acquire = |mut parts: Vec<ProjectPart>| {
                if !unrun {
                    for part in &mut parts {
                        part.capture(&memory, kit::BASE, world.game_state);
                    }
                    parts.pop();
                }
                Ok(parts)
            };
            assert!(
                super::capture(
                    &memory,
                    kit::BASE,
                    world.game_state,
                    &read,
                    true,
                    Some(ProjectDispatch {
                        lanes: 15,
                        acquire: &mut acquire
                    })
                )
                .is_some()
            );
        }
    }
    #[test]
    fn required_project_part_allocation_denial_is_terminal() {
        const NAME: &str = "required_project_part_allocation_denial_is_terminal";
        const ENV: &str = "BATTLEPLAN_COLD_PROJECT_TEST";
        if std::env::var(ENV).ok().as_deref() != Some(NAME) {
            assert!(
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", &format!("projects::cold_parts::tests::{NAME}")])
                    .env(ENV, NAME)
                    .status()
                    .unwrap()
                    .success()
            );
            return;
        }
        fn failed() {
            crate::allocation::record_failure(
                crate::allocation::Phase::Extended,
                crate::allocation::Code::Reserve,
            );
        }
        let world = space(35, 5);
        let memory = world.memory();
        let countries = ccountry::Reader::new(&memory, kit::BASE, world.game_state).unwrap();
        let country = (1..countries.count())
            .find_map(|slot| countries.at(slot).unwrap())
            .unwrap();
        let mut part = ProjectPart {
            ordinal: 0,
            owners: vec![country.id],
            projects: Vec::new(),
            history: Vec::new(),
            outcomes: (Outcome::Rows { len: 0 }, Outcome::Rows { len: 0 }),
            census_error: None,
            runtime: runtime::Runtime::default(),
            acquire_runtime: false,
            runtime_phase: false,
            acquired: false,
            runtime_done: false,
        };
        crate::allocation::reset();
        engine_allocation::set_hooks(None, Some(failed));
        engine_allocation::set_stop_hook(Some(crate::allocation::failed));
        collector_test_support::with_allocations_denied(|| {
            part.capture(&memory, kit::BASE, world.game_state)
        });
        assert!(!part.complete());
        assert!(engine_allocation::capture_stopped());
        assert!(crate::allocation::failed());
    }
    #[test]
    fn advisory_plan_denial_leaves_serial_work_available() {
        let read: Vec<_> = (1..=67).map(|id| (id, id)).collect();
        let mut acquire =
            |_: Vec<ProjectPart>| panic!("denied optional plan must dispatch nothing");
        let world = space(67, 5);
        let result = collector_test_support::with_allocations_denied(|| {
            super::capture(
                &world.memory(),
                kit::BASE,
                world.game_state,
                &read,
                true,
                Some(ProjectDispatch {
                    lanes: 15,
                    acquire: &mut acquire,
                }),
            )
        });
        assert!(result.is_none());
    }
}

#[cfg(test)]
mod runtime_merge_tests {
    use super::super::variable_test_kit;
    use super::*;
    #[test]
    fn cold_runtime_parts_copy_nonempty_stores_before_mutation_and_freeze_in_key_order() {
        const NAME: &str =
            "cold_runtime_parts_copy_nonempty_stores_before_mutation_and_freeze_in_key_order";
        const ENV: &str = "BATTLEPLAN_PROJECT_COLD_RUNTIME_TEST";
        if std::env::var(ENV).ok().as_deref() != Some(NAME) {
            assert!(
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        &format!("projects::cold_parts::runtime_merge_tests::{NAME}")
                    ])
                    .env(ENV, NAME)
                    .status()
                    .unwrap()
                    .success()
            );
            return;
        }
        collector_observers::project_variable_changes::enabled(true);
        collector_observers::project_variable_changes::project_hooks_enabled(true);
        let fixtures = [
            variable_test_kit::Fixture::new(0x10000, 11),
            variable_test_kit::Fixture::new(0x20000, 22),
        ];
        let mut reference = Captured::vacant();
        let mut parts = Vec::new();
        for (index, fixture) in fixtures.iter().enumerate() {
            let mut row = fixture.row();
            row.owner += index as u32;
            reference.projects.rows.push(row);
            reference
                .project_runtime
                .read(fixture, 0, &[row], true, false, None);
            parts.push(ProjectPart {
                ordinal: index,
                owners: vec![row.owner],
                projects: vec![row],
                history: Vec::new(),
                outcomes: (Outcome::Rows { len: 1 }, Outcome::Rows { len: 0 }),
                census_error: None,
                runtime: runtime::Runtime::default(),
                acquire_runtime: false,
                runtime_phase: false,
                acquired: true,
                runtime_done: false,
            });
        }
        reference.projects.outcome = Outcome::Rows {
            len: fixtures.len(),
        };
        let TableView::Complete(rows::TableRows::Projects(rows)) = &reference.tables()[1] else {
            panic!("complete project rows");
        };
        for (index, row) in rows.iter().enumerate() {
            let rows::Recorded::Value(runtime) = &row.runtime else {
                panic!("runtime");
            };
            let variables = runtime.variables.as_ref().unwrap();
            assert_eq!(variables.seed, [11, 22][index]);
            assert_eq!(variables.entries.len(), 3);
        }
        let expected = variable_test_kit::encoded(&reference);
        collector_observers::project_variable_changes::retain_live(|_, _| false);
        assert!(prepare_runtime(&mut parts));
        let (parts, fixtures): (Vec<_>, Vec<_>) = std::thread::scope(|scope| {
            let jobs: Vec<_> = parts
                .into_iter()
                .zip(fixtures)
                .map(|(mut part, fixture)| {
                    scope.spawn(move || {
                        part.capture(&fixture, 0, 0);
                        assert!(part.complete());
                        assert_eq!(part.runtime.pending_count(), 1);
                        fixture.replace(99);
                        fixture.refuse.set(true);
                        (part, fixture)
                    })
                })
                .collect();
            jobs.into_iter()
                .rev()
                .map(|job| job.join().unwrap())
                .unzip()
        });
        let reads: Vec<_> = fixtures.iter().map(|fixture| fixture.reads.get()).collect();
        let mut captured = Captured::vacant();
        (captured.projects.outcome, captured.history.outcome) = Acquired {
            parts,
            failure: None,
        }
        .join(
            &mut captured.projects.rows,
            &mut captured.history.rows,
            &mut captured.project_runtime,
        );
        assert_eq!(captured.project_runtime.pending_count(), 2);
        captured.decode_owned();
        assert_eq!(variable_test_kit::encoded(&captured), expected);
        assert!(reads.iter().all(|&count| count > 0));
        assert_eq!(
            fixtures
                .iter()
                .map(|fixture| fixture.reads.get())
                .collect::<Vec<_>>(),
            reads
        );
        captured.reset();
        captured.decode_owned();
        assert_eq!(captured.project_runtime.pending_count(), 0);
        assert_eq!(
            fixtures
                .iter()
                .map(|fixture| fixture.reads.get())
                .collect::<Vec<_>>(),
            reads
        );
    }
}
