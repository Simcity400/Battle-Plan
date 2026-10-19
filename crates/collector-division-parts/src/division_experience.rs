//! Stored division experience carried under its own writer certificate.
//! Each observation uses the existing authenticated census seed. There is no
//! independent roster walk, no native read for an unchanged certificate, and
//! both cache vectors reserve from census demand before any entity is visited.
use engine_native::{
    Memory,
    carmy::{self, DivisionSeed},
};

#[derive(Clone, Copy)]
struct Held<T> {
    division: u64,
    army: usize,
    certificate: (u64, u64),
    value: T,
}
#[derive(Default)]
pub struct Store<T = i64> {
    held: Vec<Held<T>>,
    next: Vec<Held<T>>,
    order: Vec<(u64, usize)>,
    shift: isize,
}
impl<T: Copy> Store<T> {
    pub fn begin(&mut self, count: usize) -> Result<(), &'static str> {
        self.next.clear();
        self.order.clear();
        self.shift = 0;
        engine_allocation::reserve(&mut self.next, count)
            .map_err(|_| "division experience cache allocation failed")
    }
    pub fn forget(&mut self) {
        self.held.clear();
        self.next.clear();
        self.order.clear();
    }
    pub fn allocated_bytes(&self) -> usize {
        (self.held.capacity() + self.next.capacity()) * std::mem::size_of::<Held<T>>()
            + self.order.capacity() * std::mem::size_of::<(u64, usize)>()
    }
    pub fn observe_with(
        &mut self,
        base: usize,
        seed: &DivisionSeed,
        certificate: Option<(u64, u64)>,
        read: impl FnOnce() -> Result<T, &'static str>,
    ) -> Result<T, &'static str> {
        if seed.division == 0
            || seed.handle != Some(seed.division)
            || seed.class != Some(0)
            || base.checked_add(carmy::VTABLE_RVA).map(|v| v as u64) != seed.primary_vtable
            || base
                .checked_add(carmy::SECONDARY_VTABLE_RVA)
                .map(|v| v as u64)
                != seed.secondary_vtable
        {
            return Err("division experience census identity unavailable");
        }
        let certificate =
            certificate.ok_or("division experience writer certificate unavailable")?;
        // Census order normally stays unchanged. Only a membership/order change
        // builds the reusable index; a quiet hour does no sort or second pass.
        let guess = self.next.len().checked_add_signed(self.shift);
        let at = if guess.is_some_and(|at| {
            self.held
                .get(at)
                .is_some_and(|held| held.division == seed.division)
        }) {
            guess
        } else {
            if self.order.is_empty() && !self.held.is_empty() {
                engine_allocation::reserve(&mut self.order, self.held.len())
                    .map_err(|_| "division experience index allocation failed")?;
                self.order.extend(
                    self.held
                        .iter()
                        .enumerate()
                        .map(|(at, held)| (held.division, at)),
                );
                self.order.sort_unstable_by_key(|entry| entry.0);
            }
            self.order
                .binary_search_by_key(&seed.division, |entry| entry.0)
                .ok()
                .map(|at| self.order[at].1)
        };
        if let Some(at) = at {
            self.shift = at as isize - self.next.len() as isize;
        }
        let held = at
            .map(|at| self.held[at])
            .filter(|held| held.army == seed.pointer && held.certificate == certificate);
        let value = match held {
            Some(held) => held.value,
            None => read()?,
        };
        {
            if self.next.len() == self.next.capacity() {
                return Err("division experience cache capacity exhausted");
            }
            self.next.push(Held {
                division: seed.division,
                army: seed.pointer,
                certificate,
                value,
            });
        }
        Ok(value)
    }
    pub fn finish(&mut self, complete: bool) {
        if !complete {
            self.forget();
            return;
        }
        std::mem::swap(&mut self.held, &mut self.next);
        self.next.clear();
    }
}

impl Store<i64> {
    pub fn observe(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seed: &DivisionSeed,
        certificate: Option<(u64, u64)>,
    ) -> Result<i64, &'static str> {
        self.observe_with(base, seed, certificate, || {
            engine_native::read::<i64>(
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
            primary_vtable: Some(carmy::VTABLE_RVA as u64),
            secondary_vtable: Some(carmy::SECONDARY_VTABLE_RVA as u64),
            ..Default::default()
        }
    }
    #[test]
    fn unchanged_experience_carries_five_thousand_divisions_without_reading() {
        let mut store = Store::default();
        let mut memory = Value {
            value: i64::MIN,
            reads: Cell::new(0),
            fail: false,
        };
        store.begin(5000).unwrap();
        for id in 1..=5000 {
            assert_eq!(
                store.observe(&memory, 0, &seed(id, id as usize), Some((1, 1))),
                Ok(i64::MIN)
            );
        }
        store.finish(true);
        memory.reads.set(0);
        memory.fail = true;
        store.begin(5000).unwrap();
        for id in 1..=5000 {
            assert_eq!(
                store.observe(&memory, 0, &seed(id, id as usize), Some((1, 1))),
                Ok(i64::MIN)
            );
        }
        assert_eq!(memory.reads.get(), 0);
        store.finish(true);
        memory.fail = false;
        memory.value = i64::MAX;
        store.begin(5000).unwrap();
        for id in 1..=5000 {
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
}
