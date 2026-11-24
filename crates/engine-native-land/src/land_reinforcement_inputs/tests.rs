use super::*;
use engine_native_memory::memory::SliceMemory;
use std::cell::{Cell, RefCell};

struct PacketMemory {
    fields: Vec<(usize, Vec<u8>)>,
    reads: RefCell<Vec<(usize, usize)>>,
    scatters: Cell<usize>,
    fault: Option<usize>,
}

impl PacketMemory {
    fn new(fields: &[(usize, Vec<u8>)], fault: Option<usize>) -> Self {
        Self {
            fields: fields.to_vec(),
            reads: RefCell::default(),
            scatters: Cell::new(0),
            fault,
        }
    }
}

impl Memory for PacketMemory {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let ordinal = self.reads.borrow().len();
        self.reads.borrow_mut().push((address, output.len()));
        if self.fault == Some(ordinal) {
            let partial = output.len().div_ceil(2);
            output[..partial].fill(0xed);
            return false;
        }
        let Some((_, value)) = self
            .fields
            .iter()
            .find(|(source, value)| *source == address && value.len() == output.len())
        else {
            return false;
        };
        output.copy_from_slice(value);
        true
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.scatters.set(self.scatters.get() + 1);
        for (ordinal, (address, destination)) in addresses.into_iter().zip(destinations).enumerate()
        {
            if !self.copy(address, destination) {
                return ordinal;
            }
        }
        N
    }
}

#[test]
fn packets_preserve_native_values_exact_spans_and_every_fault_prefix() {
    let fields = [
        (0x124c, vec![0xff]),
        (0x124d, vec![0x80]),
        (0x12ae, vec![0]),
        (0x102e, vec![0x7f]),
        (0x1600, i64::MIN.to_le_bytes().to_vec()),
        (0x1610, i64::MAX.to_le_bytes().to_vec()),
        (0x1040, (-1_i64).to_le_bytes().to_vec()),
        (0x1058, 0_i64.to_le_bytes().to_vec()),
        (0x1498, i32::MIN.to_le_bytes().to_vec()),
        (0x033e_3678, i32::MAX.to_le_bytes().to_vec()),
        (0x033e_3844, (-1_i32).to_le_bytes().to_vec()),
        (0x033e_3990, (-987_654_i64).to_le_bytes().to_vec()),
    ];
    let values = [
        0xff,
        0x80,
        0,
        0x7f,
        i64::MIN,
        i64::MAX,
        -1,
        0,
        i64::from(i32::MIN),
        i64::from(i32::MAX),
        -1,
        -987_654,
    ];
    for supply in [false, true] {
        let range = if supply { 4..12 } else { 0..4 };
        let fields = &fields[range.clone()];
        let trace: Vec<_> = fields
            .iter()
            .map(|(at, bytes)| (*at, bytes.len()))
            .collect();
        for fault in std::iter::once(None).chain((0..fields.len()).map(Some)) {
            let memory = PacketMemory::new(fields, fault);
            let result = if supply {
                supply_scalars(&memory, 0, 0x1000).map(Vec::from)
            } else {
                formation_flags(&memory, 0x1000)
                    .map(|flags| flags.into_iter().map(i64::from).collect())
            };
            let expected = fault.map_or_else(
                || Ok(values[range.clone()].to_vec()),
                |_| Err("reinforcement input unreadable"),
            );
            assert_eq!(result, expected);
            assert_eq!(memory.scatters.get(), 1);
            assert_eq!(
                *memory.reads.borrow(),
                trace[..fault.map_or(fields.len(), |n| n + 1)]
            );
        }
    }
}

