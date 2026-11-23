use super::*;
use engine_native_memory::memory::SliceMemory;
use std::cell::{Cell, RefCell};

struct NameArena {
    groups: engine_native::cname_group_member::GroupKeys,
    bytes: Vec<u8>,
}
impl NameArena {
    fn new() -> Self {
        Self {
            groups: engine_native::cname_group_member::GroupKeys::with_capacity(64, 64),
            bytes: vec![0; 4096],
        }
    }
    fn stats(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: usize,
        handle: u64,
    ) -> Result<Row, &'static str> {
        let mut names = engine_native::cname_group_member::Names {
            bytes: engine_native_memory::walk::Buffer::Growing(&mut self.bytes),
            len: 0,
            required: 0,
        };
        capture_stats(
            memory,
            base,
            &mut crate::combat_sources::Sources::new(),
            pointer,
            handle,
            &mut self.groups,
            &mut names,
        )
    }
    fn text(&self, span: (usize, usize)) -> &[u8] {
        &self.bytes[span.0..span.0 + span.1]
    }
}

fn capture_context_at(
    memory: &impl Memory,
    base: usize,
    state: usize,
    sources: &mut crate::combat_sources::Sources,
    pointer: usize,
    expected_handle: u64,
    terms: &mut Vec<WidthTerm>,
) -> Result<ContextRow, &'static str> {
    let countries = engine_native::ccountry::Reader::new(memory, base, state)?;
    capture_context(
        memory,
        base,
        &countries,
        sources,
        pointer,
        expected_handle,
        terms,
    )
}

struct Observed<'a, M> {
    source: &'a M,
    reads: RefCell<Vec<(usize, usize)>>,
    scatters: RefCell<Vec<Vec<(usize, usize)>>>,
    fault: Option<usize>,
    change: Option<(usize, usize)>,
}

impl<'a, M> Observed<'a, M> {
    fn new(source: &'a M) -> Self {
        Self {
            source,
            reads: RefCell::default(),
            scatters: RefCell::default(),
            fault: None,
            change: None,
        }
    }
}

impl<M: Memory> Memory for Observed<'_, M> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let ordinal = self.reads.borrow().len();
        self.reads.borrow_mut().push((address, destination.len()));
        if self.fault == Some(ordinal) {
            let prefix = destination.len().div_ceil(2);
            destination[..prefix].fill(0xed);
            return false;
        }
        let copied = self.source.copy(address, destination);
        if copied
            && let Some((at, byte)) = self.change
            && at == ordinal
        {
            destination[byte] ^= 1;
        }
        copied
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.scatters.borrow_mut().push(
            addresses
                .iter()
                .zip(destinations.iter())
                .map(|(&p, d)| (p, d.len()))
                .collect(),
        );
        for (index, (address, destination)) in addresses.into_iter().zip(destinations).enumerate() {
            if !read_slice(self, address, destination) {
                return index;
            }
        }
        N
    }
}

fn scalar_stats(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    expected_handle: u64,
) -> Result<Row, &'static str> {
    let identity = bytes::<8>(memory, pointer, 0)?;
    let expected_vtable = engine_sites::at(base, carmy::VTABLE_RVA)
        .ok_or("land combat division identity mismatch")? as u64;
    let handle = bytes::<8>(memory, pointer, cunit::HANDLE)?;
    if pointer == 0
        || u64::from_le_bytes(identity) != expected_vtable
        || expected_handle == 0
        || u64::from_le_bytes(handle) != expected_handle
    {
        return Err("land combat division identity mismatch");
    }
    let stat_pointer = bytes::<8>(memory, pointer, cunit::STAT_BLOCK)?;
    let stat = usize::try_from(u64::from_le_bytes(stat_pointer))
        .map_err(|_| "land combat division stat pointer invalid")?;
    let stat_identity = bytes::<8>(memory, stat, 0)?;
    if stat == 0
        || Some(u64::from_le_bytes(stat_identity))
            != engine_sites::at(base, csub_unit_definition::VTABLE_RVA).map(|p| p as u64)
    {
        return Err("land combat division stat identity mismatch");
    }
    let armour = values::<2>(&bytes::<16>(memory, stat, ARMOUR)?);
    let group = u64::from_le_bytes(bytes::<8>(memory, pointer, carmy::ORDERS_GROUP)?) as usize;
    let member = u64::from_le_bytes(bytes::<8>(memory, pointer, NAME_GROUP_MEMBER)?);
    let strength_combat_factor = i64::from_le_bytes(bytes(memory, pointer, 0x468)?);
    let fuel_inputs = [
        i64::from_le_bytes(bytes(memory, pointer, 0x620)?),
        i64::from_le_bytes(bytes(memory, stat, 0x120)?),
    ];
    let stat_values = values(&bytes::<40>(memory, stat, csub_unit_definition::DEFENSE)?);
    let air_attack = i64::from_le_bytes(bytes(memory, stat, AIR_ATTACK)?);
    let reconnaissance = i64::from_le_bytes(bytes(memory, stat, 0xd0)?);
    let initiative = i64::from_le_bytes(bytes(memory, stat, 0xe0)?);
    let scales = values(&bytes::<32>(memory, pointer, cunit::ATTACK_ACCUMULATOR)?);
    let direction_group = i32::from_le_bytes(bytes(memory, pointer, DIRECTION_GROUP)?);
    let categories = u64::from_le_bytes(bytes(memory, stat, csub_unit_definition::AIR_FLAGS)?);
    let commander = commander(memory, base, group)?;
    Ok(Row {
        division: expected_handle,
        naming: naming(
            memory,
            base,
            member,
            &mut engine_native::cname_group_member::GroupKeys::with_capacity(64, 64),
            &mut engine_native::cname_group_member::Names {
                bytes: engine_native_memory::walk::Buffer::Growing(&mut vec![0; 4096]),
                len: 0,
                required: 0,
            },
        )?,
        strength_combat_factor,
        fuel_inputs,
        stat_values,
        armour: armour[0],
        piercing: armour[1],
        air_attack,
        reconnaissance,
        initiative,
        commander,
        scales,
        direction_group,
        categories,
    })
}

