use super::*;
use crate::cgame_state;
use engine_native_memory::memory::SliceMemory;
use std::{cell::RefCell, collections::BTreeMap};

const GS: usize = 0x1000;
const COUNTRIES: usize = 0x2000;
const LINKS: usize = 0x3000;
const OBJECT: usize = 0x5000;
const RESERVED: Country = Country {
    pointer: 0x4000,
    canonical_slot: 0,
    id: 0,
};

struct Source {
    bytes: Vec<u8>,
    copies: RefCell<BTreeMap<usize, usize>>,
    reserved_calls: Cell<usize>,
    reserved: Option<CountryResult>,
    fail_once: Option<usize>,
}

impl Source {
    fn new() -> Self {
        Self {
            bytes: vec![0; 0x8000],
            copies: RefCell::new(BTreeMap::new()),
            reserved_calls: Cell::new(0),
            reserved: Some(Ok(RESERVED)),
            fail_once: None,
        }
    }
    fn put(&mut self, at: usize, bytes: &[u8]) {
        self.bytes[at..at + bytes.len()].copy_from_slice(bytes);
    }
    fn u32(&mut self, at: usize, word: u32) {
        self.put(at, &word.to_le_bytes());
    }
    fn u64(&mut self, at: usize, word: u64) {
        self.put(at, &word.to_le_bytes());
    }
    fn array(&mut self, at: usize, data: usize, count: u32) {
        self.u64(at, data as u64);
        self.u32(at + 8, count);
        self.u32(at + 12, count);
    }
    fn positive() -> Self {
        let mut source = Self::new();
        source.array(GS + cgame_state::COUNTRIES, COUNTRIES, 2);
        source.array(GS + cgame_state::COUNTRY_LINKS, LINKS, 9);
        source.u64(COUNTRIES + 8, OBJECT as u64);
        source.u64(OBJECT, super::super::VTABLE_RVA.0 as u64);
        source.u32(OBJECT + super::super::ID, 7);
        for raw in [1, 7, 8] {
            source.u32(LINKS + raw * 4, 1);
        }
        source
    }
    fn attempts(&self, at: usize) -> usize {
        self.copies.borrow().get(&at).copied().unwrap_or(0)
    }
}

impl Memory for Source {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let count = {
            let mut copies = self.copies.borrow_mut();
            let count = copies.entry(address).or_default();
            *count += 1;
            *count
        };
        if self.fail_once == Some(address) && count == 1 {
            return false;
        }
        SliceMemory {
            base: 0,
            bytes: &self.bytes,
        }
        .copy(address, destination)
    }
    fn authenticated_reserved_country(&self, base: usize, state: usize) -> Option<CountryResult> {
        self.reserved_calls.set(self.reserved_calls.get() + 1);
        ((base, state) == (0, GS))
            .then_some(self.reserved)
            .flatten()
    }
}

fn memo(raw: usize, links: usize) -> NativeReferenceMemo {
    let mut memo = NativeReferenceMemo::try_with_capacity(raw, links).unwrap();
    memo.reset(0, GS);
    memo
}

fn population(countries: usize, aliases: usize) -> (Source, usize, usize, usize) {
    let country_words = 0x1_0000;
    let link_words = country_words + (countries + 1) * 8 + 0x1000;
    let objects = link_words + (countries * aliases + 1) * 4 + 0x1000;
    let mut source = Source::new();
    source.bytes.resize(objects + countries * 0x20, 0);
    source.array(
        GS + cgame_state::COUNTRIES,
        country_words,
        (countries + 1) as u32,
    );
    source.array(
        GS + cgame_state::COUNTRY_LINKS,
        link_words,
        (countries * aliases + 1) as u32,
    );
    for slot in 1..=countries {
        let pointer = objects + (slot - 1) * 0x20;
        source.u64(country_words + slot * 8, pointer as u64);
        source.u64(pointer, super::super::VTABLE_RVA.0 as u64);
        source.u32(pointer + super::super::ID, slot as u32);
        for alias in 0..aliases {
            source.u32(link_words + (alias * countries + slot) * 4, slot as u32);
        }
    }
    (source, country_words, link_words, objects)
}

