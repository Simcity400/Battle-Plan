use super::*;
use std::cell::Cell;
use engine_native_memory::memory::SliceMemory;
use engine_native_parts::modifier_accumulator;

const BASE: usize = 0x1000;
const ENTRIES: usize = 0x1100;

struct Source {
    base: usize,
    bytes: RefCell<Vec<u8>>,
    reads: RefCell<Vec<(usize, usize)>>,
    fault: Cell<Option<usize>>,
    scalars: Cell<usize>,
    scatters: Cell<usize>,
    records: Cell<usize>,
}
impl Source {
    fn from_bytes(base: usize, bytes: Vec<u8>) -> Self {
        Self {
            base,
            bytes: RefCell::new(bytes),
            reads: RefCell::new(Vec::new()),
            fault: Cell::new(None),
            scalars: Cell::new(0),
            scatters: Cell::new(0),
            records: Cell::new(0),
        }
    }
    fn new(count: u32) -> Self {
        let source = Self::from_bytes(BASE, vec![0; 0x400 + count as usize * 16]);
        source.array(BASE, ENTRIES, count);
        for index in 0..count {
            source.put(ENTRIES + index as usize * 16, &(index + 1).to_le_bytes());
            source.put(
                ENTRIES + index as usize * 16 + 8,
                &(i64::from(index) * -13).to_le_bytes(),
            );
        }
        source
    }
    fn put(&self, address: usize, value: &[u8]) {
        let offset = address - self.base;
        self.bytes.borrow_mut()[offset..offset + value.len()].copy_from_slice(value);
    }
    fn array(&self, address: usize, data: usize, count: u32) {
        self.put(address, &(data as u64).to_le_bytes());
        self.put(address + 8, &count.to_le_bytes());
        self.put(address + 12, &count.to_le_bytes());
    }
}

struct Scalar<'a, M>(&'a M);
impl<M: Memory> Memory for Scalar<'_, M> {
    fn copy(&self, address: usize, out: &mut [u8]) -> bool {
        self.0.copy(address, out)
    }
}
impl Memory for Source {
    fn read_scalar<T: engine_native_memory::memory::Raw>(&self, address: usize) -> Option<T> {
        self.scalars.set(self.scalars.get() + 1);
        Scalar(self).read_scalar::<T>(address)
    }

    fn copy(&self, address: usize, out: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, out.len()));
        if self.fault.get() == Some(address) {
            for byte in out.iter_mut().take(2) {
                *byte = 0xe1;
            }
            return false;
        }
        SliceMemory {
            base: self.base,
            bytes: &self.bytes.borrow(),
        }
        .copy(address, out)
    }
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.scatters.set(self.scatters.get() + 1);
        Scalar(self).copy_scattered(addresses, destinations)
    }
    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        self.records.set(self.records.get() + 1);
        Scalar(self).copy_records(sources, layout, destination)
    }
}

fn capacities(nodes: usize, entries: usize) -> Capacities {
    Capacities {
        rows: 1,
        modifiers: SnapshotCapacity { nodes, entries },
    }
}

