use collector_land_events::land_events::test_support::Fixture;

#[test]
fn refused_detach_stops_land_production() {
    let fixture = Fixture::new(1024);
    let mut retained = false;
    let result = crate::runtime::refuse_land_detach_for_test(&fixture, || {
        assert!(
            !fixture.production_enabled(),
            "stop before retaining collector state"
        );
        retained = true;
    });
    assert_eq!(result, Err("synthetic land detach refused"));
    assert!(retained);
    let lost = fixture.lost();
    let required = fixture.required_words();
    fixture.observe_while_disabled();
    assert!(!fixture.production_enabled());
    assert_eq!(fixture.lost(), lost);
    assert_eq!(fixture.required_words(), required);
}
