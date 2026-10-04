use super::super::{
    characters_test_kit as kit,
    tests::{ADDRESS, CHARACTERS, IMAGE, literal, pointer, put},
};
use super::*;
use std::collections::BTreeMap;

#[derive(Default)]
struct Observer {
    epoch: u64,
    clock: u64,
    changed: BTreeMap<usize, u64>,
    verify: bool,
    enabled: bool,
}
impl Observer {
    fn new() -> Self {
        Self {
            enabled: true,
            ..Default::default()
        }
    }
    fn change(&mut self, character: usize) {
        self.clock += 1;
        self.changed.insert(
            ADDRESS + character + ccharacter::ADVISOR_MAP_HEAD,
            self.clock,
        );
    }
}
impl Changes for Observer {
    fn quiet(&self) -> Option<(u64, u64)> {
        self.enabled.then_some((self.epoch, self.clock))
    }
    fn revision(&self, map: usize) -> Option<(u64, u64)> {
        self.enabled
            .then_some((self.epoch, *self.changed.get(&map).unwrap_or(&0)))
    }
    fn verify(&self) -> bool {
        self.verify
    }
}
fn context() -> CharactersCopyContext {
    CharactersCopyContext {
        force_full: false,
        base: IMAGE,
        game_state: ADDRESS,
        active: PART_MASKS[2],
        trait_rows: [0; 2],
        trait_bytes: [0; 2],
    }
}
fn assert_fresh(cache: &Section, bytes: &[u8]) {
    let mut captured = Captured::with_capacities(&Capacities::default());
    let direct = walk_family(
        &kit::memory(bytes),
        IMAGE,
        ADDRESS,
        captured.family().select(PART_MASKS[2]),
        None,
        None,
    )[2];
    assert_eq!(cache.result.outcome, direct.outcome);
    assert_eq!(cache.result.bytes_len, direct.bytes_len);
    assert_eq!(cache.result.required_bytes, direct.required_bytes);
    assert_eq!(
        cache.current.rows[..direct.outcome.len()],
        captured.advisors.rows.rows[..direct.outcome.len()]
    );
    assert_eq!(
        cache.current.bytes[..direct.bytes_len],
        captured.advisors.bytes.first[..direct.bytes_len]
    );
}

