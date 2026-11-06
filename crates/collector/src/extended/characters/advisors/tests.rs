use super::super::{
    characters_test_kit as kit,
    tests::{ADDRESS, CHARACTERS, IMAGE, fixture_capacities, literal, pointer, put},
};
use super::*;
use std::collections::BTreeMap;

#[derive(Default)]
struct Observer {
    epoch: u64,
    clock: u64,
    changed: BTreeMap<usize, u64>,
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
}
fn context() -> CharactersCopyContext {
    CharactersCopyContext {
        force_full: false,
        base: IMAGE,
        game_state: ADDRESS,
        active: PART_MASKS[2],
        trait_rows: 0,
        trait_bytes: 0,
    }
}
fn cache_for(bytes: &[u8]) -> Cache {
    let demand = cadvisor::modifier_demand(&kit::memory(bytes), IMAGE, ADDRESS).unwrap();
    let mut cache = Cache::with_modifier_demand(demand);
    cache.sections = [
        Section::with_capacity(16384, 1 << 20, 16384),
        Section::with_capacity(4096, 1 << 18, 8192),
    ];
    cache
}

#[test]
fn advisor_modifier_sizing_survives_an_unreadable_political_census() {
    struct World<'a>(&'a [u8]);
    impl Memory for World<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == IMAGE + engine_native::cgame_state::SINGLETON_RVA.0 && output.len() == 8 {
                output.copy_from_slice(&(ADDRESS as u64).to_le_bytes());
                return true;
            }
            if address == ADDRESS + engine_native::cgame_state::COUNTRIES && output.len() == 16 {
                output[..8].copy_from_slice(&((ADDRESS + 0x3000) as u64).to_le_bytes());
                output[8..12].copy_from_slice(&2_u32.to_le_bytes());
                output[12..].copy_from_slice(&2_u32.to_le_bytes());
                return true;
            }
            kit::memory(self.0).copy(address, output)
        }
    }
    let bytes = kit::mapped_fixture();
    let memory = World(&bytes);
    assert!(crate::country_scale::observe_politics(&memory, IMAGE).is_err());
    let demand = crate::country_scale::observe_advisor_modifiers(&memory, IMAGE)
        .unwrap()
        .unwrap();
    assert!(demand.rows > 0);
    let mut cache = Cache::with_modifier_demand(demand);
    cache.sections = [
        Section::with_capacity(16384, 1 << 20, 16384),
        Section::with_capacity(4096, 1 << 18, 8192),
    ];
    cache.capture(&memory, context(), &Observer::new(), None);
    assert_eq!(cache.modifiers.shortfall, 0);
}
fn assert_fresh(cache: &Section, bytes: &[u8]) {
    for entry in &cache.current.entries {
        for row in &cache.current.rows[entry.rows.clone()] {
            // Definition-only reads retain the end of their slot bytes even
            // though the independently captured modifier span is unrecorded.
            assert_eq!(row.modifier_start, row.slot_start + row.slot_len);
            assert!(row.modifier_start >= entry.bytes.start);
        }
    }
    let mut captured = Captured::with_capacities(&fixture_capacities());
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
    // Both hourly suffixes are compared by the complete outer-cache tests.
    let definition = |row: &cadvisor::Row| cadvisor::Row {
        modifier_start: 0,
        modifier_len: 0,
        modifier_recorded: false,
        modifier_failure: None,
        tooltip_start: 0,
        tooltip_len: 0,
        tooltip_recorded: false,
        tooltip_failure: None,
        ..*row
    };
    assert_eq!(
        cache.current.rows[..direct.outcome.len()]
            .iter()
            .map(definition)
            .collect::<Vec<_>>(),
        captured.advisors.rows.rows[..direct.outcome.len()]
            .iter()
            .map(definition)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        cache.current.bytes[..direct.bytes_len],
        captured.advisors.bytes.first[..direct.bytes_len]
    );
}

