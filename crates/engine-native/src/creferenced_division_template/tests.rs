use super::*;
use engine_native_memory::memory::SliceMemory;

struct Fixture(Vec<u8>, std::cell::RefCell<Vec<(usize, usize)>>);
impl Memory for Fixture {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.1.borrow_mut().push((address, output.len()));
        let redirect = if address == subunit::NULL_OBJECT_RVA.0 {
            Some(0x9000_u64)
        } else if address == subunit::DATABASE_RVA.0 {
            Some(0x9800)
        } else {
            None
        };
        if let Some(pointer) = redirect
            && output.len() == 8
        {
            output.copy_from_slice(&pointer.to_le_bytes());
            return true;
        }
        SliceMemory {
            base: 0,
            bytes: &self.0,
        }
        .copy(address, output)
    }
}
impl Fixture {
    fn put(&mut self, at: usize, bytes: &[u8]) {
        self.0[at..at + bytes.len()].copy_from_slice(bytes);
    }
    fn word(&mut self, at: usize, value: u64) {
        self.put(at, &value.to_le_bytes());
    }
    fn array(&mut self, at: usize, data: u64, count: u32) {
        self.word(at, data);
        self.put(at + 8, &count.to_le_bytes());
        self.put(at + 12, &count.to_le_bytes());
    }
    fn text(&mut self, at: usize, bytes: &[u8]) {
        self.put(at, bytes);
        self.word(at + 16, bytes.len() as u64);
        self.word(at + 24, 15);
    }
    fn subunit(&mut self, at: usize, token: u32, index: i32) {
        self.word(at, subunit::VTABLE_RVA.0 as u64);
        self.put(at + subunit::TOKEN, &token.to_le_bytes());
        self.0[at + subunit::IS_REAL] = 1;
        self.put(at + subunit::DATABASE_INDEX, &index.to_le_bytes());
    }
}
fn fixture() -> Fixture {
    let mut memory = Fixture(vec![0; 0xc000], std::cell::RefCell::default());
    memory.array(0x1000 + crate::cgame_state::COUNTRIES, 0x2000, 2);
    memory.array(0x1000 + crate::cgame_state::COUNTRY_LINKS, 0x2100, 2);
    memory.word(0x2008, 0x3000);
    memory.put(0x2104, &1_u32.to_le_bytes());
    memory.word(0x3000, ccountry::VTABLE_RVA.0 as u64);
    memory.put(0x3008, &1_u32.to_le_bytes());
    memory.array(0x3000 + ccountry::DIVISION_TEMPLATES, 0x4000, 1);
    memory.word(0x4000, 0x5000);
    memory.word(0x5000, VTABLE_RVA.0 as u64);
    memory.word(0x5008, 0xffff_ffff_0000_0034);
    memory.text(0x5000 + NAME, &[b'A', 0, 255]);
    memory.text(0x5000 + NAME_KEY, b"fallback");
    memory.text(0x5000 + MODEL_ENTITY, b"");
    memory.array(0x5000 + REGIMENT_DATA, 0x6000, 2);
    memory.word(0x6000, 0x7000);
    memory.word(0x6008, 0);
    memory.put(0x5000 + CUSTOM_ICON, &(-1_i32).to_le_bytes());
    memory.put(0x5000 + COUNTRY_OPERAND_289A, &7_i32.to_le_bytes());
    memory.put(0x5000 + COUNTRY_OPERAND_3ABE, &(-1_i32).to_le_bytes());
    memory.subunit(0x7000, 777, 1);
    memory.text(0x7000 + subunit::NAME, b"infantry");
    memory.put(0x7000 + subunit::CATEGORY, &4_i32.to_le_bytes());
    memory.subunit(0x7800, 777, 1);
    memory.word(0x9000, subunit::NULL_VTABLE_RVA.0 as u64);
    memory.array(0x9800 + subunit::DATABASE_VECTOR, 0x9900, 2);
    memory.word(0x9900, 0x9000);
    memory.word(0x9908, 0x7000);
    memory
}

#[test]
fn unused_template_retains_historical_naming_group_and_refuses_a_broken_group() {
    let mut memory = fixture();
    let mut rows = vec![IconRow::default(); 1];
    memory.word(
        0x5000 + crate::cname_group_member::TEMPLATE_NAME_GROUP,
        0xa000,
    );
    memory.word(0xa000, crate::cname_group_member::GROUP_VTABLE_RVA.0 as u64);
    memory.text(0xa000 + crate::cname_group_member::GROUP_KEY, b"MOD_NAMES");
    let (walk, bytes, _, _) = icons_all(&memory, &mut rows);
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    let group = rows[0].names_group.unwrap();
    assert_eq!(&bytes[group.start..group.start + group.len], b"MOD_NAMES");
    memory.word(0xa000, 0);
    let (walk, _, _, _) = icons_all(&memory, &mut rows);
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    assert!(!rows[0].details_fresh);
    assert_eq!(
        rows[0].details_failure,
        Some("template naming group identity mismatch")
    );
    assert!(rows[0].model_failure.is_none());
}