fn model_source() -> (Source, cship::Row) {
    use engine_native::{
        cequipment_variant::{self, pool as cequipment_variant_pool},
        ctask_force,
        cunit,
    };
    use engine_native_memory::msvc_string;
    let source = Source::from_bytes(BASE, vec![0; 0x5000]);
    let (parent, variant, bank) = (0x2000, 0x3000, 0x4000);
    let ship = (42_u64 << 32) | u64::from(cship::HANDLE_TYPE);
    let task_force = (17_u64 << 32) | u64::from(ctask_force::DEFAULT_HANDLE_TYPE);
    source.put(BASE, &(cship::VTABLE_RVA.0 as u64).to_le_bytes());
    source.put(BASE + cship::HANDLE, &ship.to_le_bytes());
    source.put(BASE + cship::TASK_FORCE, &(parent as u64).to_le_bytes());
    source.put(parent, &(ctask_force::VTABLE_RVA.0 as u64).to_le_bytes());
    source.put(parent + cunit::HANDLE, &task_force.to_le_bytes());
    source.put(
        BASE + cship::EQUIPMENT_POOL,
        &(cequipment_variant_pool::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    source.array(
        BASE + cship::EQUIPMENT_POOL + cequipment_variant_pool::ITEMS,
        bank,
        1,
    );
    source.put(bank, &(variant as u64).to_le_bytes());
    source.put(bank + 8, &i64::MIN.to_le_bytes());
    source.put(
        variant,
        &(engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    source.put(
        variant + 8,
        &((1_u64 << 32) | u64::from(cequipment_variant::REFERENCE_TYPE)).to_le_bytes(),
    );
    let len = cship::MODEL_TEXT_CAPACITY as u64 + 1;
    source.put(
        BASE + cship::FALLBACK_SPRITE,
        &(variant as u64).to_le_bytes(),
    );
    source.put(
        BASE + cship::FALLBACK_SPRITE + msvc_string::LENGTH,
        &len.to_le_bytes(),
    );
    source.put(
        BASE + cship::FALLBACK_SPRITE + msvc_string::CAPACITY,
        &len.to_le_bytes(),
    );
    (
        source,
        cship::Row {
            id: ship,
            task_force,
            ..Default::default()
        },
    )
}

fn acquire(capture: &mut Captured, source: &Source) -> Result<(), &'static str> {
    capture.begin_details();
    let result = {
        let memory = ModifierMemory {
            inner: source,
            arena: &capture.modifiers,
            held_task: None,
        };
        modifier_accumulator::lookup_at(&memory, BASE, 1)
    };
    match result {
        Ok(value) => {
            capture.rows.rows[0].maximum_organisation = value.raw;
            capture.len = 1;
        }
        Err(reason) => capture.failure = Some(reason),
    }
    capture.finish_details(Outcome::Rows { len: 1 });
    result.map(|_| ())
}

#[test]
fn held_task_modifiers_borrow_without_copying_and_preserve_current_values_and_refusals() {
    let source = Source::new(1);
    let capture = Captured::with_capacities(&capacities(2, 4));
    let mut entries = [[0_u8; 16]];
    entries[0][..4].copy_from_slice(&1_u32.to_le_bytes());
    entries[0][8..].copy_from_slice(&117_i64.to_le_bytes());
    let memory = ModifierMemory {
        inner: &source,
        arena: &capture.modifiers,
        held_task: Some((BASE, &entries)),
    };
    assert_eq!(
        modifier_accumulator::lookup_at(&memory, BASE, 1)
            .unwrap()
            .raw,
        117
    );
    assert!(source.reads.borrow().is_empty());
    assert!(capture.modifiers.borrow().observed(BASE).is_none());

    source.put(ENTRIES + 8, &42_i64.to_le_bytes());
    capture
        .modifiers
        .borrow_mut()
        .observe(&source, BASE)
        .unwrap();
    let before = source.reads.borrow().len();
    assert_eq!(
        modifier_accumulator::lookup_at(&memory, BASE, 1)
            .unwrap()
            .raw,
        42
    );
    assert_eq!(source.reads.borrow().len(), before);

    capture.modifiers.borrow_mut().clear();
    source.fault.set(Some(BASE));
    assert!(
        capture
            .modifiers
            .borrow_mut()
            .observe(&source, BASE)
            .is_none()
    );
    let before = source.reads.borrow().len();
    assert_eq!(
        modifier_accumulator::lookup_at(&memory, BASE, 1),
        Err("modifier accumulator header unreadable or invalid")
    );
    assert_eq!(source.reads.borrow().len(), before);
}

#[test]
fn modifier_memo_reuses_current_bytes_and_release_performs_no_reads() {
    let source = Source::new(2);
    let mut capture = Captured::with_capacities(&capacities(2, 4));
    let allocated = capture.allocated_bytes();
    acquire(&mut capture, &source).unwrap();
    let before = source.reads.borrow().len();
    let observation = capture
        .modifiers
        .borrow_mut()
        .capture_at(&source, BASE, 2)
        .unwrap();
    assert_eq!(source.reads.borrow().len(), before);
    assert_eq!(
        capture.modifiers.borrow().value(observation).unwrap().raw,
        -13
    );
    capture.release_sources();
    assert_eq!(source.reads.borrow().len(), before);
    assert_eq!(
        capture.required_modifiers,
        SnapshotCapacity {
            nodes: 1,
            entries: 2
        }
    );
    assert_eq!(
        capture.modifiers.borrow().value(observation),
        Err("modifier snapshot observation expired")
    );
    assert_eq!(
        capture.modifiers.borrow().required_capacity(),
        SnapshotCapacity::default()
    );
    assert_eq!(capture.allocated_bytes(), allocated);
}

#[test]
fn failed_snapshot_prefix_keeps_first_fault_and_releases_without_retry() {
    let source = Source::new(70);
    let failed = ENTRIES + 66 * 16;
    source.fault.set(Some(failed));
    let mut capture = Captured::with_capacities(&capacities(2, 72));
    assert_eq!(
        acquire(&mut capture, &source),
        Err("modifier accumulator entry unreadable")
    );
    let before = source.reads.borrow().len();
    assert_eq!(
        source
            .reads
            .borrow()
            .iter()
            .filter(|&&(p, _)| p == failed)
            .count(),
        1
    );
    capture.release_sources();
    assert_eq!(source.reads.borrow().len(), before);
    assert_eq!(
        capture.failure,
        Some("modifier accumulator entry unreadable")
    );
    assert_eq!(
        capture.outcomes(),
        [Outcome::Unavailable {
            reason: "modifier accumulator entry unreadable"
        }]
    );
    assert_eq!(capture.required_modifiers.entries, 66);
}

#[test]
fn an_empty_modifier_arena_grows_in_the_pass_and_keeps_current_values() {
    let source = Source::new(3);
    let mut capture = Captured::with_capacities(&capacities(0, 0));
    acquire(&mut capture, &source).unwrap();
    assert_eq!(capture.rows.rows[0].maximum_organisation, 0);
    let grown = capture.modifiers.borrow().capacity();
    assert!(grown.nodes >= 1 && grown.entries >= 3);
    let reads = source.reads.borrow().len();
    capture.release_sources();
    assert_eq!(source.reads.borrow().len(), reads);
    assert_eq!(
        capture.required_modifiers,
        SnapshotCapacity {
            nodes: 1,
            entries: 3,
        }
    );
    assert_eq!(
        capture.allocated_bytes(),
        capture.rows.allocated_bytes()
            + capture.seeds.capacity() * std::mem::size_of::<Seed>()
            + capture.modifiers.borrow().allocated_bytes()
            + capture
                .extra
                .iter()
                .map(Captured::allocated_bytes)
                .sum::<usize>()
    );
}

#[test]
fn cache_demand_past_any_fixed_size_presizes_to_the_demand() {
    let required = SnapshotCapacity {
        nodes: 1_000_001,
        entries: 4_194_305,
    };
    let mut capacity = SnapshotCapacity {
        nodes: 1,
        entries: 1,
    };
    grow_snapshot_capacity(&mut capacity, required);
    assert!(capacity.nodes >= required.nodes);
    assert!(capacity.entries >= required.entries);
    let grown = capacity;
    grow_snapshot_capacity(&mut capacity, SnapshotCapacity::default());
    assert_eq!(capacity, grown);
}

#[test]
fn only_current_country_authentication_is_forwarded_for_matching_context() {
    const STATE: usize = 0x1000;
    let source = Source::from_bytes(0, vec![0; 0x8000]);
    source.array(STATE + engine_native::cgame_state::COUNTRIES, 0x5000, 2);
    source.array(STATE + engine_native::cgame_state::COUNTRY_LINKS, 0x5100, 2);
    let mut cache = engine_native_intelligence::country_memory::Cache::try_with_capacity(64).unwrap();
    let cached = cache.prepare(&source, 0, STATE);
    assert_eq!(
        cached.seed_status(),
        engine_native_intelligence::country_memory::SeedStatus::Ready
    );
    let arena = RefCell::new(SnapshotArena::try_new(SnapshotCapacity::default()).unwrap());
    let memory = ModifierMemory {
        inner: &cached,
        arena: &arena,
        held_task: None,
    };
    let reads = source.reads.borrow().len();
    assert!(memory.authenticated_countries(0, STATE).is_some());
    assert!(memory.authenticated_countries(1, STATE).is_none());
    assert!(memory.authenticated_countries(0, STATE + 1).is_none());
    assert_eq!(
        source.reads.borrow().len(),
        reads,
        "forwarding supplies no new observations"
    );
}

struct OptionalModelSource(Source);
impl OptionalModelSource {
    fn new() -> (Self, cship::Row) {
        use engine_native_parts::{cequipment_database, cequipment_type};
        use engine_native::cequipment_variant;
        use engine_native_memory::msvc_string;
        let (source, row) = model_source();
        let (variant, definition, database, list) = (0x3000, 0x5000, 0x5700, 0x5800);
        source.put(BASE + cship::FALLBACK_SPRITE, &[0; 32]);
        source.put(
            BASE + cship::FALLBACK_SPRITE + msvc_string::CAPACITY,
            &15_u64.to_le_bytes(),
        );
        source.put(
            variant + cequipment_variant::EQUIPMENT_TOKEN,
            &123_u32.to_le_bytes(),
        );
        source.put(
            variant + cequipment_variant::DEFINITION,
            &(definition as u64).to_le_bytes(),
        );
        source.put(
            definition,
            &(cequipment_type::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        source.put(definition + cequipment_type::TOKEN, &123_u32.to_le_bytes());
        source.put(
            definition + cequipment_type::HULL_SPRITE + msvc_string::CAPACITY,
            &15_u64.to_le_bytes(),
        );
        source.put(definition + 0x478, &1_i32.to_le_bytes());
        source.put(
            database,
            &(cequipment_database::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        source.array(database + 0x68, list, 2);
        source.put(list + 8, &(definition as u64).to_le_bytes());
        source.put(
            BASE + cship::SUBUNIT_DEFINITION,
            &(engine_native_parts::csub_unit_definition::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        source.put(BASE + cship::SUBUNIT_INDEX, &i32::MIN.to_le_bytes());
        for (offset, value) in [
            (0xc4, -1_i32),
            (0x414, i32::MIN),
            (0x418, i32::MAX),
            (0x4b0, 23),
        ] {
            source.put(variant + offset, &value.to_le_bytes());
        }
        source.put(variant + cequipment_variant::VERSION, &[u8::MAX]);
        (Self(source), row)
    }
}
impl Memory for OptionalModelSource {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if address == engine_native_parts::cequipment_database::SINGLETON_RVA.0 {
            self.0.reads.borrow_mut().push((address, output.len()));
            if self.0.fault.get() == Some(address) {
                return false;
            }
            assert_eq!(output.len(), 8);
            output.copy_from_slice(&0x5700_u64.to_le_bytes());
            true
        } else {
            self.0.copy(address, output)
        }
    }
}

fn optional_hull_expected() -> cship::HullVariantInputs {
    cship::HullVariantInputs {
        definition_index: Some(1),
        module_count: Some(-1),
        upgrade_level_sum: Some(i32::MIN),
        module_priority_sum: Some(i32::MAX),
        version: Some(u8::MAX),
        organisation_marker: Some(23),
    }
}

#[test]
fn optional_subunit_refusals_are_named_without_replacing_healthy_model_operands() {
    struct RelocatedHeader<'a> {
        inner: &'a OptionalModelSource,
        pointer: usize,
    }
    impl Memory for RelocatedHeader<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if self.pointer.checked_add(cship::SUBUNIT_DEFINITION) == Some(address) {
                self.inner.copy(BASE + cship::SUBUNIT_DEFINITION, output)
            } else {
                self.inner.copy(address, output)
            }
        }
    }
    for index in 0..6 {
        let (source, census) = OptionalModelSource::new();
        let mut capture = Captured::part(&capacities(0, 0));
        capture.begin_details();
        let (mut row, _) =
            cship::read_model_with_selection(&source, 0, BASE, &census, &mut capture.model)
                .unwrap();
        row.maximum_organisation = -701;
        let healthy = row.clone();
        let (mut base, mut pointer) = (0, BASE);
        match index {
            0 => pointer = 0,
            1 => source.0.fault.set(Some(BASE + cship::SUBUNIT_DEFINITION)),
            2 => base = usize::MAX,
            3 => source
                .0
                .put(BASE + cship::SUBUNIT_DEFINITION, &0_u64.to_le_bytes()),
            4 => pointer = usize::MAX - cship::SUBUNIT_INDEX + 1,
            5 => source.0.fault.set(Some(BASE + cship::SUBUNIT_INDEX)),
            _ => unreachable!(),
        }
        let memory = RelocatedHeader {
            inner: &source,
            pointer,
        };
        capture.capture_model_suffixes(&memory, base, pointer, &census, None, &mut row);
        assert_eq!(row, healthy, "only optional subunit index is refused");
        let mut wanted = [0; OPTIONAL_REFUSAL_REASONS.len()];
        wanted[index] = 1;
        assert_eq!(
            capture.optional_refusals, wanted,
            "{}",
            OPTIONAL_REFUSAL_REASONS[index]
        );
        capture.release_sources();
        assert_eq!(
            capture.optional_refusals,
            [0; OPTIONAL_REFUSAL_REASONS.len()]
        );
        capture.release_sources();
        assert_eq!(
            capture.optional_refusals,
            [0; OPTIONAL_REFUSAL_REASONS.len()]
        );
        let status = serde_json::to_value(optional_data()).unwrap();
        assert!(
            status["optional_operands"][OPTIONAL_REFUSAL_REASONS[index]]
                .as_u64()
                .unwrap()
                > 0
        );
        assert!(status.get("torpedo_cooldown_modifier_refused").is_none());
    }
}

#[test]
fn current_hull_packet_and_field_refusals_are_named_and_preserve_independent_operands() {
    let (healthy, census) = OptionalModelSource::new();
    let mut capture = Captured::part(&capacities(0, 0));
    capture.begin_details();
    let (mut row, selected) =
        cship::read_model_with_selection(&healthy, 0, BASE, &census, &mut capture.model).unwrap();
    capture.capture_model_suffixes(&healthy, 0, BASE, &census, selected.as_ref(), &mut row);
    assert_eq!(row.subunit_index, Some(i32::MIN));
    assert_eq!(row.hull_variant, Some(optional_hull_expected()));
    assert_eq!(row.naval_power, None);
    let mut wanted = [0; OPTIONAL_REFUSAL_REASONS.len()];
    wanted[14] = 1;
    assert_eq!(capture.optional_refusals, wanted);
    for (index, address) in [
        (6, 0x3000),
        (
            6,
            0x3000 + engine_native::cequipment_variant::EQUIPMENT_TOKEN,
        ),
        (6, 0x3000 + engine_native::cequipment_variant::DEFINITION),
        (7, 0x5000),
        (7, 0x5000 + 0x478),
        (7, engine_native_parts::cequipment_database::SINGLETON_RVA.0),
        (7, 0x5700),
        (7, 0x5700 + 0x68),
        (7, 0x5800 + 8),
        (8, 0x3000 + 0xc4),
        (9, 0x3000 + 0x414),
        (10, 0x3000 + 0x418),
        (11, 0x3000 + engine_native::cequipment_variant::VERSION),
        (12, 0x3000 + 0x4b0),
    ] {
        let (source, census) = OptionalModelSource::new();
        let mut capture = Captured::part(&capacities(0, 0));
        capture.begin_details();
        let (mut row, selected) =
            cship::read_model_with_selection(&source, 0, BASE, &census, &mut capture.model)
                .unwrap();
        row.maximum_organisation = -701;
        let healthy = row.clone();
        source.0.fault.set(Some(address));
        capture.capture_model_suffixes(&source, 0, BASE, &census, selected.as_ref(), &mut row);
        let mut wanted = [0; OPTIONAL_REFUSAL_REASONS.len()];
        wanted[index] = 1;
        wanted[14] = 1;
        assert_eq!(
            capture.optional_refusals, wanted,
            "{} at {address:#x}",
            OPTIONAL_REFUSAL_REASONS[index]
        );
        assert_eq!(row.maximum_organisation, healthy.maximum_organisation);
        assert_eq!(row.ship, healthy.ship);
        assert_eq!(row.variant, healthy.variant);
        assert_eq!(row.fallback_sprite, healthy.fallback_sprite);
        assert_eq!(row.hull_sprite, healthy.hull_sprite);
        assert_eq!(row.subunit_index, Some(i32::MIN));
        let mut packet = optional_hull_expected();
        match index {
            6 => {}
            7 => packet.definition_index = None,
            8 => packet.module_count = None,
            9 => packet.upgrade_level_sum = None,
            10 => packet.module_priority_sum = None,
            11 => packet.version = None,
            12 => packet.organisation_marker = None,
            _ => unreachable!(),
        }
        assert_eq!(row.hull_variant, (index != 6).then_some(packet));
        let reads = source.0.reads.borrow().len();
        source.0.fault.set(None);
        capture.capture_model_suffixes(&source, 0, BASE, &census, selected.as_ref(), &mut row);
        assert_eq!(
            source.0.reads.borrow().len(),
            reads + 2,
            "only the independent subunit spans repeat"
        );
        assert_eq!(row.hull_variant, (index != 6).then_some(packet));
        assert_eq!(capture.optional_refusals[index], 2);
        capture.release_sources();
        assert_eq!(
            capture.optional_refusals,
            [0; OPTIONAL_REFUSAL_REASONS.len()]
        );
    }
}

#[test]
fn absent_selected_hull_is_not_counted_and_changed_selection_is_named() {
    let (source, census) = OptionalModelSource::new();
    source.0.array(
        BASE + cship::EQUIPMENT_POOL + engine_native::cequipment_variant::pool::ITEMS,
        0,
        0,
    );
    let mut capture = Captured::part(&capacities(0, 0));
    capture.begin_details();
    let (mut row, selected) =
        cship::read_model_with_selection(&source, 0, BASE, &census, &mut capture.model).unwrap();
    assert!(selected.is_none());
    let before = source.0.reads.borrow().len();
    capture.capture_model_suffixes(&source, 0, BASE, &census, selected.as_ref(), &mut row);
    assert_eq!(source.0.reads.borrow().len(), before + 2);
    assert_eq!(row.subunit_index, Some(i32::MIN));
    assert_eq!(row.hull_variant, None);
    assert_eq!(
        capture.optional_refusals,
        [0; OPTIONAL_REFUSAL_REASONS.len()]
    );
    for (address, replacement) in [
        (0x3000, 0_u64.to_le_bytes().to_vec()),
        (
            0x3000 + 8,
            ((2_u64 << 32) | u64::from(engine_native::cequipment_variant::REFERENCE_TYPE))
                .to_le_bytes()
                .to_vec(),
        ),
        (
            0x3000 + engine_native::cequipment_variant::EQUIPMENT_TOKEN,
            124_u32.to_le_bytes().to_vec(),
        ),
        (
            0x3000 + engine_native::cequipment_variant::DEFINITION,
            0x5008_u64.to_le_bytes().to_vec(),
        ),
    ] {
        let (source, census) = OptionalModelSource::new();
        let mut capture = Captured::part(&capacities(0, 0));
        capture.begin_details();
        let (mut row, selected) =
            cship::read_model_with_selection(&source, 0, BASE, &census, &mut capture.model)
                .unwrap();
        source.0.put(address, &replacement);
        capture.capture_model_suffixes(&source, 0, BASE, &census, selected.as_ref(), &mut row);
        assert_eq!(row.subunit_index, Some(i32::MIN));
        assert_eq!(row.hull_variant, None);
        let mut wanted = [0; OPTIONAL_REFUSAL_REASONS.len()];
        wanted[6] = 1;
        wanted[14] = 1;
        assert_eq!(capture.optional_refusals, wanted);
    }
}

#[test]
fn cold_ship_model_modifier_arenas_grow_from_original_mod_sized_headers() {
    let mut capture = Captured::part(&Capacities::default());
    assert_eq!(capture.modifiers.borrow().allocated_bytes(), 0);
    for count in [5_000_u32, 15_000] {
        let source = Source::new(count);
        capture.begin_details();
        let core = ModifierMemory {
            inner: &source,
            arena: &capture.modifiers,
            held_task: None,
        };
        assert_eq!(
            modifier_accumulator::lookup_at(&core, BASE, count)
                .unwrap()
                .raw,
            i64::from(count - 1) * -13
        );
        assert!(capture.modifiers.borrow().capacity().entries >= count as usize);
        let reads = source.reads.borrow().len();
        assert_eq!(
            modifier_accumulator::lookup_at(&core, BASE, 1).unwrap().raw,
            0
        );
        assert_eq!(
            source.reads.borrow().len(),
            reads,
            "owned cache must retain each original acquisition"
        );
    }
}

mod fleet {
    use engine_native::{
        ccountry,
        cequipment_variant::{self, pool as cequipment_variant_pool},
        cgame_state,
        cship,
        ctask_force,
        cunit,
    };
    use engine_native_parts::{cequipment_database, cequipment_type, csub_unit_definition};

    pub(super) const IMAGE: usize = 0x1_4000_0000;
    pub(super) const LOW: usize = 0x1_0000;
    pub(super) const GAME: usize = LOW;
    pub(super) const SINGLETON: usize = IMAGE + cgame_state::SINGLETON_RVA.0;
    const COUNTRIES: usize = LOW + 0x1000;
    const LINKS: usize = LOW + 0x1100;
    pub(super) const COUNTRY: usize = LOW + 0x2000;
    pub(super) const TASKS: [usize; 2] = [LOW + 0x3000, LOW + 0x3800];
    const DOCTRINE: usize = LOW + 0x4000;
    const EXPERIENCE: usize = LOW + 0x4100;
    pub(super) const DATABASE: usize = LOW + 0x5000;
    const DEFINITIONS: usize = LOW + 0x5100;
    pub(super) const VARIANT: usize = LOW + 0x6000;
    pub(super) const DEFINITION: usize = LOW + 0x7000;
    pub(super) const SHIPS: usize = 12;
    const CLASS: i32 = 7;

    pub(super) fn ship(index: usize) -> usize {
        LOW + 0x8000 + index * 0x1000
    }

    fn task(index: usize) -> usize {
        usize::from(index >= 8)
    }

    fn task_handle(task: usize) -> u64 {
        ((task as u64 + 1) << 32) | u64::from(ctask_force::DEFAULT_HANDLE_TYPE)
    }

    pub(super) fn census(index: usize) -> cship::Row {
        cship::Row {
            id: ((index as u64 + 1) << 32) | u64::from(cship::HANDLE_TYPE),
            task_force: task_handle(task(index)),
            class_id: CLASS,
            roster_ordinal: index as u32,
            ..Default::default()
        }
    }

    pub(super) fn census_in(index: usize, per_task: usize) -> cship::Row {
        cship::Row {
            task_force: task_handle(index / per_task),
            ..census(index)
        }
    }

    pub(super) fn bytes_for(ships: usize) -> Vec<u8> {
        bytes_with(ships, &TASKS, task)
    }

    pub(super) fn bytes_in_tasks(ships: usize, per_task: usize) -> Vec<u8> {
        let tasks: Vec<usize> = (0..ships.div_ceil(per_task))
            .map(|task| LOW + 0x8000 + ships * 0x1000 + task * 0x800)
            .collect();
        bytes_with(ships, &tasks, |index| index / per_task)
    }

    fn bytes_with(ships: usize, tasks: &[usize], task: impl Fn(usize) -> usize) -> Vec<u8> {
        let end = tasks.iter().max().map_or(0, |&last| last + 0x800 - LOW);
        let mut bytes = vec![0; (0x8000 + ships * 0x1000).max(end)];
        let mut put = |at: usize, value: &[u8]| {
            bytes[at - LOW..at - LOW + value.len()].copy_from_slice(value);
        };
        let word = |value: usize| (value as u64).to_le_bytes();
        let inline = |text: &[u8]| {
            let mut header = [0; 32];
            header[..text.len()].copy_from_slice(text);
            header[16..24].copy_from_slice(&(text.len() as u64).to_le_bytes());
            header[24..].copy_from_slice(&15_u64.to_le_bytes());
            header
        };
        let array = |data: usize, count: u32| {
            let mut header = [0; 16];
            header[..8].copy_from_slice(&(data as u64).to_le_bytes());
            header[8..12].copy_from_slice(&count.to_le_bytes());
            header[12..].copy_from_slice(&count.to_le_bytes());
            header
        };
        put(GAME + cgame_state::COUNTRIES, &array(COUNTRIES, 2));
        put(COUNTRIES + 8, &word(COUNTRY));
        put(GAME + cgame_state::COUNTRY_LINKS, &array(LINKS, 2));
        put(LINKS + 4, &1_u32.to_le_bytes());
        put(COUNTRY, &word(IMAGE + ccountry::VTABLE_RVA.0));
        put(COUNTRY + ccountry::ID, &1_u32.to_le_bytes());
        for (index, &task) in tasks.iter().enumerate() {
            put(task, &word(IMAGE + ctask_force::VTABLE_RVA.0));
            put(task + cunit::HANDLE, &task_handle(index).to_le_bytes());
            put(task + cunit::ROSTER_OWNER, &1_u32.to_le_bytes());
            put(task + 0x640, &word(DOCTRINE));
        }
        put(DOCTRINE, &word(DOCTRINE));
        put(DOCTRINE + 0x19, &[1]);
        put(VARIANT, &word(IMAGE + engine_native_parts::equipment_variant_identity::VTABLE_RVA.0));
        put(
            VARIANT + 8,
            &((1_u64 << 32) | u64::from(cequipment_variant::REFERENCE_TYPE)).to_le_bytes(),
        );
        put(
            VARIANT + cequipment_variant::EQUIPMENT_TOKEN,
            &11_u32.to_le_bytes(),
        );
        put(VARIANT + cequipment_variant::DEFINITION, &word(DEFINITION));
        put(
            VARIANT + cequipment_variant::CLASSIFIER_FLAGS,
            &(1_u64 << 6).to_le_bytes(),
        );
        put(VARIANT + 0xc4, &3_i32.to_le_bytes());
        put(DEFINITION, &word(IMAGE + cequipment_type::VTABLE_RVA.0));
        put(DEFINITION + cequipment_type::TOKEN, &11_u32.to_le_bytes());
        put(DEFINITION + 0x478, &1_i32.to_le_bytes());
        put(DATABASE, &word(IMAGE + cequipment_database::VTABLE_RVA.0));
        put(DATABASE + 0x68, &array(DEFINITIONS, 2));
        put(DEFINITIONS + 8, &word(DEFINITION));
        put(DEFINITION + cequipment_type::HULL_SPRITE, &inline(b"hull"));
        for index in 0..ships {
            let ship = ship(index);
            let bank = ship + 0x980;
            put(ship, &word(IMAGE + cship::VTABLE_RVA.0));
            put(ship + cship::HANDLE, &census(index).id.to_le_bytes());
            put(ship + cship::TASK_FORCE, &word(tasks[task(index)]));
            put(
                ship + cship::EQUIPMENT_POOL,
                &word(IMAGE + cequipment_variant_pool::VTABLE_RVA.0),
            );
            put(
                ship + cship::EQUIPMENT_POOL + cequipment_variant_pool::ITEMS,
                &array(bank, 1),
            );
            put(bank, &word(VARIANT));
            put(bank + 8, &5_i64.to_le_bytes());
            put(
                ship + cship::SUBUNIT_DEFINITION,
                &word(IMAGE + csub_unit_definition::VTABLE_RVA.0),
            );
            put(ship + cship::CLASS_DATABASE_ID, &CLASS.to_le_bytes());
            put(ship + cship::DEFINITION_VALID, &[1]);
            put(ship + cship::FALLBACK_SPRITE, &inline(b"ship"));
            put(
                ship + cship::ANTI_AIR_BASE,
                &(1000 + index as i64).to_le_bytes(),
            );
            put(
                ship + cship::BASE_ORGANISATION,
                &(10_000_000 + index as i64 * 100_000).to_le_bytes(),
            );
            put(ship + cship::MAXIMUM_STRENGTH, &5_000_000_i64.to_le_bytes());
            put(ship + cship::SUBUNIT_INDEX, &(index as i32).to_le_bytes());
            put(ship + cship::MODEL_NUMBER, &(index as i32).to_le_bytes());
            put(ship + 0x8f8, &word(EXPERIENCE));
        }
        bytes
    }
}

struct Fleet {
    low: Vec<u8>,
    calls: Cell<usize>,
    watched: Cell<usize>,
    hits: Cell<usize>,
}

impl Fleet {
    fn new() -> Self {
        Self::with_ships(fleet::SHIPS)
    }

    fn with_ships(ships: usize) -> Self {
        Self {
            low: fleet::bytes_for(ships),
            calls: Cell::new(0),
            watched: Cell::new(0),
            hits: Cell::new(0),
        }
    }

    fn in_tasks(ships: usize, per_task: usize) -> Self {
        Self {
            low: fleet::bytes_in_tasks(ships, per_task),
            calls: Cell::new(0),
            watched: Cell::new(0),
            hits: Cell::new(0),
        }
    }

    fn read(&self, address: usize, output: &mut [u8]) -> bool {
        if address == self.watched.get() {
            self.hits.set(self.hits.get() + 1);
        }
        if address == fleet::SINGLETON && output.len() == 8 {
            output.copy_from_slice(&(fleet::GAME as u64).to_le_bytes());
            return true;
        }
        if address == fleet::IMAGE + engine_native_parts::cequipment_database::SINGLETON_RVA.0
            && output.len() == 8
        {
            output.copy_from_slice(&(fleet::DATABASE as u64).to_le_bytes());
            return true;
        }
        SliceMemory {
            base: fleet::LOW,
            bytes: &self.low,
        }
        .copy(address, output)
    }
}

impl Memory for Fleet {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.calls.set(self.calls.get() + 1);
        self.read(address, output)
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.calls.set(self.calls.get() + 1);
        for (completed, (address, destination)) in addresses
            .into_iter()
            .zip(destinations.iter_mut())
            .enumerate()
        {
            if !destination.is_empty() && !self.read(address, destination) {
                return completed;
            }
        }
        N
    }

    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        let calls = self.calls.get() + 1;
        let completed = Scalar(self).copy_records(sources, layout, destination);
        self.calls.set(calls);
        completed
    }
}

#[test]
fn ship_model_details_equal_the_direct_readers_in_under_half_their_guarded_calls() {
    let world = Fleet::new();
    let mut capture = Captured::with_capacities(&Capacities {
        rows: fleet::SHIPS,
        modifiers: SnapshotCapacity {
            nodes: 64,
            entries: 64,
        },
    });
    capture.begin_sources();
    for index in 0..fleet::SHIPS {
        capture.visit_source(fleet::ship(index), &fleet::census(index));
    }
    capture.finish_sources(Outcome::Rows { len: fleet::SHIPS });
    capture.capture_owned(&world, CaptureContext::new(fleet::IMAGE));
    assert_eq!(capture.outcomes(), [Outcome::Rows { len: fleet::SHIPS }]);
    let gathered = world.calls.replace(0);
    let mut direct = Captured::part(&capacities(64, 64));
    direct.begin_details();
    let mut rows = Vec::new();
    for index in 0..fleet::SHIPS {
        let ship = fleet::ship(index);
        let census = fleet::census(index);
        let (mut row, selected) = {
            let memory = ModifierMemory {
                inner: &world,
                arena: &direct.modifiers,
                held_task: None,
            };
            let (mut row, selected) = cship::read_model_with_selection(
                &memory,
                fleet::IMAGE,
                ship,
                &census,
                &mut direct.model,
            )
            .unwrap();
            row.maximum_organisation = cship::read_maximum_organisation_memo(
                &memory,
                fleet::IMAGE,
                ship,
                &mut direct.doctrine,
            )
            .unwrap();
            row.has_criticals = direct
                .doctrine
                .organisation_operands(ship)
                .map(|own| u32::from(own.has_criticals()));
            (row, selected)
        };
        direct.capture_model_suffixes(
            &world,
            fleet::IMAGE,
            ship,
            &census,
            selected.as_ref(),
            &mut row,
        );
        rows.push(row);
    }
    let read = world.calls.get();
    assert_eq!(capture.rows.rows[..fleet::SHIPS], rows[..]);
    assert_eq!(rows[11].maximum_organisation, 11_100_000);
    assert_eq!(rows[2].subunit_index, Some(2));
    assert!(
        gathered * 2 < read,
        "{gathered} guarded calls through gathered fields, {read} directly"
    );
}

#[test]
fn held_ships_are_carried_until_a_writer_touches_them() {
    use crate::ship_model_changes::Watches;
    let watches = Watches::enabled(64);
    let mut world = Fleet::new();
    let mut capture = Captured::with_capacities(&Capacities {
        rows: fleet::SHIPS,
        modifiers: SnapshotCapacity {
            nodes: 64,
            entries: 64,
        },
    });
    let hour = |capture: &mut Captured, world: &Fleet| {
        capture.begin_sources();
        for index in 0..fleet::SHIPS {
            capture.visit_source(fleet::ship(index), &fleet::census(index));
        }
        capture.finish_sources(Outcome::Rows { len: fleet::SHIPS });
        world.calls.set(0);
        capture.capture_carrying(world, CaptureContext::new(fleet::IMAGE), &watches);
        assert_eq!(capture.outcomes(), [Outcome::Rows { len: fleet::SHIPS }]);
        (
            capture.rows.rows[..fleet::SHIPS].to_vec(),
            world.calls.get(),
        )
    };
    let base_organisation = |world: &mut Fleet, index: usize, value: i64| {
        let at = fleet::ship(index) + cship::BASE_ORGANISATION - fleet::LOW;
        world.low[at..at + 8].copy_from_slice(&value.to_le_bytes());
    };
    let (fresh, read) = hour(&mut capture, &world);
    let (carried, served) = hour(&mut capture, &world);
    assert_eq!(carried, fresh);
    assert!(
        read >= served + 2 * fleet::SHIPS,
        "{served} guarded calls carried, {read} read"
    );

    drop(watches.notify(fleet::TASKS[0], 6));
    let (rows, rewalked) = hour(&mut capture, &world);
    assert_eq!(rows, fresh);
    assert!(
        rewalked >= served + 4,
        "{rewalked} guarded calls after the doctrine writer, {served} quiet"
    );

    base_organisation(&mut world, 3, 20_000_000);
    drop(watches.notify(fleet::ship(3), 0));
    let (rows, _) = hour(&mut capture, &world);
    assert_ne!(rows[3].maximum_organisation, fresh[3].maximum_organisation);
    assert_eq!(rows[..3], fresh[..3]);

    base_organisation(&mut world, 4, 30_000_000);
    let (carried, _) = hour(&mut capture, &world);
    assert_eq!(carried[4], fresh[4], "no writer was observed");
    assert!(!watches.poisoned());
}

#[test]
fn held_variants_are_read_only_after_a_design_writer() {
    use crate::ship_model_changes::Watches;
    let watches = Watches::enabled(64);
    let world = Fleet::new();
    let mut capture = Captured::with_capacities(&Capacities {
        rows: fleet::SHIPS,
        modifiers: SnapshotCapacity {
            nodes: 64,
            entries: 64,
        },
    });
    let hour = |capture: &mut Captured| {
        capture.begin_sources();
        for index in 0..fleet::SHIPS {
            capture.visit_source(fleet::ship(index), &fleet::census(index));
        }
        capture.finish_sources(Outcome::Rows { len: fleet::SHIPS });
        world.hits.set(0);
        capture.capture_carrying(&world, CaptureContext::new(fleet::IMAGE), &watches);
        assert_eq!(capture.outcomes(), [Outcome::Rows { len: fleet::SHIPS }]);
        (capture.rows.rows[..fleet::SHIPS].to_vec(), world.hits.get())
    };
    world.watched.set(fleet::VARIANT);
    let (fresh, read) = hour(&mut capture);
    assert!(read > 0);
    let (carried, quiet) = hour(&mut capture);
    assert_eq!(carried, fresh);
    assert_eq!(quiet, 0, "a certified variant is not gathered");
    drop(watches.notify_variant(fleet::VARIANT));
    let (rows, touched) = hour(&mut capture);
    assert_eq!(rows, fresh);
    assert_eq!(touched, 1, "the touched variant is gathered once");
    assert_eq!(hour(&mut capture).1, 0, "and certified again");
    drop(watches.notify_variant(fleet::VARIANT + 0x1000));
    assert_eq!(hour(&mut capture).1, 0, "another variant's writer");
    watches.boundary(true);
    let (rows, invalidated) = hour(&mut capture);
    assert_eq!(rows, fresh);
    assert!(invalidated > 0, "an invalidated hour reads afresh");
    watches.disable_variants();
    for _ in 0..2 {
        let (rows, unobserved) = hour(&mut capture);
        assert_eq!(rows, fresh);
        assert!(
            unobserved > 0,
            "an unobserved design domain certifies nothing"
        );
    }
    assert!(!watches.poisoned());
}

#[test]
fn held_fleet_terms_follow_the_leader_binding_and_tree_certificates() {
    use crate::ship_model_changes::Watches;
    const TASK: usize = 0x10_0000;
    const FLEET: usize = 0x20_0000;
    const LEADER: usize = 0x30_0000;
    let watches = Watches::enabled(64);
    let world = Fleet::new();
    let task = cship::TaskForceFields {
        pointer: TASK,
        vtable: 0,
        handle: 7,
        roster_owner: 1,
        location: 0,
        fleet: FLEET,
        mine_exposure: 0,
        modifiers: TASK + 0x668,
        influences_count: Some(0),
    };
    let certificate = || {
        (
            watches.sequence(FLEET).unwrap(),
            Some(watches.tree_sequence(LEADER).unwrap()),
        )
    };
    let served = |task: cship::TaskForceFields, certificate: FleetCertificate| {
        let mut capture = Captured::part(&capacities(4, 4));
        capture.store.tasks.push(HeldTask {
            pointer: TASK,
            handle: 7,
            roster_owner: 1,
            country: fleet::COUNTRY,
            country_terms: None,
            doctrine: None,
            mine_exposure: 0,
            modifiers: None,
            weather: None,
            location: None,
            fleet: Some(HeldFleet {
                fleet: FLEET,
                leader: Some(LEADER),
                terms: [1, 2, 3, 4],
                certificate,
            }),
            terms: None,
        });
        capture.begin_details();
        let mut carrying = Carrying {
            store: true,
            changes: &watches,
            counts: Counts::default(),
            task: None,
            task_modifiers: None,
            terms: None,
            terms_identity: 0,
            quiet: None,
            variant_quiet: None,
            guard_fallback: false,
            revalidated: false,
        };
        capture.begin_task(&world, task, &mut carrying);
        capture.doctrine.fleet_source(task.fleet)
    };
    let held = certificate();
    assert_eq!(served(task, held), Some((Some(LEADER), [1, 2, 3, 4])));
    let other = cship::TaskForceFields {
        fleet: FLEET + 0x1000,
        ..task
    };
    assert_eq!(served(other, held), None, "another fleet");
    drop(watches.notify(FLEET, 8));
    assert_eq!(served(task, held), None, "the leader binding was written");
    let held = certificate();
    assert!(served(task, held).is_some());
    drop(watches.notify(LEADER, 0));
    assert_eq!(served(task, held), None, "the leader's tree was written");
    assert_eq!(world.calls.get(), 0, "no fleet or leader byte is read");
}

#[test]
fn pooled_frames_share_the_latest_held_reads() {
    use crate::ship_model_changes::Watches;
    struct Shared<'a>(&'a Watches);
    impl Changes for Shared<'_> {
        fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
            self.0.sequence(receiver)
        }
        fn sequence_at(&self, receiver: usize, slot: usize) -> Option<((u64, u64), usize)> {
            self.0.sequence_at(receiver, slot)
        }
        fn quiet(&self) -> Option<(u64, u64)> {
            self.0.quiet()
        }
        fn certificate_at(
            &self,
            receivers: [usize; 3],
            slots: Slots,
        ) -> Option<(Certificate, Slots)> {
            self.0.certificate_at(receivers, slots)
        }
        fn tree_sequence(&self, receiver: usize) -> Option<(u64, u64)> {
            self.0.tree_sequence(receiver)
        }
        fn variant_sequence(&self, variant: usize) -> Option<(u64, u64)> {
            self.0.variant_sequence(variant)
        }
        fn variant_quiet(&self) -> Option<(u64, u64)> {
            self.0.variant_quiet()
        }
        fn shared_store(&self) -> bool {
            true
        }
    }
    let watches = Watches::enabled(64);
    let changes = Shared(&watches);
    let world = Fleet::new();
    let frame = || {
        Captured::with_capacities(&Capacities {
            rows: fleet::SHIPS,
            modifiers: SnapshotCapacity {
                nodes: 64,
                entries: 64,
            },
        })
    };
    let mut frames = [frame(), frame()];
    let hour = |capture: &mut Captured| {
        capture.begin_sources();
        for index in 0..fleet::SHIPS {
            capture.visit_source(fleet::ship(index), &fleet::census(index));
        }
        capture.finish_sources(Outcome::Rows { len: fleet::SHIPS });
        world.calls.set(0);
        capture.capture_carrying(&world, CaptureContext::new(fleet::IMAGE), &changes);
        assert_eq!(capture.outcomes(), [Outcome::Rows { len: fleet::SHIPS }]);
        (
            capture.rows.rows[..fleet::SHIPS].to_vec(),
            world.calls.get(),
        )
    };
    let (fresh, read) = hour(&mut frames[0]);
    assert!(frames[0].store.held.is_empty(), "held between captures");
    for index in 1..5 {
        let (rows, served) = hour(&mut frames[index % 2]);
        assert_eq!(rows, fresh);
        assert!(
            read >= served + 2 * fleet::SHIPS,
            "hour {index}: {served} guarded calls carried, {read} read"
        );
    }
    frames[0].forget_stores();
    let (rows, reread) = hour(&mut frames[1]);
    assert_eq!(rows, fresh);
    assert_eq!(reread, read, "a forgotten store reads every ship again");
}