fn assert_outer_fresh(cache: &Cache, bytes: &[u8]) {
    assert_outer_matches(cache, &kit::memory(bytes));
}
fn assert_outer_matches(cache: &Cache, memory: &impl Memory) {
    let mut captured = Captured::with_capacities(&fixture_capacities());
    cache.decode(&mut captured);
    let mut direct = Captured::with_capacities(&fixture_capacities());
    let expected = walk_family(
        memory,
        IMAGE,
        ADDRESS,
        direct.family().select(PART_MASKS[2]),
        None,
        None,
    )[2];
    assert_eq!(cache.result.outcome, expected.outcome);
    direct.advisors.primary(expected);
    assert_eq!(captured.advisors_table(), direct.advisors_table());
}
#[test]
fn first_capture_prepares_controls_without_a_second_roster_discovery() {
    let bytes = super::super::tests::section_fixture();
    let mut observer = Observer::new();
    let mut cache = cache_for(&bytes);
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
    }
    let mut bytes = super::super::tests::section_fixture();
    let mut cache = cache_for(&bytes);
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
fn fresh_modifier_cache_changes_without_an_advisor_mutation_hook() {
    use super::super::tests::array;
    let mut bytes = super::super::tests::section_fixture();
    let data = 0x8a00;
    array(
        &mut bytes,
        kit::ADV_ROLE + cadvisor::MODIFIER_ENTRIES,
        data,
        2,
    );
    put(&mut bytes, data, 7_u32.to_le_bytes());
    put(&mut bytes, data + 8, i64::MIN.to_le_bytes());
    put(&mut bytes, data + 16, 7_u32.to_le_bytes());
    put(&mut bytes, data + 24, i64::MAX.to_le_bytes());
    let observer = Observer::new();
    let mut cache = cache_for(&bytes);
    warm(&mut cache, &bytes, &observer);
    put(&mut bytes, data + 8, (-1_i64).to_le_bytes());
    cache.capture(&kit::memory(&bytes), context(), &observer, None);
    assert_eq!(cache.counts.planned, 2);
    assert_outer_fresh(&cache, &bytes);
    let mut captured = Captured::with_capacities(&fixture_capacities());
    // Projection consumes owned bytes after the native objects disappear.
    bytes.fill(0);
    cache.decode(&mut captured);
    let TableView::Complete(TableRows::AdvisorRoles(rows)) = captured.advisors_table() else {
        panic!("complete advisor roles");
    };
    let cache = rows[0]
        .modifier_cache
        .require("AdvisorRoles.modifier_cache")
        .unwrap()
        .as_ref()
        .unwrap();
    assert_eq!(
        cache.entries,
        [
            replay_data::rows::CachedModifier {
                definition: 7,
                value: -1,
            },
            replay_data::rows::CachedModifier {
                definition: 7,
                value: i64::MAX,
            },
        ]
    );
}

#[test]
fn fresh_cache_spans_survive_definition_relocation_and_faults() {
    use super::super::tests::{WaveMemory, array};
    let mut bytes = super::super::tests::section_fixture();
    add_second_role(&mut bytes);
    for (role, data, key, value) in [
        (kit::ADV_ROLE, 0x8a00, 11_u32, i64::MIN),
        (0x8580, 0x8b00, 12_u32, i64::MAX),
    ] {
        array(&mut bytes, role + cadvisor::MODIFIER_ENTRIES, data, 1);
        put(&mut bytes, data, key.to_le_bytes());
        put(&mut bytes, data + 8, value.to_le_bytes());
    }
    let mut observer = Observer::new();
    let mut cache = cache_for(&bytes);
    warm(&mut cache, &bytes, &observer);
    literal(
        &mut bytes,
        kit::ADV_ROLE + cadvisor::IDEA_TOKEN,
        b"longer_idea",
    );
    observer.change(CHARACTERS[0]);
    cache.capture(&kit::memory(&bytes), context(), &observer, None);
    assert_eq!(cache.sections[1].counts.planned, 1);
    assert_outer_fresh(&cache, &bytes);
    warm(&mut cache, &bytes, &observer);
    let fault = ADDRESS + 0x8b08;
    let source = WaveMemory::new(&bytes, Some(fault));
    cache.capture(&source, context(), &observer, None);
    assert_eq!(
        cache.result.outcome,
        Outcome::Unavailable {
            reason: "advisor modifier cache entries unreadable",
        }
    );
    assert_eq!(
        source
            .reads
            .borrow()
            .iter()
            .filter(|&&(address, len)| (address..address + len).contains(&fault))
            .count(),
        1
    );
}