fn assert_outer_fresh(cache: &Cache, bytes: &[u8]) {
    let mut captured = Captured::with_capacities(&Capacities::default());
    cache.decode(&mut captured);
    let mut direct = Captured::with_capacities(&Capacities::default());
    let expected = walk_family(
        &kit::memory(bytes),
        IMAGE,
        ADDRESS,
        direct.family().select(PART_MASKS[2]),
        None,
        None,
    )[2];
    assert_eq!(cache.result.outcome, expected.outcome);
    assert_eq!(cache.result.required_bytes, expected.required_bytes);
    let len = expected.outcome.len();
    assert_eq!(
        captured.advisors.rows.rows[..len],
        direct.advisors.rows.rows[..len]
    );
    assert_eq!(
        captured.advisors.bytes.first[..expected.bytes_len],
        direct.advisors.bytes.first[..expected.bytes_len]
    );
}
#[test]
fn first_capture_prepares_controls_without_a_second_roster_discovery() {
    let bytes = super::super::tests::section_fixture();
    let mut observer = Observer::new();
    let mut cache = Cache::new();
    cache.capture(&kit::memory(&bytes), context(), &observer, None);
    assert_eq!(cache.counts.read, 2);
    cache.prepare();
    cache.capture(&kit::memory(&bytes), context(), &observer, None);
    assert_eq!(cache.counts.planned, 2);
    assert_outer_fresh(&cache, &bytes);
    observer.enabled = false;
    cache.capture(&kit::memory(&bytes), context(), &observer, None);
    cache.prepare();
    observer.enabled = true;
    cache.capture(&kit::memory(&bytes), context(), &observer, None);
    assert_eq!(cache.counts.planned, 0);
    assert_outer_fresh(&cache, &bytes);
}
#[test]
fn unregistered_maps_cannot_use_a_control_only_certificate() {
    struct Unwatched;
    impl Changes for Unwatched {
        fn quiet(&self) -> Option<(u64, u64)> {
            Some((0, 0))
        }
        fn revision(&self, _map: usize) -> Option<(u64, u64)> {
            None
        }
        fn verify(&self) -> bool {
            false
        }
    }
    let mut bytes = super::super::tests::section_fixture();
    let mut cache = Cache::new();
    for idea in [b"first".as_slice(), b"second", b"third"] {
        literal(&mut bytes, kit::ADV_ROLE + cadvisor::IDEA_TOKEN, idea);
        cache.capture(&kit::memory(&bytes), context(), &Unwatched, None);
        assert_eq!(cache.counts.planned, 0);
        assert_eq!(cache.counts.read, 2);
        assert_outer_fresh(&cache, &bytes);
        cache.prepare();
    }
}
fn warm(cache: &mut Cache, bytes: &[u8], observer: &Observer) {
    for _ in 0..3 {
        cache.capture(&kit::memory(bytes), context(), observer, None);
        assert_outer_fresh(cache, bytes);
        cache.prepare();
    }
    assert_eq!(cache.counts.planned, 2);
}
#[test]
fn independent_plans_refresh_country_mutations_aliases_and_roster_growth() {
    use super::super::tests::{MANAGER, array};
    let mut bytes = super::super::tests::section_fixture();
    let mut observer = Observer::new();
    let mut cache = Cache::new();
    warm(&mut cache, &bytes, &observer);
    array(
        &mut bytes,
        MANAGER + ccharacter_manager::DYNAMIC_CHARACTERS,
        0x8800,
        2,
    );
    pointer(&mut bytes, 0x8808, 0x2300);
    cache.capture(&kit::memory(&bytes), context(), &observer, None);
    assert_eq!(cache.sections[0].counts.planned, 1);
    assert_eq!(cache.sections[1].counts.planned, 0);
    assert_outer_fresh(&cache, &bytes);
    warm(&mut cache, &bytes, &observer);
    put(
        &mut bytes,
        CHARACTERS[0] + ccharacter::COUNTRY,
        99_u32.to_le_bytes(),
    );
    cache.capture(&kit::memory(&bytes), context(), &observer, None);
    assert_eq!(cache.sections[0].counts.planned, 1);
    assert_eq!(cache.sections[1].counts.planned, 1);
    assert_outer_fresh(&cache, &bytes);
    warm(&mut cache, &bytes, &observer);
    literal(&mut bytes, kit::ADV_ROLE + cadvisor::IDEA_TOKEN, b"changed");
    observer.change(CHARACTERS[0]);
    cache.capture(&kit::memory(&bytes), context(), &observer, None);
    assert_eq!(cache.sections[0].counts.read, 1);
    assert_eq!(cache.sections[1].counts.planned, 1);
    assert_outer_fresh(&cache, &bytes);
    pointer(&mut bytes, 0x8800, CHARACTERS[0]);
    cache.capture(&kit::memory(&bytes), context(), &observer, None);
    assert_outer_fresh(&cache, &bytes);
    warm(&mut cache, &bytes, &observer);
    observer.epoch += 1;
    cache.capture(&kit::memory(&bytes), context(), &observer, None);
    assert_eq!(cache.counts.reused, 0);
    assert_outer_fresh(&cache, &bytes);
}
#[test]
fn planned_control_failures_keep_first_fault_and_recover_with_fresh_data() {
    use super::super::tests::WaveMemory;
    let bytes = super::super::tests::section_fixture();
    let observer = Observer::new();
    let mut cache = Cache::new();
    warm(&mut cache, &bytes, &observer);
    let trace = WaveMemory::new(&bytes, None);
    cache.capture(&trace, context(), &observer, None);
    assert_eq!(cache.counts.planned, 2);
    let faults: std::collections::BTreeSet<_> = trace
        .reads
        .borrow()
        .iter()
        .flat_map(|&(address, len)| [address, address + len - 1])
        .collect();
    for fault in faults {
        warm(&mut cache, &bytes, &observer);
        let source = WaveMemory::new(&bytes, Some(fault));
        cache.capture(&source, context(), &observer, None);
        assert!(
            matches!(cache.result.outcome, Outcome::Unavailable { .. }),
            "{fault:x}"
        );
        assert_eq!(
            source
                .reads
                .borrow()
                .iter()
                .filter(|&&(a, n)| (a..a + n).contains(&fault))
                .count(),
            1,
            "{fault:x}"
        );
        assert!(cache.sections.iter().all(|section| !section.valid));
    }
}

