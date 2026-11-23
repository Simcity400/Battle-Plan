use collector_land_events::land_events::{self, test_support};

#[test]
fn refused_detach_stops_land_production() {
    let (_family, mut owned) = test_support::exclusive();
    // SAFETY: This isolated fixture exclusively owns observation and installs no native hooks.
    unsafe { test_support::reset(&mut owned) };
    assert!(owned.production_enabled());
    let mut retained = false;
    let result = crate::runtime::refuse_land_detach_for_test(|| {
        assert!(
            !owned.production_enabled(),
            "stop before retaining collector state"
        );
        retained = true;
    });
    assert_eq!(result, Err("synthetic land detach refused"));
    assert!(retained);
    let lost = test_support::lost();
    let required = owned.required_words();
    owned.observe_while_disabled();
    assert!(!owned.production_enabled());
    assert_eq!(test_support::lost(), lost);
    assert_eq!(owned.required_words(), required);
    // SAFETY: This isolated fixture owns teardown and has no installed native hooks.
    unsafe { land_events::release() }.unwrap();
}