#[test]
fn stats_scatter_matches_scalar_first_values_and_every_fault() {
    let mut source = context_fixture();
    source.u64(0x10c0, 0xe000);
    source.u64(0xe000, engine_native::corders_group::VTABLE_RVA.0 as u64);
    source.u64(0xe008, 1_u64 << 32 | 53);
    source.u64(0xe088, 0xf000);
    source.u64(0xf000, engine_native::carmy_leader::VTABLE_RVA.0 as u64);
    source.u64(0xf008, 77_u64 << 32 | 4713);
    for (index, (address, count)) in [
        (0x2298, 2),
        (0x1468, 1),
        (0x1620, 1),
        (0x2120, 1),
        (0x20a8, 5),
        (0x2220, 1),
        (0x20d0, 1),
        (0x20e0, 1),
        (0x1000 + cunit::ATTACK_ACCUMULATOR, 4),
    ]
    .into_iter()
    .enumerate()
    {
        for lane in 0..count {
            source.u64(
                address + lane * 8,
                (i64::MIN + index as i64 * 31 + lane as i64) as u64,
            );
        }
    }
    source.put(0x1000 + DIRECTION_GROUP, &(-3_i32).to_le_bytes());
    source.u64(
        0x2000 + csub_unit_definition::AIR_FLAGS,
        0x8000_0000_0000_0020,
    );
    let expected = Observed::new(&source);
    let actual = Observed::new(&source);
    let row = scalar_stats(&expected, 0, 0x1000, 0x1234_0000_0033).unwrap();
    assert_eq!(row.direction_group, -3);
    assert_eq!(row.categories, 0x8000_0000_0000_0020);
    let capture =
        |memory: &Observed<'_, _>, handle| NameArena::new().stats(memory, 0, 0x1000, handle);
    assert_eq!(capture(&actual, row.division), Ok(row));
    assert_eq!(*actual.reads.borrow(), *expected.reads.borrow());
    assert_eq!(
        *actual.scatters.borrow(),
        [
            vec![
                (0x2298, 16),
                (0x10c0, 8),
                (0x1000 + NAME_GROUP_MEMBER, 8),
                (0x1468, 8),
                (0x1620, 8),
                (0x2120, 8),
                (0x20a8, 40),
                (0x2220, 8),
                (0x20d0, 8),
                (0x20e0, 8),
                (0x1000 + cunit::ATTACK_ACCUMULATOR, 32),
                (0x1000 + DIRECTION_GROUP, 4),
                (0x2000 + csub_unit_definition::AIR_FLAGS, 8)
            ],
            vec![(0xe000, 16), (0xe000 + engine_native::corders_group::LEADER, 8)],
        ]
    );
    let trace = expected.reads.into_inner();
    for ordinal in 0..trace.len() {
        for change in [false, true] {
            let mut expected = Observed::new(&source);
            let mut actual = Observed::new(&source);
            if change {
                expected.change = Some((ordinal, 0));
                actual.change = expected.change;
            } else {
                expected.fault = Some(ordinal);
                actual.fault = expected.fault;
            }
            assert_eq!(
                capture(&actual, row.division),
                scalar_stats(&expected, 0, 0x1000, row.division),
                "ordinal {ordinal}, change {change}"
            );
            assert_eq!(
                *actual.reads.borrow(),
                *expected.reads.borrow(),
                "ordinal {ordinal}"
            );
            if !change {
                assert_eq!(*actual.reads.borrow(), trace[..=ordinal]);
            }
        }
    }
}

#[test]
fn stats_preserve_identity_module_overflow_order_and_later_address_failure() {
    let mut source = context_fixture();
    for fault in [None, Some(0)] {
        let mut actual = Observed::new(&source);
        actual.fault = fault;
        let result = NameArena::new().stats(&actual, usize::MAX, 0x1000, 0x1234_0000_0033);
        assert_eq!(
            result,
            Err(if fault.is_some() {
                "land combat division inputs unreadable"
            } else {
                "land combat division identity mismatch"
            })
        );
        assert_eq!(*actual.reads.borrow(), [(0x1000, 8)]);
        assert!(actual.scatters.borrow().is_empty());
    }
    let stat = usize::MAX - 0x2a0;
    source.u64(0x1138, stat as u64);
    source.u64(stat, csub_unit_definition::VTABLE_RVA.0 as u64);
    let actual = Observed::new(&source);
    let expected = Observed::new(&source);
    assert_eq!(
        NameArena::new().stats(&actual, 0, 0x1000, 0x1234_0000_0033),
        scalar_stats(&expected, 0, 0x1000, 0x1234_0000_0033)
    );
    assert!(!actual.reads.borrow().contains(&(0x10c0, 8)));
}

#[test]
fn input_scatter_keeps_partial_failure_empty_duplicates_and_address_error_prefix() {
    let source = SliceMemory {
        base: 0x1000,
        bytes: &[1, 2, 3, 4, 5, 6, 7, 8],
    };
    for fault in [None, Some(0), Some(1)] {
        let mut memory = Observed::new(&source);
        memory.fault = fault;
        let (mut first, mut duplicate, mut later) = ([0xa5; 8], [0xa5; 8], [0xa5; 8]);
        let result = copy_input_spans(
            &memory,
            [
                Ok(0x1000),
                Err("empty address"),
                Ok(0x1000),
                Err("late address"),
            ],
            &mut [&mut first, &mut [], &mut duplicate, &mut later],
        );
        assert_eq!(
            result,
            Err(if fault.is_some() {
                "land combat division inputs unreadable"
            } else {
                "late address"
            })
        );
        assert_eq!(later, [0xa5; 8]);
        if let Some(ordinal) = fault {
            let failed = if ordinal == 0 { first } else { duplicate };
            assert_eq!(failed, [0xed, 0xed, 0xed, 0xed, 0xa5, 0xa5, 0xa5, 0xa5]);
            if ordinal == 0 {
                assert_eq!(duplicate, [0xa5; 8]);
            }
        } else {
            assert_eq!(first, [1, 2, 3, 4, 5, 6, 7, 8]);
            assert_eq!(duplicate, first);
        }
        assert_eq!(memory.reads.borrow().len(), fault.map_or(2, |i| i + 1));
    }
}

#[test]
fn context_groups_keep_first_bytes_and_fault_prefixes() {
    use engine_native_parts::modifier_definition as md;
    let source = context_fixture();
    let baseline = Observed::new(&source);
    let mut terms = vec![WidthTerm::default(); 2];
    let row = capture_context_at(
        &baseline,
        0,
        0x5000,
        &mut crate::combat_sources::Sources::new(),
        0x1000,
        0x1234_0000_0033,
        &mut terms,
    )
    .unwrap();
    assert_eq!(row.template, 1_u64 << 32 | 52);
    let metadata: Vec<_> = CONTEXT_MODIFIER_KEYS
        .map(|key| (0x10000 + key as usize * md::STRIDE + md::FLAGS, 12))
        .into();
    assert!(baseline.scatters.borrow().contains(&metadata));
    let trace = baseline.reads.into_inner();
    assert_eq!(trace.len(), 33);
    assert_eq!(trace.iter().map(|(_, len)| len).sum::<usize>(), 364);
    for group in [&metadata] {
        let start = trace
            .windows(group.len())
            .position(|p| p == group.as_slice())
            .unwrap();
        for ordinal in start..start + group.len() {
            let mut memory = Observed::new(&source);
            memory.fault = Some(ordinal);
            let mut out = vec![WidthTerm::default(); 2];
            assert_eq!(
                capture_context_at(
                    &memory,
                    0,
                    0x5000,
                    &mut crate::combat_sources::Sources::new(),
                    0x1000,
                    0x1234_0000_0033,
                    &mut out
                ),
                Err("land combat division inputs unreadable")
            );
            assert_eq!(*memory.reads.borrow(), trace[..=ordinal]);
            assert_eq!(out, [WidthTerm::default(); 2]);
        }
    }
    for span in &metadata {
        assert_eq!(trace.iter().filter(|p| *p == span).count(), 1);
    }
    assert_eq!(trace.iter().filter(|p| **p == (0xc000, 32)).count(), 1);
}

