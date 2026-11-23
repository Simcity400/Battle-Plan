use super::*;
use crate::{cpdx_array, memory::SliceMemory};

struct IdMemory<'a> {
    memory: SliceMemory<'a>,
    failures: &'a [usize],
    gathers: std::cell::Cell<usize>,
}

impl Memory for IdMemory<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        !self
            .failures
            .iter()
            .any(|fault| (address..address + output.len()).contains(fault))
            && self.memory.copy(address, output)
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.gathers.set(self.gathers.get() + 1);
        for (index, (address, output)) in addresses.into_iter().zip(destinations).enumerate() {
            if !output.is_empty() && (address == 0 || !self.copy(address, output)) {
                return index;
            }
        }
        N
    }
}

#[test]
fn ordered_state_ids_batch_nulls_duplicates_and_the_final_short_batch() {
    let mut world = crate::ccountry::synthetic::World::new(0x4000_0000, 1, 0x1100);
    let state = world.alloc(ID + 4);
    world.u32(state + ID, 77);
    let list = world.alloc(130 * 8);
    for index in 0..130 {
        world.u64(
            list + index * 8,
            if index % 3 == 0 { 0 } else { state as u64 },
        );
    }
    let memory = IdMemory {
        memory: world.memory(),
        failures: &[],
        gathers: Default::default(),
    };
    let mut ids = [u32::MAX; 130];
    assert_eq!(
        copy_ids(
            &memory,
            Array {
                data: list,
                count: 130,
            },
            &mut ids,
            "pointer fault",
            "id fault",
        ),
        Ok(())
    );
    assert_eq!(memory.gathers.get(), 6);
    for (index, id) in ids.into_iter().enumerate() {
        assert_eq!(id, if index % 3 == 0 { 0 } else { 77 });
    }
    assert_eq!(
        copy_ids(
            &memory,
            Array::default(),
            &mut [],
            "pointer fault",
            "id fault",
        ),
        Ok(())
    );
    assert_eq!(memory.gathers.get(), 6);
}

#[test]
fn an_earlier_state_id_failure_precedes_a_later_pointer_failure() {
    let mut world = crate::ccountry::synthetic::World::new(0x4000_0000, 1, 0x1100);
    let state = world.alloc(ID + 4);
    world.u32(state + ID, 77);
    let list = world.alloc(24);
    world.u64(list, state as u64);
    world.u64(list + 8, 0);
    world.u64(list + 16, state as u64);
    let source = Array {
        data: list,
        count: 3,
    };
    let faults = [state + ID, list + 16];
    let memory = IdMemory {
        memory: world.memory(),
        failures: &faults,
        gathers: Default::default(),
    };
    assert_eq!(
        copy_ids(&memory, source, &mut [9; 3], "pointer fault", "id fault"),
        Err("id fault")
    );
    let memory = IdMemory {
        failures: &faults[1..],
        ..memory
    };
    let mut output = [9; 3];
    assert_eq!(
        copy_ids(&memory, source, &mut output, "pointer fault", "id fault"),
        Err("pointer fault")
    );
    assert_eq!(output, [77, 0, 9]);
}

fn put(bytes: &mut [u8], offset: usize, data: &[u8]) {
    bytes[offset..offset + data.len()].copy_from_slice(data);
}

fn states_with_holes() -> Vec<u8> {
    let mut bytes = vec![0_u8; 0x1000];
    put(&mut bytes, cgame_state::STATES, &0x1300_u64.to_le_bytes());
    put(
        &mut bytes,
        cgame_state::STATES + cpdx_array::CAPACITY,
        &5_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        cgame_state::STATES + cpdx_array::COUNT,
        &4_u32.to_le_bytes(),
    );
    for (slot, pointer) in [(0, u64::MAX), (1, 0x1400), (3, 0x1600), (4, u64::MAX)] {
        put(&mut bytes, 0x300 + slot * 8, &pointer.to_le_bytes());
    }
    for (offset, id, owner, controller) in [
        (0x400, 1_u32, 0_u32, 0xffff_fffe_u32),
        (0x600, 3_u32, 0x8000_0001_u32, 0_u32),
    ] {
        put(&mut bytes, offset, &(VTABLE_RVA.0 as u64).to_le_bytes());
        put(&mut bytes, offset + ID, &id.to_le_bytes());
        put(&mut bytes, offset + OWNER, &owner.to_le_bytes());
        put(&mut bytes, offset + CONTROLLER, &controller.to_le_bytes());
    }
    bytes
}

struct MapMemory(std::collections::BTreeMap<usize, u8>);
impl Memory for MapMemory {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        for (index, byte) in destination.iter_mut().enumerate() {
            match self.0.get(&(address + index)) {
                Some(&value) => *byte = value,
                None => return false,
            }
        }
        true
    }
}
impl MapMemory {
    fn put(&mut self, address: usize, data: &[u8]) {
        self.0
            .extend(data.iter().enumerate().map(|(i, &b)| (address + i, b)));
    }
}

