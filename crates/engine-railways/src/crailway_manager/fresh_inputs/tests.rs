use super::*;
use engine_native_memory::memory::SliceMemory;
use std::cell::RefCell;

const DATABASE: usize = 0x1000;
const DEFINITION: usize = 0x2000;
const TEMPLATES: usize = 0x3000;
const STATES: usize = 0x10000;
const BUILDINGS: usize = 0x60000;
const VECTORS: usize = 0x90000;

struct Fixture {
    bytes: Vec<u8>,
    blocked: Option<usize>,
    reads: RefCell<Vec<(usize, usize)>>,
    fallback: i32,
}

impl Fixture {
    fn new() -> Self {
        Self {
            bytes: vec![0; 0x100000],
            blocked: None,
            reads: Default::default(),
            fallback: 5,
        }
    }
    fn put(&mut self, at: usize, value: impl AsRef<[u8]>) {
        let value = value.as_ref();
        self.bytes[at..at + value.len()].copy_from_slice(value);
    }
    fn array(&mut self, at: usize, data: usize, count: u32) {
        self.put(at, (data as u64).to_le_bytes());
        self.put(at + 8, count.to_le_bytes());
        self.put(at + 12, count.to_le_bytes());
    }
    fn state(&mut self, index: usize) -> cstate::Source {
        let pointer = STATES + index * 0x900;
        let status = pointer + cstate::BUILDING_STATUS;
        let building = BUILDINGS + index * 0x220;
        let vectors = VECTORS + index * 0x20;
        self.put(pointer, (cstate::VTABLE_RVA.0 as u64).to_le_bytes());
        self.put(pointer + cstate::ID, (index as u32 + 1).to_le_bytes());
        self.put(status, (status::VTABLE_RVA.0 as u64).to_le_bytes());
        self.array(status + status::BUILDING_INDICES, vectors, 2);
        self.put(vectors + 4, 0_i32.to_le_bytes());
        self.array(status + status::BUILDINGS, vectors + 8, 1);
        self.put(vectors + 8, (building as u64).to_le_bytes());
        self.put(building, (building::VTABLE_RVA.0 as u64).to_le_bytes());
        self.put(
            building + building::BUILDING_STATUS,
            (status as u64).to_le_bytes(),
        );
        self.put(
            building + building::TEMPLATE,
            (DEFINITION as u64).to_le_bytes(),
        );
        self.put(building + building::TYPE_ID, 71_u32.to_le_bytes());
        self.put(building + building::LEVEL, 4_i16.to_le_bytes());
        self.put(building + building::HEALTHY_LEVEL, 2_i16.to_le_bytes());
        cstate::Source {
            pointer,
            id: index as u32 + 1,
        }
    }
    fn infrastructure(&mut self) {
        self.put(DATABASE + 0x3b0, (DEFINITION as u64).to_le_bytes());
        self.array(DATABASE + database::TEMPLATES, TEMPLATES, 2);
        self.put(TEMPLATES + 8, (DEFINITION as u64).to_le_bytes());
        self.put(DEFINITION + template::LOOKUP_ENABLED_CANDIDATE, [1]);
        self.put(DEFINITION + template::DATABASE_ORDINAL, 1_u32.to_le_bytes());
        self.put(DEFINITION + template::TYPE_ID, 71_u32.to_le_bytes());
        self.put(DEFINITION + 0x2bc, 4_i32.to_le_bytes());
        self.put(DEFINITION + 0x2b8, 8_i32.to_le_bytes());
    }
    fn rail(&mut self, index: usize, level: i16, healthy: i16) -> RailwaySource {
        let info = 0x4000 + index * 0x80;
        let building = BUILDINGS + index * 0x220;
        let status = 0x50000 + index * 0x400;
        self.put(info + info::BUILDING, (building as u64).to_le_bytes());
        self.put(building, (building::VTABLE_RVA.0 as u64).to_le_bytes());
        self.put(
            building + building::BUILDING_STATUS,
            (status as u64).to_le_bytes(),
        );
        self.put(building + building::LEVEL, level.to_le_bytes());
        self.put(building + building::HEALTHY_LEVEL, healthy.to_le_bytes());
        self.put(
            building + building::PARTIAL_HEALTH,
            5_000_000_i64.to_le_bytes(),
        );
        RailwaySource {
            province: index as u32 + 1,
            info,
            building_status: status,
            cooldown: Ok(0),
            pairs: Ok(Array {
                data: 0xa0000,
                count: 1,
            }),
            selection: None,
            carried: false,
        }
    }
}