#[test]
fn growing_worker_cold_and_later_alias_populations_keep_the_original_headers() {
    let mut memo = memo(0, 0);
    for aliases in [1, 2] {
        let (source, _, link_words, objects) = population(50, aliases);
        memo.reset(0, GS);
        let context = memo.view(&source, 0, GS);
        for raw in 1..=50 * aliases {
            let slot = (raw - 1) % 50 + 1;
            assert_eq!(
                context.resolve(raw as u32),
                Ok(Country {
                    pointer: objects + (slot - 1) * 0x20,
                    canonical_slot: slot as u32,
                    id: slot as u32,
                })
            );
        }
        assert_eq!(memo.raw_used(), 50 * aliases);
        assert_eq!(memo.raw_required(), 50 * aliases);
        assert!(!memo.raw_required_is_lower_bound());
        assert_eq!(memo.canonical_used(), 50);
        assert_eq!(memo.link_used(), 50 * aliases);
        assert_eq!(memo.link_required(), 50 * aliases + 1);
        assert!(memo.link_capacity() >= memo.link_required());
        assert_eq!(source.attempts(GS + cgame_state::COUNTRIES), 1);
        assert_eq!(source.attempts(GS + cgame_state::COUNTRY_LINKS), 1);
        assert_eq!(source.attempts(link_words + 4), 1);
        assert_eq!(source.attempts(objects), 1);
        let observations = source.copies.borrow().clone();
        assert!(context.resolve(1).is_ok());
        assert!(context.resolve((50 * aliases) as u32).is_ok());
        assert_eq!(*source.copies.borrow(), observations);
    }
}

#[test]
fn worker_rehash_retains_first_failed_and_absent_canonical_results() {
    let (mut source, country_words, link_words, objects) = population(50, 2);
    source.fail_once = Some(objects);
    source.u64(country_words + 2 * 8, 0);
    let memo = memo(0, 0);
    let failed = memo.view(&source, 0, GS).resolve(1);
    let absent = memo.view(&source, 0, GS).resolve(2);
    assert!(failed.is_err());
    assert_eq!(absent, Err("country link resolves to null country"));
    source.u64(country_words + 2 * 8, (objects + 0x20) as u64);
    let context = memo.view(&source, 0, GS);
    for raw in 3..=50 {
        assert!(context.resolve(raw).is_ok());
    }
    assert_eq!(context.resolve(51), failed);
    assert_eq!(context.resolve(52), absent);
    for raw in 53..=100 {
        assert!(context.resolve(raw).is_ok());
    }
    assert_eq!(context.resolve(1), failed);
    assert_eq!(context.resolve(2), absent);
    assert_eq!(memo.raw_used(), 100);
    assert_eq!(memo.canonical_used(), 50);
    assert_eq!(source.attempts(objects), 1);
    assert_eq!(source.attempts(country_words + 2 * 8), 1);
    assert_eq!(source.attempts(link_words + 4), 1);
    assert_eq!(source.attempts(GS + cgame_state::COUNTRIES), 1);
    assert_eq!(source.attempts(GS + cgame_state::COUNTRY_LINKS), 1);
}

#[test]
fn growth_retains_header_fault_and_reserved_zero_law() {
    let mut source = Source::positive();
    source.fail_once = Some(GS + cgame_state::COUNTRY_LINKS);
    let memo = memo(0, 0);
    let context = memo.view(&source, 0, GS);
    assert_eq!(context.resolve(0), Ok(RESERVED));
    assert!(source.copies.borrow().is_empty());
    for raw in 1..=50 {
        assert_eq!(
            context.resolve(raw),
            Err("country links unreadable or invalid")
        );
    }
    assert_eq!(context.resolve(0), Ok(RESERVED));
    assert_eq!(source.reserved_calls.get(), 1);
    assert_eq!(source.attempts(GS + cgame_state::COUNTRIES), 1);
    assert_eq!(source.attempts(GS + cgame_state::COUNTRY_LINKS), 1);
    assert_eq!(memo.raw_used(), 50);
    assert_eq!(memo.canonical_used(), 0);
}

