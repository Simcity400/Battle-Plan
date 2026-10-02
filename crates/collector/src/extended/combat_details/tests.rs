use super::*;

#[test]
fn modifier_capacity_and_division_growth_keep_independent_failures() {
    let mut image = vec![0_u8; 0x4000];
    for (address, value) in [
        (0x1000, engine::carmy::VTABLE_RVA as u64),
        (0x1018, 0x1234_0000_0033),
        (0x1170, 0x3000),
        (0x1178, 2_u64 << 32 | 2),
        (0x3000, 27),
        (0x3008, (-900_i64) as u64),
        (0x3010, 800),
        (0x3018, 2),
    ] {
        image[address..address + 8].copy_from_slice(&value.to_le_bytes());
    }
    let memory = engine::memory::SliceMemory {
        base: 0,
        bytes: &image,
    };
    let mut sides = [
        ModifierSide {
            combat: 62,
            ..Default::default()
        },
        ModifierSide {
            combat: 62,
            side: 1,
            ..Default::default()
        },
    ];
    let mut members = [ModifierMember::default(); 2];
    let mut values = [carmy_combat_inputs::Modifier::default(); 1];
    let mut test_sources = Sources::new();
    let mut images = Images::default();
    let mut scratch = Scratch::default();
    scratch.ensure();
    let mut commander_bytes = [0_u8; 16];
    let mut grown_commander_bytes = vec![0_u8; 300];
    let mut destination = Destination {
        sources: &mut test_sources,
        commanders: Default::default(),
        images: &mut images,
        scratch: &mut scratch,
        name_groups: &mut engine::cname_group_member::GroupKeys::default(),
        names: engine::cname_group_member::Names {
            bytes: &mut [],
            len: 0,
            required: 0,
        },
        reference: false,
        details: &mut [],
        division_commanders: &mut Vec::new(),
        commander_names: engine::cname_group_member::Names {
            bytes: &mut commander_bytes,
            len: 0,
            required: 0,
        },
        commander_keys: &mut [],
        commander_traits: &mut [],
        detail_text: &mut [],
        timed_traits: &mut [],
        air_counts: &mut [],
        learning_traits: &mut [],
        unit_scratch: &mut Vec::new(),
        detail_used: cland_combat::DetailCounts::default(),
        detail_required: cland_combat::DetailCounts::default(),
        commander_progress: Progress::default(),
        commander_key_count: 0,
        required_commander_keys: 0,
        divisions: &mut Vec::new(),
        sides: &mut sides,
        combat_sides: &mut [],
        country_tags: &mut [],
        country_tags_progress: Progress::default(),
        country_set_limit: Ok(1),
        rosters: &mut [],
        members: &mut members,
        modifiers: &mut values,
        width_terms: &mut Vec::new(),
        width_progress: Progress::default(),
        progress: [Progress::default(); N],
        arenas: [Progress::default(); N],
        exile_tags: &mut [],
        exile_progress: Progress::default(),
        reinforcement_modifiers: &mut Vec::new(),
        supply_terms: &mut Vec::new(),
        acclimatization: &mut Vec::new(),
        transport_terms: &mut Vec::new(),
        reinforcement_progress: [Progress::default(); 4],
        ticks: [0; N],
    };
    destination.progress[2] = Progress {
        len: 2,
        required: 2,
        failure: None,
    };
    destination.modifier_member(&memory, 0, 0x1000, 0x1234_0000_0033, 0, Some(0));
    assert_eq!(destination.arenas[1].required, 1);
    assert_eq!(destination.arenas[2].required, 2);
    assert_eq!(destination.arenas[2].len, 0);
    destination.finish();
    assert_eq!(
        destination.progress[2].outcome(Outcome::Rows { len: 1 }),
        Outcome::Truncated {
            len: 1,
            unread_rows: 1
        }
    );
    assert_eq!(destination.sides[0].side, 1);
    assert_eq!(destination.sides[0].len, 0);

    // The empty division destination grows from the original source
    // count; native failures still fail both consumers and do not prevent
    // the independent modifiers census below.
    destination.progress = [Progress::default(); N];
    destination.visit(
        &memory,
        0,
        0,
        cland_combat::Source {
            pointer: 0,
            id: 62,
            attackers: engine::cpdx_array::Array { data: 0, count: 1 },
            defenders: engine::cpdx_array::Array { data: 0, count: 0 },
            provinces: engine::cpdx_array::Array { data: 0, count: 0 },
        },
        cland_combat::Members::none(),
    );
    assert_eq!(destination.divisions.len(), 1);
    for progress in &destination.progress[..2] {
        assert!(matches!(
            progress.outcome(Outcome::Rows { len: 1 }),
            Outcome::Unavailable { .. }
        ));
    }
    assert_eq!(destination.progress[2].required, 2);

    // A commander that did not fit the old row arena must still be read
    // exactly once and retained; compare every guarded-read failure prefix
    // with the native reader called directly.
    use engine::{creference_registry as registry, cunit_leader as leader};
    use std::cell::{Cell, RefCell};
    struct Traced<'a> {
        bytes: &'a [u8],
        reads: RefCell<Vec<(usize, usize)>>,
        fail: Cell<Option<usize>>,
    }
    impl Memory for Traced<'_> {
        fn copy(&self, address: usize, out: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, out.len()));
            if self.fail.get() == Some(address) {
                return false;
            }
            if address == registry::HIGH_ROOT_RVA && out.len() == 8 {
                out.copy_from_slice(&0x1000_u64.to_le_bytes());
                return true;
            }
            engine::memory::SliceMemory {
                base: 0,
                bytes: self.bytes,
            }
            .copy(address, out)
        }
    }
    let handle = (0xf000_0001_u64 << 32) | leader::REFERENCE_TYPE as u64;
    let hash = handle
        .rotate_left(32)
        .wrapping_mul(registry::HASH_MULTIPLIER);
    let slot = (((hash >> 32) as u32 ^ hash as u32) & 7) as usize;
    let entry = 0x1100 + slot * 24;
    let mut image = vec![0_u8; 0x5000];
    for (address, value) in [
        (0x1000 + registry::ENTRIES, 0x1100),
        (0x1000 + registry::COUNT, 7_u64 << 32 | 1),
        (entry + 8, handle),
        (entry + 16, 0x2000),
        (0x2000, engine::carmy_leader::VTABLE_RVA as u64),
        (0x2000 + leader::TYPE, handle),
        (0x2000 + leader::SKILL, 0x4000),
        (0x2000 + leader::TOKEN + 16, 2),
        (0x2000 + leader::TOKEN + 24, 15),
        (0x4000, engine::cunit_leader_skill::VTABLE_RVA as u64),
    ] {
        image[address..address + 8].copy_from_slice(&value.to_le_bytes());
    }
    image[0x1000 + registry::OVERFLOW_DISTANCE] = 5;
    image[0x1100 + 13 * 24 + 4] = 255;
    image[entry + 4] = 1;
    image[0x2000 + leader::TOKEN..0x2000 + leader::TOKEN + 2].copy_from_slice(&[b'n', 0xff]);
    let skill = 0x4000 + engine::cunit_leader_skill::LEVEL;
    image[skill..skill + 4].copy_from_slice(&i32::MIN.to_le_bytes());
    let memory = Traced {
        bytes: &image,
        reads: RefCell::default(),
        fail: Cell::new(None),
    };
    let mut direct_bytes = [0_u8; 16];
    let mut direct = |memory: &Traced<'_>| {
        engine::naval_commander_inputs::capture_division_commander(
            memory,
            0,
            handle,
            &mut engine::cname_group_member::Names {
                bytes: &mut direct_bytes,
                len: 0,
                required: 0,
            },
        )
    };
    let expected = direct(&memory).unwrap().unwrap();
    let complete_reads = memory.reads.take();
    let mut keys = [CommanderKey::default(); 1];
    destination.commander_keys = &mut keys;
    destination.progress = [Progress::default(); N];
    assert_eq!(destination.commander(&memory, 0, handle), Some(0));
    assert_eq!(destination.division_commanders[0], expected);
    assert_eq!(*memory.reads.borrow(), complete_reads);
    assert_eq!(destination.commander(&memory, 0, handle), Some(0));
    assert_eq!(*memory.reads.borrow(), complete_reads);
    for (address, _) in complete_reads {
        memory.fail.set(Some(address));
        memory.reads.borrow_mut().clear();
        let expected = direct(&memory).unwrap_err();
        let expected_reads = memory.reads.take();
        destination.commander_keys.fill(CommanderKey::default());
        destination.commander_progress = Progress::default();
        destination.commander_key_count = 0;
        destination.commander_names.len = 0;
        destination.division_commanders.clear();
        destination.progress = [Progress::default(); N];
        assert_eq!(destination.commander(&memory, 0, handle), None);
        assert_eq!(destination.progress[1].failure, Some(expected));
        assert_eq!(*memory.reads.borrow(), expected_reads);
        assert!(destination.division_commanders.is_empty());
    }

    // A token longer than any fixed buffer, held on the heap: it is
    // counted for growth once (the unfit key is memoized), its divisions
    // are omitted without a failure, and after growth it is kept whole.
    let token = 0x2000 + leader::TOKEN;
    image[token..token + 8].copy_from_slice(&0x4800_u64.to_le_bytes());
    image[token + 16..token + 24].copy_from_slice(&300_u64.to_le_bytes());
    image[token + 24..token + 32].copy_from_slice(&300_u64.to_le_bytes());
    image[0x4800..0x4800 + 300].fill(b'k');
    image[0x4800 + 300] = 0;
    let memory = Traced {
        bytes: &image,
        reads: RefCell::default(),
        fail: Cell::new(None),
    };
    let reset = |destination: &mut Destination<'_>| {
        destination.commander_keys.fill(CommanderKey::default());
        destination.commander_progress = Progress::default();
        destination.commander_key_count = 0;
        destination.commander_names.len = 0;
        destination.commander_names.required = 0;
        destination.division_commanders.clear();
        destination.progress = [Progress::default(); N];
    };
    reset(&mut destination);
    for _ in 0..2 {
        assert_eq!(destination.commander(&memory, 0, handle), None);
        assert_eq!(destination.progress[1].failure, None);
        assert_eq!(destination.commander_names.required, 300);
    }
    assert!(destination.division_commanders.is_empty());
    destination.commander_names = engine::cname_group_member::Names {
        bytes: &mut grown_commander_bytes,
        len: 0,
        required: 0,
    };
    reset(&mut destination);
    assert_eq!(destination.commander(&memory, 0, handle), Some(0));
    assert_eq!(destination.division_commanders[0].name_token, (0, 300));
    assert_eq!(destination.commander_names.len, 300);
    assert!(
        destination
            .commander_names
            .bytes
            .iter()
            .all(|&byte| byte == b'k')
    );
}