impl Memory for Fixture {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, output.len()));
        if self
            .blocked
            .is_some_and(|at| address <= at && at - address < output.len())
        {
            if let Some(first) = output.first_mut() {
                *first = 0xff;
            }
            return false;
        }
        if address == database::SINGLETON_RVA.0 && output.len() == 8 {
            output.copy_from_slice(&(DATABASE as u64).to_le_bytes());
            return true;
        }
        if address == 0x033e_5274 && output.len() == 4 {
            output.copy_from_slice(&self.fallback.to_le_bytes());
            return true;
        }
        SliceMemory {
            base: 0,
            bytes: &self.bytes,
        }
        .copy(address, output)
    }
}

#[test]
fn fresh_state_fraction_uses_exact_native_maximum_and_signed_healthy_level() {
    let mut memory = Fixture::new();
    memory.infrastructure();
    let source = memory.state(0);
    let game_state = 0xb0000;
    memory.array(game_state + engine_native::cgame_state::STATES, 0xb1000, 2);
    memory.put(0xb1008, (source.pointer as u64).to_le_bytes());
    let mut output = [StateFractionRow::default(); 1];
    for (preferred, fallback, global, healthy, expected) in [
        (4_i32, 8_i32, 5_i32, 2_i16, 50_000_i64),
        (0, 8, 5, 2, 25_000),
        (0, -2, 5, 2, 40_000),
        (0, 0, -5, -2, 40_000),
        (0, 0, 0, 2, 0xffff_ffff),
    ] {
        memory.put(DEFINITION + 0x2bc, preferred.to_le_bytes());
        memory.put(DEFINITION + 0x2b8, fallback.to_le_bytes());
        memory.fallback = global;
        memory.put(BUILDINGS + building::HEALTHY_LEVEL, healthy.to_le_bytes());
        assert_eq!(
            state_fractions(&memory, 0, &[source], &mut output),
            Outcome::Rows { len: 1 }
        );
        assert_eq!(output[0].fraction, Ok(expected));
        assert_eq!(
            output[0].fraction,
            super::super::eta_state_infrastructure(&memory, 0, game_state, source.pointer)
        );
    }
}

#[test]
fn fresh_state_absence_and_disabled_lookup_do_not_read_maximum() {
    let mut memory = Fixture::new();
    memory.infrastructure();
    let sources: Vec<_> = (0..4).map(|i| memory.state(i)).collect();
    memory.blocked = Some(DEFINITION + 0x2bc);
    memory.array(
        STATES + 0x900 + cstate::BUILDING_STATUS + status::BUILDING_INDICES,
        0,
        0,
    );
    memory.put(VECTORS + 2 * 0x20 + 4, (-1_i32).to_le_bytes());
    memory.put(VECTORS + 3 * 0x20 + 8, 0_u64.to_le_bytes());
    let mut output = [StateFractionRow::default(); 4];
    assert_eq!(
        state_fractions(&memory, 0, &sources, &mut output),
        Outcome::Rows { len: 4 }
    );
    assert!(output[0].fraction.is_err());
    assert!(output[1..].iter().all(|row| row.fraction == Ok(0)));
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|(at, _)| *at == DEFINITION + 0x2bc)
            .count(),
        1
    );
    memory.put(DEFINITION + template::LOOKUP_ENABLED_CANDIDATE, [0]);
    memory.reads.borrow_mut().clear();
    state_fractions(&memory, 0, &sources, &mut output);
    assert!(output.iter().all(|row| row.fraction == Ok(0)));
    assert!(
        !memory
            .reads
            .borrow()
            .iter()
            .any(|(at, _)| *at == DEFINITION + 0x2bc)
    );
}