#[test]
fn raw_zero_is_independent_of_positive_headers_and_storage() {
    let source = Source::new();
    let memo = memo(0, 0);
    let context = memo.view(&source, 0, GS);
    assert_eq!(context.canonical_slot(0), Ok(0));
    assert_eq!(source.reserved_calls.get(), 0);
    assert_eq!(context.resolve(0), Ok(RESERVED));
    assert_eq!(context.resolve(0), Ok(RESERVED));
    assert_eq!(source.reserved_calls.get(), 1);
    assert!(source.copies.borrow().is_empty());
    assert_eq!(memo.raw_used(), 0);
    assert_eq!(memo.link_used(), 0);
}

#[test]
fn positive_header_failure_is_terminal_but_does_not_poison_zero() {
    let mut source = Source::positive();
    source.fail_once = Some(GS + cgame_state::COUNTRY_LINKS);
    let memo = memo(4, 9);
    let context = memo.view(&source, 0, GS);
    assert_eq!(
        context.resolve(7),
        Err("country links unreadable or invalid")
    );
    assert_eq!(
        context.resolve(8),
        Err("country links unreadable or invalid")
    );
    assert_eq!(source.attempts(GS + cgame_state::COUNTRY_LINKS), 1);
    assert_eq!(context.resolve(0), Ok(RESERVED));
    assert_eq!(source.attempts(RESERVED.pointer), 0);
}

#[test]
fn readable_alias_zero_and_link_failures_remain_distinct() {
    let mut source = Source::positive();
    source.u32(LINKS + 7 * 4, 0);
    source.fail_once = Some(LINKS + 8 * 4);
    let memo = memo(4, 9);
    let context = memo.view(&source, 0, GS);
    assert_eq!(context.resolve(7), Ok(RESERVED));
    assert_eq!(context.canonical_slot(7), Ok(0));
    let failed = context.resolve(8);
    assert_eq!(
        failed,
        Err("country link unreadable or outside country table")
    );
    assert_eq!(context.resolve(8), failed);
    assert_eq!(
        context.canonical_slot(8),
        failed.map(|country| country.canonical_slot)
    );
    assert_eq!(source.attempts(LINKS + 7 * 4), 1);
    assert_eq!(source.attempts(LINKS + 8 * 4), 1);
    assert_eq!(context.resolve(0), Ok(RESERVED));
    assert!(context.resolve(9).is_err());
    assert_eq!(source.attempts(LINKS + 9 * 4), 0);
}

#[test]
fn at_canonical_checks_reuse_first_link_and_native_id() {
    let source = Source::positive();
    let memo = memo(4, 9);
    let context = memo.view(&source, 0, GS);
    let expected = Country {
        pointer: OBJECT,
        canonical_slot: 1,
        id: 7,
    };
    assert_eq!(context.resolve(7), Ok(expected));
    assert_eq!(context.resolve(8), Ok(expected));
    assert_eq!(context.resolve(1), Ok(expected));
    assert_eq!(source.attempts(LINKS + 7 * 4), 1);
    assert_eq!(source.attempts(LINKS + 4), 1);
    assert_eq!(source.attempts(LINKS + 8 * 4), 1);
    assert_eq!(source.attempts(OBJECT), 1);
    assert_eq!(memo.canonical_used(), 1);
}