/// The viewer builds the land window's lists straight from these spans, so
/// each combat's committed and reserve arrays keep their native index
/// order (1415CA230/1415CA460) after the arena offset is applied.
#[test]
fn roster_spans_project_committed_and_reserve_lists_in_arena_order() {
    let c = Capacities {
        combats: 2,
        census_participants: 1,
        details: 2,
        divisions: 1,
        sides: 1,
        combat_sides: 1,
        country_tags: 1,
        rosters: 6,
        modifier_members: 1,
        modifiers: 1,
        width_terms: 1,
        exile_tags: 1,
        division_names: 1,
        name_templates: 1,
        name_groups: 1,
        reinforcement_modifiers: 1,
        supply_terms: 1,
        acclimatization: 1,
        transport_terms: 1,
        division_commanders: 1,
        commander_names: 1,
        commander_keys: 2,
        commander_traits: 2,
        detail_text: 2,
        timed_traits: 2,
        air_counts: 2,
        learning_traits: 2,
    };
    let mut captured = Captured::with_capacities(&c);
    captured.details.outcome = Outcome::Rows { len: 2 };
    captured.rosters.len = 6;
    captured
        .rosters
        .first
        .copy_from_slice(&[13, 6, 11, 4, 1, 9]);
    for (row, id, spans) in [
        (0, 0x1000_0000_003e, [(0, 2, 2, 1), (3, 0, 3, 0)]),
        (1, 0x2000_0000_003e, [(3, 0, 3, 0), (3, 1, 4, 2)]),
    ] {
        let detail = &mut captured.details.rows[row].row;
        detail.id = id;
        for (side, (committed_start, committed_len, reserve_start, reserve_len)) in
            detail.sides.iter_mut().zip(spans)
        {
            side.committed_start = committed_start;
            side.committed_len = committed_len;
            side.reserve_start = reserve_start;
            side.reserve_len = reserve_len;
        }
    }
    let [
        TableView::Complete(replay::rows::TableRows::LandCombatDetails(details)),
        _,
        _,
        _,
        ..,
    ] = captured.tables()
    else {
        panic!("complete land details were not projected")
    };
    let lists: Vec<_> = details
        .iter()
        .map(|row| {
            row.sides
                .clone()
                .map(|side| [side.committed, side.reserves])
        })
        .collect();
    assert_eq!(
        lists,
        [
            [[vec![13, 6], vec![11]], [vec![], vec![]]],
            [[vec![], vec![]], [vec![4], vec![1, 9]]],
        ]
    );
}