#[test]
fn context_metadata_address_errors_follow_readable_prefix_and_earlier_fault() {
    use engine_native_parts::modifier_definition as md;
    let second_offset = CONTEXT_MODIFIER_KEYS[1] as usize * md::STRIDE;
    for (table, reason) in [
        (
            usize::MAX - second_offset + 1,
            "land combat context metadata address overflow",
        ),
        (
            usize::MAX - second_offset - md::FLAGS + 1,
            "land combat division inputs unreadable",
        ),
        (
            usize::MAX - second_offset - md::FLAGS - 5,
            "land combat division inputs unreadable",
        ),
    ] {
        let mut source = context_fixture();
        source.u64(md::TABLE_RVA.0, table as u64);
        let first = table + CONTEXT_MODIFIER_KEYS[0] as usize * md::STRIDE + md::FLAGS;
        source.put(first, &[0x17; 12]);
        let memory = Observed::new(&source);
        assert_eq!(
            capture_context_at(
                &memory,
                0,
                0x5000,
                &mut crate::combat_sources::Sources::new(),
                0x1000,
                0x1234_0000_0033,
                &mut Vec::new()
            ),
            Err(reason)
        );
        let trace = memory.reads.into_inner();
        assert_eq!(trace.last(), Some(&(first, 12)));
        let mut failed = Observed::new(&source);
        failed.fault = Some(trace.len() - 1);
        assert_eq!(
            capture_context_at(
                &failed,
                0,
                0x5000,
                &mut crate::combat_sources::Sources::new(),
                0x1000,
                0x1234_0000_0033,
                &mut Vec::new()
            ),
            Err("land combat division inputs unreadable")
        );
        assert_eq!(*failed.reads.borrow(), trace);
    }
}

struct Counted<'a> {
    inner: SliceMemory<'a>,
    copies: Cell<usize>,
    bytes: Cell<usize>,
}
impl Memory for Counted<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.copies.set(self.copies.get() + 1);
        self.bytes.set(self.bytes.get() + destination.len());
        self.inner.copy(address, destination)
    }
}

fn fixture(count: u32) -> Vec<u8> {
    let mut image = vec![0_u8; 0x5000];
    let mut put = |address: usize, value: u64| {
        image[address..address + 8].copy_from_slice(&value.to_le_bytes());
    };
    put(0x1000, carmy::VTABLE_RVA.0 as u64);
    put(0x1018, 0x1234_0000_0033);
    put(0x1138, 0x2000);
    put(0x1340, 0x3800);
    put(0x3800, engine_native::cname_group_member::VTABLE_RVA.0 as u64);
    put(0x3880, i32::MAX as u64);
    put(0x3888, 0xff);
    put(0x3898, 1);
    put(0x38a0, 15);
    put(0x38a8, 1);
    put(0x1170, 0x3000);
    put(0x1178, (count as u64) << 32 | count as u64);
    put(0x2000, csub_unit_definition::VTABLE_RVA.0 as u64);
    put(0x1420, (-123_i64) as u64);
    put(0x2280, 1);
    put(0x3000, 27);
    put(0x3008, (-900_i64) as u64);
    put(0x3010, 800);
    put(0x3018, 2);
    image
}

#[test]
fn preserves_signed_values_order_and_raw_maximum_with_bounded_copy_cost() {
    let image = fixture(2);
    let memory = Counted {
        inner: SliceMemory {
            base: 0,
            bytes: &image,
        },
        copies: Cell::new(0),
        bytes: Cell::new(0),
    };
    let mut modifiers = vec![Modifier::default(); 2];
    let mut arena = NameArena::new();
    let row = arena.stats(&memory, 0, 0x1000, 0x1234_0000_0033).unwrap();
    assert_eq!(
        capture_modifiers_buffer(&memory, 0, 0x1000, 0x1234_0000_0033, engine_native_memory::walk::Buffer::Growing(&mut modifiers)).unwrap(),
        2
    );
    let naming = row.naming.unwrap();
    assert_eq!(arena.text(naming.custom), &[255]);
    assert_eq!(naming.order, i32::MAX);
    assert_eq!(
        modifiers[0],
        Modifier {
            id: 27,
            value_a: -900,
            value_b: 800
        }
    );
    assert_eq!(modifiers[1].id, 2);
    assert_eq!(memory.copies.get(), 22);
    assert_eq!(memory.bytes.get(), 446);
}

#[test]
fn refuses_modifier_pressure_and_wrong_identity() {
    let image = fixture(2);
    let memory = SliceMemory {
        base: 0,
        bytes: &image,
    };
    assert_eq!(
        capture_modifiers_buffer(&memory, 0, 0x1000, 1, engine_native_memory::walk::Buffer::Count),
        Err("land combat division identity mismatch")
    );
    assert_eq!(
        capture_modifiers_buffer(&memory, 0, 0x1000, 0x1234_0000_0033, engine_native_memory::walk::Buffer::Count),
        Err("land combat division modifier arena full")
    );
    let image = fixture(257);
    let memory = SliceMemory {
        base: 0,
        bytes: &image,
    };
    assert_eq!(
        capture_modifiers_buffer(&memory, 0, 0x1000, 0x1234_0000_0033, engine_native_memory::walk::Buffer::Count),
        Err("land combat division modifier arena full")
    );
}

#[test]
fn worker_modifier_vector_has_no_chosen_limit_and_grows_from_original_headers() {
    let mut modifiers = Vec::new();
    for count in [50, 75] {
        let mut image = fixture(0);
        image.resize(0x10_0000 + count * 24, 0);
        image[0x1170..0x1178].copy_from_slice(&0x10_0000_u64.to_le_bytes());
        image[0x1178..0x117c].copy_from_slice(&(count as u32).to_le_bytes());
        image[0x117c..0x1180].copy_from_slice(&(count as u32).to_le_bytes());
        for index in 0..count {
            let at = 0x10_0000 + index * 24;
            image[at..at + 4].copy_from_slice(&(index as u32 + 1).to_le_bytes());
            image[at + 8..at + 16].copy_from_slice(&(-(index as i64)).to_le_bytes());
        }
        let memory = SliceMemory {
            base: 0,
            bytes: &image,
        };
        let source = Observed::new(&memory);
        assert_eq!(
            capture_modifiers_buffer(
                &source,
                0,
                0x1000,
                0x1234_0000_0033,
                engine_native_memory::walk::Buffer::Growing(&mut modifiers)
            ),
            Ok(count)
        );
        assert_eq!(modifiers[count - 1].id, count as u32);
        assert_eq!(modifiers[count - 1].value_a, -(count as i64 - 1));
        assert_eq!(
            source
                .reads
                .borrow()
                .iter()
                .filter(|&&(at, len)| (at, len) == (0x1170, 16))
                .count(),
            1
        );
    }
}

#[test]
fn worker_width_and_context_accumulator_accept_mod_sized_native_vectors() {
    let mut terms = Vec::new();
    for count in [50, 75] {
        let mut memory = context_fixture();
        memory.image.resize(0x40_0000, 0);
        memory.array(0xb0c0, 0x20_0000, count as u32);
        memory.array(
            0x9000 + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR,
            0x10_0000,
            count as u32,
        );
        for index in 0..count {
            memory.put(0x10_0000 + index * 16, &(index as u32 + 1).to_le_bytes());
            memory.u64(0x10_0000 + index * 16 + 8, index as u64);
            memory.u64(0x20_0000 + index * 16, 0xd000);
            memory.put(0x20_0000 + index * 16 + 8, &(-(index as i32)).to_le_bytes());
        }
        let countries = engine_native::ccountry::Reader::new(&memory, 0, 0x5000).unwrap();
        let row = capture_context_buffer(
            &memory,
            0,
            &countries,
            &mut crate::combat_sources::Sources::new(),
            0x1000,
            0x1234_0000_0033,
            engine_native_memory::walk::Buffer::Growing(&mut terms),
        )
        .unwrap();
        assert_eq!(row.width_term_count, count);
        assert_eq!(terms[count - 1].count, -(count as i32 - 1));
        assert_eq!(terms[count - 1].width, 12345);
    }
}