#[test]
fn planned_country_prefix_survives_later_control_change_and_fault() {
    use super::super::tests::WaveMemory;
    let mut original = kit::mapped_fixture();
    add_second_role(&mut original);
    let header = CHARACTERS[1] + ccharacter::ADVISOR_MAP_HEAD;
    let sentinel = u64::from_le_bytes(original[header..header + 8].try_into().unwrap()) as usize;
    let country = ADDRESS + CHARACTERS[0] + ccharacter::COUNTRY;
    let observer = Observer::new();
    for fault in [false, true] {
        let mut cache = Cache::new();
        warm(&mut cache, &original, &observer);
        let baseline = WaveMemory::new(&original, None);
        cache.capture(&baseline, context(), &observer, None);
        assert_eq!(cache.sections[0].counts.planned, 1);
        let expected_reads = baseline.reads.borrow().clone();
        let country_index = expected_reads
            .iter()
            .position(|&read| read == (country, 4))
            .unwrap();
        let stop_index = expected_reads
            .iter()
            .position(|&read| read == (sentinel, 26))
            .unwrap();
        assert!(country_index < stop_index);

        let mut bytes = original.clone();
        put(
            &mut bytes,
            CHARACTERS[0] + ccharacter::COUNTRY,
            99_u32.to_le_bytes(),
        );
        // A changed, readable color byte stops the equality control without
        // invalidating the map. The alternate case partially copies and fails.
        bytes[sentinel - ADDRESS + engine::msvc_tree_node::COLOR] ^= 1;
        let source = WaveMemory::new(&bytes, fault.then_some(sentinel));
        cache.capture(&source, context(), &observer, None);
        assert_eq!(cache.sections[0].counts.planned, 0);
        let reads = source.reads.borrow();
        assert_eq!(reads[..=stop_index], expected_reads[..=stop_index]);
        for read in [(country, 4), (sentinel, 26)] {
            assert_eq!(reads.iter().filter(|&&seen| seen == read).count(), 1);
        }
        assert_eq!(cache.sections[0].current.rows[0].country, 99);
        if fault {
            assert_eq!(
                cache.result.outcome,
                Outcome::Unavailable {
                    reason: "character role map sentinel unreadable",
                }
            );
            assert_eq!(reads.as_slice(), &expected_reads[..=stop_index]);
            assert!(cache.sections.iter().all(|section| !section.valid));
            let mut captured = Captured::with_capacities(&Capacities::default());
            cache.decode(&mut captured);
            assert_eq!(captured.advisors.rows.outcome, cache.result.outcome);
        } else {
            assert_eq!(cache.sections[0].counts.read, 1);
            assert_eq!(cache.sections[0].counts.reused, 2);
            assert_outer_fresh(&cache, &bytes);
        }
    }
}

