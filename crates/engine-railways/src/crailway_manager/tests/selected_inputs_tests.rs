use super::*;

fn selected_fixture() -> EtaMemory {
    let mut memory = eta_fixture();
    put(
        &mut memory.bytes,
        0x2100 + engine_native::cprovince::definition::NEIGHBOUR_COUNT,
        3_i32.to_le_bytes(),
    );
    array(
        &mut memory.bytes,
        0x1000 + info::LEVELS_BY_NEIGHBOR_INDEX,
        0x8500,
        4,
        3,
    );
    for (index, level) in [0_i32, 5, i32::MIN].into_iter().enumerate() {
        put(&mut memory.bytes, 0x7500 + index * 4, level.to_le_bytes());
    }
    put(&mut memory.bytes, 0x750c, i32::MAX.to_le_bytes());
    memory
}

#[test]
fn attach_sizing_reads_only_authenticated_level_headers_and_preserves_faults() {
    let mut memory = selected_fixture();
    memory.refused = Some(0x8500);
    let required = selected_inputs::demand(&memory, 0, MEMORY_BASE).unwrap();
    assert_eq!(required, 3);
    assert!(
        selected(&memory, required).1[0]
            .selection
            .unwrap()
            .levels
            .is_err()
    );
    memory.refused = None;
    assert!(
        selected(&memory, required).1[0]
            .selection
            .unwrap()
            .levels
            .is_ok()
    );
    put(
        &mut memory.bytes,
        0x2100 + engine_native::cprovince::definition::NEIGHBOUR_COUNT,
        2_i32.to_le_bytes(),
    );
    assert_eq!(
        selected_inputs::demand(&memory, 0, MEMORY_BASE),
        Err("selected railway level count differs from native definition neighbours")
    );
    put(
        &mut memory.bytes,
        0x2100 + engine_native::cprovince::definition::NEIGHBOUR_COUNT,
        3_i32.to_le_bytes(),
    );
    memory.refused = Some(0x2000 + info::PROVINCE_RECORD);
    assert!(selected_inputs::demand(&memory, 0, MEMORY_BASE).is_err());
}

fn selected(
    memory: &impl Memory,
    capacity: usize,
) -> (
    BuildingOutcomes,
    Vec<fresh_inputs::RailwayBuildingRow>,
    Vec<i32>,
    usize,
    usize,
) {
    let mut values = vec![71; capacity];
    let mut levels = selected_inputs::Levels {
        values: &mut values,
        len: 0,
        required: 0,
    };
    let mut buildings = vec![fresh_inputs::RailwayBuildingRow::default(); 2];
    let outcomes = walk_with_selection(
        memory,
        0,
        MEMORY_BASE,
        &mut vec![info::Row::default(); 2],
        &mut vec![edge::Row::default(); 3],
        &mut buildings,
        &mut levels,
    );
    let (len, required) = (levels.len, levels.required);
    (outcomes, buildings, values, len, required)
}

#[test]
fn selected_railway_keeps_full_signed_indexed_levels_independent_from_sorted_pairs() {
    let memory = selected_fixture();
    let (outcomes, rows, values, len, required) = selected(&memory, 4);
    assert_eq!(outcomes.rows.provinces, Outcome::Rows { len: 2 });
    assert_eq!(outcomes.rows.edges, Outcome::Rows { len: 3 });
    assert_eq!((len, required), (3, 3));
    assert_eq!(values, [0, 5, i32::MIN, 71]);
    let first = rows[0].selection.unwrap();
    assert_eq!(
        first.levels,
        Ok(selected_inputs::LevelSpan {
            first: 0,
            len: 3,
            positive: true
        })
    );
    assert_eq!(first.pair_count, Ok(2));
    assert_eq!(first.building_level, Ok(Some(Some(4))));
    let empty = rows[1].selection.unwrap();
    assert_eq!(
        empty.levels,
        Ok(selected_inputs::LevelSpan {
            first: 3,
            len: 0,
            positive: false
        })
    );
    assert_eq!(empty.building_level, Ok(None));
}