#[test]
fn template_extension_fault_preserves_existing_icon_and_model_inputs() {
    struct ExtensionFault(Fixture);
    impl Memory for ExtensionFault {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            address != 0x5000 + GRID_DIMENSIONS && self.0.copy(address, output)
        }
    }
    let mut healthy = vec![IconRow::default(); 1];
    let (expected, _, _, _) = icons_all(&fixture(), &mut healthy);
    let memory = ExtensionFault(fixture());
    let mut rows = vec![IconRow::default(); 1];
    let mut bytes = vec![0; 64];
    let mut counts = vec![SubunitCount::default(); 64];
    let mut variants = vec![0; 8];
    let walked = walk_icons_inner(&memory, 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut rows), engine_native_memory::walk::Buffer::Growing(&mut bytes), engine_native_memory::walk::Buffer::Growing(&mut counts), engine_native_memory::walk::Buffer::Growing(&mut variants), TestSlots::default().slots(), None, None, None, None);
    assert_eq!(walked.outcome, expected.outcome);
    assert!(!rows[0].details_fresh);
    assert_eq!(
        rows[0].details_failure,
        Some("template extension operands unreadable")
    );
    assert_eq!(rows[0].custom_icon, healthy[0].custom_icon);
    assert_eq!(rows[0].representative, healthy[0].representative);
    assert_eq!(rows[0].model_failure, None);
    assert!(rows[0].cells.iter().all(|span| span.len == 0));
}

#[test]
fn template_extension_demand_skips_prefix_tail_forbidden_payload_and_spatial_copy() {
    let mut memory = fixture();
    memory.array(0x5000 + FORBIDDEN_EQUIPMENT, u64::MAX - 8, 1);
    let (mut rows, mut bytes, mut counts, mut variants, mut names) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut database = vec![0; 4];
    let mut globals = vec![GlobalSlot::default(); 8];
    let mut locals = vec![LocalSlot::default(); 8];
    let walked = walk_icons_with_selected_details(
        &memory,
        0,
        0x1000,
        &mut rows,
        &mut bytes,
        &mut counts,
        &mut variants,
        DefinitionSlots {
            database: &mut database,
            globals: &mut globals,
            locals: &mut locals,
        },
        &mut names,
        &mut |_| {},
        &mut |_: u32, _: usize, _: u64, _: u32| false,
    );
    assert_eq!(walked.outcome, Outcome::Rows { len: 1 });
    assert!(!rows[0].details_fresh);
    assert!(rows[0].cells.iter().all(|cells| cells.len == 0));
    assert!(rows[0].equipment_failure.is_none());
    assert!(!memory.1.borrow().iter().any(|&(at, _)|
        at == 0x5000 + GRID_DIMENSIONS || at == 0x5000 + UNIT_TYPE_OVERRIDE + 4));
}

struct TestSlots {
    database: Vec<usize>,
    globals: Vec<GlobalSlot>,
    locals: Vec<LocalSlot>,
}
impl Default for TestSlots {
    fn default() -> Self {
        Self {
            database: vec![0; 8],
            globals: vec![GlobalSlot::default(); 8],
            locals: vec![LocalSlot::default(); 8],
        }
    }
}
impl TestSlots {
    fn slots(&mut self) -> DefinitionSlots<'_> {
        DefinitionSlots {
            database: &mut self.database,
            globals: &mut self.globals,
            locals: &mut self.locals,
        }
    }
}

type IconAll = (IconWalk, Vec<u8>, Vec<SubunitCount>, Vec<u64>);

fn icons_all(memory: &Fixture, rows: &mut Vec<IconRow>) -> IconAll {
    let (mut bytes, mut counts, mut variants) =
        (vec![0; 64], vec![Default::default(); 64], vec![0; 8]);
    let walk = walk_icons_inner(memory, 0, 0x1000, engine_native_memory::walk::Buffer::Growing(rows), engine_native_memory::walk::Buffer::Growing(&mut bytes), engine_native_memory::walk::Buffer::Growing(&mut counts), engine_native_memory::walk::Buffer::Growing(&mut variants), TestSlots::default().slots(), None, None, None, None);
    bytes.truncate(walk.bytes_len);
    counts.truncate(walk.counts_len);
    variants.truncate(walk.variants_len);
    (walk, bytes, counts, variants)
}

fn daily_all(memory: &Fixture, rows: &mut Vec<Row>) -> (Walk, Vec<u8>) {
    let mut bytes = vec![0; 64];
    let walk = walk(memory, 0, 0x1000, rows, &mut bytes, &mut vec![(0, 0); 16]);
    bytes.truncate(walk.bytes_len);
    (walk, bytes)
}