#[test]
fn fresh_state_capacity_faults_keep_independent_rows_in_each_window() {
    let mut memory = Fixture::new();
    memory.infrastructure();
    let sources: Vec<_> = (0..66).map(|i| memory.state(i)).collect();
    let mut output = vec![StateFractionRow::default(); 66];
    memory.blocked = Some(STATES + cstate::BUILDING_STATUS + status::BUILDING_INDICES);
    assert_eq!(
        state_fractions(&memory, 0, &sources, &mut output),
        Outcome::Rows { len: 66 }
    );
    assert_eq!(
        output[0].fraction,
        Err("ETA infrastructure status unreadable")
    );
    assert!(output[1..].iter().all(|row| row.fraction == Ok(50_000)));
    memory.blocked = None;
    assert_eq!(
        state_fractions(&memory, 0, &sources, &mut output[..1]),
        Outcome::Truncated {
            len: 1,
            unread_rows: 65
        }
    );
    assert_eq!(output[0].fraction, Ok(50_000));
    assert_eq!(
        state_fractions(&memory, 0, &sources, &mut []),
        Outcome::Truncated {
            len: 0,
            unread_rows: 66
        }
    );
    state_fractions(&memory, 0, &[], &mut output);
    assert!(output.iter().all(|row| *row == StateFractionRow::default()));
}

#[test]
fn fresh_state_each_gather_fault_preserves_prefix_and_independent_rows_without_retry() {
    let mut memory = Fixture::new();
    memory.infrastructure();
    let sources: Vec<_> = (0..66).map(|i| memory.state(i)).collect();
    let mut output = vec![StateFractionRow::default(); sources.len()];
    let fields = |index: usize| {
        let status = sources[index].pointer + cstate::BUILDING_STATUS;
        let vectors = VECTORS + index * 0x20;
        let building = BUILDINGS + index * 0x220;
        [
            (status, "ETA infrastructure status unreadable"),
            (
                status + status::BUILDING_INDICES,
                "ETA infrastructure status unreadable",
            ),
            (vectors + 4, "ETA infrastructure index unreadable"),
            (
                status + status::BUILDINGS,
                "ETA infrastructure buildings unreadable",
            ),
            (
                vectors + 8,
                "ETA infrastructure building pointer unreadable",
            ),
            (building, "ETA infrastructure building unreadable"),
            (
                building + building::BUILDING_STATUS,
                "ETA infrastructure building unreadable",
            ),
            (
                building + building::TEMPLATE,
                "ETA infrastructure building unreadable",
            ),
            (
                building + building::TYPE_ID,
                "ETA infrastructure building unreadable",
            ),
            (
                building + building::HEALTHY_LEVEL,
                "ETA infrastructure building unreadable",
            ),
        ]
    };
    for failed_row in [0, 31, 63] {
        for (failed_field, (fault, reason)) in fields(failed_row).into_iter().enumerate() {
            memory.blocked = Some(fault);
            memory.reads.borrow_mut().clear();
            assert_eq!(
                state_fractions(&memory, 0, &sources, &mut output),
                Outcome::Rows { len: sources.len() }
            );
            for (index, row) in output.iter().enumerate() {
                let expected = if index == failed_row {
                    Err(reason)
                } else {
                    Ok(50_000)
                };
                assert_eq!(row.fraction, expected, "fault {fault:#x}, state {index}");
                for (field_index, (at, _)) in fields(index).into_iter().enumerate() {
                    let attempts = memory
                        .reads
                        .borrow()
                        .iter()
                        .filter(|(read, _)| *read == at)
                        .count();
                    let expected = usize::from(index != failed_row || field_index <= failed_field);
                    assert_eq!(attempts, expected, "fault {fault:#x}, read {at:#x}");
                }
            }
        }
    }
}

#[test]
fn fresh_state_authentication_and_shared_control_faults_are_not_hidden_by_retry() {
    let mut memory = Fixture::new();
    memory.infrastructure();
    let sources: Vec<_> = (0..2).map(|i| memory.state(i)).collect();
    let mut output = [StateFractionRow::default(); 2];
    memory.blocked = Some(DEFINITION + template::DATABASE_ORDINAL);
    state_fractions(&memory, 0, &sources, &mut output);
    assert!(output.iter().all(|row| row.fraction.is_err()));
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|(at, _)| *at == DEFINITION + template::DATABASE_ORDINAL)
            .count(),
        1
    );
    memory.blocked = None;
    memory.put(BUILDINGS + building::TEMPLATE, 0_u64.to_le_bytes());
    state_fractions(&memory, 0, &sources, &mut output);
    assert!(output[0].fraction.is_err());
    assert_eq!(output[1].fraction, Ok(50_000));
}

