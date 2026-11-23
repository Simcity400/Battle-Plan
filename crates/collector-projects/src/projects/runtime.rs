use super::*;
use collector_observers::project_variable_changes;
use std::collections::HashMap;

#[derive(Default)]
struct PendingRaw {
    entries: Vec<engine_scripts::cvariables::Row>,
    names: Vec<u8>,
    entries_len: usize,
    seed: Option<u32>,
}

impl PendingRaw {
    fn allocated_bytes(&self) -> usize {
        size_of::<Self>()
            + self.entries.capacity() * size_of::<engine_scripts::cvariables::Row>()
            + self.names.capacity()
    }
}

#[derive(Default)]
struct Held {
    pointer: usize,
    stamp: Option<(u64, u64)>,
    pending_raw: Option<Box<PendingRaw>>,
    recorded_variables: Option<std::sync::Arc<rows::ProjectVariableStore>>,
    recorded_bytes: u32,
    variables_known: bool,
    freeze_pending: bool,
    queued: bool,
    pending_known: bool,
    pending: Option<u32>,
    pending_ordinal: Option<u32>,
    pending_stamp: Option<(u64, u64)>,
    iterations: i32,
}

impl Held {
    fn freeze(&mut self) {
        if !std::mem::take(&mut self.freeze_pending) {
            return;
        }
        let Some(raw) = self.pending_raw.take() else {
            self.variables_known = false;
            return;
        };
        let value = freeze_project_variables(
            &raw.entries[..raw.entries_len],
            &raw.names,
            raw.seed.unwrap(),
        );
        match value {
            Ok(value) => {
                let bytes = size_of::<rows::ProjectVariableStore>()
                    + 2 * size_of::<usize>()
                    + value.entries.capacity() * size_of::<rows::ProjectVariable>()
                    + value
                        .entries
                        .iter()
                        .map(|entry| entry.name.capacity())
                        .sum::<usize>();
                self.recorded_bytes = u32::try_from(bytes).unwrap_or(u32::MAX);
                self.recorded_variables = Some(value);
            }
            Err(_) => self.variables_known = false,
        }
    }

    fn allocated_bytes(&self) -> usize {
        self.pending_raw
            .as_ref()
            .map_or(0, |raw| raw.allocated_bytes())
            + self.recorded_bytes as usize
    }
}

#[derive(Default)]
pub(super) struct Runtime {
    held: HashMap<(u32, u32), Held>,
    heap_bytes: usize,
    pending: Vec<(u32, u32)>,
}

impl Runtime {
    pub(super) fn is_empty(&self) -> bool {
        self.held.is_empty() && self.pending.is_empty()
    }