#[test]
fn hourly_rows_keep_the_counter_choice_and_the_stored_model_results() {
    let mut memory = fixture();
    memory.array(0x5000 + SUBUNIT_COUNTS, 0x8000, 2);
    memory.word(0x8000, 0x7000);
    memory.put(0x8008, &2_i32.to_le_bytes());
    memory.word(0x8010, 0x7800);
    memory.put(0x8018, &(-1_i32).to_le_bytes());
    memory.word(0x5000 + VISUAL_REPRESENTATIVE, 0x7800);
    memory.0[0x5000 + MODEL_CATEGORY_PRESENT] = 1;
    memory.word(0x5000 + MODEL_CATEGORY, 1 << 5);
    memory.text(0x5000 + MODEL_ENTITY, b"GER_x_entity");
    memory.array(0x5000 + EXCLUDED_VARIANT_DATA, 0x8100, 1);
    memory.word(0x8100, 0x8200);
    memory.word(0x8200, engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64);
    memory.word(0x8208, 0x0000_0009_0000_0046);
    let mut rows = vec![IconRow::default(); 2];
    let (walk, bytes, counts, variants) = icons_all(&memory, &mut rows);
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    let reads = memory.1.borrow();
    assert_eq!(
        reads
            .iter()
            .filter(|(at, _)| (0x5000..0x5240).contains(at))
            .count(),
        2 + 2 + 1
    );
    assert!(!reads.iter().any(|(at, _)| *at == 0x5000 + NAME));
    for definition in [0x7000, 0x7800] {
        assert_eq!(
            reads
                .iter()
                .filter(|read| **read == (definition, 17))
                .count(),
            1
        );
    }
    assert_eq!((walk.required_database, walk.required_locals), (2, 4));
    drop(reads);
    let row = rows[0];
    assert_eq!(row.template, 0xffff_ffff_0000_0034);
    assert_eq!((row.priority_unit, row.custom_icon), (777, -1));
    assert_eq!((row.country, row.foreign_country), (7, -1));
    assert_eq!(bytes, b"infantryGER_x_entity");
    assert_eq!(row.priority_unit_name, Span { start: 0, len: 8 });
    assert_eq!(row.model_entity, Span { start: 8, len: 12 });
    assert_eq!(row.model_category, Some(1 << 5));
    let local = SubunitCount {
        width: Some(0),
        token: 777,
        local: true,
        count: 0,
    };
    assert_eq!(row.representative, local);
    assert_eq!(
        counts,
        [
            SubunitCount {
                width: Some(0),
                token: 777,
                local: false,
                count: 2
            },
            SubunitCount { count: -1, ..local },
            SubunitCount {
                width: Some(0),
                token: 777,
                local: false,
                count: 1
            },
            SubunitCount {
                width: Some(0),
                token: 777,
                local: false,
                count: 1
            },
            SubunitCount::default(),
        ]
    );
    assert_eq!(row.counts, Span { start: 0, len: 2 });
    assert_eq!(
        row.grids,
        [
            Span { start: 2, len: 1 },
            Span { start: 5, len: 0 },
            Span { start: 5, len: 0 }
        ]
    );
    assert_eq!(variants, [0x0000_0009_0000_0046]);
}

#[test]
fn every_grid_is_read_and_counted_per_cell() {
    let mut memory = fixture();
    memory.array(0x5000 + REGIMENT_DATA, 0x6000, 5);
    for (cell, definition) in [0x7000, 0x7800, 0x7000, 0, 0x9000].into_iter().enumerate() {
        memory.word(0x6000 + cell * 8, definition);
    }
    memory.array(0x5000 + SUPPORT_DATA, 0x6100, 2);
    memory.word(0x6100, 0x7800);
    memory.word(0x6108, 0x7800);
    memory.array(0x5000 + REGIMENTAL_SUPPORT_DATA, 0x6200, 1);
    memory.word(0x6200, 0x7000);
    let mut rows = vec![IconRow::default(); 1];
    let (walk, _, counts, _) = icons_all(&memory, &mut rows);
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    assert_eq!(rows[0].model_failure, None);
    assert_eq!(walk.required_counts, 16);
    let entry = |token, local, count| SubunitCount {
        width: Some(0),
        token,
        local,
        count,
    };
    let grid = |index: usize| {
        let span = rows[0].grids[index];
        counts[span.start..span.start + span.len].to_vec()
    };
    assert_eq!(
        grid(0),
        [
            entry(777, false, 2),
            entry(777, true, 1),
            entry(0, false, 1)
        ]
    );
    assert_eq!(grid(1), [entry(777, true, 2)]);
    assert_eq!(grid(2), [entry(777, false, 1)]);
    assert_eq!(rows[0].counts.len, 0);
    memory.array(0x5000 + SUPPORT_DATA, 0xbff8, 2);
    let (walk, _, counts, _) = icons_all(&memory, &mut rows);
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    assert_eq!(
        rows[0].model_failure,
        Some("template grid cells unreadable")
    );
    assert_eq!(rows[0].priority_unit, 777);
    assert!(counts.is_empty());
}

#[test]
fn daily_rows_keep_names_and_the_category_without_the_model_inputs() {
    let mut memory = fixture();
    memory.array(0x5000 + SUBUNIT_COUNTS, 0x8000, 1);
    memory.word(0x8000, 0x7000);
    let mut rows = vec![Row::default(); 2];
    let (walk, bytes) = daily_all(&memory, &mut rows);
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    assert_eq!(
        (rows[0].template, rows[0].owner),
        (0xffff_ffff_0000_0034, 1)
    );
    assert_eq!(rows[0].unit_type, 1);
    assert_eq!(bytes, b"A\0\xfffallback");
    assert_eq!(rows[0].name_key, Span { start: 3, len: 8 });
    let reads = memory.1.borrow();
    assert_eq!(
        reads
            .iter()
            .filter(|(at, _)| (0x5000..0x5240).contains(at))
            .count(),
        3 + 1
    );
    for address in [0x8000, subunit::DATABASE_RVA.0] {
        assert!(!reads.iter().any(|(at, _)| *at == address));
    }
}

