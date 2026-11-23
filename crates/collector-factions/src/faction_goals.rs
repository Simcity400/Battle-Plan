use crate::faction_goal_changes::Changes;
use engine_allocation as allocation;
use engine_native::{Memory, cfaction_goal_status as native};
use replay_data::{
    rows::FactionGoalEntryRow, rows::FactionGoalStatusRow, rows::RowCodec, rows::TableRows,
    rows::tables,
};
use replay_frame::frame::TableInput;
use std::sync::Arc;

pub const TABLES: [replay_data::rows::TableId; 2] = [
    replay_data::rows::TableId::FactionGoalStatus,
    replay_data::rows::TableId::FactionGoalEntries,
];

#[derive(Default)]
pub struct Captured {
    pub headers: Option<Arc<TableRows>>,
    pub entries: Option<Arc<TableRows>>,
    pub failure: Option<&'static str>,
}
impl Captured {
    pub fn inputs(&self) -> [TableInput; 2] {
        match (&self.headers, &self.entries, self.failure) {
            (Some(headers), Some(entries), None) => [
                TableInput::SharedComplete(Arc::clone(headers)),
                TableInput::SharedComplete(Arc::clone(entries)),
            ],
            _ => std::array::from_fn(|_| {
                collector_projection::table_conversion::unavailable(
                    self.failure
                        .unwrap_or("faction goal acquisition unavailable"),
                )
                .into()
            }),
        }
    }
}

#[derive(Default)]
struct Progress {
    certificate: Option<(u64, u64)>,
    failure: Option<&'static str>,
}

#[derive(Default)]
struct Held {
    pointer: usize,
    id: u64,
    certificate: Option<(u64, u64)>,
    failure: Option<&'static str>,
    seen: u64,
    data: native::Snapshot,
    progress: Vec<Progress>,
}

#[derive(Default)]
struct Tables {
    headers: Option<Arc<TableRows>>,
    entries: Option<Arc<TableRows>>,
    spare_headers: Vec<Arc<TableRows>>,
    spare_entries: Vec<Arc<TableRows>>,
}
pub fn writable(
    current: &mut Option<Arc<TableRows>>,
    spares: &mut Vec<Arc<TableRows>>,
    empty: impl FnOnce() -> TableRows,
) -> Result<Arc<TableRows>, &'static str> {
    if let Some(previous) = current.take() {
        allocation::reserve(spares, 1)
            .map_err(|_| "faction goal table retention allocation failed")?;
        spares.push(previous);
    }
    if let Some(index) = spares
        .iter()
        .position(|value| Arc::strong_count(value) == 1)
    {
        return Ok(spares.swap_remove(index));
    }
    Ok(Arc::new(empty()))
}

