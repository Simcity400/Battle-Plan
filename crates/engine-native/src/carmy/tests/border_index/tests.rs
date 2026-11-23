use super::*;

#[test]
fn real_status_capture_builds_sorted_full_handle_border_index() {
    assert_eq!(carmy::StatusPacket::vacant().allocated_bytes(), 0);
    let memory = presentation_fixture();
    let mut first = [carmy::DivisionSeed {
        pointer: A1,
        division: presentation_source()[0].division,
        ..Default::default()
    }];
    carmy::capture_division_seeds(&memory, &mut first);
    let mut seeds = [first[0]; 2];
    let handles = [handle(9, 9000), handle(3, 9000)];
    for (seed, handle) in seeds.iter_mut().zip(handles) {
        seed.division = handle;
        seed.handle = Some(handle);
    }
    let capacities = carmy::StatusCapacities {
        membership: crate::corders_group_member::MembershipCapacity {
            queries: 2,
            groups: 1,
            ..Default::default()
        },
        modifiers: engine_native_parts::modifier_accumulator::SnapshotCapacity {
            nodes: 1,
            entries: 1,
        },
        ..carmy::StatusCapacities::new(2)
    };
    let mut packet = carmy::StatusPacket::with_capacities(capacities).unwrap();
    let floored = engine_native_parts::modifier_accumulator::SnapshotCapacity {
        nodes: 2,
        entries: 1,
    };
    let allocated = carmy::StatusPacket::with_capacities(carmy::StatusCapacities {
        modifiers: floored,
        ..capacities
    })
    .unwrap()
    .allocated_bytes();
    assert!(packet.allocated_bytes() < allocated);
    assert_eq!(
        packet.capture(&memory, 0, GS, &seeds, Outcome::Rows { len: 2 }),
        Outcome::Rows { len: 2 }
    );
    assert_eq!(packet.modifiers.capacity(), floored);
    assert_eq!(packet.allocated_bytes(), allocated);
    assert_eq!(packet.border_index, [(handles[1], 1), (handles[0], 0)]);
    assert_eq!(
        packet.capture_border_stats(&memory, 0, [handles[1]]),
        Outcome::Rows { len: 2 }
    );
    assert!(packet.units[0].border_combat_stats.is_none());
    assert_eq!(packet.units[1].border_combat_stats, Some([0; 3]));
    assert_eq!(packet.allocated_bytes(), allocated);
    let first = Outcome::Truncated {
        len: 1,
        unread_rows: 1,
    };
    assert_eq!(packet.capture(&memory, 0, GS, &seeds[..1], first), first);
    assert_eq!(packet.border_index, [(handles[0], 0)]);
    assert_eq!(
        packet.capture_border_stats(&memory, 0, [handles[1]]),
        Outcome::Truncated {
            len: 1,
            unread_rows: 1
        }
    );
    packet.fail_border_stats("later selection fault");
    assert_eq!(
        packet.outcome,
        Outcome::Truncated {
            len: 1,
            unread_rows: 1
        }
    );
}
