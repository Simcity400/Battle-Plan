use super::*;
use engine_native::{cgame_state, memory::SliceMemory};
use std::cell::{Cell, RefCell};

struct Trace<'a> {
    bytes: &'a [u8],
    spans: RefCell<Vec<(usize, usize)>>,
}
impl Memory for Trace<'_> {
    fn copy(&self, at: usize, output: &mut [u8]) -> bool {
        self.spans.borrow_mut().push((at, output.len()));
        SliceMemory {
            base: 0,
            bytes: self.bytes,
        }
        .copy(at, output)
    }
}

fn doctrine_fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x6000];
    let mut put = |at: usize, value: &[u8]| bytes[at..at + value.len()].copy_from_slice(value);
    for (field, pointer, count) in [
        (cgame_state::COUNTRIES, 0x1000_u64, 4_u32),
        (cgame_state::COUNTRY_LINKS, 0x1100, 8),
    ] {
        put(field, &pointer.to_le_bytes());
        put(field + 8, &count.to_le_bytes());
        put(field + 12, &count.to_le_bytes());
    }
    for (slot, pointer) in [(1, 0x2000_u64), (2, 0x2000), (3, 0x3000)] {
        put(0x1000 + slot * 8, &pointer.to_le_bytes());
    }
    for (raw, canonical) in [(1, 1_u32), (2, 1), (7, 1), (3, 3)] {
        put(0x1100 + raw * 4, &canonical.to_le_bytes());
    }
    for (pointer, id, status) in [(0x2000, 7_u32, 0x4000_u64), (0x3000, 3, 0x5000)] {
        put(pointer, &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes());
        put(pointer + ccountry::ID, &id.to_le_bytes());
        put(pointer + ccountry::TECHNOLOGY_STATUS, &status.to_le_bytes());
        put(
            status as usize,
            &(ctechnology_status::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            status as usize + ctechnology_status::COUNTRY,
            &(pointer as u64).to_le_bytes(),
        );
    }
    put(
        0x4000 + ctechnology_status::LAND_DOCTRINE_LEVEL,
        &(-19_i32).to_le_bytes(),
    );
    bytes
}

#[test]
fn doctrine_reads_empty_province_controller_and_memos_aliases_without_inactive_reads() {
    let bytes = doctrine_fixture();
    let memory = Trace {
        bytes: &bytes,
        spans: RefCell::default(),
    };
    let flags = [
        cstate::FlagRow {
            state: 1,
            border_conflict_active: 1,
            ..Default::default()
        },
        cstate::FlagRow {
            state: 2,
            ..Default::default()
        },
    ];
    let provinces = [
        cprovince::Row {
            id: 1,
            state: 1,
            controller: 2,
            ..Default::default()
        },
        cprovince::Row {
            id: 2,
            state: 1,
            controller: 7,
            ..Default::default()
        },
        cprovince::Row {
            id: 3,
            state: 2,
            controller: 3,
            ..Default::default()
        },
    ];
    let mut rows = [
        ccountry::Row {
            id: 7,
            canonical_slot: 1,
            ..Default::default()
        },
        ccountry::Row {
            id: 3,
            canonical_slot: 3,
            ..Default::default()
        },
    ];
    doctrine(
        &memory,
        0,
        0,
        &provinces,
        &flags,
        &mut rows,
        Outcome::Rows { len: 2 },
    )
    .unwrap();
    assert_eq!(rows[0].border_land_doctrine_level, Some(-19));
    assert_eq!(rows[1].border_land_doctrine_level, None);
    let spans = memory.spans.borrow();
    assert_eq!(
        spans
            .iter()
            .filter(
                |&&(at, len)| at == 0x4000 + ctechnology_status::LAND_DOCTRINE_LEVEL && len == 4
            )
            .count(),
        1
    );
    assert!(
        !spans
            .iter()
            .any(|&(at, _)| at == 0x3000 + ccountry::TECHNOLOGY_STATUS || at >= 0x5000)
    );
}