fn put_string(memory: &mut MapMemory, at: usize, heap: usize, text: &[u8]) {
    use crate::msvc_string::{CAPACITY, INLINE_CAPACITY, LENGTH};
    let mut header = [0_u8; 32];
    if text.len() as u64 <= INLINE_CAPACITY {
        header[..text.len()].copy_from_slice(text);
        header[CAPACITY..CAPACITY + 8].copy_from_slice(&INLINE_CAPACITY.to_le_bytes());
    } else {
        header[..8].copy_from_slice(&(heap as u64).to_le_bytes());
        header[CAPACITY..CAPACITY + 8].copy_from_slice(&(text.len() as u64).to_le_bytes());
        memory.put(heap, text);
        memory.put(heap + text.len(), &[0]);
    }
    header[LENGTH..LENGTH + 8].copy_from_slice(&(text.len() as u64).to_le_bytes());
    memory.put(at, &header);
}

fn named_states(count: usize, names: &dyn Fn(usize) -> (Vec<u8>, Vec<u8>)) -> MapMemory {
    let mut memory = MapMemory(std::collections::BTreeMap::new());
    const SLOTS: usize = 0x10_0000;
    memory.put(cgame_state::STATES, &(SLOTS as u64).to_le_bytes());
    let slots = (count + 1) as u32;
    memory.put(
        cgame_state::STATES + cpdx_array::CAPACITY,
        &slots.to_le_bytes(),
    );
    memory.put(
        cgame_state::STATES + cpdx_array::COUNT,
        &slots.to_le_bytes(),
    );
    memory.put(SLOTS, &0_u64.to_le_bytes());
    for index in 1..=count {
        let state = 0x20_0000 + index * 0x1000;
        memory.put(SLOTS + index * 8, &(state as u64).to_le_bytes());
        memory.put(state, &(VTABLE_RVA.0 as u64).to_le_bytes());
        memory.put(state + PRIMARY_DISPLAY_NAME, &[0; 0xd0]);
        memory.put(state + ID, &(index as u32).to_le_bytes());
        let (custom, owner) = names(index);
        put_string(
            &mut memory,
            state + PRIMARY_DISPLAY_NAME,
            state + 0x900,
            &custom,
        );
        put_string(
            &mut memory,
            state + SECONDARY_DISPLAY_NAME,
            state + 0xa00,
            &owner,
        );
    }
    memory
}

fn census_rows(count: usize) -> Vec<Row> {
    (1..=count)
        .map(|id| Row {
            id: id as u32,
            ..Row::default()
        })
        .collect()
}

#[test]
fn state_rows_and_names_grow_before_first_and_later_publication() {
    let mut rows = Vec::new();
    let mut text = Vec::new();
    for count in [50, 70] {
        let mut memory = named_states(count, &|_| (vec![b'p'; 31], vec![b'o'; 19]));
        for index in 1..=count {
            let state = 0x20_0000 + index * 0x1000;
            memory.put(state + OWNER, &0_u32.to_le_bytes());
            memory.put(state + CONTROLLER, &0_u32.to_le_bytes());
            memory.put(state + MANPOWER_AVAILABLE, &[0; 12]);
        }
        let mut sources = 0;
        assert_eq!(name_demand(&memory, 0, 0), Ok(count * 50));
        let census = walk_with(&memory, 0, 0, &mut rows, |_| sources += 1);
        assert_eq!(census, Outcome::Rows { len: count });
        assert_eq!(sources, count);
        let names = read_names(&memory, 0, census, &mut rows, &mut text);
        assert_eq!(names.outcome, census);
        assert_eq!(
            (names.bytes_len, names.required_bytes),
            (count * 50, count * 50)
        );
        assert_eq!(rows[count - 1].owner_name.len, 19);
        assert_eq!(rows[count - 1].id, count as u32);
    }
}