/// The pass's destination over a capture's own buffers, as `pass` builds it.
fn destination(c: &mut Captured) -> Destination<'_> {
    c.scratch.ensure();
    Destination {
        sources: &mut c.sources,
        commanders: Default::default(),
        images: &mut c.images,
        scratch: &mut c.scratch,
        reference: false,
        name_groups: &mut c.name_cache,
        names: engine::cname_group_member::Names {
            bytes: &mut c.division_names.first,
            len: 0,
            required: 0,
        },
        division_commanders: &mut c.division_commanders.first,
        commander_names: engine::cname_group_member::Names {
            bytes: &mut c.commander_names.first,
            len: 0,
            required: 0,
        },
        commander_progress: Progress::default(),
        commander_keys: &mut c.commander_keys,
        commander_key_count: 0,
        required_commander_keys: 0,
        details: &mut c.details.rows,
        divisions: &mut c.divisions.rows,
        sides: &mut c.sides.rows,
        combat_sides: &mut c.combat_sides.rows,
        country_tags: &mut c.country_tags.first,
        country_tags_progress: Progress::default(),
        country_set_limit: Ok(1),
        rosters: &mut c.rosters.first,
        commander_traits: &mut c.commander_traits.first,
        detail_text: &mut c.detail_text.first,
        timed_traits: &mut c.timed_traits.first,
        air_counts: &mut c.air_counts.first,
        learning_traits: &mut c.learning_traits.first,
        unit_scratch: &mut c.unit_scratch,
        detail_used: cland_combat::DetailCounts::default(),
        detail_required: cland_combat::DetailCounts::default(),
        members: &mut c.members.first,
        modifiers: &mut c.modifiers.first,
        width_terms: &mut c.width_terms.first,
        width_progress: Progress::default(),
        exile_tags: &mut c.exile_tags.first,
        exile_progress: Progress::default(),
        reinforcement_modifiers: &mut c.reinforcement_modifiers.first,
        supply_terms: &mut c.supply_terms.first,
        acclimatization: &mut c.acclimatization.first,
        transport_terms: &mut c.transport_terms.first,
        reinforcement_progress: [Progress::default(); 4],
        progress: [Progress::default(); N],
        arenas: [Progress::default(); N],
        ticks: [0; N],
    }
}

