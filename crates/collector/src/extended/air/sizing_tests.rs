use super::*;

#[test]
fn cold_air_capacity_waits_for_live_demand() {
    let capacity = Capacities::default();
    assert_eq!(capacity.rows, [0; SIZED]);
    assert_eq!([capacity.ace_names, capacity.wings], [0; 2]);
    assert_eq!(
        [
            capacity.scores.rows,
            capacity.scores.regions,
            capacity.scores.countries
        ],
        [0; 3]
    );
    let job = carrier::Job::new();
    assert_eq!(job.index.len(), 0);
    assert_eq!(job.bases.rows.len(), 0);
    assert_eq!(job.registrations.rows.len(), 0);
    assert_eq!(job.scratch.capacity(), [0; 5]);
    assert_eq!(job.log.borrow().spans.capacity(), 0);
    assert_eq!(job.log.borrow().bytes.capacity(), 0);
}

#[test]
fn growing_patrol_rows_and_region_scratch_publish_5000_country_combat_occurrences() {
    let count = 5_000usize;
    let mut scratch: Vec<u64> = (1..=count).map(|slot| (slot as u64) << 32 | 7).collect();
    let mut rows = Vec::new();
    let (outcome, (first, regions)) = patrol_combats_buffer(
        Buffer::Growing(&mut scratch),
        count,
        Buffer::Growing(&mut rows),
        |slot, emit| {
            for occurrence in 0..9 {
                emit(engine_air_history::air_kamikaze::TaskForceCombat {
                    province: slot + occurrence,
                    region: 7,
                })?;
            }
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(outcome, Outcome::Rows { len: 45_000 });
    assert_eq!((first, regions), (count, 1));
    assert_eq!(scratch[first], 7);
    assert_eq!(rows[44_999].country, 5_000);
    assert_eq!(rows[44_999].ordinal, 8);
    assert_eq!(rows[44_999].province, 5_008);
}