#[test]
fn distinct_aliases_cannot_retry_a_failed_or_absent_canonical_object() {
    for absent in [false, true] {
        let mut source = Source::positive();
        if absent {
            source.u64(COUNTRIES + 8, 0);
        } else {
            source.fail_once = Some(OBJECT);
        }
        let memo = memo(4, 9);
        let context = memo.view(&source, 0, GS);
        let failure = context.resolve(7);
        assert!(failure.is_err());
        assert_eq!(context.resolve(8), failure);
        assert_eq!(source.attempts(COUNTRIES + 8), 1);
        assert_eq!(source.attempts(OBJECT), usize::from(!absent));
        assert_eq!(memo.canonical_used(), 1);
    }
}

#[test]
fn shared_reserved_fault_or_missing_source_never_falls_back() {
    for supplied in [None, Some(Err("initial reserved fault"))] {
        let mut source = Source::positive();
        source.reserved = supplied;
        source.u32(LINKS + 7 * 4, 0);
        let memo = memo(4, 9);
        let context = memo.view(&source, 0, GS);
        let expected =
            supplied.unwrap_or(Err("native country reference lacks shared reserved source"));
        assert_eq!(context.resolve(0), expected);
        assert_eq!(context.resolve(7), expected);
        assert_eq!(source.reserved_calls.get(), 1);
        assert_eq!(source.attempts(COUNTRIES), 0);
        assert_eq!(source.attempts(RESERVED.pointer), 0);
    }
}

#[test]
fn short_storage_grows_in_the_pass_and_wrong_context_does_not_read_or_invent_zero() {
    let source = Source::positive();
    let mut raw_short = memo(0, 9);
    let expected = raw_short.view(&source, 0, GS).resolve(7);
    assert!(expected.is_ok());
    assert_eq!(raw_short.raw_required(), 1);
    assert!(!raw_short.raw_required_is_lower_bound());
    assert_eq!(raw_short.view(&source, 0, GS).resolve(0), Ok(RESERVED));
    assert!(raw_short.view(&source, 0, GS + 1).resolve(0).is_err());
    let link_short = memo(4, 2);
    assert_eq!(link_short.view(&source, 0, GS).resolve(7), expected);
    assert_eq!(link_short.link_required(), 9);
    assert!(link_short.link_capacity() >= 9);
    assert_eq!(link_short.view(&source, 0, GS).resolve(0), Ok(RESERVED));
    raw_short.reserve(4, 9).unwrap();
    assert!(raw_short.view(&source, 0, GS).resolve(0).is_err());
    raw_short.reset(0, GS);
    assert!(raw_short.view(&source, 0, GS).resolve(7).is_ok());
}

#[test]
fn changing_workers_reuses_owned_context_and_failure_results_until_reset() {
    let mut source = Source::positive();
    source.fail_once = Some(LINKS + 7 * 4);
    let mut memo = memo(4, 9);
    let failure = memo.view(&source, 0, GS).resolve(7);
    assert!(failure.is_err());
    assert_eq!(memo.view(&source, 0, GS).resolve(7), failure);
    assert_eq!(source.attempts(GS + cgame_state::COUNTRIES), 1);
    assert_eq!(source.attempts(LINKS + 7 * 4), 1);
    memo.reset(0, GS);
    assert!(memo.view(&source, 0, GS).resolve(7).is_ok());
    assert_eq!(source.attempts(GS + cgame_state::COUNTRIES), 2);
    assert_eq!(source.attempts(LINKS + 7 * 4), 2);
}

#[test]
fn heap_accounting_excludes_the_embedded_placeholder_and_queries_never_grow() {
    let vacant = NativeReferenceMemo::vacant();
    assert_eq!(vacant.allocated_bytes(), 0);
    let mut memo = memo(4, 9);
    let source = Source::positive();
    let prepared_bytes = memo.allocated_bytes();
    assert!(prepared_bytes > 0);
    assert!(memo.view(&source, 0, GS).resolve(7).is_ok());
    assert_eq!(memo.allocated_bytes(), prepared_bytes);
    memo.reset(0, GS);
    assert_eq!(memo.allocated_bytes(), prepared_bytes);
    memo.reserve(8, 18).unwrap();
    assert!(memo.allocated_bytes() > prepared_bytes);
}

