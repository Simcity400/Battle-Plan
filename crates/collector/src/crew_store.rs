//! Worker-owned acquired-crew scalars, carried only for an unchanged full
//! handle and writer certificate. This rides the existing unit census.
use engine_allocation as allocation;
use engine_native::{Memory, read};
#[derive(Clone, Copy, Debug)]
struct Entry {
    object: usize,
    handle: u64,
    token: (u64, u64),
    value: Option<[i32; 2]>,
}
#[derive(Default, Debug)]
pub(crate) struct CrewStore {
    previous: Vec<Entry>,
    current: Vec<Entry>,
    counts: [u64; 3],
}
impl CrewStore {
    pub fn begin(&mut self, demand: usize) {
        std::mem::swap(&mut self.previous, &mut self.current);
        self.current.clear();
        self.counts = [0; 3];
        collector_observers::crew_changes::offer(demand.max(self.previous.len()));
        let _ = allocation::reserve(&mut self.current, demand.max(self.previous.len()));
    }
    pub fn read(
        &mut self,
        memory: &impl Memory,
        object: usize,
        handle: u64,
        offset: usize,
    ) -> Option<[i32; 2]> {
        let Some(token) = collector_observers::crew_changes::sequence(object) else {
            self.counts[2] += 1;
            return None;
        };
        self.read_certified(object, handle, token, || {
            let bytes = read::<[u8; 8]>(memory, object.checked_add(offset)?)?;
            let value = [
                i32::from_le_bytes(bytes[..4].try_into().unwrap()),
                i32::from_le_bytes(bytes[4..].try_into().unwrap()),
            ];
            (collector_observers::crew_changes::sequence(object) == Some(token)).then_some(value)
        })
    }
    pub fn read_air(
        &mut self,
        memory: &impl Memory,
        object: usize,
        handle: u64,
    ) -> Option<[i32; 2]> {
        let Some(token) = collector_observers::crew_changes::sequence(object) else {
            self.counts[2] += 1;
            return None;
        };
        self.read_certified(object, handle, token, || {
            let value = [
                read::<i32>(
                    memory,
                    object.checked_add(engine_native::cair_wing::ACQUIRED_MANPOWER)?,
                )?,
                read::<i32>(
                    memory,
                    object.checked_add(engine_native::cair_wing::EXILE_COUNTRY)?,
                )?,
            ];
            (collector_observers::crew_changes::sequence(object) == Some(token)).then_some(value)
        })
    }
    fn read_certified(
        &mut self,
        object: usize,
        handle: u64,
        token: (u64, u64),
        acquire: impl FnOnce() -> Option<[i32; 2]>,
    ) -> Option<[i32; 2]> {
        let old = self
            .previous
            .binary_search_by_key(&object, |entry| entry.object)
            .ok()
            .map(|index| self.previous[index]);
        let value = match old.filter(|entry| entry.handle == handle && entry.token == token) {
            Some(entry) => {
                self.counts[1] += 1;
                entry.value
            }
            None => {
                self.counts[0] += 1;
                acquire()
            }
        };
        if value.is_none() {
            self.counts[2] += 1;
        }
        allocation::reserve(&mut self.current, 1).ok()?;
        self.current.push(Entry {
            object,
            handle,
            token,
            value,
        });
        value
    }
    pub fn finish(&mut self) {
        self.current.sort_unstable_by_key(|entry| entry.object);
        collector_observers::crew_changes::record(self.counts);
    }
    pub fn forget(&mut self) {
        self.previous.clear();
        self.current.clear();
    }
    pub fn allocated_bytes(&self) -> usize {
        (self.current.capacity() + self.previous.capacity()) * size_of::<Entry>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manpower_crews_carry_quiet_5000_and_refresh_only_dirty_or_replaced() {
        let mut store = CrewStore::default();
        for pass in 0..4 {
            let growths = allocation::growths();
            store.begin(5_000);
            let mut reads = 0;
            for index in 0..5_000 {
                let dirty = index == 41 && pass >= 2;
                let replaced = index == 99 && pass >= 3;
                let value = store.read_certified(
                    index + 1,
                    index as u64 + u64::from(replaced) * 5_000,
                    (1, u64::from(dirty)),
                    || {
                        reads += 1;
                        Some([-7, 19])
                    },
                );
                assert_eq!(value, Some([-7, 19]));
            }
            store.finish();
            if pass >= 2 {
                assert_eq!(allocation::growths(), growths);
            }
            assert_eq!(
                reads,
                if pass == 0 {
                    5_000
                } else if pass == 1 {
                    0
                } else {
                    1
                }
            );
        }
        store.begin(5_000);
        assert_eq!(store.read_certified(42, 41, (2, 0), || None), None);
        store.finish();
        store.begin(5_000);
        assert_eq!(
            store.read_certified(42, 41, (2, 0), || panic!("quiet refusal must not retry")),
            None
        );
        store.finish();
        store.begin(5_000);
        assert_eq!(
            store.read_certified(42, 41, (2, 1), || Some([3, 0])),
            Some([3, 0])
        );
    }
}