/// Two committed exile divisions of one combat where the second's lists
/// overflow the arena: the first keeps its rebased spans, the second is
/// counted out as unread, and after the spool's growth the next pass
/// keeps both. The native reads before admission are the engine's
/// `capture_exile` tests; here each division's reads are given.
#[test]
fn an_overflowing_exile_division_is_counted_out_then_kept_after_growth() {
    let mut c = Capacities {
        exile_tags: 3,
        ..Capacities::default()
    };
    let mut captured = Captured::with_capacities(&c);
    let combat = 0x1000_0000_003e;
    let empty = memory_less();
    let division = |exile| Acquired {
        stats: carmy_combat_inputs::Row {
            naming: Some(Default::default()),
            ..Default::default()
        },
        exile,
        context: Default::default(),
        reinforcement: Default::default(),
    };
    let place = |ordinal, exile_start| Placement {
        pointer: 0,
        combat,
        side: 0,
        ordinal,
        province: 7,
        role: 1,
        exile_start,
    };
    let row = |cores, occupiers| {
        carmy_combat_inputs::Exile::Row(carmy_combat_inputs::ExileRow {
            origin_type: 2,
            country: 4,
            cores,
            occupiers,
            ..Default::default()
        })
    };
    let pass = |captured: &mut Captured, second: carmy_combat_inputs::Exile| {
        let mut d = destination(captured);
        grow_vec(d.divisions, 2);
        // What capture_exile leaves: the first division's two cores.
        d.exile_tags[..2].copy_from_slice(&[11, 12]);
        for (ordinal, exile) in [row((0, 2), (2, 0)), second].into_iter().enumerate() {
            d.progress[1].required += 1;
            let start = d.exile_progress.len;
            if let carmy_combat_inputs::Exile::Row(_) = exile
                && ordinal == 1
            {
                d.exile_tags[start..start + 3].copy_from_slice(&[21, 22, 23]);
            }
            d.admit(&empty, 0, place(ordinal as u32, start), division(exile));
        }
        (d.progress[1], d.exile_progress)
    };
    // The second division needs three more tags than the arena holds.
    let (divisions, exile) = pass(
        &mut captured,
        carmy_combat_inputs::Exile::DoesNotFit { required: 3 },
    );
    assert_eq!(
        divisions.outcome(Outcome::Rows { len: 1 }),
        Outcome::Truncated {
            len: 1,
            unread_rows: 1
        }
    );
    assert_eq!((exile.len, exile.required), (2, 5));
    let first = captured.divisions.rows[0].exile.unwrap();
    assert_eq!((first.cores, first.occupiers), ((0, 2), (2, 0)));
    // The spool grows the arena from the counted demand.
    captured.exile_tags.len = exile.len;
    captured.exile_tags.required = exile.required;
    captured.grow(&mut c);
    assert!(captured.exile_tags.first.len() >= 5);
    // The next pass keeps both, the second rebased past the first.
    let (divisions, exile) = pass(&mut captured, row((0, 1), (1, 2)));
    assert_eq!(
        divisions.outcome(Outcome::Rows { len: 1 }),
        Outcome::Rows { len: 2 }
    );
    assert_eq!((exile.len, exile.required), (5, 5));
    let second = captured.divisions.rows[1].exile.unwrap();
    assert_eq!((second.cores, second.occupiers), ((2, 1), (3, 2)));
    captured.exile_tags.len = exile.len;
    captured.divisions.outcome = Outcome::Rows { len: 2 };
    let [
        _,
        TableView::Complete(replay::rows::TableRows::LandCombatDivisions(rows)),
        _,
        _,
        ..,
    ] = captured.tables()
    else {
        panic!("complete land divisions were not projected")
    };
    assert_eq!(rows[0].exile.as_ref().unwrap().location_cores, [11, 12]);
    assert_eq!(rows[1].exile.as_ref().unwrap().location_cores, [21]);
    assert_eq!(rows[1].exile.as_ref().unwrap().occupiers, [22, 23]);
}