#[derive(Default)]
pub struct Store {
    held: Vec<Held>,
    tables: Tables,
    before: Option<(u64, u64)>,
    quiet: Option<(u64, u64)>,
    quiet_carried: bool,
    failure: Option<&'static str>,
    changed: bool,
    unpublished: bool,
    generation: u64,
    seen: usize,
    force: bool,
    pub read: usize,
    pub carried: usize,
    pub read_ticks: u64,
    invalidated: bool,
    bytes: usize,
}
impl Store {
    pub fn begin(&mut self, changes: &impl Changes, force: bool) {
        self.before = changes.quiet();
        self.failure = self
            .before
            .is_none()
            .then_some("faction goals could not be followed in the game");
        self.force = force || std::mem::take(&mut self.invalidated);
        self.changed = self.force;
        self.quiet_carried = false;
        self.read = 0;
        self.read_ticks = 0;
        self.carried = 0;
        self.seen = 0;
        if let Some(next) = self.generation.checked_add(1) {
            self.generation = next;
        } else {
            self.before = None;
            self.failure = Some("faction acquisition generation exhausted");
        }
    }
    pub fn invalidate(&mut self) {
        self.invalidated = true;
        self.quiet = None;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.bytes
    }
    pub fn visit(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: usize,
        id: u64,
        changes: &impl Changes,
    ) {
        if self.before.is_none() {
            return;
        }
        let index = match self.held.binary_search_by_key(&id, |held| held.id) {
            Ok(index) => index,
            Err(index) => {
                if allocation::reserve(&mut self.held, 1).is_err() {
                    self.failure
                        .get_or_insert("faction goal owner storage allocation failed");
                    return;
                }
                self.held.insert(
                    index,
                    Held {
                        pointer,
                        id,
                        ..Default::default()
                    },
                );
                self.changed = true;
                index
            }
        };
        let held = &mut self.held[index];
        if held.seen != self.generation {
            self.seen += 1;
            held.seen = self.generation;
        }
        let status = match pointer.checked_add(native::STATUS) {
            Some(status) => status,
            None => {
                self.failure
                    .get_or_insert("faction goal owner address overflow");
                return;
            }
        };
        let unchanged = !self.force
            && held.pointer == pointer
            && held.certificate.is_some()
            && (self.before == self.quiet || held.certificate == changes.sequence(status));
        if unchanged && self.before == self.quiet {
            self.quiet_carried = true;
            self.carried += 1;
            return;
        }
        let started = collector_primitives::clock::qpc();
        let _reads = collector_memory::read_accounting::table(
            replay_data::rows::TableId::FactionGoalEntries,
        );
        let mut read = false;
        if !unchanged {
            self.changed = true;
            held.pointer = pointer;
            held.failure = None;
            held.certificate = None;
            held.progress.clear();
            let before = changes.sequence(status);
            if before.is_none() {
                self.failure
                    .get_or_insert("faction goal owner registration unavailable");
                return;
            }
            read = true;
            match native::read_structure_into(memory, base, pointer, &mut held.data) {
                Ok(()) if before == changes.sequence(status) => {
                    held.certificate = before;
                }
                Ok(()) => {
                    self.failure
                        .get_or_insert("faction goal writer changed during acquisition");
                }
                Err(reason) => {
                    if before == changes.sequence(status) {
                        held.certificate = before;
                        held.failure = Some(reason);
                    }
                    self.failure.get_or_insert(reason);
                }
            }
        }
        if let Some(reason) = held.failure {
            self.failure.get_or_insert(reason);
        } else if held.certificate.is_some() {
            if allocation::resize_default(&mut held.progress, held.data.goals.len()).is_err() {
                self.failure
                    .get_or_insert("faction goal cache certificate allocation failed");
            } else if !unchanged || self.before != self.quiet {
                for (ordinal, progress) in held.progress.iter_mut().enumerate() {
                    let receiver =
                        held.data.goal_addresses[ordinal].checked_add(native::GOAL_PROGRESS_STATUS);
                    let before = receiver.and_then(|receiver| changes.sequence(receiver));
                    if before.is_none() {
                        self.failure
                            .get_or_insert("faction goal progress registration unavailable");
                        continue;
                    }
                    if progress.certificate != before {
                        self.changed = true;
                        read = true;
                        progress.certificate = None;
                        progress.failure = None;
                        let result = native::read_progress_into(
                            memory,
                            held.data.goal_addresses[ordinal],
                            &mut held.data.goals[ordinal],
                            &mut held.data.active_sections[ordinal],
                        );
                        if before == receiver.and_then(|receiver| changes.sequence(receiver)) {
                            progress.certificate = before;
                            progress.failure = result.err();
                        } else {
                            self.failure
                                .get_or_insert("faction goal progress changed during acquisition");
                        }
                    }
                    if let Some(reason) = progress.failure {
                        self.failure.get_or_insert(reason);
                    }
                }
                if changes.sequence(status) != held.certificate {
                    self.failure
                        .get_or_insert("faction goal writer changed during acquisition");
                }
            }
        }
        if read {
            self.read += 1;
            self.read_ticks += collector_primitives::clock::qpc()
                .saturating_sub(started)
                .max(0) as u64;
        } else {
            self.carried += 1;
        }
    }

