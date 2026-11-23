use super::*;
use engine_native_memory::{cpdx_array, memory::SliceMemory};

mod selected_inputs_tests;

const MEMORY_BASE: usize = 0x1000;

fn put(bytes: &mut [u8], offset: usize, value: impl AsRef<[u8]>) {
    let value = value.as_ref();
    bytes[offset..offset + value.len()].copy_from_slice(value);
}

fn array(bytes: &mut [u8], offset: usize, data: u64, capacity: u32, count: u32) {
    put(bytes, offset, data.to_le_bytes());
    put(bytes, offset + cpdx_array::CAPACITY, capacity.to_le_bytes());
    put(bytes, offset + cpdx_array::COUNT, count.to_le_bytes());
}

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0_u8; 0x2400];
    put(
        &mut bytes,
        cgame_state::RAIL_MANAGER,
        0x1400_u64.to_le_bytes(),
    );
    put(&mut bytes, 0x400, (VTABLE_RVA.0 as u64).to_le_bytes());
    array(&mut bytes, 0x400 + PROVINCE_INFO_TABLE, 0x1500, 5, 4);
    array(&mut bytes, cgame_state::PROVINCES, 0x1540, 5, 4);
    put(&mut bytes, 0x508, 0x2000_u64.to_le_bytes());
    put(&mut bytes, 0x518, 0x2080_u64.to_le_bytes());
    put(&mut bytes, 0x520, u64::MAX.to_le_bytes());
    put(&mut bytes, 0x560, u64::MAX.to_le_bytes());
    for (id, offset) in [(1_u32, 0x600), (2, 0x900), (3, 0xc00)] {
        put(
            &mut bytes,
            0x540 + id as usize * 8,
            ((MEMORY_BASE + offset) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            offset,
            (cprovince::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut bytes, offset + cprovince::ID, id.to_le_bytes());
        put(
            &mut bytes,
            offset + cprovince::DEFINITION,
            (0x3000_u64 + u64::from(id) * 0x100).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x2000 + id as usize * 0x100 + engine_native::cprovince::definition::ID,
            id.to_le_bytes(),
        );
    }
    for (offset, id, province_offset, ticks) in
        [(0x1000, 1_u32, 0x600, 0_i32), (0x1080, 3, 0xc00, i32::MAX)]
    {
        put(&mut bytes, offset, (info::VTABLE_RVA.0 as u64).to_le_bytes());
        put(
            &mut bytes,
            offset + info::PROVINCE_RECORD,
            (0x3000_u64 + u64::from(id) * 0x100).to_le_bytes(),
        );
        put(
            &mut bytes,
            offset + info::BUILDING_STATUS,
            ((MEMORY_BASE + province_offset + cprovince::BUILDING_STATUS) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            offset + info::COOLDOWN_TICKS,
            ticks.to_le_bytes(),
        );
    }
    array(&mut bytes, 0x1000 + info::NEIGHBOR_PAIRS, 0x2200, 3, 2);
    array(&mut bytes, 0x1080 + info::NEIGHBOR_PAIRS, 0x2300, 1, 1);
    for (offset, neighbor, level) in [
        (0x1200, 2_i32, 7_i32),
        (0x1208, 3, i32::MAX),
        (0x1300, 1, i32::MIN),
    ] {
        put(&mut bytes, offset, neighbor.to_le_bytes());
        put(&mut bytes, offset + edge::LEVEL, level.to_le_bytes());
    }
    put(&mut bytes, 0x1210, u64::MAX.to_le_bytes());
    bytes
}

struct EtaMemory {
    bytes: Vec<u8>,
    defines: Vec<(usize, Vec<u8>)>,
    refused: Option<usize>,
}

impl Memory for EtaMemory {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if self
            .refused
            .is_some_and(|at| address <= at && at - address < output.len())
        {
            return false;
        }
        if let Some((_, value)) = self.defines.iter().find(|(at, _)| *at == address) {
            if value.len() != output.len() {
                return false;
            }
            output.copy_from_slice(value);
            return true;
        }
        SliceMemory {
            base: MEMORY_BASE,
            bytes: &self.bytes,
        }
        .copy(address, output)
    }
}

fn eta_fixture() -> EtaMemory {
    let mut bytes = fixture();
    bytes.resize(0x8000, 0);
    put(
        &mut bytes,
        0x1000 + info::BUILDING,
        0x4000_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        0x3000,
        (engine_native_parts::cbuilding::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        0x3000 + engine_native_parts::cbuilding::BUILDING_STATUS,
        (0x1600_u64 + cprovince::BUILDING_STATUS as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        0x3000 + engine_native_parts::cbuilding::LEVEL,
        4_i16.to_le_bytes(),
    );
    put(
        &mut bytes,
        0x3000 + engine_native_parts::cbuilding::HEALTHY_LEVEL,
        2_i16.to_le_bytes(),
    );
    put(
        &mut bytes,
        0x3000 + engine_native_parts::cbuilding::PARTIAL_HEALTH,
        5_000_000_i64.to_le_bytes(),
    );
    EtaMemory {
        bytes,
        refused: None,
        defines: vec![
            (0x033e_0928, 200_000_i64.to_le_bytes().to_vec()),
            (0x033e_09b8, 1_000_000_i64.to_le_bytes().to_vec()),
            (0x033e_0a48, 100_000_i64.to_le_bytes().to_vec()),
            (0x033e_0ae0, 1_000_000_i64.to_le_bytes().to_vec()),
            (0x033d_fbe8, 5_i32.to_le_bytes().to_vec()),
            (0x033d_d268, 0x6000_u64.to_le_bytes().to_vec()),
            (0x033e_5274, 5_i32.to_le_bytes().to_vec()),
        ],
    }
}

#[test]
fn rail_building_supplement_keeps_old_rows_and_faults_independent() {
    let mut memory = eta_fixture();
    let mut cooldowns = vec![info::Row::default(); 2];
    let mut edges = vec![edge::Row::default(); 3];
    let old = walk(&memory, 0, MEMORY_BASE, &mut cooldowns, &mut edges);
    let expected_cooldowns = cooldowns.clone();
    let expected_edges = edges.clone();
    {
        let mut buildings = Vec::new();
        let result = walk_with_buildings(
            &memory,
            0,
            MEMORY_BASE,
            &mut cooldowns,
            &mut edges,
            &mut buildings,
        );
        assert_eq!(result.rows, old);
        assert_eq!(cooldowns, expected_cooldowns);
        assert_eq!(edges, expected_edges);
        assert_eq!(result.required, 2);
        assert_eq!(result.buildings, Outcome::Rows { len: 2 });
        assert_eq!(buildings[0].province, 1);
        assert_eq!(
            buildings[0].building,
            Ok(Some(fresh_inputs::RailwayBuildingState {
                building: Some(fresh_inputs::RailwayBuildingHealth {
                    level: 4,
                    healthy: 2,
                    partial: Some(5_000_000),
                }),
            }))
        );
        assert_eq!(buildings[1].building, Ok(None));
    }
    memory.refused = Some(0x4000 + engine_native_parts::cbuilding::PARTIAL_HEALTH);
    let mut buildings = vec![fresh_inputs::RailwayBuildingRow::default(); 2];
    let result = walk_with_buildings(
        &memory,
        0,
        MEMORY_BASE,
        &mut cooldowns,
        &mut edges,
        &mut buildings,
    );
    assert_eq!(result.rows, old);
    assert_eq!(cooldowns, expected_cooldowns);
    assert_eq!(edges, expected_edges);
    assert!(buildings[0].building.is_err());
    assert_eq!(buildings[1].building, Ok(None));
    assert_eq!(result.buildings, Outcome::Rows { len: 2 });

    memory.refused = None;
    put(
        &mut memory.bytes,
        0x1080 + info::BUILDING_STATUS,
        0_u64.to_le_bytes(),
    );
    let result = walk_with_buildings(
        &memory,
        0,
        MEMORY_BASE,
        &mut cooldowns,
        &mut edges,
        &mut buildings,
    );
    assert!(matches!(result.rows.provinces, Outcome::Unavailable { .. }));
    assert!(matches!(result.buildings, Outcome::Unavailable { .. }));
    assert_eq!(result.required, 1);
}

#[test]
fn eta_rail_speed_uses_directed_edge_damage_and_cooldown() {
    let mut memory = eta_fixture();
    assert_eq!(
        movement_speed(&memory, 0, MEMORY_BASE, 0x1600, 0x1900),
        Ok(975_000)
    );
    put(
        &mut memory.bytes,
        0x1000 + info::COOLDOWN_TICKS,
        (-1_i32).to_le_bytes(),
    );
    assert_eq!(
        movement_speed(&memory, 0, MEMORY_BASE, 0x1600, 0x1900),
        Ok(975_000)
    );
    put(
        &mut memory.bytes,
        0x1000 + info::COOLDOWN_TICKS,
        1_i32.to_le_bytes(),
    );
    memory.refused = Some(0x2000 + info::NEIGHBOR_PAIRS);
    assert_eq!(
        movement_speed(&memory, 0, MEMORY_BASE, 0x1600, 0x1900),
        Ok(200_000)
    );
    memory.refused = None;
    assert_eq!(
        movement_speed(&memory, 0, MEMORY_BASE, 0x1900, 0x1600),
        Ok(200_000)
    );
    put(
        &mut memory.bytes,
        0x1000 + info::COOLDOWN_TICKS,
        0_i32.to_le_bytes(),
    );
    memory.refused = Some(0x4000 + engine_native_parts::cbuilding::PARTIAL_HEALTH);
    assert!(movement_speed(&memory, 0, MEMORY_BASE, 0x1600, 0x1900).is_err());
    memory.refused = None;
    put(
        &mut memory.bytes,
        0x1000 + info::BUILDING,
        0_u64.to_le_bytes(),
    );
    assert_eq!(
        movement_speed(&memory, 0, MEMORY_BASE, 0x1600, 0x1900),
        Ok(200_000)
    );
}

#[test]
fn eta_infrastructure_uses_healthy_levels_and_native_maximum_precedence() {
    use engine_native_parts::{cbuilding as b, cbuilding_template as t};
    use engine_native::{cprovince::buildings as s, cstate};
    let mut memory = eta_fixture();
    let bytes = &mut memory.bytes;
    put(bytes, 0x900 + cprovince::STATE, 0x5000_u64.to_le_bytes());
    array(bytes, cgame_state::STATES, 0x5800, 2, 2);
    put(bytes, 0x4808, 0x5000_u64.to_le_bytes());
    put(bytes, 0x4000, (cstate::VTABLE_RVA.0 as u64).to_le_bytes());
    put(bytes, 0x4000 + cstate::ID, 1_u32.to_le_bytes());
    put(
        bytes,
        0x4000 + cstate::BUILDING_STATUS,
        (s::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    array(
        bytes,
        0x4000 + cstate::BUILDING_STATUS + s::BUILDING_INDICES,
        0x5900,
        2,
        2,
    );
    put(bytes, 0x4904, 0_i32.to_le_bytes());
    array(
        bytes,
        0x4000 + cstate::BUILDING_STATUS + s::BUILDINGS,
        0x5920,
        1,
        1,
    );
    put(bytes, 0x4920, 0x8000_u64.to_le_bytes());
    put(bytes, 0x5000 + 0x3b0, 0x7000_u64.to_le_bytes());
    array(
        bytes,
        0x5000 + engine_native_parts::cbuilding_database::TEMPLATES,
        0x6800,
        2,
        2,
    );
    put(bytes, 0x5808, 0x7000_u64.to_le_bytes());
    put(bytes, 0x6000 + t::LOOKUP_ENABLED_CANDIDATE, [1]);
    put(bytes, 0x6000 + t::DATABASE_ORDINAL, 1_u32.to_le_bytes());
    put(bytes, 0x6000 + t::TYPE_ID, 71_u32.to_le_bytes());
    put(bytes, 0x6000 + 0x2bc, 4_i32.to_le_bytes());
    put(bytes, 0x6000 + 0x2b8, 8_i32.to_le_bytes());
    put(bytes, 0x7000, (b::VTABLE_RVA.0 as u64).to_le_bytes());
    put(bytes, 0x7000 + b::TYPE_ID, 71_u32.to_le_bytes());
    put(
        bytes,
        0x7000 + b::BUILDING_STATUS,
        (0x5000_u64 + cstate::BUILDING_STATUS as u64).to_le_bytes(),
    );
    put(bytes, 0x7000 + b::TEMPLATE, 0x7000_u64.to_le_bytes());
    put(bytes, 0x7000 + b::LEVEL, 4_i16.to_le_bytes());
    put(bytes, 0x7000 + b::HEALTHY_LEVEL, 2_i16.to_le_bytes());
    put(bytes, 0x600 + cprovince::STATE, 0x5000_u64.to_le_bytes());
    struct Observed<'a> {
        source: &'a EtaMemory,
        reads: std::cell::RefCell<Vec<usize>>,
    }
    impl Memory for Observed<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.borrow_mut().push(address);
            self.source.copy(address, output)
        }
    }
    let direct_reads = {
        let observed = Observed {
            source: &memory,
            reads: Default::default(),
        };
        assert_eq!(
            eta_infrastructure(&observed, 0, MEMORY_BASE, 0x1900),
            Ok(50_000)
        );
        observed.reads.take()
    };
    assert_eq!(
        eta_infrastructure(&memory, 0, MEMORY_BASE, 0x1600),
        Ok(50_000)
    );
    for address in direct_reads {
        memory.refused = Some(address);
        assert!(eta_infrastructure(&memory, 0, MEMORY_BASE, 0x1900).is_err());
    }
    memory.refused = None;
    put(&mut memory.bytes, 0x6000 + 0x2bc, 0_i32.to_le_bytes());
    assert_eq!(
        eta_infrastructure(&memory, 0, MEMORY_BASE, 0x1900),
        Ok(25_000)
    );
    put(&mut memory.bytes, 0x6000 + 0x2b8, (-2_i32).to_le_bytes());
    assert_eq!(
        eta_infrastructure(&memory, 0, MEMORY_BASE, 0x1900),
        Ok(40_000)
    );
    put(&mut memory.bytes, 0x7000 + b::TEMPLATE, 0_u64.to_le_bytes());
    assert!(eta_infrastructure(&memory, 0, MEMORY_BASE, 0x1900).is_err());
}

#[test]
fn eta_rail_fixedpoint_retains_native_sentinels_and_signed_levels() {
    assert_eq!(eta_building_efficiency(0, 4, i64::MAX), Ok(0));
    assert_eq!(eta_building_efficiency(-2, -2, i64::MAX), Ok(100_000));
    assert_eq!(eta_building_efficiency(4, 2, 5_000_000), Ok(62_500));
    assert_eq!(eta_building_efficiency(4, 2, -5_000_000), Ok(37_500));
    assert_eq!(eta_ratio(1, 0), Ok(0xffff_ffff));
    for value in [i64::MIN, i64::MIN + 1, i64::MAX] {
        assert_eq!(eta_ratio(value, 100_000), Ok(0xffff_ffff));
    }
    assert_eq!(eta_ratio(-200_000, 400_000), Ok(-50_000));
    let mut memory = eta_fixture();
    memory
        .defines
        .iter_mut()
        .find(|(at, _)| *at == 0x033d_fbe8)
        .unwrap()
        .1 = 0_i32.to_le_bytes().to_vec();
    assert_eq!(
        movement_speed(&memory, 0, MEMORY_BASE, 0x1600, 0x1900),
        Ok(200_000)
    );
    put(
        &mut memory.bytes,
        0x1000 + info::BUILDING_STATUS,
        0_u64.to_le_bytes(),
    );
    assert!(movement_speed(&memory, 0, MEMORY_BASE, 0x1600, 0x1900).is_err());
}

fn run(bytes: &[u8], provinces: &mut Vec<info::Row>, edges: &mut Vec<edge::Row>) -> Outcomes {
    walk(
        &SliceMemory {
            base: MEMORY_BASE,
            bytes,
        },
        0,
        MEMORY_BASE,
        provinces,
        edges,
    )
}

#[test]
fn directed_edges_and_native_values_survive_without_vanilla_caps() {
    let mut provinces = vec![info::Row::default(); 4];
    let mut edges = vec![edge::Row::default(); 4];
    let mut bytes = fixture();
    put(&mut bytes, 0x1080 + info::BUILDING, 0x7000_u64.to_le_bytes());
    let result = run(&bytes, &mut provinces, &mut edges);
    assert_eq!(
        result,
        Outcomes {
            provinces: Outcome::Rows { len: 2 },
            edges: Outcome::Rows { len: 3 }
        }
    );
    assert_eq!(
        provinces[0],
        info::Row {
            province: 1,
            cooldown_ticks: 0,
            building_bound: false
        }
    );
    assert_eq!(
        provinces[1],
        info::Row {
            province: 3,
            cooldown_ticks: i32::MAX,
            building_bound: true
        }
    );
    assert_eq!(
        &edges[..3],
        &[
            edge::Row {
                from_province: 1,
                to_province: 2,
                level: 7
            },
            edge::Row {
                from_province: 1,
                to_province: 3,
                level: i32::MAX
            },
            edge::Row {
                from_province: 3,
                to_province: 1,
                level: i32::MIN
            },
        ]
    );
}

#[test]
fn null_slots_and_empty_vectors_are_known_absence() {
    let mut bytes = fixture();
    array(&mut bytes, 0x1000 + info::NEIGHBOR_PAIRS, 0, 0, 0);
    array(&mut bytes, 0x1080 + info::NEIGHBOR_PAIRS, 0, 0, 0);
    let mut provinces = vec![info::Row::default(); 4];
    assert_eq!(
        run(&bytes, &mut provinces, &mut Vec::new()).edges,
        Outcome::Rows { len: 0 }
    );
    bytes[0x500..0x520].fill(0);
    assert_eq!(
        run(&bytes, &mut Vec::new(), &mut Vec::new()),
        Outcomes {
            provinces: Outcome::Rows { len: 0 },
            edges: Outcome::Rows { len: 0 },
        }
    );
}

#[test]
fn malformed_roots_and_province_backlinks_are_unavailable() {
    for (offset, replacement) in [
        (cgame_state::RAIL_MANAGER, 0_u64.to_le_bytes().to_vec()),
        (0x400, 0_u64.to_le_bytes().to_vec()),
        (0x400 + PROVINCE_INFO_COUNT, 3_u32.to_le_bytes().to_vec()),
        (
            0x2100 + engine_native::cprovince::definition::ID,
            3_u32.to_le_bytes().to_vec(),
        ),
        (0x1000, 0_u64.to_le_bytes().to_vec()),
        (
            0x1000 + info::PROVINCE_RECORD,
            0x3003_u64.to_le_bytes().to_vec(),
        ),
        (
            0x1000 + info::BUILDING_STATUS,
            0x1d90_u64.to_le_bytes().to_vec(),
        ),
        (0x600 + cprovince::ID, 3_u32.to_le_bytes().to_vec()),
    ] {
        let mut bytes = fixture();
        put(&mut bytes, offset, replacement);
        let result = run(
            &bytes,
            &mut vec![info::Row::default(); 4],
            &mut vec![edge::Row::default(); 4],
        );
        assert!(
            matches!(result.provinces, Outcome::Unavailable { .. }),
            "offset {offset:x}"
        );
        assert!(
            matches!(result.edges, Outcome::Unavailable { .. }),
            "offset {offset:x}"
        );
    }
}

#[test]
fn invalid_edges_do_not_erase_readable_cooldowns() {
    for (offset, replacement) in [
        (0x1208, 2_u32.to_le_bytes().to_vec()),
        (0x1200, 1_u32.to_le_bytes().to_vec()),
        (0x1208, 4_u32.to_le_bytes().to_vec()),
        (
            0x1000 + info::NEIGHBOR_PAIR_COUNT,
            u32::MAX.to_le_bytes().to_vec(),
        ),
        (0x550, 0_u64.to_le_bytes().to_vec()),
    ] {
        let mut bytes = fixture();
        put(&mut bytes, offset, replacement);
        let result = run(
            &bytes,
            &mut vec![info::Row::default(); 4],
            &mut vec![edge::Row::default(); 4],
        );
        assert_eq!(result.provinces, Outcome::Rows { len: 2 });
        assert!(matches!(result.edges, Outcome::Unavailable { .. }));
    }
}

#[test]
fn faults_are_scoped_to_the_table_that_reads_the_field() {
    struct Refuse<'a> {
        memory: SliceMemory<'a>,
        address: usize,
    }
    impl Memory for Refuse<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if address <= self.address && self.address - address < destination.len() {
                return false;
            }
            self.memory.copy(address, destination)
        }
    }
    let bytes = fixture();
    for (address, provinces, edges) in [
        (
            0x2000 + info::COOLDOWN_TICKS,
            Outcome::Unavailable {
                reason: "rail cooldown unreadable",
            },
            Outcome::Rows { len: 3 },
        ),
        (
            0x2000 + info::NEIGHBOR_PAIRS,
            Outcome::Rows { len: 2 },
            Outcome::Unavailable {
                reason: "rail neighbor array unreadable or invalid",
            },
        ),
        (
            0x2200,
            Outcome::Rows { len: 2 },
            Outcome::Unavailable {
                reason: "rail neighbor rows unreadable",
            },
        ),
        (
            0x2000 + info::BUILDING,
            Outcome::Unavailable {
                reason: "rail info identity unreadable",
            },
            Outcome::Unavailable {
                reason: "rail info identity unreadable",
            },
        ),
    ] {
        let memory = Refuse {
            memory: SliceMemory {
                base: MEMORY_BASE,
                bytes: &bytes,
            },
            address,
        };
        let result = walk(
            &memory,
            0,
            MEMORY_BASE,
            &mut vec![info::Row::default(); 4],
            &mut vec![edge::Row::default(); 4],
        );
        assert_eq!(result.provinces, provinces, "refused {address:#x}");
        assert_eq!(result.edges, edges, "refused {address:#x}");
    }
}
#[allow(clippy::too_many_arguments)]
fn fresh_scratch_reference_edges(
    memory: &impl Memory,
    provinces: Array,
    province_vtable: usize,
    batch: &mut ProvinceBatch,
    pairs: Array,
    province: u32,
    output: &mut [edge::Row],
    progress: &mut Progress,
) -> Result<(), &'static str> {
    let take = (pairs.count as usize).min(output.len() - progress.len);
    progress.omit(pairs.count - take as u32);
    let mut buffer = [0_u8; PROVINCE_BATCH * edge::STRIDE];
    let mut neighbors = [0_u32; PROVINCE_BATCH];
    let mut previous = 0;
    for first in (0..take).step_by(PROVINCE_BATCH) {
        let count = (take - first).min(PROVINCE_BATCH);
        let address = pairs
            .element(first as u32, edge::STRIDE)
            .ok_or("rail pair address overflow")?;
        if !read_slice(memory, address, &mut buffer[..count * edge::STRIDE]) {
            return Err("rail neighbor rows unreadable");
        }
        let mut staged = 0;
        let mut invalid = false;
        let mut ordered = previous;
        for bytes in buffer[..count * edge::STRIDE].chunks_exact(edge::STRIDE) {
            let neighbor = i32::from_le_bytes(bytes[..edge::LEVEL].try_into().unwrap());
            if neighbor <= 0 || neighbor as u32 <= ordered || neighbor as u32 == province {
                invalid = true;
                break;
            }
            ordered = neighbor as u32;
            neighbors[staged] = ordered;
            staged += 1;
        }
        batch.gather(memory, provinces, &neighbors[..staged], &NEIGHBOR_LAYOUT);
        for (index, bytes) in buffer[..count * edge::STRIDE]
            .chunks_exact(edge::STRIDE)
            .enumerate()
        {
            if index == staged {
                if invalid {
                    return Err("rail neighbors duplicate, unsorted or invalid");
                }
                break;
            }
            let to_province = neighbors[index];
            batch.authenticate(index, to_province, province_vtable)?;
            output[progress.len] = edge::Row {
                from_province: province,
                to_province,
                level: i32::from_le_bytes(bytes[edge::LEVEL..].try_into().unwrap()),
            };
            progress.len += 1;
            previous = to_province;
        }
    }
    Ok(())
}

