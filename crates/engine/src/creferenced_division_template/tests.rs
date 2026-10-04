use super::*;
use crate::memory::SliceMemory;

struct Fixture(Vec<u8>, std::cell::RefCell<Vec<(usize, usize)>>);
impl Memory for Fixture {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.1.borrow_mut().push((address, output.len()));
        let redirect = match address {
            subunit::NULL_OBJECT_RVA => Some(0x9000_u64),
            subunit::DATABASE_RVA => Some(0x9800),
            _ => None,
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
        self.word(at, subunit::VTABLE_RVA as u64);
        self.put(at + subunit::TOKEN, &token.to_le_bytes());
        self.0[at + subunit::IS_REAL] = 1;
        self.put(at + subunit::DATABASE_INDEX, &index.to_le_bytes());
    }
}
/// One country, one template. Global infantry (777) is database entry 1 at
/// 0x7000; a country-local clone of it sits at 0x7800.
fn fixture() -> Fixture {
    let mut memory = Fixture(vec![0; 0xc000], std::cell::RefCell::default());
    memory.array(0x1000 + crate::cgame_state::COUNTRIES, 0x2000, 2);
    memory.array(0x1000 + crate::cgame_state::COUNTRY_LINKS, 0x2100, 2);
    memory.word(0x2008, 0x3000);
    memory.put(0x2104, &1_u32.to_le_bytes());
    memory.word(0x3000, ccountry::VTABLE_RVA as u64);
    memory.put(0x3008, &1_u32.to_le_bytes());
    memory.array(0x3000 + ccountry::DIVISION_TEMPLATES, 0x4000, 1);
    memory.word(0x4000, 0x5000);
    memory.word(0x5000, VTABLE_RVA as u64);
    memory.word(0x5008, 0xffff_ffff_0000_0034);
    memory.text(0x5000 + NAME, &[b'A', 0, 255]);
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
    memory.word(0x9000, subunit::NULL_VTABLE_RVA as u64);
    memory.array(0x9800 + subunit::DATABASE_VECTOR, 0x9900, 2);
    memory.word(0x9900, 0x9000);
    memory.word(0x9908, 0x7000);
    memory
}

/// Owned storage for [`walk_icons`]'s definition lookups.
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

fn icons_all(memory: &Fixture, rows: &mut [IconRow]) -> IconAll {
    let (mut bytes, mut counts, mut variants) =
        (vec![0; 64], vec![Default::default(); 8], vec![0; 8]);
    let walk = walk_icons(
        memory,
        0,
        0x1000,
        rows,
        &mut bytes,
        &mut counts,
        &mut variants,
        TestSlots::default().slots(),
    );
    bytes.truncate(walk.bytes_len);
    counts.truncate(walk.counts_len);
    variants.truncate(walk.variants_len);
    (walk, bytes, counts, variants)
}

fn daily_all(memory: &Fixture, rows: &mut [Row]) -> (Walk, Vec<u8>) {
    let mut bytes = vec![0; 64];
    let walk = walk(memory, 0, 0x1000, rows, &mut bytes, &mut [(0, 0); 16]);
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
    memory.word(0x8200, crate::cequipment_variant::VTABLE_RVA as u64);
    memory.word(0x8208, 0x0000_0009_0000_0046);
    let mut rows = [IconRow::default(); 2];
    let (walk, bytes, counts, variants) = icons_all(&memory, &mut rows);
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    // The fixed fields come in two spans, and each definition is
    // authenticated once although the fallback, priority and grouped
    // entries all name it. The name is not read.
    let reads = memory.1.borrow();
    assert_eq!(
        reads
            .iter()
            .filter(|(at, _)| (0x5000..0x5240).contains(at))
            .count(),
        2 + 1 // Plus the fallback's regiment grid header.
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
    // The country names two definitions: four slots at half load.
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
        token: 777,
        local: true,
        count: 0,
    };
    assert_eq!(row.representative, local);
    assert_eq!(
        counts,
        [
            SubunitCount {
                token: 777,
                local: false,
                count: 2
            },
            SubunitCount { count: -1, ..local },
        ]
    );
    assert_eq!(variants, [0x0000_0009_0000_0046]);
}