struct FullObservationSource {
    source: Source,
    observation: super::super::ReservedCountryObservation,
    observation_calls: Cell<usize>,
}

impl FullObservationSource {
    fn failed_identity_once() -> Self {
        let mut source = Source::positive();
        source.reserved = None;
        source.u64(COUNTRIES, RESERVED.pointer as u64);
        source.u64(COUNTRIES + 8, RESERVED.pointer as u64);
        source.u64(RESERVED.pointer, super::super::VTABLE_RVA.0 as u64);
        source.u32(RESERVED.pointer + super::super::ID, 0);
        source.fail_once = Some(RESERVED.pointer);
        let observation = super::super::read_reserved_observation(&source, 0, GS);
        assert_eq!(
            observation.result(),
            Err("reserved country object identity mismatch")
        );
        Self {
            source,
            observation,
            observation_calls: Cell::new(0),
        }
    }
}

impl Memory for FullObservationSource {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.source.copy(address, destination)
    }

    fn authenticated_reserved_country(&self, _: usize, _: usize) -> Option<CountryResult> {
        panic!("a full reserved observation was downgraded to a bare Result");
    }

    fn authenticated_reserved_country_observation(
        &self,
        base: usize,
        state: usize,
    ) -> Option<super::super::ReservedCountryObservation> {
        self.observation_calls.set(self.observation_calls.get() + 1);
        ((base, state) == (0, GS)).then_some(self.observation)
    }
}

#[test]
fn failed_selected_pointer_survives_observed_links_without_reauthentication() {
    let source = FullObservationSource::failed_identity_once();
    let memo = memo(4, 9);
    let context = memo.view(&source, 0, GS);
    let memory = ObservedLinks::new(&context, None);
    for _ in 0..2 {
        let observation = super::super::read_reserved_observation(&memory, 0, GS);
        assert_eq!(observation, source.observation);
        assert_eq!(
            observation.physical_alias(RESERVED.pointer),
            Some(Err("reserved country object identity mismatch"))
        );
        assert_eq!(observation.physical_alias(OBJECT), None);
    }
    assert_eq!(source.observation_calls.get(), 1);
    assert_eq!(source.source.reserved_calls.get(), 1);
    assert_eq!(source.source.attempts(COUNTRIES), 1);
    assert_eq!(source.source.attempts(RESERVED.pointer), 1);
    let reader = Reader::new(&memory, 0, GS).unwrap();
    assert_eq!(
        reader.at(1),
        Err("reserved country object identity mismatch")
    );
    assert_eq!(source.source.attempts(RESERVED.pointer), 1);
    assert_eq!(source.source.attempts(COUNTRIES), 1);
    assert_eq!(source.observation_calls.get(), 1);
}

#[test]
fn observation_request_and_pass_mismatches_preserve_the_live_pass() {
    let source = FullObservationSource::failed_identity_once();
    let memo = memo(4, 9);
    let context = memo.view(&source, 0, GS);
    let memory = ObservedLinks::new(&context, None);
    let original = super::super::read_reserved_observation(&memory, 0, GS);
    let copies = source.source.copies.borrow().clone();
    for (base, state) in [(1, GS), (0, GS + 1)] {
        let wrong_request = super::super::read_reserved_observation(&memory, base, state);
        assert_eq!(
            wrong_request.result(),
            Err("native country reserved source belongs to a different context")
        );
        assert_eq!(wrong_request.physical_alias(RESERVED.pointer), None);
        let wrong_context = memo.view(&source, base, state);
        let wrong_memory = ObservedLinks::new(&wrong_context, None);
        let wrong_pass = super::super::read_reserved_observation(&wrong_memory, base, state);
        assert_eq!(
            wrong_pass.result(),
            Err("native country references belong to a different pass context")
        );
        assert_eq!(wrong_pass.physical_alias(RESERVED.pointer), None);
    }
    assert_eq!(memo.reserved.get(), Some(original));
    assert_eq!(
        super::super::read_reserved_observation(&memory, 0, GS),
        original
    );
    assert_eq!(source.observation_calls.get(), 1);
    assert_eq!(*source.source.copies.borrow(), copies);
}

