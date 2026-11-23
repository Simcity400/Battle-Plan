use engine_native_memory::country_identity::Country;
use engine_raid_inputs::{raid_intel_inputs::Carry, raid_intel_inputs::RawAnswer};
use std::sync::{
    Mutex, MutexGuard, TryLockError,
    atomic::{AtomicU64, Ordering::Relaxed},
};

type Certificate = (u64, u64);

static COUNTS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
static STORE: Mutex<Store> = Mutex::new(Store { held: Vec::new() });

pub trait Certificates {
    fn certificate(&self, country: usize) -> Option<Certificate>;
}

pub struct Observer;
impl Certificates for Observer {
    fn certificate(&self, country: usize) -> Option<Certificate> {
        crate::naval_tree_changes::sequence(country)
    }
}

#[derive(Clone, Copy)]
struct Held {
    id: u32,
    pointer: usize,
    header: [u8; 16],
    certificate: Certificate,
    answers: [RawAnswer; 2],
}

pub struct Store {
    held: Vec<Option<Held>>,
}

#[cfg(any(test, feature = "test-support"))]
impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

impl Store {
    #[cfg(any(test, feature = "test-support"))]
    pub const fn new() -> Self {
        Self { held: Vec::new() }
    }

    fn get(&self, country: &Country) -> Option<&Held> {
        self.held
            .get(country.canonical_slot as usize)
            .and_then(Option::as_ref)
    }
}

struct Pending {
    pointer: usize,
    certificate: Option<Certificate>,
}

pub struct Walk<'a, C: Certificates> {
    store: MutexGuard<'a, Store>,
    certificates: &'a C,
    pending: Option<Pending>,
    receivers: u64,
    carried: u64,
    read: u64,
}

pub fn begin() -> Option<Walk<'static, Observer>> {
    if !cfg!(feature = "raid_intel_changes") {
        return None;
    }
    Walk::begin_in(&STORE, &Observer)
}

impl<'a, C: Certificates> Walk<'a, C> {
    pub fn begin_in(store: &'a Mutex<Store>, certificates: &'a C) -> Option<Self> {
        let store = match store.try_lock() {
            Ok(store) => store,
            Err(TryLockError::Poisoned(poisoned)) => {
                let mut store = poisoned.into_inner();
                store.held.clear();
                store
            }
            Err(TryLockError::WouldBlock) => return None,
        };
        Some(Self {
            store,
            certificates,
            pending: None,
            receivers: 0,
            carried: 0,
            read: 0,
        })
    }

    pub fn finish(mut self, complete: bool) {
        if !complete {
            self.store.held.clear();
            COUNTS[2].fetch_add(1, Relaxed);
        }
        for (target, value) in COUNTS.iter().zip([self.carried, self.read]) {
            target.fetch_add(value, Relaxed);
        }
        crate::naval_tree_changes::raid_intel(self.receivers as usize);
    }

    fn hold(&mut self, country: &Country, held: Option<Held>) {
        let slot = country.canonical_slot as usize;
        let stored = &mut self.store.held;
        if slot >= stored.len() {
            let additional = slot + 1 - stored.len();
            if held.is_none() || engine_allocation::reserve(stored, additional).is_err() {
                return;
            }
            stored.resize(slot + 1, None);
        }
        stored[slot] = held;
    }
}