    /// # Safety
    /// Caller holds hourly capture admission for the active session (or isolated fixture control)
    /// so global `faction_goal_changes` sizing demand updates belong to this pass.
    pub unsafe fn finish(&mut self, complete: bool, changes: &impl Changes, output: &mut Captured) {
        self.unpublished |= self.changed;
        // SAFETY: Admitted faction-goal capture records measured owner demand.
        unsafe {
            crate::faction_goal_changes::demand(
                self.held.iter().fold(self.held.len(), |count, held| {
                    count.saturating_add(held.data.goals.len())
                }),
            );
        }
        if !complete {
            self.failure
                .get_or_insert("faction goal identity roster incomplete");
        }
        if self.before.is_none() || (self.quiet_carried && self.before != changes.quiet()) {
            self.failure
                .get_or_insert("faction goals changed in the game while they were read");
        }
        if let Some(reason) = self.failure {
            self.quiet = None;
            *output = Captured {
                failure: Some(reason),
                ..Default::default()
            };
            return;
        }
        if self.seen != self.held.len() {
            self.changed = true;
            self.held.retain(|held| held.seen == self.generation);
        }
        self.unpublished |= self.changed;
        self.changed = self.unpublished;
        if self.unpublished || self.tables.headers.is_none() || self.tables.entries.is_none() {
            if let Err(reason) = self.rebuild() {
                self.quiet = None;
                *output = Captured {
                    failure: Some(reason),
                    ..Default::default()
                };
                return;
            }
            self.unpublished = false;
        }
        self.quiet = self.before;
        if self.changed {
            self.bytes = self.held.capacity() * size_of::<Held>()
                + self
                    .held
                    .iter()
                    .map(|held| {
                        held.data.goals.capacity() * size_of::<native::Goal>()
                            + held.data.goal_addresses.capacity() * size_of::<usize>()
                            + held.progress.capacity() * size_of::<Progress>()
                            + held.data.active_sections.capacity() * size_of::<Vec<u32>>()
                            + held
                                .data
                                .active_sections
                                .iter()
                                .map(|values| values.capacity() * size_of::<u32>())
                                .sum::<usize>()
                            + (held.data.completed.capacity() + held.data.cancelled.capacity())
                                * size_of::<Option<u32>>()
                            + (held.data.extra_slots.capacity() + held.data.term_dates.capacity())
                                * size_of::<i32>()
                    })
                    .sum::<usize>()
                + self
                    .tables
                    .headers
                    .iter()
                    .chain(self.tables.entries.iter())
                    .chain(self.tables.spare_headers.iter())
                    .chain(self.tables.spare_entries.iter())
                    .map(|rows| size_of::<TableRows>() + rows.allocated_bytes())
                    .sum::<usize>()
                + (self.tables.spare_headers.capacity() + self.tables.spare_entries.capacity())
                    * size_of::<Arc<TableRows>>();
        }
        *output = Captured {
            headers: self.tables.headers.as_ref().map(Arc::clone),
            entries: self.tables.entries.as_ref().map(Arc::clone),
            failure: None,
        };
    }
    fn rebuild(&mut self) -> Result<(), &'static str> {
        let mut headers = writable(
            &mut self.tables.headers,
            &mut self.tables.spare_headers,
            || TableRows::FactionGoalStatus(Vec::new()),
        )?;
        let mut entries = writable(
            &mut self.tables.entries,
            &mut self.tables.spare_entries,
            || TableRows::FactionGoalEntries(Vec::new()),
        )?;
        let headers_rows = Arc::get_mut(&mut headers)
            .unwrap()
            .rows_mut::<tables::FactionGoalStatus>()
            .unwrap();
        let entries_rows = Arc::get_mut(&mut entries)
            .unwrap()
            .rows_mut::<tables::FactionGoalEntries>()
            .unwrap();
        headers_rows.clear();
        allocation::reserve(headers_rows, self.held.len())
            .map_err(|_| "faction manifest rows allocation failed")?;
        let count = self
            .held
            .iter()
            .try_fold(0_usize, |count, held| {
                [
                    &held.data.goals.len(),
                    &held.data.completed.len(),
                    &held.data.cancelled.len(),
                    &held.data.extra_slots.len(),
                    &held.data.term_dates.len(),
                ]
                .into_iter()
                .try_fold(count, |count, size| count.checked_add(*size))
            })
            .ok_or("faction goal occurrence count overflow")?;
        allocation::resize_default(entries_rows, count)
            .map_err(|_| "faction goal occurrence rows allocation failed")?;
        fn write<'a>(
            rows: &'a mut [FactionGoalEntryRow],
            next: &mut usize,
            value: FactionGoalEntryRow,
        ) -> &'a mut FactionGoalEntryRow {
            let target = &mut rows[*next];
            *next += 1;
            let sections = std::mem::take(&mut target.active_sections);
            *target = value;
            target.active_sections = sections;
            target.active_sections.clear();
            target
        }
        let mut next = 0;
        for held in &self.held {
            headers_rows.push(FactionGoalStatusRow {
                faction: held.id,
                manifest: held.data.manifest,
            });
            for (ordinal, goal) in held.data.goals.iter().enumerate() {
                let target = write(
                    entries_rows,
                    &mut next,
                    FactionGoalEntryRow {
                        faction: held.id,
                        kind: 0,
                        ordinal: ordinal as u32,
                        token: Some(goal.token),
                        status: goal.status,
                        progress_raw: goal.progress_raw,
                        active_sections: Vec::new(),
                        date: goal.date,
                        slots: 0,
                    },
                );
                let source = &held.data.active_sections[ordinal];
                allocation::reserve(&mut target.active_sections, source.len())
                    .map_err(|_| "faction goal active section allocation failed")?;
                target.active_sections.extend_from_slice(source);
            }
            for (kind, list) in [(1, &held.data.completed), (2, &held.data.cancelled)] {
                for (ordinal, &token) in list.iter().enumerate() {
                    write(
                        entries_rows,
                        &mut next,
                        FactionGoalEntryRow {
                            faction: held.id,
                            kind,
                            ordinal: ordinal as u32,
                            token,
                            ..Default::default()
                        },
                    );
                }
            }
            for (ordinal, &slots) in held.data.extra_slots.iter().enumerate() {
                write(
                    entries_rows,
                    &mut next,
                    FactionGoalEntryRow {
                        faction: held.id,
                        kind: 3,
                        ordinal: ordinal as u32,
                        slots,
                        ..Default::default()
                    },
                );
            }
            for (ordinal, &date) in held.data.term_dates.iter().enumerate() {
                write(
                    entries_rows,
                    &mut next,
                    FactionGoalEntryRow {
                        faction: held.id,
                        kind: 4,
                        ordinal: ordinal as u32,
                        date,
                        ..Default::default()
                    },
                );
            }
        }
        headers_rows.sort_unstable_by_key(RowCodec::key);
        entries_rows.sort_unstable_by_key(RowCodec::key);
        self.tables.headers = Some(headers);
        self.tables.entries = Some(entries);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use collector_certificates::watch_registry::WatchRegistry;
    use engine_native::memory::SliceMemory;
    use std::cell::Cell;
    use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
    const FIRST: usize = 0x1000;
    const STRIDE: usize = 0x2000;
    struct ChangesTest(WatchRegistry);
    impl Changes for ChangesTest {
        fn quiet(&self) -> Option<(u64, u64)> {
            self.0.quiet()
        }
        fn sequence(&self, owner: usize) -> Option<(u64, u64)> {
            self.0.sequence(owner)
        }
    }
    struct Counted<'a> {
        bytes: &'a [u8],
        calls: Cell<usize>,
    }
    impl Memory for Counted<'_> {
        fn copy(&self, at: usize, output: &mut [u8]) -> bool {
            self.calls.set(self.calls.get() + 1);
            SliceMemory {
                base: 0,
                bytes: self.bytes,
            }
            .copy(at, output)
        }
    }
    fn fixture(owners: usize) -> Vec<u8> {
        let mut bytes = vec![0; FIRST + STRIDE * owners];
        for owner in 0..owners {
            let pointer = FIRST + owner * STRIDE;
            let at = pointer + native::STATUS;
            bytes[at..at + 8].copy_from_slice(&(native::VTABLE_RVA.0 as u64).to_le_bytes());
            bytes[at + native::OWNER..at + native::OWNER + 8]
                .copy_from_slice(&(pointer as u64).to_le_bytes());
        }
        bytes
    }
    fn changes(owners: usize) -> ChangesTest {
        let watches = WatchRegistry::new();
        watches.offer(owners);
        watches.boundary(false);
        watches.set_enabled(true);
        ChangesTest(watches)
    }
    fn acquire(
        store: &mut Store,
        output: &mut Captured,
        memory: &impl Memory,
        changes: &impl Changes,
        owners: usize,
    ) {
        store.begin(changes, false);
        for owner in 0..owners {
            store.visit(memory, 0, FIRST + owner * STRIDE, owner as u64 + 1, changes);
        }
        // SAFETY: Unit test runs against an isolated fixture.
        unsafe { store.finish(true, changes, output) };
    }
    #[test]
    fn faction_goal_store_carries_owners_without_native_reads_or_allocations() {
        let bytes = fixture(40);
        let memory = Counted {
            bytes: &bytes,
            calls: Cell::new(0),
        };
        let changes = changes(40);
        let mut store = Store::default();
        let mut output = Captured::default();
        acquire(&mut store, &mut output, &memory, &changes, 40);
        assert_eq!(output.failure, None);
        assert_eq!(store.read, 40);
        let initial = memory.calls.get();
        let headers = output.headers.as_ref().unwrap().clone();
        collector_test_support::with_allocations_denied(|| {
            acquire(&mut store, &mut output, &memory, &changes, 40)
        });
        assert_eq!(memory.calls.get(), initial);
        assert_eq!(store.read, 0);
        assert_eq!(store.carried, 40);
        assert!(Arc::ptr_eq(&headers, output.headers.as_ref().unwrap()));
        drop(changes.0.notify(FIRST + 39 * STRIDE + native::STATUS));
        acquire(&mut store, &mut output, &memory, &changes, 40);
        assert_eq!(output.failure, None);
        assert_eq!(store.read, 1);
        assert_eq!(store.carried, 39);
        let one_owner_reads = memory.calls.get() - initial;
        assert!(one_owner_reads > 0 && one_owner_reads < 20);
    }
    #[test]
    fn faction_goal_store_carries_a_stable_native_refusal_until_its_writer_changes() {
        let mut bytes = fixture(1);
        bytes[FIRST + native::STATUS] ^= 1;
        let memory = Counted {
            bytes: &bytes,
            calls: Cell::new(0),
        };
        let changes = changes(1);
        let mut store = Store::default();
        let mut output = Captured::default();
        acquire(&mut store, &mut output, &memory, &changes, 1);
        let reason = output.failure;
        assert!(reason.is_some());
        let first = memory.calls.get();
        collector_test_support::with_allocations_denied(|| {
            acquire(&mut store, &mut output, &memory, &changes, 1);
        });
        assert_eq!(memory.calls.get(), first);
        assert_eq!(store.read, 0);
        assert_eq!(output.failure, reason);
        drop(changes.0.notify(FIRST + native::STATUS));
        acquire(&mut store, &mut output, &memory, &changes, 1);
        assert!(memory.calls.get() > first);
    }
    #[test]
    fn faction_goal_store_disabled_observation_refuses_without_native_reads() {
        let bytes = fixture(2);
        let memory = Counted {
            bytes: &bytes,
            calls: Cell::new(0),
        };
        let changes = changes(2);
        changes.0.set_enabled(false);
        let mut store = Store::default();
        let mut output = Captured::default();
        acquire(&mut store, &mut output, &memory, &changes, 2);
        assert!(output.failure.is_some());
        assert_eq!(memory.calls.get(), 0);
    }
    #[test]
    fn faction_goal_store_incomplete_roster_does_not_publish_deletions_or_stale_rows() {
        let bytes = fixture(2);
        let memory = Counted {
            bytes: &bytes,
            calls: Cell::new(0),
        };
        let changes = changes(2);
        let mut store = Store::default();
        let mut output = Captured::default();
        acquire(&mut store, &mut output, &memory, &changes, 2);
        store.begin(&changes, false);
        store.visit(&memory, 0, FIRST, 1, &changes);
        // SAFETY: Unit test runs against an isolated fixture.
        unsafe { store.finish(false, &changes, &mut output) };
        assert!(output.failure.is_some());
        acquire(&mut store, &mut output, &memory, &changes, 2);
        assert_eq!(output.failure, None);
        assert_eq!(
            output
                .headers
                .as_ref()
                .unwrap()
                .rows::<tables::FactionGoalStatus>()
                .unwrap()
                .len(),
            2
        );
    }
    fn goal_fixture(count: usize) -> (Vec<u8>, usize, usize) {
        let mut bytes = fixture(1);
        let goals = bytes.len();
        let definition = goals + count * native::GOAL_STRIDE;
        let sections = definition + 16;
        bytes.resize(sections + count * 4, 0);
        fn put(bytes: &mut [u8], at: usize, value: impl AsRef<[u8]>) {
            let value = value.as_ref();
            bytes[at..at + value.len()].copy_from_slice(value);
        }
        put(
            &mut bytes,
            definition,
            (native::DEFINITION_VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            definition + native::DEFINITION_TOKEN,
            77_u32.to_le_bytes(),
        );
        let header = FIRST + native::STATUS + native::GOALS;
        put(&mut bytes, header, (goals as u64).to_le_bytes());
        put(&mut bytes, header + 8, (count as u32).to_le_bytes());
        put(&mut bytes, header + 12, (count as u32).to_le_bytes());
        for ordinal in 0..count {
            let goal = goals + ordinal * native::GOAL_STRIDE;
            put(&mut bytes, goal, (definition as u64).to_le_bytes());
            let date = goal + native::GOAL_DATE;
            put(
                &mut bytes,
                date,
                (engine_native::cgame_date::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                date + engine_native::cgame_date::PERSISTENT_BASE,
                (engine_native::cgame_date::PERSISTENT_VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                goal + native::GOAL_PROGRESS_STATUS,
                ((sections + ordinal * 4) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                goal + native::GOAL_PROGRESS_STATUS + 8,
                1_u32.to_le_bytes(),
            );
            put(
                &mut bytes,
                goal + native::GOAL_PROGRESS_STATUS + 12,
                1_u32.to_le_bytes(),
            );
        }
        (bytes, goals, sections)
    }
    struct Selected<'a> {
        bytes: &'a [u8],
        goal: usize,
        section: usize,
        calls: Cell<usize>,
        fail: bool,
    }
    impl Memory for Selected<'_> {
        fn copy(&self, at: usize, output: &mut [u8]) -> bool {
            assert!(
                (at == self.goal + native::GOAL_PROGRESS && output.len() == 8)
                    || (at >= self.goal + native::GOAL_PROGRESS_STATUS
                        && at + output.len() <= self.goal + native::GOAL_PROGRESS_STATUS + 16)
                    || (at == self.section && output.len() == 4),
                "unselected native read at {at:x}, length {}",
                output.len()
            );
            self.calls.set(self.calls.get() + 1);
            !self.fail
                && SliceMemory {
                    base: 0,
                    bytes: self.bytes,
                }
                .copy(at, output)
        }
    }
    #[test]
    fn faction_goal_progress_selects_one_cache_without_allocations() {
        let (mut bytes, goals, sections) = goal_fixture(40);
        let changes = changes(41);
        let mut store = Store::default();
        let mut output = Captured::default();
        acquire(
            &mut store,
            &mut output,
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            &changes,
            1,
        );
        assert_eq!(output.failure, None);
        output = Captured::default();
        store.rebuild().unwrap();
        let goal = goals + 39 * native::GOAL_STRIDE;
        bytes[goal + native::GOAL_PROGRESS..goal + native::GOAL_PROGRESS + 8]
            .copy_from_slice(&123_i64.to_le_bytes());
        bytes[sections + 39 * 4..sections + 40 * 4].copy_from_slice(&7_u32.to_le_bytes());
        drop(changes.0.notify(goal + native::GOAL_PROGRESS_STATUS));
        let selected = Selected {
            bytes: &bytes,
            goal,
            section: sections + 39 * 4,
            calls: Cell::new(0),
            fail: false,
        };
        collector_test_support::with_allocations_denied(|| {
            acquire(&mut store, &mut output, &selected, &changes, 1)
        });
        assert_eq!(output.failure, None);
        assert!(selected.calls.get() > 0 && selected.calls.get() < 6);
        let rows = output
            .entries
            .as_ref()
            .unwrap()
            .rows::<tables::FactionGoalEntries>()
            .unwrap();
        assert_eq!(rows[39].progress_raw, 123);
        assert_eq!(rows[39].active_sections, [7]);
        assert_eq!(rows[0].progress_raw, 0);
        let first = selected.calls.get();
        acquire(&mut store, &mut output, &selected, &changes, 1);
        assert_eq!(selected.calls.get(), first);
        drop(changes.0.notify(FIRST + native::STATUS));
        acquire(
            &mut store,
            &mut output,
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            &changes,
            1,
        );
        assert_eq!(output.failure, None);
        assert_eq!(store.read, 1);
    }
    #[test]
    fn faction_goal_progress_failure_carries_until_that_cache_writer_changes() {
        let (bytes, goal, section) = goal_fixture(1);
        let changes = changes(2);
        let mut store = Store::default();
        let mut output = Captured::default();
        acquire(
            &mut store,
            &mut output,
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            &changes,
            1,
        );
        drop(changes.0.notify(goal + native::GOAL_PROGRESS_STATUS));
        let selected = Selected {
            bytes: &bytes,
            goal,
            section,
            calls: Cell::new(0),
            fail: true,
        };
        acquire(&mut store, &mut output, &selected, &changes, 1);
        let reason = output.failure;
        assert!(reason.is_some());
        let first = selected.calls.get();
        collector_test_support::with_allocations_denied(|| {
            acquire(&mut store, &mut output, &selected, &changes, 1)
        });
        assert_eq!(output.failure, reason);
        assert_eq!(selected.calls.get(), first);
        drop(changes.0.notify(goal + native::GOAL_PROGRESS_STATUS));
        acquire(&mut store, &mut output, &selected, &changes, 1);
        assert_eq!(selected.calls.get(), first + 1);
    }
    struct NoProgress<'a> {
        changes: &'a ChangesTest,
        status: usize,
    }
    impl Changes for NoProgress<'_> {
        fn quiet(&self) -> Option<(u64, u64)> {
            self.changes.quiet()
        }
        fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
            (receiver == self.status)
                .then(|| self.changes.sequence(receiver))
                .flatten()
        }
    }
    #[test]
    fn faction_goal_progress_registration_retry_does_not_repeat_structure() {
        let (bytes, goal, section) = goal_fixture(1);
        let changes = changes(2);
        let missing = NoProgress {
            changes: &changes,
            status: FIRST + native::STATUS,
        };
        let mut store = Store::default();
        let mut output = Captured::default();
        acquire(
            &mut store,
            &mut output,
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            &missing,
            1,
        );
        assert_eq!(
            output.failure,
            Some("faction goal progress registration unavailable")
        );
        let selected = Selected {
            bytes: &bytes,
            goal,
            section,
            calls: Cell::new(0),
            fail: false,
        };
        acquire(&mut store, &mut output, &selected, &missing, 1);
        assert_eq!(selected.calls.get(), 0);
        acquire(&mut store, &mut output, &selected, &changes, 1);
        assert_eq!(output.failure, None);
        assert!(selected.calls.get() > 0);
    }
    #[test]
    fn faction_goal_changed_rows_survive_an_incomplete_publication() {
        let (bytes, goal, section) = goal_fixture(1);
        let changes = changes(2);
        let mut store = Store::default();
        let mut output = Captured::default();
        acquire(
            &mut store,
            &mut output,
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            &changes,
            1,
        );
        store.held[0].data.goals[0].progress_raw = 789;
        store.begin(&changes, false);
        store.changed = true;
        // SAFETY: Unit test runs against an isolated fixture.
        unsafe { store.finish(false, &changes, &mut output) };
        let selected = Selected {
            bytes: &bytes,
            goal,
            section,
            calls: Cell::new(0),
            fail: false,
        };
        acquire(&mut store, &mut output, &selected, &changes, 1);
        assert_eq!(selected.calls.get(), 0);
        assert_eq!(
            output
                .entries
                .as_ref()
                .unwrap()
                .rows::<tables::FactionGoalEntries>()
                .unwrap()[0]
                .progress_raw,
            789
        );
    }
    #[test]
    fn faction_goal_owners_read_under_their_own_certificates_survive_other_writers_during_the_walk()
    {
        let (mut bytes, goals, sections) = goal_fixture(2);
        let goal = goals + native::GOAL_STRIDE;
        bytes[goal + native::GOAL_PROGRESS..goal + native::GOAL_PROGRESS + 8]
            .copy_from_slice(&123_i64.to_le_bytes());
        bytes[sections + 4..sections + 8].copy_from_slice(&7_u32.to_le_bytes());
        let memory = Counted {
            bytes: &bytes,
            calls: Cell::new(0),
        };
        let changes = changes(3);
        let unrelated = 0x10_0000;
        let mut store = Store::default();
        let mut output = Captured::default();
        store.begin(&changes, true);
        store.visit(&memory, 0, FIRST, 1, &changes);
        drop(changes.0.notify(unrelated));
        // SAFETY: Unit test runs against an isolated fixture.
        unsafe { store.finish(true, &changes, &mut output) };
        assert_eq!(output.failure, None);
        let headers = output
            .headers
            .as_ref()
            .unwrap()
            .rows::<tables::FactionGoalStatus>()
            .unwrap();
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].faction, 1);
        let entries = output
            .entries
            .as_ref()
            .unwrap()
            .rows::<tables::FactionGoalEntries>()
            .unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|row| row.faction == 1 && row.kind == 0));
        assert_eq!(entries[0].progress_raw, 0);
        assert_eq!(entries[1].progress_raw, 123);
        assert_eq!(entries[1].active_sections, [7]);

        acquire(&mut store, &mut output, &memory, &changes, 1);
        assert_eq!(output.failure, None);
        let calls = memory.calls.get();
        store.begin(&changes, false);
        store.visit(&memory, 0, FIRST, 1, &changes);
        assert_eq!(memory.calls.get(), calls);
        assert_eq!(store.carried, 1, "a quiet family carries the owner");
        drop(changes.0.notify(unrelated));
        // SAFETY: Unit test runs against an isolated fixture.
        unsafe { store.finish(true, &changes, &mut output) };
        assert_eq!(
            output.failure,
            Some("faction goals changed in the game while they were read")
        );
    }
    struct Interrupting<'a> {
        changes: &'a ChangesTest,
        at: usize,
        writer: usize,
        fired: AtomicBool,
    }
    impl Changes for Interrupting<'_> {
        fn quiet(&self) -> Option<(u64, u64)> {
            self.changes.quiet()
        }
        fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
            if receiver == self.at && !self.fired.swap(true, SeqCst) {
                drop(self.changes.0.notify(self.writer));
            }
            self.changes.sequence(receiver)
        }
    }
    #[test]
    fn faction_goal_structure_writer_before_progress_reads_refuses_a_forced_hour() {
        let (bytes, goals, _) = goal_fixture(2);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let changes = changes(3);
        let interrupting = Interrupting {
            changes: &changes,
            at: goals + native::GOAL_PROGRESS_STATUS,
            writer: FIRST + native::STATUS,
            fired: AtomicBool::new(false),
        };
        let mut store = Store::default();
        let mut output = Captured::default();
        store.begin(&interrupting, true);
        store.visit(&memory, 0, FIRST, 1, &interrupting);
        // SAFETY: Unit test runs against an isolated fixture.
        unsafe { store.finish(true, &interrupting, &mut output) };
        assert!(interrupting.fired.load(SeqCst));
        assert!(output.entries.is_none());
        assert_eq!(
            output.failure,
            Some("faction goal writer changed during acquisition")
        );
        acquire(&mut store, &mut output, &memory, &changes, 1);
        assert_eq!(output.failure, None);
    }
}