#[test]
fn both_display_names_are_copied_exactly_with_off_thread_growth() {
    let long = b"Synthetic State Display Name".to_vec();
    let memory = named_states(3, &|index| match index {
        1 => (Vec::new(), b"Region Beta".to_vec()),
        2 => (long.clone(), Vec::new()),
        _ => (b"Tiny".to_vec(), b"Test Region AAA".to_vec()),
    });
    let mut rows = census_rows(3);
    let mut arena = vec![0_u8; 256];
    let walk = read_names(&memory, 0, Outcome::Rows { len: 3 }, &mut rows, &mut arena);
    assert_eq!(walk.outcome, Outcome::Rows { len: 3 });
    let text = |span: NameSpan| &arena[span.start..span.start + span.len];
    assert_eq!(text(rows[0].custom_name), b"");
    assert_eq!(text(rows[0].owner_name), b"Region Beta");
    assert_eq!(text(rows[1].custom_name), long.as_slice());
    assert_eq!(text(rows[1].owner_name), b"");
    assert_eq!(text(rows[2].custom_name), b"Tiny");
    assert_eq!(text(rows[2].owner_name), b"Test Region AAA");
    let total = 11 + long.len() + 4 + 15;
    assert_eq!((walk.bytes_len, walk.required_bytes), (total, total));
    let mut small = vec![0_u8; 20];
    let mut rows = census_rows(3);
    let walk = read_names(&memory, 0, Outcome::Rows { len: 3 }, &mut rows, &mut small);
    assert_eq!(walk.outcome, Outcome::Rows { len: 3 });
    assert_eq!((walk.bytes_len, walk.required_bytes), (total, total));
    let mut rows = census_rows(3);
    let walk = read_names(
        &memory,
        0,
        Outcome::Truncated {
            len: 3,
            unread_rows: 4,
        },
        &mut rows,
        &mut arena,
    );
    assert_eq!(
        walk.outcome,
        Outcome::Truncated {
            len: 3,
            unread_rows: 4
        }
    );
    let unavailable = Outcome::Unavailable { reason: "census" };
    assert_eq!(
        read_names(&memory, 0, unavailable, &mut census_rows(3), &mut arena).outcome,
        unavailable
    );
    let mut broken = named_states(1, &|_| (long.clone(), Vec::new()));
    broken.0.remove(&(0x20_1000 + 0x900 + long.len()));
    assert!(matches!(
        read_names(
            &broken,
            0,
            Outcome::Rows { len: 1 },
            &mut census_rows(1),
            &mut arena
        )
        .outcome,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn scripted_controller_branch_is_carried_without_copying_cached_groups() {
    let mut memory = named_states(3, &|_| (b"a".to_vec(), Vec::new()));
    for (id, count) in [(1, 1_i32), (2, 0), (3, 7)] {
        memory.put(
            0x20_0000 + id * 0x1000 + CONTROLLER_GROUP_COUNT,
            &count.to_le_bytes(),
        );
        memory.put(
            0x20_0000 + id * 0x1000 + BEST_VP_INDEX,
            &(count - 2).to_le_bytes(),
        );
    }
    let mut rows = census_rows(3);
    let mut arena = vec![0; 32];
    let outcome = Outcome::Rows { len: 3 };
    assert_eq!(
        read_names_carried_scripted(
            &memory,
            0,
            outcome,
            &mut rows,
            walk::Buffer::Growing(&mut arena),
            |_| None,
            |_| (true, true)
        )
        .outcome,
        outcome
    );
    assert_eq!(
        rows.iter()
            .map(|row| row.single_controller)
            .collect::<Vec<_>>(),
        [Some(true), Some(false), Some(false)]
    );
    assert_eq!(
        rows.iter().map(|row| row.best_vp_index).collect::<Vec<_>>(),
        [Some(-1), Some(-2), Some(5)]
    );
    let mut carried = census_rows(3);
    let held = rows
        .iter()
        .map(|row| {
            (
                [
                    &arena[row.custom_name.start..][..row.custom_name.len],
                    &[][..],
                ],
                row.single_controller,
                row.best_vp_index,
                row.marked_owners
                    .map(|span| &arena[span.start..][..span.len]),
            )
        })
        .collect::<Vec<_>>();
    let mut fresh_arena = vec![0; 32];
    let empty = MapMemory(Default::default());
    let mut headers = empty;
    for (at, byte) in &memory.0 {
        if *at < 0x20_0000 {
            headers.0.insert(*at, *byte);
        }
    }
    assert_eq!(
        read_names_carried_scripted(
            &headers,
            0,
            outcome,
            &mut carried,
            walk::Buffer::Growing(&mut fresh_arena),
            |index| Some(held[index]),
            |_| (true, true)
        )
        .outcome,
        outcome
    );
    assert_eq!(carried, rows);
    assert_eq!(fresh_arena, arena);
    memory.0.remove(&(0x20_1000 + CONTROLLER_GROUP_COUNT));
    assert_eq!(
        read_names_carried_scripted(
            &memory,
            0,
            outcome,
            &mut rows,
            walk::Buffer::Growing(&mut arena),
            |_| None,
            |_| (true, true)
        )
        .outcome,
        outcome
    );
    assert_eq!(rows[0].single_controller, None);
    assert_eq!(
        read_names(&memory, 0, outcome, &mut rows, &mut arena).outcome,
        outcome
    );
    assert!(
        rows.iter()
            .all(|row| row.best_vp_index.is_none() && row.single_controller.is_none())
    );
}

#[test]
fn movement_infrastructure_is_read_per_state_over_the_census() {
    use crate::{cbuilding_database, cbuilding_template};
    let mut memory = MapMemory(std::collections::BTreeMap::new());
    memory.put(cgame_state::STATES, &0x2000_u64.to_le_bytes());
    memory.put(
        cgame_state::STATES + cpdx_array::CAPACITY,
        &2_u32.to_le_bytes(),
    );
    memory.put(
        cgame_state::STATES + cpdx_array::COUNT,
        &2_u32.to_le_bytes(),
    );
    memory.put(0x2000, &0_u64.to_le_bytes());
    memory.put(0x2008, &0x3000_u64.to_le_bytes());
    memory.put(0x3000, &(VTABLE_RVA.0 as u64).to_le_bytes());
    memory.put(0x3000 + ID, &1_u32.to_le_bytes());
    memory.put(
        cbuilding_database::SINGLETON_RVA.0,
        &0x4000_u64.to_le_bytes(),
    );
    memory.put(0x4000 + 0x3b0, &0x5000_u64.to_le_bytes());
    memory.put(0x5000 + cbuilding_template::SCOPE, &5_i32.to_le_bytes());
    memory.put(0x5000 + cbuilding_template::LOOKUP_ENABLED_CANDIDATE, &[0]);
    memory.put(0x033e_5080, &(-10_000_i64).to_le_bytes());
    let census = Outcome::Rows { len: 1 };
    let mut rows = [Row {
        id: 1,
        ..Row::default()
    }];
    assert_eq!(
        read_movement_infrastructure(&memory, 0, 0, census, &mut rows),
        census
    );
    assert_eq!(rows[0].movement_infrastructure, -50_000);
    memory.put(0x033e_5080, &(-12_000_i64).to_le_bytes());
    assert_eq!(
        read_movement_infrastructure(&memory, 0, 0, census, &mut rows),
        census
    );
    assert_eq!(rows[0].movement_infrastructure, -60_000);
    memory.0.remove(&0x033e_5080);
    assert!(matches!(
        read_movement_infrastructure(&memory, 0, 0, census, &mut rows),
        Outcome::Unavailable { .. }
    ));
    memory.put(0x033e_5080, &(-10_000_i64).to_le_bytes());

    let unavailable = Outcome::Unavailable { reason: "census" };
    assert_eq!(
        read_movement_infrastructure(&memory, 0, 0, unavailable, &mut rows),
        unavailable
    );
    let mut missing = [Row {
        id: 7,
        ..Row::default()
    }];
    assert!(matches!(
        read_movement_infrastructure(&memory, 0, 0, census, &mut missing),
        Outcome::Unavailable { .. }
    ));
    memory.put(0x3000, &0_u64.to_le_bytes());
    assert!(matches!(
        read_movement_infrastructure(&memory, 0, 0, census, &mut rows),
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn infrastructure_globals_keep_cold_faults_and_lazy_ordinal_acquisition() {
    use crate::{
        cbuilding_database,
        cprovince::buildings as cbuilding_status,
        cbuilding_template,
    };
    use std::cell::RefCell;
    struct Traced<'a> {
        memory: &'a MapMemory,
        reads: RefCell<Vec<(usize, usize)>>,
        fault: Option<usize>,
    }
    impl Memory for Traced<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let ordinal = self.reads.borrow().len();
            self.reads.borrow_mut().push((address, output.len()));
            self.fault != Some(ordinal) && self.memory.copy(address, output)
        }
    }
    let mut memory = MapMemory(std::collections::BTreeMap::new());
    let first = 0x3000;
    let second = 0x6000;
    let definition = 0x5000;
    memory.put(first, &(VTABLE_RVA.0 as u64).to_le_bytes());
    memory.put(second, &(VTABLE_RVA.0 as u64).to_le_bytes());
    memory.put(
        cbuilding_database::SINGLETON_RVA.0,
        &0x4000_u64.to_le_bytes(),
    );
    memory.put(0x4000 + 0x3b0, &(definition as u64).to_le_bytes());
    memory.put(definition + cbuilding_template::SCOPE, &5_i32.to_le_bytes());
    memory.put(
        definition + cbuilding_template::LOOKUP_ENABLED_CANDIDATE,
        &[1],
    );
    memory.put(
        definition + cbuilding_template::DATABASE_ORDINAL,
        &0_u32.to_le_bytes(),
    );
    memory.put(
        first + BUILDING_STATUS + cbuilding_status::BUILDING_INDEX_COUNT,
        &0_i32.to_le_bytes(),
    );
    let indices = second + BUILDING_STATUS + cbuilding_status::BUILDING_INDICES;
    memory.put(indices, &0x7000_u64.to_le_bytes());
    memory.put(indices + 8, &1_u32.to_le_bytes());
    memory.put(indices + 12, &1_u32.to_le_bytes());
    memory.put(0x7000, &cbuilding_status::NO_BUILDING_INDEX.to_le_bytes());
    memory.put(0x033e_5080, &(-10_000_i64).to_le_bytes());
    let cold = [
        (first, 8),
        (cbuilding_database::SINGLETON_RVA.0, 8),
        (0x4000 + 0x3b0, 8),
        (definition + cbuilding_template::SCOPE, 4),
        (definition + cbuilding_template::LOOKUP_ENABLED_CANDIDATE, 1),
        (
            first + BUILDING_STATUS + cbuilding_status::BUILDING_INDEX_COUNT,
            4,
        ),
        (0x033e_5080, 8),
    ];
    for fault in 0..cold.len() {
        let traced = Traced {
            memory: &memory,
            reads: RefCell::new(Vec::new()),
            fault: Some(fault),
        };
        let mut shared = None;
        assert!(movement_infrastructure_shared(&traced, 0, first, &mut shared).is_err());
        assert!(shared.is_none());
        assert_eq!(*traced.reads.borrow(), cold[..=fault]);
    }
    let traced = Traced {
        memory: &memory,
        reads: RefCell::new(Vec::new()),
        fault: None,
    };
    let mut shared = None;
    assert_eq!(
        movement_infrastructure_shared(&traced, 0, first, &mut shared),
        Ok(-50_000)
    );
    assert_eq!(*traced.reads.borrow(), cold);
    assert_eq!(shared.as_ref().unwrap().ordinal, None);
    traced.reads.borrow_mut().clear();
    assert_eq!(
        movement_infrastructure_shared(&traced, 0, second, &mut shared),
        Ok(-50_000)
    );
    assert_eq!(
        *traced.reads.borrow(),
        [
            (second, 8),
            (indices + 12, 4),
            (indices, 16),
            (definition + cbuilding_template::DATABASE_ORDINAL, 4),
            (0x7000, 4),
        ]
    );
    traced.reads.borrow_mut().clear();
    assert_eq!(
        movement_infrastructure_shared(&traced, 0, second, &mut shared),
        Ok(-50_000)
    );
    assert_eq!(
        *traced.reads.borrow(),
        [(second, 8), (indices + 12, 4), (indices, 16), (0x7000, 4)]
    );
}

#[test]
fn state_slots_and_native_country_tags_are_preserved() {
    let bytes = states_with_holes();
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let mut rows = vec![Row::default(); 5];
    assert_eq!(
        walk(&memory, 0, 0x1000, &mut rows),
        Outcome::Rows { len: 2 }
    );
    assert_eq!(
        &rows[..2],
        &[
            Row {
                id: 1,
                owner: 0,
                controller: 0xffff_fffe,
                ..Row::default()
            },
            Row {
                id: 3,
                owner: 0x8000_0001,
                controller: 0,
                ..Row::default()
            },
        ]
    );
    assert_eq!(
        walk_with_buffer(&memory, 0, 0x1000, crate::walk::Buffer::Count, |_| {}),
        Outcome::Truncated {
            len: 0,
            unread_rows: 2,
        }
    );
}

#[test]
fn country_tag_scatter_keeps_raw_words_and_ordered_failure_prefixes() {
    use std::cell::{Cell, RefCell};
    struct Observed {
        calls: Cell<usize>,
        reads: RefCell<Vec<(usize, usize)>>,
        fault: Option<usize>,
    }
    impl Observed {
        fn new(fault: Option<usize>) -> Self {
            Self {
                calls: Cell::new(0),
                reads: RefCell::default(),
                fault,
            }
        }
        fn field(&self, address: usize, output: &mut [u8]) -> bool {
            let ordinal = self.reads.borrow().len();
            self.reads.borrow_mut().push((address, output.len()));
            if self.fault == Some(ordinal) {
                output[0] = 0xff;
                return false;
            }
            output.copy_from_slice(
                &(if ordinal == 0 {
                    0x8000_0001_u32
                } else {
                    0xffff_fffe_u32
                })
                .to_le_bytes(),
            );
            true
        }
    }
    impl Memory for Observed {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.calls.set(self.calls.get() + 1);
            self.field(address, output)
        }
        fn copy_scattered<const N: usize>(
            &self,
            addresses: [usize; N],
            output: &mut [&mut [u8]; N],
        ) -> usize {
            struct Fields<'a>(&'a Observed);
            impl Memory for Fields<'_> {
                fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                    self.0.field(address, output)
                }
            }
            self.calls.set(self.calls.get() + 1);
            Fields(self).copy_scattered(addresses, output)
        }
    }
    let serial = |memory: &Observed, pointer: usize| -> Result<[u32; 2], &'static str> {
        let at = |offset| pointer.checked_add(offset).ok_or("state address overflow");
        let owner = read::<u32>(memory, at(OWNER)?).ok_or("state owner unreadable")?;
        let controller =
            read::<u32>(memory, at(CONTROLLER)?).ok_or("state controller unreadable")?;
        Ok([owner, controller])
    };
    for pointer in [
        0x1400,
        usize::MAX - CONTROLLER,
        usize::MAX - CONTROLLER + 1,
        usize::MAX - OWNER + 1,
    ] {
        for fault in [None, Some(0), Some(1)] {
            let actual = Observed::new(fault);
            let expected = Observed::new(fault);
            let result = read_country_tags(&actual, pointer);
            assert_eq!(result, serial(&expected, pointer));
            assert_eq!(*actual.reads.borrow(), *expected.reads.borrow());
            if pointer.checked_add(CONTROLLER).is_none() {
                assert_eq!(actual.calls.get(), expected.calls.get());
            } else {
                assert_eq!(actual.calls.get(), 1);
            }
            if pointer == 0x1400 && fault.is_none() {
                assert_eq!(result, Ok([0x8000_0001, 0xffff_fffe]));
                assert_eq!(
                    *actual.reads.borrow(),
                    [(pointer + OWNER, 4), (pointer + CONTROLLER, 4)]
                );
                assert_eq!(expected.calls.get(), 2);
            }
        }
    }
}

