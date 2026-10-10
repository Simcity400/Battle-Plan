use super::*;
use crate::raw_memory::{Capacity, Charge, Tracker, vector_bytes};

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
    job.transport_raw_bytes = 4;
    job.clear();
    assert_eq!(
        (job.transport.required_map_capacity, job.transport.failures),
        (0, 0)
    );
    assert_eq!((job.transport_ticks, job.transport_raw_bytes), (0, 0));
    assert_eq!(LOCAL.snapshot().current, grown);
    let mut moved = Some(job);
    let job = moved.take().unwrap();
    assert_eq!(LOCAL.snapshot().current, grown);
    drop(job);
    assert_eq!(LOCAL.snapshot().current, Capacity::default());
}