#[test]
fn planned_duplicate_characters_keep_each_country_observation() {
    use super::super::tests::{ROSTER, WaveMemory};
    struct Countries<'a> {
        bytes: &'a [u8],
        values: [u32; 3],
        next: Cell<usize>,
        reads: std::cell::RefCell<Vec<(usize, usize)>>,
    }
    impl<'a> Countries<'a> {
        fn new(bytes: &'a [u8], values: [u32; 3]) -> Self {
            Self {
                bytes,
                values,
                next: Cell::new(0),
                reads: Default::default(),
            }
        }
    }
    impl Memory for Countries<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, output.len()));
            if address == ADDRESS + CHARACTERS[0] + ccharacter::COUNTRY && output.len() == 4 {
                let index = self.next.get();
                let value = self.values.get(index).expect("country occurrence retried");
                self.next.set(index + 1);
                output.copy_from_slice(&value.to_le_bytes());
                true
            } else {
                kit::memory(self.bytes).copy(address, output)
            }
        }
    }
    let mut bytes = kit::mapped_fixture();
    pointer(&mut bytes, ROSTER + 8, CHARACTERS[0]);
    pointer(&mut bytes, ROSTER + 16, CHARACTERS[0]);
    let observer = Observer::new();
    let mut cache = Cache::new();
    warm(&mut cache, &bytes, &observer);
    let baseline = WaveMemory::new(&bytes, None);
    cache.capture(&baseline, context(), &observer, None);
    let expected_reads = baseline.reads.borrow().clone();
    let country_read = (ADDRESS + CHARACTERS[0] + ccharacter::COUNTRY, 4);
    assert_eq!(
        expected_reads
            .iter()
            .filter(|&&read| read == country_read)
            .count(),
        3
    );

    for countries in [[11, 22, 33], [44, 55, 66]] {
        let source = Countries::new(&bytes, countries);
        cache.capture(&source, context(), &observer, None);
        assert_eq!(cache.sections[0].counts.planned, 1);
        assert_eq!(cache.counts.read, 0);
        assert_eq!(source.next.get(), 3);
        assert_eq!(*source.reads.borrow(), expected_reads);

        // A separate observation sequence gives the ordinary reader the
        // same three distinct values; an address-keyed oracle would hide bugs.
        let direct_source = Countries::new(&bytes, countries);
        let mut direct = Captured::with_capacities(&Capacities::default());
        let expected = walk_family(
            &direct_source,
            IMAGE,
            ADDRESS,
            direct.family().select(PART_MASKS[2]),
            None,
            None,
        )[2];
        assert_eq!(direct_source.next.get(), 3);
        assert_eq!(expected.outcome, Outcome::Rows { len: 3 });
        assert_eq!(cache.result.outcome, expected.outcome);
        assert_eq!(cache.result.bytes_len, expected.bytes_len);
        assert_eq!(cache.result.required_bytes, expected.required_bytes);
        let mut captured = Captured::with_capacities(&Capacities::default());
        cache.decode(&mut captured);
        assert_eq!(
            captured.advisors.rows.rows[..3],
            direct.advisors.rows.rows[..3]
        );
        assert_eq!(
            [0, 1, 2].map(|index| captured.advisors.rows.rows[index].country),
            countries
        );
        assert_eq!(
            captured.advisors.bytes.first[..expected.bytes_len],
            direct.advisors.bytes.first[..expected.bytes_len]
        );
        cache.prepare();
    }
}

#[test]
fn earlier_truncation_suppresses_later_rows_and_retains_missing_demand() {
    let mut bytes = super::super::tests::section_fixture();
    add_second_role(&mut bytes);
    let observer = Observer::new();
    let mut cache = Cache::new();
    cache.sections[0] = Section::with_capacity(0, 0, 8);
    cache.capture(&kit::memory(&bytes), context(), &observer, None);
    assert!(matches!(
        cache.result.outcome,
        Outcome::Truncated {
            len: 0,
            unread_rows: 2
        }
    ));
    assert_eq!(cache.sections[1].result.outcome.len(), 0);
    for _ in 0..3 {
        cache.grow();
        cache.capture(&kit::memory(&bytes), context(), &observer, None);
    }
    assert_outer_fresh(&cache, &bytes);
    warm(&mut cache, &bytes, &observer);
}