#[test]
fn invalid_state_rows_never_publish_a_successful_prefix() {
    for (offset, replacement, reason) in [
        (0x600, 0_u32, "state vtable mismatch"),
        (0x600 + ID, 1_u32, "state id differs from slot"),
        (0x600 + ID, 0_u32, "state id differs from slot"),
    ] {
        let mut bytes = states_with_holes();
        put(&mut bytes, offset, &replacement.to_le_bytes());
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut rows = vec![Row::default(); 5];
        assert_eq!(
            walk(&memory, 0, 0x1000, &mut rows),
            Outcome::Unavailable { reason }
        );
    }
    struct Fault<'a> {
        memory: SliceMemory<'a>,
        address: usize,
    }
    impl Memory for Fault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address <= self.address && self.address < address.saturating_add(output.len()) {
                return false;
            }
            self.memory.copy(address, output)
        }
    }
    for (offset, reason) in [
        (ID, "state id unreadable"),
        (OWNER, "state owner unreadable"),
        (CONTROLLER, "state controller unreadable"),
        (MANPOWER_AVAILABLE, "state manpower unreadable"),
        (POPULATION, "state manpower unreadable"),
    ] {
        let bytes = states_with_holes();
        let memory = Fault {
            memory: SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            },
            address: 0x1600 + offset,
        };
        let mut rows = vec![Row::default(); 5];
        assert_eq!(
            walk(&memory, 0, 0x1000, &mut rows),
            Outcome::Unavailable { reason }
        );
    }
    let mut bytes = states_with_holes();
    put(&mut bytes, cgame_state::STATES, &0x1fe8_u64.to_le_bytes());
    put(&mut bytes, 0xff0, &0x1400_u64.to_le_bytes());
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let mut rows = vec![Row::default(); 5];
    assert_eq!(
        walk(&memory, 0, 0x1000, &mut rows),
        Outcome::Unavailable {
            reason: "table pointer unreadable",
        }
    );
}