#[test]
fn reset_and_reserve_clear_full_observation_before_the_next_pass() {
    let mut source = FullObservationSource::failed_identity_once();
    let mut memo = memo(4, 9);
    let failed = {
        let context = memo.view(&source, 0, GS);
        super::super::read_reserved_observation(&ObservedLinks::new(&context, None), 0, GS)
    };
    source.observation = super::super::read_reserved_observation(&source.source, 0, GS);
    assert_eq!(source.observation.result(), Ok(RESERVED));
    {
        let context = memo.view(&source, 0, GS);
        assert_eq!(
            super::super::read_reserved_observation(&ObservedLinks::new(&context, None), 0, GS),
            failed
        );
    }
    assert_eq!(source.observation_calls.get(), 1);
    let copies = source.source.copies.borrow().clone();
    memo.reset(0, GS);
    assert!(memo.reserved.get().is_none());
    {
        let context = memo.view(&source, 0, GS);
        let current =
            super::super::read_reserved_observation(&ObservedLinks::new(&context, None), 0, GS);
        assert_eq!(current, source.observation);
        assert_eq!(current.physical_alias(RESERVED.pointer), Some(Ok(())));
    }
    assert_eq!(source.observation_calls.get(), 2);
    memo.reserve(4, 9).unwrap();
    assert!(memo.reserved.get().is_none());
    {
        let context = memo.view(&source, 0, GS);
        let cleared =
            super::super::read_reserved_observation(&ObservedLinks::new(&context, None), 0, GS);
        assert_eq!(
            cleared.result(),
            Err("native country references belong to a different pass context")
        );
        assert_eq!(cleared.physical_alias(RESERVED.pointer), None);
    }
    assert!(memo.reserved.get().is_none());
    assert_eq!(source.observation_calls.get(), 2);
    memo.reset(0, GS);
    {
        let context = memo.view(&source, 0, GS);
        assert_eq!(
            super::super::read_reserved_observation(&ObservedLinks::new(&context, None), 0, GS),
            source.observation
        );
    }
    assert_eq!(source.observation_calls.get(), 3);
    assert_eq!(*source.source.copies.borrow(), copies);
}

#[test]
fn legacy_bare_failure_has_no_selected_pointer_and_does_not_hide_a_positive_country() {
    let mut source = Source::positive();
    source.reserved = Some(Err("legacy reserved fault"));
    let memo = memo(4, 9);
    let context = memo.view(&source, 0, GS);
    let memory = ObservedLinks::new(&context, None);
    let observation = super::super::read_reserved_observation(&memory, 0, GS);
    assert_eq!(observation.result(), Err("legacy reserved fault"));
    assert_eq!(observation.physical_alias(RESERVED.pointer), None);
    assert_eq!(observation.physical_alias(OBJECT), None);
    assert_eq!(
        super::super::read_reserved_observation(&memory, 0, GS),
        observation
    );
    assert_eq!(context.resolve(0), Err("legacy reserved fault"));
    assert_eq!(source.reserved_calls.get(), 1);
    assert!(source.copies.borrow().is_empty());
    assert_eq!(
        context.resolve(7),
        Ok(Country {
            pointer: OBJECT,
            canonical_slot: 1,
            id: 7,
        })
    );
    assert_eq!(source.attempts(OBJECT), 1);
    assert_eq!(source.attempts(COUNTRIES), 0);
    assert_eq!(source.attempts(RESERVED.pointer), 0);
    assert_eq!(source.reserved_calls.get(), 1);
}
