use collector_land_events::land_events::test_support::{self, exclusive, reset};

#[test]
fn refused_detach_stops_land_production() {
    let _guard = exclusive();
    reset();
    let mut retained = false;
    let result = crate::runtime::refuse_land_detach_for_test(|| {
        assert!(
            !test_support::production_enabled(),
            "stop before retaining collector state"
        );
        retained = true;
    });
    assert_eq!(result, Err("synthetic land detach refused"));
    assert!(retained);
    let lost = test_support::thread_lost();
    let required = test_support::required_words();
    test_support::observe_while_disabled();
    assert!(!test_support::production_enabled());
    assert_eq!(test_support::thread_lost(), lost);
    assert_eq!(test_support::required_words(), required);
}