#[test]
fn fresh_rail_health_keeps_absence_unreached_and_damaged_domains_distinct() {
    let mut memory = Fixture::new();
    let mut sources = [
        memory.rail(0, 4, 2),
        memory.rail(1, 0, 2),
        memory.rail(2, -2, -2),
        memory.rail(3, 4, 4),
    ];
    memory.put(sources[3].info + info::BUILDING, 0_u64.to_le_bytes());
    let mut output = [RailwayBuildingRow::default(); 4];
    assert_eq!(
        railway_buildings(&memory, 0, &sources, &mut output),
        Outcome::Rows { len: 4 }
    );
    let first = output[0].building.unwrap().unwrap().building.unwrap();
    assert_eq!(first.partial, Some(5_000_000));
    assert_eq!(building_efficiency(first), Ok(62_500));
    assert_eq!(
        output[1]
            .building
            .unwrap()
            .unwrap()
            .building
            .unwrap()
            .partial,
        None
    );
    assert_eq!(
        output[2]
            .building
            .unwrap()
            .unwrap()
            .building
            .unwrap()
            .partial,
        None
    );
    assert_eq!(
        output[3].building,
        Ok(Some(RailwayBuildingState { building: None }))
    );
    for index in [1, 2] {
        let partial = BUILDINGS + index * 0x220 + building::PARTIAL_HEALTH;
        memory.blocked = Some(partial);
        memory.reads.borrow_mut().clear();
        railway_buildings(&memory, 0, &sources, &mut output);
        assert!(output[index].building.is_ok());
        assert!(
            !memory
                .reads
                .borrow()
                .iter()
                .any(|(at, len)| *at <= partial && partial - *at < *len)
        );
    }
    sources[0].cooldown = Ok(1);
    sources[0].pairs = Err("pairs failed but cooldown excludes it");
    sources[1].pairs = Ok(Array::default());
    memory.blocked = Some(sources[0].info + info::BUILDING);
    railway_buildings(&memory, 0, &sources, &mut output);
    assert_eq!(output[0].building, Ok(None));
    assert_eq!(output[1].building, Ok(None));
}

#[test]
fn fresh_rail_optional_fault_preserves_independent_rows_and_full_capacity_demand() {
    let mut memory = Fixture::new();
    let sources: Vec<_> = (0..65).map(|i| memory.rail(i, 4, 2)).collect();
    let mut output = vec![RailwayBuildingRow::default(); 65];
    memory.blocked = Some(BUILDINGS + building::PARTIAL_HEALTH);
    railway_buildings(&memory, 0, &sources, &mut output);
    assert_eq!(
        output[0].building,
        Err("fresh railway partial health unreadable")
    );
    assert!(output[1..].iter().all(|row| row.building.is_ok()));
    memory.blocked = None;
    memory.put(BUILDINGS + building::BUILDING_STATUS, 0_u64.to_le_bytes());
    railway_buildings(&memory, 0, &sources, &mut output);
    assert!(output[0].building.is_err());
    assert!(output[1].building.is_ok());
    assert_eq!(
        railway_buildings(&memory, 0, &sources, &mut output[..1]),
        Outcome::Truncated {
            len: 1,
            unread_rows: 64
        }
    );
    railway_buildings(&memory, 0, &[], &mut output);
    assert!(
        output
            .iter()
            .all(|row| *row == RailwayBuildingRow::default())
    );
}

#[test]
fn fresh_rail_each_gather_fault_preserves_prefix_and_independent_rows_without_retry() {
    use super::super::selected_inputs::SelectionRow;
    let mut memory = Fixture::new();
    let sources: Vec<_> = (0..65)
        .map(|i| {
            let mut source = memory.rail(i, 4, 2);
            source.selection = Some(SelectionRow {
                pair_count: Ok(1),
                ..Default::default()
            });
            source
        })
        .collect();
    let mut output = vec![RailwayBuildingRow::default(); sources.len()];
    let fields = |index: usize| {
        let building = BUILDINGS + index * 0x220;
        [
            (
                sources[index].info + info::BUILDING,
                "fresh railway building pointer unreadable",
            ),
            (building, "fresh railway building health unreadable"),
            (
                building + building::BUILDING_STATUS,
                "fresh railway building health unreadable",
            ),
            (
                building + building::LEVEL,
                "fresh railway building health unreadable",
            ),
            (
                building + building::HEALTHY_LEVEL,
                "fresh railway building health unreadable",
            ),
            (
                building + building::PARTIAL_HEALTH,
                "fresh railway partial health unreadable",
            ),
        ]
    };
    let healthy = Ok(Some(RailwayBuildingState {
        building: Some(RailwayBuildingHealth {
            level: 4,
            healthy: 2,
            partial: Some(5_000_000),
        }),
    }));
    for failed_row in [0, 31, 63] {
        for (failed_field, (fault, reason)) in fields(failed_row).into_iter().enumerate() {
            memory.blocked = Some(fault);
            memory.reads.borrow_mut().clear();
            assert_eq!(
                railway_buildings(&memory, 0, &sources, &mut output),
                Outcome::Rows { len: sources.len() }
            );
            for (index, row) in output.iter().enumerate() {
                let expected = if index == failed_row {
                    Err(reason)
                } else {
                    healthy
                };
                assert_eq!(row.building, expected, "fault {fault:#x}, province {index}");
                let selected = if index == failed_row && failed_field < 4 {
                    Err(reason)
                } else {
                    Ok(Some(Some(4)))
                };
                assert_eq!(row.selection.unwrap().building_level, selected);
                for (field_index, (at, _)) in fields(index).into_iter().enumerate() {
                    let attempts = memory
                        .reads
                        .borrow()
                        .iter()
                        .filter(|(read, _)| *read == at)
                        .count();
                    let expected = usize::from(index != failed_row || field_index <= failed_field);
                    assert_eq!(attempts, expected, "fault {fault:#x}, read {at:#x}");
                }
            }
        }
    }
}