#[cfg(test)]
mod progress_buffer_tests {
    use super::*;
    #[test]
    fn faction_goal_changed_progress_reuses_nested_section_storage() {
        let mut store = Store::default();
        store.held.push(Held {
            id: 7,
            data: native::Snapshot {
                goals: vec![native::Goal {
                    token: 9,
                    ..Default::default()
                }],
                active_sections: vec![vec![1, 3, 7]],
                ..Default::default()
            },
            ..Default::default()
        });
        store.rebuild().unwrap();
        store.rebuild().unwrap();
        let pointer = store
            .tables
            .entries
            .as_ref()
            .unwrap()
            .rows::<tables::FactionGoalEntries>()
            .unwrap()[0]
            .active_sections
            .as_ptr();
        store.held[0].data.goals[0].progress_raw = 123;
        store.held[0].data.active_sections[0][1] = 5;
        collector_test_support::with_allocations_denied(|| store.rebuild().unwrap());
        let row = &store
            .tables
            .entries
            .as_ref()
            .unwrap()
            .rows::<tables::FactionGoalEntries>()
            .unwrap()[0];
        assert_eq!(row.progress_raw, 123);
        assert_eq!(row.active_sections, [1, 5, 7]);
        assert_eq!(row.active_sections.as_ptr(), pointer);
    }
}