#[test]
fn icon_rows_carry_the_dictionary_unit_type_owner_and_object() {
    let mut memory = fixture();
    let mut daily = vec![Row::default(); 1];
    daily_all(&memory, &mut daily);
    let mut rows = vec![IconRow::default(); 1];
    assert_eq!(
        icons_all(&memory, &mut rows).0.outcome,
        Outcome::Rows { len: 1 }
    );
    assert_eq!(
        (rows[0].unit_type, rows[0].unit_type_failure),
        (daily[0].unit_type, None)
    );
    assert_eq!(rows[0].unit_type, 1);
    assert_eq!((rows[0].owner, rows[0].pointer), (1, 0x5000));
    memory.put(0x5000 + UNIT_TYPE_OVERRIDE, &123_i32.to_le_bytes());
    icons_all(&memory, &mut rows);
    assert_eq!(rows[0].unit_type, 123);
    memory.put(0x5000 + UNIT_TYPE_OVERRIDE, &0_i32.to_le_bytes());
    memory.0.truncate(0x7000 + subunit::CATEGORY);
    memory.0.resize(0x7000 + subunit::CATEGORY, 0);
    let walk = icons_all(&memory, &mut rows).0;
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    assert_eq!(
        rows[0].unit_type_failure,
        Some("subunit category unreadable")
    );
}

#[test]
fn names_copy_as_the_full_walk_copies_them() {
    let memory = fixture();
    let mut bytes = b"xy".to_vec();
    let [name, key] = copy_names(&memory, 0x5000, &mut bytes, 2).unwrap();
    assert_eq!(bytes, b"xyA\0\xfffallback");
    assert_eq!(
        (name, key),
        (Span { start: 2, len: 3 }, Span { start: 5, len: 8 })
    );
    let mut daily = vec![Row::default(); 1];
    let (_, names) = daily_all(&memory, &mut daily);
    assert_eq!(names, bytes[2..]);
}

#[test]
fn the_representative_falls_back_to_the_priority_unit_and_an_absent_category_stays_absent() {
    let memory = fixture();
    let mut rows = vec![IconRow::default(); 1];
    let (walk, bytes, counts, variants) = icons_all(&memory, &mut rows);
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    assert_eq!(
        rows[0].representative,
        SubunitCount {
            width: Some(0),
            token: 777,
            local: false,
            count: 0
        }
    );
    assert_eq!(rows[0].model_category, None);
    assert_eq!(bytes, b"infantry");
    assert_eq!(
        counts,
        [
            SubunitCount {
                width: Some(0),
                token: 777,
                local: false,
                count: 1
            },
            SubunitCount {
                width: Some(0),
                token: 777,
                local: false,
                count: 1
            },
            SubunitCount::default()
        ]
    );
    assert_eq!(rows[0].counts.len, 0);
    assert!(variants.is_empty());
}

#[test]
fn the_count_probe_counts_whole_rows_and_bad_identity_never_becomes_empty() {
    let mut memory = fixture();
    memory.array(0x5000 + SUBUNIT_COUNTS, 0x8000, 1);
    memory.word(0x8000, 0x7000);
    let result = walk_icons_inner(&memory, 0, 0x1000, engine_native_memory::walk::Buffer::Count, engine_native_memory::walk::Buffer::Count, engine_native_memory::walk::Buffer::Count, engine_native_memory::walk::Buffer::Growing(&mut vec![0; 8]), TestSlots::default().slots(), None, None, None, None);
    assert_eq!(
        result.outcome,
        Outcome::Truncated {
            len: 0,
            unread_rows: 1
        }
    );
    assert_eq!((result.bytes_len, result.counts_len), (0, 0));
    assert_eq!((result.required_bytes, result.required_counts), (8, 5));
    let result = count(&memory, 0, 0x1000);
    assert_eq!(
        (result.outcome, result.required_bytes),
        (
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            },
            11
        )
    );
    memory.put(0x7000 + subunit::DATABASE_INDEX, &2_i32.to_le_bytes());
    let mut rows = vec![IconRow::default(); 1];
    let (walk, bytes, counts, _) = icons_all(&memory, &mut rows);
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    assert_eq!(rows[0].priority_unit, 777);
    assert_eq!(bytes, b"infantry");
    assert_eq!(
        rows[0].model_failure,
        Some("subunit database index outside the database")
    );
    assert!(counts.is_empty());
    let mut memory = fixture();
    memory.word(0x5000, 0);
    assert!(matches!(
        icons_all(&memory, &mut vec![IconRow::default(); 1]).0.outcome,
        Outcome::Unavailable { .. }
    ));
    assert!(matches!(
        daily_all(&memory, &mut vec![Row::default(); 1]).0.outcome,
        Outcome::Unavailable { .. }
    ));
    let mut memory = fixture();
    memory.word(0x5000 + PRIORITY_UNIT, 0x7000);
    memory.put(0x5000 + UNIT_TYPE_OVERRIDE, &123_i32.to_le_bytes());
    let mut rows = vec![Row::default(); 1];
    assert_eq!(
        daily_all(&memory, &mut rows).0.outcome,
        Outcome::Rows { len: 1 }
    );
    assert_eq!(rows[0].unit_type, 123);
}