#[test]
fn selected_doctrine_fault_preserves_missing_value() {
    let mut bytes = doctrine_fixture();
    bytes[0x4000 + ctechnology_status::COUNTRY] = 1;
    let flags = [cstate::FlagRow {
        state: 1,
        border_conflict_active: 1,
        ..Default::default()
    }];
    let provinces = [cprovince::Row {
        id: 1,
        state: 1,
        controller: 2,
        ..Default::default()
    }];
    let mut rows = [ccountry::Row {
        id: 7,
        canonical_slot: 1,
        ..Default::default()
    }];
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    assert!(
        doctrine(
            &memory,
            0,
            0,
            &provinces,
            &flags,
            &mut rows,
            Outcome::Rows { len: 1 }
        )
        .unwrap_err()
        .contains("backlink")
    );
    assert_eq!(rows[0].border_land_doctrine_level, None);
}

#[test]
fn incomplete_membership_is_refused_before_scalar_reads() {
    let flags = [cstate::FlagRow {
        state: 1,
        border_conflict_active: 1,
        ..Default::default()
    }];
    let rows = [cprovince::Row {
        id: 1,
        state: 2,
        ..Default::default()
    }];
    assert!(provinces(&rows, Outcome::Rows { len: 1 }, &flags).is_err());
    assert!(
        provinces(
            &rows,
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            },
            &flags
        )
        .is_err()
    );
    assert!(land(&[(99, 1)], &rows).is_err());
}

struct Inputs<'a> {
    bytes: &'a [u8],
    fault: Option<usize>,
    reads: Cell<usize>,
}
impl Memory for Inputs<'_> {
    fn copy(&self, at: usize, output: &mut [u8]) -> bool {
        self.reads.set(self.reads.get() + 1);
        if Some(at) == self.fault {
            return false;
        }
        if at == engine_native::carmy::COUNTER_ATTRITION_THRESHOLD_RVA.0 {
            output.copy_from_slice(&500_i64.to_le_bytes());
            return true;
        }
        SliceMemory {
            base: 0,
            bytes: self.bytes,
        }
        .copy(at, output)
    }
}

fn seed() -> engine_native::carmy::DivisionSeed {
    engine_native::carmy::DivisionSeed {
        pointer: 0x8000,
        division: 11,
        primary_vtable: Some(engine_native::carmy::VTABLE_RVA.0 as u64),
        handle: Some(11),
        roster_owner: Some(2),
        expeditionary_owner: Some(0),
        orders_group: Some(0),
        stat_block: Some(0x7000),
        counter_attrition: Some(0),
        entrenchment: Some(0),
        entrenchment_cap: Some(0),
        counter_supply_status: Some(0),
        supply_grace_maximum: Some(0),
        supply_grace_remaining: Some(0),
        invalid_orders: Some(0),
        strength_factor: Some(0),
        exiled: Some(0),
        noncore_supply_bypass: Some(0),
        model_number: Some(0),
        ..Default::default()
    }
}