#[test]
fn selected_railway_building_union_keeps_native_level_null_and_unreached_gates() {
    use super::super::selected_inputs::{LevelSpan, SelectionRow};
    let mut memory = Fixture::new();
    let mut source = memory.rail(0, -3, 2);
    source.pairs = Ok(Array::default());
    source.selection = Some(SelectionRow {
        levels: Ok(LevelSpan {
            positive: true,
            ..Default::default()
        }),
        pair_count: Ok(0),
        ..Default::default()
    });
    memory.blocked = Some(BUILDINGS + building::HEALTHY_LEVEL);
    let mut output = [RailwayBuildingRow::default()];
    railway_buildings(&memory, 0, &[source], &mut output);
    assert_eq!(output[0].building, Ok(None));
    assert_eq!(
        output[0].selection.unwrap().building_level,
        Ok(Some(Some(-3)))
    );
    assert!(
        !memory
            .reads
            .borrow()
            .iter()
            .any(|&(at, len)| at <= BUILDINGS + building::HEALTHY_LEVEL
                && BUILDINGS + building::HEALTHY_LEVEL - at < len)
    );
    memory.blocked = None;
    memory.put(source.info + info::BUILDING, 0_u64.to_le_bytes());
    railway_buildings(&memory, 0, &[source], &mut output);
    assert_eq!(output[0].selection.unwrap().building_level, Ok(Some(None)));
    for cooldown in [-1, 1] {
        source.cooldown = Ok(cooldown);
        source.selection.as_mut().unwrap().pair_count = Err("unreached count");
        source.selection.as_mut().unwrap().levels = Err("unreached levels");
        memory.blocked = Some(source.info + info::BUILDING);
        railway_buildings(&memory, 0, &[source], &mut output);
        assert_eq!(output[0].selection.unwrap().building_level, Ok(None));
    }
}

#[test]
fn fresh_rail_arithmetic_preserves_native_branch_laziness_and_signed_values() {
    let health = RailwayBuildingState {
        building: Some(RailwayBuildingHealth {
            level: 4,
            healthy: 2,
            partial: Some(5_000_000),
        }),
    };
    assert_eq!(
        directed_percentage(-1, Some(7), Some(health), 5),
        Ok(87_500)
    );
    assert_eq!(directed_percentage(1, Some(7), None, 5), Ok(0));
    assert_eq!(directed_percentage(0, None, None, 5), Ok(0));
    assert_eq!(
        directed_percentage(0, Some(-2), Some(health), 5),
        Ok(-25_000)
    );
    assert_eq!(directed_percentage(0, Some(7), Some(health), 0), Ok(0));
    assert!(directed_percentage(0, Some(7), None, 0).is_err());
    assert!(
        building_efficiency(RailwayBuildingHealth {
            level: 4,
            healthy: 2,
            partial: None
        })
        .is_err()
    );
    assert_eq!(
        building_efficiency(RailwayBuildingHealth {
            level: 0,
            healthy: 2,
            partial: None
        }),
        Ok(0)
    );
    assert_eq!(
        road_or_rail_speed(50_000, 87_500, [200_000, 1_000_000, 100_000, 1_000_000]),
        975_000
    );
}