#[test]
fn selected_railway_raw_nonzero_count_survives_unreadable_or_invalid_pair_headers() {
    for (count, refused) in [(2_i32, Some(0x2000 + info::NEIGHBOR_PAIRS)), (-1, None)] {
        let mut memory = selected_fixture();
        put(
            &mut memory.bytes,
            0x1000 + info::NEIGHBOR_PAIR_COUNT,
            count.to_le_bytes(),
        );
        for index in 0..3 {
            put(&mut memory.bytes, 0x7500 + index * 4, 0_i32.to_le_bytes());
        }
        memory.refused = refused;
        let (outcomes, rows, _, _, _) = selected(&memory, 3);
        assert_eq!(outcomes.rows.provinces, Outcome::Rows { len: 2 });
        assert!(matches!(outcomes.rows.edges, Outcome::Unavailable { .. }));
        assert!(rows[0].building.is_err());
        let inputs = rows[0].selection.unwrap();
        assert_eq!(inputs.pair_count, Ok(count));
        assert!(!inputs.levels.unwrap().positive);
        assert_eq!(inputs.building_level, Ok(Some(Some(4))));
    }
}

#[test]
fn selected_railway_level_faults_retain_demand_and_independent_building() {
    let mut memory = selected_fixture();
    let (_, rows, _, len, required) = selected(&memory, 3);
    assert_eq!((len, required), (3, 3));
    assert!(rows[0].selection.unwrap().levels.is_ok());
    memory.refused = Some(0x8500);
    let (_, rows, _, len, required) = selected(&memory, 3);
    assert_eq!((len, required), (0, 3));
    assert_eq!(
        rows[0].selection.unwrap().levels,
        Err("selected railway indexed levels unreadable")
    );
    assert_eq!(rows[0].selection.unwrap().building_level, Ok(Some(Some(4))));
    memory.refused = None;
    put(
        &mut memory.bytes,
        0x2100 + engine_native::cprovince::definition::NEIGHBOUR_COUNT,
        2_i32.to_le_bytes(),
    );
    let (_, rows, _, _, _) = selected(&memory, 3);
    assert_eq!(
        rows[0].selection.unwrap().levels,
        Err("selected railway level count differs from native definition neighbours")
    );
}

#[test]
fn selected_railway_indexed_gate_survives_a_refused_raw_count() {
    let mut memory = selected_fixture();
    memory.refused = Some(0x2000 + info::NEIGHBOR_PAIR_COUNT);
    let (outcomes, rows, values, len, _) = selected(&memory, 3);
    assert_eq!(outcomes.rows.provinces, Outcome::Rows { len: 2 });
    assert!(matches!(outcomes.rows.edges, Outcome::Unavailable { .. }));
    assert_eq!(&values[..len], &[0, 5, i32::MIN]);
    let inputs = rows[0].selection.unwrap();
    assert_eq!(
        inputs.pair_count,
        Err("selected railway raw pair count unreadable")
    );
    assert_eq!(inputs.building_level, Ok(Some(Some(4))));
}

#[test]
fn selected_railway_health_fault_does_not_hide_level_and_original_reads_are_single() {
    struct Traced<'a> {
        memory: &'a EtaMemory,
        reads: std::cell::RefCell<Vec<(usize, usize)>>,
    }
    impl Memory for Traced<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, output.len()));
            self.memory.copy(address, output)
        }
    }
    for fault in [
        engine_native_parts::cbuilding::HEALTHY_LEVEL,
        engine_native_parts::cbuilding::PARTIAL_HEALTH,
    ] {
        let mut memory = selected_fixture();
        memory.refused = Some(0x4000 + fault);
        let traced = Traced {
            memory: &memory,
            reads: Default::default(),
        };
        let (_, rows, _, _, _) = selected(&traced, 3);
        assert!(rows[0].building.is_err());
        assert_eq!(rows[0].selection.unwrap().building_level, Ok(Some(Some(4))));
        for (address, width) in [
            (0x2000 + info::NEIGHBOR_PAIR_COUNT, 4),
            (0x4000 + engine_native_parts::cbuilding::LEVEL, 2),
        ] {
            assert_eq!(
                traced
                    .reads
                    .borrow()
                    .iter()
                    .filter(|&&(at, len)| at == address && len == width)
                    .count(),
                1
            );
        }
    }
}