#[test]
fn real_border_capture_isolates_faults_and_preserves_incomplete_censuses() {
    use super::super::{Capacities, Captured};
    let mut bytes = doctrine_fixture();
    bytes.resize(0x9000, 0);
    bytes[0x7000..0x7008]
        .copy_from_slice(&(engine_native::csub_unit_definition::VTABLE_RVA.0 as u64).to_le_bytes());
    let memory = Inputs {
        bytes: &bytes,
        fault: None,
        reads: Cell::new(0),
    };
    let mut captured = Captured::with_capacities(&Capacities {
        division_presentation:
            collector_divisions::extended::division_presentation::Capacities::status_fixture(1),
        ..Default::default()
    });
    let provinces = [cprovince::Row {
        id: 1,
        state: 1,
        controller: 2,
        ..Default::default()
    }];
    for (active, flag_fault, fault, row_capacity) in [
        (0, false, None, 1),
        (1, true, None, 1),
        (
            1,
            false,
            Some(0x4000 + ctechnology_status::LAND_DOCTRINE_LEVEL),
            1,
        ),
        (
            1,
            false,
            Some(0x7000 + engine_native::csub_unit_definition::DEFENSE),
            1,
        ),
        (1, true, None, 0),
    ] {
        captured
            .division_presentation
            .capture_status_fixture(&memory, &[seed()], row_capacity);
        let status = captured.division_presentation.outcomes()[1];
        assert_eq!(
            status,
            if row_capacity == 0 {
                Outcome::Truncated {
                    len: 0,
                    unread_rows: 1,
                }
            } else {
                Outcome::Rows { len: 1 }
            }
        );
        captured.state_flags.install_border_fixture(
            vec![cstate::FlagRow {
                state: 1,
                border_conflict_active: active,
                ..Default::default()
            }],
            if flag_fault {
                Outcome::Unavailable {
                    reason: "state flag fixture fault",
                }
            } else {
                Outcome::Rows { len: 1 }
            },
        );
        captured
            .province_unit_order
            .install_land_fixture(vec![(1, 11)], Outcome::Rows { len: 1 });
        let mut countries = [ccountry::Row {
            id: 7,
            canonical_slot: 1,
            ..Default::default()
        }];
        let original = if row_capacity == 0 {
            Outcome::Truncated {
                len: 0,
                unread_rows: 1,
            }
        } else {
            Outcome::Rows { len: 1 }
        };
        let mut country_outcome = original;
        let guarded = Inputs {
            bytes: &bytes,
            fault,
            reads: Cell::new(0),
        };
        captured.capture_border_inputs(
            &guarded,
            0,
            0,
            (&provinces, Outcome::Rows { len: 1 }),
            (&mut countries, &mut country_outcome),
        );
        let stats = captured.division_presentation.outcomes()[1];
        if row_capacity == 0 {
            assert_eq!(country_outcome, original);
            assert_eq!(stats, status);
        } else if active == 0 {
            assert_eq!(guarded.reads.get(), 0);
            assert_eq!(country_outcome, original);
            assert_eq!(stats, status);
        } else if flag_fault {
            assert_eq!(
                country_outcome,
                Outcome::Unavailable {
                    reason: "state flag fixture fault"
                }
            );
            assert_eq!(stats, country_outcome);
        } else {
            assert_eq!(
                matches!(country_outcome, Outcome::Unavailable { .. }),
                fault == Some(0x4000 + ctechnology_status::LAND_DOCTRINE_LEVEL)
            );
            assert_eq!(
                matches!(stats, Outcome::Unavailable { .. }),
                fault == Some(0x7000 + engine_native::csub_unit_definition::DEFENSE)
            );
            if matches!(country_outcome, Outcome::Rows { .. }) {
                assert_eq!(countries[0].border_land_doctrine_level, Some(-19));
            }
        }
        captured.division_presentation.decode_inputs();
        assert_eq!(captured.division_presentation.outcomes()[1], stats);
    }
    captured
        .division_presentation
        .capture_status_fixture(&memory, &[seed()], 1);
    captured.province_unit_order.install_land_fixture(
        vec![],
        Outcome::Unavailable {
            reason: "land fixture fault",
        },
    );
    let mut countries = [ccountry::Row {
        id: 7,
        canonical_slot: 1,
        ..Default::default()
    }];
    let mut outcome = Outcome::Rows { len: 1 };
    captured.state_flags.install_border_fixture(
        vec![cstate::FlagRow {
            state: 1,
            border_conflict_active: 1,
            ..Default::default()
        }],
        Outcome::Rows { len: 1 },
    );
    captured.capture_border_inputs(
        &memory,
        0,
        0,
        (&provinces, Outcome::Rows { len: 1 }),
        (&mut countries, &mut outcome),
    );
    assert_eq!(outcome, Outcome::Rows { len: 1 });
    assert_eq!(
        captured.division_presentation.outcomes()[1],
        Outcome::Unavailable {
            reason: "land fixture fault"
        }
    );
}