#[test]
fn fallback_preserves_native_real_count_wrap_and_rejects_zero_tokens() {
    for count in [65_535_u32, 65_536, 65_537] {
        let mut memory = fixture();
        memory.0.resize(0x10000 + count as usize * 8, 0);
        memory.array(0x5000 + REGIMENT_DATA, 0x10000, count);
        for index in 0..count as usize {
            memory.word(0x10000 + index * 8, 0x7000);
        }
        let mut rows = vec![IconRow::default(); 1];
        let mut counts = vec![SubunitCount::default(); count as usize * 2];
        let walk = walk_icons_inner(&memory, 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut rows), engine_native_memory::walk::Buffer::Growing(&mut vec![0; 64]), engine_native_memory::walk::Buffer::Growing(&mut counts), engine_native_memory::walk::Buffer::Growing(&mut vec![0; 8]), TestSlots::default().slots(), None, None, None, None);
        assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
        let expected = if count == 65_536 { 0 } else { 777 };
        assert_eq!(rows[0].representative.token, expected);
        assert_eq!(rows[0].priority_unit, expected);
        assert_eq!(
            counts[..rows[0].grids[0].len],
            [SubunitCount {
                width: Some(0),
                token: 777,
                local: false,
                count: count as i32
            }]
        );
    }
    let mut memory = fixture();
    memory.put(0x7000 + subunit::TOKEN, &0_u32.to_le_bytes());
    assert_eq!(
        icons_all(&memory, &mut vec![IconRow::default(); 1]).0.outcome,
        Outcome::Unavailable {
            reason: "real template subunit has unsupported zero token"
        }
    );
}

fn model_fixture() -> Fixture {
    let mut memory = fixture();
    memory.array(0x5000 + SUBUNIT_COUNTS, 0x8000, 2);
    memory.word(0x8000, 0x7000);
    memory.put(0x8008, &2_i32.to_le_bytes());
    memory.word(0x8010, 0x7800);
    memory.put(0x8018, &1_i32.to_le_bytes());
    memory.array(0x5000 + EXCLUDED_VARIANT_DATA, 0x8100, 1);
    memory.word(0x8100, 0x8200);
    memory.word(0x8200, engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64);
    memory.word(0x8208, 0x0000_0009_0000_0046);
    memory
}

fn icons_with(memory: &Fixture, database: usize, locals: usize) -> (IconAll, IconRow) {
    let mut rows = vec![IconRow::default(); 1];
    let (mut bytes, mut counts, mut variants) =
        (vec![0; 64], vec![SubunitCount::default(); 64], vec![0; 8]);
    let walk = walk_icons_inner(memory, 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut rows), engine_native_memory::walk::Buffer::Growing(&mut bytes), engine_native_memory::walk::Buffer::Growing(&mut counts), engine_native_memory::walk::Buffer::Growing(&mut variants), DefinitionSlots {
            database: &mut vec![0; database],
            globals: &mut vec![GlobalSlot::default(); database],
            locals: &mut vec![LocalSlot::default(); locals],
        }, None, None, None, None);
    bytes.truncate(walk.bytes_len);
    counts.truncate(walk.counts_len);
    variants.truncate(walk.variants_len);
    ((walk, bytes, counts, variants), rows[0])
}

#[test]
fn a_database_larger_than_its_slots_is_read_live_and_asks_for_room() {
    let memory = model_fixture();
    let ((copied, copied_bytes, copied_counts, copied_variants), copied_row) =
        icons_with(&memory, 8, 8);
    assert_eq!(copied.outcome, Outcome::Rows { len: 1 });
    assert_eq!(copied_row.model_failure, None);
    for (database, locals) in [(1, 8), (1, 0), (8, 0)] {
        let ((walk, bytes, counts, variants), row) = icons_with(&memory, database, locals);
        assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
        assert_eq!(
            (row, bytes, counts, variants),
            (
                copied_row,
                copied_bytes.clone(),
                copied_counts.clone(),
                copied_variants.clone()
            )
        );
        assert_eq!(walk.required_database, 2);
        assert!(walk.required_locals >= 4);
    }
}

#[test]
fn an_unreadable_database_keeps_every_counter_icon_and_fails_only_the_model() {
    let mut memory = model_fixture();
    memory.array(0x9800 + subunit::DATABASE_VECTOR, 0, 0);
    let ((walk, bytes, counts, variants), row) = icons_with(&memory, 8, 8);
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    assert_eq!((row.priority_unit, row.custom_icon), (777, -1));
    assert_eq!(bytes, b"infantry");
    assert!(row.model_failure.is_some());
    assert_eq!(
        (row.representative, row.model_entity.len, row.counts.len),
        (SubunitCount::default(), 0, 0)
    );
    assert!(counts.is_empty() && variants.is_empty());
}

