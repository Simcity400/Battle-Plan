//! Runtime project details share the existing selected project acquisition.
//! Variable maps are read only on first observation or a notified native write;
//! the native census supplies identity and address, so there is no second walk.
use super::*;
use collector_observers::project_variable_changes;
use std::collections::HashMap;

#[derive(Default)]
struct Held {
    pointer: usize,
    stamp: Option<(u64, u64)>,
    entries: Vec<engine_scripts::cvariables::Row>,
    names: Vec<u8>,
    entries_len: usize,
    seed: Option<u32>,
    variables_known: bool,
    recorded_variables: Option<std::sync::Arc<rows::ProjectVariableStore>>,
    recorded_bytes: usize,
    pending_known: bool,
    pending: Option<u32>,
    pending_ordinal: Option<u32>,
    pending_stamp: Option<(u64, u64)>,
    iterations: i32,
}

impl Held {
    fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<engine_scripts::cvariables::Row>()
            + self.names.capacity()
            + self.recorded_bytes
    }
}

#[derive(Default)]
pub(super) struct Runtime {
    held: HashMap<(u32, u32), Held>,
    heap_bytes: usize,
}

impl Runtime {
    pub(super) fn read(
        &mut self,
        memory: &impl Memory,
        base: usize,
        rows: &[nproject_cproject::Row],
        force: bool,
        full_census: bool,
        selection: Option<&SliceSelection>,
    ) {
        for row in rows {
            let key = (row.owner, row.local_id);
            if !self.held.contains_key(&key) {
                if engine_allocation::reserve_map(&mut self.held, 1).is_err() {
                    return;
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
                        ..Default::default()
                    };
                }
                let Some(store) = pointer.checked_add(nproject_cproject::VARIABLES) else {
                    held.variables_known = false;
                    held.pending_known = false;
                    return;
                };
                // Registration reuses the authenticated project; no extra pool walk.
                let stamp = project_variable_changes::register(store, row.owner, row.local_id);
                let pending_stamp = project_variable_changes::pending_stamp(store);
                // Daily offer generation advances the iteration counter. Commands
                // and effects forward their existing project writer around the call.
                if pending_stamp.is_none() {
                    held.pending_known = false;
                    PROJECT_REWARD_REFUSALS.fetch_add(1, Relaxed);
                } else if force
                    || !held.pending_known
                    || pending_stamp != held.pending_stamp
                    || row.state.iterations != held.iterations
                {
                    let pending = pointer
                        .checked_add(nproject_cproject::PENDING_REWARD)
                        .and_then(|address| engine_native::read::<u64>(memory, address))
                        .and_then(|value| usize::try_from(value).ok());
                    held.pending_known = false;
                    if let Some(pending) = pending {
                        if pending == 0 {
                            held.pending = None;
                            held.pending_ordinal = None;
                            held.pending_known = true;
                        } else if let Some(header) =
                            engine_native::read::<[u8; 12]>(memory, pending).filter(|header| {
                                base.checked_add(nproject_cproject::REWARD_VTABLE_RVA)
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
                    PROJECT_VARIABLE_REFUSALS.fetch_add(1, Relaxed);
                    return;
                }
                if !force && held.variables_known && stamp == held.stamp {
                    return;
                }
                held.variables_known = false;
                let read = engine_scripts::cvariables::read_store_carried(
                    memory,
                    base,
                    engine_scripts::cvariables::Scope::Global,
                    store,
                    engine_native::walk::Buffer::Growing(&mut held.entries),
                    engine_native::walk::Buffer::Growing(&mut held.names),
                    &mut engine_scripts::cvariables::Fresh,
                );
                if let Ok((read, seed)) = read
                    && let Outcome::Rows { len } = read.outcome
                {
                    held.entries_len = len;
                    held.seed = seed;
                    held.variables_known = seed.is_some()
                        && stamp.is_some()
                        && stamp
                            == project_variable_changes::register(store, row.owner, row.local_id);
                    held.stamp = stamp;
                    if held.variables_known {
                        let freeze = || -> std::io::Result<_> {
                            let entries = map_slice(&held.entries[..held.entries_len], |entry| {
                                rows::ProjectVariable {
                                    name: owned_vec(copy_vec(
                                        &held.names
                                            [entry.name_start..entry.name_start + entry.name_len],
                                    )),
                                    value_raw: entry.value_raw,
                                }
                            })?;
                            engine_allocation::allocation_attempt()?;
                            Ok(std::sync::Arc::new(rows::ProjectVariableStore {
                                seed: held.seed.unwrap(),
                                entries,
                            }))
                        };
                        match freeze() {
                            Ok(value) => {
                                held.recorded_bytes = size_of::<rows::ProjectVariableStore>()
                                    + 2 * size_of::<usize>()
                                    + value.entries.capacity() * size_of::<rows::ProjectVariable>()
                                    + value
                                        .entries
                                        .iter()
                                        .map(|entry| entry.name.capacity())
                                        .sum::<usize>();
                                held.recorded_variables = Some(value);
                            }
                            Err(_) => held.variables_known = false,
                        }
                    }
                }
                if !held.variables_known {
                    PROJECT_VARIABLE_REFUSALS.fetch_add(1, Relaxed);
                }
            })();
            self.heap_bytes += held.allocated_bytes();
        }
        if full_census {
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
            variables: held
                .variables_known
                .then(|| std::sync::Arc::clone(held.recorded_variables.as_ref().unwrap())),
        })
    }