    pub(super) fn cold_counts(&self) -> (usize, usize) {
        (self.held.len(), self.pending.len())
    }
    pub(super) fn reserve_cold(&mut self, held: usize, pending: usize) -> Result<(), &'static str> {
        engine_allocation::reserve_map(&mut self.held, held)
            .map_err(|_| "project runtime join allocation failed")?;
        engine_allocation::reserve(&mut self.pending, pending)
            .map_err(|_| "project runtime join allocation failed")
    }
    pub(super) fn append_cold(&mut self, other: &mut Self) -> Result<(), &'static str> {
        engine_allocation::reserve_map(&mut self.held, other.held.len())
            .map_err(|_| "project runtime join allocation failed")?;
        engine_allocation::reserve(&mut self.pending, other.pending.len())
            .map_err(|_| "project runtime join allocation failed")?;
        self.held.extend(other.held.drain());
        self.pending.append(&mut other.pending);
        self.heap_bytes += std::mem::take(&mut other.heap_bytes);
        Ok(())
    }

    pub(super) fn order_pending(&mut self) {
        self.pending.sort_unstable();
    }

    pub(super) fn read(
        &mut self,
        memory: &impl Memory,
        base: usize,
        rows: &[nproject_cproject::Row],
        force: bool,
        full_census: bool,
        selection: Option<&SliceSelection>,
    ) {
        self.read_impl(memory, base, rows, force, full_census, selection, false);
    }

    pub(super) fn acquire(
        &mut self,
        memory: &impl Memory,
        base: usize,
        rows: &[nproject_cproject::Row],
        force: bool,
        full_census: bool,
        selection: Option<&SliceSelection>,
    ) {
        self.read_impl(memory, base, rows, force, full_census, selection, true);
    }

    pub(super) fn decode(&mut self) {
        for key in self.pending.drain(..) {
            let Some(held) = self.held.get_mut(&key) else {
                continue;
            };
            held.queued = false;
            if !held.freeze_pending {
                continue;
            }
            self.heap_bytes -= held.allocated_bytes();
            held.freeze();
            if !held.variables_known {
                PROJECT_VARIABLE_REFUSALS.fetch_add(1, Relaxed);
            }
            self.heap_bytes += held.allocated_bytes();
        }
    }

    fn read_impl(
        &mut self,
        memory: &impl Memory,
        base: usize,
        rows: &[nproject_cproject::Row],
        force: bool,
        full_census: bool,
        selection: Option<&SliceSelection>,
        defer: bool,
    ) {
        let mut reader = project_variable_changes::stamp_reader();
        for row in rows {
            let key = (row.owner, row.local_id);
            if !self.held.contains_key(&key) {
                if engine_allocation::reserve_map(&mut self.held, 1).is_err() {
                    PROJECT_VARIABLE_REFUSALS.fetch_add(1, Relaxed);
                    PROJECT_REWARD_REFUSALS.fetch_add(1, Relaxed);
                    continue;
                }
                self.held.insert(key, Held::default());
            }
            let held = self.held.get_mut(&key).unwrap();
            self.heap_bytes -= held.allocated_bytes();
            (|| {
                let pointer = row.source_pointer;
                if pointer != held.pointer {
                    *held = Held {
                        pointer,
                        queued: held.queued,
                        ..Default::default()
                    };
                }
                let Some(store) = pointer.checked_add(nproject_cproject::VARIABLES) else {
                    held.variables_known = false;
                    held.pending_known = false;
                    held.freeze_pending = false;
                    return;
                };
                if reader.is_none() {
                    reader = project_variable_changes::stamp_reader();
                }
                let (stamp, pending_stamp) = match reader
                    .as_ref()
                    .and_then(|reader| reader.stamps(store, row.owner, row.local_id))
                {
                    Some(stamps) => stamps,
                    None => {
                        reader = None;
                        (
                            project_variable_changes::register(store, row.owner, row.local_id),
                            project_variable_changes::pending_stamp(store),
                        )
                    }
                };
                if pending_stamp.is_none() {
                    held.pending_known = false;
                    PROJECT_REWARD_REFUSALS.fetch_add(1, Relaxed);
                } else if force
                    || !held.pending_known
                    || pending_stamp != held.pending_stamp
                    || row.state.iterations != held.iterations
                {
                    reader = None;
                    let pending = pointer
                        .checked_add(nproject_cproject::PENDING_REWARD)
                        .and_then(|address| engine_native_memory::read::<u64>(memory, address))
                        .and_then(|value| usize::try_from(value).ok());
                    held.pending_known = false;
                    if let Some(pending) = pending {
                        if pending == 0 {
                            held.pending = None;
                            held.pending_ordinal = None;
                            held.pending_known = true;
                        } else if let Some(header) =
                            engine_native_memory::read::<[u8; 12]>(memory, pending).filter(|header| {
                                engine_sites::at(base, nproject_cproject::REWARD_VTABLE_RVA)
                                    .is_some_and(|expected| {
                                        u64::from_le_bytes(header[..8].try_into().unwrap())
                                            == expected as u64
                                    })
                            })
                        {
                            let token = nproject_cproject::REWARD_TOKEN;
                            held.pending = Some(u32::from_le_bytes(
                                header[token..token + 4].try_into().unwrap(),
                            ));
                            held.pending_ordinal = nproject_cproject::pending_reward_ordinal(
                                memory, base, pointer, pending,
                            )
                            .ok();
                            held.pending_known = held.pending_ordinal.is_some();
                        }
                    }
                    held.pending_known &=
                        pending_stamp == project_variable_changes::pending_stamp(store);
                    held.pending_stamp = pending_stamp;
                    held.iterations = row.state.iterations;
                    if !held.pending_known {
                        PROJECT_REWARD_REFUSALS.fetch_add(1, Relaxed);
                    }
                }
                if stamp.is_none() {
                    held.variables_known = false;
                    held.freeze_pending = false;
                    PROJECT_VARIABLE_REFUSALS.fetch_add(1, Relaxed);
                    return;
                }
                if !force && held.variables_known && stamp == held.stamp {
                    return;
                }
                reader = None;
                held.variables_known = false;
                held.freeze_pending = false;
                if held.pending_raw.is_none() {
                    let Ok(raw) = engine_allocation::boxed(PendingRaw::default()) else {
                        PROJECT_VARIABLE_REFUSALS.fetch_add(1, Relaxed);
                        return;
                    };
                    held.pending_raw = Some(raw);
                }
                let raw = held.pending_raw.as_mut().unwrap();
                let read = engine_scripts::cvariables::read_store_carried(
                    memory,
                    base,
                    engine_scripts::cvariables::Scope::Global,
                    store,
                    engine_native_memory::walk::Buffer::Growing(&mut raw.entries),
                    engine_native_memory::walk::Buffer::Growing(&mut raw.names),
                    &mut engine_scripts::cvariables::Fresh,
                );
                if let Ok((read, seed)) = read
                    && let Outcome::Rows { len } = read.outcome
                {
                    raw.entries_len = len;
                    raw.seed = seed;
                    held.variables_known = seed.is_some()
                        && stamp.is_some()
                        && stamp
                            == project_variable_changes::register(store, row.owner, row.local_id);
                    held.stamp = stamp;
                    if held.variables_known {
                        if defer && !held.queued {
                            if engine_allocation::reserve_push_batch(&mut self.pending, 1).is_err()
                            {
                                held.variables_known = false;
                                held.pending_raw = None;
                                PROJECT_VARIABLE_REFUSALS.fetch_add(1, Relaxed);
                                return;
                            }
                            self.pending.push(key);
                            held.queued = true;
                        }
                        held.freeze_pending = true;
                        if !defer {
                            held.freeze();
                        }
                    }
                }
                if !held.variables_known {
                    held.pending_raw = None;
                    PROJECT_VARIABLE_REFUSALS.fetch_add(1, Relaxed);
                }
            })();
            self.heap_bytes += held.allocated_bytes();
        }
        drop(reader);
        if full_census {
            self.finish_census(rows, selection);
        }
    }

    pub(super) fn finish_census(
        &mut self,
        rows: &[nproject_cproject::Row],
        selection: Option<&SliceSelection>,
    ) {
        let heap_bytes = &mut self.heap_bytes;
        self.held.retain(|key, held| {
            let (live, read) = selection.map_or((true, true), |selection| {
                (selection.live(key.0), selection.contains(key.0))
            });
            let keep = live
                && (!read
                    || rows
                        .binary_search_by_key(key, |row| (row.owner, row.local_id))
                        .is_ok());
            if !keep {
                *heap_bytes -= held.allocated_bytes();
            }
            keep
        });
        project_variable_changes::retain_live(|owner, id| self.held.contains_key(&(owner, id)));
    }

    pub(super) fn value(
        &self,
        row: &nproject_cproject::Row,
    ) -> rows::Recorded<rows::ProjectRuntime> {
        let Some(held) = self
            .held
            .get(&(row.owner, row.local_id))
            .filter(|held| held.pointer == row.source_pointer)
        else {
            return rows::Recorded::Value(rows::ProjectRuntime::default());
        };
        rows::Recorded::Value(rows::ProjectRuntime {
            pending_known: held.pending_known,
            pending_reward: held.pending_known.then_some(held.pending).flatten(),
            pending_ordinal: held.pending_known.then_some(held.pending_ordinal).flatten(),
            variables: (held.variables_known && !held.freeze_pending)
                .then(|| std::sync::Arc::clone(held.recorded_variables.as_ref().unwrap())),
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn pending_count(&self) -> usize {
        self.pending.len()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn allocation_failure_fixture() -> Self {
        let held = Held {
            pending_raw: Some(Box::new(PendingRaw {
                entries: vec![engine_scripts::cvariables::Row {
                    name_start: 0,
                    name_len: 1,
                    value_raw: -1,
                }],
                names: vec![b'x'],
                entries_len: 1,
                seed: Some(7),
            })),
            variables_known: true,
            freeze_pending: true,
            queued: true,
            ..Default::default()
        };
        Self {
            heap_bytes: held.allocated_bytes(),
            held: [((5_000, 9), held)].into(),
            pending: vec![(5_000, 9)],
        }
    }

    pub(super) fn allocated_bytes(&self) -> usize {
        self.held.capacity() * (size_of::<(u32, u32)>() + size_of::<Held>())
            + self.heap_bytes
            + self.pending.capacity() * size_of::<(u32, u32)>()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::allocation;
    use std::cell::Cell;
    #[test]
    fn unavailable_runtime_keeps_an_encodable_explicit_refusal() {
        let runtime = Runtime::default();
        let row = rows::ProjectRow {
            runtime: runtime.value(&nproject_cproject::Row::default()),
            ..Default::default()
        };
        assert_eq!(
            row.runtime,
            rows::Recorded::Value(rows::ProjectRuntime::default())
        );
        rows::TableRows::Projects(vec![row])
            .encode_rows(&mut Vec::new())
            .unwrap();
    }
    #[test]
    fn deferred_freeze_owns_names_and_keeps_exact_encoded_runtime() {
        let row = nproject_cproject::Row {
            owner: 5_000,
            local_id: 7,
            source_pointer: 123,
            ..Default::default()
        };
        let make = || Held {
            pointer: row.source_pointer,
            pending_raw: Some(Box::new(PendingRaw {
                entries: vec![
                    engine_scripts::cvariables::Row {
                        name_start: 0,
                        name_len: 3,
                        value_raw: i64::MIN,
                    },
                    engine_scripts::cvariables::Row {
                        name_start: 3,
                        name_len: 2,
                        value_raw: i64::MAX,
                    },
                ],
                names: vec![b'a', 0, 255, b'b', b'c'],
                entries_len: 2,
                seed: Some(u32::MAX),
            })),
            variables_known: true,
            freeze_pending: true,
            pending_known: true,
            pending: Some(42),
            pending_ordinal: Some(3),
            ..Default::default()
        };
        let encode = |value| {
            let mut bytes = Vec::new();
            rows::TableRows::Projects(vec![rows::ProjectRow {
                runtime: value,
                ..Default::default()
            }])
            .encode_rows(&mut bytes)
            .unwrap();
            bytes
        };
        let mut eager = make();
        eager.freeze();
        let eager = Runtime {
            heap_bytes: eager.allocated_bytes(),
            held: [((row.owner, row.local_id), eager)].into(),
            pending: Vec::new(),
        };
        let held = make();
        let mut deferred = Runtime {
            heap_bytes: held.allocated_bytes(),
            held: [((row.owner, row.local_id), held)].into(),
            pending: vec![(row.owner, row.local_id)],
        };
        let rows::Recorded::Value(before) = deferred.value(&row) else {
            panic!("runtime")
        };
        assert!(
            before.variables.is_none(),
            "unsettled rows cannot expose an earlier store"
        );
        deferred.decode();
        assert_eq!(encode(eager.value(&row)), encode(deferred.value(&row)));
        let bytes = deferred.allocated_bytes();
        collector_test_support::with_allocations_denied(|| deferred.decode());
        assert_eq!(deferred.allocated_bytes(), bytes);
        assert_eq!(encode(eager.value(&row)), encode(deferred.value(&row)));
    }

    #[test]
    fn failed_deferred_freeze_refuses_variables_instead_of_reusing_an_older_store() {
        isolated(
            "failed_deferred_freeze_refuses_variables_instead_of_reusing_an_older_store",
            || {
                let row = nproject_cproject::Row {
                    owner: 5_000,
                    local_id: 7,
                    source_pointer: 123,
                    ..Default::default()
                };
                let held = Held {
                    pointer: row.source_pointer,
                    pending_raw: Some(Box::new(PendingRaw {
                        entries: vec![engine_scripts::cvariables::Row {
                            name_start: 0,
                            name_len: 3,
                            value_raw: -9,
                        }],
                        names: b"new".to_vec(),
                        entries_len: 1,
                        seed: Some(9),
                    })),
                    variables_known: true,
                    freeze_pending: true,
                    pending_known: true,
                    pending: Some(42),
                    pending_ordinal: Some(1),
                    recorded_variables: Some(
                        rows::ProjectVariableStore {
                            seed: 8,
                            entries: Vec::new(),
                        }
                        .into(),
                    ),
                    ..Default::default()
                };
                let mut runtime = Runtime {
                    heap_bytes: held.allocated_bytes(),
                    held: [((row.owner, row.local_id), held)].into(),
                    pending: vec![(row.owner, row.local_id)],
                };
                install_terminal_hooks();
                collector_test_support::with_allocations_denied(|| runtime.decode());
                assert_terminal_refusal();
                let rows::Recorded::Value(value) = runtime.value(&row) else {
                    panic!("runtime")
                };
                assert!(value.variables.is_none());
                assert_eq!(value.pending_reward, Some(42));
                assert_eq!(value.pending_ordinal, Some(1));
                let held = runtime.held.get(&(row.owner, row.local_id)).unwrap();
                assert!(!held.variables_known && !held.freeze_pending);
                assert_eq!(runtime.heap_bytes, held.allocated_bytes());
            },
        );
    }

    fn isolated(name: &str, run: impl FnOnce()) {
        const CHILD: &str = "BATTLEPLAN_PROJECT_RUNTIME_TEST";
        if std::env::var(CHILD).ok().as_deref() == Some(name) {
            run();
            return;
        }
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg(format!("projects::runtime::tests::{name}"))
            .arg("--nocapture")
            .env(CHILD, name)
            .status()
            .unwrap();
        assert!(status.success(), "isolated project runtime exercise failed");
    }
    fn install_terminal_hooks() {
        fn failed() {
            allocation::record_failure(allocation::Phase::Decode, allocation::Code::Reserve);
        }
        allocation::reset();
        engine_allocation::set_hooks(None, Some(failed));
        engine_allocation::set_stop_hook(Some(allocation::failed));
    }
    fn assert_terminal_refusal() {
        assert!(allocation::failed());
        assert!(engine_allocation::capture_stopped());
        let failure = allocation::failure().unwrap();
        assert_eq!(failure.phase, allocation::Phase::Decode as u8);
        assert_eq!(failure.code, allocation::Code::Reserve as u8);
    }
    use super::super::variable_test_kit::Fixture;
    fn encoded(runtime: &Runtime, row: &nproject_cproject::Row) -> Vec<u8> {
        let mut bytes = Vec::new();
        rows::TableRows::Projects(vec![rows::ProjectRow {
            owner: row.owner,
            local_id: row.local_id,
            runtime: runtime.value(row),
            ..Default::default()
        }])
        .encode_rows(&mut bytes)
        .unwrap();
        bytes
    }
    fn variables(
        runtime: &Runtime,
        row: &nproject_cproject::Row,
    ) -> Option<std::sync::Arc<rows::ProjectVariableStore>> {
        let rows::Recorded::Value(value) = runtime.value(row) else {
            panic!("runtime")
        };
        value.variables
    }
    fn observers() {
        project_variable_changes::enabled(true);
        project_variable_changes::project_hooks_enabled(true);
    }
    #[test]
    fn nonempty_acquire_fifo_decode_and_recycle_keep_callback_time_bytes() {
        isolated(
            "nonempty_acquire_fifo_decode_and_recycle_keep_callback_time_bytes",
            || {
                observers();
                let memory = Fixture::new(0x1000, 7);
                let row = memory.row();
                let mut fifo = std::collections::VecDeque::new();
                let mut expected = Vec::new();
                for seed in [7, 9] {
                    let write = project_variable_changes::writing(
                        memory.pointer + nproject_cproject::VARIABLES,
                    );
                    memory.replace(seed);
                    drop(write);
                    let mut eager = Runtime::default();
                    eager.read(&memory, 0, &[row], false, false, None);
                    assert_eq!(variables(&eager, &row).unwrap().entries.len(), 3);
                    expected.push(encoded(&eager, &row));
                    let mut acquired = Runtime::default();
                    acquired.acquire(&memory, 0, &[row], false, false, None);
                    assert!(variables(&acquired, &row).is_none());
                    assert_eq!(acquired.pending.len(), 1);
                    fifo.push_back(acquired);
                }
                memory.replace(11);
                memory.refuse.set(true);
                let reads = memory.reads.get();
                let mut recycled = None;
                for bytes in expected {
                    let mut runtime = fifo.pop_front().unwrap();
                    runtime.decode();
                    assert_eq!(encoded(&runtime, &row), bytes);
                    assert!(runtime.pending.is_empty());
                    runtime.decode();
                    assert_eq!(encoded(&runtime, &row), bytes);
                    recycled = Some(runtime);
                }
                assert_eq!(memory.reads.get(), reads);
                memory.refuse.set(false);
                let write = project_variable_changes::writing(
                    memory.pointer + nproject_cproject::VARIABLES,
                );
                memory.replace(13);
                drop(write);
                let mut eager = Runtime::default();
                eager.read(&memory, 0, &[row], true, false, None);
                let mut recycled = recycled.unwrap();
                recycled.acquire(&memory, 0, &[row], false, false, None);
                memory.replace(15);
                recycled.decode();
                assert_eq!(encoded(&recycled, &row), encoded(&eager, &row));
            },
        );
    }
    #[test]
    fn dropped_pruned_and_pointer_reused_acquisitions_never_publish_stale_stores() {
        isolated(
            "dropped_pruned_and_pointer_reused_acquisitions_never_publish_stale_stores",
            || {
                observers();
                let first = Fixture::new(0x1000, 7);
                let second = Fixture::new(0x9000, 9);
                let mut runtime = Runtime::default();
                runtime.acquire(&first, 0, &[first.row()], false, false, None);
                runtime.acquire(&second, 0, &[second.row()], false, false, None);
                assert_eq!(runtime.pending.len(), 1);
                first.refuse.set(true);
                second.refuse.set(true);
                runtime.decode();
                assert!(variables(&runtime, &first.row()).is_none());
                assert_eq!(variables(&runtime, &second.row()).unwrap().seed, 9);
                second.refuse.set(false);
                runtime.acquire(&second, 0, &[second.row()], true, false, None);
                assert_eq!(runtime.pending.len(), 1);
                runtime.acquire(&second, 0, &[], false, true, None);
                assert!(runtime.held.is_empty());
                runtime.decode();
                assert!(runtime.pending.is_empty());
                assert_eq!(runtime.heap_bytes, 0);
                let mut dropped = Runtime::default();
                dropped.acquire(&second, 0, &[second.row()], false, false, None);
                let bytes = encoded(&runtime, &second.row());
                drop(dropped);
                assert_eq!(encoded(&runtime, &second.row()), bytes);
            },
        );
    }
    #[test]
    fn acquisition_read_and_certificate_refusal_cancel_queued_freezing() {
        isolated(
            "acquisition_read_and_certificate_refusal_cancel_queued_freezing",
            || {
                observers();
                let memory = Fixture::new(0x1000, 7);
                let row = memory.row();
                let mut runtime = Runtime::default();
                runtime.read(&memory, 0, &[row], false, false, None);
                runtime.acquire(&memory, 0, &[row], true, false, None);
                memory.refuse.set(true);
                runtime.acquire(&memory, 0, &[row], true, false, None);
                runtime.decode();
                assert!(variables(&runtime, &row).is_none());
                memory.refuse.set(false);
                runtime.acquire(&memory, 0, &[row], false, false, None);
                let write = project_variable_changes::writing(
                    memory.pointer + nproject_cproject::VARIABLES,
                );
                runtime.acquire(&memory, 0, &[row], false, false, None);
                drop(write);
                runtime.decode();
                assert!(variables(&runtime, &row).is_none());
                assert!(runtime.pending.is_empty());
            },
        );
    }
    #[test]
    fn pending_queue_allocation_failure_is_terminal_and_cannot_publish_variables() {
        isolated(
            "pending_queue_allocation_failure_is_terminal_and_cannot_publish_variables",
            || {
                observers();
                let memory = Fixture::new(0x1000, 7);
                let row = memory.row();
                let mut runtime = Runtime::default();
                runtime.read(&memory, 0, &[row], false, false, None);
                assert!(runtime.pending.is_empty());
                install_terminal_hooks();
                collector_test_support::with_allocations_denied(|| {
                    runtime.acquire(&memory, 0, &[row], true, false, None);
                });
                assert_terminal_refusal();
                runtime.decode();
                assert!(runtime.pending.is_empty());
                assert!(variables(&runtime, &row).is_none());
            },
        );
    }

    struct Counted<'a> {
        bytes: &'a [u8],
        variable_reads: Cell<usize>,
        pending_reads: Cell<usize>,
    }
    impl Memory for Counted<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if (0x1000 + nproject_cproject::VARIABLES..0x1000 + nproject_cproject::VARIABLES + 0x30)
                .contains(&address)
            {
                self.variable_reads.set(self.variable_reads.get() + 1);
            }
            if address == 0x1000 + nproject_cproject::PENDING_REWARD {
                self.pending_reads.set(self.pending_reads.get() + 1);
            }
            engine_native_memory::memory::SliceMemory {
                base: 0x1000,
                bytes: self.bytes,
            }
            .copy(address, destination)
        }
    }
    #[test]
    fn gap_research_grants_project_runtime_reads_only_selected_objects_and_carries_variables_until_their_writer()
     {
        project_variable_changes::enabled(true);
        project_variable_changes::project_hooks_enabled(true);
        let mut bytes = vec![0; 0x900];
        bytes[0x20..0x28].copy_from_slice(&0x1418u64.to_le_bytes());
        bytes[0x400..0x408].copy_from_slice(&0x29b_dee8u64.to_le_bytes());
        bytes[0x418 + 0x358..0x418 + 0x360].copy_from_slice(&0x1800u64.to_le_bytes());
        bytes[0x418 + 0x360..0x418 + 0x364].copy_from_slice(&1u32.to_le_bytes());
        bytes[0x418 + 0x364..0x418 + 0x368].copy_from_slice(&1i32.to_le_bytes());
        bytes[0x800..0x808].copy_from_slice(&0x11c0u64.to_le_bytes());
        bytes[nproject_cproject::PENDING_REWARD..nproject_cproject::PENDING_REWARD + 8]
            .copy_from_slice(&0x11c0u64.to_le_bytes());
        bytes[0x1c0..0x1c8]
            .copy_from_slice(&(nproject_cproject::REWARD_VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[0x1c8..0x1cc].copy_from_slice(&123u32.to_le_bytes());
        let store = nproject_cproject::VARIABLES;
        bytes[store..store + 8]
            .copy_from_slice(&(engine_scripts::cvariables::VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[store + engine_scripts::cvariables::SEED
            ..store + engine_scripts::cvariables::SEED + 4]
            .copy_from_slice(&7u32.to_le_bytes());
        bytes[store + engine_scripts::cvariables::STORAGE
            ..store + engine_scripts::cvariables::STORAGE + 8]
            .copy_from_slice(
                &(engine_scripts::cvariables::EMPTY_STORAGE_RVA.0 as u64).to_le_bytes(),
            );
        let row = nproject_cproject::Row {
            owner: 5_000,
            local_id: 9,
            source_pointer: 0x1000,
            ..Default::default()
        };
        let memory = Counted {
            bytes: &bytes,
            variable_reads: Cell::new(0),
            pending_reads: Cell::new(0),
        };
        let mut runtime = Runtime::default();
        runtime.read(&memory, 0, &[], false, false, None);
        assert_eq!(memory.variable_reads.get(), 0);
        runtime.read(&memory, 0, &[row], false, false, None);
        let rows::Recorded::Value(value) = runtime.value(&row) else {
            panic!("captured runtime")
        };
        assert_eq!(value.pending_reward, Some(123));
        assert_eq!(value.pending_ordinal, Some(0));
        assert!(value.pending_known);
        assert_eq!(value.variables.as_ref().unwrap().seed, 7);
        assert!(value.variables.unwrap().entries.is_empty());
        runtime
            .held
            .get_mut(&(row.owner, row.local_id))
            .unwrap()
            .recorded_variables = Some(
            rows::ProjectVariableStore {
                seed: 7,
                entries: vec![rows::ProjectVariable {
                    name: b"mod_variable".to_vec(),
                    value_raw: 123,
                }],
            }
            .into(),
        );
        let held = runtime.held.get_mut(&(row.owner, row.local_id)).unwrap();
        runtime.heap_bytes -= held.recorded_bytes as usize;
        let value = held.recorded_variables.as_ref().unwrap();
        let bytes = size_of::<rows::ProjectVariableStore>()
            + 2 * size_of::<usize>()
            + value.entries.capacity() * size_of::<rows::ProjectVariable>()
            + value
                .entries
                .iter()
                .map(|entry| entry.name.capacity())
                .sum::<usize>();
        held.recorded_bytes = u32::try_from(bytes).unwrap_or(u32::MAX);
        runtime.heap_bytes += held.recorded_bytes as usize;
        let growths = engine_allocation::growths();
        let (first, second) = collector_test_support::with_allocations_denied(|| {
            (runtime.value(&row), runtime.value(&row))
        });
        let rows::Recorded::Value(first) = first else {
            panic!("runtime")
        };
        let rows::Recorded::Value(second) = second else {
            panic!("runtime")
        };
        assert_eq!(
            engine_allocation::growths(),
            growths,
            "hourly conversion must not rebuild variable maps"
        );
        assert!(std::sync::Arc::ptr_eq(
            first.variables.as_ref().unwrap(),
            second.variables.as_ref().unwrap()
        ));
        let initial = memory.variable_reads.get();
        assert!(initial > 0);
        runtime.read(&memory, 0, &[row], false, false, None);
        assert_eq!(
            memory.variable_reads.get(),
            initial,
            "quiet variables cause no native reads"
        );
        assert_eq!(
            memory.pending_reads.get(),
            1,
            "quiet selected objects do not reread pending offers"
        );
        let variable_reads = memory.variable_reads.get();
        drop(project_variable_changes::writing_project(
            row.source_pointer,
        ));
        runtime.read(&memory, 0, &[row], false, false, None);
        assert_eq!(memory.pending_reads.get(), 2);
        assert_eq!(
            memory.variable_reads.get(),
            variable_reads,
            "project lifecycle marks do not demand variable maps"
        );
        drop(project_variable_changes::writing(0x1000 + store));
        runtime.read(&memory, 0, &[row], false, false, None);
        assert!(memory.variable_reads.get() > initial);
        let stamp = project_variable_changes::register(0x1000 + store, row.owner, row.local_id);
        let write = project_variable_changes::writing(0x1000 + store);
        assert!(
            project_variable_changes::register(0x1000 + store, row.owner, row.local_id).is_none()
        );
        drop(write);
        assert_ne!(
            stamp,
            project_variable_changes::register(0x1000 + store, row.owner, row.local_id)
        );
        let write = project_variable_changes::writing_project(row.source_pointer);
        let pending_reads = memory.pending_reads.get();
        runtime.read(&memory, 0, &[row], false, false, None);
        let rows::Recorded::Value(value) = runtime.value(&row) else {
            panic!("captured runtime")
        };
        assert!(!value.pending_known);
        assert_eq!(memory.pending_reads.get(), pending_reads);
        drop(write);
        runtime.read(&memory, 0, &[row], false, false, None);
        assert_eq!(memory.pending_reads.get(), pending_reads + 1);
        let variable_reads = memory.variable_reads.get();
        project_variable_changes::enabled(false);
        runtime.read(&memory, 0, &[row], false, false, None);
        let rows::Recorded::Value(value) = runtime.value(&row) else {
            panic!("runtime")
        };
        assert!(
            value.variables.is_none(),
            "an unavailable observer withdraws its detail"
        );
        assert_eq!(
            memory.variable_reads.get(),
            variable_reads,
            "unavailable detail never polls"
        );
        let expected = runtime
            .held
            .values()
            .map(Held::allocated_bytes)
            .sum::<usize>()
            + runtime.held.capacity() * (size_of::<(u32, u32)>() + size_of::<Held>())
            + runtime.pending.capacity() * size_of::<(u32, u32)>();
        assert_eq!(runtime.allocated_bytes(), expected);
        let unread = SliceSelection::for_test(1, false, &[]).with_census(&[row.owner]);
        assert!(unread.live(row.owner) && !unread.contains(row.owner));
        runtime.read(&memory, 0, &[], false, true, Some(&unread));
        assert!(
            runtime.held.contains_key(&(row.owner, row.local_id)),
            "a live unread owner retains its detail registration"
        );
        let read = SliceSelection::for_test(1, true, &[]).with_census(&[row.owner]);
        runtime.read(&memory, 0, &[], false, true, Some(&read));
        assert_eq!(
            runtime.heap_bytes, 0,
            "daily pruning retires the retained-byte total"
        );
        project_variable_changes::project_hooks_enabled(false);
        project_variable_changes::enabled(false);
    }
}

#[cfg(test)]
mod cold_join_tests {
    use super::*;
    #[test]
    fn presized_uneven_cold_runtime_join_keeps_nonempty_values_without_rehashing_or_allocation() {
        let variables = freeze_project_variables(
            &[engine_scripts::cvariables::Row {
                name_start: 0,
                name_len: 1,
                value_raw: -17,
            }],
            b"x",
            23,
        )
        .unwrap();
        let recorded_bytes = u32::try_from(
            size_of::<rows::ProjectVariableStore>()
                + 2 * size_of::<usize>()
                + variables.entries.capacity() * size_of::<rows::ProjectVariable>()
                + variables
                    .entries
                    .iter()
                    .map(|entry| entry.name.capacity())
                    .sum::<usize>(),
        )
        .unwrap_or(u32::MAX);
        let mut parts = Vec::new();
        let mut expected = Vec::new();
        let mut first = 1u32;
        for count in [503, 499, 498] {
            let mut runtime = Runtime::default();
            for owner in first..first + count {
                let row = nproject_cproject::Row {
                    owner,
                    local_id: 7,
                    source_pointer: owner as usize,
                    ..Default::default()
                };
                runtime.held.insert(
                    (owner, 7),
                    Held {
                        pointer: owner as usize,
                        pending_known: true,
                        pending: Some(owner),
                        variables_known: true,
                        recorded_variables: Some(std::sync::Arc::clone(&variables)),
                        recorded_bytes,
                        ..Default::default()
                    },
                );
                runtime.heap_bytes += runtime.held[&(owner, 7)].allocated_bytes();
                expected.push((row, runtime.value(&row)));
                runtime.pending.push((owner, 7));
            }
            first += count;
            parts.push(runtime);
        }
        let heap_bytes = parts.iter().map(|part| part.heap_bytes).sum::<usize>();
        let mut joined = Runtime::default();
        joined.reserve_cold(1500, 1500).unwrap();
        let capacity = (joined.held.capacity(), joined.pending.capacity());
        collector_test_support::with_allocations_denied(|| {
            for part in &mut parts {
                joined.append_cold(part).unwrap();
            }
            joined.order_pending();
        });
        assert_eq!(
            (joined.held.capacity(), joined.pending.capacity()),
            capacity
        );
        assert_eq!(joined.cold_counts(), (1500, 1500));
        assert_eq!(joined.heap_bytes, heap_bytes);
        assert!(parts.iter().all(|part| part.heap_bytes == 0));
        for (row, value) in expected {
            assert_eq!(joined.value(&row), value);
        }
        assert!(parts.iter().all(Runtime::is_empty));
    }
}