#[test]
fn deferred_ship_store_publication_preserves_carry_and_discards_unfinished_stores() {
    use crate::ship_model_changes::Watches;
    let watches = Watches::enabled(64);
    let world = Fleet::new();
    let capacity = Capacities {
        rows: fleet::SHIPS,
        modifiers: SnapshotCapacity {
            nodes: 64,
            entries: 64,
        },
    };
    let mut synchronous = Captured::with_capacities(&capacity);
    let mut deferred = Captured::with_capacities(&capacity);
    let seed = |capture: &mut Captured| {
        capture.begin_sources();
        for index in 0..fleet::SHIPS {
            capture.visit_source(fleet::ship(index), &fleet::census(index));
        }
        capture.finish_sources(Outcome::Rows { len: fleet::SHIPS });
    };
    for _ in 0..2 {
        seed(&mut synchronous);
        synchronous.capture_carrying(&world, CaptureContext::new(fleet::IMAGE), &watches);
        seed(&mut deferred);
        deferred.pending_owned = true;
        deferred.defer_owned = true;
        deferred.capture_carrying(&world, CaptureContext::new(fleet::IMAGE), &watches);
        deferred.defer_owned = false;
        drop(watches.notify(fleet::ship(0), 0));
        let reads = world.calls.get();
        let bytes = deferred.allocated_bytes() as u64;
        let tail_bytes = deferred.owned_tail_allocated_bytes() as u64;
        deferred.settle_owned();
        assert_eq!(
            (deferred.allocated_bytes() as u64).wrapping_sub(bytes),
            (deferred.owned_tail_allocated_bytes() as u64).wrapping_sub(tail_bytes)
        );
        assert_eq!(world.calls.get(), reads);
        assert_eq!(deferred.outcomes(), synchronous.outcomes());
        assert_eq!(deferred.tables(), synchronous.tables());
        assert_eq!(deferred.store.quiet, synchronous.store.quiet);
        assert!(
            deferred.seeds[..deferred.seed_len]
                .iter()
                .all(|seed| seed.pointer == 0)
        );
    }
    seed(&mut deferred);
    deferred.pending_owned = true;
    deferred.defer_owned = true;
    deferred.capture_carrying(&world, CaptureContext::new(fleet::IMAGE), &watches);
    deferred.reset();
    assert!(!deferred.pending_owned);
    assert!(deferred.store.held.is_empty());
    assert!(deferred.store.next.is_empty());
}