#[test]
fn tooltip_shortage_preserves_later_numeric_caches() {
    use super::super::tests::{SECOND_ADVISOR, two_advisor_fixture};
    let bytes = two_advisor_fixture(true);
    let rows = [kit::ADV_ROLE, SECOND_ADVISOR].map(|pointer| cadvisor::Row {
        pointer: ADDRESS + pointer,
        ..Default::default()
    });
    for capacity in [32, 40, 49] {
        let mut modifiers = Modifiers::with_demand(cadvisor::ModifierDemand {
            rows: 2,
            bytes: capacity,
            tooltip_bytes: 0,
        });
        modifiers.bytes.truncate(capacity);
        modifiers.capture(&kit::memory(&bytes), rows.iter());
        assert_eq!(modifiers.failure, None);
        assert_eq!(modifiers.required_rows, 2);
        assert_eq!(modifiers.required_bytes, 49);
        for (index, span) in modifiers.spans[..2].iter().enumerate() {
            assert!(span.recorded);
            assert_eq!(span.start, index * 16);
            assert_eq!(
                &modifiers.bytes[span.start + 8..span.start + 16],
                &if index == 0 { -17_i64 } else { 19_i64 }.to_le_bytes()
            );
            assert_eq!(
                span.tooltip_recorded,
                capacity >= if index == 0 { 40 } else { 49 }
            );
            if !span.tooltip_recorded {
                assert_eq!(
                    span.tooltip_failure,
                    Some(cadvisor::TOOLTIP_CAPACITY_REASON)
                );
            }
        }
        assert_eq!(
            modifiers.shortfall,
            if capacity == 32 {
                2
            } else if capacity == 40 {
                1
            } else {
                0
            }
        );
    }
}

#[test]
fn optional_cache_capacity_shortfall_keeps_roles_and_definition_certificates() {
    use super::super::tests::array;
    let mut bytes = super::super::tests::section_fixture();
    array(
        &mut bytes,
        kit::ADV_ROLE + cadvisor::MODIFIER_ENTRIES,
        0x8a00,
        2,
    );
    let observer = Observer::new();
    let mut cache = cache_for(&bytes);
    warm(&mut cache, &bytes, &observer);
    cache.modifiers.spans.clear();
    cache.modifiers.bytes.clear();
    cache.capture(&kit::memory(&bytes), context(), &observer, None);
    assert_eq!(cache.result.outcome, Outcome::Rows { len: 1 });
    assert_eq!(cache.modifiers.shortfall, 1);
    assert_eq!(cache.modifiers.failure, None);
    assert!(cache.sections.iter().all(|section| section.valid));
    assert_eq!(cache.counts.planned, 2);
    let mut captured = Captured::with_capacities(&fixture_capacities());
    cache.decode(&mut captured);
    let TableView::Complete(TableRows::AdvisorRoles(rows)) = captured.advisors_table() else {
        panic!("capacity shortfall keeps advisor roles");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].idea, b"idea");
    assert_eq!(
        rows[0].modifier_cache,
        replay_data::rows::Recorded::Value(None)
    );
    assert_eq!(
        captured.advisors.rows.rows[0].modifier_failure,
        Some(cadvisor::MODIFIER_CAPACITY_REASON)
    );
    cache.prepare();
    cache.grow();
    cache.capture(&kit::memory(&bytes), context(), &observer, None);
    assert_eq!(cache.counts.planned, 2);
    assert_outer_fresh(&cache, &bytes);
}

