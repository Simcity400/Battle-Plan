use engine_native::carmy::{self, DivisionSeed};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

#[cfg(test)]
mod oracle;

static NEXT_BASELINE: AtomicU64 = AtomicU64::new(1);

pub(crate) fn next_baseline() -> Option<u64> {
    NEXT_BASELINE
        .fetch_update(Relaxed, Relaxed, |next| next.checked_add(1))
        .ok()
}

#[derive(Default)]
pub struct Order {
    baseline: Option<u64>,
    entries: Vec<(u64, usize)>,
    shift: isize,
    unique: bool,
    #[cfg(test)]
    rebuilds: usize,
    #[cfg(test)]
    probes: usize,
}
impl Order {
    pub fn clear(&mut self) {
        self.baseline = None;
        self.entries.clear();
        self.shift = 0;
        self.unique = false;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * std::mem::size_of::<(u64, usize)>()
    }
    pub(crate) fn guess(&mut self, baseline: Option<u64>, index: usize) -> Option<usize> {
        if self.baseline != baseline {
            self.clear();
            self.baseline = baseline;
            return None;
        }
        baseline?;
        index.checked_add_signed(self.shift)
    }
    pub(crate) fn set_shift(&mut self, baseline: Option<u64>, shift: isize) {
        if self.baseline != baseline {
            self.clear();
            self.baseline = baseline;
        }
        self.shift = shift;
    }
    pub(crate) fn position(
        &mut self,
        baseline: Option<u64>,
        divisions: impl ExactSizeIterator<Item = u64>,
        division: u64,
    ) -> Option<Option<usize>> {
        baseline?;
        if self.baseline != baseline {
            self.clear();
            engine_allocation::advisory(|| {
                engine_allocation::reserve(&mut self.entries, divisions.len())
            })
            .ok()?;
            self.entries
                .extend(divisions.enumerate().map(|(at, division)| (division, at)));
            self.entries.sort_unstable_by_key(|entry| entry.0);
            self.baseline = baseline;
            #[cfg(test)]
            {
                self.rebuilds += 1;
            }
        }
        Some(
            self.entries
                .binary_search_by_key(&division, |entry| entry.0)
                .ok()
                .map(|at| self.entries[at].1),
        )
    }
    #[cfg(test)]
    pub(crate) fn rebuilds(&self) -> usize {
        self.rebuilds
    }
}

#[derive(Clone, Copy)]
struct Held<T> {
    division: u64,
    army: usize,
    certificate: (u64, u64),
    value: T,
    current: bool,
}
fn is_vtable(base: usize, key: engine_sites::Rva, seen: Option<u64>) -> bool {
    engine_sites::at(base, key).is_some_and(|vtable| seen == Some(vtable as u64))
}
fn identity(base: usize, seed: &DivisionSeed) -> Result<(), &'static str> {
    if seed.division == 0
        || seed.handle != Some(seed.division)
        || seed.class != Some(0)
        || !is_vtable(base, carmy::VTABLE_RVA, seed.primary_vtable)
        || !is_vtable(base, carmy::SECONDARY_VTABLE_RVA, seed.secondary_vtable)
    {
        return Err("division experience census identity unavailable");
    }
    Ok(())
}