#[test]
fn a_carried_ships_model_part_is_not_read_again() {
    use crate::ship_model_changes::Watches;
    let watches = Watches::enabled(64);
    let mut world = Fleet::new();
    let mut capture = Captured::with_capacities(&Capacities {
        rows: fleet::SHIPS,
        modifiers: SnapshotCapacity {
            nodes: 64,
            entries: 64,
        },
    });
    let hour = |capture: &mut Captured, world: &Fleet| {
        capture.begin_sources();
        for index in 0..fleet::SHIPS {
            capture.visit_source(fleet::ship(index), &fleet::census(index));
        }
        capture.finish_sources(Outcome::Rows { len: fleet::SHIPS });
        capture.capture_carrying(world, CaptureContext::new(fleet::IMAGE), &watches);
        assert_eq!(capture.outcomes(), [Outcome::Rows { len: fleet::SHIPS }]);
        capture.rows.rows[..fleet::SHIPS].to_vec()
    };
    let fresh = hour(&mut capture, &world);
    assert_eq!(fresh[0].hull_sprite.as_bytes(), b"hull");
    world.low[fleet::DEFINITION + engine_native_parts::cequipment_type::HULL_SPRITE - fleet::LOW] = b'w';
    assert_eq!(hour(&mut capture, &world), fresh);
    capture.forget_stores();
    let read = hour(&mut capture, &world);
    assert_eq!(read[0].hull_sprite.as_bytes(), b"wull");
    assert_eq!(read[8].hull_sprite.as_bytes(), b"wull");
}

#[test]
fn a_touched_ship_with_unchanged_bytes_carries_again() {
    use crate::ship_model_changes::Watches;
    let watches = Watches::enabled(64);
    let mut world = Fleet::new();
    let mut capture = Captured::with_capacities(&Capacities {
        rows: fleet::SHIPS,
        modifiers: SnapshotCapacity {
            nodes: 64,
            entries: 64,
        },
    });
    let mut hour = |world: &Fleet| {
        capture.begin_sources();
        for index in 0..fleet::SHIPS {
            capture.visit_source(fleet::ship(index), &fleet::census(index));
        }
        capture.finish_sources(Outcome::Rows { len: fleet::SHIPS });
        capture.capture_carrying(world, CaptureContext::new(fleet::IMAGE), &watches);
        assert_eq!(capture.outcomes(), [Outcome::Rows { len: fleet::SHIPS }]);
        capture.rows.rows[..fleet::SHIPS].to_vec()
    };
    let fresh = hour(&world);
    world.low[fleet::DEFINITION + engine_native_parts::cequipment_type::HULL_SPRITE - fleet::LOW] = b'w';
    drop(watches.notify(fleet::ship(3), 0));
    assert_eq!(
        hour(&world),
        fresh,
        "the touched ship's bytes are unchanged"
    );
    let at = fleet::ship(3) + cship::BASE_ORGANISATION - fleet::LOW;
    world.low[at..at + 8].copy_from_slice(&20_000_000_i64.to_le_bytes());
    drop(watches.notify(fleet::ship(3), 0));
    let read = hour(&world);
    assert_ne!(read[3].maximum_organisation, fresh[3].maximum_organisation);
    assert_eq!(read[3].hull_sprite.as_bytes(), b"hull");
    assert_eq!(read[..3], fresh[..3]);
    let at = fleet::ship(3) + cship::MODEL_NUMBER - fleet::LOW;
    world.low[at..at + 4].copy_from_slice(&19_i32.to_le_bytes());
    drop(watches.notify(fleet::ship(3), 0));
    let read = hour(&world);
    assert_eq!(read[3].model_number, 19);
    assert_eq!(read[3].hull_sprite.as_bytes(), b"wull");
}

#[test]
fn a_held_row_in_the_buffer_is_not_rewritten() {
    use crate::ship_model_changes::Watches;
    let watches = Watches::enabled(64);
    let mut world = Fleet::new();
    let mut capture = Captured::with_capacities(&Capacities {
        rows: fleet::SHIPS,
        modifiers: SnapshotCapacity {
            nodes: 64,
            entries: 64,
        },
    });
    let hour = |capture: &mut Captured, world: &Fleet| {
        capture.begin_sources();
        for index in 0..fleet::SHIPS {
            capture.visit_source(fleet::ship(index), &fleet::census(index));
        }
        capture.finish_sources(Outcome::Rows { len: fleet::SHIPS });
        capture.capture_carrying(world, CaptureContext::new(fleet::IMAGE), &watches);
        assert_eq!(capture.outcomes(), [Outcome::Rows { len: fleet::SHIPS }]);
    };
    hour(&mut capture, &world);
    hour(&mut capture, &world);
    const MARK: i32 = 0x5eed;
    capture.rows.rows[5].model_number = MARK;
    hour(&mut capture, &world);
    assert_eq!(capture.rows.rows[5].model_number, MARK, "carried in place");
    let at = fleet::ship(5) + cship::BASE_ORGANISATION - fleet::LOW;
    world.low[at..at + 8].copy_from_slice(&20_000_000_i64.to_le_bytes());
    drop(watches.notify(fleet::ship(5), 0));
    hour(&mut capture, &world);
    assert_eq!(capture.rows.rows[5].model_number, 5, "read afresh");
}

#[test]
fn a_task_forces_first_ship_carries_its_row() {
    use crate::ship_model_changes::Watches;
    let watches = Watches::enabled(64);
    let world = Fleet::new();
    let mut capture = Captured::with_capacities(&Capacities {
        rows: fleet::SHIPS,
        modifiers: SnapshotCapacity {
            nodes: 64,
            entries: 64,
        },
    });
    let hour = |capture: &mut Captured| {
        capture.begin_sources();
        for index in 0..fleet::SHIPS {
            capture.visit_source(fleet::ship(index), &fleet::census(index));
        }
        capture.finish_sources(Outcome::Rows { len: fleet::SHIPS });
        capture.capture_carrying(&world, CaptureContext::new(fleet::IMAGE), &watches);
        assert_eq!(capture.outcomes(), [Outcome::Rows { len: fleet::SHIPS }]);
        capture.rows.rows[..fleet::SHIPS].to_vec()
    };
    let fresh = hour(&mut capture);
    assert_eq!(hour(&mut capture), fresh);
    const MARK: i32 = 0x5eed;
    for first in [0, 8] {
        capture.rows.rows[first].model_number = MARK;
    }
    let rows = hour(&mut capture);
    assert_eq!((rows[0].model_number, rows[8].model_number), (MARK, MARK));
    assert_eq!(rows[1..8], fresh[1..8]);
}

#[test]
fn unavailable_power_certificate_preserves_core_carry_without_suffix_polling() {
    use crate::ship_model_changes::Watches;
    assert!(naval_power_certificate().is_none());
    let watches = Watches::enabled(64);
    let world = Fleet::new();
    const POWER_ATTACK: usize = 0x1d8;
    world.watched.set(fleet::VARIANT + POWER_ATTACK);
    let mut capture = Captured::with_capacities(&Capacities {
        rows: fleet::SHIPS,
        modifiers: SnapshotCapacity {
            nodes: 64,
            entries: 64,
        },
    });
    let hour = |capture: &mut Captured| {
        capture.begin_sources();
        for index in 0..fleet::SHIPS {
            capture.visit_source(fleet::ship(index), &fleet::census(index));
        }
        capture.finish_sources(Outcome::Rows { len: fleet::SHIPS });
        capture.capture_carrying(&world, CaptureContext::new(fleet::IMAGE), &watches);
        assert_eq!(capture.outcomes(), [Outcome::Rows { len: fleet::SHIPS }]);
        assert!(
            capture.rows.rows[..fleet::SHIPS]
                .iter()
                .all(|row| row.naval_power.is_none())
        );
    };
    hour(&mut capture);
    const MARK: i32 = 0x5eed;
    capture.rows.rows[0].model_number = MARK;
    hour(&mut capture);
    assert_eq!(
        capture.rows.rows[0].model_number, MARK,
        "unknown power cannot invalidate unrelated held model fields"
    );
    assert_eq!(
        world.hits.get(),
        0,
        "an uncertified suffix never polls mutable variant words"
    );
}

#[test]
fn held_naval_power_from_another_certificate_is_read_again() {
    use crate::ship_model_changes::Watches;
    assert!(naval_power_certificate().is_none());
    let watches = Watches::enabled(64);
    let world = Fleet::new();
    let mut capture = Captured::with_capacities(&Capacities {
        rows: fleet::SHIPS,
        modifiers: SnapshotCapacity {
            nodes: 64,
            entries: 64,
        },
    });
    let hour = |capture: &mut Captured| {
        capture.begin_sources();
        for index in 0..fleet::SHIPS {
            capture.visit_source(fleet::ship(index), &fleet::census(index));
        }
        capture.finish_sources(Outcome::Rows { len: fleet::SHIPS });
        capture.capture_carrying(&world, CaptureContext::new(fleet::IMAGE), &watches);
        assert_eq!(capture.outcomes(), [Outcome::Rows { len: fleet::SHIPS }]);
    };
    hour(&mut capture);
    const MARK: i32 = 0x5eed;
    capture.store.naval_power = Some((1, 2, 3));
    for held in capture.store.held.iter_mut().flatten() {
        if let Some((row, _)) = held.row.as_mut() {
            row.naval_power = Some([1; 5]);
        }
    }
    for row in &mut capture.rows.rows[..fleet::SHIPS] {
        row.naval_power = Some([1; 5]);
    }
    capture.rows.rows[0].model_number = MARK;
    hour(&mut capture);
    assert!(
        capture.rows.rows[..fleet::SHIPS]
            .iter()
            .all(|row| row.naval_power.is_none())
    );
    assert_ne!(capture.rows.rows[0].model_number, MARK, "read afresh");
}

#[test]
fn held_country_terms_are_not_read_again() {
    use crate::ship_model_changes::Watches;
    let watches = Watches::enabled(64);
    let world = Fleet::new();
    world.watched.set(fleet::COUNTRY + 0x5b8);
    let mut capture = Captured::with_capacities(&Capacities {
        rows: fleet::SHIPS,
        modifiers: SnapshotCapacity {
            nodes: 64,
            entries: 64,
        },
    });
    let hour = |capture: &mut Captured| {
        capture.begin_sources();
        for index in 0..fleet::SHIPS {
            capture.visit_source(fleet::ship(index), &fleet::census(index));
        }
        capture.finish_sources(Outcome::Rows { len: fleet::SHIPS });
        capture.capture_carrying(&world, CaptureContext::new(fleet::IMAGE), &watches);
        assert_eq!(capture.outcomes(), [Outcome::Rows { len: fleet::SHIPS }]);
    };
    hour(&mut capture);
    assert!(world.hits.get() > 0, "the first hour reads the country");
    hour(&mut capture);
    world.hits.set(0);
    hour(&mut capture);
    assert_eq!(world.hits.get(), 0, "a quiet hour serves the held terms");
    drop(watches.notify(fleet::COUNTRY, 0));
    hour(&mut capture);
    assert!(world.hits.get() > 0, "a touched country is read again");
}

#[test]
#[ignore]
fn ship_model_compute_bench() {
    use crate::ship_model_changes::Watches;
    const HOURS: usize = 200;
    const SHIPS: usize = 2048;
    for (label, per_task, afresh, model_carry, touched) in [
        ("read afresh", 1024, true, true, false),
        ("rows carried", 1024, false, true, false),
        ("three per task force, read afresh", 3, true, true, false),
        ("three per task force, rows carried", 3, false, true, false),
        (
            "three per task force, no model carry",
            3,
            false,
            false,
            false,
        ),
        (
            "three per task force, every ship touched",
            3,
            false,
            true,
            true,
        ),
    ] {
        let world = Fleet::in_tasks(SHIPS, per_task);
        let census: Vec<_> = (0..SHIPS)
            .map(|index| fleet::census_in(index, per_task))
            .collect();
        let watches = Watches::enabled(4 * SHIPS);
        let mut capture = Captured::with_capacities(&Capacities {
            rows: SHIPS,
            modifiers: SnapshotCapacity {
                nodes: 4 * SHIPS,
                entries: 4 * SHIPS,
            },
        });
        super::MODEL_CARRY_OFF.set(!model_carry);
        let mut samples: Vec<f64> = (0..HOURS)
            .map(|_| {
                capture.begin_sources();
                for (index, source) in census.iter().enumerate() {
                    capture.visit_source(fleet::ship(index), source);
                }
                capture.finish_sources(Outcome::Rows { len: SHIPS });
                if touched {
                    for index in 0..SHIPS {
                        drop(watches.notify(fleet::ship(index), 0));
                    }
                }
                if afresh {
                    capture.forget_stores();
                }
                let started = std::time::Instant::now();
                capture.capture_carrying(&world, CaptureContext::new(fleet::IMAGE), &watches);
                assert_eq!(capture.outcomes(), [Outcome::Rows { len: SHIPS }]);
                started.elapsed().as_nanos() as f64 / SHIPS as f64
            })
            .collect();
        super::MODEL_CARRY_OFF.set(false);
        samples.sort_by(f64::total_cmp);
        println!(
            "BENCH ship models, {label}: median {:.0} ns per ship",
            samples[HOURS / 2]
        );
    }
}

#[test]
#[ignore]
fn ship_model_phase_bench() {
    const HOURS: usize = 200;
    const SHIPS: usize = 2048;
    let world = Fleet::with_ships(SHIPS);
    let census: Vec<_> = (0..SHIPS).map(fleet::census).collect();
    let median = |mut samples: Vec<f64>| {
        samples.sort_by(f64::total_cmp);
        samples[samples.len() / 2]
    };
    let mut direct = Captured::part(&capacities(4 * SHIPS, 4 * SHIPS));
    let mut phase = |depth: u8| {
        median(
            (0..HOURS)
                .map(|_| {
                    direct.begin_details();
                    let mut fields = cship::ShipFields::new(fleet::IMAGE);
                    let started = std::time::Instant::now();
                    for (index, source) in census.iter().enumerate() {
                        let ship = fleet::ship(index);
                        fields.visit(&world, ship);
                        if depth == 0 {
                            continue;
                        }
                        let gathered = fields.memory(&world);
                        let memory = ModifierMemory {
                            inner: &gathered,
                            arena: &direct.modifiers,
                            held_task: None,
                        };
                        let model = cship::read_model_with_selection(
                            &memory,
                            fleet::IMAGE,
                            ship,
                            source,
                            &mut direct.model,
                        );
                        assert!(model.is_ok());
                        if depth == 2 {
                            assert!(
                                cship::read_maximum_organisation_memo(
                                    &memory,
                                    fleet::IMAGE,
                                    ship,
                                    &mut direct.doctrine,
                                )
                                .is_ok()
                            );
                        }
                    }
                    started.elapsed().as_nanos() as f64 / SHIPS as f64
                })
                .collect(),
        )
    };
    let gather = phase(0);
    let model = phase(1);
    let organisation = phase(2);
    let mut capture = Captured::with_capacities(&Capacities {
        rows: SHIPS,
        modifiers: SnapshotCapacity {
            nodes: 4 * SHIPS,
            entries: 4 * SHIPS,
        },
    });
    let whole = median(
        (0..HOURS)
            .map(|_| {
                capture.begin_sources();
                for (index, source) in census.iter().enumerate() {
                    capture.visit_source(fleet::ship(index), source);
                }
                capture.finish_sources(Outcome::Rows { len: SHIPS });
                let started = std::time::Instant::now();
                capture.capture_owned(&world, CaptureContext::new(fleet::IMAGE));
                started.elapsed().as_nanos() as f64 / SHIPS as f64
            })
            .collect(),
    );
    println!(
        "BENCH ship model phases, ns per ship: gather {gather:.0}, + model reader {model:.0}, + organisation reader {organisation:.0}, whole capture without a store {whole:.0}"
    );
}

#[test]
#[ignore]
fn ship_model_quiet_bench() {
    use crate::ship_model_changes::{Changes, Watches};
    const HOURS: usize = 200;
    const SHIPS: usize = 2048;
    let world = Fleet::in_tasks(SHIPS, 3);
    let census: Vec<_> = (0..SHIPS).map(|index| fleet::census_in(index, 3)).collect();
    let watches = Watches::enabled(4 * SHIPS);
    let mut capture = Captured::with_capacities(&Capacities {
        rows: SHIPS,
        modifiers: SnapshotCapacity {
            nodes: 4 * SHIPS,
            entries: 4 * SHIPS,
        },
    });
    let median = |mut samples: Vec<f64>| {
        samples.sort_by(f64::total_cmp);
        samples[samples.len() / 2]
    };
    let per_ship = |started: std::time::Instant| started.elapsed().as_nanos() as f64 / SHIPS as f64;
    let (mut census_ns, mut capture_ns, mut certificate_ns) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..HOURS {
        let started = std::time::Instant::now();
        capture.begin_sources();
        for (index, source) in census.iter().enumerate() {
            capture.visit_source(fleet::ship(index), source);
        }
        capture.finish_sources(Outcome::Rows { len: SHIPS });
        census_ns.push(per_ship(started));
        let started = std::time::Instant::now();
        capture.capture_carrying(&world, CaptureContext::new(fleet::IMAGE), &watches);
        capture_ns.push(per_ship(started));
        let started = std::time::Instant::now();
        for index in 0..SHIPS {
            std::hint::black_box(watches.certificate([fleet::ship(index), 0, 0]));
        }
        certificate_ns.push(per_ship(started));
    }
    println!(
        "BENCH ship models quiet hour, ns per ship: census {:.0}, certificate take {:.0}, capture {:.0}",
        median(census_ns),
        median(certificate_ns),
        median(capture_ns)
    );
}