#[test]
fn packet_failure_keeps_partial_field_and_later_destinations_untouched() {
    let fields = [
        (0, vec![0xcc; 4]),
        (0x1000, vec![1, 2, 3, 4]),
        (0x1010, vec![5, 6, 7, 8]),
        (0x1020, vec![9, 10, 11, 12]),
    ];
    let memory = PacketMemory::new(&fields, Some(1));
    let mut values = [[0xa5; 4]; 3];
    assert_eq!(
        copy_input_spans(
            &memory,
            [(0x1000, 0), (0x1000, 0x10), (0x1000, 0x20)],
            &mut values.each_mut().map(|value| value.as_mut_slice()),
        ),
        Err("reinforcement input unreadable")
    );
    assert_eq!(values, [[1, 2, 3, 4], [0xed, 0xed, 0xa5, 0xa5], [0xa5; 4]]);
    assert_eq!(*memory.reads.borrow(), [(0x1000, 4), (0x1010, 4)]);
    let mut zero = [0; 4];
    assert!(PacketMemory::new(&fields, None).copy(0, &mut zero));
    assert_eq!(zero, [0xcc; 4]);
    let memory = PacketMemory::new(&fields, None);
    let mut values = [[0xa5; 4]; 3];
    assert_eq!(
        copy_input_spans(
            &memory,
            [(0x1000, 0), (usize::MAX, 1), (0x1000, 0x20)],
            &mut values.each_mut().map(|value| value.as_mut_slice()),
        ),
        Err("reinforcement input unreadable")
    );
    assert_eq!(values, [[1, 2, 3, 4], [0xa5; 4], [0xa5; 4]]);
    assert_eq!(*memory.reads.borrow(), [(0x1000, 4)]);
    assert_eq!(memory.scatters.get(), 0);
}

struct Counted<T> {
    inner: T,
    copies: Cell<usize>,
    bytes: Cell<usize>,
}
impl<T: Memory> Memory for Counted<T> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.copies.set(self.copies.get() + 1);
        self.bytes.set(self.bytes.get() + output.len());
        self.inner.copy(address, output)
    }
}

#[test]
fn raw_modifier_preserves_negative_values_absence_and_order() {
    let mut bytes = vec![0_u8; 0x300];
    bytes[0x10..0x18].copy_from_slice(&0x100_u64.to_le_bytes());
    bytes[0x18..0x1c].copy_from_slice(&1_u32.to_le_bytes());
    bytes[0x1c..0x20].copy_from_slice(&1_u32.to_le_bytes());
    bytes[0x100..0x104].copy_from_slice(&5_u32.to_le_bytes());
    bytes[0x108..0x110].copy_from_slice(&(-170_005_i64).to_le_bytes());
    struct WithMetadata<'a>(&'a [u8]);
    impl Memory for WithMetadata<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == engine_native_parts::modifier_definition::TABLE_RVA.0 {
                output.copy_from_slice(&0x10000_u64.to_le_bytes());
                return true;
            }
            if address
                == 0x10000
                    + 5 * engine_native_parts::modifier_definition::STRIDE
                    + engine_native_parts::modifier_definition::FLAGS
            {
                output.fill(0);
                output[..4].copy_from_slice(&7_u32.to_le_bytes());
                return true;
            }
            SliceMemory {
                base: 0,
                bytes: self.0,
            }
            .copy(address, output)
        }
    }
    let mut output = vec![ModifierTerm::default(); 3];
    let mut len = 0;
    let counted = Counted {
        inner: WithMetadata(&bytes),
        copies: Cell::new(0),
        bytes: Cell::new(0),
    };
    append_modifiers(
        &counted,
        0,
        &mut Sources::new(),
        0x10,
        4,
        &[6, 5, 7],
        &mut Buffer::Growing(&mut output),
        &mut len,
    )
    .unwrap();
    assert_eq!(len, 3);
    assert_eq!(output[0].present, 0);
    assert_eq!(output[1].raw, -170_005);
    assert_eq!(output[1].metadata_flags, 7);
    assert_eq!(output[1].scope, 4);
    assert_eq!(output[2].key, 7);
    assert_eq!((counted.copies.get(), counted.bytes.get()), (4, 52));
}

#[test]
fn a_failed_census_leaves_no_entries_in_the_arena() {
    let mut bytes = vec![0_u8; 0x400];
    let mut put = |at: usize, value: u64| bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    put(0x100, 0x200);
    put(0x108, (3_u64 << 32) | 3);
    for (index, key) in [5_u64, 7, 6].into_iter().enumerate() {
        put(0x200 + index * 16, key);
        put(0x208 + index * 16, 100 + index as u64);
    }
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let mut arena = vec![(1, 1)];
    assert_eq!(
        census(&memory, 0x100, &mut arena),
        Err("reinforcement modifier order invalid")
    );
    assert_eq!(arena, [(1, 1)]);
}

