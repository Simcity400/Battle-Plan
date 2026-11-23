use crate::ship_model_changes::Changes;
use engine_allocation as allocation;
use engine_native_memory::Memory;
use engine_native::cship;

#[derive(Clone, Copy)]
struct Entry {
    pointer: usize,
    ship: u64,
    certificate: (u64, u64),
    kind: Option<u32>,
}

#[derive(Default)]
pub(crate) struct Store {
    previous: Vec<Entry>,
    current: Vec<Entry>,
}
impl Store {
    pub fn begin(&mut self, ships: usize) {
        std::mem::swap(&mut self.previous, &mut self.current);
        self.current.clear();
        let _ = allocation::advisory(|| allocation::reserve(&mut self.current, ships));
    }
    pub fn clear(&mut self) {
        self.previous.clear();
        self.current.clear();
    }
    pub fn finish(&mut self) {
        self.current.sort_unstable_by_key(|row| row.pointer);
        let mut index = 0;
        while index < self.current.len() {
            let mut end = index + 1;
            while end < self.current.len()
                && self.current[end].pointer == self.current[index].pointer
            {
                end += 1;
            }
            if end > index + 1 {
                self.current.drain(index..end);
            } else {
                index = end;
            }
        }
        self.previous.clear();
    }
    pub fn len(&self) -> usize {
        self.current.len()
    }
    pub fn allocated_bytes(&self) -> usize {
        (self.previous.capacity() + self.current.capacity()) * size_of::<Entry>()
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: usize,
        row: &cship::ModelRow,
        changes: &dyn Changes,
    ) -> Option<u32> {
        if row.carrier_air_base == 0 {
            return None;
        }
        if row.formation_flags & 0x80 != 0 {
            return None;
        }
        self.capture_with(
            pointer,
            row.ship,
            |at| changes.sequence(at),
            || cship::air_capacity_kind(memory, base, pointer, row.formation_flags).ok(),
        )
    }
    fn capture_with(
        &mut self,
        pointer: usize,
        ship: u64,
        mut sequence: impl FnMut(usize) -> Option<(u64, u64)>,
        read: impl FnOnce() -> Option<u32>,
    ) -> Option<u32> {
        if self.current.len() == self.current.capacity() {
            return None;
        }
        let receiver = pointer.checked_add(cship::MODULE_PREDICATE)?;
        let certificate = sequence(receiver)?;
        let held = self
            .previous
            .binary_search_by_key(&pointer, |row| row.pointer)
            .ok()
            .map(|at| self.previous[at])
            .filter(|row| row.ship == ship && row.certificate == certificate);
        let kind = match held {
            Some(row) => row.kind,
            None => read(),
        };
        if sequence(receiver) != Some(certificate) {
            return None;
        }
        if self.current.len() < self.current.capacity() {
            self.current.push(Entry {
                pointer,
                ship,
                certificate,
                kind,
            });
        }
        kind
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quiet_carriers_do_not_read_and_dirty_or_reused_hosts_do() {
        let mut store = Store::default();
        store.begin(20);
        for ship in 1..=20 {
            assert_eq!(
                store.capture_with(ship as usize * 4096, ship, |_| Some((1, 0)), || Some(2)),
                Some(2)
            );
        }
        store.finish();
        store.begin(20);
        for ship in 1..=20 {
            assert_eq!(
                store.capture_with(
                    ship as usize * 4096,
                    ship,
                    |_| Some((1, 0)),
                    || panic!("quiet host reread")
                ),
                Some(2)
            );
        }
        store.finish();
        store.begin(20);
        let mut reads = 0;
        assert_eq!(
            store.capture_with(
                4096,
                1,
                |_| Some((1, 2)),
                || {
                    reads += 1;
                    Some(1)
                }
            ),
            Some(1)
        );
        assert_eq!(
            store.capture_with(
                8192,
                9000,
                |_| Some((1, 0)),
                || {
                    reads += 1;
                    Some(1)
                }
            ),
            Some(1)
        );
        assert_eq!(reads, 2);
    }
    #[test]
    fn uncertified_capture_never_polls_and_running_writers_refuse_the_field() {
        let mut store = Store::default();
        store.begin(2);
        assert_eq!(
            store.capture_with(4096, 1, |_| None, || panic!("uncertified read")),
            None
        );
        let mut calls = 0;
        assert_eq!(
            store.capture_with(
                4096,
                1,
                |_| {
                    calls += 1;
                    (calls == 1).then_some((1, 0))
                },
                || Some(1)
            ),
            None
        );
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn refused_or_unstorable_values_never_turn_into_periodic_reads() {
        let mut store = Store::default();
        assert_eq!(
            store.capture_with(4096, 1, |_| Some((1, 0)), || panic!("no storage")),
            None
        );
        store.begin(1);
        assert_eq!(store.capture_with(4096, 1, |_| Some((1, 0)), || None), None);
        store.finish();
        store.begin(1);
        assert_eq!(
            store.capture_with(
                4096,
                1,
                |_| Some((1, 0)),
                || panic!("quiet refused value reread")
            ),
            None
        );
        store.finish();
        store.begin(1);
        assert_eq!(
            store.capture_with(4096, 1, |_| Some((2, 0)), || Some(2)),
            Some(2)
        );
    }
}
