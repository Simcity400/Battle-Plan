use super::*;
use engine::{cgame_state, cstrategic_navy, cstrategic_navy_manager, memory::SliceMemory};

fn empty_navy(capture: &mut Captured, hour: Option<i32>) {
    let mut bytes = vec![0; 0x4000];
    let pointer = |bytes: &mut [u8], address: usize, value: usize| {
        bytes[address..address + 8].copy_from_slice(&(value as u64).to_le_bytes());
    };
    pointer(
        &mut bytes,
        0x1000 + cgame_state::STRATEGIC_NAVY_MANAGER,
        0x2000,
    );
    pointer(&mut bytes, 0x2000, cstrategic_navy_manager::VTABLE_RVA);
    pointer(&mut bytes, 0x2000 + cstrategic_navy_manager::NAVIES, 0x2100);
    bytes[0x2010..0x2014].copy_from_slice(&1_u32.to_le_bytes());
    bytes[0x2014..0x2018].copy_from_slice(&1_u32.to_le_bytes());
    pointer(&mut bytes, 0x2100, 0x3000);
    pointer(&mut bytes, 0x3000, cstrategic_navy::VTABLE_RVA);
    pointer(
        &mut bytes,
        0x3000 + cstrategic_navy::SECONDARY,
        cstrategic_navy::SECONDARY_VTABLE_RVA,
    );
    pointer(
        &mut bytes,
        0x3000 + engine::country_naval_movement_inputs::MANAGER,
        0x2000,
    );
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let mut movement = naval::Captured::new(naval::Capacities::default());
    movement.capture(
        &memory,
        0,
        0x1000,
        hour,
        naval::OwnerCensus {
            owners: [],
            failure: Some("unrelated moving-owner refusal"),
        },
        capture.naval_movement_mut(),
    );
}

#[test]
fn packet_projection_keeps_independent_vectors_and_refuses_without_hour() {
    let mut capture = Captured::vacant();
    capture.reserved.outcome = Outcome::Rows { len: 1 };
    capture.launch_state_result = Err("test state roster refusal");
    empty_navy(&mut capture, Some(71));
    let TableView::Complete(replay::rows::TableRows::ReservedCountryNaming(rows)) =
        capture.reserved_table_attack(None, None)
    else {
        panic!("healthy naming prefix");
    };
    let replay::rows::Recorded::Value(Some(packet)) = &rows[0].launch_sources else {
        panic!("current packet");
    };
    assert_eq!(packet.sample_hour, 71);
    assert_eq!(
        packet.naval_bases,
        replay::rows::LaunchNavalBases::Bases(vec![])
    );
    assert!(packet.owned_states.is_none());
    capture.launch_state_result = Ok(0);
    let packet = capture.launch_projection().unwrap().unwrap();
    assert_eq!(
        (packet.naval_bases, packet.owned_states),
        (replay::rows::LaunchNavalBases::Bases(vec![]), Some(vec![]))
    );
    empty_navy(&mut capture, None);
    assert!(capture.launch_projection().unwrap().is_none());
}

#[test]
fn healthy_internal_launch_cannot_publish_after_mandatory_naming_failure() {
    let mut capture = Captured::vacant();
    capture.launch_state_result = Ok(0);
    empty_navy(&mut capture, Some(71));
    assert!(capture.launch_projection().unwrap().is_some());
    capture.reserved.outcome = Outcome::Unavailable {
        reason: "test mandatory naming refusal",
    };
    assert!(matches!(
        capture.reserved_table_attack(None, None),
        TableView::Unavailable { .. }
    ));
}

#[test]
fn state_payload_and_scratch_grow_off_thread_from_the_full_refused_demand() {
    let mut capacity = Capacities {
        rows: 0,
        subjects: 0,
        ..Default::default()
    };
    let mut capture = Captured::with_capacities(&capacity);
    capture.launch_states.required = 3;
    capture.launch_state_scratch.required = 3;
    capture.launch_state_result = Err("test state arena exhausted");
    let before = capture.allocated_bytes();
    capture.grow(&mut capacity);
    assert!(capture.launch_states.first.len() >= 3);
    assert_eq!(
        capture.launch_state_scratch.first.len(),
        capture.launch_states.first.len()
    );
    assert!(
        capture.allocated_bytes()
            >= before
                + 3 * (std::mem::size_of::<u32>()
                    + std::mem::size_of::<ccountry::launch_sources::Occurrence>())
    );
    capture.reset();
    assert!(capture.reserved.launch_states.is_none());
    assert!(capture.launch_projection().unwrap().is_none());
}