#[test]
fn invalid_or_empty_state_tables_are_unavailable() {
    for (capacity, count, reason) in [
        (4_u32, 5_u32, "state array unreadable or invalid"),
        (u32::MAX, 4, "state array unreadable or invalid"),
        (4, u32::MAX, "state array unreadable or invalid"),
        (0, 0, "state table has no state slots"),
        (1, 1, "state table has no state slots"),
    ] {
        let mut bytes = states_with_holes();
        put(
            &mut bytes,
            cgame_state::STATES + cpdx_array::CAPACITY,
            &capacity.to_le_bytes(),
        );
        put(
            &mut bytes,
            cgame_state::STATES + cpdx_array::COUNT,
            &count.to_le_bytes(),
        );
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut rows = vec![Row::default(); 5];
        assert_eq!(
            walk(&memory, 0, 0x1000, &mut rows),
            Outcome::Unavailable { reason }
        );
    }
    let mut bytes = states_with_holes();
    bytes[0x308..0x320].fill(0);
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    assert_eq!(
        walk(&memory, 0, 0x1000, &mut Vec::new()),
        Outcome::Unavailable {
            reason: "state table has no readable states",
        }
    );
}

#[test]
fn shared_state_sources_preserve_sparse_identity_and_native_flag_bytes() {
    let mut bytes = states_with_holes();
    bytes.resize(0x1000, 0);
    bytes[0x400 + DEMILITARIZED] = 2;
    bytes[0x600 + DEMILITARIZED] = 255;
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let mut rows = vec![Row::default(); 2];
    let mut sources = Vec::new();
    let parent = walk_with(&memory, 0, 0x1000, &mut rows, |source| sources.push(source));
    assert_eq!(
        sources,
        [
            Source {
                pointer: 0x1400,
                id: 1
            },
            Source {
                pointer: 0x1600,
                id: 3
            }
        ]
    );
    let mut flags = vec![FlagRow::default(); 2];
    assert_eq!(
        read_flags(&memory, 0, 0x1000, &sources, parent, &mut flags),
        Outcome::Rows { len: 2 }
    );
    assert_eq!(
        flags,
        [
            FlagRow {
                border_conflict_active: 0,
                border_conflict_countries: Vec::new(),
                state: 1,
                demilitarized: 2
            },
            FlagRow {
                border_conflict_active: 0,
                border_conflict_countries: Vec::new(),
                state: 3,
                demilitarized: 255
            }
        ]
    );
    assert_eq!(
        read_flags(&memory, 0, 0x1000, &sources, parent, &mut flags[..1]),
        Outcome::Truncated {
            len: 1,
            unread_rows: 1
        }
    );
}