    pub(super) fn allocated_bytes(&self) -> usize {
        self.held.capacity() * (size_of::<(u32, u32)>() + size_of::<Held>()) + self.heap_bytes
    }
}
#[cfg(test)]
mod tests {
    use super::*;
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
            engine_native::memory::SliceMemory {
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
        bytes[0x400..0x408].copy_from_slice(&0x292_a158u64.to_le_bytes());
        bytes[0x418 + 0x358..0x418 + 0x360].copy_from_slice(&0x1800u64.to_le_bytes());
        bytes[0x418 + 0x360..0x418 + 0x364].copy_from_slice(&1u32.to_le_bytes());
        bytes[0x418 + 0x364..0x418 + 0x368].copy_from_slice(&1i32.to_le_bytes());
        bytes[0x800..0x808].copy_from_slice(&0x11c0u64.to_le_bytes());
        bytes[nproject_cproject::PENDING_REWARD..nproject_cproject::PENDING_REWARD + 8]
            .copy_from_slice(&0x11c0u64.to_le_bytes());
        bytes[0x1c0..0x1c8]
            .copy_from_slice(&(nproject_cproject::REWARD_VTABLE_RVA as u64).to_le_bytes());
        bytes[0x1c8..0x1cc].copy_from_slice(&123u32.to_le_bytes());
        let store = nproject_cproject::VARIABLES;
        bytes[store..store + 8]
            .copy_from_slice(&(engine_scripts::cvariables::VTABLE_RVA as u64).to_le_bytes());
        bytes[store + engine_scripts::cvariables::SEED
            ..store + engine_scripts::cvariables::SEED + 4]
            .copy_from_slice(&7u32.to_le_bytes());
        bytes[store + engine_scripts::cvariables::STORAGE
            ..store + engine_scripts::cvariables::STORAGE + 8]
            .copy_from_slice(&(engine_scripts::cvariables::EMPTY_STORAGE_RVA as u64).to_le_bytes());
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
        runtime.heap_bytes -= held.recorded_bytes;
        let value = held.recorded_variables.as_ref().unwrap();
        held.recorded_bytes = size_of::<rows::ProjectVariableStore>()
            + 2 * size_of::<usize>()
            + value.entries.capacity() * size_of::<rows::ProjectVariable>()
            + value
                .entries
                .iter()
                .map(|entry| entry.name.capacity())
                .sum::<usize>();
        runtime.heap_bytes += held.recorded_bytes;
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
            + runtime.held.capacity() * (size_of::<(u32, u32)>() + size_of::<Held>());
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
