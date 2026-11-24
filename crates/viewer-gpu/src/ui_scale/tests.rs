use super::*;
use crate::test_support::ui_scale::*;

#[test]
fn manual_and_unavailable_display_keep_direct_raster_and_input_scale() {
    assert_eq!(
        Setting::Manual(1.5).resolve_raster(Err("unused"), Err("unused")),
        (Raster::direct(1.5), None)
    );
    let (raster, note) = Setting::MatchGame.resolve_raster(Err("missing"), Ok(&[]));
    assert_eq!(raster, Raster::direct(1.));
    assert!(note.is_some());
    let (raster, _) =
        Setting::MatchGame.resolve_raster(Ok(&borderless_at(1.2)), Ok(&[Ok(MONITOR)]));
    let screen_point = [200., 150.];
    let frame_point = screen_point.map(|point| point / raster.stretch);
    let hit = to_interface(frame_point, raster.ui_scale);
    let direct = to_interface(screen_point, raster.screen_scale());
    assert!((0..2).all(|axis| (hit[axis] - direct[axis]).abs() < 1e-4));
}

#[test]
fn matching_the_game_multiplies_its_scale_by_its_stretch() {
    let (scale, note) = Setting::MatchGame.resolve(Ok(&borderless()), Ok(&[Ok(MONITOR)]));
    assert_eq!((scale, note), (1.25, None));
    let unscaled = Desktop { dpi: Ok(96), ..MONITOR };
    assert_eq!(
        Setting::MatchGame.resolve(Ok(&borderless()), Ok(&[Ok(unscaled.clone())])),
        (1.0, None)
    );
    let (scale, _) =
        Setting::MatchGame.resolve(Ok(&borderless_at(1.2)), Ok(&[Ok(unscaled)]));
    assert_eq!(scale, 2560.0 / 2133.0);
    let tv = Desktop {
        physical: [3840, 2160],
        dpi: Ok(192),
    };
    let dpi_err = || {
        Err("Could not read the display scale of monitor DISPLAY2 (0x80070057)".into())
    };
    let dpi_failed_tv = Desktop {
        physical: [3840, 2160],
        dpi: dpi_err(),
    };
    let on_unavailable = Display {
        display_index: 1,
        ..borderless()
    };
    let (scale, note) = Setting::MatchGame.resolve(
        Ok(&on_unavailable),
        Ok(&[Ok(MONITOR), Ok(dpi_failed_tv.clone()), Ok(tv.clone())]),
    );
    assert_eq!(scale, 1.0);
    assert!(
        note.as_deref()
            .is_some_and(|text| text.contains("DISPLAY2") && text.contains("0x80070057")),
        "{note:?}"
    );
    let on_third = Display {
        display_index: 2,
        ..borderless()
    };
    assert_eq!(
        Setting::MatchGame.resolve(
            Ok(&on_third),
            Ok(&[Ok(MONITOR), Ok(dpi_failed_tv.clone()), Ok(tv.clone())]),
        ),
        (2.0, None)
    );
    let on_secondary_without_primary = Display {
        display_index: 1,
        clamps: Vec::new(),
        ..borderless()
    };
    let (raster, note) = Setting::MatchGame.resolve_raster(
        Ok(&on_secondary_without_primary),
        Ok(&[Err("primary DPI failed".into()), Ok(tv)]),
    );
    assert_eq!(note, None);
    assert_eq!(raster.screen_scale(), 2.0);
    let fullscreen_dpi_failed = Display {
        clamps: vec![content_reader_settings::settings::Clamp {
            gui_scale: Some(1.0),
            size: Some([1920, 1080]),
            borderless: false,
        }],
        size: Some([1920, 1080]),
        mode: content_reader_settings::settings::DisplayMode::Fullscreen,
        display_index: 1,
        ..borderless()
    };
    let (raster, note) = Setting::MatchGame.resolve_raster(
        Ok(&fullscreen_dpi_failed),
        Ok(&[Ok(MONITOR), Ok(dpi_failed_tv)]),
    );
    assert_eq!(note, None);
    assert_eq!(raster.screen_scale(), 2.0);
}

#[test]
fn missing_game_settings_fall_back_to_one_with_a_note() {
    let (scale, note) = Setting::MatchGame.resolve(Err("Missing graphics block"), Ok(&[]));
    assert_eq!(scale, 1.0);
    assert!(note.unwrap().contains("Missing graphics block"));
    let (scale, note) = Setting::MatchGame.resolve(Ok(&borderless()), Err("not DPI aware"));
    assert_eq!(scale, 1.0);
    assert!(note.unwrap().contains("not DPI aware"));
    let (scale, note) = Setting::MatchGame.resolve(Ok(&borderless()), Ok(&[]));
    assert_eq!(scale, 1.0);
    assert!(note.is_some());
}

#[test]
fn a_manual_scale_ignores_the_game_and_invalid_stored_values_match_it() {
    assert_eq!(
        Setting::Manual(1.5).resolve(Err("unused"), Err("unused")),
        (1.5, None)
    );
    assert_eq!(Setting::from_stored(Some(1.5)), Setting::Manual(1.5));
    assert_eq!(Setting::from_stored(None), Setting::MatchGame);
    assert_eq!(Setting::from_stored(Some(0.1)), Setting::MatchGame);
    assert_eq!(Setting::from_stored(Some(f32::NAN)), Setting::MatchGame);
}