#[test]
fn transport_without_static_feature_preserves_branch_without_normal_dependencies() {
    let mut bytes = vec![0_u8; 0x9000];
    let mut put = |at: usize, value: u64| bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    put(0x1000, carmy::VTABLE_RVA.0 as u64);
    put(0x1000 + cunit::HANDLE, 0x1234_0000_0033);
    put(0x1000 + cunit::STAT_BLOCK, 0x2000);
    put(0x2000, engine_native_parts::csub_unit_definition::VTABLE_RVA.0 as u64);
    put(0x1000 + cunit::LOCATION, 0x3000);
    put(0x1000 + carmy::NAVAL_TRANSFER_MEMBERSHIP, 42);
    put(0x1000 + cunit::LOGICAL_OWNER, 1);
    put(0x3000, cprovince::VTABLE_RVA.0 as u64);
    put(0x3000 + cprovince::DEFINITION, 0x3800);
    put(0x3000 + cprovince::ID, 14);
    put(0x3800 + engine_native::cprovince::definition::ID, 14);
    put(0x3000 + cprovince::STRATEGIC_REGION, 0x4000);
    put(0x4000, engine_native_parts::cstrategic_region::VTABLE_RVA.0 as u64);
    put(0x4000 + engine_native_parts::cstrategic_region::ID, 22);
    put(0x7000 + cgame_state::WEATHER_MANAGER, 0x5000);
    put(0x5000, engine_native::cweather_manager::VTABLE_RVA.0 as u64);
    put(0x7000 + cgame_state::COUNTRIES, 0x8000);
    put(0x7000 + cgame_state::COUNTRIES + 8, 0x2_0000_0002);
    put(0x8008, 0x6000);
    put(0x7000 + cgame_state::COUNTRY_LINKS, 0x8100);
    put(0x7000 + cgame_state::COUNTRY_LINKS + 8, 0x2_0000_0002);
    put(0x8100, 0x1_0000_0000);
    put(0x6000, ccountry::VTABLE_RVA.0 as u64);
    put(0x6000 + ccountry::ID, 1);
    struct Source<'a>(&'a [u8]);
    impl Memory for Source<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            assert_ne!(address, 0x1000 + carmy::STANDARD_TEMPLATE);
            assert!(
                ![
                    0x124c, 0x124d, 0x12ae, 0x102e, 0x1600, 0x1610, 0x1040, 0x1058, 0x1498
                ]
                .contains(&address),
                "naval transfer reached a normal land packet"
            );
            if [
                engine_native_parts::modifier_definition::ELIGIBILITY_CALLBACK_RVA.0,
                engine_native_parts::modifier_definition::TRADE_FEATURE_FLAGS_RVA.0,
            ]
            .contains(&address)
            {
                output.fill(0);
                return true;
            }
            SliceMemory {
                base: 0,
                bytes: self.0,
            }
            .copy(address, output)
        }
    }
    let row = capture_context_buffer(&Source(&bytes), 0, 0x7000, &mut Sources::new(), 0x1000, 0x1234_0000_0033, engine_native_memory::walk::Buffer::Count, engine_native_memory::walk::Buffer::Count, engine_native_memory::walk::Buffer::Count, &mut Vec::new(), 0)
    .unwrap();
    assert_eq!(row.flags[11], 2);
    assert_eq!(row.ids[0], 14);
    assert_eq!(row.ids[7], 22);
    assert_eq!((row.modifier_count, row.transport_term_count), (0, 0));
}

#[test]
fn acclimatization_retains_native_ties_and_active_reference_join() {
    let mut bytes = vec![0_u8; 0x1000];
    let object = 0x100 + carmy::ACCLIMATIZATION;
    bytes[object + 8..object + 16].copy_from_slice(&0x800_u64.to_le_bytes());
    bytes[object + 16..object + 20].copy_from_slice(&2_u32.to_le_bytes());
    bytes[object + 20..object + 24].copy_from_slice(&2_u32.to_le_bytes());
    bytes[object + 0x20..object + 0x28].copy_from_slice(&0x900_u64.to_le_bytes());
    bytes[object + 0x38..object + 0x40].copy_from_slice(&123_i64.to_le_bytes());
    for (index, pointer) in [0x900_u64, 0x920].into_iter().enumerate() {
        bytes[0x800 + index * 16..0x808 + index * 16].copy_from_slice(&pointer.to_le_bytes());
        bytes[0x808 + index * 16..0x810 + index * 16].copy_from_slice(&99_i64.to_le_bytes());
    }
    bytes[0x930..0x938].copy_from_slice(&(-500_i64).to_le_bytes());
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let mut row = ContextRow::default();
    let mut terms = vec![AcclimatizationTerm::default(); 2];
    acclimatization(&memory, 0x100, &mut row, &mut Buffer::Growing(&mut terms)).unwrap();
    assert_eq!(
        terms,
        [
            AcclimatizationTerm {
                weight: 99,
                active: 1
            },
            AcclimatizationTerm {
                weight: 99,
                active: 0
            }
        ]
    );
    assert_eq!(row.scalar_values[0], 123);
    assert_eq!(row.scalar_values[1], -500);
}