#[test]
fn flags_reject_stale_sources_faults_and_incomplete_censuses() {
    let mut bytes = states_with_holes();
    bytes.resize(0x1000, 0);
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let mut flags = vec![FlagRow::default(); 2];
    let valid = [
        Source {
            pointer: 0x1400,
            id: 1,
        },
        Source {
            pointer: 0x1600,
            id: 3,
        },
    ];
    for sources in [
        [valid[0], valid[0]],
        [valid[0], Source { pointer: 0, id: 3 }],
        [
            valid[0],
            Source {
                pointer: 0x1600,
                id: 2,
            },
        ],
        [
            valid[0],
            Source {
                pointer: 0x1600,
                id: 4,
            },
        ],
    ] {
        assert!(matches!(
            read_flags(
                &memory,
                0,
                0x1000,
                &sources,
                Outcome::Rows { len: 2 },
                &mut flags
            ),
            Outcome::Unavailable { .. }
        ));
    }
    for parent in [
        Outcome::Rows { len: 0 },
        Outcome::Rows { len: 3 },
        Outcome::Truncated {
            len: 2,
            unread_rows: 0,
        },
        Outcome::Unavailable {
            reason: "census fault",
        },
    ] {
        assert!(matches!(
            read_flags(&memory, 0, 0x1000, &valid, parent, &mut flags),
            Outcome::Unavailable { .. }
        ));
    }
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes[..0x600 + DEMILITARIZED],
    };
    assert_eq!(
        read_flags(
            &memory,
            0,
            0x1000,
            &valid,
            Outcome::Rows { len: 2 },
            &mut flags
        ),
        Outcome::Unavailable {
            reason: "state flag bytes unreadable"
        }
    );
}

