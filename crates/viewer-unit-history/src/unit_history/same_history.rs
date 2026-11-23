use super::*;

pub(super) fn assert_instances(a: &[(usize, ModelInstance)], b: &[(usize, ModelInstance)]) {
    assert_eq!(a.len(), b.len());
    for ((a_mesh, a), (b_mesh, b)) in a.iter().zip(b) {
        assert_eq!(a_mesh, b_mesh);
        assert_eq!(bytemuck::bytes_of(a), bytemuck::bytes_of(b));
    }
}

pub(super) fn assert_history(a: &History, b: &History) {
    assert_eq!(a.targets, b.targets);
    assert_eq!(a.bounds, b.bounds);
    assert_eq!(a.steady, b.steady);
    assert_instances(&a.raids, &b.raids);
    assert_eq!(a.provinces, b.provinces);
    assert_eq!(a.provinces.populated_ids(), b.provinces.populated_ids());
    assert_eq!(a.updates, b.updates);
    assert_eq!(a.queued, b.queued);
    assert_eq!(a.queue_trace, b.queue_trace);
    assert_eq!(a.epoch, b.epoch);
    assert_eq!(a.last.map(f32::to_bits), b.last.map(f32::to_bits));
    assert_eq!(a.raid_epoch, b.raid_epoch);
    assert_eq!(a.province_bounds, b.province_bounds);
    a.naval.assert_same_state(&b.naval);
    assert_eq!(a.combat_clocks(), b.combat_clocks());
    assert_eq!(a.combat_warnings(), b.combat_warnings());
    let a_schedule = a.schedule.lock().unwrap();
    let b_schedule = b.schedule.lock().unwrap();
    assert_eq!(
        a_schedule.presentation_state(),
        b_schedule.presentation_state()
    );
    assert_eq!(a_schedule.pending(), b_schedule.pending());
    for &id in a.provinces.keys() {
        assert_eq!(a_schedule.update(id), b_schedule.update(id));
        assert_eq!(a_schedule.dirty(id), b_schedule.dirty(id));
    }
}