#[test]
fn worker_acclimatization_has_no_chosen_limit_and_retains_later_growth() {
    let mut terms = Vec::new();
    for count in [50, 75] {
        let mut bytes = vec![0_u8; 0x1000 + count * 16];
        let object = 0x100 + carmy::ACCLIMATIZATION;
        bytes[object + 8..object + 16].copy_from_slice(&0x1000_u64.to_le_bytes());
        bytes[object + 16..object + 20].copy_from_slice(&(count as u32).to_le_bytes());
        bytes[object + 20..object + 24].copy_from_slice(&(count as u32).to_le_bytes());
        bytes[object + 0x20..object + 0x28].copy_from_slice(&0x900_u64.to_le_bytes());
        for index in 0..count {
            let at = 0x1000 + index * 16;
            bytes[at..at + 8].copy_from_slice(&0x900_u64.to_le_bytes());
            bytes[at + 8..at + 16].copy_from_slice(&(-(index as i64)).to_le_bytes());
        }
        let mut row = ContextRow::default();
        acclimatization(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0x100,
            &mut row,
            &mut Buffer::Growing(&mut terms),
        )
        .unwrap();
        assert_eq!(row.acclimatization_count, count);
        assert_eq!(terms[count - 1].weight, -(count as i64 - 1));
        assert_eq!(terms[count - 1].active, 1);
    }
}

#[test]
fn worker_modifier_batches_keep_mod_sized_populations_on_the_original_pass() {
    struct Shared;
    impl Memory for Shared {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == engine_native_parts::modifier_definition::TABLE_RVA.0 && output.len() == 8 {
                output.copy_from_slice(&0x100_0000_u64.to_le_bytes());
                return true;
            }
            let metadata = address.checked_sub(0x100_0000 + engine_native_parts::modifier_definition::FLAGS);
            if metadata.is_some_and(|offset| offset % engine_native_parts::modifier_definition::STRIDE == 0)
                && output.len() == 12
            {
                output.fill(0);
                return true;
            }
            false
        }
        fn modifier_accumulator_value(
            &self,
            _: usize,
            key: u32,
        ) -> Option<Result<engine_native_memory::modifier_cache::Value, &'static str>> {
            Some(Ok(engine_native_memory::modifier_cache::Value {
                raw: -(key as i64),
                present: true,
            }))
        }
    }
    let mut output = Vec::new();
    for count in [50, 75] {
        let keys: Vec<_> = (1..=count as u32).collect();
        let mut len = 0;
        append_modifiers(
            &Shared,
            0,
            &mut Sources::new(),
            0x1000,
            6,
            &keys,
            &mut Buffer::Growing(&mut output),
            &mut len,
        )
        .unwrap();
        assert_eq!(len, count);
        assert_eq!(output[count - 1].key, count as u32);
        assert_eq!(output[count - 1].raw, -(count as i64));
    }
}