/// No native reads happen during admission of a division without a
/// commander; any read would fail.
fn memory_less() -> engine::memory::SliceMemory<'static> {
    engine::memory::SliceMemory {
        base: 0,
        bytes: &[],
    }
}

/// The exile arena is a fixed destination during the pass: a division whose
/// lists do not fit is counted, and the spool grows the arena from that
/// demand rather than the pass allocating it.
#[test]
fn exile_arena_grows_off_the_pass_from_its_counted_demand() {
    let mut c = Capacities::default();
    let mut captured = Captured::with_capacities(&c);
    let before = captured.exile_tags.first.len();
    captured.exile_tags.len = 3;
    captured.exile_tags.required = before * 5 + 3;
    captured.grow(&mut c);
    assert!(captured.exile_tags.first.len() >= before * 5 + 3);
    assert!(c.exile_tags >= before * 5 + 3);
}

/// Exile spans index the shared tag arena; a division whose template is
/// not exiled (140C79AC0) carries no exile operands.
#[test]
fn naming_direction_and_exile_operands_project_onto_division_rows() {
    let c = Capacities {
        combats: 1,
        census_participants: 1,
        details: 1,
        divisions: 2,
        sides: 1,
        combat_sides: 1,
        country_tags: 1,
        rosters: 1,
        modifier_members: 1,
        modifiers: 1,
        width_terms: 1,
        exile_tags: 1,
        division_names: 1,
        name_templates: 1,
        name_groups: 1,
        reinforcement_modifiers: 1,
        supply_terms: 1,
        acclimatization: 1,
        transport_terms: 1,
        division_commanders: 1,
        commander_names: 1,
        commander_keys: 2,
        commander_traits: 2,
        detail_text: 2,
        timed_traits: 2,
        air_counts: 2,
        learning_traits: 2,
    };
    let mut captured = Captured::with_capacities(&c);
    captured.divisions.outcome = Outcome::Rows { len: 2 };
    captured.exile_tags.first = vec![99, 7, 8, 11];
    captured.exile_tags.len = 4;
    // Names index the pass's byte arena: a key and a player's name.
    captured.division_names.first = b"xGER_INF_01Guards".to_vec();
    captured.division_names.len = 17;
    let stats = carmy_combat_inputs::Row {
        direction_group: -3,
        categories: 0x4000_0000_0000_0001,
        naming: Some(engine::cname_group_member::Inputs {
            kind: 2,
            order: 14,
            ordered: 1,
            custom: (11, 6),
            group: (1, 10),
            ..Default::default()
        }),
        ..Default::default()
    };
    captured.divisions.rows[0] = Division {
        combat: 0x1000_0000_003e,
        province: 362,
        role: 1,
        stats,
        exile: Some(carmy_combat_inputs::ExileRow {
            origin_type: 2,
            country: 0,
            country_fallback: 12,
            cores: (1, 2),
            occupiers: (3, 1),
        }),
        ..Default::default()
    };
    captured.divisions.rows[1] = Division {
        combat: 0x1000_0000_003e,
        ordinal: 1,
        role: 1,
        exile: None,
        ..Default::default()
    };
    let [
        _,
        TableView::Complete(replay::rows::TableRows::LandCombatDivisions(divisions)),
        _,
        _,
        ..,
    ] = captured.tables()
    else {
        panic!("complete land divisions were not projected")
    };
    assert_eq!(divisions[0].province, 362);
    assert_eq!(divisions[0].direction_group, -3);
    assert_eq!(divisions[0].categories, 0x4000_0000_0000_0001);
    assert_eq!(
        (
            divisions[0].name_type,
            divisions[0].name_order,
            divisions[0].name_ordered,
            divisions[0].name_group.as_slice(),
            divisions[0].name_custom.as_slice()
        ),
        (2, 14, 1, b"GER_INF_01".as_slice(), b"Guards".as_slice())
    );
    assert_eq!(
        divisions[0].exile,
        Some(LandCombatExileInputs {
            origin_type: 2,
            country: 0,
            country_fallback: 12,
            location_cores: vec![7, 8],
            occupiers: vec![11],
        })
    );
    assert_eq!(divisions[1].exile, None);
}

