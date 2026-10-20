use super::*;

#[test]
fn handle_only_witnesses_take_the_roster_and_others_their_own_fields() {
    let manager = [AirBaseRow {
        id: 10,
        state: Some(7),
        province: Some(9),
        capacity: 5,
        ..Default::default()
    }];
    let witnesses = [
        AirReferenceWitnessRow {
            handle: 10,
            ..Default::default()
        },
        AirReferenceWitnessRow {
            handle: 11,
            base_kind: Some(0),
            host: Some(77),
            host_owner: Some(3),
            ..Default::default()
        },
        AirReferenceWitnessRow {
            handle: 12,
            base_kind: Some(0),
            host: Some(78),
            host_without_task_force: true,
            ..Default::default()
        },
    ];
    let sources = AirRegistrySources::from_rows(&witnesses, Some(&manager)).unwrap();
    assert_eq!(sources.bases[&10].capacity, 5);
    assert_eq!(sources.bases[&11].host, 77);
    assert!(sources.transfer_resolves(11));
    assert!(!sources.transfer_resolves(13));
    assert_eq!(
        [10, 11, 12].map(|base| sources.host_owner(base)),
        [Some(0), Some(3), None]
    );
    assert!(AirRegistrySources::from_rows(&witnesses, None).is_err());
}