#[test]
fn bad_stat_identity_does_not_hide_independent_modifiers() {
    let mut image = fixture(2);
    image[0x2000..0x2008].fill(0);
    let memory = SliceMemory {
        base: 0,
        bytes: &image,
    };
    assert_eq!(
        NameArena::new().stats(&memory, 0, 0x1000, 0x1234_0000_0033),
        Err("land combat division stat identity mismatch")
    );
    let mut modifiers = vec![Modifier::default(); 2];
    assert_eq!(
        capture_modifiers_buffer(&memory, 0, 0x1000, 0x1234_0000_0033, engine_native_memory::walk::Buffer::Growing(&mut modifiers)),
        Ok(2)
    );
}

struct ContextMemory {
    image: Vec<u8>,
    globals: std::collections::BTreeMap<usize, u8>,
    copies: Cell<usize>,
    copied: Cell<usize>,
}
impl Memory for ContextMemory {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.copies.set(self.copies.get() + 1);
        self.copied.set(self.copied.get() + destination.len());
        for (index, byte) in destination.iter_mut().enumerate() {
            let Some(value) = self
                .image
                .get(address + index)
                .or_else(|| self.globals.get(&(address + index)))
            else {
                return false;
            };
            *byte = *value;
        }
        true
    }
}
impl ContextMemory {
    fn put(&mut self, address: usize, bytes: &[u8]) {
        if address < self.image.len() {
            self.image[address..address + bytes.len()].copy_from_slice(bytes);
        } else {
            for (index, value) in bytes.iter().enumerate() {
                self.globals.insert(address + index, *value);
            }
        }
    }
    fn u64(&mut self, address: usize, value: u64) {
        self.put(address, &value.to_le_bytes());
    }
    fn array(&mut self, address: usize, data: usize, count: u32) {
        self.u64(address, data as u64);
        self.u64(address + 8, (count as u64) << 32 | count as u64);
    }
}
fn context_fixture() -> ContextMemory {
    let mut memory = ContextMemory {
        image: fixture(2),
        globals: Default::default(),
        copies: Cell::new(0),
        copied: Cell::new(0),
    };
    memory.image.resize(0x40000, 0);
    memory.u64(0x11d8, 1);
    memory.array(0x5000 + engine_native::cgame_state::COUNTRIES, 0x6000, 2);
    memory.array(0x5000 + engine_native::cgame_state::COUNTRY_LINKS, 0x6100, 2);
    memory.u64(0x6008, 0x9000);
    memory.put(0x6104, &1_u32.to_le_bytes());
    memory.u64(0x9000, engine_native::ccountry::VTABLE_RVA.0 as u64);
    memory.put(0x9008, &1_u32.to_le_bytes());
    memory.array(0x9000 + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR, 0xa000, 3);
    for (index, key) in CONTEXT_MODIFIER_KEYS.into_iter().enumerate() {
        memory.put(0xa000 + index * 16, &key.to_le_bytes());
        memory.u64(0xa008 + index * 16, (-111_i64 - index as i64) as u64);
    }
    memory.u64(engine_native_parts::modifier_definition::TABLE_RVA.0, 0x10000);
    memory.u64(engine_native_parts::modifier_definition::ELIGIBILITY_CALLBACK_RVA.0, 0);
    memory.put(
        engine_native_parts::modifier_definition::TRADE_FEATURE_FLAGS_RVA.0,
        &0_u32.to_le_bytes(),
    );
    memory.u64(0x13b8, 0xb000);
    memory.u64(
        0xb000,
        engine_native::creferenced_division_template::VTABLE_RVA.0 as u64,
    );
    memory.u64(0xb008, 1_u64 << 32 | 52);
    memory.array(0xb0c0, 0xc000, 2);
    memory.u64(0xc000, 0xd000);
    memory.u64(0xc010, 0xd000);
    memory.put(0xc008, &3_i32.to_le_bytes());
    memory.put(0xc018, &(-2_i32).to_le_bytes());
    memory.u64(0xd000, csub_unit_definition::VTABLE_RVA.0 as u64);
    memory.put(0xd008, &7_u32.to_le_bytes());
    memory.u64(0xd058, 12345);
    memory
}

#[test]
fn width_context_preserves_order_signed_counts_and_raw_modifiers() {
    let memory = context_fixture();
    let mut terms = vec![WidthTerm::default(); 2];
    let row = capture_context_at(
        &memory,
        0,
        0x5000,
        &mut crate::combat_sources::Sources::new(),
        0x1000,
        0x1234_0000_0033,
        &mut terms,
    )
    .unwrap();
    assert_eq!(row.width_term_count, 2);
    assert_eq!(
        terms,
        [
            WidthTerm {
                subunit: 7,
                count: 3,
                width: 12345
            },
            WidthTerm {
                subunit: 7,
                count: -2,
                width: 12345
            }
        ]
    );
    assert_eq!(row.country_modifiers.map(|v| v.raw), [-111, -112, -113]);
    assert_eq!(row.leader_modifiers, [ContextModifier::default(); 3]);
    assert_eq!(row.template, 1_u64 << 32 | 52);
    assert_eq!(memory.copies.get(), 33);
    assert_eq!(memory.copied.get(), 364);
}

#[test]
fn context_modifier_reuse_is_shared_only_within_the_current_pass() {
    let mut memory = context_fixture();
    let mut sources = crate::combat_sources::Sources::new();
    let mut terms = vec![WidthTerm::default(); 2];
    let first = capture_context_at(
        &memory,
        0,
        0x5000,
        &mut sources,
        0x1000,
        0x1234_0000_0033,
        &mut terms,
    )
    .unwrap();
    let first_terms = terms.clone();
    let before = (memory.copies.get(), memory.copied.get());
    let second = capture_context_at(
        &memory,
        0,
        0x5000,
        &mut sources,
        0x1000,
        0x1234_0000_0033,
        &mut terms,
    )
    .unwrap();
    assert_eq!(second, first);
    assert_eq!(terms, first_terms);
    assert_eq!(memory.copies.get() - before.0, 17);

    memory.u64(0xa008, 170);
    sources.clear();
    let current = capture_context_at(
        &memory,
        0,
        0x5000,
        &mut sources,
        0x1000,
        0x1234_0000_0033,
        &mut terms,
    )
    .unwrap();
    assert_eq!(
        current.country_modifiers[0],
        ContextModifier {
            raw: 170,
            present: 1
        }
    );

    memory.array(0xe000, 0xe100, 1);
    memory.put(0xe100, &0x10f_u32.to_le_bytes());
    memory.u64(0xe108, 999);
    let before = memory.copies.get();
    let other = context_modifiers(&memory, 0xe000).unwrap();
    assert_eq!(other[0], ContextModifier::default());
    assert_eq!(
        other[1],
        ContextModifier {
            raw: 999,
            present: 1
        }
    );
    assert_eq!(memory.copies.get() - before, 2);
}

