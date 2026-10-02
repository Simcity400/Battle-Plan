use super::*;

fn captured() -> Captured {
    Captured::with_capacities(&Capacities {
        rows: [0, 0, 0, 0, 0, 4, 4],
        radar_provinces: 0,
        area_provinces: 0,
        wings: 8,
    })
}

#[test]
fn registrations_memo_validates_ranges_and_preserves_required_capacity() {
    let mut source = captured();
    source.registrations_raw_mode = true;
    source.registrations.outcome = Outcome::Rows { len: 1 };
    source.registrations.rows[0] = cair_base::RegistrationRow {
        base: 12,
        slot: 3,
        capacity_raw: 7,
        wings_start: 0,
        wings_len: 2,
    };
    source.wings.first[..2].copy_from_slice(&[100, 200]);
    source.wings.len = 2;
    source.wings.required = 5;
    assert!(source.registrations_memo_complete());
    let mut memo = RegistrationsMemo::default();
    source.save_registrations_memo(&mut memo);
    let mut target = captured();
    target.registrations_raw_mode = true;
    assert_eq!(target.registrations_memo_context(), Some([4, 8]));
    target.restore_registrations_memo(&memo);
    assert_eq!(target.registrations.outcome, Outcome::Rows { len: 1 });
    assert_eq!(target.registrations.rows[0], source.registrations.rows[0]);
    assert_eq!(target.wings.len, 2);
    assert_eq!(target.wings.required, 5);
    assert_eq!(&target.wings.first[..2], &[100, 200]);
    assert!(!target.registrations_raw_mode);
    source.registrations.rows[0].wings_start = usize::MAX;
    assert!(!source.registrations_memo_complete());
    source.registrations.rows[0].wings_start = 0;
    source.wings.required = 1;
    assert!(!source.registrations_memo_complete());
    source.wings.required = 5;
    source.wings.len = 9;
    assert!(!source.registrations_memo_complete());
}

#[test]
fn registration_demand_growth_is_accounted_and_propagated_to_pooled_capacities() {
    use crate::extended::air::air_bases_test_kit as kit;
    let bytes = kit::fixture();
    let (_, _, job) = kit::split(&kit::memory(&bytes));
    for initial in [0, 1] {
        let mut capacities = Capacities {
            rows: [0, 0, 0, 0, 0, initial, 0],
            radar_provinces: 0,
            area_provinces: 0,
            wings: initial,
        };
        let mut captured = Captured::with_capacities(&capacities);
        captured.registrations_raw_mode = true;
        let before = captured.allocated_bytes();
        captured.decode_registrations_payload(kit::BASE, &job, &mut [0; 3]);
        assert_eq!(captured.registrations.outcome, Outcome::Rows { len: 2 });
        assert_eq!((captured.wings.len, captured.wings.required), (2, 2));
        assert!(captured.allocated_bytes() > before);
        assert_eq!(
            captured.allocated_bytes(),
            captured.registrations.rows.capacity() * size_of::<cair_base::RegistrationRow>()
                + captured.wings.first.capacity() * size_of::<u64>()
        );
        captured.grow(&mut capacities);
        assert_eq!(capacities.rows[5], captured.registrations.rows.len());
        assert_eq!(capacities.wings, captured.wings.first.len());
        assert!(capacities.rows[5] >= 2 && capacities.wings >= 2);
        let next = Captured::with_capacities(&capacities);
        assert_eq!(
            next.registrations.rows.len(),
            captured.registrations.rows.len()
        );
        assert_eq!(next.wings.first.len(), captured.wings.first.len());
    }
}
