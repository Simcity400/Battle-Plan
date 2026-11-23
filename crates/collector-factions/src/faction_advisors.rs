use crate::faction_companion_changes::Changes;
use engine_allocation as allocation;
use engine_native_memory::{Memory, cpdx_array::Array};
use engine_stability::cfaction_upgrade_status as native;
use replay_data::rows::{TableRows, tables};
use std::sync::Arc;
type Certificate = (u64, u64, u64);
#[derive(Default)]
struct Slot {
    certificate: Option<Certificate>,
    failure: Option<&'static str>,
    data: native::Slot,
}
#[derive(Default)]
struct Owner {
    id: u64,
    pointer: usize,
    seen: u64,
    certificate: Option<Certificate>,
    failure: Option<&'static str>,
    status: Option<native::Status>,
    array: Array,
    slots: Vec<Slot>,
}
#[derive(Default)]
pub struct Store {
    owners: Vec<Owner>,
    before: Option<Certificate>,
    quiet: Option<Certificate>,
    quiet_carried: bool,
    generation: u64,
    force: bool,
    pending: bool,
    failure: Option<&'static str>,
    null_date: Option<i32>,
    null_date_failed: bool,
    null_date_epoch: Option<u64>,
    rows: Option<Arc<TableRows>>,
    spares: Vec<Arc<TableRows>>,
}
impl Store {
    pub fn begin(&mut self, changes: &impl Changes, force: bool) {
        self.before = changes.quiet(11);
        if force || self.null_date_epoch != self.before.map(|value| value.0) {
            self.null_date = None;
            self.null_date_failed = false;
            self.null_date_epoch = self.before.map(|value| value.0);
        }
        self.force = force;
        self.quiet_carried = false;
        self.failure = None;
        self.generation = self.generation.checked_add(1).unwrap_or(0);
        if self.before.is_none() || self.generation == 0 {
            self.failure = Some("faction advisors could not be followed in the game");
        }
    }
    pub fn visit(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: usize,
        id: u64,
        changes: &impl Changes,
    ) {
        if self.before.is_none() || self.generation == 0 {
            return;
        }
        let at = match self.owners.binary_search_by_key(&id, |owner| owner.id) {
            Ok(at) => at,
            Err(at) => {
                if allocation::reserve(&mut self.owners, 1).is_err() {
                    self.failure = Some("faction advisor owner allocation failed");
                    return;
                }
                self.owners.insert(
                    at,
                    Owner {
                        id,
                        pointer,
                        ..Default::default()
                    },
                );
                self.pending = true;
                at
            }
        };
        let owner = &mut self.owners[at];
        owner.seen = self.generation;
        if !self.force
            && owner.pointer == pointer
            && owner.certificate.is_some()
            && self.before == self.quiet
        {
            self.quiet_carried = true;
            return;
        }
        let Some(receiver) = pointer.checked_add(native::STATUS) else {
            self.failure = Some("faction advisor owner address overflow");
            return;
        };
        let before = changes.sequence(11, receiver);
        if before.is_none() {
            self.failure = Some("faction advisor owner registration unavailable");
            return;
        }
        let _reads = collector_memory::read_accounting::table(
            replay_data::rows::TableId::FactionAdvisorModifiers,
        );
        if self.force || owner.pointer != pointer || before != owner.certificate {
            owner.pointer = pointer;
            owner.failure = None;
            owner.certificate = None;
            owner.status = None;
            self.pending = true;
            let result = (|| {
                let status = native::authenticate(memory, base, pointer)?;
                let array = status.slots(memory)?;
                allocation::resize_default(&mut owner.slots, array.count as usize)
                    .map_err(|_| "faction advisor slot allocation failed")?;
                for slot in &mut owner.slots {
                    slot.certificate = None;
                    slot.failure = None;
                }
                owner.status = Some(status);
                owner.array = array;
                Ok::<(), &'static str>(())
            })();
            if before == changes.sequence(11, receiver) {
                owner.certificate = before;
                owner.failure = result.err();
            } else {
                self.failure = Some("faction advisor structure changed during acquisition");
                return;
            }
        }
        if let Some(why) = owner.failure {
            self.failure = Some(why);
            return;
        }
        let Some(status) = owner.status else {
            self.failure = Some("faction advisor owner baseline unavailable");
            return;
        };
        if !owner.slots.is_empty() && self.null_date_failed {
            self.failure = Some("faction advisor null-date operand unavailable");
            return;
        }
        if !owner.slots.is_empty() && self.null_date.is_none() {
            self.null_date = engine_sites::at(base, native::UNSET_CHANGE_HOUR_RVA)
                .and_then(|at| engine_native_memory::read(memory, at));
            if self.null_date.is_none() {
                self.null_date_failed = true;
                self.failure = Some("faction advisor null-date operand unavailable");
                return;
            }
        }
        for (ordinal, slot) in owner.slots.iter_mut().enumerate() {
            let Some(address) = owner.array.element(ordinal as u32, native::SLOT_STRIDE) else {
                self.failure = Some("faction advisor slot address overflow");
                return;
            };
            let before = changes.sequence(11, address);
            if before.is_none() {
                self.failure = Some("faction advisor slot registration unavailable");
                continue;
            }
            if slot.certificate != before {
                self.pending = true;
                slot.certificate = None;
                let result = status.slot_into(memory, owner.array, ordinal as u32, &mut slot.data);
                if before == changes.sequence(11, address) {
                    slot.certificate = before;
                    slot.failure = result.err();
                } else {
                    self.failure = Some("faction advisor slot changed during acquisition");
                }
            }
            if let Some(why) = slot.failure {
                self.failure = Some(why);
            }
        }
        if changes.sequence(11, receiver) != owner.certificate {
            self.failure = Some("faction advisor structure changed during acquisition");
        }
    }
    /// # Safety
    /// Caller holds hourly capture admission for the active session (or isolated fixture control)
    /// so global `faction_companion_changes` demand updates belong to this pass.
    pub(crate) unsafe fn finish(&mut self, complete: bool, changes: &impl Changes) {
        // SAFETY: Admitted faction-advisor capture records measured owner demand.
        unsafe {
            crate::faction_companion_changes::demand(
                self.owners.iter().fold(self.owners.len(), |n, owner| {
                    n.saturating_add(owner.slots.len())
                }),
            );
        }
        if !complete {
            self.failure = Some("faction advisor identity roster incomplete");
        }
        if self.before.is_none() || (self.quiet_carried && self.before != changes.quiet(11)) {
            self.failure = Some("faction advisor observation changed during acquisition");
        }
        if self.failure.is_some() {
            self.quiet = None;
            return;
        }
        if self
            .owners
            .iter()
            .any(|owner| owner.seen != self.generation)
        {
            self.owners.retain(|owner| owner.seen == self.generation);
            self.pending = true;
        }
        if self.pending || self.rows.is_none() {
            if let Err(why) = self.rebuild() {
                self.failure = Some(why);
                self.quiet = None;
                return;
            }
            self.pending = false;
        }
        self.quiet = self.before;
    }
    fn rebuild(&mut self) -> Result<(), &'static str> {
        let count = self
            .owners
            .iter()
            .flat_map(|owner| &owner.slots)
            .try_fold(0usize, |n, slot| {
                n.checked_add(
                    3 + slot.data.spymaster.entries.len() + slot.data.modifier.entries.len(),
                )
            })
            .ok_or("faction advisor occurrence count overflow")?;
        let mut table = crate::faction_goals::writable(&mut self.rows, &mut self.spares, || {
            TableRows::FactionAdvisorModifiers(Vec::new())
        })?;
        let rows = Arc::get_mut(&mut table)
            .unwrap()
            .rows_mut::<tables::FactionAdvisorModifiers>()
            .unwrap();
        allocation::resize_default(rows, count)
            .map_err(|_| "faction advisor replay allocation failed")?;
        let mut next = 0;
        for owner in &self.owners {
            for (slot_index, slot) in owner.slots.iter().enumerate() {
                let data = &slot.data;
                let mut ordinal = 0;
                let mut write =
                    |kind: u32, name: &[u8], modifier: u32, raw: i64| -> Result<(), &'static str> {
                        let row = &mut rows[next];
                        next += 1;
                        row.faction = owner.id;
                        row.slot = slot_index as u32;
                        row.ordinal = ordinal;
                        ordinal += 1;
                        row.kind = kind;
                        row.country = if kind == 0 { data.owner_country } else { 0 };
                        row.character = if kind == 0 {
                            data.advisor_character
                        } else {
                            None
                        };
                        row.date = if kind == 0 {
                            data.spymaster_change_hour
                        } else {
                            0
                        };
                        row.date_present = u8::from(
                            kind == 0 && Some(data.spymaster_change_hour) != self.null_date,
                        );
                        row.modifier = modifier;
                        row.raw = raw;
                        let additional = name.len().saturating_sub(row.name.len());
                        allocation::reserve(&mut row.name, additional)
                            .map_err(|_| "faction advisor name allocation failed")?;
                        row.name.clear();
                        row.name.extend_from_slice(name);
                        Ok(())
                    };
                write(0, &[], 0, 0)?;
                for (kind, value) in [(1, &data.spymaster), (3, &data.modifier)] {
                    write(kind, &value.name, 0, 0)?;
                    for entry in &value.entries {
                        write(kind + 1, &[], entry.key, entry.raw)?;
                    }
                }
            }
        }
        self.rows = Some(table);
        Ok(())
    }
    pub fn output(&self) -> (Option<Arc<TableRows>>, Option<&'static str>) {
        (
            self.failure
                .is_none()
                .then(|| self.rows.as_ref().map(Arc::clone))
                .flatten(),
            self.failure,
        )
    }
    pub fn allocated_bytes(&self) -> usize {
        self.owners.capacity() * size_of::<Owner>()
            + self
                .owners
                .iter()
                .map(|owner| {
                    owner.slots.capacity() * size_of::<Slot>()
                        + owner
                            .slots
                            .iter()
                            .map(|slot| {
                                slot.data.spymaster.name.capacity()
                                    + slot.data.modifier.name.capacity()
                                    + (slot.data.spymaster.entries.capacity()
                                        + slot.data.modifier.entries.capacity())
                                        * size_of::<engine_stability::cmodifier::Entry>()
                            })
                            .sum::<usize>()
                })
                .sum::<usize>()
            + self
                .rows
                .iter()
                .chain(&self.spares)
                .map(|rows| size_of::<TableRows>() + rows.allocated_bytes())
                .sum::<usize>()
            + self.spares.capacity() * size_of::<Arc<TableRows>>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use collector_certificates::watch_registry::WatchRegistry;
    use std::cell::Cell;
    use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
    const OWNER: usize = 0x1000;
    const DATA: usize = 0x3000;
    struct Observed(WatchRegistry);
    impl Changes for Observed {
        fn take_accounts(&self, _: &mut dyn FnMut(usize)) -> Option<u64> {
            None
        }
        fn quiet(&self, _: usize) -> Option<Certificate> {
            self.0.quiet().map(|(a, b)| (a, b, 0))
        }
        fn sequence(&self, _: usize, address: usize) -> Option<Certificate> {
            self.0.sequence(address).map(|(a, b)| (a, b, 0))
        }
    }
    struct Counted {
        bytes: Vec<u8>,
        reads: Cell<usize>,
    }
    impl Memory for Counted {
        fn copy(&self, at: usize, out: &mut [u8]) -> bool {
            self.reads.set(self.reads.get() + 1);
            if at == native::UNSET_CHANGE_HOUR_RVA.0 && out.len() == 4 {
                out.copy_from_slice(&engine_native_memory::cgame_date::NULL_RAW_HOUR.to_le_bytes());
                return true;
            }
            engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &self.bytes,
            }
            .copy(at, out)
        }
    }
    fn put(bytes: &mut [u8], at: usize, value: impl AsRef<[u8]>) {
        let value = value.as_ref();
        bytes[at..at + value.len()].copy_from_slice(value);
    }
    fn fixture(count: u32) -> (Counted, Observed) {
        let mut bytes = vec![0; DATA + count as usize * native::SLOT_STRIDE];
        let status = OWNER + native::STATUS;
        put(
            &mut bytes,
            status,
            (native::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            status + native::OWNER,
            (OWNER as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            status + native::SLOTS,
            (DATA as u64).to_le_bytes(),
        );
        put(&mut bytes, status + native::SLOTS + 8, count.to_le_bytes());
        put(&mut bytes, status + native::SLOTS + 12, count.to_le_bytes());
        for ordinal in 0..count as usize {
            let slot = DATA + ordinal * native::SLOT_STRIDE;
            let date = slot + native::SLOT_SPYMASTER_DATE;
            put(
                &mut bytes,
                date,
                (engine_native_memory::cgame_date::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                date + engine_native_memory::cgame_date::PERSISTENT_BASE,
                (engine_native_memory::cgame_date::PERSISTENT_VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                date + engine_native_memory::cgame_date::DATE_BASE,
                engine_native_memory::cgame_date::NULL_RAW_HOUR.to_le_bytes(),
            );
            for offset in [native::SLOT_SPYMASTER_MODIFIER, native::SLOT_MODIFIER] {
                let modifier = slot + offset;
                put(
                    &mut bytes,
                    modifier,
                    (engine_stability::cmodifier::VTABLE_RVA.0 as u64).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    modifier
                        + engine_stability::cmodifier::DISPLAY_NAME
                        + engine_native_memory::msvc_string::CAPACITY,
                    15u64.to_le_bytes(),
                );
            }
        }
        let watch = WatchRegistry::retaining();
        watch.offer(count as usize + 1);
        watch.boundary(false);
        watch.set_enabled(true);
        (
            Counted {
                bytes,
                reads: Cell::new(0),
            },
            Observed(watch),
        )
    }
    fn acquire(store: &mut Store, memory: &Counted, changes: &Observed, complete: bool) {
        store.begin(changes, false);
        store.visit(memory, 0, OWNER, 7, changes);
        // SAFETY: Unit test runs against an isolated fixture.
        unsafe { store.finish(complete, changes) };
    }
    #[test]
    fn advisor_certificate_carries_slots_and_reads_only_the_changed_slot() {
        let (mut memory, changes) = fixture(40);
        let mut store = Store::default();
        acquire(&mut store, &memory, &changes, true);
        assert!(store.output().0.is_some());
        let reads = memory.reads.get();
        collector_test_support::with_allocations_denied(|| {
            acquire(&mut store, &memory, &changes, true)
        });
        assert_eq!(memory.reads.get(), reads);
        let selected = DATA + 39 * native::SLOT_STRIDE;
        put(
            &mut memory.bytes,
            selected + native::SLOT_COUNTRY,
            19u32.to_le_bytes(),
        );
        drop(changes.0.notify(selected));
        acquire(&mut store, &memory, &changes, false);
        assert!(store.output().0.is_none());
        let changed_reads = memory.reads.get();
        assert!(changed_reads > reads && changed_reads - reads < 30);
        acquire(&mut store, &memory, &changes, true);
        assert_eq!(memory.reads.get(), changed_reads);
        let rows = store.output().0.unwrap();
        let rows = rows.rows::<tables::FactionAdvisorModifiers>().unwrap();
        assert_eq!(
            rows.iter()
                .find(|row| row.slot == 39 && row.kind == 0)
                .unwrap()
                .country,
            19
        );
    }
    #[test]
    fn advisor_slot_read_refusal_is_stable_until_its_writer_changes() {
        let (mut memory, changes) = fixture(2);
        let slot = DATA + native::SLOT_STRIDE;
        memory.bytes[slot + native::SLOT_MODIFIER] ^= 1;
        let mut store = Store::default();
        acquire(&mut store, &memory, &changes, true);
        assert!(store.output().1.is_some());
        let reads = memory.reads.get();
        acquire(&mut store, &memory, &changes, true);
        assert_eq!(memory.reads.get(), reads);
        memory.bytes[slot + native::SLOT_MODIFIER] ^= 1;
        drop(changes.0.notify(slot));
        acquire(&mut store, &memory, &changes, true);
        assert!(store.output().0.is_some());
    }
    struct Interrupting<'a> {
        changes: &'a Observed,
        at: usize,
        writer: usize,
        fired: AtomicBool,
    }
    impl Changes for Interrupting<'_> {
        fn take_accounts(&self, _: &mut dyn FnMut(usize)) -> Option<u64> {
            None
        }
        fn quiet(&self, family: usize) -> Option<Certificate> {
            self.changes.quiet(family)
        }
        fn sequence(&self, family: usize, address: usize) -> Option<Certificate> {
            if address == self.at && !self.fired.swap(true, SeqCst) {
                drop(self.changes.0.notify(self.writer));
            }
            self.changes.sequence(family, address)
        }
    }
    #[test]
    fn advisor_owners_read_under_their_own_certificates_survive_other_writers_during_the_walk() {
        let (mut memory, changes) = fixture(2);
        put(
            &mut memory.bytes,
            DATA + native::SLOT_STRIDE + native::SLOT_COUNTRY,
            19u32.to_le_bytes(),
        );
        let unrelated = 0x10_0000;
        let mut store = Store::default();
        store.begin(&changes, true);
        store.visit(&memory, 0, OWNER, 7, &changes);
        drop(changes.0.notify(unrelated));
        // SAFETY: Unit test runs against an isolated fixture.
        unsafe { store.finish(true, &changes) };
        assert_eq!(store.output().1, None);
        let rows = store.output().0.unwrap();
        let rows = rows.rows::<tables::FactionAdvisorModifiers>().unwrap();
        assert_eq!(rows.len(), 6);
        assert!(rows.iter().all(|row| row.faction == 7));
        assert_eq!(
            rows.iter()
                .filter(|row| row.kind == 0)
                .map(|row| (row.slot, row.country))
                .collect::<Vec<_>>(),
            [(0, 0), (1, 19)]
        );

        acquire(&mut store, &memory, &changes, true);
        assert_eq!(store.output().1, None);
        let reads = memory.reads.get();
        store.begin(&changes, false);
        store.visit(&memory, 0, OWNER, 7, &changes);
        assert_eq!(memory.reads.get(), reads, "a quiet family carries the owner");
        drop(changes.0.notify(unrelated));
        // SAFETY: Unit test runs against an isolated fixture.
        unsafe { store.finish(true, &changes) };
        assert_eq!(
            store.output().1,
            Some("faction advisor observation changed during acquisition")
        );
    }
    #[test]
    fn advisor_structure_writer_before_slot_reads_refuses_a_forced_hour() {
        let (memory, changes) = fixture(2);
        let interrupting = Interrupting {
            changes: &changes,
            at: DATA,
            writer: OWNER + native::STATUS,
            fired: AtomicBool::new(false),
        };
        let mut store = Store::default();
        store.begin(&interrupting, true);
        store.visit(&memory, 0, OWNER, 7, &interrupting);
        // SAFETY: Unit test runs against an isolated fixture.
        unsafe { store.finish(true, &interrupting) };
        assert!(interrupting.fired.load(SeqCst));
        assert!(store.output().0.is_none());
        assert_eq!(
            store.output().1,
            Some("faction advisor structure changed during acquisition")
        );
        acquire(&mut store, &memory, &changes, true);
        assert_eq!(store.output().1, None);
    }
}