#[test]
fn context_modifier_census_fails_at_its_read_and_on_a_malformed_suffix() {
    let mut source = context_fixture();
    let address = 0x9000 + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR;
    for ordinal in [0, 1] {
        let mut observed = Observed::new(&source);
        observed.fault = Some(ordinal);
        assert!(context_modifiers(&observed, address).is_err());
        assert_eq!(observed.reads.borrow().len(), ordinal + 1);
    }
    source.array(address, 0x20000, 65);
    for (index, key) in CONTEXT_MODIFIER_KEYS
        .into_iter()
        .chain(1003..1065)
        .enumerate()
    {
        source.put(0x20000 + index * 16, &key.to_le_bytes());
        source.u64(0x20008 + index * 16, index as u64 + 1);
    }
    source.put(0x20000 + 64 * 16, &1_u32.to_le_bytes());
    let before = source.copies.get();
    assert_eq!(
        context_modifiers(&source, address),
        Err("land combat context modifier keys not increasing")
    );
    assert_eq!(source.copies.get() - before, 3);
    source.put(0x20000 + 64 * 16, &1064_u32.to_le_bytes());
    let values = context_modifiers(&source, address).unwrap();
    assert_eq!(values.map(|value| value.raw), [1, 2, 3]);
}

#[test]
fn exile_lists_are_bounded_by_the_live_country_count_and_never_sized_from_it() {
    let mut memory = context_fixture();
    memory.put(0xb000 + TEMPLATE_COUNTRY, &1_i32.to_le_bytes());
    memory.put(0xb000 + TEMPLATE_ORIGIN_TYPE, &2_i32.to_le_bytes());
    memory.u64(0x1000 + cunit::LOCATION, 0x7000);
    memory.u64(0x7000 + engine_native::cprovince::STATE, 0x7800);
    memory.array(0x7800 + engine_native::cstate::CORES, 0x7c00, 2);
    memory.put(0x7c00, &[1, 0, 0, 0, 2, 0, 0, 0]);
    memory.u64(0x9000 + engine_native::ccountry::DIPLOMACY_STATUS, 0xe800);
    memory.array(0xe800 + OCCUPIER_SET, 0xec00, 1);
    memory.put(0xec00, &2_u32.to_le_bytes());
    let countries = engine_native::ccountry::Reader::new(&memory, 0, 0x5000).unwrap();
    assert_eq!(countries.count(), 2);
    let mut tags = vec![0; 3];
    assert_eq!(
        capture_exile(&memory, &countries, 0x1000, &mut tags),
        Ok(Exile::Row(ExileRow {
            origin_type: 2,
            country: 1,
            country_fallback: 0,
            cores: (0, 2),
            occupiers: (2, 1),
        }))
    );
    assert_eq!(tags, [1, 2, 2]);
    for count in [3, i32::MAX as u32] {
        memory.array(0x7800 + engine_native::cstate::CORES, 0x7c00, count);
        let countries = engine_native::ccountry::Reader::new(&memory, 0, 0x5000).unwrap();
        let before = memory.copies.get();
        assert_eq!(
            capture_exile(&memory, &countries, 0x1000, &mut vec![0; 8]),
            Err("exile country list exceeds the live country count")
        );
        assert_eq!(memory.copies.get() - before, 5);
    }
    for origin in [0, 1, 3, 7, -1, i32::MIN, i32::MAX] {
        memory.put(0xb000 + TEMPLATE_ORIGIN_TYPE, &origin.to_le_bytes());
        let countries = engine_native::ccountry::Reader::new(&memory, 0, 0x5000).unwrap();
        let before = memory.copies.get();
        assert_eq!(
            capture_exile(&memory, &countries, 0x1000, &mut Vec::new()),
            Ok(Exile::Absent),
            "origin {origin}"
        );
        assert_eq!(memory.copies.get() - before, 2, "origin {origin}");
    }
}

#[test]
fn worker_exile_arena_keeps_country_lists_and_later_growth_without_truncation() {
    let mut tags = vec![0xfeed];
    for count in [3, 7] {
        let mut memory = context_fixture();
        memory.image.resize(0x40_0000, 0);
        memory.array(
            0x5000 + engine_native::cgame_state::COUNTRIES,
            0x10_0000,
            count as u32 + 1,
        );
        memory.array(
            0x5000 + engine_native::cgame_state::COUNTRY_LINKS,
            0x20_0000,
            count as u32 + 1,
        );
        memory.u64(0x10_0008, 0x9000);
        memory.put(0x20_0004, &1_u32.to_le_bytes());
        memory.put(0xb000 + TEMPLATE_COUNTRY, &1_i32.to_le_bytes());
        memory.put(0xb000 + TEMPLATE_ORIGIN_TYPE, &2_i32.to_le_bytes());
        memory.u64(0x1000 + cunit::LOCATION, 0x7000);
        memory.u64(0x7000 + engine_native::cprovince::STATE, 0x7800);
        memory.array(0x7800 + engine_native::cstate::CORES, 0x30_0000, count as u32);
        memory.u64(0x9000 + engine_native::ccountry::DIPLOMACY_STATUS, 0xe800);
        memory.array(0xe800 + OCCUPIER_SET, 0x31_0000, count as u32);
        for index in 0..count {
            memory.put(0x30_0000 + index * 4, &(index as u32 + 1).to_le_bytes());
            memory.put(
                0x31_0000 + index * 4,
                &(count as u32 - index as u32).to_le_bytes(),
            );
        }
        let countries = engine_native::ccountry::Reader::new(&memory, 0, 0x5000).unwrap();
        let row = capture_exile_buffer(
            &memory,
            &countries,
            0x1000,
            engine_native_memory::walk::Buffer::GrowingFrom(&mut tags, 1),
        )
        .unwrap();
        let Exile::Row(row) = row else {
            panic!("complete original exile lists were not retained")
        };
        assert_eq!(row.cores, (0, count));
        assert_eq!(row.occupiers, (count, count));
        assert_eq!(tags[0], 0xfeed);
        assert_eq!(tags[count], count as u32);
        assert_eq!(tags[count * 2], 1);
    }
}

#[test]
fn exile_country_faults_fail_the_division_with_their_reason() {
    let mut memory = context_fixture();
    memory.put(0x1000 + cunit::ROSTER_OWNER, &1_u32.to_le_bytes());
    memory.put(0xb000 + TEMPLATE_ORIGIN_TYPE, &2_i32.to_le_bytes());
    memory.put(0xb000 + TEMPLATE_COUNTRY, &1_i32.to_le_bytes());
    memory.u64(0x9000 + engine_native::ccountry::DIPLOMACY_STATUS, 0xe800);
    let countries = engine_native::ccountry::Reader::new(&memory, 0, 0x5000).unwrap();
    assert!(matches!(
        capture_exile(&memory, &countries, 0x1000, &mut vec![0; 4]),
        Ok(Exile::Row(_))
    ));
    memory.put(0xb000 + TEMPLATE_COUNTRY, &5_i32.to_le_bytes());
    let countries = engine_native::ccountry::Reader::new(&memory, 0, 0x5000).unwrap();
    let unlinked = capture_exile(&memory, &countries, 0x1000, &mut vec![0; 4]);
    assert_eq!(unlinked, countries.resolve(5).map(|_| Exile::Absent));
    assert!(unlinked.is_err());
    memory.put(0xb000 + TEMPLATE_COUNTRY, &1_i32.to_le_bytes());
    memory.u64(0x9000 + engine_native::ccountry::DIPLOMACY_STATUS, 0);
    let countries = engine_native::ccountry::Reader::new(&memory, 0, 0x5000).unwrap();
    assert_eq!(
        capture_exile(&memory, &countries, 0x1000, &mut vec![0; 4]),
        Err("exile country diplomacy status missing")
    );
    let status = 0x9000 + engine_native::ccountry::DIPLOMACY_STATUS;
    memory.u64(status, 0xe800);
    let mut observed = Observed::new(&memory);
    let countries = engine_native::ccountry::Reader::new(&memory, 0, 0x5000).unwrap();
    let _ = capture_exile(&observed, &countries, 0x1000, &mut vec![0; 4]);
    let ordinal = observed
        .reads
        .borrow()
        .iter()
        .position(|&(at, _)| at == status)
        .unwrap();
    observed = Observed::new(&memory);
    observed.fault = Some(ordinal);
    assert_eq!(
        capture_exile(&observed, &countries, 0x1000, &mut vec![0; 4]),
        Err("land combat division inputs unreadable")
    );
}