#[test]
fn supply_and_priority_category_arenas_keep_all_mod_sized_occurrences() {
    let mut supply = Vec::new();
    let mut categories = Vec::new();
    for count in [50, 75] {
        let mut memory = Sparse::default();
        memory.u64(
            0x1000,
            engine_native::creferenced_division_template::VTABLE_RVA.0 as u64,
        );
        memory.u64(
            0x1000 + engine_native::creferenced_division_template::PRIORITY_UNIT,
            0x2000,
        );
        memory.array(0x1000 + 0xc0, 0x10_0000, count as u32);
        memory.u64(0x2000, engine_native_parts::csub_unit_definition::VTABLE_RVA.0 as u64);
        memory.u32(0x2008, 7);
        memory.u32(0x2000 + 0x5d8, 9);
        memory.put(0x2000 + 0x5b7, &[1]);
        memory.array(0x2000 + 0x508, 0x20_0000, count as u32);
        memory.u32(0x3000 + 0x60, 11);
        for index in 0..count {
            memory.u64(0x10_0000 + index * 16, 0x2000);
            memory.u32(0x10_0000 + index * 16 + 8, index as u32);
            memory.u64(0x20_0000 + index * 8, 0x3000);
        }
        let mut row = ContextRow::default();
        supply_terms(
            &memory,
            0,
            &mut Sources::new(),
            0x1000,
            &mut row,
            &mut Buffer::Growing(&mut supply),
        )
        .unwrap();
        assert_eq!(row.supply_term_count, count);
        assert_eq!(supply[count - 1].count, count as i32 - 1);
        assert_eq!(supply[count - 1].special, 1);
        categories.clear();
        let selected = priority(&memory, 0, 0x1000, &mut categories).unwrap();
        let selected = selected.categories.unwrap();
        assert_eq!(selected.count, count as u32);
        assert_eq!(selected.tokens.len, count);
        assert_eq!(selected.failure, None);
        assert_eq!(categories[count - 1], 11);
    }
}

#[derive(Default)]
struct Sparse(std::collections::BTreeMap<usize, Vec<u8>>);
impl Sparse {
    fn put(&mut self, at: usize, bytes: &[u8]) {
        self.0.insert(at, bytes.to_vec());
    }
    fn u64(&mut self, at: usize, value: u64) {
        self.put(at, &value.to_le_bytes());
    }
    fn u32(&mut self, at: usize, value: u32) {
        self.put(at, &value.to_le_bytes());
    }
    fn array(&mut self, at: usize, data: usize, count: u32) {
        let mut header = [0; 16];
        header[..8].copy_from_slice(&(data as u64).to_le_bytes());
        header[8..12].copy_from_slice(&count.to_le_bytes());
        header[12..].copy_from_slice(&count.to_le_bytes());
        self.put(at, &header);
    }
}
impl Memory for Sparse {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let Some((start, bytes)) = self.0.range(..=address).next_back() else {
            return false;
        };
        let offset = address - start;
        match bytes.get(offset..offset + output.len()) {
            Some(source) => {
                output.copy_from_slice(source);
                true
            }
            None => false,
        }
    }
}

const GAME: usize = 0x10_0000;
const ARMY: usize = 0x20_0000;
const HANDLE: u64 = 0x77;
fn diplomacy_at(slot: usize) -> usize {
    0x800_0000 + slot * 0x400
}