#[test]
fn state_census_and_flags_keep_primary_read_order_without_terminal_rereads() {
    use std::cell::RefCell;
    struct Traced<'a> {
        memory: SliceMemory<'a>,
        reads: RefCell<Vec<(usize, usize)>>,
    }
    impl Memory for Traced<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, destination.len()));
            self.memory.copy(address, destination)
        }
    }
    let mut bytes = states_with_holes();
    bytes.resize(0x1000, 0);
    bytes[0x400 + DEMILITARIZED] = 2;
    bytes[0x600 + DEMILITARIZED] = 255;
    let memory = Traced {
        memory: SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        },
        reads: RefCell::new(Vec::new()),
    };
    let mut sources = Vec::new();
    let parent = walk_with(&memory, 0, 0x1000, &mut vec![Row::default(); 2], |source| {
        sources.push(source)
    });
    assert_eq!(parent, Outcome::Rows { len: 2 });
    let mut expected = vec![
        (0x1000 + cgame_state::STATES, 16),
        (0x1308, 8),
        (0x1310, 8),
        (0x1318, 8),
    ];
    for pointer in [0x1400, 0x1600] {
        expected.extend([
            (pointer, 8),
            (pointer + ID, 4),
            (pointer + OWNER, 4),
            (pointer + CONTROLLER, 4),
            (pointer + MANPOWER_AVAILABLE, MANPOWER_SPAN),
            (pointer + CATEGORY, 8),
        ]);
    }
    assert_eq!(*memory.reads.borrow(), expected);
    memory.reads.borrow_mut().clear();
    let mut flags = vec![FlagRow::default(); 2];
    assert_eq!(
        read_flags(&memory, 0, 0x1000, &sources, parent, &mut flags),
        parent
    );
    assert_eq!(
        flags
            .iter()
            .map(|row| (row.state, row.demilitarized))
            .collect::<Vec<_>>(),
        [(1, 2), (3, 255)]
    );
    let mut expected = vec![(0x1000 + cgame_state::STATES, 16)];
    for source in &sources {
        expected.extend([
            (0x1300 + source.id as usize * 8, 8),
            (source.pointer, 8),
            (source.pointer + ID, 4),
            (source.pointer + DEMILITARIZED, 2),
        ]);
    }
    assert_eq!(*memory.reads.borrow(), expected);
}

#[test]
fn continent_capture_keeps_cached_identity_null_and_independent_failure() {
    const DATABASE_GLOBAL: usize = 0x033d_d4b8;
    struct ContinentMemory<'a>(&'a [u8]);
    impl Memory for ContinentMemory<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == DATABASE_GLOBAL && output.len() == 8 {
                output.copy_from_slice(&0x2000_u64.to_le_bytes());
                true
            } else {
                SliceMemory {
                    base: 0,
                    bytes: self.0,
                }
                .copy(address, output)
            }
        }
    }
    let mut bytes = vec![0; 0x6200];
    let mut put = |address: usize, data: &[u8]| {
        bytes[address..address + data.len()].copy_from_slice(data);
    };
    let (game, database, state_array, continents, state, template, null, continent) = (
        0x1000, 0x2000, 0x3000, 0x3100, 0x4000, 0x5000, 0x6000, 0x6100,
    );
    put(database, &0x029c_0988_u64.to_le_bytes());
    for (at, array, count) in [
        (game + cgame_state::STATES, state_array, 2_u32),
        (database + 0x58, continents, 2_u32),
    ] {
        put(at, &(array as u64).to_le_bytes());
        put(at + 8, &count.to_le_bytes());
        put(at + 12, &count.to_le_bytes());
    }
    put(state_array + 8, &(state as u64).to_le_bytes());
    put(state + TEMPLATE, &(template as u64).to_le_bytes());
    put(
        template,
        &(crate::cstate_template::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        template + crate::cstate_template::CONTINENT,
        &(continent as u64).to_le_bytes(),
    );
    put(continents, &(null as u64).to_le_bytes());
    put(continents + 8, &(continent as u64).to_le_bytes());
    put(continent + 0x28, &[1]);
    put(continent + 0x2c, &991_u32.to_le_bytes());
    put(continent + 0x30, &1_u32.to_le_bytes());
    let mut rows = [Row {
        id: 1,
        owner: 123,
        ..Default::default()
    }];
    let read_rows = |bytes: &[u8], rows: &mut [Row]| {
        read_continents(
            &ContinentMemory(bytes),
            0,
            game,
            Outcome::Rows { len: 1 },
            rows,
        )
    };
    read_rows(&bytes, &mut rows);
    assert_eq!(rows[0].continent, Some(991));
    bytes[template + crate::cstate_template::CONTINENT
        ..template + crate::cstate_template::CONTINENT + 8]
        .copy_from_slice(&(null as u64).to_le_bytes());
    read_rows(&bytes, &mut rows);
    assert_eq!(rows[0].continent, Some(0));
    bytes[template + crate::cstate_template::CONTINENT
        ..template + crate::cstate_template::CONTINENT + 8]
        .copy_from_slice(&0_u64.to_le_bytes());
    read_rows(&bytes, &mut rows);
    assert_eq!(rows[0].continent, None);
    assert_eq!(rows[0].owner, 123);
}