#[derive(Default)]
pub struct Store<T = i64> {
    held: Vec<Held<T>>,
    baseline: Option<u64>,
    next: Vec<Held<T>>,
    sorted: Vec<(u64, usize)>,
    sorted_ready: bool,
    held_sorted: bool,
    unique: bool,
    shift: isize,
    daily: bool,
}
impl<T: Clone> Store<T> {
    pub fn daily(&mut self, refresh: bool) {
        self.daily = true;
        if refresh {
            self.forget();
        }
    }
    fn matches(&self, held: &Held<T>, certificate: (u64, u64)) -> bool {
        held.certificate == certificate || self.daily && held.certificate.0 == certificate.0
    }
    pub fn current(&self, seed: &DivisionSeed) -> Option<T> {
        self.next
            .last()
            .filter(|held| held.division == seed.division && held.army == seed.pointer && held.current)
            .map(|held| held.value.clone())
    }
    pub fn begin(&mut self, count: usize) -> Result<(), &'static str> {
        self.next.clear();
        self.shift = 0;
        engine_allocation::reserve(&mut self.next, count)
            .map_err(|_| "division experience cache allocation failed")
    }
    pub fn forget(&mut self) {
        self.held.clear();
        self.baseline = None;
        self.next.clear();
        self.sorted.clear();
        self.sorted_ready = false;
        self.held_sorted = false;
        self.unique = false;
    }
    pub fn allocated_bytes(&self) -> usize {
        (self.held.capacity() + self.next.capacity()) * std::mem::size_of::<Held<T>>()
            + self.sorted.capacity() * std::mem::size_of::<(u64, usize)>()
    }
    pub fn observe_with(
        &mut self,
        base: usize,
        seed: &DivisionSeed,
        certificate: Option<(u64, u64)>,
        read: impl FnOnce() -> Result<T, &'static str>,
    ) -> Result<T, &'static str> {
        identity(base, seed)?;
        let certificate =
            certificate.ok_or("division experience writer certificate unavailable")?;
        let at = self.locate(seed.division)?;
        let held = at
            .map(|at| &self.held[at])
            .filter(|held| held.army == seed.pointer && self.matches(held, certificate));
        let (value, current, certificate) = match held {
            Some(held) => (held.value.clone(), held.certificate == certificate, held.certificate),
            None => (read()?, true, certificate),
        };
        self.push(seed, certificate, value.clone(), current)?;
        Ok(value)
    }

    fn locate(&mut self, division: u64) -> Result<Option<usize>, &'static str> {
        let guess = self.next.len().checked_add_signed(self.shift);
        let at = if guess.is_some_and(|at| {
            self.held
                .get(at)
                .is_some_and(|held| held.division == division)
        }) {
            guess
        } else if self.held.is_empty() {
            None
        } else if self.held_sorted {
            self.held
                .binary_search_by_key(&division, |held| held.division)
                .ok()
        } else {
            if !self.sorted_ready {
                self.sorted.clear();
                engine_allocation::reserve(&mut self.sorted, self.held.len())
                    .map_err(|_| "division experience index allocation failed")?;
                self.sorted.extend(
                    self.held
                        .iter()
                        .enumerate()
                        .map(|(at, held)| (held.division, at)),
                );
                self.sorted.sort_unstable_by_key(|entry| entry.0);
                self.unique = self.sorted.windows(2).all(|pair| pair[0].0 != pair[1].0);
                self.sorted_ready = true;
            }
            self.sorted
                .binary_search_by_key(&division, |entry| entry.0)
                .ok()
                .map(|at| self.sorted[at].1)
        };
        if let Some(at) = at {
            self.shift = at as isize - self.next.len() as isize;
        }
        Ok(at)
    }

    fn push(
        &mut self,
        seed: &DivisionSeed,
        certificate: (u64, u64),
        value: T,
        current: bool,
    ) -> Result<(), &'static str> {
        if self.next.len() == self.next.capacity() {
            return Err("division experience cache capacity exhausted");
        }
        self.next.push(Held {
            division: seed.division,
            army: seed.pointer,
            certificate,
            value,
            current,
        });
        Ok(())
    }

    pub fn quiet(
        &mut self,
        seed: &DivisionSeed,
        receiver: usize,
        taken: &crate::observations::Taken,
    ) -> Option<(usize, (u64, u64))> {
        let at = self.locate(seed.division).ok()??;
        let held = &self.held[at];
        let quiet = if self.daily {
            taken.epoch() == Some(held.certificate.0)
        } else {
            taken.quiet(receiver, held.certificate.0)
        };
        let quiet = held.army == seed.pointer && quiet;
        quiet.then_some((at, held.certificate))
    }

    pub fn keep(
        &mut self,
        base: usize,
        seed: &DivisionSeed,
        at: usize,
        touched: bool,
    ) -> Result<T, &'static str> {
        identity(base, seed)?;
        let held = self.held[at].clone();
        self.push(seed, held.certificate, held.value.clone(), held.current && !touched)?;
        Ok(held.value)
    }
    #[cfg(test)]
    pub(super) fn prefetch(
        &self,
        base: usize,
        index: usize,
        seed: &DivisionSeed,
        certificate: Option<(u64, u64)>,
        order: &mut Order,
        read: impl FnOnce() -> Result<T, &'static str>,
    ) -> Option<Result<T, &'static str>> {
        if seed.division == 0
            || seed.handle != Some(seed.division)
            || seed.class != Some(0)
            || !is_vtable(base, carmy::VTABLE_RVA, seed.primary_vtable)
            || !is_vtable(base, carmy::SECONDARY_VTABLE_RVA, seed.secondary_vtable)
        {
            return None;
        }
        let certificate = certificate?;
        match self.prefetch_held(index, seed, certificate, order) {
            Some(_) => None,
            None => Some(read()),
        }
    }

    fn prefetch_held_at(
        &self,
        index: usize,
        division: u64,
        order: &mut Order,
    ) -> Option<&Held<T>> {
        #[cfg(test)]
        {
            order.probes += 1;
        }
        if order.baseline != self.baseline {
            order.clear();
            order.baseline = self.baseline;
        }
        let unique = self.unique || order.unique;
        let guess = (self.baseline.is_some() && unique)
            .then(|| index.checked_add_signed(order.shift))
            .flatten();
        let at = if self
            .held
            .get(index)
            .is_some_and(|held| held.division == division)
        {
            Some(index)
        } else if guess.is_some_and(|at| {
            self.held
                .get(at)
                .is_some_and(|held| held.division == division)
        }) {
            guess
        } else if self.held.is_empty() {
            return None;
        } else if self.held_sorted {
            self.held
                .binary_search_by_key(&division, |held| {
                    #[cfg(test)]
                    {
                        order.probes += 1;
                    }
                    held.division
                })
                .ok()
        } else if self.sorted_ready {
            self.sorted
                .binary_search_by_key(&division, |entry| {
                    #[cfg(test)]
                    {
                        order.probes += 1;
                    }
                    entry.0
                })
                .ok()
                .map(|at| self.sorted[at].1)
        } else {
            if self.baseline.is_none() || order.entries.is_empty() {
                order.entries.clear();
                if engine_allocation::advisory(|| {
                    engine_allocation::reserve(&mut order.entries, self.held.len())
                })
                .is_ok()
                {
                    order.entries.extend(
                        self.held
                            .iter()
                            .enumerate()
                            .map(|(at, held)| (held.division, at)),
                    );
                    order.entries.sort_unstable_by_key(|entry| entry.0);
                    order.unique = order.entries.windows(2).all(|pair| pair[0].0 != pair[1].0);
                    order.baseline = self.baseline;
                    #[cfg(test)]
                    {
                        order.rebuilds += 1;
                    }
                } else {
                    return self.held.iter().find(|held| {
                        #[cfg(test)]
                        {
                            order.probes += 1;
                        }
                        held.division == division
                    });
                }
            }
            order
                .entries
                .binary_search_by_key(&division, |entry| {
                    #[cfg(test)]
                    {
                        order.probes += 1;
                    }
                    entry.0
                })
                .ok()
                .map(|at| order.entries[at].1)
        };
        if let Some(at) = at {
            order.shift = at as isize - index as isize;
        }
        at.and_then(|at| self.held.get(at))
            .filter(|held| held.division == division)
    }

    fn prefetch_held(
        &self,
        index: usize,
        seed: &DivisionSeed,
        certificate: (u64, u64),
        order: &mut Order,
    ) -> Option<&Held<T>> {
        self.prefetch_held_at(index, seed.division, order)
            .filter(|held| held.army == seed.pointer && self.matches(held, certificate))
    }

    pub(super) fn prefetch_quiet(
        &self,
        index: usize,
        seed: &DivisionSeed,
        receiver: usize,
        taken: &crate::observations::Taken,
        order: &mut Order,
    ) -> Option<((u64, u64), T)> {
        taken.epoch()?;
        let held = self.prefetch_held_any(index, seed, order)?;
        (held.current && taken.quiet(receiver, held.certificate.0))
            .then(|| (held.certificate, held.value.clone()))
    }

    fn prefetch_held_any(
        &self,
        index: usize,
        seed: &DivisionSeed,
        order: &mut Order,
    ) -> Option<&Held<T>> {
        self.prefetch_held_at(index, seed.division, order)
            .filter(|held| held.army == seed.pointer)
    }

    pub(super) fn prefetch_value(
        &self,
        base: usize,
        index: usize,
        seed: &DivisionSeed,
        certificate: Option<(u64, u64)>,
        order: &mut Order,
        read: impl FnOnce() -> Result<T, &'static str>,
    ) -> Option<Result<T, &'static str>> {
        let certificate = certificate?;
        if let Some(held) = self.prefetch_held(index, seed, certificate, order) {
            return Some(Ok(held.value.clone()));
        }
        if seed.division == 0
            || seed.handle != Some(seed.division)
            || seed.class != Some(0)
            || !is_vtable(base, carmy::VTABLE_RVA, seed.primary_vtable)
            || !is_vtable(base, carmy::SECONDARY_VTABLE_RVA, seed.secondary_vtable)
        {
            return None;
        }
        Some(read())
    }

    pub fn finish(&mut self, complete: bool) {
        if !complete {
            self.forget();
            return;
        }
        std::mem::swap(&mut self.held, &mut self.next);
        self.baseline = next_baseline();
        self.next.clear();
        self.sorted.clear();
        self.held_sorted = self
            .held
            .windows(2)
            .all(|pair| pair[0].division < pair[1].division);
        if self.held_sorted {
            self.sorted_ready = true;
            self.unique = true;
        } else if engine_allocation::advisory(|| {
            engine_allocation::reserve(&mut self.sorted, self.held.len())
        })
        .is_ok()
        {
            self.sorted.extend(
                self.held
                    .iter()
                    .enumerate()
                    .map(|(at, held)| (held.division, at)),
            );
            self.sorted.sort_unstable_by_key(|entry| entry.0);
            self.unique = self.sorted.windows(2).all(|pair| pair[0].0 != pair[1].0);
            self.sorted_ready = true;
        } else {
            self.sorted_ready = false;
            self.unique = false;
        }
    }
}

