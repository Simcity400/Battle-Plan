//! Each country's RaidIntelInputs modifier answers (its accumulator's
//! `CRYPTO_STRENGTH` and `INTELLIGENCE_AGENCY_DEFENSE` raw values), held
//! between the Projects job's hourly walks ([`engine::raid_intel_inputs::Carry`]).
//! The walk still reads each country's accumulator header; a country whose
//! header is the held one, whose CCountry and id are the held ones and whose
//! certificate is the one taken before the held read skips its entries copy
//! and answers from the held values. The certificate is the naval tree
//! family's ([`crate::naval_tree_changes::sequence`]): its country sites,
//! country modifier recalculation and country appearance reset,
//! are the native writers of `CCountry+0x5B8`. A read is held only when the
//! certificate stood still across it; a walk that does not complete forgets
//! the store.
//!
//! Each country is read afresh and compared with its held answers on its
//! hour of the sampled rotation (by id), and `raid_intel_verify`, on by
//! default during the family's burn-in, compares every carryable country
//! every hour. A compared read across which the certificate stood still and
//! that differs is a write no hook saw: it disables every naval
//! tree certificate for the attachment ([`crate::naval_tree_changes::missed_write`]),
//! and every country is read from then on.
use crate::sampled_verify::Check;
use engine::{
    ccountry::Country,
    raid_intel_inputs::{Carry, RawAnswer},
};
use std::sync::{
    Mutex, MutexGuard, TryLockError,
    atomic::{AtomicU64, Ordering::Relaxed},
};

type Certificate = (u64, u64);

/// Carried, read, compared and mismatched countries; walks that did not
/// complete.
static COUNTS: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
static SAMPLED: crate::sampled_verify::Sampled = crate::sampled_verify::Sampled::new(&[
    "a carried country's raid intel modifier answers differ from a fresh read",
]);
static STORE: Mutex<Store> = Mutex::new(Store { held: Vec::new() });

/// Compare every carryable country every hour (burn-in).
const VERIFY: bool = cfg!(feature = "raid_intel_verify");

/// The certificates as a walk consults them; tests substitute their own.
pub trait Certificates {
    /// A CCountry's certificate, registering it; `None` when it cannot be
    /// observed.
    fn certificate(&self, country: usize) -> Option<Certificate>;
    /// A held country differed from a fresh read.
    fn mismatch(&self);
}

/// The naval tree family's country certificates.
pub struct Observer;
impl Certificates for Observer {
    fn certificate(&self, country: usize) -> Option<Certificate> {
        crate::naval_tree_changes::sequence(country)
    }
    fn mismatch(&self) {
        crate::naval_tree_changes::missed_write();
    }
}

/// One country's latest complete read.
#[derive(Clone, Copy)]
struct Held {
    id: u32,
    pointer: usize,
    header: [u8; 16],
    certificate: Certificate,
    answers: [RawAnswer; 2],
}

/// Held countries by canonical slot.
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

/// The country a walk asked about last: its certificate before its entries
/// copy, and the held answers that copy is compared with.
struct Pending {
    pointer: usize,
    certificate: Option<Certificate>,
    compare: Option<[RawAnswer; 2]>,
}

/// One walk's use of the store.
pub struct Walk<'a, C: Certificates> {
    store: MutexGuard<'a, Store>,
    certificates: &'a C,
    check: Check,
    pending: Option<Pending>,
    /// Countries asked about (watch receivers), carried, read, compared and
    /// mismatched.
    receivers: u64,
    carried: u64,
    read: u64,
    compared: u64,
    mismatches: u64,
}

/// The production walk, when `raid_intel_changes` is configured and no other
/// walk holds the store.
pub fn begin() -> Option<Walk<'static, Observer>> {
    if !cfg!(feature = "raid_intel_changes") {
        return None;
    }
    Walk::begin_in(&STORE, &Observer, crate::sampled_verify::current(VERIFY))
}