fn second_template(memory: &mut Fixture) {
    let bytes = memory.0[0x5000..0x5240].to_vec();
    memory.put(0xa000, &bytes);
    memory.word(0xa008, 0xffff_fffe_0000_0034);
    memory.array(0x3000 + ccountry::DIVISION_TEMPLATES, 0x4000, 2);
    memory.word(0x4008, 0xa000);
}

fn reads_of(memory: &Fixture, definition: usize) -> usize {
    memory
        .1
        .borrow()
        .iter()
        .filter(|read| **read == (definition, 17))
        .count()
}

#[test]
fn a_failed_row_rolls_back_and_the_next_row_starts_where_it_began() {
    let mut memory = model_fixture();
    second_template(&mut memory);
    memory.array(0x5000 + EXCLUDED_VARIANT_DATA, 0x8300, 1);
    memory.word(0x8300, 0x7000);
    let mut rows = vec![IconRow::default(); 2];
    let (mut bytes, mut counts, mut variants) =
        (vec![0; 64], vec![SubunitCount::default(); 64], vec![0; 8]);
    let walk = walk_icons_inner(&memory, 0, 0x1000, engine_native_memory::walk::Buffer::Growing(&mut rows), engine_native_memory::walk::Buffer::Growing(&mut bytes), engine_native_memory::walk::Buffer::Growing(&mut counts), engine_native_memory::walk::Buffer::Growing(&mut variants), TestSlots::default().slots(), None, None, None, None);
    assert_eq!(walk.outcome, Outcome::Rows { len: 2 });
    let failed = rows[0];
    assert!(failed.model_failure.is_some());
    assert_eq!((failed.priority_unit, failed.country), (777, 7));
    assert_eq!(failed.priority_unit_name, Span { start: 0, len: 8 });
    assert_eq!(failed.model_entity, Span { start: 8, len: 0 });
    assert_eq!(failed.counts, Span { start: 0, len: 0 });
    assert_eq!(failed.excluded_variants, Span { start: 0, len: 0 });
    let good = rows[1];
    assert_eq!(good.model_failure, None);
    assert_eq!(good.priority_unit_name, Span { start: 8, len: 8 });
    assert_eq!(good.counts, Span { start: 0, len: 2 });
    assert_eq!(good.excluded_variants, Span { start: 0, len: 1 });
    assert_eq!(&bytes[..walk.bytes_len], b"infantryinfantry");
    assert_eq!(good.grids[0], Span { start: 2, len: 1 });
    assert_eq!((walk.counts_len, walk.variants_len), (5, 1));
    assert_eq!(
        (reads_of(&memory, 0x7000), reads_of(&memory, 0x7800)),
        (1, 1)
    );
}

#[test]
fn a_row_without_room_still_asks_for_its_model_inputs() {
    let memory = model_fixture();
    let walk = walk_icons_inner(&memory, 0, 0x1000, engine_native_memory::walk::Buffer::Count, engine_native_memory::walk::Buffer::Growing(&mut vec![0; 64]), engine_native_memory::walk::Buffer::Growing(&mut vec![SubunitCount::default(); 8]), engine_native_memory::walk::Buffer::Growing(&mut vec![0; 8]), TestSlots::default().slots(), None, None, None, None);
    assert_eq!(
        walk.outcome,
        Outcome::Truncated {
            len: 0,
            unread_rows: 1
        }
    );
    assert_eq!(
        (
            walk.required_bytes,
            walk.required_counts,
            walk.required_variants
        ),
        (8, 2 + 4, 1)
    );
}

#[test]
fn a_live_database_with_an_unreadable_entry_fails_only_the_model() {
    let mut memory = model_fixture();
    memory.array(0x9800 + subunit::DATABASE_VECTOR, 0xbff8, 2);
    for database in [1, 8] {
        memory.1.borrow_mut().clear();
        let ((walk, bytes, counts, variants), row) = icons_with(&memory, database, 8);
        assert_eq!(walk.outcome, Outcome::Rows { len: 1 }, "{database}");
        assert_eq!(row.priority_unit, 777);
        assert_eq!(bytes, b"infantry");
        assert_eq!(
            row.model_failure,
            Some("subunit database index outside the database")
        );
        assert!(counts.is_empty() && variants.is_empty());
        assert_eq!(reads_of(&memory, 0x7000), 1, "{database}");
    }
}

#[test]
fn a_bad_exclusion_or_model_string_fails_only_that_row_s_model() {
    for fault in 0..2 {
        let mut memory = model_fixture();
        memory.put(0x5000 + FORMATION_DIVISION_COUNT, &i32::MIN.to_le_bytes());
        memory.put(0x5000 + ORIGIN_TYPE, &2_i32.to_le_bytes());
        match fault {
            0 => memory.word(0x8100, 0x7000),
            _ => {
                memory.word(0x5000 + MODEL_ENTITY, 0xdead_0000);
                memory.word(0x5000 + MODEL_ENTITY + 16, 40);
                memory.word(0x5000 + MODEL_ENTITY + 24, 47);
            }
        }
        let ((walk, bytes, counts, variants), row) = icons_with(&memory, 8, 8);
        assert_eq!(walk.outcome, Outcome::Rows { len: 1 }, "fault {fault}");
        assert!(row.model_failure.is_some(), "fault {fault}");
        assert_eq!(row.priority_unit, 777);
        assert_eq!(row.formation_division_count, Some(i32::MIN));
        assert!(row.manpower_exile);
        assert_eq!(bytes, b"infantry");
        assert!(counts.is_empty() && variants.is_empty());
    }
}