mod carry_cost_bench {
    use super::*;
    use crate::ship_model_changes::Watches;
    use engine_native::{ccountry, cgame_state, cprovince, cunit, cweather_manager};
    use engine_native_parts::cterrain_type;
    use std::{
        hint::black_box,
        sync::atomic::{AtomicUsize, Ordering},
        time::Instant,
    };

    const PER_TASK: usize = 3;
    const SAMPLES: usize = 31;
    const EVICTION_BYTES: usize = 128 * 1024 * 1024;
    const MEMORY_BUDGET: usize = 2 * 1024 * 1024 * 1024;

    struct World {
        memory: Fleet,
        census: Vec<cship::Row>,
        tasks: Vec<usize>,
        locations: [usize; 2],
    }

    impl World {
        fn new(countries: usize) -> Self {
            let ships = countries * PER_TASK;
            let mut memory = Fleet::in_tasks(ships, PER_TASK);
            let tasks: Vec<_> = (0..countries)
                .map(|index| fleet::LOW + 0x8000 + ships * 0x1000 + index * 0x800)
                .collect();
            let slots = fleet::LOW + memory.low.len();
            let links = slots + (countries + 1) * 8;
            let objects = (links + (countries + 1) * 4).next_multiple_of(0x1000);
            let locations = [
                objects + countries * 0x2000,
                objects + countries * 0x2000 + 0x1000,
            ];
            let definitions = [locations[1] + 0x1000, locations[1] + 0x2000];
            let template = definitions[1] + 0x1000;
            let terrain = template + 0x1000;
            let bucket = terrain + 0x1000;
            let weather = bucket + 0x1000;
            let regions = weather + 0x1000;
            let provinces = regions + 0x1000;
            memory.low.resize(provinces + 0x1000 - fleet::LOW, 0);
            let mut put = |address: usize, value: &[u8]| {
                let offset = address - fleet::LOW;
                memory.low[offset..offset + value.len()].copy_from_slice(value);
            };
            let word = |value: usize| (value as u64).to_le_bytes();
            let array = |data: usize, count: usize| {
                let count = u32::try_from(count).unwrap();
                let mut header = [0; 16];
                header[..8].copy_from_slice(&word(data));
                header[8..12].copy_from_slice(&count.to_le_bytes());
                header[12..].copy_from_slice(&count.to_le_bytes());
                header
            };
            put(
                fleet::GAME + cgame_state::COUNTRIES,
                &array(slots, countries + 1),
            );
            put(
                fleet::GAME + cgame_state::COUNTRY_LINKS,
                &array(links, countries + 1),
            );
            for (index, &task) in tasks.iter().enumerate() {
                let id = u32::try_from(index + 1).unwrap();
                let country = objects + index * 0x2000;
                put(slots + (index + 1) * 8, &word(country));
                put(links + (index + 1) * 4, &id.to_le_bytes());
                put(country, &word(fleet::IMAGE + ccountry::VTABLE_RVA.0));
                put(country + ccountry::ID, &id.to_le_bytes());
                put(task + cunit::ROSTER_OWNER, &id.to_le_bytes());
                put(task + cunit::LOCATION, &word(locations[0]));
            }
            for (index, (&location, &definition)) in locations.iter().zip(&definitions).enumerate()
            {
                put(location, &word(fleet::IMAGE + cprovince::VTABLE_RVA.0));
                put(location + cprovince::ID, &(index as u32).to_le_bytes());
                put(location + 0xc8, &word(definition));
                put(definition + 0x30, &word(template));
            }
            put(template + 0x130, &word(terrain));
            put(terrain, &word(fleet::IMAGE + cterrain_type::VTABLE_RVA.0));
            put(terrain + cterrain_type::VALID, &[1]);
            put(terrain + 0x1b8, &word(bucket));
            put(fleet::GAME + cgame_state::WEATHER_MANAGER, &word(weather));
            put(weather, &word(fleet::IMAGE + cweather_manager::VTABLE_RVA.0));
            put(weather + 0x40, &array(regions, 1));
            put(weather + 0x370, &array(provinces, locations.len()));
            for index in 0..ships {
                put(fleet::ship(index) + 0x270, &100_000_i64.to_le_bytes());
            }
            Self {
                memory,
                census: (0..ships)
                    .map(|index| fleet::census_in(index, PER_TASK))
                    .collect(),
                tasks,
                locations,
            }
        }

        fn move_tasks(&mut self, hour: usize) {
            let value = (self.locations[hour % self.locations.len()] as u64).to_le_bytes();
            for &task in &self.tasks {
                let offset = task + cunit::LOCATION - fleet::LOW;
                self.memory.low[offset..offset + value.len()].copy_from_slice(&value);
            }
        }

        fn seed(&self, capture: &mut Captured) {
            capture.begin_sources();
            for (index, source) in self.census.iter().enumerate() {
                capture.visit_source(fleet::ship(index), source);
            }
            capture.finish_sources(Outcome::Rows {
                len: self.census.len(),
            });
        }
    }