impl<'a, C: Certificates> Walk<'a, C> {
    pub fn begin_in(store: &'a Mutex<Store>, certificates: &'a C, check: Check) -> Option<Self> {
        let store = match store.try_lock() {
            Ok(store) => store,
            // A walk that panicked left it: hold nothing it held.
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
            check,
            pending: None,
            receivers: 0,
            carried: 0,
            read: 0,
            compared: 0,
            mismatches: 0,
        })
    }

    /// The walk ended; `complete` when it produced its rows. One that did not
    /// forgets every held country.
    pub fn finish(mut self, complete: bool) {
        if !complete {
            self.store.held.clear();
            COUNTS[4].fetch_add(1, Relaxed);
        }
        for (target, value) in
            COUNTS
                .iter()
                .zip([self.carried, self.read, self.compared, self.mismatches])
        {
            target.fetch_add(value, Relaxed);
        }
        SAMPLED.record(self.compared, self.mismatches, 0);
        crate::naval_tree_changes::raid_intel(self.receivers as usize);
    }

    fn hold(&mut self, country: &Country, held: Option<Held>) {
        let slot = country.canonical_slot as usize;
        let stored = &mut self.store.held;
        if slot >= stored.len() {
            let additional = slot + 1 - stored.len();
            if held.is_none() || engine::allocation::reserve(stored, additional).is_err() {
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
        let answers = carryable.map(|held| held.answers);
        if answers.is_some() && !self.check.covers(u64::from(country.id)) {
            self.carried += 1;
            self.pending = None;
            return answers;
        }
        self.pending = Some(Pending {
            pointer: country.pointer,
            certificate,
            compare: answers,
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
        // The certificate stood still across the copy: no observed writer
        // ran, so a held answer that differs is a write no hook saw. A read
        // across an observed write (or while observation failed) is neither
        // compared nor held.
        let steady = pending.certificate.filter(|&certificate| {
            self.certificates.certificate(country.pointer) == Some(certificate)
        });
        if let (Some(_), Some(held)) = (steady, pending.compare) {
            self.compared += 1;
            if held != answers {
                self.mismatches += 1;
                self.certificates.mismatch();
            }
        }
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

pub fn status() -> impl serde::Serialize {
    let counts = COUNTS.each_ref().map(|count| count.load(Relaxed));
    crate::status::status!({
        "configured": cfg!(feature = "raid_intel_changes"),
        "verify": VERIFY,
        "carried_countries": counts[0],
        "read_countries": counts[1],
        "compared_countries": counts[2],
        "mismatches": counts[3],
        "failed_walks": counts[4],
        "sampled_verified": SAMPLED.verified(),
        "sampled_mismatches": SAMPLED.mismatches(),
        "last_mismatch": SAMPLED.last_mismatch(),
        "slice": SAMPLED.slice(),
        "scope": "cumulative since attach; RaidIntelInputs reads each country's modifier accumulator header every hour and skips its entries copy, answering CRYPTO_STRENGTH and INTELLIGENCE_AGENCY_DEFENSE from the held raw values, while the header, CCountry, id and naval tree country certificate (modifier recalculation, country appearance reset) are the held read's; effective values always use the current modifier definitions. Each country is read and compared with its held answers on its hour of the day's rotation (every hour in verify mode); the first mismatch disables every naval tree certificate",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// Sequences a test bumps; `mismatch` poisons them all.
    #[derive(Default)]
    struct Sequences {
        sequence: Cell<u64>,
        poisoned: Cell<bool>,
    }
    impl Certificates for Sequences {
        fn certificate(&self, _: usize) -> Option<Certificate> {
            (!self.poisoned.get()).then(|| (1, self.sequence.get()))
        }
        fn mismatch(&self) {
            self.poisoned.set(true);
        }
    }

    const COUNTRY: Country = Country {
        pointer: 0x7000,
        canonical_slot: 3,
        id: 7,
    };
    const HEADER: [u8; 16] = [1; 16];

    /// One walk over `COUNTRY` as the engine drives the carry: the held
    /// answers, or a copy whose answers are `fresh` (`write` bumps the
    /// certificate during the copy).
    fn hour(
        store: &Mutex<Store>,
        sequences: &Sequences,
        check: Check,
        header: [u8; 16],
        fresh: [RawAnswer; 2],
        write: bool,
    ) -> Option<[RawAnswer; 2]> {
        let mut walk = Walk::begin_in(store, sequences, check).unwrap();
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

    /// A country no writer touched answers from its held read; a certificate
    /// bump, a different header and a write across the read each read
    /// afresh; the country's hour of the rotation compares a read with the
    /// held answers, and a difference poisons the certificates.
    #[test]
    fn a_country_is_carried_until_its_accumulator_may_have_changed() {
        let store = Mutex::new(Store::new());
        let sequences = Sequences::default();
        let quiet = Check::Slice { hour: 1 };
        let a = [Some(250), None];
        let b = [Some(9), Some(-4)];
        assert_eq!(
            hour(&store, &sequences, quiet, HEADER, a, false),
            None,
            "first read"
        );
        assert_eq!(hour(&store, &sequences, quiet, HEADER, b, false), Some(a));
        // Modifier recalculation ran: read afresh, then carried.
        sequences.sequence.set(sequences.sequence.get() + 1);
        assert_eq!(hour(&store, &sequences, quiet, HEADER, b, false), None);
        assert_eq!(hour(&store, &sequences, quiet, HEADER, a, false), Some(b));
        // Another header (array moved or resized): read afresh.
        let moved = [2; 16];
        assert_eq!(hour(&store, &sequences, quiet, moved, a, false), None);
        assert_eq!(hour(&store, &sequences, quiet, moved, b, false), Some(a));
        // A writer ran during the read: not held.
        sequences.sequence.set(sequences.sequence.get() + 1);
        assert_eq!(hour(&store, &sequences, quiet, HEADER, b, true), None);
        assert_eq!(hour(&store, &sequences, quiet, HEADER, b, false), None);
        assert_eq!(hour(&store, &sequences, quiet, HEADER, a, false), Some(b));
        // Another country in the slot: read afresh.
        let other = Country { id: 8, ..COUNTRY };
        let mut walk = Walk::begin_in(&store, &sequences, quiet).unwrap();
        assert_eq!(walk.held(&other, &HEADER), None);
        walk.finish(true);
        // Country 7's hour of the rotation: read and compared.
        let day = Check::Slice { hour: 7 };
        assert_eq!(hour(&store, &sequences, day, HEADER, b, false), None);
        assert!(!sequences.poisoned.get());
        assert_eq!(hour(&store, &sequences, day, HEADER, a, false), None);
        assert!(sequences.poisoned.get());
        assert_eq!(
            hour(&store, &sequences, quiet, HEADER, a, false),
            None,
            "poisoned"
        );
    }

    /// A compared read across an observed write is neither compared nor
    /// held: the hook saw the write, so a different answer is no missed
    /// write.
    #[test]
    fn a_compared_read_across_an_observed_write_does_not_poison() {
        let store = Mutex::new(Store::new());
        let sequences = Sequences::default();
        let a = [Some(250), None];
        let b = [Some(9), Some(-4)];
        for every in [Check::Every, Check::Slice { hour: 7 }] {
            assert_eq!(hour(&store, &sequences, every, HEADER, a, false), None);
            // Carryable but compared; modifier recalculation runs during the copy.
            assert_eq!(hour(&store, &sequences, every, HEADER, b, true), None);
            assert!(!sequences.poisoned.get());
            // Not held: the next hour reads afresh rather than comparing.
            assert_eq!(
                hour(
                    &store,
                    &sequences,
                    Check::Slice { hour: 1 },
                    HEADER,
                    b,
                    false
                ),
                None
            );
            assert!(!sequences.poisoned.get());
            sequences.sequence.set(sequences.sequence.get() + 1);
        }
    }

    /// 5,000 countries, each in its own canonical slot under its own
    /// certificate: every one carries its own answers, only the countries
    /// whose certificate or header changed are read, and a sampled hour reads
    /// and compares exactly its slice.
    #[test]
    fn five_thousand_countries_carry_and_invalidate_independently() {
        const COUNTRIES: u32 = 5_000;
        const BASE: usize = 0x1000_0000;
        const SIZE: usize = 0x1400;
        struct PerCountry(std::cell::RefCell<Vec<u64>>);
        impl Certificates for PerCountry {
            fn certificate(&self, country: usize) -> Option<Certificate> {
                Some((1, self.0.borrow()[(country - BASE) / SIZE]))
            }
            fn mismatch(&self) {
                panic!("no country differs");
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
        // One walk over every country: its carried, read and compared counts.
        let pass = |check: Check, moved: &dyn Fn(u32) -> bool| {
            let mut walk = Walk::begin_in(&store, &certificates, check).unwrap();
            for slot in 1..=COUNTRIES {
                let (country, header) = (country_at(slot), header_at(slot, moved(slot)));
                match walk.held(&country, &header) {
                    Some(held) => assert_eq!(held, answers_at(slot), "slot {slot}"),
                    None => walk.read(&country, &header, answers_at(slot)),
                }
            }
            let counts = (walk.carried, walk.read, walk.compared);
            walk.finish(true);
            counts
        };
        let all = u64::from(COUNTRIES);
        let eleventh = |slot: u32| slot.is_multiple_of(11);
        assert_eq!(pass(Check::Nothing, &|_: u32| false), (0, all, 0));
        assert_eq!(pass(Check::Nothing, &|_: u32| false), (all, 0, 0));
        // A writer touched every seventh country; every eleventh one's array
        // moved.
        for slot in (7..=COUNTRIES).step_by(7) {
            certificates.0.borrow_mut()[slot as usize] += 1;
        }
        let changed = (1..=COUNTRIES)
            .filter(|slot| slot % 7 == 0 || eleventh(*slot))
            .count() as u64;
        assert_eq!(pass(Check::Nothing, &eleventh), (all - changed, changed, 0));
        assert_eq!(pass(Check::Nothing, &eleventh), (all, 0, 0));
        // Hour 5 of the rotation compares the countries whose id is 5 mod 24.
        let slice = (1..=COUNTRIES)
            .filter(|slot| (slot + 100) % 24 == 5)
            .count() as u64;
        assert_eq!(
            pass(Check::Slice { hour: 5 }, &eleventh),
            (all - slice, slice, slice)
        );
    }

    /// A walk that does not complete forgets every held country.
    #[test]
    fn an_incomplete_walk_forgets_the_store() {
        let store = Mutex::new(Store::new());
        let sequences = Sequences::default();
        let quiet = Check::Slice { hour: 1 };
        let a = [Some(250), None];
        assert_eq!(hour(&store, &sequences, quiet, HEADER, a, false), None);
        let mut walk = Walk::begin_in(&store, &sequences, quiet).unwrap();
        assert_eq!(walk.held(&COUNTRY, &HEADER), Some(a));
        walk.finish(false);
        assert_eq!(hour(&store, &sequences, quiet, HEADER, a, false), None);
    }
}
