use crate::test_support::movement_timing::terrain;

#[test]
fn terrain_uses_first_registered_class_plus_common_and_native_fallback() {
    let mut terrain = terrain(
        "categories={custom_water={naval_terrain=no units={movement=0.1} custom_hull={units={movement=0.2}} custom_hull={units={movement=0.8}} ghost_hull={units={movement=2}}} ocean={units={movement=-0.4}}}",
    );
    assert_eq!(terrain.movement(9, || Ok(Some("CUSTOM_HULL"))), Ok(30_000));
    assert_eq!(terrain.movement(9, || Ok(Some("ghost_hull"))), Ok(10_000));
    assert_eq!(
        terrain.movement(11, || Err("unreached class".into())),
        Ok(-40_000)
    );
    assert!(
        terrain
            .movement(99, || Ok(None))
            .unwrap_err()
            .contains("no content template")
    );
    terrain.subunits = Err("sub-unit domain refused".into());
    assert_eq!(terrain.movement(9, || Ok(Some("no_bucket"))), Ok(10_000));
    assert!(terrain.movement(9, || Ok(Some("custom_hull"))).is_err());
}

#[test]
fn missing_ocean_is_native_null_terrain_with_zero_adjuster() {
    let terrain = terrain("categories={custom_water={units={movement=0.2}}}");
    assert_eq!(
        terrain.movement(11, || Err("unreached class".into())),
        Ok(0)
    );
}