/// The reinforcement reader writes enemy transport terms straight into the
/// arena past its admitted length and grows it on the worker; a division
/// with thousands of enemies projects every term in native order and the
/// grown arena feeds the next capacity.
#[test]
fn transport_terms_beyond_the_default_arena_project_every_enemy() {
    let mut c = Capacities {
        combats: 1,
        census_participants: 1,
        details: 1,
        divisions: 2,
        sides: 1,
        combat_sides: 1,
        country_tags: 1,
        rosters: 1,
        modifier_members: 1,
        modifiers: 1,
        width_terms: 1,
        exile_tags: 1,
        division_names: 1,
        name_templates: 1,
        name_groups: 1,
        reinforcement_modifiers: 1,
        supply_terms: 1,
        acclimatization: 1,
        transport_terms: 1,
        division_commanders: 1,
        commander_names: 1,
        commander_keys: 2,
        commander_traits: 2,
        detail_text: 2,
        timed_traits: 2,
        air_counts: 2,
        learning_traits: 2,
    };
    let mut captured = Captured::with_capacities(&c);
    let enemies = 5_000;
    let term = |index: usize| reinforcement::TransportTerm {
        country: index as u32 + 2,
        regional_value: index as i64 * 10,
        own_intel: i64::MIN + index as i64,
        enemy_intel: -(index as i64),
    };
    // What the reader leaves behind: the arena grown past its capacity.
    captured
        .transport_terms
        .first
        .resize(1 + enemies, reinforcement::TransportTerm::default());
    captured.transport_terms.first[0] = term(usize::MAX >> 40);
    for index in 0..enemies {
        captured.transport_terms.first[1 + index] = term(index);
    }
    captured.transport_terms.len = 1 + enemies;
    captured.transport_terms.required = 1 + enemies;
    captured.divisions.outcome = Outcome::Rows { len: 2 };
    for (ordinal, row) in captured.divisions.rows[..2].iter_mut().enumerate() {
        row.combat = 0x1000_0000_003e;
        row.ordinal = ordinal as u32;
    }
    captured.divisions.rows[0]
        .reinforcement
        .transport_term_count = 1;
    captured.divisions.rows[1].reinforcement_starts[3] = 1;
    captured.divisions.rows[1]
        .reinforcement
        .transport_term_count = enemies;
    let [
        _,
        TableView::Complete(replay::rows::TableRows::LandCombatDivisions(divisions)),
        _,
        _,
        ..,
    ] = captured.tables()
    else {
        panic!("complete land divisions were not projected")
    };
    assert_eq!(divisions[0].reinforcement_transport.len(), 1);
    let projected = &divisions[1].reinforcement_transport;
    assert_eq!(projected.len(), enemies);
    for (index, row) in projected.iter().enumerate() {
        let expected = term(index);
        assert_eq!(
            (
                row.country,
                row.regional_value,
                row.own_intel,
                row.enemy_intel
            ),
            (
                expected.country,
                expected.regional_value,
                expected.own_intel,
                expected.enemy_intel
            )
        );
    }
    captured.grow(&mut c);
    assert!(c.transport_terms > enemies);
    assert!(captured.transport_terms.first.len() >= c.transport_terms);
}