fn sized_railways(count: usize, pair_count: usize, level_count: usize) -> Vec<u8> {
    let slots = count + 1;
    let rail_slots = 0x1000;
    let province_slots = rail_slots + slots * 8;
    let provinces = province_slots + slots * 8;
    let definitions = provinces + count * 0x300;
    let infos = definitions + count * 0x100;
    let indexed_levels = infos + count * info::SIZE;
    let pairs = indexed_levels + count * level_count * 4;
    let mut bytes = vec![0; pairs + count * pair_count * edge::STRIDE];
    put(
        &mut bytes,
        cgame_state::RAIL_MANAGER,
        0x1400_u64.to_le_bytes(),
    );
    put(&mut bytes, 0x400, (VTABLE_RVA.0 as u64).to_le_bytes());
    array(
        &mut bytes,
        0x400 + PROVINCE_INFO_TABLE,
        (MEMORY_BASE + rail_slots) as u64,
        slots as u32,
        slots as u32,
    );
    array(
        &mut bytes,
        cgame_state::PROVINCES,
        (MEMORY_BASE + province_slots) as u64,
        slots as u32,
        slots as u32,
    );
    for index in 0..count {
        let id = index as u32 + 1;
        let province = provinces + index * 0x300;
        let definition = definitions + index * 0x100;
        let node = infos + index * info::SIZE;
        let levels = indexed_levels + index * level_count * 4;
        let neighbors = pairs + index * pair_count * edge::STRIDE;
        put(
            &mut bytes,
            rail_slots + id as usize * 8,
            ((MEMORY_BASE + node) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            province_slots + id as usize * 8,
            ((MEMORY_BASE + province) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            province,
            (cprovince::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut bytes, province + cprovince::ID, id.to_le_bytes());
        put(
            &mut bytes,
            province + cprovince::DEFINITION,
            ((MEMORY_BASE + definition) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            definition + engine_native::cprovince::definition::ID,
            id.to_le_bytes(),
        );
        put(
            &mut bytes,
            definition + engine_native::cprovince::definition::NEIGHBOUR_COUNT,
            (level_count as i32).to_le_bytes(),
        );
        put(&mut bytes, node, (info::VTABLE_RVA.0 as u64).to_le_bytes());
        put(
            &mut bytes,
            node + info::PROVINCE_RECORD,
            ((MEMORY_BASE + definition) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            node + info::BUILDING_STATUS,
            ((MEMORY_BASE + province + cprovince::BUILDING_STATUS) as u64).to_le_bytes(),
        );
        array(
            &mut bytes,
            node + info::LEVELS_BY_NEIGHBOR_INDEX,
            (MEMORY_BASE + levels) as u64,
            level_count as u32,
            level_count as u32,
        );
        array(
            &mut bytes,
            node + info::NEIGHBOR_PAIRS,
            (MEMORY_BASE + neighbors) as u64,
            pair_count as u32,
            pair_count as u32,
        );
        for ordinal in 0..level_count {
            put(
                &mut bytes,
                levels + ordinal * 4,
                (ordinal as i32 - 2).to_le_bytes(),
            );
        }
        for (ordinal, neighbor) in (1..)
            .filter(|neighbor| *neighbor != id)
            .take(pair_count)
            .enumerate()
        {
            put(
                &mut bytes,
                neighbors + ordinal * edge::STRIDE,
                neighbor.to_le_bytes(),
            );
            put(
                &mut bytes,
                neighbors + ordinal * edge::STRIDE + edge::LEVEL,
                (ordinal as i32 - 1).to_le_bytes(),
            );
        }
    }
    bytes
}

#[test]
fn growing_railway_rows_edges_buildings_and_levels_follow_mod_sized_headers() {
    const COUNT: usize = 5_000;
    let mut provinces = Vec::new();
    let mut edges = Vec::new();
    let mut buildings = Vec::new();
    let mut values = Vec::new();
    for (pair_count, level_count) in [(1, 1), (3, 7)] {
        let bytes = sized_railways(COUNT, pair_count, level_count);
        let memory = SliceMemory {
            base: MEMORY_BASE,
            bytes: &bytes,
        };
        let mut levels = selected_inputs::Levels {
            values: &mut values,
            len: 0,
            required: 0,
        };
        let result = walk_with_selection(
            &memory,
            0,
            MEMORY_BASE,
            &mut provinces,
            &mut edges,
            &mut buildings,
            &mut levels,
        );
        assert_eq!(result.rows.provinces, Outcome::Rows { len: COUNT });
        assert_eq!(
            result.rows.edges,
            Outcome::Rows {
                len: COUNT * pair_count
            }
        );
        assert_eq!(result.buildings, Outcome::Rows { len: COUNT });
        assert_eq!(result.required, COUNT);
        assert_eq!(
            (levels.len, levels.required),
            (COUNT * level_count, COUNT * level_count)
        );
        assert_eq!(provinces[COUNT - 1].province, COUNT as u32);
        let span = buildings[COUNT - 1].selection.unwrap().levels.unwrap();
        assert_eq!(span.first, (COUNT - 1) * level_count);
        assert_eq!(span.len, level_count);
        assert_eq!(span.positive, level_count > 3);
        assert_eq!(
            &levels.values[span.first..span.first + span.len],
            &(0..level_count)
                .map(|ordinal| ordinal as i32 - 2)
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn growing_selected_railway_storage_keeps_reads_and_independent_faults() {
    use std::cell::RefCell;
    struct Traced<'a> {
        memory: &'a EtaMemory,
        reads: RefCell<Vec<(usize, usize)>>,
    }
    impl Memory for Traced<'_> {
        fn copy(&self, at: usize, out: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((at, out.len()));
            self.memory.copy(at, out)
        }
    }
    for fault in [
        None,
        Some(0x2000 + info::COOLDOWN_TICKS),
        Some(0x2000 + info::LEVELS_BY_NEIGHBOR_INDEX),
        Some(0x8500),
        Some(0x2200),
        Some(0x4000 + engine_native_parts::cbuilding::HEALTHY_LEVEL),
    ] {
        let mut memory = selected_fixture();
        memory.refused = fault;
        let fixed = Traced {
            memory: &memory,
            reads: RefCell::default(),
        };
        let (expected, expected_buildings, expected_values, expected_len, expected_required) =
            selected(&fixed, 3);
        let growing = Traced {
            memory: &memory,
            reads: RefCell::default(),
        };
        let mut provinces = Vec::new();
        let mut edges = Vec::new();
        let mut buildings = Vec::new();
        let mut values = Vec::new();
        let mut levels = selected_inputs::Levels {
            values: &mut values,
            len: 0,
            required: 0,
        };
        let actual = walk_with_selection(
            &growing,
            0,
            MEMORY_BASE,
            &mut provinces,
            &mut edges,
            &mut buildings,
            &mut levels,
        );
        assert_eq!(actual, expected, "fault {fault:?}");
        assert_eq!(
            &buildings[..expected.buildings.len()],
            &expected_buildings[..expected.buildings.len()],
            "fault {fault:?}"
        );
        assert_eq!(
            (levels.len, levels.required),
            (expected_len, expected_required),
            "fault {fault:?}"
        );
        assert_eq!(
            &levels.values[..expected_len],
            &expected_values[..expected_len],
            "fault {fault:?}"
        );
        assert_eq!(
            *growing.reads.borrow(),
            *fixed.reads.borrow(),
            "fault {fault:?}"
        );
    }
}