struct EdgeTrace<'a> {
    bytes: &'a [u8],
    blocked: Option<usize>,
    reads: std::cell::RefCell<Vec<(usize, usize)>>,
}

impl Memory for EdgeTrace<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, destination.len()));
        if self
            .blocked
            .is_some_and(|blocked| address <= blocked && blocked - address < destination.len())
        {
            let partial = destination.len().min(3);
            destination[..partial].fill(0xfd);
            return false;
        }
        SliceMemory {
            base: MEMORY_BASE,
            bytes: self.bytes,
        }
        .copy(address, destination)
    }
}

fn many_edge_pairs(count: usize) -> (Vec<u8>, Array, Array) {
    let mut bytes = vec![0; 0x12000];
    let provinces = Array {
        data: 0x1800,
        count: 70,
    };
    let pairs = Array {
        data: 0x2000,
        count: count as u32,
    };
    for id in 1..=66_u32 {
        let pointer = 0x4000 + id as usize * 0x200;
        put(
            &mut bytes,
            provinces.data - MEMORY_BASE + id as usize * 8,
            (pointer as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            pointer - MEMORY_BASE,
            (cprovince::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            pointer - MEMORY_BASE + cprovince::ID,
            id.to_le_bytes(),
        );
        if id as usize <= count {
            let at = pairs.data - MEMORY_BASE + (id as usize - 1) * edge::STRIDE;
            put(&mut bytes, at, id.to_le_bytes());
            put(
                &mut bytes,
                at + edge::LEVEL,
                (i32::MIN + id as i32).to_le_bytes(),
            );
        }
    }
    (bytes, provinces, pairs)
}

#[allow(clippy::too_many_arguments)]
fn compare_reused_edge_scratch(
    bytes: &[u8],
    blocked: Option<usize>,
    provinces: Array,
    pairs: Array,
    province: u32,
    capacity: usize,
    scratch: &mut EdgeScratch,
) {
    let current = EdgeTrace {
        bytes,
        blocked,
        reads: std::cell::RefCell::new(Vec::new()),
    };
    let reference = EdgeTrace {
        bytes,
        blocked,
        reads: std::cell::RefCell::new(Vec::new()),
    };
    let mut batch = ProvinceBatch::new();
    let mut old_batch = ProvinceBatch::new();
    batch.records.fill(0xa3);
    old_batch.records.fill(0xa3);
    batch.slot_bytes.fill(0xb4);
    old_batch.slot_bytes.fill(0xb4);
    let mut progress = Progress::default();
    let mut old_progress = Progress::default();
    let sentinel = edge::Row {
        from_province: 71,
        to_province: 72,
        level: -73,
    };
    let mut rows = vec![sentinel; capacity.max(pairs.count as usize)];
    let mut old_rows = rows.clone();
    assert_eq!(
        read_edges(
            &current,
            provinces,
            cprovince::VTABLE_RVA.0,
            &mut batch,
            scratch,
            pairs,
            province,
            &mut Buffer::Growing(&mut rows),
            &mut progress
        ),
        fresh_scratch_reference_edges(
            &reference,
            provinces,
            cprovince::VTABLE_RVA.0,
            &mut old_batch,
            pairs,
            province,
            &mut old_rows,
            &mut old_progress
        ),
        "pairs {}, capacity {capacity}, fault {blocked:?}",
        pairs.count
    );
    assert_eq!(progress.outcome(), old_progress.outcome());
    assert_eq!(rows, old_rows);
    assert_eq!(*current.reads.borrow(), *reference.reads.borrow());
    assert_eq!(batch.records, old_batch.records);
    assert_eq!(batch.slot_bytes, old_batch.slot_bytes);
    assert_eq!(
        (batch.slot_copied, batch.copied, batch.fields),
        (old_batch.slot_copied, old_batch.copied, old_batch.fields)
    );
}

#[test]
fn reused_edge_scratch_matches_fresh_windows_and_partial_faults() {
    let mut scratch = EdgeScratch::new();
    scratch.bytes.fill(0xa5);
    scratch.neighbors.fill(u32::MAX);
    for count in [66, 1, 0, 63, 64, 65, 1, 0] {
        let (bytes, provinces, pairs) = many_edge_pairs(count);
        for capacity in [count, count + 1] {
            compare_reused_edge_scratch(&bytes, None, provinces, pairs, 68, capacity, &mut scratch);
        }
        let reference = EdgeTrace {
            bytes: &bytes,
            blocked: None,
            reads: std::cell::RefCell::new(Vec::new()),
        };
        let _ = fresh_scratch_reference_edges(
            &reference,
            provinces,
            cprovince::VTABLE_RVA.0,
            &mut ProvinceBatch::new(),
            pairs,
            68,
            &mut vec![edge::Row::default(); count],
            &mut Progress::default(),
        );
        for &(address, width) in reference.reads.borrow().iter() {
            for fault in [address, address + width - 1] {
                compare_reused_edge_scratch(
                    &bytes,
                    Some(fault),
                    provinces,
                    pairs,
                    68,
                    count,
                    &mut scratch,
                );
            }
        }
    }
}

#[test]
fn reused_edge_scratch_keeps_invalid_prefixes_and_overflow_independent_of_old_bytes() {
    let mut scratch = EdgeScratch::new();
    let (bytes, provinces, pairs) = many_edge_pairs(66);
    compare_reused_edge_scratch(&bytes, None, provinces, pairs, 68, 66, &mut scratch);
    for index in [0, 1, 63, 64, 65] {
        for neighbor in [0_u32, 68, u32::MAX, index as u32] {
            let (mut bytes, provinces, pairs) = many_edge_pairs(66);
            put(
                &mut bytes,
                pairs.data - MEMORY_BASE + index * edge::STRIDE,
                neighbor.to_le_bytes(),
            );
            compare_reused_edge_scratch(&bytes, None, provinces, pairs, 68, 66, &mut scratch);
        }
    }
    for index in [1, 64, 65, 66] {
        for offset in [0, cprovince::ID] {
            let (mut bytes, provinces, pairs) = many_edge_pairs(66);
            put(
                &mut bytes,
                0x4000 + index * 0x200 - MEMORY_BASE + offset,
                0_u64.to_le_bytes(),
            );
            compare_reused_edge_scratch(&bytes, None, provinces, pairs, 68, 66, &mut scratch);
        }
    }
    let (bytes, provinces, pairs) = many_edge_pairs(1);
    compare_reused_edge_scratch(
        &bytes,
        Some(pairs.data),
        provinces,
        pairs,
        68,
        1,
        &mut scratch,
    );
    compare_reused_edge_scratch(
        &bytes,
        None,
        provinces,
        Array { count: 0, ..pairs },
        68,
        1,
        &mut scratch,
    );
    compare_reused_edge_scratch(
        &bytes,
        None,
        provinces,
        Array {
            data: usize::MAX - 4,
            count: 2,
        },
        68,
        2,
        &mut scratch,
    );
    compare_reused_edge_scratch(&bytes, None, provinces, pairs, 68, 0, &mut scratch);
}
