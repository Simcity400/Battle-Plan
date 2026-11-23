use engine_allocation as allocation;
use engine_mutations::{
    air_wing_equipment_mutations::ATTACHMENT_COVERAGE, air_wing_equipment_mutations::TRAINING_STOP,
};
use engine_native::Memory;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Entry {
    address: usize,
    wing: u64,
    certificate: (u64, u64),
    value: Option<u8>,
}

#[derive(Default, Debug)]
pub struct Store {
    previous: Vec<Entry>,
    current: Vec<Entry>,
    training_groups: std::collections::HashSet<(u32, u64, u64)>,
    raid: super::wing_raid_flags::Store,
}
impl Store {
    pub fn raid_reference(
        &self,
        pointer: usize,
        wing: u64,
    ) -> Option<engine_native_air::cair_wing::transport_operands::RaidReference> {
        self.raid.reference(pointer, wing)
    }
    pub fn begin(&mut self, wings: usize, invalidate: bool) {
        self.raid.begin(wings, invalidate);
        if invalidate {
            self.current.clear();
        }
        std::mem::swap(&mut self.current, &mut self.previous);
        self.current.clear();
        self.training_groups.clear();
        self.ensure_capacity(wings);
    }
    pub fn ensure_capacity(&mut self, wings: usize) -> bool {
        let flags = wings.checked_mul(2).is_some_and(|demand| {
            let additional = demand.saturating_sub(self.current.len());
            demand <= self.current.capacity()
                || allocation::advisory(|| allocation::reserve(&mut self.current, additional))
                    .is_ok()
        });
        let groups = wings <= self.training_groups.capacity()
            || self
                .training_groups
                .try_reserve(wings.saturating_sub(self.training_groups.len()))
                .is_ok();
        let raids = self.raid.ensure_capacity(wings);
        flags && groups && raids
    }
    pub fn allocated_bytes(&self) -> u64 {
        ((self.previous.capacity() + self.current.capacity()) * size_of::<Entry>()
            + self.training_groups.capacity() * size_of::<(u32, u64, u64)>()) as u64
            + self.raid.allocated_bytes()
    }
    fn flag(
        &mut self,
        address: usize,
        wing: u64,
        mut certificate: impl FnMut(usize) -> Option<(u64, u64)>,
        mut quiet: impl FnMut() -> Option<(u64, u64)>,
        read: impl FnOnce(usize) -> Option<u8>,
    ) -> Option<bool> {
        if self.current.len() == self.current.capacity() {
            return None;
        }
        let Some(before) = certificate(address) else {
            let token = quiet()?;
            let value = read(address)?;
            return (quiet() == Some(token)).then_some(value != 0);
        };
        let held = self
            .previous
            .binary_search_by_key(&address, |entry| entry.address)
            .ok()
            .map(|index| self.previous[index])
            .filter(|entry| entry.wing == wing && entry.certificate == before);
        let value = match held {
            Some(entry) => entry.value,
            None => read(address),
        };
        if certificate(address) != Some(before) {
            return None;
        }
        if self.current.len() < self.current.capacity() {
            self.current.push(Entry {
                address,
                wing,
                certificate: before,
                value,
            });
        }
        value.map(|value| value != 0)
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: usize,
        row: &engine_native_air::cair_wing::Row,
        observed_raid_pointer: Option<usize>,
        observed_raid_token: Option<(u64, u64)>,
    ) -> u32 {
        let stop_mask = if self.stop_demand(row) { 0x800 } else { 0 };
        let coverage = row.attached_group_pointer != 0
            || (row.mission_mask != 0
                && row.mission_mask & 0x800 == 0
                && row.mission_region.is_none()
                && row.eligibility_veto == 0
                && row.eligibility_counter <= row.eligibility_limit);
        let mut flags = self.capture_with(
            pointer,
            row.id,
            stop_mask,
            coverage,
            collector_observers::wing_changes::map_sequence,
            collector_observers::wing_changes::map_quiet,
            |at| engine_native::read::<u8>(memory, at),
        );
        if let Some(value) = self.raid.capture(
            memory,
            base,
            pointer,
            row,
            observed_raid_pointer,
            observed_raid_token,
        ) {
            flags |= (1 | (u32::from(value) << 1)) << 4;
        }
        flags
    }
    fn stop_demand(&mut self, row: &engine_native_air::cair_wing::Row) -> bool {
        let key = (row.owner_slot, row.air_base, row.attached_group_pointer);
        let training = row.mission_mask & 0x800 != 0;
        if training
            && (self.training_groups.contains(&key)
                || self.training_groups.len() < self.training_groups.capacity())
        {
            self.training_groups.insert(key);
        }
        training || self.training_groups.contains(&key)
    }
    fn capture_with(
        &mut self,
        pointer: usize,
        wing: u64,
        mask: u32,
        attached: bool,
        mut certificate: impl FnMut(usize) -> Option<(u64, u64)>,
        mut quiet: impl FnMut() -> Option<(u64, u64)>,
        mut read: impl FnMut(usize) -> Option<u8>,
    ) -> u32 {
        let mut packed = 0;
        for (demand, offset, bit) in [
            (mask & 0x800 != 0, TRAINING_STOP, 0),
            (attached, ATTACHMENT_COVERAGE, 2),
        ] {
            if !demand {
                continue;
            }
            if let Some(value) = pointer
                .checked_add(offset)
                .and_then(|at| self.flag(at, wing, &mut certificate, &mut quiet, &mut read))
            {
                packed |= (1 | (u32::from(value) << 1)) << bit;
            }
        }
        packed
    }
    pub fn finish(&mut self) {
        self.raid.finish();
        self.current.sort_unstable_by_key(|entry| entry.address);
        let mut index = 0;
        while index < self.current.len() {
            let mut end = index + 1;
            while end < self.current.len()
                && self.current[end].address == self.current[index].address
            {
                end += 1;
            }
            if end - index > 1 {
                self.current.drain(index..end);
            } else {
                index = end;
            }
        }
        self.previous.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use collector_certificates::watch_registry::WatchRegistry;
    fn no_quiet() -> Option<(u64, u64)> {
        panic!("a certified flag took the quiet token")
    }
    #[test]
    fn refused_flags_carry_as_unknown_and_unstorable_flags_do_not_read() {
        let mut store = Store::default();
        assert_eq!(
            store.flag(0x1000, 1, |_| Some((1, 0)), no_quiet, |_| panic!("no storage")),
            None
        );
        store.begin(1, true);
        assert_eq!(store.flag(0x1000, 1, |_| Some((1, 0)), no_quiet, |_| None), None);
        store.finish();
        store.begin(1, false);
        assert_eq!(
            store.flag(
                0x1000,
                1,
                |_| Some((1, 0)),
                no_quiet,
                |_| panic!("quiet refused byte reread")
            ),
            None
        );
        store.finish();
        store.begin(1, false);
        assert_eq!(
            store.flag(0x1000, 1, |_| Some((2, 0)), no_quiet, |_| Some(1)),
            Some(true)
        );
    }
    #[test]
    fn an_observed_registry_without_a_table_reads_flags_between_quiet_tokens() {
        let registry = WatchRegistry::new();
        registry.set_enabled(true);
        let stop = 0x1000 + TRAINING_STOP;
        assert_eq!(registry.sequence(stop), None, "no table certifies a receiver");
        let mut store = Store::default();
        store.begin(1, true);
        assert_eq!(
            store.flag(stop, 1, |at| registry.sequence(at), || registry.quiet(), |_| Some(1)),
            Some(true)
        );
        assert_eq!(
            store.flag(stop, 1, |at| registry.sequence(at), || registry.quiet(), |_| Some(0)),
            Some(false)
        );
        {
            let _running = registry.notify(stop);
            assert_eq!(
                store.flag(
                    stop,
                    1,
                    |at| registry.sequence(at),
                    || registry.quiet(),
                    |_| panic!("read beside a running writer")
                ),
                None
            );
        }
        let calls = std::cell::Cell::new(0);
        let crossed = || {
            calls.set(calls.get() + 1);
            if calls.get() == 2 {
                drop(registry.notify(0));
            }
            registry.quiet()
        };
        assert_eq!(
            store.flag(stop, 1, |at| registry.sequence(at), crossed, |_| Some(1)),
            None,
            "a writer between the tokens"
        );
        assert_eq!(
            store.flag(stop, 1, |at| registry.sequence(at), || registry.quiet(), |_| None),
            None
        );
        let unobserved = WatchRegistry::new();
        assert_eq!(
            store.flag(
                stop,
                1,
                |at| unobserved.sequence(at),
                || unobserved.quiet(),
                |_| panic!("unobserved read")
            ),
            None
        );
        assert!(store.current.is_empty(), "an uncertified read carries nothing");
    }
    #[test]
    fn stop_demand_covers_later_possible_members_and_resets_at_each_census() {
        use engine_native_air::cair_wing::Row;
        let mut store = Store::default();
        store.begin(5000, false);
        let idle = Row {
            owner_slot: 2,
            air_base: 8,
            ..Default::default()
        };
        assert!(!store.stop_demand(&idle));
        assert!(store.stop_demand(&Row {
            mission_mask: 0x800,
            ..idle
        }));
        assert!(store.stop_demand(&idle));
        for different in [
            Row {
                owner_slot: 3,
                ..idle
            },
            Row {
                air_base: 9,
                ..idle
            },
            Row {
                attached_group_pointer: 10,
                ..idle
            },
        ] {
            assert!(!store.stop_demand(&different));
        }
        store.finish();
        store.begin(5000, false);
        assert!(!store.stop_demand(&idle));
    }
    #[test]
    fn quiet_demanded_flags_carry_without_reads_and_unrelated_fields_stay_unknown() {
        let mut store = Store::default();
        store.begin(5000, false);
        for id in 1..=5000 {
            assert_eq!(
                store.capture_with(
                    id as usize * 0x1000,
                    id,
                    0x800,
                    true,
                    |_| Some((1, 0)),
                    no_quiet,
                    |_| Some(1)
                ),
                15
            );
        }
        store.finish();
        store.begin(5000, false);
        for id in 1..=5000 {
            assert_eq!(
                store.capture_with(
                    id as usize * 0x1000,
                    id,
                    0x800,
                    true,
                    |_| Some((1, 0)),
                    no_quiet,
                    |_| panic!("quiet flag read")
                ),
                15
            );
        }
        assert_eq!(
            store.capture_with(0x1000, 1, 0, false, |_| panic!(), no_quiet, |_| panic!()),
            0
        );
        store.finish();
    }
    #[test]
    fn dirty_flags_object_reuse_epochs_and_running_writers_withhold_carry_independently() {
        let mut store = Store::default();
        store.begin(2, false);
        assert_eq!(
            store.capture_with(0x1000, 1, 0x800, true, |_| Some((1, 0)), no_quiet, |_| Some(1)),
            15
        );
        store.finish();
        store.begin(2, false);
        assert_eq!(
            store.capture_with(
                0x1000,
                1,
                0x800,
                true,
                |at| Some((1, u64::from(at == 0x1000 + TRAINING_STOP))),
                no_quiet,
                |at| {
                    assert_eq!(at, 0x1000 + TRAINING_STOP);
                    Some(0)
                }
            ),
            13
        );
        store.finish();
        store.begin(2, false);
        assert_eq!(
            store.capture_with(0x1000, 2, 0x800, true, |_| Some((2, 0)), no_quiet, |_| Some(0)),
            5
        );
        store.finish();
        store.begin(2, false);
        assert_eq!(
            store.capture_with(
                0x1000,
                2,
                0x800,
                true,
                |_| None,
                || None,
                |_| panic!("unobserved polling")
            ),
            0
        );
        let mut calls = 0;
        assert_eq!(
            store.flag(
                0x2000,
                3,
                |_| {
                    calls += 1;
                    Some((1, calls))
                },
                no_quiet,
                |_| Some(1)
            ),
            None
        );
    }
}