#[test]
fn source_failures_and_primary_arena_capacity_are_explicit() {
    let mut c = Capacities {
        combats: 1,
        census_participants: 2,
        details: 1,
        divisions: 2,
        sides: 2,
        combat_sides: 2,
        country_tags: 2,
        rosters: 2,
        modifier_members: 2,
        modifiers: 2,
        width_terms: 2,
        exile_tags: 2,
        division_names: 1,
        name_templates: 1,
        name_groups: 1,
        reinforcement_modifiers: 2,
        supply_terms: 2,
        acclimatization: 2,
        transport_terms: 2,
        division_commanders: 2,
        commander_names: 64,
        commander_keys: 1,
        commander_traits: 1,
        detail_text: 1,
        timed_traits: 1,
        air_counts: 1,
        learning_traits: 1,
    };
    let mut captured = Captured::with_capacities(&c);
    let memory = engine::memory::SliceMemory {
        base: 0,
        bytes: &[],
    };
    captured.capture(&memory, 0, 0);
    assert!(captured.commander_keys.len() >= captured.census_participants.len());
    assert!(
        captured
            .tables()
            .iter()
            .all(|t| matches!(t, TableView::Unavailable { .. }))
    );
    assert!(matches!(
        Progress {
            len: 1,
            required: 1,
            failure: None
        }
        .outcome(Outcome::Truncated {
            len: 1,
            unread_rows: 2
        }),
        Outcome::Unavailable { .. }
    ));
    captured.reset();
    captured.sides.outcome = Outcome::Truncated {
        len: 1,
        unread_rows: 3,
    };
    captured.members.required = 7;
    captured.modifiers.required = 19;
    captured.width_terms.required = 9;
    captured.division_commanders.required = 5;
    captured.required_commander_keys = 7;
    captured.grow(&mut c);
    assert!(c.division_commanders >= 5 && c.commander_keys >= 7);
    assert!(c.sides >= 4 && c.modifier_members >= 7 && c.modifiers >= 19 && c.width_terms >= 9);
    captured.details.outcome = Outcome::Rows { len: 1 };
    captured.divisions.outcome = Outcome::Unavailable {
        reason: "native division read failed",
    };
    captured.sides.outcome = Outcome::Rows { len: 1 };
    assert_eq!(captured.outcomes()[0], Outcome::Rows { len: 1 });
    assert!(matches!(
        captured.outcomes()[1],
        Outcome::Unavailable { .. }
    ));
    assert_eq!(captured.outcomes()[2], Outcome::Rows { len: 1 });
    let allocated = captured.allocated_bytes();
    captured.fail("ordinary worker unavailable");
    assert_eq!(
        captured.outcomes(),
        [Outcome::Unavailable {
            reason: "ordinary worker unavailable"
        }; N],
    );
    assert_eq!(captured.allocated_bytes(), allocated);
}

