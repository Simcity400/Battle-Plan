use crate::template_changes::Changes;
use engine_native_memory::Memory;
use engine_native::corders_group::Root;
use engine_native_land::template_hq_context as native;
use replay_data::rows::{HqDeployment, HqEquipment, HqRequirement, TemplateHqContext};
use std::sync::Arc;

type Certificate = [(u64, u64); 2];
#[derive(Default)]
struct Entry {
    group: u64,
    pointer: usize,
    attempted: bool,
    certificate: Option<Certificate>,
    attempted_holder: usize,
    header: native::Header,
    pools: native::Pools,
    value: Option<Arc<TemplateHqContext>>,
    valid: bool,
}
impl Entry {
    fn allocated_bytes(&self) -> usize {
        self.pools.equipment.capacity() * size_of::<native::Equipment>()
            + self.pools.required.capacity() * size_of::<native::Requirement>()
            + self.value.as_ref().map_or(0, |value| {
                size_of::<TemplateHqContext>()
                    + value.equipment.capacity() * size_of::<HqEquipment>()
                    + value.required.capacity() * size_of::<HqRequirement>()
            })
    }
}
#[derive(Default)]
pub(super) struct Store {
    entries: Vec<Entry>,
    live: Vec<u64>,
    payload_bytes: usize,
}
fn certificate(changes: &dyn Changes, pointer: usize, holder: usize) -> Option<Certificate> {
    Some([
        changes.hq_sequence(pointer)?,
        if holder == 0 {
            (0, 0)
        } else {
            changes.hq_sequence(holder)?
        },
    ])
}
fn deployment(value: native::Deployment) -> HqDeployment {
    HqDeployment {
        country: value.country,
        priority: value.priority,
        allow_zero: value.allow_zero,
        assembled_manpower: value.assembled_manpower,
        requested_manpower: value.requested_manpower,
        order: value.order,
        requisitioned: value.requisitioned,
    }
}
fn same(entry: &Entry, value: &TemplateHqContext) -> bool {
    value.target == entry.header.target
        && value.deployed == entry.header.state[0]
        && value.queued == entry.header.state[1]
        && value.deployment == entry.header.deployment.map(deployment)
        && value.equipment.len() == entry.pools.equipment.len()
        && value.required.len() == entry.pools.required.len()
        && value
            .equipment
            .iter()
            .zip(&entry.pools.equipment)
            .all(|(a, b)| a.variant == b.variant && a.amount == b.amount)
        && value
            .required
            .iter()
            .zip(&entry.pools.required)
            .all(|(a, b)| a.token == b.token && a.amount == b.amount)
}
impl Store {
    pub fn forget(&mut self) {
        for entry in &self.entries {
            crate::template_changes::forget_hq_holder(entry.attempted_holder);
        }
        self.entries.clear();
        self.payload_bytes = 0;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<Entry>()
            + self.live.capacity() * size_of::<u64>()
            + self.payload_bytes
    }
    pub fn prune(&mut self, roots: &[Root]) {
        if self.entries.len() <= roots.len() {
            return;
        }
        self.live.clear();
        if engine_allocation::reserve(&mut self.live, roots.len()).is_err() {
            return;
        }
        self.live.extend(roots.iter().map(|root| root.handle));
        self.live.sort_unstable();
        self.entries.retain(|entry| {
            let keep = self.live.binary_search(&entry.group).is_ok();
            if !keep {
                crate::template_changes::forget_hq_holder(entry.attempted_holder);
                self.payload_bytes -= entry.allocated_bytes();
            }
            keep
        });
    }
    pub fn read(
        &mut self,
        memory: &impl Memory,
        base: usize,
        root: Root,
        changes: &dyn Changes,
    ) -> Result<Arc<TemplateHqContext>, &'static str> {
        let at = match self
            .entries
            .binary_search_by_key(&root.handle, |entry| entry.group)
        {
            Ok(at) => at,
            Err(at) => {
                engine_allocation::reserve(&mut self.entries, 1)
                    .map_err(|_| "headquarters context cache allocation failed")?;
                self.entries.insert(
                    at,
                    Entry {
                        group: root.handle,
                        ..Default::default()
                    },
                );
                at
            }
        };
        let entry = &mut self.entries[at];
        if entry.pointer != root.pointer() {
            crate::template_changes::forget_hq_holder(entry.attempted_holder);
            let previous_bytes = entry.allocated_bytes();
            entry.pointer = root.pointer();
            entry.attempted = false;
            entry.certificate = None;
            entry.attempted_holder = 0;
            entry.value = None;
            entry.header = native::Header::default();
            self.payload_bytes = self.payload_bytes - previous_bytes + entry.allocated_bytes();
        }
        let fresh = changes.fresh_only();
        let current = certificate(changes, root.pointer(), entry.attempted_holder);
        if current.is_none() && !fresh {
            entry.attempted = false;
            entry.valid = false;
            return Err("headquarters context writer certificate unavailable");
        }
        let acquire = fresh || !entry.attempted || current != entry.certificate;
        if !acquire {
            entry.valid = entry.value.is_some();
            return entry.value.clone().ok_or(
                "headquarters context acquisition refused until a writer change or refresh",
            );
        }
        entry.attempted = true;
        entry.valid = false;
        entry.certificate = current;
        let previous_bytes = entry.allocated_bytes();
        let read = (|| {
            let group_before = changes.hq_sequence(root.pointer());
            if group_before.is_none() && !fresh {
                entry.attempted = false;
                return Err("headquarters context writer certificate unavailable");
            }
            let holder = root
                .pointer()
                .checked_add(native::HOLDER)
                .and_then(|at| engine_native_memory::read::<u64>(memory, at))
                .and_then(|holder| usize::try_from(holder).ok())
                .ok_or("headquarters deployment holder pointer unreadable")?;
            if holder != entry.attempted_holder {
                crate::template_changes::forget_hq_holder(entry.attempted_holder);
            }
            if !fresh {
                crate::template_changes::track_hq_holder(holder)?;
            }
            let before = certificate(changes, root.pointer(), holder);
            entry.attempted_holder = holder;
            if !fresh && (before.is_none() || before.map(|stamp| stamp[0]) != group_before) {
                entry.attempted = false;
                return Err("headquarters context writer certificate unavailable");
            }
            entry.certificate = before;
            let certified_header = native::header(memory, base, root)?;
            if certified_header.holder != holder {
                entry.attempted = false;
                return Err("headquarters deployment holder changed during acquisition");
            }
            entry.header = certified_header;
            native::pools(memory, base, &entry.header, &mut entry.pools)?;
            if !fresh && (group_before != changes.hq_sequence(root.pointer())
                || before != certificate(changes, root.pointer(), entry.header.holder))
            {
                entry.attempted = false;
                return Err("headquarters template context changed during acquisition");
            }
            if let Some(value) = entry.value.as_ref().filter(|value| same(entry, value)) {
                return Ok(Arc::clone(value));
            }
            let equipment =
                engine_allocation::collect_vec(entry.pools.equipment.iter().map(|row| {
                    HqEquipment {
                        variant: row.variant,
                        amount: row.amount,
                    }
                }))
                .map_err(|_| "headquarters context equipment projection allocation failed")?;
            let required = engine_allocation::collect_vec(entry.pools.required.iter().map(|row| {
                HqRequirement {
                    token: row.token,
                    amount: row.amount,
                }
            }))
            .map_err(|_| "headquarters context requirement projection allocation failed")?;
            Ok(Arc::new(TemplateHqContext {
                target: entry.header.target,
                deployed: entry.header.state[0],
                queued: entry.header.state[1],
                deployment: entry.header.deployment.map(deployment),
                equipment,
                required,
            }))
        })();
        let result = match read {
            Ok(value) => {
                entry.value = Some(Arc::clone(&value));
                entry.valid = true;
                Ok(value)
            }
            Err(reason) => {
                if entry.certificate.is_none()
                    || entry.certificate
                        != certificate(changes, root.pointer(), entry.attempted_holder)
                {
                    entry.attempted = false;
                }
                entry.value = None;
                Err(reason)
            }
        };
        self.payload_bytes = self.payload_bytes - previous_bytes + entry.allocated_bytes();
        result
    }
    pub fn value(&self, group: u64) -> Option<Arc<TemplateHqContext>> {
        self.value_ref(group).cloned()
    }
    pub fn value_ref(&self, group: u64) -> Option<&Arc<TemplateHqContext>> {
        self.entries
            .binary_search_by_key(&group, |entry| entry.group)
            .ok()
            .filter(|&at| self.entries[at].valid)
            .and_then(|at| self.entries[at].value.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template_changes::Watches;
    use engine_native::corders_group;
    use engine_native_parts::{
        creference_registry as registry,
        reference_bucket as bucket,
    };
    struct Fixture(Vec<u8>);
    impl Memory for Fixture {
        fn copy(&self, at: usize, out: &mut [u8]) -> bool {
            if at == registry::LOW_ROOTS_RVA.0 + corders_group::REFERENCE_TYPE as usize * 8
                && out.len() == 8
            {
                out.copy_from_slice(&0x3000_u64.to_le_bytes());
                return true;
            }
            engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &self.0,
            }
            .copy(at, out)
        }
    }
    fn put(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn fixture() -> (Fixture, Root, u64) {
        let mut memory = Fixture(vec![0; 0x5000]);
        let handle = (7_u64 << 32) | 53;
        put(&mut memory.0, 0x1000, corders_group::VTABLE_RVA.0 as u64);
        put(&mut memory.0, 0x1008, handle);
        put(&mut memory.0, 0x1000 + corders_group::COUNTRY, 7);
        put(&mut memory.0, 0x3008, 0x4000);
        put(&mut memory.0, 0x3010, 1 | (7 << 32));
        memory.0[0x3018] = 5;
        memory.0[0x4000 + bucket::DISTANCE] = 1;
        put(&mut memory.0, 0x4000 + bucket::TYPE, handle);
        put(&mut memory.0, 0x4000 + bucket::OBJECT, 0x1000);
        memory.0[0x4000 + 13 * bucket::STRIDE + bucket::DISTANCE] = bucket::TERMINAL_DISTANCE;
        let mut roots = vec![Root::default(); 1];
        assert_eq!(
            corders_group::walk_roots(&memory, 0, &mut roots),
            engine_native_memory::walk::Outcome::Rows { len: 1 }
        );
        (memory, roots[0], handle)
    }
    #[test]
    fn fresh_only_hq_context_rereads_without_certificates_and_preserves_native_refusals() {
        struct Fresh;
        impl Changes for Fresh {
            fn fresh_only(&self) -> bool { true }
            fn quiet(&self) -> Option<(u64, u64)> { None }
            fn sequence(&self, _: usize) -> Option<(u64, u64)> { None }
        }
        let (mut memory, root, handle) = fixture();
        let mut store = Store::default();
        assert_eq!(store.read(&memory, 0, root, &Fresh).unwrap().queued, 0);
        memory.0[0x1000 + 0x19a] = 3;
        assert_eq!(store.read(&memory, 0, root, &Fresh).unwrap().queued, 3);
        put(&mut memory.0, 0x1000 + native::HOLDER, 0x6000);
        assert!(store.read(&memory, 0, root, &Fresh).is_err());
        assert!(store.value(handle).is_none());
    }
    #[test]
    fn template_hq_quiet_cache_has_no_native_reads_or_allocations_and_equal_writer_results_share_payload()
     {
        let (mut memory, root, handle) = fixture();
        let roots = [root];
        let watches = Watches::enabled(32);
        let mut store = Store::default();
        let running = watches.notify(0x1000, 43);
        assert!(store.read(&memory, 0, roots[0], &watches).is_err());
        assert!(store.value(handle).is_none());
        assert!(!store.entries[0].attempted);
        drop(running);
        let first = store.read(&memory, 0, roots[0], &watches).unwrap();
        struct Unreadable;
        impl Memory for Unreadable {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("quiet HQ context must make no native reads")
            }
        }
        collector_test_support::with_allocations_denied(|| {
            assert!(Arc::ptr_eq(
                &first,
                &store.read(&Unreadable, 0, roots[0], &watches).unwrap()
            ));
        });
        drop(watches.notify(0x1000, 44));
        collector_test_support::with_allocations_denied(|| {
            assert!(Arc::ptr_eq(
                &first,
                &store.read(&memory, 0, roots[0], &watches).unwrap()
            ));
        });
        let running = watches.notify(0x1000, 44);
        assert!(store.read(&Unreadable, 0, roots[0], &watches).is_err());
        assert!(store.value(handle).is_none());
        drop(running);
        memory.0[0x1000 + 0x19a] = 3;
        assert_eq!(
            store.read(&memory, 0, roots[0], &watches).unwrap().queued,
            3
        );
        assert_eq!(
            store.payload_bytes,
            store
                .entries
                .iter()
                .map(Entry::allocated_bytes)
                .sum::<usize>()
        );
        for holder in [0x6000, 0x2000, 0x1234_5678_9abc_def0] {
            put(&mut memory.0, 0x1000 + native::HOLDER, holder as u64);
            drop(watches.notify(0x1000, 44));
            assert!(store.read(&memory, 0, roots[0], &watches).is_err());
            assert_eq!(store.entries[0].attempted_holder, holder);
            assert!(store.entries[0].attempted);
            collector_test_support::with_allocations_denied(|| {
                assert!(store.read(&Unreadable, 0, roots[0], &watches).is_err());
            });
            drop(watches.notify(holder, 48));
            assert!(store.read(&memory, 0, roots[0], &watches).is_err());
            assert!(store.entries[0].attempted);
        }
        store.prune(&[]);
        assert!(store.entries.is_empty());
        assert_eq!(store.payload_bytes, 0);
    }
}
