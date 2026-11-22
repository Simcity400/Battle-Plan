use super::*;
use collector_primitives::raw_memory::{Capacity, Charge, Tracker, vector_bytes};

#[test]
fn zero_limit_split_parts_prepare_all_wing_field_certificates_on_their_first_census() {
    use super::super::wing_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let ctx = AirWingCopyContext {
        base: kit::BASE,
        game_state: kit::GAME_STATE,
        hour: 0,
        force_full: true,
        limits: [0; 3],
        game_score: false,
    };
    let mut census = AirWingJob::part();
    census.slots = 0..1;
    census.split = true;
    let mut parts = [AirWingJob::part()];
    parts[0].slots = 1..u32::MAX;
    parts[0].split = true;
    for job in std::iter::once(&mut census).chain(&mut parts) {
        assert!(copy_air_refresh(&memory, ctx, job).raw_ok);
    }
    assert_eq!(census.census.shared.map.len(), 0);
    assert_eq!(parts[0].census.shared.map.len(), 2);
    collector_test_support::with_allocations_denied(|| {
        assert!(parts[0].map_flags.ensure_capacity(2));
    });
    census.join_parts(&mut parts).unwrap();
    assert_eq!(census.map_outcome, Outcome::Rows { len: 2 });
    assert_eq!(census.map_rows[0].id, 0x4500_0001);
}

#[test]
fn transport_capacity_charge_survives_clear_and_move_then_releases_on_drop() {
    static LOCAL: Tracker = Tracker::new();
    let mut job = AirWingJob {
        map_rows: vec![cair_wing::Row::default(); 3],
        sources: vec![cair_wing::Source::default(); 1],
        transport: cair_wing::transport_operands::Scratch::with_capacity(1).unwrap(),
        capacity_charge: Charge::empty_on(&LOCAL),
        ..Default::default()
    };
    job.update_capacity_charge();
    let initial = LOCAL.snapshot().current;
    assert_eq!(initial.payload_vectors, vector_bytes(&job.map_rows));
    assert_eq!(
        initial.auxiliary_vectors,
        vector_bytes(&job.sources) + job.transport.allocated_bytes() as u64
    );
    assert!(
        job.grow_for_next(),
        "undersized optional arenas request off-thread growth"
    );
    job.transport.grow(job.map_rows.len()).unwrap();
    job.update_capacity_charge();
    let grown = LOCAL.snapshot().current;
    assert!(grown.auxiliary_vectors > initial.auxiliary_vectors);
    job.transport.required_map_capacity = 9;
    job.transport.failures = 2;
    job.transport_ticks = 3;
    job.optional_raw_bytes = 4;
    job.clear();
    assert_eq!(
        (job.transport.required_map_capacity, job.transport.failures),
        (0, 0)
    );
    assert_eq!((job.transport_ticks, job.optional_raw_bytes), (0, 0));
    assert_eq!(LOCAL.snapshot().current, grown);
    let mut moved = Some(job);
    let job = moved.take().unwrap();
    assert_eq!(LOCAL.snapshot().current, grown);
    drop(job);
    assert_eq!(LOCAL.snapshot().current, Capacity::default());
}