fn transport_fixture(enemies: u32) -> (Sparse, Vec<TransportTerm>) {
    use engine_native_intelligence::{ccountry_intel, country_intel_row};
    use engine_native_politics::cdiplomacy_status;
    use engine_native::{cweather_manager, cprovince::definition as province_definition};
    use engine_native_naval::{cstrategic_navy, cstrategic_navy_manager};
    use engine_native_parts::{
        cstatic_modifier,
        cstrategic_region,
        modifier_definition as metadata,
    };
    const STATS: usize = 0x21_0000;
    const LOCATION: usize = 0x22_0000;
    const REGION: usize = 0x23_0000;
    const WEATHER: usize = 0x24_0000;
    const DEFINITIONS: usize = 0x25_0000;
    const STATIC: usize = 0x26_0000;
    const META: usize = 0x27_0000;
    const NAVY_MANAGER: usize = 0x28_0000;
    const NAVIES: usize = 0x29_0000;
    const COUNTRIES: usize = 0x2a_0000;
    const LINKS: usize = 0x2b_0000;
    const ENEMIES: usize = 0x2c_0000;
    const OWNER_ROWS: usize = 0x2d_0000;
    let country = |slot: usize| 0x100_0000 + slot * 0x1000;
    let intel = |slot: usize| 0x1000_0000 + slot * 0x100;
    let intel_rows = |slot: usize| 0x1800_0000 + slot * 0x40;
    let navy = |slot: usize| 0x2000_0000 + slot * 0x100;
    let regional = |slot: usize| 0x3000_0000 + slot * 0x100;
    let slots = enemies as usize + 2;
    let mut m = Sparse::default();
    m.u64(ARMY, carmy::VTABLE_RVA.0 as u64);
    m.u64(ARMY + cunit::HANDLE, HANDLE);
    m.u64(ARMY + carmy::NAVAL_TRANSFER_MEMBERSHIP, 1);
    m.u64(ARMY + cunit::STAT_BLOCK, STATS as u64);
    m.u64(ARMY + cunit::LOCATION, LOCATION as u64);
    m.u32(ARMY + cunit::LOGICAL_OWNER, 1);
    m.u64(STATS, engine_native_parts::csub_unit_definition::VTABLE_RVA.0 as u64);
    m.u64(LOCATION, cprovince::VTABLE_RVA.0 as u64);
    m.u64(LOCATION + cprovince::DEFINITION, LOCATION as u64 + 0x1000);
    m.u32(LOCATION + cprovince::ID, 14);
    m.u32(LOCATION + 0x1000 + province_definition::ID, 14);
    m.u64(LOCATION + cprovince::STRATEGIC_REGION, REGION as u64);
    m.u64(REGION, cstrategic_region::VTABLE_RVA.0 as u64);
    m.u32(REGION + cstrategic_region::ID, 22);
    m.u64(GAME + cgame_state::WEATHER_MANAGER, WEATHER as u64);
    m.u64(WEATHER, cweather_manager::VTABLE_RVA.0 as u64);
    m.array(WEATHER + cweather_manager::REGIONS, 0, 0);
    m.u32(metadata::TRADE_FEATURE_FLAGS_RVA.0, 1 << 4);
    m.u64(metadata::ELIGIBILITY_CALLBACK_RVA.0, 0);
    m.u64(0x033d_e590, DEFINITIONS as u64);
    m.u64(DEFINITIONS + 0xf8, STATIC as u64);
    m.u64(STATIC, cstatic_modifier::VTABLE_RVA.0 as u64);
    m.array(STATIC + cstatic_modifier::ACCUMULATOR, 0, 0);
    m.u64(metadata::TABLE_RVA.0, META as u64);
    m.u32(
        META + 0x29 * metadata::STRIDE + metadata::CATEGORY_MASK,
        0xabc,
    );
    for (index, rva) in [0x033e_1d68, 0x033e_1000, 0x033e_1088, 0x033e_2200]
        .into_iter()
        .enumerate()
    {
        m.put(rva, &(-(index as i64) - 1).to_le_bytes());
    }
    m.u64(
        GAME + cgame_state::STRATEGIC_NAVY_MANAGER,
        NAVY_MANAGER as u64,
    );
    m.u64(NAVY_MANAGER, cstrategic_navy_manager::VTABLE_RVA.0 as u64);
    m.array(
        NAVY_MANAGER + cstrategic_navy_manager::NAVIES,
        NAVIES,
        slots as u32,
    );
    m.array(GAME + cgame_state::COUNTRIES, COUNTRIES, slots as u32);
    m.array(GAME + cgame_state::COUNTRY_LINKS, LINKS, slots as u32);
    let mut countries = vec![0; slots * 8];
    let mut links = vec![0; slots * 4];
    let mut navies = vec![0; slots * 8];
    let mut owner_rows = vec![0; slots * country_intel_row::STRIDE];
    let mut expected = Vec::new();
    for slot in 1..slots {
        let c = country(slot);
        countries[slot * 8..slot * 8 + 8].copy_from_slice(&(c as u64).to_le_bytes());
        links[slot * 4..slot * 4 + 4].copy_from_slice(&(slot as u32).to_le_bytes());
        navies[slot * 8..slot * 8 + 8].copy_from_slice(&(navy(slot) as u64).to_le_bytes());
        let row = slot * country_intel_row::STRIDE + country_intel_row::NAVY;
        owner_rows[row..row + 8].copy_from_slice(&(1000 + slot as i64).to_le_bytes());
        m.u64(c, ccountry::VTABLE_RVA.0 as u64);
        m.u32(c + ccountry::ID, slot as u32);
        let fog = i32::from(slot % 11 != 0);
        m.put(c + ccountry::FOG_NETWORK_TABLE_COUNT, &fog.to_le_bytes());
        m.u64(c + ccountry::DIPLOMACY_STATUS, diplomacy_at(slot) as u64);
        m.u64(c + ccountry::INTEL_TABLE, intel(slot) as u64);
        m.u64(diplomacy_at(slot), engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA.0 as u64);
        m.u64(diplomacy_at(slot) + engine_native_parts::cdiplomacy_status_layout::COUNTRY, c as u64);
        let capitulated = u8::from(slot % 7 == 0);
        m.put(
            diplomacy_at(slot) + cdiplomacy_status::CAPITULATED,
            &[capitulated],
        );
        m.u64(intel(slot), ccountry_intel::VTABLE_RVA.0 as u64);
        m.put(
            intel(slot) + ccountry_intel::OWNER,
            &(slot as i32).to_le_bytes(),
        );
        m.u64(navy(slot), cstrategic_navy::VTABLE_RVA.0 as u64);
        m.u64(
            navy(slot) + cstrategic_navy::SECONDARY,
            cstrategic_navy::SECONDARY_VTABLE_RVA.0 as u64,
        );
        m.u32(navy(slot) + cstrategic_navy::OWNER, slot as u32);
        m.array(
            navy(slot) + cstrategic_navy::REGIONAL_OPERANDS,
            regional(slot),
            23,
        );
        let mut operands = vec![0; 23 * 8];
        operands[22 * 8..].copy_from_slice(&(slot as i64 * 10).to_le_bytes());
        m.put(regional(slot), &operands);
        if slot == 1 {
            m.array(c + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR, 0, 0);
            m.array(
                diplomacy_at(slot) + engine_native_parts::cdiplomacy_status_layout::ENEMY_COUNTRIES,
                ENEMIES,
                enemies,
            );
            m.array(intel(slot) + ccountry_intel::DATA, OWNER_ROWS, slots as u32);
        } else {
            m.array(intel(slot) + ccountry_intel::DATA, intel_rows(slot), 2);
            let mut rows = vec![0; 2 * country_intel_row::STRIDE];
            let row = country_intel_row::STRIDE + country_intel_row::NAVY;
            rows[row..row + 8].copy_from_slice(&(-(slot as i64)).to_le_bytes());
            m.put(intel_rows(slot), &rows);
            if slot % 7 != 0 && slot % 11 != 0 {
                expected.push(TransportTerm {
                    country: slot as u32,
                    regional_value: slot as i64 * 10,
                    own_intel: 1000 + slot as i64,
                    enemy_intel: -(slot as i64),
                });
            }
        }
    }
    let raws: Vec<u8> = (2..slots as u32).flat_map(u32::to_le_bytes).collect();
    m.put(ENEMIES, &raws);
    m.put(COUNTRIES, &countries);
    m.put(LINKS, &links);
    m.put(NAVIES, &navies);
    m.put(OWNER_ROWS, &owner_rows);
    (m, expected)
}

