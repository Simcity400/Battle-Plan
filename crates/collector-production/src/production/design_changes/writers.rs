use super::*;

static WRITERS: OnceLock<Mutex<State>> = OnceLock::new();
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Footprint {
    Design,
    Identity,
    Ancestry,
    Lifetime,
}
struct Claim {
    thread: u32,
    count: usize,
    writers: usize,
    identities: usize,
    ancestry: usize,
    lifetime: usize,
    conflicted: bool,
    dependency_conflicted: bool,
}
#[derive(Default)]
struct PointerHasher(u64);

impl std::hash::Hasher for PointerHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 = self.0.wrapping_mul(0x517c_c1b7_2722_0a95) ^ u64::from(byte);
        }
    }
    #[inline]
    fn write_usize(&mut self, value: usize) {
        self.0 = (value as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }
    #[inline]
    fn finish(&self) -> u64 {
        self.0 ^ (self.0 >> 32)
    }
}

type PointerBuildHasher = std::hash::BuildHasherDefault<PointerHasher>;

#[derive(Default)]
struct State {
    claims: HashMap<usize, Claim, PointerBuildHasher>,
    required: usize,
    ancestry: usize,
    lifetime: usize,
}
pub(super) fn prepare(population: usize) -> Result<(), &'static str> {
    prepare_with(population, || {})
}
fn prepare_with(population: usize, before_growth: impl FnOnce()) -> Result<(), &'static str> {
    let writers = WRITERS.get_or_init(|| Mutex::new(State::default()));
    let mut state = writers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if DESIGN_GATE.any_active() || !state.claims.is_empty() {
        return Err("variant receiver claims have a running writer");
    }
    let required = population.max(state.required);
    let replacement = if state.claims.capacity() < required {
        drop(state);
        before_growth();
        let mut claims = HashMap::with_hasher(PointerBuildHasher::default());
        engine_allocation::reserve_map(&mut claims, required)
            .map_err(|_| "variant receiver claim allocation failed")?;
        state = writers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if DESIGN_GATE.any_active() || !state.claims.is_empty() {
            drop(state);
            return Err("variant receiver claims have a running writer");
        }
        if claims.capacity() < population.max(state.required) {
            drop(state);
            return Err("variant receiver claim demand changed during preparation");
        }
        Some(claims)
    } else {
        None
    };
    state.ancestry = 0;
    state.lifetime = 0;
    let replaced = replacement.map(|claims| std::mem::replace(&mut state.claims, claims));
    drop(state);
    drop(replaced);
    Ok(())
}
pub(super) fn release() {
    if let Some(state) = WRITERS.get() {
        *state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = State::default();
    }
}
fn room(state: &mut State, additional: usize) -> bool {
    state.required = state
        .required
        .max(state.claims.len().saturating_add(additional));
    let additional = state.required.saturating_sub(state.claims.len());
    if state.required > state.claims.capacity()
        && engine_allocation::advisory(|| engine_allocation::reserve_map(&mut state.claims, additional))
            .is_err()
    {
        CLAIM_CAPACITY_REFUSALS.fetch_add(1, Ordering::Relaxed);
        unobserved!();
        return false;
    }
    true
}
fn claim(state: &mut State, pointer: usize, thread: u32, footprint: Footprint) -> bool {
    let mut conflicted = false;
    if !state.claims.is_empty() {
        conflicted = state.claims.get_mut(&0).is_some_and(|claim| {
            claim.conflicted |= claim.thread != thread;
            claim.dependency_conflicted |= claim.thread != thread;
            claim.conflicted
        });
        if pointer == 0
            || (footprint == Footprint::Ancestry && state.lifetime != 0)
            || (footprint == Footprint::Lifetime && state.ancestry != 0)
        {
            for claim in state.claims.values_mut() {
                if (claim.thread != thread || claim.conflicted || claim.identities != 0)
                    && (pointer == 0
                        || (footprint == Footprint::Ancestry && claim.lifetime != 0)
                        || (footprint == Footprint::Lifetime && claim.ancestry != 0))
                {
                    claim.conflicted = true;
                    claim.dependency_conflicted = true;
                    conflicted = true;
                }
            }
        }
    }
    let claim = state.claims.entry(pointer).or_insert(Claim {
        thread,
        count: 0,
        writers: 0,
        identities: 0,
        ancestry: 0,
        lifetime: 0,
        conflicted: false,
        dependency_conflicted: false,
    });
    if footprint == Footprint::Identity {
        claim.dependency_conflicted |=
            conflicted || (claim.lifetime != 0 && claim.thread != thread);
        claim.identities += 1;
    } else {
        if claim.writers == 0 {
            claim.thread = thread;
        }
        claim.conflicted |= conflicted
            || claim.thread != thread
            || (footprint == Footprint::Lifetime && claim.identities != 0);
        claim.dependency_conflicted |= conflicted
            || (claim.conflicted && (footprint == Footprint::Lifetime || claim.lifetime != 0));
        claim.writers += 1;
    }
    claim.count += 1;
    claim.ancestry += usize::from(footprint == Footprint::Ancestry);
    claim.lifetime += usize::from(footprint == Footprint::Lifetime);
    let admitted = if footprint == Footprint::Identity {
        !claim.dependency_conflicted
    } else {
        !claim.conflicted
    };
    state.ancestry += usize::from(footprint == Footprint::Ancestry);
    state.lifetime += usize::from(footprint == Footprint::Lifetime);
    state.required = state.required.max(state.claims.len());
    admitted
}
pub(super) struct Flight {
    pointers: [Option<(usize, Footprint)>; 2],
    thread: u32,
    admitted: bool,
}
pub(super) fn enter(pointer: usize, other: Option<usize>, footprint: Footprint) -> Option<Flight> {
    let Some(state) = WRITERS.get() else {
        unknown_receiver!();
        return None;
    };
    let thread = unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() };
    let mut state = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let other = other.filter(|other| *other != pointer);
    if state.required > state.claims.capacity()
        || state.claims.capacity() - state.claims.len() < 1 + usize::from(other.is_some())
    {
        let additional = usize::from(!state.claims.contains_key(&pointer))
            + usize::from(other.is_some_and(|other| !state.claims.contains_key(&other)));
        if !room(&mut state, additional) {
            return None;
        }
    }
    let mut admitted = claim(&mut state, pointer, thread, footprint);
    if let Some(other) = other {
        admitted &= claim(&mut state, other, thread, Footprint::Lifetime);
    }
    if !admitted {
        COMPETING_WRITER_REJECTIONS.fetch_add(1, Ordering::Relaxed);
    }
    Some(Flight {
        pointers: [
            Some((pointer, footprint)),
            other.map(|pointer| (pointer, Footprint::Lifetime)),
        ],
        thread,
        admitted,
    })
}
impl Flight {
    pub(super) fn admitted(&self) -> bool {
        self.admitted
    }
    pub(super) fn extend(&mut self, pointer: usize) {
        let mut state = WRITERS
            .get()
            .unwrap()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.required > state.claims.capacity() || state.claims.len() == state.claims.capacity() {
            let additional = usize::from(!state.claims.contains_key(&pointer));
            if !room(&mut state, additional) {
                self.admitted = false;
                return;
            }
        }
        self.pointers[1] = Some((pointer, Footprint::Design));
        if !claim(&mut state, pointer, self.thread, Footprint::Design) {
            if self.admitted {
                COMPETING_WRITER_REJECTIONS.fetch_add(1, Ordering::Relaxed);
            }
            self.admitted = false;
        }
    }
    pub(super) fn finish_with_dependency(mut self) -> (bool, bool) {
        self.close()
    }
    fn close(&mut self) -> (bool, bool) {
        if self.pointers[0].is_none() {
            return (false, false);
        }
        let pointers = std::mem::take(&mut self.pointers);
        let mut state = WRITERS
            .get()
            .unwrap()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut conflicted = false;
        let mut dependency = false;
        for (pointer, footprint) in pointers.into_iter().flatten() {
            let Some(mut claim) = state.claims.remove(&pointer) else {
                conflicted = true;
                dependency = true;
                unobserved!();
                continue;
            };
            conflicted |= if footprint == Footprint::Identity {
                claim.dependency_conflicted
            } else {
                claim.conflicted
            };
            dependency |= claim.dependency_conflicted;
            claim.count -= 1;
            claim.writers -= usize::from(footprint != Footprint::Identity);
            claim.identities -= usize::from(footprint == Footprint::Identity);
            claim.ancestry -= usize::from(footprint == Footprint::Ancestry);
            claim.lifetime -= usize::from(footprint == Footprint::Lifetime);
            if claim.count != 0 {
                state.claims.insert(pointer, claim);
            }
            state.ancestry -= usize::from(footprint == Footprint::Ancestry);
            state.lifetime -= usize::from(footprint == Footprint::Lifetime);
        }
        (conflicted, dependency)
    }
}
impl Drop for Flight {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "synthetic per-call claim measurement"]
    fn hook_cost_variant_receiver_claims() {
        const CALLS: usize = 100_000;
        prepare(5_000).unwrap();
        let cost = collector_test_support::per_call_ns(CALLS, || {
            for pointer in 1..=CALLS {
                let flight = enter(
                    std::hint::black_box(pointer % 5_000 + 1),
                    None,
                    Footprint::Design,
                )
                .unwrap();
                assert!(flight.admitted());
                assert!(!flight.finish_with_dependency().0);
            }
        });
        let state = WRITERS.get().unwrap().lock().unwrap();
        assert!(state.claims.is_empty());
        assert_eq!((state.ancestry, state.lifetime), (0, 0));
        println!("hook_cost variant receiver claim enter+finish: {cost:.1} ns/call");
    }

    #[test]
    fn claim_growth_allows_callback_entry_and_revalidates_running_flights() {
        if !isolated() {
            return;
        }
        use std::sync::Barrier;
        prepare(1).unwrap();
        let capacity = WRITERS.get().unwrap().lock().unwrap().claims.capacity();
        let pause = Barrier::new(2);
        std::thread::scope(|threads| {
            let worker = threads.spawn(|| {
                prepare_with(capacity + 1, || {
                    pause.wait();
                    pause.wait();
                })
            });
            pause.wait();
            let active_guard = DESIGN_GATE.enter(false);
            let flight = collector_test_support::with_allocations_denied(|| {
                enter(1, None, Footprint::Design).unwrap()
            });
            assert!(flight.admitted());
            pause.wait();
            assert_eq!(
                worker.join().unwrap(),
                Err("variant receiver claims have a running writer")
            );
            assert!(!flight.finish_with_dependency().0);
            drop(active_guard);
        });
        prepare(capacity + 1).unwrap();
        assert!(WRITERS.get().unwrap().lock().unwrap().claims.capacity() > capacity);
    }
    #[test]
    fn prepared_claims_cover_designs_without_callback_growth() {
        if !isolated() {
            return;
        }
        let population = 5;
        prepare(population).unwrap();
        let mut flights = Vec::with_capacity(population);
        collector_test_support::with_allocations_denied(|| {
            for pointer in 1..=population {
                let flight = enter(pointer, None, Footprint::Design).unwrap();
                assert!(flight.admitted());
                flights.push(flight);
            }
            for pointer in 1..=population {
                let witness = enter(pointer, None, Footprint::Identity).unwrap();
                assert!(witness.admitted());
                assert!(!witness.finish_with_dependency().0);
            }
            for flight in flights.drain(..) {
                assert!(!flight.finish_with_dependency().0);
            }
        });
        let state = WRITERS.get().unwrap().lock().unwrap();
        assert!(state.claims.is_empty());
        assert_eq!((state.ancestry, state.lifetime), (0, 0));
        assert_eq!(state.required, population);
        assert_eq!(CLAIM_CAPACITY_REFUSALS.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn full_claim_storage_admits_nested_operands_without_growth_and_retains_overflow_demand() {
        if !isolated() {
            return;
        }
        prepare(1).unwrap();
        let capacity = WRITERS.get().unwrap().lock().unwrap().claims.capacity();
        let mut flights = Vec::with_capacity(capacity);
        collector_test_support::with_allocations_denied(|| {
            for pointer in 1..=capacity {
                flights.push(enter(pointer, None, Footprint::Design).unwrap());
            }
            let nested = enter(1, Some(capacity), Footprint::Lifetime).unwrap();
            assert!(nested.admitted());
            assert_eq!(nested.finish_with_dependency(), (false, false));
            assert!(enter(capacity + 1, None, Footprint::Design).is_none());
            for flight in flights.drain(..) {
                assert!(!flight.finish_with_dependency().0);
            }
            assert!(enter(1, None, Footprint::Design).is_none());
        });
        prepare(capacity + 1).unwrap();
        let flight = enter(1, None, Footprint::Design).unwrap();
        assert!(flight.admitted());
        assert!(!flight.finish_with_dependency().0);
    }

    #[test]
    fn claims_past_the_prepared_table_grow_in_the_callback() {
        if !isolated() {
            return;
        }
        prepare(1).unwrap();
        let capacity = WRITERS.get().unwrap().lock().unwrap().claims.capacity();
        let mut flights = Vec::with_capacity(capacity + 2);
        for pointer in 1..=capacity + 2 {
            let flight = enter(pointer, None, Footprint::Design).unwrap();
            assert!(flight.admitted());
            flights.push(flight);
        }
        assert!(WRITERS.get().unwrap().lock().unwrap().claims.capacity() >= capacity + 2);
        for flight in flights.drain(..) {
            assert!(!flight.finish_with_dependency().0);
        }
        assert!(WRITERS.get().unwrap().lock().unwrap().claims.is_empty());
    }

    #[test]
    fn an_unmatched_full_claim_refuses_without_callback_growth() {
        if !isolated() {
            return;
        }
        prepare(1).unwrap();
        let capacity = WRITERS.get().unwrap().lock().unwrap().claims.capacity();
        let mut flights = Vec::with_capacity(capacity);
        collector_test_support::with_allocations_denied(|| {
            for pointer in 1..=capacity {
                flights.push(enter(pointer, None, Footprint::Design).unwrap());
            }
            let missing = Flight {
                pointers: [Some((capacity + 1, Footprint::Design)), None],
                thread: unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() },
                admitted: true,
            };
            assert_eq!(missing.finish_with_dependency(), (true, true));
            for flight in flights.drain(..) {
                assert!(!flight.finish_with_dependency().0);
            }
        });
        assert!(MISSED.load(Ordering::Acquire));
        let state = WRITERS.get().unwrap().lock().unwrap();
        assert!(state.claims.is_empty());
        assert_eq!(state.claims.capacity(), capacity);
    }
}