#[cfg(test)]
impl Store<i64> {
    pub fn observe(
        &mut self,
        memory: &impl engine_native_memory::Memory,
        base: usize,
        seed: &DivisionSeed,
        certificate: Option<(u64, u64)>,
    ) -> Result<i64, &'static str> {
        self.observe_with(base, seed, certificate, || {
            engine_native_memory::read::<i64>(
                memory,
                seed.pointer
                    .checked_add(carmy::EXPERIENCE)
                    .ok_or("division experience address overflow")?,
            )
            .ok_or("division experience unreadable")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::Memory;
    use std::cell::Cell;
    struct Value {
        value: i64,
        reads: Cell<usize>,
        fail: bool,
    }
    impl Memory for Value {
        fn copy(&self, _: usize, out: &mut [u8]) -> bool {
            self.reads.set(self.reads.get() + 1);
            if self.fail {
                return false;
            }
            out.copy_from_slice(&self.value.to_le_bytes());
            true
        }
    }
    fn seed(id: u64, pointer: usize) -> DivisionSeed {
        DivisionSeed {
            pointer,
            division: id,
            handle: Some(id),
            class: Some(0),
            primary_vtable: Some(carmy::VTABLE_RVA.0 as u64),
            secondary_vtable: Some(carmy::SECONDARY_VTABLE_RVA.0 as u64),
            ..Default::default()
        }
    }
    #[test]
    fn provisional_fresh_values_never_replace_ordered_carries_or_admission() {
        let mut store = Store::<i64>::default();
        let mut order = Order::default();
        let seeds = [seed(1, 0x1000), seed(2, 0x2000), seed(3, 0x3000)];
        store.begin(seeds.len()).unwrap();
        for seed in &seeds {
            store
                .observe_with(0, seed, Some((1, 1)), || Ok(seed.division as i64))
                .unwrap();
        }
        store.finish(true);
        store.begin(seeds.len()).unwrap();
        for (index, seed) in seeds.iter().rev().enumerate() {
            assert_eq!(
                store.prefetch(0, index, seed, Some((1, 1)), &mut order, || panic!(
                    "a carry must not be prefetched"
                )),
                None
            );
            assert_eq!(
                store.observe_with(0, seed, Some((1, 1)), || panic!("ordered carry was lost")),
                Ok(seed.division as i64)
            );
        }
        let seed = seed(4, 0x4000);
        assert_eq!(
            store.prefetch(0, 3, &seed, None, &mut order, || panic!(
                "absent certificate must not read"
            )),
            None
        );
        let staged = store.prefetch(0, 3, &seed, Some((1, 2)), &mut order, || {
            Err("fresh scalar fault")
        });
        assert_eq!(staged, Some(Err("fresh scalar fault")));
        assert_eq!(
            store.observe_with(0, &seed, None, || panic!(
                "failed admission must not consume a staged read"
            )),
            Err("division experience writer certificate unavailable")
        );
        assert_eq!(store.next.len(), 3);
    }

    #[test]
    fn one_scalar_lookup_matches_the_two_lookup_path_and_its_refusals() {
        let mut store = Store::<i64>::default();
        let certificate = Some((1, 1));
        store.begin(3).unwrap();
        for id in 1..=3 {
            store
                .observe_with(0, &seed(id, id as usize), certificate, || Ok(-(id as i64)))
                .unwrap();
        }
        store.finish(true);
        let mut old_order = Order::default();
        let mut order = Order::default();
        for denied in [false, true] {
            for index in 0..5 {
                for id in 1..=5 {
                    for certificate in [None, certificate, Some((1, 2)), Some((2, 1))] {
                        for invalid in [false, true] {
                            let mut seed = seed(id, id as usize);
                            if invalid {
                                seed.class = None;
                            }
                            if denied {
                                old_order = Order::default();
                                order = Order::default();
                            } else {
                                old_order.clear();
                                order.clear();
                            }
                            let compare = || {
                                let old_reads = Cell::new(0);
                                let reads = Cell::new(0);
                                let read = |count: &Cell<usize>| {
                                    count.set(count.get() + 1);
                                    Err("fresh scalar fault")
                                };
                                let old = store
                                    .prefetch(0, index, &seed, certificate, &mut old_order, || {
                                        read(&old_reads)
                                    })
                                    .or_else(|| {
                                        store
                                            .prefetch_held(index, &seed, certificate?, &mut old_order)
                                            .map(|held| Ok(held.value))
                                    });
                                let actual = store.prefetch_value(
                                    0, index, &seed, certificate, &mut order, || read(&reads),
                                );
                                assert_eq!(actual, old);
                                assert_eq!(reads.get(), old_reads.get());
                            };
                            if denied {
                                collector_test_support::with_allocations_denied(compare);
                            } else {
                                let mut compare = compare;
                                compare();
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn unchanged_experience_carries_divisions_without_reading() {
        const COUNT: u64 = 20;
        let mut store = Store::default();
        let mut memory = Value {
            value: i64::MIN,
            reads: Cell::new(0),
            fail: false,
        };
        store.begin(COUNT as usize).unwrap();
        for id in 1..=COUNT {
            assert_eq!(
                store.observe(&memory, 0, &seed(id, id as usize), Some((1, 1))),
                Ok(i64::MIN)
            );
        }
        store.finish(true);
        memory.reads.set(0);
        memory.fail = true;
        store.begin(COUNT as usize).unwrap();
        for id in 1..=COUNT {
            assert_eq!(
                store.observe(&memory, 0, &seed(id, id as usize), Some((1, 1))),
                Ok(i64::MIN)
            );
        }
        assert_eq!(memory.reads.get(), 0);
        store.finish(true);
        memory.fail = false;
        memory.value = i64::MAX;
        store.begin(COUNT as usize).unwrap();
        for id in 1..=COUNT {
            store
                .observe(
                    &memory,
                    0,
                    &seed(id, id as usize),
                    Some((1, if id == 12 { 2 } else { 1 })),
                )
                .unwrap();
        }
        assert_eq!(memory.reads.get(), 1);
        store.finish(true);
        store.begin(1).unwrap();
        memory.fail = true;
        assert_eq!(
            store.observe(&memory, 0, &seed(12, 99), Some((1, 2))),
            Err("division experience unreadable")
        );
        assert_eq!(
            store.observe(&memory, 0, &seed(12, 12), Some((2, 2))),
            Err("division experience unreadable")
        );
    }

    #[test]
    fn daily_scalars_keep_the_sample_but_never_certify_stale_exercise_xp() {
        let mut store = Store::<i64>::default();
        let original = seed(1, 0x1000);
        store.daily(true);
        store.begin(1).unwrap();
        assert_eq!(
            store.observe_with(0, &original, Some((1, 1)), || Ok(-7)),
            Ok(-7)
        );
        assert_eq!(store.current(&original), Some(-7));
        store.finish(true);
        store.daily(false);
        store.begin(2).unwrap();
        assert_eq!(
            store.observe_with(0, &original, Some((1, 2)), || panic!("daily carry read")),
            Ok(-7)
        );
        assert_eq!(store.current(&original), None);
        let born = seed(2, 0x2000);
        assert_eq!(
            store.observe_with(0, &born, Some((1, 2)), || Ok(19)),
            Ok(19)
        );
        store.finish(true);
        store.begin(1).unwrap();
        assert_eq!(
            store.observe_with(0, &seed(1, 0x3000), Some((1, 2)), || Ok(21)),
            Ok(21)
        );
        store.finish(true);
        store.daily(true);
        store.begin(1).unwrap();
        assert_eq!(
            store.observe_with(0, &original, Some((1, 2)), || Ok(23)),
            Ok(23)
        );
        assert_eq!(store.current(&original), Some(23));
        store.finish(false);
        store.begin(1).unwrap();
        assert_eq!(
            store.observe_with(0, &original, Some((1, 2)), || Ok(29)),
            Ok(29)
        );
    }
}