#[test]
fn transport_terms_follow_every_native_enemy_into_caller_storage() {
    for enemies in [3, 5_000] {
        let (memory, expected) = transport_fixture(enemies);
        let sentinel = TransportTerm {
            country: 0xdead,
            ..TransportTerm::default()
        };
        let mut terms = vec![sentinel; 4];
        let row = capture_context_buffer(&memory, 0, GAME, &mut Sources::new(), ARMY, HANDLE, engine_native_memory::walk::Buffer::Growing(&mut vec![ModifierTerm::default(); 2]), engine_native_memory::walk::Buffer::Count, engine_native_memory::walk::Buffer::Count, &mut terms, 2)
        .unwrap();
        assert_eq!(row.flags[11], 2);
        assert_eq!(row.scalar_values[25..30], [-1, -2, -3, -4, 0xabc]);
        assert_eq!(row.transport_term_count, expected.len());
        assert_eq!(terms[..2], [sentinel; 2]);
        assert_eq!(terms[2..2 + expected.len()], expected[..]);
        if enemies > 64 {
            assert!(expected.len() > 64 && expected.len() < enemies as usize);
        }
    }
}

#[test]
fn transport_enemy_failure_is_not_a_shorter_enemy_list() {
    let (mut memory, _) = transport_fixture(5_000);
    memory
        .0
        .remove(&(diplomacy_at(4_901) + engine_native_politics::cdiplomacy_status::CAPITULATED));
    let result = capture_context_buffer(&memory, 0, GAME, &mut Sources::new(), ARMY, HANDLE, engine_native_memory::walk::Buffer::Growing(&mut vec![ModifierTerm::default(); 2]), engine_native_memory::walk::Buffer::Count, engine_native_memory::walk::Buffer::Count, &mut Vec::new(), 0);
    assert_eq!(result, Err("reinforcement input unreadable"));
}