#[test]
fn state_population_is_read_beside_available_manpower() {
    let mut bytes = states_with_holes();
    for (offset, available, locked, population) in
        [(0x400, 6_792_i32, 121_i32, 363_133_i32), (0x600, -5, -7, 0)]
    {
        put(
            &mut bytes,
            offset + MANPOWER_AVAILABLE,
            &available.to_le_bytes(),
        );
        put(&mut bytes, offset + MANPOWER_LOCKED, &locked.to_le_bytes());
        put(&mut bytes, offset + POPULATION, &population.to_le_bytes());
    }
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let mut rows = vec![Row::default(); 5];
    assert_eq!(
        walk(&memory, 0, 0x1000, &mut rows),
        Outcome::Rows { len: 2 }
    );
    assert_eq!(
        rows[..2]
            .iter()
            .map(|row| (
                row.id,
                row.manpower_available,
                row.manpower_locked,
                row.population
            ))
            .collect::<Vec<_>>(),
        [(1, 6_792, 121, 363_133), (3, -5, -7, 0)]
    );
    let mut manpower = [Manpower::default(); 2];
    read_manpower(&memory, 0, &[0x1400, 0x1600], &[1, 3], &mut manpower).unwrap();
    assert_eq!(
        manpower,
        [
            Manpower {
                available: 6_792,
                locked: 121,
                population: 363_133,
                category: 0
            },
            Manpower {
                available: -5,
                locked: -7,
                population: 0,
                category: 0
            }
        ]
    );
}

#[test]
fn state_category_is_named_by_its_authenticated_definitions_token() {
    let mut bytes = states_with_holes();
    for (offset, vtable, token) in [
        (0x100, CATEGORY_VTABLE_RVA.0, 0x2fab_u32),
        (0x140, VTABLE_RVA.0, 7),
    ] {
        put(&mut bytes, offset, &(vtable as u64).to_le_bytes());
        put(&mut bytes, offset + CATEGORY_TOKEN, &token.to_le_bytes());
    }
    put(&mut bytes, 0x400 + CATEGORY, &0x1100_u64.to_le_bytes());
    put(&mut bytes, 0x600 + CATEGORY, &0x1140_u64.to_le_bytes());
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let mut rows = vec![Row::default(); 5];
    assert_eq!(
        walk(&memory, 0, 0x1000, &mut rows),
        Outcome::Rows { len: 2 }
    );
    assert_eq!(
        rows[..2]
            .iter()
            .map(|row| (row.id, row.category))
            .collect::<Vec<_>>(),
        [(1, Some(0x2fab)), (3, None)]
    );
    let mut manpower = [Manpower::default(); 2];
    read_manpower(&memory, 0, &[0x1400, 0x1600], &[1, 3], &mut manpower).unwrap();
    assert_eq!(manpower.map(|state| state.category), [0x1100, 0x1140]);
    assert_eq!(category_token(&memory, 0, 0x1100), Ok(0x2fab));
    assert_eq!(
        category_token(&memory, 0, 0x1140),
        Err("state category vtable mismatch")
    );
    assert_eq!(
        category_token(&memory, 0, 0x9000),
        Err("state category unreadable")
    );
}

#[test]
fn contested_owners_copy_with_the_names_and_carry_exactly() {
    let mut memory = named_states(3, &|index| (vec![b'c'; index], b"Owner".to_vec()));
    let second = 0x20_2000;
    memory.put(second + CONTESTED_OWNERS, &((second + 0xb00) as u64).to_le_bytes());
    memory.put(second + CONTESTED_OWNERS + cpdx_array::CAPACITY, &2_u32.to_le_bytes());
    memory.put(second + CONTESTED_OWNERS + cpdx_array::COUNT, &2_u32.to_le_bytes());
    memory.put(second + 0xb00, &7_u32.to_le_bytes());
    memory.put(second + 0xb04, &0x8000_0001_u32.to_le_bytes());
    let third = 0x20_3000;
    memory.put(third + CONTESTED_OWNERS + cpdx_array::CAPACITY, &1_u32.to_le_bytes());
    memory.put(third + CONTESTED_OWNERS + cpdx_array::COUNT, &2_u32.to_le_bytes());
    let mut rows = census_rows(3);
    let mut arena = Vec::new();
    let walk = read_names(&memory, 0, Outcome::Rows { len: 3 }, &mut rows, &mut arena);
    assert_eq!(walk.outcome, Outcome::Rows { len: 3 });
    let words = |span: NameSpan| -> Vec<u32> {
        arena[span.start..][..span.len]
            .chunks_exact(4)
            .map(|word| u32::from_le_bytes(word.try_into().unwrap()))
            .collect()
    };
    assert_eq!(rows[0].marked_owners.map(words), Some(Vec::new()));
    assert_eq!(rows[1].marked_owners.map(words), Some(vec![7, 0x8000_0001]));
    assert_eq!(rows[2].marked_owners, None);
    let total = (1 + 5) + (2 + 5 + 8) + (3 + 5);
    assert_eq!((walk.bytes_len, walk.required_bytes), (total, total));
    let held = rows
        .iter()
        .map(|row| {
            let bytes = |span: NameSpan| &arena[span.start..][..span.len];
            (
                [bytes(row.custom_name), bytes(row.owner_name)],
                row.single_controller,
                row.best_vp_index,
                row.marked_owners.map(bytes),
            )
        })
        .collect::<Vec<_>>();
    let mut carried = census_rows(3);
    let mut carried_arena = Vec::new();
    let walk = read_names_carried(
        &memory,
        0,
        Outcome::Rows { len: 3 },
        &mut carried,
        walk::Buffer::Growing(&mut carried_arena),
        |index| (index == 1).then(|| held[index]),
    );
    assert_eq!(walk.outcome, Outcome::Rows { len: 3 });
    assert_eq!(carried, rows);
    assert_eq!(carried_arena[..total], arena[..total]);
}
