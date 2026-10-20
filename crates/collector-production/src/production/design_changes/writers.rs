//! Prepared receiver claims outlive originals and their terminal snapshots.
//! The claim mutex is never held while calling native code or taking STORE.
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
}
#[derive(Default)]
struct State {
    claims: HashMap<usize, Claim>,
    required: usize,
    ancestry: usize,
    lifetime: usize,
}
pub(super) fn prepare(population: usize) -> Result<(), &'static str> {
    let mut state = WRITERS
        .get_or_init(|| Mutex::new(State::default()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if ACTIVE.load(Ordering::SeqCst) != 0 {
        return Err("variant receiver claims have a running writer");
    }
    // A failed callback try_lock can leave a stale claim. Only a quiescent
    // complete census may discard it, never an intervening callback.
    state.claims.clear();
    state.ancestry = 0;
    state.lifetime = 0;
    let required = population.max(state.required);
    engine_allocation::reserve_map(&mut state.claims, required)
        .map_err(|_| "variant receiver claim allocation failed")
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
    if state.required > state.claims.capacity() {
        CLAIM_CAPACITY_REFUSALS.fetch_add(1, Ordering::Relaxed);
        unobserved();
        return false;
    }
    true
}
fn claim(state: &mut State, pointer: usize, thread: u32, footprint: Footprint) -> bool {
    let mut conflicted = state.claims.get_mut(&0).is_some_and(|claim| {
        claim.conflicted |= claim.thread != thread;
        claim.conflicted
    });
    // Roster/create commands have no single known receiver. Ancestry readers
    // additionally exclude foreign consuming moves/destruction: even an
    // intermediate ancestor's lifetime can change the game's version owner.
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
                conflicted = true;
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
    });
    if footprint == Footprint::Identity {
        // Ordinary edits do not change the parent's authenticated identity.
        // An existing same-thread consuming move may snapshot its own parent.
        claim.conflicted |= conflicted || (claim.lifetime != 0 && claim.thread != thread);
        claim.identities += 1;
    } else {
        if claim.writers == 0 {
            claim.thread = thread;
        }
        claim.conflicted |= conflicted
            || claim.thread != thread
            || (footprint == Footprint::Lifetime && claim.identities != 0);
        claim.writers += 1;
    }
    claim.count += 1;
    claim.ancestry += usize::from(footprint == Footprint::Ancestry);
    claim.lifetime += usize::from(footprint == Footprint::Lifetime);
    let admitted = !claim.conflicted;
    state.ancestry += usize::from(footprint == Footprint::Ancestry);
    state.lifetime += usize::from(footprint == Footprint::Lifetime);
    admitted
}
pub(super) struct Flight {
    // A mutation names one receiver and, for move/derived refresh, one other
    // consumed receiver or version owner. These are the writer's operands,
    // not a population or thread limit.
    pointers: [Option<(usize, Footprint)>; 2],
    thread: u32,
    admitted: bool,
}
pub(super) fn enter(pointer: usize, other: Option<usize>, footprint: Footprint) -> Option<Flight> {
    let Some(state) = WRITERS.get() else {
        unknown_receiver();
        return None;
    };
    let Ok(mut state) = state.try_lock() else {
        lock_contended();
        return None;
    };
    let other = other.filter(|other| *other != pointer);
    let additional = usize::from(!state.claims.contains_key(&pointer))
        + usize::from(other.is_some_and(|other| !state.claims.contains_key(&other)));
    if !room(&mut state, additional) {
        return None;
    }
    let thread = unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() };
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
        let Ok(mut state) = WRITERS.get().unwrap().try_lock() else {
            lock_contended();
            self.admitted = false;
            return;
        };
        let additional = usize::from(!state.claims.contains_key(&pointer));
        if !room(&mut state, additional) {
            self.admitted = false;
            return;
        }
        self.pointers[1] = Some((pointer, Footprint::Design));
        if !claim(&mut state, pointer, self.thread, Footprint::Design) {
            if self.admitted {
                COMPETING_WRITER_REJECTIONS.fetch_add(1, Ordering::Relaxed);
            }
            self.admitted = false;
        }
    }
    /// Conflict inspection and release are one cut. Otherwise a foreign
    /// writer could enter after inspection but before release, when a saved
    /// destruction's binding is already gone and looks like an excluded draft.
    pub(super) fn finish(mut self) -> bool {
        self.close()
    }
    fn close(&mut self) -> bool {
        if self.pointers[0].is_none() {
            return false;
        }
        let pointers = std::mem::replace(&mut self.pointers, [None; 2]);
        let Ok(mut state) = WRITERS.get().unwrap().try_lock() else {
            lock_contended();
            return true;
        };
        let conflicted = pointers.into_iter().flatten().any(|(pointer, _)| {
            state
                .claims
                .get(&pointer)
                .is_none_or(|claim| claim.conflicted)
        });
        for (pointer, footprint) in pointers.into_iter().flatten() {
            let Some(claim) = state.claims.get_mut(&pointer) else {
                unobserved();
                continue;
            };
            claim.count -= 1;
            claim.writers -= usize::from(footprint != Footprint::Identity);
            claim.identities -= usize::from(footprint == Footprint::Identity);
            claim.ancestry -= usize::from(footprint == Footprint::Ancestry);
            claim.lifetime -= usize::from(footprint == Footprint::Lifetime);
            let remove = claim.count == 0;
            state.ancestry -= usize::from(footprint == Footprint::Ancestry);
            state.lifetime -= usize::from(footprint == Footprint::Lifetime);
            if remove {
                state.claims.remove(&pointer);
            }
        }
        conflicted
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
    fn prepared_claims_cover_five_thousand_designs_without_callback_growth() {
        let population = 5_000;
        prepare(population).unwrap();
        let mut flights = Vec::with_capacity(population);
        collector_test_support::with_allocations_denied(|| {
            for pointer in 1..=population {
                let flight = enter(pointer, None, Footprint::Design).unwrap();
                assert!(flight.admitted());
                flights.push(flight);
            }
            for pointer in 1..=population {
                // Identity witnesses share a receiver with its same-thread
                // ordinary writer, without another receiver-map allocation.
                let witness = enter(pointer, None, Footprint::Identity).unwrap();
                assert!(witness.admitted());
                assert!(!witness.finish());
            }
            for flight in flights.drain(..) {
                assert!(!flight.finish());
            }
        });
        let state = WRITERS.get().unwrap().lock().unwrap();
        assert!(state.claims.is_empty());
        assert_eq!((state.ancestry, state.lifetime), (0, 0));
        assert_eq!(CLAIM_CAPACITY_REFUSALS.load(Ordering::Relaxed), 0);
    }
}