#[test]
fn advisor_raw_bytes_charge_current_reads_once_even_after_a_root_fault() {
    use super::super::tests::WaveMemory;
    let bytes = super::super::tests::section_fixture();
    let observer = Observer::new();
    let mut cache = cache_for(&bytes);
    warm(&mut cache, &bytes, &observer);
    let source = WaveMemory::new(&bytes, None);
    cache.capture(&source, context(), &observer, None);
    assert_eq!(
        cache.raw_bytes,
        source
            .reads
            .borrow()
            .iter()
            .map(|&(_, len)| len as u64)
            .sum::<u64>()
    );
    let fault = ADDRESS + cgame_state::CHARACTER_MANAGER;
    let source = WaveMemory::new(&bytes, Some(fault));
    cache.capture(&source, context(), &observer, None);
    assert!(matches!(cache.result.outcome, Outcome::Unavailable { .. }));
    assert_eq!(cache.raw_bytes, 0);
    assert!(cache.sections.iter().all(|section| section.raw_bytes == 0));
}
#[test]
fn independent_plans_refresh_country_mutations_aliases_and_roster_growth() {
    use super::super::tests::{MANAGER, array};
    let mut bytes = super::super::tests::section_fixture();
    let mut observer = Observer::new();
    let mut cache = cache_for(&bytes);
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
    let mut cache = cache_for(&bytes);
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
        let reference = WaveMemory::new(&bytes, Some(fault));
        assert_outer_matches(&cache, &reference);
        assert_eq!(
            source
                .reads
                .borrow()
                .iter()
                .filter(|&&(a, n)| (a..a + n).contains(&fault))
                .count(),
            reference
                .reads
                .borrow()
                .iter()
                .filter(|&&(a, n)| (a..a + n).contains(&fault))
                .count(),
            "{fault:x}"
        );
        if matches!(cache.result.outcome, Outcome::Unavailable { .. }) {
            assert!(cache.sections.iter().all(|section| !section.valid));
        } else {
            // Appended tooltip faults do not invalidate observed definitions.
            assert!(cache.sections.iter().all(|section| section.valid));
            let mut captured = Captured::with_capacities(&fixture_capacities());
            cache.decode(&mut captured);
            assert!(
                captured.advisors.rows.rows[..cache.result.outcome.len()]
                    .iter()
                    .any(|row| row.tooltip_failure.is_some())
            );
        }
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
        let mut cache = cache_for(&original);
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
        bytes[sentinel - ADDRESS + engine_native::msvc_tree_node::COLOR] ^= 1;
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
            let mut captured = Captured::with_capacities(&fixture_capacities());
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
    let mut cache = cache_for(&bytes);
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
        let mut direct = Captured::with_capacities(&fixture_capacities());
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
        let mut captured = Captured::with_capacities(&fixture_capacities());
        cache.decode(&mut captured);
        direct.advisors.primary(expected);
        assert_eq!(captured.advisors_table(), direct.advisors_table());
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
    let mut cache = cache_for(&bytes);
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
    let mut output = Captured::with_capacities(&fixture_capacities());
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

#[test]
fn tooltip_keys_stay_fresh_under_definition_reuse_and_shortfalls_keep_roles() {
    let mut bytes = super::super::tests::section_fixture();
    let table = kit::ADV_ROLE + cadvisor::TOOLTIP_KEYS + 8;
    pointer(&mut bytes, table, 0x8b00);
    put(&mut bytes, table + 8, 1_u32.to_le_bytes());
    put(&mut bytes, table + 12, 3_u32.to_le_bytes());
    put(&mut bytes, 0x8b04, [1]);
    literal(&mut bytes, 0x8b08, b"first");
    let observer = Observer::new();
    let mut cache = cache_for(&bytes);
    warm(&mut cache, &bytes, &observer);
    literal(&mut bytes, 0x8b08, b"later");
    cache.capture(&kit::memory(&bytes), context(), &observer, None);
    assert_eq!(cache.counts.planned, 2);
    let mut captured = Captured::with_capacities(&fixture_capacities());
    cache.decode(&mut captured);
    let TableView::Complete(TableRows::AdvisorRoles(rows)) = captured.advisors_table() else {
        panic!("tooltip capture keeps roles");
    };
    assert_eq!(
        rows[0].tooltip_keys,
        replay_data::rows::Recorded::Value(Some(vec![b"later".to_vec()]))
    );
    cache.modifiers.bytes.clear();
    cache.capture(&kit::memory(&bytes), context(), &observer, None);
    cache.decode(&mut captured);
    let TableView::Complete(TableRows::AdvisorRoles(rows)) = captured.advisors_table() else {
        panic!("tooltip capacity shortfall keeps roles");
    };
    assert_eq!(rows[0].idea, b"idea");
    assert_eq!(
        rows[0].tooltip_keys,
        replay_data::rows::Recorded::Value(None)
    );
    assert_eq!(
        captured.advisors.rows.rows[0].tooltip_failure,
        Some(cadvisor::TOOLTIP_CAPACITY_REASON)
    );
    cache.grow();
    cache.capture(&kit::memory(&bytes), context(), &observer, None);
    assert_outer_fresh(&cache, &bytes);
}

/// `count` historical characters with one advisor role each; every role
/// shares one numeric cache (`numeric`) and one tooltip key set (`tooltip`).
struct ManyAdvisors {
    bytes: Vec<u8>,
    first_character: usize,
    numeric: usize,
    tooltip: usize,
}

fn many_advisors(count: usize) -> ManyAdvisors {
    use super::super::tests::MANAGER;
    use engine_native::{ccharacter_manager, character_advisor_node, msvc_tree_node as node};
    fn alloc(bytes: &mut Vec<u8>, len: usize) -> usize {
        let at = bytes.len();
        bytes.resize(at + len, 0);
        at
    }
    let mut bytes = kit::mapped_fixture();
    let roster = alloc(&mut bytes, count * 8);
    pointer(
        &mut bytes,
        MANAGER + ccharacter_manager::HISTORICAL_CHARACTERS,
        roster,
    );
    for offset in [8, 12] {
        put(
            &mut bytes,
            MANAGER + ccharacter_manager::HISTORICAL_CHARACTERS + offset,
            (count as u32).to_le_bytes(),
        );
    }
    for offset in [0, 8, 12] {
        put(
            &mut bytes,
            MANAGER + ccharacter_manager::DYNAMIC_CHARACTERS + offset,
            [0; 4],
        );
    }
    let numeric = alloc(&mut bytes, 16);
    put(&mut bytes, numeric, 71_u32.to_le_bytes());
    put(&mut bytes, numeric + 8, (-17_i64).to_le_bytes());
    let tooltip = alloc(&mut bytes, 0x28);
    bytes[tooltip + 4] = 1;
    literal(&mut bytes, tooltip + 8, b"key");
    let mut first_character = 0;
    for index in 0..count {
        let character = alloc(&mut bytes, 0x120);
        let head = alloc(&mut bytes, 32);
        let entry = alloc(&mut bytes, character_advisor_node::SIZE);
        // The header after the table pointer contains data/count/mask and its final overflow byte.
        let role = alloc(&mut bytes, cadvisor::TOOLTIP_KEYS + 8 + 17);
        if index == 0 {
            first_character = character;
        }
        pointer(&mut bytes, roster + index * 8, character);
        put(
            &mut bytes,
            character,
            ((IMAGE + ccharacter::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            character + ccharacter::TYPE,
            ccharacter::REFERENCE_TYPE.to_le_bytes(),
        );
        put(
            &mut bytes,
            character + ccharacter::ID,
            (index as u32 + 1).to_le_bytes(),
        );
        put(
            &mut bytes,
            character + ccharacter::COUNTRY,
            (index as u32 + 1).to_le_bytes(),
        );
        pointer(&mut bytes, character + ccharacter::ADVISOR_MAP_HEAD, head);
        put(
            &mut bytes,
            character + ccharacter::ADVISOR_MAP_SIZE,
            1_u64.to_le_bytes(),
        );
        for link in [node::LEFT, node::PARENT, node::RIGHT] {
            pointer(&mut bytes, head + link, entry);
            pointer(&mut bytes, entry + link, head);
        }
        bytes[head + node::NIL] = 1;
        literal(&mut bytes, entry + character_advisor_node::KEY, b"key");
        pointer(&mut bytes, entry + character_advisor_node::ADVISOR, role);
        put(
            &mut bytes,
            role,
            ((IMAGE + cadvisor::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        pointer(&mut bytes, role + cadvisor::CHARACTER, character);
        literal(&mut bytes, role + cadvisor::IDEA_TOKEN, b"idea");
        literal(&mut bytes, role + cadvisor::SLOT, b"slot");
        pointer(&mut bytes, role + cadvisor::MODIFIER_ENTRIES, numeric);
        for offset in [8, 12] {
            put(
                &mut bytes,
                role + cadvisor::MODIFIER_ENTRIES + offset,
                1_u32.to_le_bytes(),
            );
        }
        pointer(&mut bytes, role + cadvisor::TOOLTIP_KEYS + 8, tooltip);
        put(
            &mut bytes,
            role + cadvisor::TOOLTIP_KEYS + 16,
            1_u32.to_le_bytes(),
        );
    }
    ManyAdvisors {
        bytes,
        first_character,
        numeric,
        tooltip,
    }
}

/// Counts reads overlapping fixture `ranges` and serves every read.
struct Counting<'a> {
    bytes: &'a [u8],
    ranges: Vec<std::ops::Range<usize>>,
    reads: std::sync::atomic::AtomicUsize,
}
impl<'a> Counting<'a> {
    fn new(bytes: &'a [u8], ranges: Vec<std::ops::Range<usize>>) -> Self {
        Self {
            bytes,
            ranges,
            reads: Default::default(),
        }
    }
    fn reads(&self) -> usize {
        self.reads.load(std::sync::atomic::Ordering::Relaxed)
    }
}
impl Memory for Counting<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let start = address.wrapping_sub(ADDRESS);
        let end = start.saturating_add(output.len());
        if self
            .ranges
            .iter()
            .any(|range| start < range.end && range.start < end)
        {
            self.reads
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        kit::memory(self.bytes).copy(address, output)
    }
}

/// The published modifier cache of the fixture's first advisor role.
fn first_role_entries(cache: &Cache) -> Vec<replay_data::rows::CachedModifier> {
    let mut captured = Captured::with_capacities(&fixture_capacities());
    cache.decode(&mut captured);
    let TableView::Complete(TableRows::AdvisorRoles(rows)) = captured.advisors_table() else {
        panic!("complete advisor roles");
    };
    rows[0]
        .modifier_cache
        .require("AdvisorRoles.modifier_cache")
        .unwrap()
        .as_ref()
        .unwrap()
        .entries
        .clone()
}

/// The section fixture with a two-entry numeric cache on its first role.
fn two_entry_cache() -> (Vec<u8>, std::ops::Range<usize>) {
    use super::super::tests::array;
    let mut bytes = super::super::tests::section_fixture();
    let data = 0x8a00;
    array(
        &mut bytes,
        kit::ADV_ROLE + cadvisor::MODIFIER_ENTRIES,
        data,
        2,
    );
    put(&mut bytes, data, 7_u32.to_le_bytes());
    put(&mut bytes, data + 8, (-5_i64).to_le_bytes());
    put(&mut bytes, data + 16, 8_u32.to_le_bytes());
    put(&mut bytes, data + 24, 9_i64.to_le_bytes());
    (bytes, data..data + 32)
}

fn entry(definition: u32, value: i64) -> replay_data::rows::CachedModifier {
    replay_data::rows::CachedModifier { definition, value }
}

#[test]
fn certified_advisor_modifiers_are_carried_until_a_hooked_rebuild() {
    use crate::character_changes::Watches;
    static WATCHES: Watches = Watches::new();
    WATCHES.enable(64);
    let (mut bytes, data) = two_entry_cache();
    let role = kit::ADV_ROLE;
    // The role's accumulator header, entries and tooltip key table.
    let watched = || {
        vec![
            role + cadvisor::MODIFIER_ENTRIES..role + cadvisor::MODIFIER_ENTRIES + 16,
            data.clone(),
            role + cadvisor::TOOLTIP_KEYS..role + cadvisor::TOOLTIP_KEYS + 24,
        ]
    };
    let observer = Observer::new();
    let mut cache = cache_for(&bytes);
    cache.modifiers.roles = RoleObservation::with(&WATCHES);
    // The first hour reads and holds every role.
    cache.capture_growing(&kit::memory(&bytes), context(), &observer, None);
    let roles = cache.result.outcome.len() as u64;
    assert!(roles >= 1);
    assert_eq!(
        (
            cache.modifiers.role_counts.reused,
            cache.modifiers.role_counts.read
        ),
        (0, roles)
    );
    assert_outer_fresh(&cache, &bytes);
    cache.prepare();
    // A quiet hour carries every role without reading its cache or keys.
    let quiet = Counting::new(&bytes, watched());
    cache.capture_growing(&quiet, context(), &observer, None);
    assert_eq!(quiet.reads(), 0, "a carried role is not read");
    assert_eq!(
        (
            cache.modifiers.role_counts.reused,
            cache.modifiers.role_counts.read
        ),
        (roles, 0)
    );
    assert_outer_fresh(&cache, &bytes);
    assert_eq!(first_role_entries(&cache), [entry(7, -5), entry(8, 9)]);
    // No native writer changes the cache outside the hooked rebuild, so a
    // write no hook saw stays unread: the held entries are published.
    put(&mut bytes, data.start + 8, (-1_i64).to_le_bytes());
    cache.capture_growing(&kit::memory(&bytes), context(), &observer, None);
    assert_eq!(first_role_entries(&cache), [entry(7, -5), entry(8, 9)]);
    // A trait-addition effect rebuilds the cache through the hooked
    // advisor modifier rebuild on the role's character: that role alone is read afresh.
    drop(WATCHES.notify(
        Some(ADDRESS + CHARACTERS[0]),
        engine_mutations::character_role_mutations::ADVISOR_MODIFIERS,
    ));
    let rebuilt = Counting::new(&bytes, watched());
    cache.capture_growing(&rebuilt, context(), &observer, None);
    assert!(rebuilt.reads() > 0);
    assert_eq!(cache.modifiers.role_counts.read, 1);
    assert_eq!(cache.modifiers.role_counts.reused, roles - 1);
    assert_outer_fresh(&cache, &bytes);
    assert_eq!(first_role_entries(&cache), [entry(7, -1), entry(8, 9)]);
    // A load, gap or recovery starts an epoch: every role is read.
    WATCHES.boundary(true);
    cache.capture_growing(&kit::memory(&bytes), context(), &observer, None);
    assert_eq!(
        (
            cache.modifiers.role_counts.reused,
            cache.modifiers.role_counts.read
        ),
        (0, roles)
    );
    assert_outer_fresh(&cache, &bytes);
}

#[test]
fn quiet_hours_carry_advisor_modifier_caches_and_read_only_a_rebuilt_role() {
    use crate::character_changes::Watches;
    static WATCHES: Watches = Watches::new();
    let ManyAdvisors {
        bytes,
        first_character,
        numeric,
        tooltip,
        ..
    } = many_advisors(40);
    WATCHES.enable(40);
    let observer = Observer::new();
    let mut cache = Cache::with_modifier_demand(cadvisor::ModifierDemand::default());
    cache.sections = std::array::from_fn(|_| Section::with_capacity(0, 0, 0));
    cache.modifiers.roles = RoleObservation::with(&WATCHES);
    let mut captured = Captured::with_capacities(&Capacities {
        rows: [0; N],
        bytes: [0; N],
    });
    cache.capture_growing(&kit::memory(&bytes), context(), &observer, None);
    assert_eq!(cache.result.outcome, Outcome::Rows { len: 40 });
    assert_eq!(cache.modifiers.role_counts.read, 40);
    cache.decode(&mut captured);
    let expected = captured.advisors_table();
    captured.reset();
    cache.prepare();
    let watched = || vec![numeric..numeric + 16, tooltip..tooltip + 0x28];
    let quiet = Counting::new(&bytes, watched());
    cache.capture_growing(&quiet, context(), &observer, None);
    assert_eq!(quiet.reads(), 0);
    assert_eq!(
        (
            cache.modifiers.role_counts.reused,
            cache.modifiers.role_counts.read
        ),
        (40, 0)
    );
    cache.decode(&mut captured);
    assert_eq!(captured.advisors_table(), expected);
    captured.reset();
    drop(WATCHES.notify(
        Some(ADDRESS + first_character),
        engine_mutations::character_role_mutations::ADVISOR_MODIFIERS,
    ));
    let rebuilt = Counting::new(&bytes, watched());
    cache.capture_growing(&rebuilt, context(), &observer, None);
    assert!(rebuilt.reads() > 0, "the rebuilt role is read");
    assert_eq!(
        (
            cache.modifiers.role_counts.reused,
            cache.modifiers.role_counts.read
        ),
        (39, 1)
    );
    cache.decode(&mut captured);
    assert_eq!(captured.advisors_table(), expected);
}

#[cfg(feature = "advisor_profile")]
#[test]
fn profile_diagnostics_have_five_slots_independent_of_table_count() {
    let section = Section::with_capacity(0, 0, 0);
    let memory = engine_native::memory::SliceMemory {
        base: 0,
        bytes: &[],
    };
    let counted = Counted::new(&memory);
    let _: [u64; 5] = section.diagnostic;
    let _: [u64; 5] = counted.plan.get();
    assert_eq!(DIAGNOSTIC_SLOTS, 5);
}