#[test]
fn width_context_reports_a_malformed_accumulator() {
    let mut memory = context_fixture();
    memory.array(0x9000 + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR, 0xa000, 4097);
    assert_eq!(
        capture_context_at(
            &memory,
            0,
            0x5000,
            &mut crate::combat_sources::Sources::new(),
            0x1000,
            0x1234_0000_0033,
            &mut Vec::new()
        ),
        Err("land combat context modifier keys not increasing")
    );
}

#[test]
fn width_context_selects_authenticated_leader_raw_modifiers() {
    let mut memory = context_fixture();
    memory.u64(0x10c0, 0xe000);
    memory.u64(0xe000, engine_native::corders_group::VTABLE_RVA.0 as u64);
    memory.u64(0xe008, 1_u64 << 32 | 53);
    memory.u64(0xe088, 0xf000);
    memory.u64(0xf000, engine_native::carmy_leader::VTABLE_RVA.0 as u64);
    memory.u64(0xf008, 77_u64 << 32 | 4713);
    memory.array(0xf418, 0xa100, 1);
    memory.put(0xa100, &0x10f_u32.to_le_bytes());
    memory.u64(0xa108, (-777_i64) as u64);
    let row = capture_context_at(
        &memory,
        0,
        0x5000,
        &mut crate::combat_sources::Sources::new(),
        0x1000,
        0x1234_0000_0033,
        &mut vec![WidthTerm::default(); 2],
    )
    .unwrap();
    assert_eq!(row.leader_modifier_kind, 1);
    assert_eq!(
        row.leader_modifiers[1],
        ContextModifier {
            raw: -777,
            present: 1
        }
    );
    assert_eq!(commander(&memory, 0, 0xe000), Ok(77_u64 << 32 | 4713));
    memory.u64(0xe008, 1_u64 << 32 | 99);
    assert_eq!(
        commander(&memory, 0, 0xe000),
        Err("land combat division group identity mismatch")
    );
}

#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
mod scale {
    use super::*;
    use engine_native::cname_group_member as name;
    use engine_native_parts::creference_registry as registry;

    const ARMY: usize = 0x700;
    const STAT: usize = 0x2c0;
    const MEMBER: usize = 0xb0;
    const HEAP_NAME: usize = 0x40;
    const GROUPS: usize = 100;
    const NAME_GROUPS: usize = 8;

    pub(super) struct World {
        module: Vec<u8>,
        heap: Vec<u8>,
        pub(super) armies: Vec<(usize, u64)>,
    }

    impl World {
        pub(super) fn base(&self) -> usize {
            self.module.as_ptr() as usize
        }
        fn at(&mut self, address: usize) -> &mut [u8] {
            let module = self.module.as_ptr() as usize;
            let heap = self.heap.as_ptr() as usize;
            if (heap..heap + self.heap.len()).contains(&address) {
                &mut self.heap[address - heap..]
            } else {
                &mut self.module[address - module..]
            }
        }
        fn put(&mut self, address: usize, bytes: &[u8]) {
            self.at(address)[..bytes.len()].copy_from_slice(bytes);
        }
        fn u64(&mut self, address: usize, value: u64) {
            self.put(address, &value.to_le_bytes());
        }
        fn string(&mut self, address: usize, text: &[u8], heap: usize) {
            if text.len() <= 15 {
                self.put(address, text);
                self.put(address + text.len(), &[0]);
                self.u64(address + 24, 15);
            } else {
                self.put(heap, text);
                self.put(heap + text.len(), &[0]);
                self.u64(address, heap as u64);
                self.u64(address + 24, 63);
            }
            self.u64(address + 16, text.len() as u64);
        }

        pub(super) fn new(count: usize, template_count: usize) -> Self {
            let capacity = (template_count * 2).next_power_of_two().max(1024);
            let overflow = capacity.trailing_zeros() as usize + 2;
            let module = vec![0_u8; registry::LOW_ROOTS_RVA.0 + 0x1000];
            let sizes = [
                count * ARMY,
                count * STAT,
                count * MEMBER,
                count * HEAP_NAME,
                GROUPS * 0x100,
                GROUPS * 0x20,
                template_count * 0x200,
                NAME_GROUPS * 0x40,
                0x40,
                (capacity + overflow + 1) * 24,
            ];
            let heap = vec![0_u8; sizes.iter().sum::<usize>() + 64];
            let mut world = Self {
                module,
                heap,
                armies: Vec::with_capacity(count),
            };
            let base = world.base();
            let mut next = (world.heap.as_ptr() as usize).next_multiple_of(16);
            let mut region = |size: usize| {
                let start = next;
                next += size;
                start
            };
            let [
                armies,
                stats,
                members,
                names,
                groups,
                leaders,
                templates,
                name_groups,
                table,
                entries,
            ] = sizes.map(&mut region);
            for g in 0..GROUPS {
                let (group, leader) = (groups + g * 0x100, leaders + g * 0x20);
                world.u64(group, (base + engine_native::corders_group::VTABLE_RVA.0) as u64);
                world.u64(group + 8, (g as u64 + 1) << 32 | 53);
                world.u64(group + engine_native::corders_group::LEADER, leader as u64);
                world.u64(leader, (base + engine_native::carmy_leader::VTABLE_RVA.0) as u64);
                world.u64(leader + 8, (g as u64 + 1) << 32 | 4713);
            }
            for n in 0..NAME_GROUPS {
                let group = name_groups + n * 0x40;
                world.u64(group, (base + name::GROUP_VTABLE_RVA.0) as u64);
                world.string(group + name::GROUP_KEY, format!("GRP_{n}").as_bytes(), 0);
            }
            world.u64(base + registry::LOW_ROOTS_RVA.0 + 52 * 8, table as u64);
            world.u64(table + registry::ENTRIES, entries as u64);
            world.put(
                table + registry::COUNT,
                &(template_count as u32).to_le_bytes(),
            );
            world.put(table + registry::MASK, &(capacity as u32 - 1).to_le_bytes());
            world.put(table + registry::OVERFLOW_DISTANCE, &[overflow as u8]);
            world.put(entries + (capacity + overflow) * 24 + 4, &[255]);
            for t in 0..template_count {
                let template = templates + t * 0x200;
                let handle = (t as u64 + 1) << 32 | 52;
                world.u64(
                    template,
                    (base + engine_native::creferenced_division_template::VTABLE_RVA.0) as u64,
                );
                world.u64(template + 8, handle);
                world.u64(
                    template + name::TEMPLATE_NAME_GROUP,
                    (name_groups + (t % NAME_GROUPS) * 0x40) as u64,
                );
                let hash = handle
                    .rotate_left(32)
                    .wrapping_mul(registry::HASH_MULTIPLIER);
                let mut index = (((hash >> 32) as u32 ^ hash as u32) as usize) & (capacity - 1);
                let (mut key, mut object, mut distance) = (handle, template as u64, 1_u8);
                loop {
                    let cell = entries + index * 24;
                    let stored = world.at(cell)[4];
                    if stored == 0 {
                        world.put(cell + 4, &[distance]);
                        world.u64(cell + 8, key);
                        world.u64(cell + 16, object);
                        break;
                    }
                    if stored < distance {
                        let word = |world: &mut Self, at: usize| {
                            u64::from_le_bytes(world.at(at)[..8].try_into().unwrap())
                        };
                        let (old_key, old_object) =
                            (word(&mut world, cell + 8), word(&mut world, cell + 16));
                        world.put(cell + 4, &[distance]);
                        world.u64(cell + 8, key);
                        world.u64(cell + 16, object);
                        (key, object, distance) = (old_key, old_object, stored);
                    }
                    index += 1;
                    distance += 1;
                    assert!(index < capacity + overflow && usize::from(distance) <= overflow + 1);
                }
            }
            for i in 0..count {
                let (army, stat, member) =
                    (armies + i * ARMY, stats + i * STAT, members + i * MEMBER);
                let handle = (i as u64 + 1) << 32 | 0x33;
                world.u64(army, (base + carmy::VTABLE_RVA.0) as u64);
                world.u64(army + cunit::HANDLE, handle);
                world.u64(army + cunit::STAT_BLOCK, stat as u64);
                world.u64(
                    army + carmy::ORDERS_GROUP,
                    (groups + (i % GROUPS) * 0x100) as u64,
                );
                world.u64(army + NAME_GROUP_MEMBER, member as u64);
                world.u64(army + carmy::STRENGTH_FACTOR, 100_000);
                world.u64(stat, (base + csub_unit_definition::VTABLE_RVA.0) as u64);
                world.u64(stat + ARMOUR, 5 << 20);
                world.u64(member, (base + name::VTABLE_RVA.0) as u64);
                if i % 26 != 13 {
                    world.u64(
                        member + name::TEMPLATE_REFERENCE,
                        (((i % template_count) as u64) + 1) << 32 | 52,
                    );
                }
                world.put(member + name::ORDER, &(i as i32 + 1).to_le_bytes());
                world.put(member + name::FLAG_A, &[1, u8::from(i % 13 == 0)]);
                world.u64(member + name::OVERRIDE_NAME + 24, 15);
                if i % 20 == 0 || i % 13 == 0 {
                    world.string(member + name::OVERRIDE_NAME, b"Guards", 0);
                } else if i % 97 == 0 {
                    world.string(
                        member + name::OVERRIDE_NAME,
                        b"Guards Rifle Division of the Line",
                        names + i * HEAP_NAME,
                    );
                }
                world.armies.push((army, handle));
            }
            world
        }
    }