impl<C: Certificates> Carry for Walk<'_, C> {
    fn held(&mut self, country: &Country, header: &[u8; 16]) -> Option<[RawAnswer; 2]> {
        self.receivers += 1;
        let certificate = self.certificates.certificate(country.pointer);
        let carryable = self.store.get(country).filter(|held| {
            certificate == Some(held.certificate)
                && held.pointer == country.pointer
                && held.id == country.id
                && held.header == *header
        });
        if let Some(held) = carryable {
            self.carried += 1;
            self.pending = None;
            return Some(held.answers);
        }
        self.pending = Some(Pending {
            pointer: country.pointer,
            certificate,
        });
        None
    }

    fn read(&mut self, country: &Country, header: &[u8; 16], answers: [RawAnswer; 2]) {
        let Some(pending) = self
            .pending
            .take()
            .filter(|pending| pending.pointer == country.pointer)
        else {
            return;
        };
        self.read += 1;
        let steady = pending.certificate.filter(|&certificate| {
            self.certificates.certificate(country.pointer) == Some(certificate)
        });
        let held = steady.map(|certificate| Held {
            id: country.id,
            pointer: country.pointer,
            header: *header,
            certificate,
            answers,
        });
        self.hold(country, held);
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[derive(Default)]
    struct Sequences {
        sequence: Cell<u64>,
    }
    impl Certificates for Sequences {
        fn certificate(&self, _: usize) -> Option<Certificate> {
            Some((1, self.sequence.get()))
        }
    }

    const COUNTRY: Country = Country {
        pointer: 0x7000,
        canonical_slot: 3,
        id: 7,
    };
    const HEADER: [u8; 16] = [1; 16];

    fn hour(
        store: &Mutex<Store>,
        sequences: &Sequences,
        header: [u8; 16],
        fresh: [RawAnswer; 2],
        write: bool,
    ) -> Option<[RawAnswer; 2]> {
        let mut walk = Walk::begin_in(store, sequences).unwrap();
        let held = walk.held(&COUNTRY, &header);
        if held.is_none() {
            if write {
                sequences.sequence.set(sequences.sequence.get() + 1);
            }
            walk.read(&COUNTRY, &header, fresh);
        }
        walk.finish(true);
        held
    }

    #[test]
    fn a_country_is_carried_until_its_accumulator_may_have_changed() {
        let store = Mutex::new(Store::new());
        let sequences = Sequences::default();
        let a = [Some(250), None];
        let b = [Some(9), Some(-4)];
        assert_eq!(
            hour(&store, &sequences, HEADER, a, false),
            None,
            "first read"
        );
        assert_eq!(hour(&store, &sequences, HEADER, b, false), Some(a));
        sequences.sequence.set(sequences.sequence.get() + 1);
        assert_eq!(hour(&store, &sequences, HEADER, b, false), None);
        assert_eq!(hour(&store, &sequences, HEADER, a, false), Some(b));
        let moved = [2; 16];
        assert_eq!(hour(&store, &sequences, moved, a, false), None);
        assert_eq!(hour(&store, &sequences, moved, b, false), Some(a));
        sequences.sequence.set(sequences.sequence.get() + 1);
        assert_eq!(hour(&store, &sequences, HEADER, b, true), None);
        assert_eq!(hour(&store, &sequences, HEADER, b, false), None);
        assert_eq!(hour(&store, &sequences, HEADER, a, false), Some(b));
        let other = Country { id: 8, ..COUNTRY };
        let mut walk = Walk::begin_in(&store, &sequences).unwrap();
        assert_eq!(walk.held(&other, &HEADER), None);
        walk.finish(true);
    }

    #[test]
    fn countries_carry_and_invalidate_independently() {
        const COUNTRIES: u32 = 40;
        const BASE: usize = 0x1000_0000;
        const SIZE: usize = 0x1400;
        struct PerCountry(std::cell::RefCell<Vec<u64>>);
        impl Certificates for PerCountry {
            fn certificate(&self, country: usize) -> Option<Certificate> {
                Some((1, self.0.borrow()[(country - BASE) / SIZE]))
            }
        }
        let country_at = |slot: u32| Country {
            pointer: BASE + slot as usize * SIZE,
            canonical_slot: slot,
            id: slot + 100,
        };
        let answers_at = |slot: u32| {
            [
                Some(i64::from(slot)),
                slot.is_multiple_of(3).then(|| -i64::from(slot)),
            ]
        };
        let header_at = |slot: u32, moved: bool| {
            let mut header = [0_u8; 16];
            header[..4].copy_from_slice(&slot.to_le_bytes());
            header[4] = u8::from(moved);
            header
        };
        let store = Mutex::new(Store::new());
        let certificates = PerCountry(std::cell::RefCell::new(vec![0; COUNTRIES as usize + 1]));
        let pass = |moved: &dyn Fn(u32) -> bool| {
            let mut walk = Walk::begin_in(&store, &certificates).unwrap();
            for slot in 1..=COUNTRIES {
                let (country, header) = (country_at(slot), header_at(slot, moved(slot)));
                match walk.held(&country, &header) {
                    Some(held) => assert_eq!(held, answers_at(slot), "slot {slot}"),
                    None => walk.read(&country, &header, answers_at(slot)),
                }
            }
            let counts = (walk.carried, walk.read);
            walk.finish(true);
            counts
        };
        let all = u64::from(COUNTRIES);
        let eleventh = |slot: u32| slot.is_multiple_of(11);
        assert_eq!(pass(&|_: u32| false), (0, all));
        assert_eq!(pass(&|_: u32| false), (all, 0));
        for slot in (7..=COUNTRIES).step_by(7) {
            certificates.0.borrow_mut()[slot as usize] += 1;
        }
        let changed = (1..=COUNTRIES)
            .filter(|slot| slot % 7 == 0 || eleventh(*slot))
            .count() as u64;
        assert_eq!(pass(&eleventh), (all - changed, changed));
        assert_eq!(pass(&eleventh), (all, 0));
    }

    #[test]
    fn an_incomplete_walk_forgets_the_store() {
        let store = Mutex::new(Store::new());
        let sequences = Sequences::default();
        let a = [Some(250), None];
        assert_eq!(hour(&store, &sequences, HEADER, a, false), None);
        let mut walk = Walk::begin_in(&store, &sequences).unwrap();
        assert_eq!(walk.held(&COUNTRY, &HEADER), Some(a));
        walk.finish(false);
        assert_eq!(hour(&store, &sequences, HEADER, a, false), None);
    }
}