#[test]
fn combat_sides_project_independently_of_the_larger_tables() {
    let mut captured = Captured::with_capacities(&Capacities::default());
    captured.details.outcome = Outcome::Unavailable {
        reason: "tactic context failed",
    };
    captured.country_tags.first[..5].copy_from_slice(&[7, 0, 9, 4, 2]);
    captured.combat_sides.rows[0] = CombatSides {
        sets: cland_combat::CountrySetsRow {
            combat: 62,
            indicator: 1,
            flags: [0x4000, 0x800],
            start: 0,
            lens: [2, 1, 2, 0],
        },
    };
    captured.combat_sides.outcome = Outcome::Rows { len: 1 };
    let [
        TableView::Unavailable { .. },
        _,
        _,
        TableView::Complete(replay::rows::TableRows::LandCombatSides(sides)),
        ..,
    ] = captured.tables()
    else {
        panic!("the side table was not projected independently")
    };
    assert_eq!(
        sides,
        [LandCombatSideRow {
            combat: 62,
            indicator: 1,
            flags: [0x4000, 0x800],
            attacker_primary: vec![7, 0],
            attacker_secondary: vec![9],
            defender_primary: vec![4, 2],
            defender_secondary: vec![],
        }]
    );
}

/// No detail list has a chosen bound: every arena follows the demand of
/// the largest hour, off the game thread.
#[test]
fn detail_arenas_grow_from_live_demand() {
    let mut c = Capacities::default();
    let mut captured = Captured::with_capacities(&c);
    captured.commander_traits.required = 1_000;
    captured.detail_text.required = 100_000;
    captured.timed_traits.required = 500;
    captured.air_counts.required = 3_000;
    captured.learning_traits.required = 7_000;
    captured.rosters.required = 9_000;
    captured.grow(&mut c);
    assert!(captured.air_counts.first.len() >= 3_000);
    assert!(captured.learning_traits.first.len() >= 7_000);
    assert!(captured.rosters.first.len() >= 9_000);
    assert!(c.commander_traits >= 1_000);
    assert!(c.detail_text >= 100_000);
    assert!(c.timed_traits >= 500);
    assert!(captured.commander_traits.first.len() >= 1_000);
    assert!(captured.detail_text.first.len() >= 100_000);
    assert!(captured.timed_traits.first.len() >= 500);
}