#[test]
fn formation_count_reuses_the_authenticated_hourly_span_and_keeps_signed_words() {
    let mut memory = fixture();
    let mut rows = vec![IconRow::default(); 1];
    for count in [i32::MIN, -7, -1, 0, 1, i32::MAX] {
        memory.put(0x5000 + FORMATION_DIVISION_COUNT, &count.to_le_bytes());
        memory.1.borrow_mut().clear();
        assert_eq!(
            icons_all(&memory, &mut rows).0.outcome,
            Outcome::Rows { len: 1 }
        );
        assert_eq!(rows[0].formation_division_count, Some(count));
        let reads = memory.1.borrow();
        assert!(
            !reads
                .iter()
                .any(|(at, _)| *at == 0x5000 + FORMATION_DIVISION_COUNT)
        );
        assert!(reads.iter().any(|&(at, len)| {
            at <= 0x5000 + FORMATION_DIVISION_COUNT
                && at + len >= 0x5000 + FORMATION_DIVISION_COUNT + 4
        }));
    }
}

#[test]
fn icon_rows_do_not_read_the_template_name_and_gather_every_grid_header() {
    let memory = fixture();
    let mut rows = vec![IconRow::default(); 2];
    let (walk, bytes, _, _) = icons_all(&memory, &mut rows);
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    assert_eq!(bytes, b"infantry");
    let reads = memory.1.borrow().clone();
    assert!(!reads.iter().any(|(at, _)| *at == 0x5000 + NAME));
    for grid in GRIDS {
        assert!(
            !reads
                .iter()
                .any(|(at, _)| *at == 0x5000 + grid && grid != REGIMENT_DATA)
        );
        assert!(
            reads
                .iter()
                .any(|&(at, len)| { at <= 0x5000 + grid && at + len >= 0x5000 + grid + 16 })
        );
    }
}

#[test]
fn full_handle_survives_and_a_wrong_type_word_fails() {
    let mut bytes = vec![0; 16];
    bytes[..8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
    bytes[8..].copy_from_slice(&0xffff_ffff_0000_0034_u64.to_le_bytes());
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    assert_eq!(identity(&memory, 0, 0x1000), Ok(0xffff_ffff_0000_0034));
    bytes[8] = 0x33;
    assert_eq!(
        identity(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes
            },
            0,
            0x1000
        ),
        Err("division template vtable or reference type mismatch")
    );
    assert_eq!(
        identity(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes[..8]
            },
            0,
            0x1000
        ),
        Err("division template identity unreadable")
    );
}

#[test]
fn a_walk_reads_a_shared_priority_name_once_and_gathers_literal_headers() {
    let mut memory = fixture();
    second_template(&mut memory);
    let (mut rows, mut bytes, mut counts, mut variants, mut names) = (
        vec![IconRow::default(); 2],
        vec![0; 64],
        vec![Default::default(); 64],
        vec![0; 8],
        vec![0; 64],
    );
    let walk = walk_icons_with_names(
        &memory,
        0,
        0x1000,
        &mut rows,
        &mut bytes,
        &mut counts,
        &mut variants,
        TestSlots::default().slots(),
        &mut names,
    );
    assert_eq!(walk.outcome, Outcome::Rows { len: 2 });
    assert_eq!(&bytes[..walk.bytes_len], b"infantryinfantry");
    for row in rows {
        let literal = row.display_name.unwrap().literal;
        assert_eq!(
            names[literal.start..literal.start + literal.len],
            [b'A', 0, 255]
        );
    }
    let reads = memory.1.borrow();
    let at = |address: usize| reads.iter().filter(|read| read.0 == address).count();
    assert_eq!(at(0x7000 + subunit::NAME), 1);
    for template in [0x5000, 0xa000] {
        assert_eq!(at(template + NAME), 1);
        let span = reads
            .iter()
            .position(|read| {
                *read
                    == (
                        template + REGIMENT_DATA,
                        UNIT_TYPE_OVERRIDE + 4 - REGIMENT_DATA,
                    )
            })
            .unwrap();
        assert_eq!(reads[span + 1], (template + NAME, msvc_string::STRIDE));
    }
}

#[test]
fn manpower_exile_predicate_reuses_the_icon_record_and_survives_model_refusal() {
    for origin in [i32::MIN, -1, 0, 1, 2, 3, i32::MAX] {
        let mut memory = fixture();
        memory.put(0x5000 + ORIGIN_TYPE, &origin.to_le_bytes());
        memory.1.borrow_mut().clear();
        let mut rows = vec![IconRow::default(); 1];
        assert_eq!(
            icons_all(&memory, &mut rows).0.outcome,
            Outcome::Rows { len: 1 }
        );
        assert_eq!(rows[0].manpower_exile, origin == 2);
        let reads = memory.1.borrow();
        assert!(!reads.iter().any(|(at, _)| *at == 0x5000 + ORIGIN_TYPE));
        assert!(
            reads.iter().any(
                |&(at, len)| at <= 0x5000 + ORIGIN_TYPE && at + len >= 0x5000 + ORIGIN_TYPE + 4
            )
        );
    }
}