#[test]
fn daily_rows_keep_names_and_the_category_without_the_model_inputs() {
    let mut memory = fixture();
    memory.array(0x5000 + SUBUNIT_COUNTS, 0x8000, 1);
    memory.word(0x8000, 0x7000);
    let mut rows = [Row::default(); 2];
    let (walk, bytes) = daily_all(&memory, &mut rows);
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    assert_eq!(
        (rows[0].template, rows[0].owner),
        (0xffff_ffff_0000_0034, 1)
    );
    assert_eq!(rows[0].unit_type, 1);
    assert_eq!(bytes, b"A\0\xff");
    let reads = memory.1.borrow();
    assert_eq!(
        reads
            .iter()
            .filter(|(at, _)| (0x5000..0x5240).contains(at))
            .count(),
        3 + 1 // Plus the category fallback's regiment grid header.
    );
    for address in [0x8000, subunit::DATABASE_RVA] {
        assert!(!reads.iter().any(|(at, _)| *at == address));
    }
}

#[test]
fn the_representative_falls_back_to_the_priority_unit_and_an_absent_category_stays_absent() {
    let memory = fixture();
    let mut rows = [IconRow::default(); 1];
    let (walk, bytes, counts, variants) = icons_all(&memory, &mut rows);
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    assert_eq!(
        rows[0].representative,
        SubunitCount {
            token: 777,
            local: false,
            count: 0
        }
    );
    assert_eq!(rows[0].model_category, None);
    assert_eq!(bytes, b"infantry");
    assert!(counts.is_empty() && variants.is_empty());
}

#[test]
fn capacity_omits_whole_rows_and_bad_identity_never_becomes_empty() {
    let mut memory = fixture();
    memory.array(0x5000 + SUBUNIT_COUNTS, 0x8000, 1);
    memory.word(0x8000, 0x7000);
    for (rows, text, counts) in [(0, 64, 8), (2, 2, 8), (2, 64, 0)] {
        let result = walk_icons(
            &memory,
            0,
            0x1000,
            &mut vec![IconRow::default(); rows],
            &mut vec![0; text],
            &mut vec![SubunitCount::default(); counts],
            &mut [0; 8],
            TestSlots::default().slots(),
        );
        assert_eq!(
            result.outcome,
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
        assert_eq!((result.bytes_len, result.counts_len), (0, 0));
        assert_eq!((result.required_bytes, result.required_counts), (8, 1));
    }
    let result = walk(
        &memory,
        0,
        0x1000,
        &mut [Row::default(); 1],
        &mut [0; 2],
        &mut [],
    );
    assert_eq!(
        (result.outcome, result.required_bytes),
        (
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            },
            3
        )
    );
    // An index naming no database entry is a fault, not a global
    // definition: the model fields fail, and the counter keeps its icon.
    memory.put(0x7000 + subunit::DATABASE_INDEX, &2_i32.to_le_bytes());
    let mut rows = [IconRow::default(); 1];
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
        icons_all(&memory, &mut [IconRow::default(); 1]).0.outcome,
        Outcome::Unavailable { .. }
    ));
    assert!(matches!(
        daily_all(&memory, &mut [Row::default(); 1]).0.outcome,
        Outcome::Unavailable { .. }
    ));
    let mut memory = fixture();
    memory.word(0x5000 + PRIORITY_UNIT, 0x7000);
    memory.put(0x5000 + UNIT_TYPE_OVERRIDE, &123_i32.to_le_bytes());
    let mut rows = [Row::default(); 1];
    assert_eq!(
        daily_all(&memory, &mut rows).0.outcome,
        Outcome::Rows { len: 1 }
    );
    assert_eq!(rows[0].unit_type, 123); // Never clamp a native override to strip size.
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
        let mut rows = [IconRow::default(); 1];
        assert_eq!(
            icons_all(&memory, &mut rows).0.outcome,
            Outcome::Rows { len: 1 }
        );
        let expected = if count == 65_536 { 0 } else { 777 };
        assert_eq!(rows[0].representative.token, expected);
        assert_eq!(rows[0].priority_unit, expected);
    }
    let mut memory = fixture();
    memory.put(0x7000 + subunit::TOKEN, &0_u32.to_le_bytes());
    assert_eq!(
        icons_all(&memory, &mut [IconRow::default(); 1]).0.outcome,
        Outcome::Unavailable {
            reason: "real template subunit has unsupported zero token"
        }
    );
}