fn add_second_role(bytes: &mut [u8]) {
    let character = CHARACTERS[1];
    let offset = character + ccharacter::ADVISOR_MAP_HEAD;
    let head = u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap()) as usize - ADDRESS;
    let node = 0x8500;
    let role = 0x8580;
    bytes.copy_within(kit::ADV_NODE..kit::ADV_NODE + 0x50, node);
    bytes.copy_within(kit::ADV_ROLE..kit::ADV_ROLE + 0xa0, role);
    for link in [0, 8, 16] {
        pointer(bytes, head + link, node);
        pointer(bytes, node + link, head);
    }
    put(bytes, offset + 8, 1_u64.to_le_bytes());
    pointer(bytes, node + character_advisor_node::ADVISOR, role);
    pointer(bytes, role + cadvisor::CHARACTER, character);
}

#[test]
fn role_addition_removal_and_arena_relocation_keep_other_characters_reusable() {
    let mut bytes = kit::mapped_fixture();
    let mut cache = Section::with_capacity(8, 512, 8);
    let mut observer = Observer::new();
    cache.capture(&kit::memory(&bytes), context(), &observer);
    add_second_role(&mut bytes);
    observer.change(CHARACTERS[1]);
    cache.capture(&kit::memory(&bytes), context(), &observer);
    assert_fresh(&cache, &bytes);
    assert_eq!((cache.counts.read, cache.counts.reused), (1, 2));
    literal(
        &mut bytes,
        kit::ADV_ROLE + cadvisor::IDEA_TOKEN,
        b"longer_first",
    );
    observer.change(CHARACTERS[0]);
    cache.capture(&kit::memory(&bytes), context(), &observer);
    assert_fresh(&cache, &bytes);
    assert_eq!(
        (cache.counts.read, cache.counts.reused, cache.counts.roles),
        (1, 2, 1)
    );
    put(
        &mut bytes,
        CHARACTERS[0] + ccharacter::ADVISOR_MAP_HEAD + 8,
        0_u64.to_le_bytes(),
    );
    for link in [0, 8, 16] {
        pointer(&mut bytes, 0x2430 + link, 0x2430);
    }
    observer.change(CHARACTERS[0]);
    cache.capture(&kit::memory(&bytes), context(), &observer);
    assert_fresh(&cache, &bytes);
    assert_eq!(
        (cache.counts.read, cache.counts.reused, cache.counts.roles),
        (1, 2, 1)
    );
}
#[test]
fn unchanged_roles_skip_payload_reads_but_country_stays_current() {
    let mut bytes = kit::mapped_fixture();
    let mut cache = Section::with_capacity(8, 512, 8);
    let observer = Observer::new();
    cache.capture(&kit::memory(&bytes), context(), &observer);
    assert_fresh(&cache, &bytes);
    let cold = cache.raw_bytes;
    put(
        &mut bytes,
        CHARACTERS[0] + ccharacter::COUNTRY,
        99_u32.to_le_bytes(),
    );
    cache.capture(&kit::memory(&bytes), context(), &observer);
    assert_fresh(&cache, &bytes);
    assert_eq!(
        (cache.counts.reused, cache.counts.read, cache.counts.roles),
        (3, 0, 1)
    );
    assert!(cache.raw_bytes < cold);
    assert_eq!(cache.current.rows[0].country, 99);
    // Decode uses only owned bytes, even after all source storage disappears.
    bytes.fill(0);
    let mut output = Captured::with_capacities(&Capacities::default());
    cache.decode(&mut output);
    assert_eq!(output.advisors.rows.rows[0].country, 99);
}
#[test]
fn same_count_replacement_and_larger_strings_refresh_only_the_affected_character() {
    let mut bytes = kit::mapped_fixture();
    let mut cache = Section::with_capacity(8, 512, 8);
    let mut observer = Observer::new();
    cache.capture(&kit::memory(&bytes), context(), &observer);
    bytes.copy_within(kit::ADV_ROLE..kit::ADV_ROLE + 0xa0, 0x8600);
    pointer(
        &mut bytes,
        kit::ADV_NODE + character_advisor_node::ADVISOR,
        0x8600,
    );
    literal(&mut bytes, 0x8600 + cadvisor::IDEA_TOKEN, b"different_idea");
    observer.change(CHARACTERS[0]);
    cache.capture(&kit::memory(&bytes), context(), &observer);
    assert_fresh(&cache, &bytes);
    assert_eq!((cache.counts.read, cache.counts.reused), (1, 2));
    // Epoch invalidation forces all entries fresh, including empty maps.
    observer.epoch += 1;
    cache.capture(&kit::memory(&bytes), context(), &observer);
    assert_fresh(&cache, &bytes);
    assert_eq!((cache.counts.read, cache.counts.reused), (3, 0));
}
#[test]
fn verification_detects_an_unobserved_in_place_write_and_returns_fresh_data() {
    let mut bytes = kit::mapped_fixture();
    let mut cache = Section::with_capacity(8, 512, 8);
    let mut observer = Observer::new();
    observer.verify = true;
    cache.capture(&kit::memory(&bytes), context(), &observer);
    literal(
        &mut bytes,
        kit::ADV_ROLE + cadvisor::IDEA_TOKEN,
        b"unobserved",
    );
    cache.capture(&kit::memory(&bytes), context(), &observer);
    assert_fresh(&cache, &bytes);
    assert_eq!(cache.counts.verified, 3);
    assert_eq!(cache.counts.mismatches, 1);
    assert!(!cache.valid);
    cache.capture(&kit::memory(&bytes), context(), &observer);
    assert_eq!(cache.counts.verified, 0);
}
#[test]
fn truncation_and_unwatched_entries_stay_fresh_and_grow_off_capture() {
    let bytes = kit::mapped_fixture();
    let mut cache = Section::with_capacity(0, 0, 1);
    let mut observer = Observer::new();
    cache.capture(&kit::memory(&bytes), context(), &observer);
    assert!(matches!(cache.result.outcome, Outcome::Truncated { .. }));
    assert!(!cache.valid);
    for _ in 0..3 {
        cache.grow();
        cache.capture(&kit::memory(&bytes), context(), &observer);
    }
    assert_fresh(&cache, &bytes);
    observer.enabled = false;
    cache.capture(&kit::memory(&bytes), context(), &observer);
    assert_fresh(&cache, &bytes);
    assert_eq!(cache.counts.reused, 0);
}
#[test]
fn fresh_control_faults_and_dirty_role_faults_never_reuse_stale_success() {
    let bytes = kit::mapped_fixture();
    let mut observer = Observer::new();
    struct Fault<'a> {
        bytes: &'a [u8],
        address: usize,
        attempts: Cell<usize>,
    }
    impl Memory for Fault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address <= self.address && self.address < address + output.len() {
                self.attempts.set(self.attempts.get() + 1);
                return false;
            }
            kit::memory(self.bytes).copy(address, output)
        }
    }
    for address in [
        ADDRESS + CHARACTERS[0] + ccharacter::COUNTRY,
        ADDRESS + CHARACTERS[0] + ccharacter::ADVISOR_MAP_HEAD,
        ADDRESS + 0x2430,
        ADDRESS + kit::ADV_ROLE + cadvisor::IDEA_TOKEN,
    ] {
        let mut cache = Section::with_capacity(8, 512, 8);
        cache.capture(&kit::memory(&bytes), context(), &observer);
        observer.change(CHARACTERS[0]);
        let fault = Fault {
            bytes: &bytes,
            address,
            attempts: Cell::new(0),
        };
        cache.capture(&fault, context(), &observer);
        assert!(matches!(cache.result.outcome, Outcome::Unavailable { .. }));
        assert_eq!(fault.attempts.get(), 1);
        assert!(!cache.valid);
    }
}