#[test]
fn spatial_cells_keep_holes_dimensions_null_objects_and_native_settings() {
    let mut memory = fixture();
    memory.put(0x5000 + GRID_DIMENSIONS, &(2_u32 | (3 << 16)).to_le_bytes());
    memory.array(0x5000 + REGIMENT_DATA, 0x6000, 6);
    for (index, definition) in [0x7000, 0, 0x7800, 0x9000, 0, 0x7000]
        .into_iter()
        .enumerate()
    {
        memory.word(0x6000 + index * 8, definition);
    }
    memory.put(0x5000 + PRIORITY, &(-9_i32).to_le_bytes());
    memory.0[0x5000 + IS_ARMY_HQ] = 7;
    memory.0[0x5000 + LOCKED] = 2;
    memory.0[0x5000 + OBSOLETE_FLAGS] = 4;
    memory.0[0x5000 + FORCE_ALLOW_RECRUITING] = 3;
    memory.0[0x5000 + EQUIPMENT_NICHE] = 254;
    memory.0[0x5000 + ROLE_PRESENT] = 2;
    memory.put(0x5000 + ROLE, &4294967294_u32.to_le_bytes());
    memory.put(0x5000 + AI_ROLE, &11_u32.to_le_bytes());
    memory.put(0x5000 + AI_ROLE_FALLBACK, &u32::MAX.to_le_bytes());
    memory.put(0x5000 + TOTAL_MANPOWER, &(-6_000_i32).to_le_bytes());
    memory.put(0x5000 + COMMAND_POWER_COST, &(-1234567_i64).to_le_bytes());
    memory.put(0x5000 + TRAINING_TIME, &(-44_i32).to_le_bytes());
    memory.0[0x5000 + ALLOW_NEW_EQUIPMENT] = 2;
    memory.0[0x5000 + ALLOW_FOREIGN_EQUIPMENT] = 5;
    memory.word(0x5000 + PARENT, 0xfeed_beef_0000_0034);
    memory.0[0x5000 + DIVISION_CAP_PRESENT] = 1;
    memory.put(0x5000 + DIVISION_CAP, &(-11_i32).to_le_bytes());
    memory.array(0x5000 + FORBIDDEN_EQUIPMENT, 0x8100, 2);
    memory.word(0x8100, 0x8200);
    memory.word(0x8108, 0x8200);
    memory.word(0x8200, engine_native_parts::cequipment_type::VTABLE_RVA.0 as u64);
    memory.put(0x8208, &4294967294_u32.to_le_bytes());
    let mut rows = vec![IconRow::default(); 1];
    let (walk, _, counts, variants) = icons_all(&memory, &mut rows);
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    let row = rows[0];
    assert_eq!(row.model_failure, None);
    assert_eq!(row.grid_dimensions[0], 2 | (3 << 16));
    let cells = &counts[row.cells[0].start..row.cells[0].start + row.cells[0].len];
    assert_eq!(
        cells
            .iter()
            .map(|cell| (cell.token, cell.local, cell.count))
            .collect::<Vec<_>>(),
        [
            (777, false, 1),
            (0, false, 0),
            (777, true, 1),
            (0, false, 1),
            (0, false, 0),
            (777, false, 1)
        ]
    );
    assert_eq!(row.settings.priority, -9);
    assert_eq!(
        (
            row.settings.is_army_hq,
            row.settings.locked,
            row.settings.obsolete
        ),
        (7, 2, 4)
    );
    assert_eq!(row.settings.force_allow_recruiting, 3);
    assert_eq!(row.settings.equipment_niche, 254);
    assert_eq!(row.settings.role, Some(4294967294));
    assert_eq!(row.settings.ai_role, None);
    assert_eq!(row.settings.ai_role_fallback, u32::MAX);
    assert_eq!(row.settings.total_manpower, -6_000);
    assert_eq!(row.settings.command_power_cost, -1234567);
    assert_eq!(row.settings.training_time, -44);
    assert_eq!(
        (
            row.settings.allow_new_equipment,
            row.settings.allow_foreign_equipment
        ),
        (2, 5)
    );
    assert_eq!(row.settings.parent, 0xfeed_beef_0000_0034);
    assert_eq!(row.settings.division_cap, Some(-11));
    assert_eq!(variants, [4294967294, 4294967294]);
    assert_eq!(
        memory
            .1
            .borrow()
            .iter()
            .filter(|&&(at, len)| at == 0x6000 && len == 48)
            .count(),
        1
    );
    memory.array(0x5000 + FORBIDDEN_EQUIPMENT, 0xbff8, 2);
    let (walk, _, _, _) = icons_all(&memory, &mut rows);
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    assert!(rows[0].equipment_failure.is_some());
    assert_eq!(rows[0].model_failure, None);
    assert_eq!(rows[0].settings, row.settings);
    assert_eq!(rows[0].cells, row.cells);
}