/// The fixture with a clone and a global in the grouped entries and one
/// excluded variant, read with the given database slots.
fn model_fixture() -> Fixture {
    let mut memory = fixture();
    memory.array(0x5000 + SUBUNIT_COUNTS, 0x8000, 2);
    memory.word(0x8000, 0x7000);
    memory.put(0x8008, &2_i32.to_le_bytes());
    memory.word(0x8010, 0x7800);
    memory.put(0x8018, &1_i32.to_le_bytes());
    memory.array(0x5000 + EXCLUDED_VARIANT_DATA, 0x8100, 1);
    memory.word(0x8100, 0x8200);
    memory.word(0x8200, crate::cequipment_variant::VTABLE_RVA as u64);
    memory.word(0x8208, 0x0000_0009_0000_0046);
    memory
}

fn icons_with(memory: &Fixture, database: usize, locals: usize) -> (IconAll, IconRow) {
    let mut rows = [IconRow::default(); 1];
    let (mut bytes, mut counts, mut variants) =
        (vec![0; 64], vec![SubunitCount::default(); 8], vec![0; 8]);
    let walk = walk_icons(
        memory,
        0,
        0x1000,
        &mut rows,
        &mut bytes,
        &mut counts,
        &mut variants,
        DefinitionSlots {
            database: &mut vec![0; database],
            globals: &mut vec![GlobalSlot::default(); database],
            locals: &mut vec![LocalSlot::default(); locals],
        },
    );
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
    // Too small, or no per-country slots at all: the same rows, exactly,
    // and the demand to grow.
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
    // The vector's header no longer names a database.
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

/// A second template in the same roster, a copy of the first with its own
/// handle, at 0xA000.
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
    // The first template's exclusion names no variant.
    memory.array(0x5000 + EXCLUDED_VARIANT_DATA, 0x8300, 1);
    memory.word(0x8300, 0x7000);
    let mut rows = [IconRow::default(); 2];
    let (mut bytes, mut counts, mut variants) =
        (vec![0; 64], vec![SubunitCount::default(); 8], vec![0; 8]);
    let walk = walk_icons(
        &memory,
        0,
        0x1000,
        &mut rows,
        &mut bytes,
        &mut counts,
        &mut variants,
        TestSlots::default().slots(),
    );
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
    assert_eq!((walk.counts_len, walk.variants_len), (2, 1));
    // Every definition is read once, however many templates name it.
    assert_eq!(
        (reads_of(&memory, 0x7000), reads_of(&memory, 0x7800)),
        (1, 1)
    );
}

#[test]
fn a_row_without_room_still_asks_for_its_model_inputs() {
    let memory = model_fixture();
    let walk = walk_icons(
        &memory,
        0,
        0x1000,
        &mut [],
        &mut [0; 64],
        &mut [SubunitCount::default(); 8],
        &mut [0; 8],
        TestSlots::default().slots(),
    );
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
        (8, 2, 1)
    );
}

#[test]
fn a_live_database_with_an_unreadable_entry_fails_only_the_model() {
    let mut memory = model_fixture();
    // Entry one lies past the end of readable memory.
    memory.array(0x9800 + subunit::DATABASE_VECTOR, 0xbff8, 2);
    // Slots too small (live), and slots that fit but whose copy faults.
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
        // The failed clone flag is kept, not read again.
        assert_eq!(reads_of(&memory, 0x7000), 1, "{database}");
    }
}

#[test]
fn a_bad_exclusion_or_model_string_fails_only_that_row_s_model() {
    for fault in 0..2 {
        let mut memory = model_fixture();
        match fault {
            // An excluded pointer that names no variant.
            0 => memory.word(0x8100, 0x7000),
            // A heap model string whose bytes are not readable.
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
        // The partial model reads are rolled back.
        assert_eq!(bytes, b"infantry");
        assert!(counts.is_empty() && variants.is_empty());
    }
}

#[test]
fn icon_rows_do_not_read_the_template_name_or_support_grids() {
    let memory = fixture();
    let mut rows = [IconRow::default(); 2];
    let (walk, bytes, _, _) = icons_all(&memory, &mut rows);
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    assert_eq!(bytes, b"infantry");
    let reads = memory.1.borrow().clone();
    for address in [
        0x5000 + NAME,
        0x5000 + SUPPORT_DATA,
        0x5000 + REGIMENTAL_SUPPORT_DATA,
    ] {
        assert!(!reads.iter().any(|(at, _)| *at == address));
    }
}

#[test]
fn full_handle_survives_and_a_wrong_type_word_fails() {
    let mut bytes = vec![0; 16];
    bytes[..8].copy_from_slice(&(VTABLE_RVA as u64).to_le_bytes());
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