    struct Borrowed<'a>(&'a Fleet);
    impl Memory for Borrowed<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.0.copy(address, output)
        }

        fn copy_scattered<const N: usize>(
            &self,
            addresses: [usize; N],
            destinations: &mut [&mut [u8]; N],
        ) -> usize {
            self.0.copy_scattered(addresses, destinations)
        }

        fn copy_records(
            &self,
            sources: RecordSources<'_>,
            layout: &RecordLayout<'_>,
            destination: &mut [u8],
        ) -> usize {
            self.0.copy_records(sources, layout, destination)
        }
    }

    struct CountedChanges<'a> {
        inner: &'a Watches,
        sequences: AtomicUsize,
        trees: AtomicUsize,
        certificates: AtomicUsize,
    }

    impl Changes for CountedChanges<'_> {
        fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
            self.sequences.fetch_add(1, Ordering::Relaxed);
            self.inner.sequence(receiver)
        }

        fn tree_sequence(&self, receiver: usize) -> Option<(u64, u64)> {
            self.trees.fetch_add(1, Ordering::Relaxed);
            self.inner.tree_sequence(receiver)
        }

        fn sequence_at(&self, receiver: usize, slot: usize) -> Option<((u64, u64), usize)> {
            self.sequences.fetch_add(1, Ordering::Relaxed);
            self.inner.sequence_at(receiver, slot)
        }

        fn certificate_at(
            &self,
            receivers: [usize; 3],
            slots: Slots,
        ) -> Option<(Certificate, Slots)> {
            self.certificates.fetch_add(1, Ordering::Relaxed);
            self.sequences.fetch_add(
                receivers.iter().filter(|&&receiver| receiver != 0).count(),
                Ordering::Relaxed,
            );
            self.inner.certificate_at(receivers, slots)
        }

        fn quiet(&self) -> Option<(u64, u64)> {
            self.inner.quiet()
        }
    }

    #[derive(Clone, Copy)]
    enum Path {
        Carried,
        Moved,
        Revalidated,
        Fresh,
    }

    impl Path {
        fn label(self) -> &'static str {
            match self {
                Self::Carried => "rows carried",
                Self::Moved => "task force moved",
                Self::Revalidated => "writer touched unchanged bytes",
                Self::Fresh => "read afresh",
            }
        }

        fn prepare(
            self,
            world: &mut World,
            watches: &Watches,
            capture: &mut Captured,
            hour: usize,
        ) {
            match self {
                Self::Moved => world.move_tasks(hour),
                Self::Revalidated => {
                    for index in 0..world.census.len() {
                        drop(watches.notify(fleet::ship(index), 0));
                    }
                }
                Self::Fresh => capture.forget_stores(),
                Self::Carried => {}
            }
        }
    }

    fn capture(ships: usize) -> Captured {
        Captured::part(&Capacities {
            rows: ships,
            modifiers: SnapshotCapacity {
                nodes: 4 * ships,
                entries: 4 * ships,
            },
        })
    }

    fn evict(buffer: &mut [u64]) {
        let mut total = 0_u64;
        for word in buffer {
            *word = word.wrapping_add(1);
            total = total.wrapping_add(*word);
        }
        black_box(total);
    }

    fn median(mut samples: Vec<f64>) -> f64 {
        samples.sort_by(f64::total_cmp);
        samples[samples.len() / 2]
    }

    fn check_budget(world: &World, capture: &Captured, eviction: &[u64]) {
        let owned = world.memory.low.capacity()
            + world.census.capacity() * std::mem::size_of::<cship::Row>()
            + world.tasks.capacity() * std::mem::size_of::<usize>()
            + capture.allocated_bytes()
            + std::mem::size_of_val(eviction);
        assert!(
            owned < MEMORY_BUDGET,
            "ship-model benchmark exceeded two GiB: {owned}"
        );
    }

    fn account(world: &World, capture: &mut Captured, watches: &Watches, path: Path) {
        let changes = CountedChanges {
            inner: watches,
            sequences: AtomicUsize::new(0),
            trees: AtomicUsize::new(0),
            certificates: AtomicUsize::new(0),
        };
        let accounting = crate::read_accounting::Capture::new();
        let memory = engine_native_memory::memory::CountedMemory::new(Borrowed(&world.memory));
        capture.capture_carrying(&memory, CaptureContext::new(fleet::IMAGE), &changes);
        let reads = accounting.finish()[TableId::ShipModels.index()];
        assert!(reads.copy_calls + reads.scattered_calls + reads.record_calls != 0);
        assert_eq!(
            capture.outcomes(),
            [Outcome::Rows {
                len: world.census.len()
            }]
        );
        let ships = world.census.len() as f64;
        println!(
            "ACCOUNT ship models {} countries, {}: guarded calls/ship {:.2}, requested native bytes/ship {:.2}, sequence queries/ship {:.2}, tree lookups/ship {:.2}, certificate calls/ship {:.2}",
            world.tasks.len(),
            path.label(),
            (reads.copy_calls + reads.scattered_calls + reads.record_calls + reads.plan_calls)
                as f64
                / ships,
            reads.bytes as f64 / ships,
            changes.sequences.load(Ordering::Relaxed) as f64 / ships,
            changes.trees.load(Ordering::Relaxed) as f64 / ships,
            changes.certificates.load(Ordering::Relaxed) as f64 / ships,
        );
        let cost = CARRY_COST.get();
        println!(
            "ACCOUNT carried-state operations: ship-index searches {}, cold HeldShip bytes exposed {}, organisation packet bytes read {}, cold terms bytes compared {}, variant comparisons {}, task-term comparisons {}, row clones {}; hot record {} B",
            cost.searches,
            cost.cold_ship_bytes,
            cost.cold_organisation_bytes,
            cost.cold_terms_bytes,
            cost.variant_compares,
            cost.terms_compares,
            cost.row_clones,
            std::mem::size_of::<Hot>()
        );
    }

    fn terms_hour(memory: &impl Memory, held: &[cship::HeldShip], capture: &mut Captured) -> u128 {
        capture.begin_details();
        let mut fields = cship::ShipFields::new(fleet::IMAGE);
        let mut elapsed = 0_u128;
        for held in held {
            let task = fields
                .task_force_of(memory, held)
                .expect("complete task-force gather");
            let gathered = fields.memory(memory);
            let memory = ModifierMemory {
                inner: &gathered,
                arena: &capture.modifiers,
                held_task: None,
            };
            let timer = Instant::now();
            let terms = cship::read_task_terms(&memory, fleet::IMAGE, &task, &mut capture.doctrine);
            elapsed += timer.elapsed().as_nanos();
            black_box(
                terms
                    .expect("valid task-force terms")
                    .expect("complete task-force terms"),
            );
        }
        elapsed
    }

    fn task_terms(world: &World, eviction: &mut [u64], cold: bool) {
        let mut fields = cship::ShipFields::new(fleet::IMAGE);
        let held: Vec<_> = (0..world.tasks.len())
            .map(|index| {
                fields.visit(&world.memory, fleet::ship(index * PER_TASK));
                fields.held()
            })
            .collect();
        let mut capture = capture(world.census.len());
        let mut samples = Vec::with_capacity(SAMPLES);
        for hour in 0..=SAMPLES {
            if cold {
                evict(eviction);
            }
            let elapsed = terms_hour(&world.memory, &held, &mut capture);
            if hour != 0 {
                samples.push(elapsed as f64 / world.census.len() as f64);
            }
            check_budget(world, &capture, eviction);
        }
        let outer = engine_native_memory::memory::ReadCounts::take_current();
        let counted = engine_native_memory::memory::CountedMemory::new(Borrowed(&world.memory));
        terms_hour(&counted, &held, &mut capture);
        let reads = engine_native_memory::memory::ReadCounts::take_current();
        outer.add_current();
        let ns = median(samples);
        println!(
            "BENCH ship models {} countries, {} cache, task terms read: median {ns:.0} ns per ship ({:.0} ns per task force); guarded calls/task {:.2}, requested native bytes/task {:.2} (including task gathers, timers exclude gathers)",
            world.tasks.len(),
            if cold { "cold" } else { "warm" },
            ns * PER_TASK as f64,
            (reads.copy_calls + reads.scattered_calls + reads.record_calls + reads.plan_calls)
                as f64
                / world.tasks.len() as f64,
            reads.bytes as f64 / world.tasks.len() as f64,
        );
    }

    fn hour(world: &World, capture: &mut Captured, changes: &dyn Changes) -> Vec<cship::ModelRow> {
        world.seed(capture);
        capture.capture_carrying(&world.memory, CaptureContext::new(fleet::IMAGE), changes);
        assert_eq!(
            capture.outcomes(),
            [Outcome::Rows {
                len: world.census.len()
            }]
        );
        capture.rows.rows[..world.census.len()].to_vec()
    }

    fn fresh_hour(
        world: &World,
        capture: &mut Captured,
        changes: &dyn Changes,
    ) -> Vec<cship::ModelRow> {
        capture.forget_stores();
        hour(world, capture, changes)
    }

    #[test]
    fn quiet_task_arrays_share_ownership_through_roster_changes() {
        let mut world = World::new(4);
        let mut fields = cship::ShipFields::new(fleet::IMAGE);
        for index in 0..world.tasks.len() {
            fields.visit(&world.memory, fleet::ship(index * PER_TASK));
            let task = fields.task_force().unwrap();
            let entry = task.pointer + 0x780;
            let offset = task.modifiers - fleet::LOW;
            world.memory.low[offset..offset + 8].copy_from_slice(&(entry as u64).to_le_bytes());
            world.memory.low[offset + 8..offset + 16]
                .copy_from_slice(&0x0000_0001_0000_0001_u64.to_le_bytes());
            let offset = entry - fleet::LOW;
            world.memory.low[offset..offset + 4].copy_from_slice(&1_u32.to_le_bytes());
            world.memory.low[offset + 8..offset + 16]
                .copy_from_slice(&(index as i64 + 1).to_le_bytes());
        }
        let watches = Watches::enabled(4 * world.census.len());
        let mut captured = capture(world.census.len());
        let fresh = hour(&world, &mut captured, &watches);
        let arrays: Vec<_> = captured
            .store
            .tasks
            .iter()
            .map(|task| {
                (
                    task.pointer,
                    Arc::clone(task.modifiers.as_ref().expect("task modifiers observed")),
                )
            })
            .collect();
        for _ in 0..3 {
            assert_eq!(hour(&world, &mut captured, &watches), fresh);
            for (pointer, entries) in &arrays {
                assert!(!entries.is_empty());
                let held = captured
                    .store
                    .task(*pointer)
                    .unwrap()
                    .modifiers
                    .as_ref()
                    .unwrap();
                assert!(
                    Arc::ptr_eq(entries, held),
                    "equal task arrays must not be copied"
                );
            }
        }
        let all = world.census.clone();
        world.census.truncate(2 * PER_TASK);
        let deleted = hour(&world, &mut captured, &watches);
        assert_eq!(captured.store.tasks.len(), 2);
        let mut direct = capture(all.len());
        assert_eq!(deleted, fresh_hour(&world, &mut direct, &watches));
        world.census = all;
        let added = hour(&world, &mut captured, &watches);
        assert_eq!(added, fresh_hour(&world, &mut direct, &watches));
        assert_eq!(captured.store.tasks.len(), 4);
    }

    #[test]
    fn organisation_only_writers_keep_models_and_equal_fresh_reads() {
        let countries = 12;
        let mut world = World::new(countries);
        let watches = Watches::enabled(4 * world.census.len());
        let mut held = capture(world.census.len());
        let previous = hour(&world, &mut held, &watches);
        for index in 0..world.census.len() {
            let at = fleet::ship(index) + cship::BASE_ORGANISATION - fleet::LOW;
            world.memory.low[at..at + 8].copy_from_slice(&30_000_000_i64.to_le_bytes());
            drop(watches.notify(fleet::ship(index), 0));
        }
        world
            .memory
            .watched
            .set(fleet::IMAGE + engine_native_parts::cequipment_database::SINGLETON_RVA.0);
        world.memory.hits.set(0);
        let current = hour(&world, &mut held, &watches);
        assert_ne!(
            current[0].maximum_organisation,
            previous[0].maximum_organisation
        );
        assert_eq!(
            world.memory.hits.get(),
            0,
            "equal model suffixes need no definition database"
        );
        let mut fresh = capture(world.census.len());
        assert_eq!(current, fresh_hour(&world, &mut fresh, &watches));
        assert!(
            world.memory.hits.get() > 0,
            "fresh reference still reads the model suffix"
        );
        assert!(!watches.poisoned());
        watches.boundary(true);
        let at = fleet::ship(0) + cship::BASE_ORGANISATION - fleet::LOW;
        world.memory.low[at..at + 8].copy_from_slice(&31_000_000_i64.to_le_bytes());
        world.memory.hits.set(0);
        let recovered = hour(&world, &mut held, &watches);
        assert!(
            world.memory.hits.get() > 0,
            "an epoch change cannot use partial revalidation"
        );
        let mut fresh = capture(world.census.len());
        assert_eq!(recovered, fresh_hour(&world, &mut fresh, &watches));
    }

    #[test]
    #[ignore = "ship-model organisation-only revalidation benchmark"]
    fn organisation_only_model_carry_bench() {
        let mut expected = None;
        for model_carry in [false, true] {
            let mut world = World::new(1500);
            let ships = world.census.len();
            let watches = Watches::enabled(4 * ships);
            let mut held = capture(ships);
            hour(&world, &mut held, &watches);
            super::MODEL_CARRY_OFF.set(!model_carry);
            let mut samples = Vec::with_capacity(SAMPLES);
            let mut calls = 0;
            for sample in 0..SAMPLES {
                for index in 0..ships {
                    let at = fleet::ship(index) + cship::BASE_ORGANISATION - fleet::LOW;
                    world.memory.low[at..at + 8]
                        .copy_from_slice(&(30_000_000_i64 + sample as i64).to_le_bytes());
                    drop(watches.notify(fleet::ship(index), 0));
                }
                world.seed(&mut held);
                let before = world.memory.calls.get();
                let timer = Instant::now();
                held.capture_carrying(&world.memory, CaptureContext::new(fleet::IMAGE), &watches);
                samples.push(timer.elapsed().as_nanos() as f64 / ships as f64);
                calls += world.memory.calls.get() - before;
                assert_eq!(held.outcomes(), [Outcome::Rows { len: ships }]);
            }
            super::MODEL_CARRY_OFF.set(false);
            assert!(!watches.poisoned());
            let rows = &held.rows.rows[..ships];
            if let Some(expected) = &expected {
                assert_eq!(rows, expected);
            } else {
                expected = Some(rows.to_vec());
            }
            println!(
                "BENCH organisation-only ship writers, model carry {model_carry}: median {:.0} ns/ship, {:.2} native copies/ship",
                median(samples),
                calls as f64 / (ships * SAMPLES) as f64,
            );
        }
    }

    #[test]
    fn hot_carried_rows_avoid_cold_records_and_ship_searches() {
        let countries = 12;
        let world = World::new(countries);
        let watches = Watches::enabled(4 * world.census.len());
        let mut capture = capture(world.census.len());
        let before_fresh = world.memory.calls.get();
        let fresh = hour(&world, &mut capture, &watches);
        let fresh_calls = world.memory.calls.get() - before_fresh;
        let changes = CountedChanges {
            inner: &watches,
            sequences: AtomicUsize::new(0),
            trees: AtomicUsize::new(0),
            certificates: AtomicUsize::new(0),
        };
        world.seed(&mut capture);
        let before = world.memory.calls.get();
        capture.capture_carrying(&world.memory, CaptureContext::new(fleet::IMAGE), &changes);
        assert_eq!(capture.rows.rows[..world.census.len()], fresh);
        let cost = CARRY_COST.get();
        assert_eq!(cost.searches, 0);
        assert_eq!(cost.cold_ship_bytes, 0);
        assert_eq!(cost.cold_organisation_bytes, 0);
        assert_eq!(cost.cold_terms_bytes, 0);
        assert_eq!(cost.row_clones, 0);
        assert_eq!(cost.variant_compares, 1, "one shared variant");
        assert_eq!(
            cost.terms_compares, countries,
            "one comparison per task force"
        );
        assert_eq!(changes.certificates.load(Ordering::Relaxed), 0);
        assert_eq!(
            changes.sequences.load(Ordering::Relaxed),
            3 * countries,
            "task-force and two weather-region certificate queries per task force"
        );
        let carried_calls = world.memory.calls.get() - before;
        assert!(
            carried_calls > 0,
            "shared sources remain current-hour reads"
        );
        assert!(
            carried_calls < fresh_calls,
            "{carried_calls} carried guarded calls, {fresh_calls} fresh"
        );
    }

    #[test]
    fn moved_task_rows_equal_fresh_reads_without_rewriting_the_model() {
        let mut world = World::new(4);
        let watches = Watches::enabled(4 * world.census.len());
        let mut capture = capture(world.census.len());
        hour(&world, &mut capture, &watches);
        world.move_tasks(1);
        let carried = hour(&world, &mut capture, &watches);
        let cost = CARRY_COST.get();
        assert_eq!(
            cost.cold_ship_bytes, 0,
            "only compact organisation operands are read"
        );
        assert_eq!(cost.row_clones, 0);
        let mut direct = self::capture(world.census.len());
        assert_eq!(carried, fresh_hour(&world, &mut direct, &watches));
        const MARK: i32 = 0x5eed;
        capture.rows.rows[5].model_number = MARK;
        world.move_tasks(0);
        let rows = hour(&world, &mut capture, &watches);
        assert_eq!(
            rows[5].model_number, MARK,
            "only organisation and roster position are written"
        );
    }

    #[test]
    fn reordered_ship_census_uses_the_index_and_keeps_serial_rows() {
        let mut world = World::new(4);
        let watches = Watches::enabled(4 * world.census.len());
        let mut capture = capture(world.census.len());
        hour(&world, &mut capture, &watches);
        world.census.swap(0, 1);
        capture.begin_sources();
        for (index, source) in world.census.iter().enumerate() {
            let pointer = if index < 2 {
                fleet::ship(1 - index)
            } else {
                fleet::ship(index)
            };
            capture.visit_source(pointer, source);
        }
        capture.finish_sources(Outcome::Rows {
            len: world.census.len(),
        });
        capture.capture_carrying(&world.memory, CaptureContext::new(fleet::IMAGE), &watches);
        assert_eq!(
            capture.outcomes(),
            [Outcome::Rows {
                len: world.census.len()
            }]
        );
        let cost = CARRY_COST.get();
        assert_eq!(cost.searches, 2);
        assert_eq!(cost.row_clones, 2);
        assert_eq!(capture.rows.rows[0].ship, world.census[0].id);
        assert_eq!(capture.rows.rows[1].ship, world.census[1].id);
        assert_eq!(capture.rows.rows[0].model_number, 1);
        assert_eq!(capture.rows.rows[1].model_number, 0);
    }

    struct CriticalMemory<'a> {
        inner: &'a Fleet,
        watched: usize,
        count_reads: Cell<usize>,
        fault: Option<usize>,
    }

    impl Memory for CriticalMemory<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == self.watched {
                self.count_reads.set(self.count_reads.get() + 1);
            }
            if self
                .fault
                .is_some_and(|fault| fault >= address && fault - address < output.len())
            {
                return false;
            }
            self.inner.copy(address, output)
        }
    }

    fn critical_world() -> World {
        let mut world = World::new(4);
        let critical = fleet::LOW + world.memory.low.len();
        world.memory.low.resize(world.memory.low.len() + 0x1000, 0);
        for index in 0..world.census.len() {
            let ship = fleet::ship(index);
            let entry = ship + 0xa00;
            let at = ship + 0x918 - fleet::LOW;
            world.memory.low[at..at + 8].copy_from_slice(&(entry as u64).to_le_bytes());
            world.memory.low[at + 8..at + 12].copy_from_slice(&1_u32.to_le_bytes());
            world.memory.low[at + 12..at + 16].copy_from_slice(&1_u32.to_le_bytes());
            let at = entry - fleet::LOW;
            world.memory.low[at..at + 8].copy_from_slice(&(critical as u64).to_le_bytes());
            world.memory.low[at + 8..at + 12].copy_from_slice(&2_i32.to_le_bytes());
        }
        world
    }

    #[test]
    fn moved_rows_with_criticals_keep_both_live_count_reads_and_serial_faults() {
        for fault in [None, Some(fleet::ship(0) + 0xa08)] {
            let mut world = critical_world();
            let watches = Watches::enabled(4 * world.census.len());
            let mut held = capture(world.census.len());
            let seeded = hour(&world, &mut held, &watches);
            assert!(seeded.iter().all(|row| row.has_criticals == Some(1)));
            let quiet = hour(&world, &mut held, &watches);
            assert_eq!(seeded, quiet);
            world.move_tasks(1);
            let memory = CriticalMemory {
                inner: &world.memory,
                watched: fleet::ship(0) + 0xa08,
                count_reads: Cell::new(0),
                fault,
            };
            world.seed(&mut held);
            held.capture_carrying(&memory, CaptureContext::new(fleet::IMAGE), &watches);
            let outcome = held.outcomes();
            let carried = held.rows.rows[..world.census.len()].to_vec();
            assert_eq!(
                memory.count_reads.get(),
                if fault.is_some() { 1 } else { 2 }
            );
            let cost = CARRY_COST.get();
            assert_eq!(cost.cold_ship_bytes, 0);
            assert_eq!(
                cost.cold_organisation_bytes,
                if fault.is_some() {
                    1
                } else {
                    world.census.len()
                } * std::mem::size_of::<cship::OrganisationOperands>()
            );
            assert_eq!(
                cost.cold_terms_bytes,
                if fault.is_some() {
                    0
                } else {
                    world.census.len()
                } * std::mem::size_of::<Terms>()
            );
            assert_eq!(cost.row_clones, 0);
            let mut direct = capture(world.census.len());
            memory.count_reads.set(0);
            world.seed(&mut direct);
            direct.capture_carrying(&memory, CaptureContext::new(fleet::IMAGE), &watches);
            assert_eq!(outcome, direct.outcomes());
            if fault.is_none() {
                assert_eq!(carried, direct.rows.rows[..world.census.len()]);
            }
        }
    }

    #[test]
    fn touched_rows_revalidate_and_revoke_quiet_certificates() {
        let world = World::new(4);
        let watches = Watches::enabled(4 * world.census.len());
        let mut capture = capture(world.census.len());
        let expected = hour(&world, &mut capture, &watches);
        drop(watches.notify(fleet::ship(3), 0));
        let changes = CountedChanges {
            inner: &watches,
            sequences: AtomicUsize::new(0),
            trees: AtomicUsize::new(0),
            certificates: AtomicUsize::new(0),
        };
        assert_eq!(hour(&world, &mut capture, &changes), expected);
        let cost = CARRY_COST.get();
        assert_eq!(cost.searches, 0);
        assert_eq!(cost.row_clones, 0);
        assert_eq!(cost.cold_ship_bytes, std::mem::size_of::<cship::HeldShip>());
        assert_eq!(
            changes.certificates.load(Ordering::Relaxed),
            world.census.len()
        );
        assert!(!watches.poisoned());
        watches.boundary(true);
        assert_eq!(hour(&world, &mut capture, &watches), expected);
        assert!(
            CARRY_COST.get().cold_ship_bytes
                >= world.census.len() * std::mem::size_of::<cship::HeldShip>()
        );
    }

    struct WriterDuringReads<'a> {
        bytes: RefCell<Vec<u8>>,
        watches: &'a Watches,
        trigger: usize,
        fired: Cell<bool>,
    }

    impl Memory for WriterDuringReads<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == self.trigger && !self.fired.replace(true) {
                let notification = self.watches.notify(fleet::ship(4), 0);
                let at = fleet::ship(4) + cship::BASE_ORGANISATION - fleet::LOW;
                self.bytes.borrow_mut()[at..at + 8].copy_from_slice(&60_000_000_i64.to_le_bytes());
                drop(notification);
            }
            if address == fleet::SINGLETON && output.len() == 8 {
                output.copy_from_slice(&(fleet::GAME as u64).to_le_bytes());
                return true;
            }
            if address == fleet::IMAGE + engine_native_parts::cequipment_database::SINGLETON_RVA.0
                && output.len() == 8
            {
                output.copy_from_slice(&(fleet::DATABASE as u64).to_le_bytes());
                return true;
            }
            SliceMemory {
                base: fleet::LOW,
                bytes: &self.bytes.borrow(),
            }
            .copy(address, output)
        }
    }

    #[test]
    fn a_writer_during_shared_reads_revokes_later_quiet_row_carries() {
        let world = World::new(4);
        let watches = Watches::enabled(4 * world.census.len());
        let mut held = capture(world.census.len());
        let previous = hour(&world, &mut held, &watches);
        let memory = WriterDuringReads {
            bytes: RefCell::new(world.memory.low.clone()),
            watches: &watches,
            trigger: world.tasks[0],
            fired: Cell::new(false),
        };
        world.seed(&mut held);
        held.capture_carrying(&memory, CaptureContext::new(fleet::IMAGE), &watches);
        assert!(memory.fired.get());
        assert_eq!(
            held.outcomes(),
            [Outcome::Rows {
                len: world.census.len()
            }]
        );
        assert_ne!(
            held.rows.rows[4].maximum_organisation,
            previous[4].maximum_organisation
        );
        let mut direct = capture(world.census.len());
        world.seed(&mut direct);
        direct.capture_carrying(&memory, CaptureContext::new(fleet::IMAGE), &watches);
        assert_eq!(
            held.rows.rows[..world.census.len()],
            direct.rows.rows[..world.census.len()]
        );
        assert!(!watches.poisoned());
    }

    #[test]
    fn pools_beyond_gathered_variant_storage_keep_the_live_model_reader() {
        let mut world = World::new(4);
        let watches = Watches::enabled(4 * world.census.len());
        let mut variant = 0;
        for index in 0..world.census.len() {
            let ship = fleet::ship(index);
            let bank = ship + 0x980;
            variant = u64::from_le_bytes(
                world.memory.low[bank - fleet::LOW..bank - fleet::LOW + 8]
                    .try_into()
                    .unwrap(),
            ) as usize;
            let at = ship + cship::EQUIPMENT_POOL + engine_native::cequipment_variant::pool::ITEMS
                - fleet::LOW;
            world.memory.low[at + 8..at + 12].copy_from_slice(&3_u32.to_le_bytes());
            world.memory.low[at + 12..at + 16].copy_from_slice(&3_u32.to_le_bytes());
            for entry in 1..3 {
                let at = bank + entry * 16 - fleet::LOW;
                world.memory.low[at..at + 8].copy_from_slice(&(variant as u64).to_le_bytes());
                world.memory.low[at + 8..at + 16].copy_from_slice(&5_i64.to_le_bytes());
            }
        }
        let mut held = capture(world.census.len());
        let initial = hour(&world, &mut held, &watches);
        world.memory.low[variant + engine_native::cequipment_variant::VERSION - fleet::LOW] = 7;
        let carried = hour(&world, &mut held, &watches);
        let mut direct = capture(world.census.len());
        let fresh = fresh_hour(&world, &mut direct, &watches);
        assert_ne!(fresh[0].hull_variant, initial[0].hull_variant);
        assert_eq!(carried, fresh);
        assert!(!watches.poisoned());
    }

    struct MetadataMemory<'a> {
        inner: &'a Fleet,
        table: usize,
        fault: Option<usize>,
    }

    impl Memory for MetadataMemory<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if self
                .fault
                .is_some_and(|fault| fault >= address && fault - address < output.len())
            {
                return false;
            }
            if address == fleet::IMAGE + engine_native_parts::modifier_definition::TABLE_RVA.0
                && output.len() == 8
            {
                output.copy_from_slice(&(self.table as u64).to_le_bytes());
                return true;
            }
            self.inner.copy(address, output)
        }
    }

    #[test]
    fn partial_definition_suffixes_recover_without_carrying_or_poisoning() {
        let world = World::new(4);
        let watches = Watches::enabled(4 * world.census.len());
        let mut held = capture(world.census.len());
        world.seed(&mut held);
        held.capture_carrying(
            &CriticalMemory {
                inner: &world.memory,
                watched: 0,
                count_reads: Cell::new(0),
                fault: Some(fleet::IMAGE + engine_native_parts::cequipment_database::SINGLETON_RVA.0),
            },
            CaptureContext::new(fleet::IMAGE),
            &watches,
        );
        assert_eq!(
            held.outcomes(),
            [Outcome::Rows {
                len: world.census.len()
            }]
        );
        assert_eq!(
            held.rows.rows[0].hull_variant.unwrap().definition_index,
            None
        );
        let recovered = hour(&world, &mut held, &watches);
        let mut direct = capture(world.census.len());
        assert_eq!(recovered, fresh_hour(&world, &mut direct, &watches));
        assert_eq!(recovered[0].hull_variant.unwrap().definition_index, Some(1));
        assert!(!watches.poisoned());
    }

    #[test]
    fn revalidated_rows_reread_critical_counts_and_pointees_with_equal_task_terms() {
        for change_pointer in [false, true] {
            let mut world = critical_world();
            let watches = Watches::enabled(4 * world.census.len());
            let first_entry = fleet::ship(0) + 0xa00 - fleet::LOW;
            let critical = u64::from_le_bytes(
                world.memory.low[first_entry..first_entry + 8]
                    .try_into()
                    .unwrap(),
            ) as usize;
            let other = fleet::LOW + world.memory.low.len();
            let table = other + 0x1000;
            world.memory.low.resize(world.memory.low.len() + 0x21000, 0);
            for (effect, value) in [(critical, 100_000_i64), (other, 300_000_i64)] {
                let entry = effect + 0x200;
                let at = effect + 0x98 - fleet::LOW;
                world.memory.low[at..at + 8].copy_from_slice(&(entry as u64).to_le_bytes());
                world.memory.low[at + 8..at + 12].copy_from_slice(&1_u32.to_le_bytes());
                world.memory.low[at + 12..at + 16].copy_from_slice(&1_u32.to_le_bytes());
                let at = entry - fleet::LOW;
                world.memory.low[at..at + 4]
                    .copy_from_slice(&engine_executable::modifier_keys::NAVY_ORG.to_le_bytes());
                world.memory.low[at + 8..at + 16].copy_from_slice(&value.to_le_bytes());
            }
            let mut held = capture(world.census.len());
            world.seed(&mut held);
            held.capture_carrying(
                &MetadataMemory {
                    inner: &world.memory,
                    table,
                    fault: None,
                },
                CaptureContext::new(fleet::IMAGE),
                &watches,
            );
            assert_eq!(
                held.outcomes(),
                [Outcome::Rows {
                    len: world.census.len()
                }]
            );
            let before = held.rows.rows[0].maximum_organisation;
            let notification = watches.notify(fleet::ship(0), 0);
            if change_pointer {
                world.memory.low[first_entry..first_entry + 8]
                    .copy_from_slice(&(other as u64).to_le_bytes());
            } else {
                world.memory.low[first_entry + 8..first_entry + 12]
                    .copy_from_slice(&5_i32.to_le_bytes());
            }
            drop(notification);
            let memory = MetadataMemory {
                inner: &world.memory,
                table,
                fault: None,
            };
            world.seed(&mut held);
            held.capture_carrying(&memory, CaptureContext::new(fleet::IMAGE), &watches);
            assert_eq!(
                held.outcomes(),
                [Outcome::Rows {
                    len: world.census.len()
                }]
            );
            assert_ne!(held.rows.rows[0].maximum_organisation, before);
            let mut direct = capture(world.census.len());
            world.seed(&mut direct);
            direct.capture_carrying(&memory, CaptureContext::new(fleet::IMAGE), &watches);
            assert_eq!(held.outcomes(), direct.outcomes());
            assert_eq!(
                held.rows.rows[..world.census.len()],
                direct.rows.rows[..world.census.len()]
            );
            assert!(!watches.poisoned());
        }
    }

    #[test]
    fn modifier_guard_fallback_preserves_model_fault_order_and_is_bounded() {
        use engine_native_memory::batch_fallback::{Batch, count};
        use engine_native_parts::modifier_definition as metadata;
        for bad_variant in [false, true] {
            let mut world = World::new(4);
            let watches = Watches::enabled(4 * world.census.len());
            let table = fleet::LOW + world.memory.low.len();
            world.memory.low.resize(world.memory.low.len() + 0x20000, 0);
            for index in 0..world.census.len() {
                let ship = fleet::ship(index);
                let entry = ship + 0xb00;
                let at = ship + 0x750 - fleet::LOW;
                world.memory.low[at..at + 8].copy_from_slice(&(entry as u64).to_le_bytes());
                world.memory.low[at + 8..at + 12].copy_from_slice(&1_u32.to_le_bytes());
                world.memory.low[at + 12..at + 16].copy_from_slice(&1_u32.to_le_bytes());
                let at = entry - fleet::LOW;
                world.memory.low[at..at + 4]
                    .copy_from_slice(&engine_executable::modifier_keys::NAVY_ORG.to_le_bytes());
                world.memory.low[at + 8..at + 16].copy_from_slice(&100_000_i64.to_le_bytes());
            }
            let mut held = capture(world.census.len());
            world.seed(&mut held);
            held.capture_carrying(
                &MetadataMemory {
                    inner: &world.memory,
                    table,
                    fault: None,
                },
                CaptureContext::new(fleet::IMAGE),
                &watches,
            );
            assert_eq!(
                held.outcomes(),
                [Outcome::Rows {
                    len: world.census.len()
                }]
            );
            if bad_variant {
                let bank = fleet::ship(0) + 0x980 - fleet::LOW;
                let variant =
                    u64::from_le_bytes(world.memory.low[bank..bank + 8].try_into().unwrap())
                        as usize;
                world.memory.low[variant - fleet::LOW..variant - fleet::LOW + 8].fill(0);
                drop(watches.notify_variant(variant));
            }
            let memory = MetadataMemory {
                inner: &world.memory,
                table,
                fault: Some(
                    table
                        + engine_executable::modifier_keys::NAVY_ORG as usize * metadata::STRIDE
                        + metadata::FLAGS,
                ),
            };
            world.seed(&mut held);
            let before = count(Batch::ShipModelGuards);
            held.capture_carrying(&memory, CaptureContext::new(fleet::IMAGE), &watches);
            assert_eq!(
                count(Batch::ShipModelGuards) - before,
                u64::from(!bad_variant)
            );
            let mut direct = capture(world.census.len());
            world.seed(&mut direct);
            direct.capture_carrying(&memory, CaptureContext::new(fleet::IMAGE), &watches);
            assert_eq!(held.outcomes(), direct.outcomes());
        }
    }

    struct VariantVersionFault<'a> {
        inner: &'a Fleet,
        version: usize,
    }

    impl Memory for VariantVersionFault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if self.version >= address && self.version - address < output.len() {
                return false;
            }
            self.inner.copy(address, output)
        }
    }

    #[test]
    fn partial_variant_gathers_keep_later_optional_fields_current() {
        let mut world = World::new(4);
        let watches = Watches::enabled(4 * world.census.len());
        let bank = fleet::ship(0) + 0x980 - fleet::LOW;
        let variant =
            u64::from_le_bytes(world.memory.low[bank..bank + 8].try_into().unwrap()) as usize;
        let marker = variant + 0x4b0 - fleet::LOW;
        world.memory.low[marker..marker + 4].copy_from_slice(&23_i32.to_le_bytes());
        let memory = VariantVersionFault {
            inner: &world.memory,
            version: variant + engine_native::cequipment_variant::VERSION,
        };
        let mut fields = cship::ShipFields::new(fleet::IMAGE);
        fields.visit(&memory, fleet::ship(0));
        let observation = fields.held();
        assert!(!observation.variants_complete());
        let variant_observation = observation.variants()[0];
        assert!(!fields.held_variant_unchanged(&memory, &variant_observation));
        let mut held = capture(world.census.len());
        world.seed(&mut held);
        held.capture_carrying(&memory, CaptureContext::new(fleet::IMAGE), &watches);
        assert_eq!(
            held.outcomes(),
            [Outcome::Rows {
                len: world.census.len()
            }]
        );
        let initial = held.rows.rows[0].hull_variant.unwrap();
        assert_eq!(initial.version, None);
        assert_eq!(initial.organisation_marker, Some(23));

        world.memory.low[marker..marker + 4].copy_from_slice(&41_i32.to_le_bytes());
        let memory = VariantVersionFault {
            inner: &world.memory,
            version: variant + engine_native::cequipment_variant::VERSION,
        };
        world.seed(&mut held);
        held.capture_carrying(&memory, CaptureContext::new(fleet::IMAGE), &watches);
        let mut direct = capture(world.census.len());
        world.seed(&mut direct);
        direct.capture_carrying(&memory, CaptureContext::new(fleet::IMAGE), &watches);
        assert_eq!(held.outcomes(), direct.outcomes());
        assert_eq!(
            held.outcomes(),
            [Outcome::Rows {
                len: world.census.len()
            }]
        );
        assert_eq!(
            held.rows.rows[..world.census.len()],
            direct.rows.rows[..world.census.len()]
        );
        let current = held.rows.rows[0].hull_variant.unwrap();
        assert_eq!(current.version, None);
        assert_eq!(current.organisation_marker, Some(41));
        assert!(!watches.poisoned());
    }

    struct SpriteGatherFault<'a> {
        inner: &'a Fleet,
        payload: usize,
        len: usize,
        fail_gather: bool,
    }

    impl Memory for SpriteGatherFault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if self.fail_gather && address == self.payload && output.len() == self.len + 1 {
                return false;
            }
            self.inner.copy(address, output)
        }
    }

    #[test]
    fn notified_sprite_payload_changes_require_complete_revalidation() {
        for (len, fail_gather) in [
            (32, false),
            (32, true),
            (cship::MODEL_TEXT_CAPACITY + 37, false),
        ] {
            let mut world = World::new(4);
            let watches = Watches::enabled(4 * world.census.len());
            let ship = fleet::ship(0);
            let payload = fleet::LOW + world.memory.low.len();
            world.memory.low.resize(world.memory.low.len() + len + 1, 0);
            let at = payload - fleet::LOW;
            world.memory.low[at..at + len].fill(b'a');
            let header_at = ship + cship::FALLBACK_SPRITE - fleet::LOW;
            world.memory.low[header_at..header_at + 8]
                .copy_from_slice(&(payload as u64).to_le_bytes());
            world.memory.low[header_at + 16..header_at + 24]
                .copy_from_slice(&(len as u64).to_le_bytes());
            world.memory.low[header_at + 24..header_at + 32]
                .copy_from_slice(&(len as u64).to_le_bytes());
            let header = world.memory.low[header_at..header_at + 32].to_vec();
            let memory = SpriteGatherFault {
                inner: &world.memory,
                payload,
                len,
                fail_gather,
            };
            let mut fields = cship::ShipFields::new(fleet::IMAGE);
            fields.visit(&memory, ship);
            let observation = fields.held();
            assert_eq!(
                observation.revalidation_complete(),
                !fail_gather && len <= cship::MODEL_TEXT_CAPACITY
            );
            let mut held = capture(world.census.len());
            world.seed(&mut held);
            held.capture_carrying(&memory, CaptureContext::new(fleet::IMAGE), &watches);
            assert_eq!(
                held.outcomes(),
                [Outcome::Rows {
                    len: world.census.len()
                }]
            );
            assert_eq!(
                held.rows.rows[0].fallback_sprite.as_bytes(),
                vec![b'a'; len]
            );

            let writer = watches.notify(ship, 0);
            world.memory.low[at..at + len].fill(b'b');
            drop(writer);
            assert_eq!(world.memory.low[header_at..header_at + 32], header);
            let memory = SpriteGatherFault {
                inner: &world.memory,
                payload,
                len,
                fail_gather,
            };
            fields.visit(&memory, ship);
            assert!(!observation.same(&fields.held()));
            world.seed(&mut held);
            held.capture_carrying(&memory, CaptureContext::new(fleet::IMAGE), &watches);
            let mut direct = capture(world.census.len());
            world.seed(&mut direct);
            direct.capture_carrying(&memory, CaptureContext::new(fleet::IMAGE), &watches);
            assert_eq!(held.outcomes(), direct.outcomes());
            assert_eq!(
                held.outcomes(),
                [Outcome::Rows {
                    len: world.census.len()
                }]
            );
            assert_eq!(
                held.rows.rows[..world.census.len()],
                direct.rows.rows[..world.census.len()]
            );
            assert_eq!(
                held.rows.rows[0].fallback_sprite.as_bytes(),
                vec![b'b'; len]
            );
            assert!(!watches.poisoned());
        }
    }

    struct SubunitGatherFault<'a> {
        inner: &'a Fleet,
        subunit: usize,
    }

    impl Memory for SubunitGatherFault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if self.subunit >= address && self.subunit - address < output.len() {
                return false;
            }
            self.inner.copy(address, output)
        }
    }

    #[test]
    fn cleared_own_subunit_gather_fault_refreshes_the_optional_suffix() {
        let world = World::new(4);
        let watches = Watches::enabled(4 * world.census.len());
        let memory = SubunitGatherFault {
            inner: &world.memory,
            subunit: fleet::ship(0) + cship::SUBUNIT_INDEX,
        };
        let mut held = capture(world.census.len());
        world.seed(&mut held);
        held.capture_carrying(&memory, CaptureContext::new(fleet::IMAGE), &watches);
        assert_eq!(
            held.outcomes(),
            [Outcome::Rows {
                len: world.census.len()
            }]
        );
        assert_eq!(held.rows.rows[0].subunit_index, None);

        let mut fields = cship::ShipFields::new(fleet::IMAGE);
        fields.visit(&memory, fleet::ship(0));
        let observation = fields.held();
        assert!(!observation.revalidation_complete());
        fields.visit_held(&world.memory, &observation);
        assert!(fields.held().revalidation_complete());

        let current = hour(&world, &mut held, &watches);
        let mut direct = capture(world.census.len());
        assert_eq!(current, fresh_hour(&world, &mut direct, &watches));
        assert_eq!(current[0].subunit_index, Some(0));
        assert!(!watches.poisoned());
    }

    #[test]
    fn a_held_location_is_gathered_only_after_its_task_force_moves() {
        let mut world = World::new(4);
        let watches = Watches::enabled(4 * world.census.len());
        let mut held = capture(world.census.len());
        let fresh = hour(&world, &mut held, &watches);
        world.memory.watched.set(world.locations[0]);
        world.memory.hits.set(0);
        assert_eq!(hour(&world, &mut held, &watches), fresh);
        assert_eq!(
            world.memory.hits.get(),
            0,
            "a held location is not gathered"
        );
        world.move_tasks(1);
        world.memory.watched.set(world.locations[1]);
        world.memory.hits.set(0);
        let moved = hour(&world, &mut held, &watches);
        assert!(world.memory.hits.get() > 0, "the new location is gathered");
        let mut direct = capture(world.census.len());
        assert_eq!(moved, fresh_hour(&world, &mut direct, &watches));
        world.memory.hits.set(0);
        assert_eq!(hour(&world, &mut held, &watches), moved);
        assert_eq!(world.memory.hits.get(), 0, "and held again");
    }

    #[test]
    fn a_replaced_task_force_rereads_only_its_ships() {
        let mut world = World::new(4);
        let watches = Watches::enabled(4 * world.census.len());
        let mut held = capture(world.census.len());
        hour(&world, &mut held, &watches);
        let replaced = (77_u64 << 32) | u64::from(engine_native::ctask_force::DEFAULT_HANDLE_TYPE);
        let at = world.tasks[1] + cunit::HANDLE - fleet::LOW;
        world.memory.low[at..at + 8].copy_from_slice(&replaced.to_le_bytes());
        for row in &mut world.census[PER_TASK..2 * PER_TASK] {
            row.task_force = replaced;
        }
        let memory = ShipReads {
            inner: &world.memory,
            ships: RefCell::new(vec![0; world.census.len()]),
        };
        world.seed(&mut held);
        held.capture_carrying(&memory, CaptureContext::new(fleet::IMAGE), &watches);
        assert_eq!(
            held.outcomes(),
            [Outcome::Rows {
                len: world.census.len()
            }]
        );
        for (index, &reads) in memory.ships.borrow().iter().enumerate() {
            let own = (PER_TASK..2 * PER_TASK).contains(&index);
            assert_eq!(reads, usize::from(own), "ship {index}");
        }
        let rows = held.rows.rows[..world.census.len()].to_vec();
        let mut direct = capture(world.census.len());
        assert_eq!(rows, fresh_hour(&world, &mut direct, &watches));
    }

    #[test]
    fn an_invalidated_hour_reads_every_ship_and_location() {
        let world = World::new(4);
        let watches = Watches::enabled(4 * world.census.len());
        let mut held = capture(world.census.len());
        let fresh = hour(&world, &mut held, &watches);
        watches.boundary(true);
        let memory = ShipReads {
            inner: &world.memory,
            ships: RefCell::new(vec![0; world.census.len()]),
        };
        world.memory.watched.set(world.locations[0]);
        world.memory.hits.set(0);
        world.seed(&mut held);
        held.capture_carrying(&memory, CaptureContext::new(fleet::IMAGE), &watches);
        assert_eq!(held.rows.rows[..world.census.len()], fresh);
        assert!(memory.ships.borrow().iter().all(|&reads| reads == 1));
        assert!(world.memory.hits.get() > 0, "locations are gathered again");
    }

    #[test]
    fn only_wholly_carried_hours_keep_the_held_rows_witness() {
        let mut world = World::new(4);
        let watches = Watches::enabled(4 * world.census.len());
        let mut held = capture(world.census.len());
        hour(&world, &mut held, &watches);
        let first = held.held_witness().expect("a complete hour names its rows");
        hour(&world, &mut held, &watches);
        assert_eq!(held.held_witness(), Some(first), "a quiet hour");
        world.census[0].roster_ordinal += 7;
        hour(&world, &mut held, &watches);
        let moved = held.held_witness().unwrap();
        assert_ne!(moved, first, "a moved roster position");
        hour(&world, &mut held, &watches);
        assert_eq!(held.held_witness(), Some(moved));
        let at = fleet::ship(1) + cship::BASE_ORGANISATION - fleet::LOW;
        world.memory.low[at..at + 8].copy_from_slice(&30_000_000_i64.to_le_bytes());
        drop(watches.notify(fleet::ship(1), 0));
        hour(&world, &mut held, &watches);
        let recomputed = held.held_witness().unwrap();
        assert_ne!(recomputed, moved, "a recomputed row");
        world.census.truncate(3 * PER_TASK);
        hour(&world, &mut held, &watches);
        let removed = held.held_witness().unwrap();
        assert_ne!(removed, recomputed, "a removed task force");
        hour(&world, &mut held, &watches);
        assert_eq!(held.held_witness(), Some(removed));
        held.forget_stores();
        hour(&world, &mut held, &watches);
        assert_ne!(held.held_witness(), Some(removed), "a forgotten store");
        world.seed(&mut held);
        held.parent = Outcome::Truncated {
            len: world.census.len(),
            unread_rows: 1,
        };
        held.capture_carrying(&world.memory, CaptureContext::new(fleet::IMAGE), &watches);
        assert_eq!(held.held_witness(), None, "an incomplete hour");
    }

    #[test]
    fn quiet_hours_republish_ship_models_unconverted() {
        use crate::projection::ProjectionScratch;
        use replay_frame::frame::{FrameEncoder, FrameInput, Metrics};
        let mut world = World::new(4);
        let watches = Watches::enabled(4 * world.census.len());
        let mut held = capture(world.census.len());
        let scratch = ProjectionScratch::default();
        let mut optimized = FrameEncoder::new();
        let mut reference = FrameEncoder::new();
        let frame = |hour, [table]: [TableInput; N]| {
            let mut frame = FrameInput {
                branch: 1,
                hour,
                metrics: Metrics::default(),
                tables: std::array::from_fn(|_| {
                    crate::table_conversion::unavailable("fixture table").into()
                }),
            };
            frame.tables[TABLES[0].index()] = table;
            frame
        };
        for (at, republished) in [(0, 0), (1, 1), (2, 1), (3, 2), (4, 2), (5, 3)] {
            if at == 2 {
                world.census[0].roster_ordinal += 7;
            }
            if at == 4 {
                held.forget_stores();
            }
            hour(&world, &mut held, &watches);
            let mut projection = Projection::new(&optimized, 1, at, false, &scratch);
            let [input] = held.table_inputs(Some(&mut projection));
            assert_eq!(
                scratch.republish_counts(TableId::ShipModels)[0],
                republished,
                "hour {at}"
            );
            assert_eq!(
                matches!(input, TableInput::ComparedEqual(_)),
                matches!(at, 1 | 3 | 4 | 5),
                "hour {at}"
            );
            assert_eq!(
                optimized.encode_input(frame(at, [input]), false).unwrap(),
                reference
                    .encode_input(frame(at, held.table_inputs(None)), false)
                    .unwrap(),
                "hour {at}"
            );
        }
    }

    #[test]
    fn a_split_family_witnesses_only_when_every_range_is_quiet() {
        let mut world = World::new(4);
        let watches = Watches::enabled(4 * world.census.len());
        let mut held = Captured::with_capacities(&Capacities {
            rows: world.census.len(),
            modifiers: SnapshotCapacity {
                nodes: 64,
                entries: 64,
            },
        });
        let witness = |world: &World, held: &mut Captured| {
            world.seed(held);
            split_hour(held, &world.memory, &watches);
            assert_eq!(
                held.outcomes(),
                [Outcome::Rows {
                    len: world.census.len()
                }]
            );
            held.held_witness()
        };
        let first = witness(&world, &mut held).expect("a complete split family");
        assert_eq!(witness(&world, &mut held), Some(first), "a quiet hour");
        let last = world.census.len() - 1;
        world.census[last].roster_ordinal += 7;
        let changed = witness(&world, &mut held).unwrap();
        assert_ne!(changed, first, "one range's moved roster position");
        assert_eq!(witness(&world, &mut held), Some(changed));
    }

    fn split_hour(capture: &mut Captured, memory: &impl Memory, changes: &dyn Changes) {
        let parts = capture.split_details(3);
        capture.capture_carrying(memory, CaptureContext::new(fleet::IMAGE), changes);
        for index in 0..parts - 1 {
            let mut part = capture.take_part(index);
            part.capture_carrying(memory, CaptureContext::new(fleet::IMAGE), changes);
            capture.restore_part(index, part);
        }
        capture.finish_split();
    }

    fn assert_stores_forgotten(capture: &Captured) {
        for range in std::iter::once(capture).chain(&capture.extra) {
            let store = &range.store;
            assert!(store.held.is_empty());
            assert!(store.hot.is_empty());
            assert!(store.index.is_empty());
            assert!(store.tasks.is_empty());
            assert!(store.variants.is_empty());
            assert!(store.next.is_empty());
            assert!(store.next_hot.is_empty());
            assert!(store.next_tasks.is_empty());
            assert!(store.next_variants.is_empty());
            assert!(store.next_variant_head.is_empty());
            assert_eq!(store.quiet, None);
            assert_eq!(store.generation, 0);
            assert_eq!(store.rows, None);
            assert!(shared_slot(range.range, |slot| slot.is_none()));
        }
    }

    struct ShipReads<'a> {
        inner: &'a Fleet,
        ships: RefCell<Vec<usize>>,
    }

    impl Memory for ShipReads<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if output.len() == 16 {
                let mut ships = self.ships.borrow_mut();
                for (index, reads) in ships.iter_mut().enumerate() {
                    if address == fleet::ship(index) {
                        *reads += 1;
                    }
                }
            }
            self.inner.copy(address, output)
        }
    }

    fn assert_split_recovery(world: &World, capture: &mut Captured, watches: &Watches) {
        let memory = ShipReads {
            inner: &world.memory,
            ships: RefCell::new(vec![0; world.census.len()]),
        };
        world.seed(capture);
        split_hour(capture, &memory, watches);
        assert_eq!(
            capture.outcomes(),
            [Outcome::Rows {
                len: world.census.len()
            }]
        );
        assert!(memory.ships.borrow().iter().all(|&reads| reads == 1));
        for range in std::iter::once(&*capture).chain(&capture.extra[..2]) {
            assert!(!range.store.held.is_empty());
            assert!(range.store.quiet.is_some());
        }
        let recovered = capture.tables();
        memory.ships.borrow_mut().fill(0);
        world.seed(capture);
        split_hour(capture, &memory, watches);
        assert_eq!(capture.tables(), recovered);
        assert!(memory.ships.borrow().iter().all(|&reads| reads == 0));
    }

    #[test]
    fn failed_split_families_forget_every_range_and_recover() {
        for failed in [0, 4, 8] {
            let world = World::new(4);
            let watches = Watches::enabled(4 * world.census.len());
            let mut held = Captured::with_capacities(&Capacities {
                rows: world.census.len(),
                modifiers: SnapshotCapacity {
                    nodes: 64,
                    entries: 64,
                },
            });
            world.seed(&mut held);
            split_hour(&mut held, &world.memory, &watches);
            drop(watches.notify(fleet::ship(failed), 0));
            world.seed(&mut held);
            split_hour(
                &mut held,
                &CriticalMemory {
                    inner: &world.memory,
                    watched: 0,
                    count_reads: Cell::new(0),
                    fault: Some(fleet::ship(failed) + cship::BASE_ORGANISATION),
                },
                &watches,
            );
            assert!(matches!(held.outcomes()[0], Outcome::Unavailable { .. }));
            assert_stores_forgotten(&held);
            assert_split_recovery(&world, &mut held, &watches);
        }
    }

    #[test]
    fn incomplete_split_censuses_forget_every_range_and_recover() {
        let world = World::new(4);
        for parent in [
            Outcome::Truncated {
                len: world.census.len(),
                unread_rows: 1,
            },
            Outcome::Unavailable {
                reason: "synthetic ship census failed",
            },
            Outcome::Rows {
                len: world.census.len() + 1,
            },
        ] {
            let watches = Watches::enabled(4 * world.census.len());
            let mut held = Captured::with_capacities(&Capacities {
                rows: world.census.len(),
                modifiers: SnapshotCapacity {
                    nodes: 64,
                    entries: 64,
                },
            });
            world.seed(&mut held);
            split_hour(&mut held, &world.memory, &watches);
            world.seed(&mut held);
            held.parent = parent;
            split_hour(&mut held, &world.memory, &watches);
            assert!(!matches!(held.outcomes()[0], Outcome::Rows { .. }));
            assert_stores_forgotten(&held);
            assert_split_recovery(&world, &mut held, &watches);
        }
    }

    #[test]
    fn oversized_split_ranges_forget_every_store_and_recover() {
        let world = World::new(4);
        let watches = Watches::enabled(4 * world.census.len());
        let mut held = Captured::with_capacities(&Capacities {
            rows: world.census.len(),
            modifiers: SnapshotCapacity {
                nodes: 64,
                entries: 64,
            },
        });
        world.seed(&mut held);
        split_hour(&mut held, &world.memory, &watches);
        world.seed(&mut held);
        held.extra[0].rows.rows.truncate(1);
        split_hour(&mut held, &world.memory, &watches);
        assert_eq!(
            held.outcomes(),
            [Outcome::Unavailable {
                reason: "ship model part capacity below its census range"
            }]
        );
        assert_stores_forgotten(&held);
        assert_split_recovery(&world, &mut held, &watches);
    }

    #[test]
    fn terminal_split_families_forget_every_store_including_early_returns() {
        for terminal in 0..4 {
            let world = World::new(4);
            let watches = Watches::enabled(4 * world.census.len());
            let mut held = Captured::with_capacities(&Capacities {
                rows: world.census.len(),
                modifiers: SnapshotCapacity {
                    nodes: 64,
                    entries: 64,
                },
            });
            world.seed(&mut held);
            split_hour(&mut held, &world.memory, &watches);
            match terminal {
                0 => held.terminal_failure = Some("synthetic terminal family failure"),
                1 => held.extra[0].terminal_failure = Some("synthetic terminal part failure"),
                2 => {
                    held.fail_owned_capture("synthetic stopped family");
                }
                3 => {
                    held.terminal_failure = Some("synthetic early terminal capture");
                    held.capture_owned(&world.memory, CaptureContext::new(fleet::IMAGE));
                }
                _ => unreachable!(),
            }
            held.finish_split();
            assert!(held.has_terminal_failure());
            assert_stores_forgotten(&held);
            assert_split_recovery(&world, &mut held, &watches);
        }
    }

    pub(super) fn run(countries: usize) {
        if !ship_model_changes::CARRY {
            panic!("benchmark requires ship_model_changes");
        }
        let mut world = World::new(countries);
        let ships = world.census.len();
        let mut eviction = vec![1_u64; EVICTION_BYTES / std::mem::size_of::<u64>()];
        println!(
            "BENCH ship-model packets: HeldShip {} B, ModelRow {} B, TaskTerms {} B; {countries} distinct country slots/objects and task forces, {ships} ships, {SAMPLES} samples, {} MiB eviction outside capture timers",
            std::mem::size_of::<cship::HeldShip>(),
            std::mem::size_of::<cship::ModelRow>(),
            std::mem::size_of::<cship::TaskTerms>(),
            EVICTION_BYTES / (1024 * 1024),
        );
        for cold in [false, true] {
            for path in [Path::Carried, Path::Moved, Path::Revalidated, Path::Fresh] {
                world.move_tasks(0);
                let watches = Watches::enabled(4 * ships);
                let mut capture = capture(ships);
                for _ in 0..2 {
                    world.seed(&mut capture);
                    capture.capture_carrying(
                        &world.memory,
                        CaptureContext::new(fleet::IMAGE),
                        &watches,
                    );
                    assert_eq!(capture.outcomes(), [Outcome::Rows { len: ships }]);
                }
                let mut samples = Vec::with_capacity(SAMPLES);
                for hour in 1..=SAMPLES {
                    path.prepare(&mut world, &watches, &mut capture, hour);
                    world.seed(&mut capture);
                    if cold {
                        evict(&mut eviction);
                    }
                    let timer = Instant::now();
                    capture.capture_carrying(
                        &world.memory,
                        CaptureContext::new(fleet::IMAGE),
                        &watches,
                    );
                    samples.push(timer.elapsed().as_nanos() as f64 / ships as f64);
                    assert_eq!(capture.outcomes(), [Outcome::Rows { len: ships }]);
                    assert!(!watches.poisoned());
                    check_budget(&world, &capture, &eviction);
                }
                println!(
                    "BENCH ship models {countries} countries, {ships} ships, {} cache, {}: median {:.0} ns per ship",
                    if cold { "cold" } else { "warm" },
                    path.label(),
                    median(samples),
                );
                path.prepare(&mut world, &watches, &mut capture, SAMPLES + 1);
                world.seed(&mut capture);
                account(&world, &mut capture, &watches, path);
            }
            task_terms(&world, &mut eviction, cold);
        }
    }
}

#[test]
#[ignore = "release ship-model carry-cost benchmark"]
fn ship_model_carry_cost_bench_1500_countries() {
    carry_cost_bench::run(1500);
}

#[test]
#[ignore = "release ship-model carry-cost benchmark"]
fn ship_model_carry_cost_bench_5000_countries() {
    carry_cost_bench::run(5000);
}