    struct Counted<M> {
        inner: M,
        calls: Cell<usize>,
        bytes: Cell<usize>,
    }
    impl<M: Memory> Memory for Counted<M> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.calls.set(self.calls.get() + 1);
            self.bytes.set(self.bytes.get() + destination.len());
            self.inner.copy(address, destination)
        }
        fn copy_scattered<const N: usize>(
            &self,
            addresses: [usize; N],
            destinations: &mut [&mut [u8]; N],
        ) -> usize {
            self.calls.set(self.calls.get() + 1);
            self.bytes
                .set(self.bytes.get() + destinations.iter().map(|d| d.len()).sum::<usize>());
            self.inner.copy_scattered(addresses, destinations)
        }
        fn copy_scattered_prefetched<const N: usize>(
            &self,
            addresses: [usize; N],
            destinations: &mut [&mut [u8]; N],
        ) -> usize {
            self.copy_scattered(addresses, destinations)
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    struct Plain {
        kind: i32,
        order: i32,
        ordered: u8,
        programmatic: u8,
        custom: engine_native_memory::msvc_string::Text<256>,
        group: engine_native_memory::msvc_string::Text<256>,
    }

    fn reference_naming(
        memory: &impl Memory,
        base: usize,
        member: usize,
        keys: &mut std::collections::BTreeMap<usize, engine_native_memory::msvc_string::Text<256>>,
    ) -> Result<Plain, &'static str> {
        let order = name::order(memory, base, member)?;
        let kind = read::<i32>(memory, member + name::TYPE).ok_or("type")?;
        let [ordered, programmatic] =
            read::<[u8; 2]>(memory, member + name::FLAG_A).ok_or("flags")?;
        let mut row = Plain {
            kind,
            order,
            ordered,
            programmatic,
            custom: Default::default(),
            group: Default::default(),
        };
        if programmatic == 0 {
            let (source, len, _, _) = name::custom(memory, base, member)?;
            row.custom.len = len;
            name::copy_custom(memory, source, len, &mut row.custom.bytes, 0)?;
        }
        if kind == 0 && row.custom.len == 0 {
            let template =
                read::<u64>(memory, member + name::TEMPLATE_REFERENCE).ok_or("template")?;
            let mut null = true;
            if template != 0
                && let Some(object) = registry::resolve(memory, base, template)?
            {
                assert_eq!(
                    engine_native::creferenced_division_template::identity(memory, base, object)?,
                    template
                );
                let group = read::<u64>(memory, object + name::TEMPLATE_NAME_GROUP)
                    .ok_or("group")? as usize;
                if group != 0 {
                    null = false;
                    assert_eq!(
                        read::<u64>(memory, group),
                        Some((base + name::GROUP_VTABLE_RVA.0) as u64)
                    );
                    row.group = match keys.get(&group) {
                        Some(key) => *key,
                        None => {
                            let key =
                                engine_native_memory::msvc_string::read_text(memory, group + name::GROUP_KEY)
                                    .ok_or("key")?;
                            *keys.entry(group).or_insert(key)
                        }
                    };
                }
            }
            if null && programmatic != 0 {
                let (source, len, _, _) = name::custom(memory, base, member)?;
                row.custom.len = len;
                name::copy_custom(memory, source, len, &mut row.custom.bytes, 0)?;
            }
        }
        Ok(row)
    }

    struct Pass {
        groups: name::GroupKeys,
        bytes: Vec<u8>,
        sources: crate::combat_sources::Sources,
    }
    impl Pass {
        fn new(world: &World) -> Self {
            Self::with(world, world.armies.len(), 1024)
        }
        fn with(world: &World, templates: usize, groups: usize) -> Self {
            Self {
                groups: name::GroupKeys::with_capacity(templates, groups),
                bytes: vec![0; world.armies.len() * 64 + 4096],
                sources: crate::combat_sources::Sources::new(),
            }
        }
        fn plain(&self, row: name::Inputs) -> Plain {
            let text = |(start, len): (usize, usize)| {
                let mut text = engine_native_memory::msvc_string::Text::<256>::default();
                text.bytes[..len].copy_from_slice(&self.bytes[start..start + len]);
                text.len = len;
                text
            };
            Plain {
                kind: row.kind,
                order: row.order,
                ordered: row.ordered,
                programmatic: row.programmatic,
                custom: text(row.custom),
                group: text(row.group),
            }
        }
    }

    fn pass(memory: &impl Memory, world: &World, context: &mut Pass) -> usize {
        context.groups.reset();
        let mut names = name::Names {
            bytes: engine_native_memory::walk::Buffer::Growing(&mut context.bytes),
            len: 0,
            required: 0,
        };
        let mut named = 0;
        for &(army, handle) in &world.armies {
            let row = capture_stats(
                memory,
                world.base(),
                &mut context.sources,
                army,
                handle,
                &mut context.groups,
                &mut names,
            )
            .unwrap();
            let naming = row.naming.unwrap();
            names.len += naming.custom.1;
            named += usize::from(naming.group.1 != 0 || naming.custom.1 != 0);
        }
        named
    }

    fn read_all(live: &impl Memory, world: &World, context: &mut Pass) -> Vec<Plain> {
        let mut rows = Vec::new();
        for _ in 0..2 {
            context.groups.reset();
            rows.clear();
            let mut names = name::Names {
                bytes: engine_native_memory::walk::Buffer::Growing(&mut context.bytes),
                len: 0,
                required: 0,
            };
            let mut spans = Vec::new();
            for &(army, handle) in &world.armies {
                let row = capture_stats(
                    live,
                    world.base(),
                    &mut context.sources,
                    army,
                    handle,
                    &mut context.groups,
                    &mut names,
                )
                .unwrap()
                .naming
                .unwrap();
                names.len += row.custom.1;
                spans.push(row);
            }
            rows.extend(spans.into_iter().map(|row| context.plain(row)));
        }
        rows
    }

    #[test]
    fn batched_naming_matches_the_per_field_reader_over_guarded_memory() {
        for templates in [64, 500] {
            let world = World::new(2_000, templates);
            // SAFETY: the world owns the complete synthetic graph, alive and immutable through every read.
            let live = unsafe { engine_native_memory::memory::LiveMemory::new() };
            let mut keys = Default::default();
            let expected: Vec<_> = world
                .armies
                .iter()
                .map(|&(army, _)| {
                    let member = read::<u64>(&live, army + NAME_GROUP_MEMBER).unwrap() as usize;
                    reference_naming(&live, world.base(), member, &mut keys).unwrap()
                })
                .collect();
            for (templates, groups) in [(world.armies.len(), 1024), (1 << 16, 1 << 16), (1, 1)] {
                let mut context = Pass::with(&world, templates, groups);
                assert_eq!(
                    read_all(&live, &world, &mut context),
                    expected,
                    "{templates}/{groups}"
                );
            }
            let mut kinds = [0; 4];
            for row in &expected {
                kinds[match (row.programmatic, row.custom.len) {
                    (0, 0) => 0,
                    (0, _) => 1,
                    (_, 0) => 2,
                    _ => 3,
                }] += 1;
            }
            assert!(kinds.iter().all(|count| *count > 0), "{kinds:?}");
        }
    }

    #[test]
    fn a_key_that_does_not_fit_is_counted_once_per_pass() {
        let world = World::new(4, 1);
        // SAFETY: the world owns the complete synthetic graph, alive and immutable through every read.
        let live = unsafe { engine_native_memory::memory::LiveMemory::new() };
        let mut groups = name::GroupKeys::with_capacity(4, 4);
        for pass in 0..2 {
            groups.reset();
            let mut names = name::Names {
                bytes: engine_native_memory::walk::Buffer::Count,
                len: 0,
                required: 0,
            };
            for &(army, _) in &world.armies[1..] {
                let member = read::<u64>(&live, army + NAME_GROUP_MEMBER).unwrap() as usize;
                assert_eq!(
                    name::inputs(&live, world.base(), member, &mut groups, &mut names),
                    Ok(name::Naming::DoesNotFit),
                    "pass {pass}"
                );
            }
            assert_eq!((names.len, names.required), (0, 5), "pass {pass}");
        }
    }

    #[test]
    fn a_second_division_of_a_template_costs_only_the_member_copy() {
        let world = World::new(3, 1);
        // SAFETY: the world owns the complete synthetic graph, alive and immutable through every read.
        let counted = Counted {
            inner: unsafe { engine_native_memory::memory::LiveMemory::new() },
            calls: Cell::new(0),
            bytes: Cell::new(0),
        };
        let mut context = Pass::new(&world);
        let mut names = name::Names {
            bytes: engine_native_memory::walk::Buffer::Growing(&mut context.bytes),
            len: 0,
            required: 0,
        };
        let mut costs = Vec::new();
        for &(army, _) in &world.armies[1..] {
            let member = read::<u64>(&counted, army + NAME_GROUP_MEMBER).unwrap() as usize;
            let before = counted.calls.get();
            let row = name::inputs(
                &counted,
                world.base(),
                member,
                &mut context.groups,
                &mut names,
            );
            assert!(matches!(row, Ok(name::Naming::Row(row)) if row.group.1 != 0));
            costs.push(counted.calls.get() - before);
        }
        assert!(costs[0] > 1);
        assert_eq!(costs[1], 1);
    }

    #[test]
    #[ignore = "timing benchmark: cargo test -p engine --release scale -- --ignored --nocapture"]
    fn division_stats_and_naming_capture_cost_at_scale() {
        for (count, templates) in [(5_000, 64), (5_000, 1_250), (10_000, 64), (10_000, 2_500)] {
            let world = World::new(count, templates);
            let members: Vec<usize> = world
                .armies
                .iter()
                .map(|&(army, _)| {
                    // SAFETY: the world owns the complete synthetic graph, alive and immutable through every read.
                    let live = unsafe { engine_native_memory::memory::LiveMemory::new() };
                    read::<u64>(&live, army + NAME_GROUP_MEMBER).unwrap() as usize
                })
                .collect();
            let context = std::cell::RefCell::new(Pass::new(&world));
            type Run<'a> = (
                &'a str,
                &'a dyn Fn(&Counted<engine_native_memory::memory::LiveMemory>) -> usize,
            );
            let runs: [Run<'_>; 3] = [
                (
                    "capture_stats (identity, one 12-span scatter, commander, name)",
                    &|m| pass(m, &world, &mut context.borrow_mut()),
                ),
                ("batched name read", &|m| {
                    let mut context = context.borrow_mut();
                    let context = &mut *context;
                    context.groups.reset();
                    let mut names = name::Names {
                        bytes: engine_native_memory::walk::Buffer::Growing(&mut context.bytes),
                        len: 0,
                        required: 0,
                    };
                    let mut named = 0;
                    for &member in &members {
                        if let Ok(name::Naming::Row(row)) =
                            name::inputs(m, world.base(), member, &mut context.groups, &mut names)
                        {
                            names.len += row.custom.1;
                            named += 1;
                        }
                    }
                    named
                }),
                ("per-field name reader (before)", &|m| {
                    let mut keys = Default::default();
                    members
                        .iter()
                        .map(|&member| {
                            usize::from(
                                reference_naming(m, world.base(), member, &mut keys).is_ok(),
                            )
                        })
                        .sum()
                }),
            ];
            for (label, run) in runs {
                // SAFETY: the world owns the complete synthetic graph, alive and immutable through every read.
                let counted = Counted {
                    inner: unsafe { engine_native_memory::memory::LiveMemory::new() },
                    calls: Cell::new(0),
                    bytes: Cell::new(0),
                };
                assert_eq!(run(&counted), count, "{label}");
                let (calls, bytes) = (counted.calls.get(), counted.bytes.get());
                let mut best = u128::MAX;
                for _ in 0..7 {
                    let started = std::time::Instant::now();
                    std::hint::black_box(run(&counted));
                    best = best.min(started.elapsed().as_nanos());
                }
                println!(
                    "{count} divisions, {templates} templates, {label}: {:.3} ms/pass best of 7, {:.0} ns/division, {:.2} guarded calls and {:.0} bytes per division",
                    best as f64 / 1e6,
                    best as f64 / count as f64,
                    calls as f64 / count as f64,
                    bytes as f64 / count as f64,
                );
            }
        }
    }
}
