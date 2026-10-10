use super::*;
use installed_content::settings::{Clamp, DisplayMode};

#[test]
fn manual_and_unavailable_display_keep_direct_raster_and_input_scale() {
    assert_eq!(
        Setting::Manual(1.5).resolve_raster(Err("unused"), Err("unused")),
        (Raster::direct(1.5), None)
    );
    let (raster, note) = Setting::MatchGame.resolve_raster(Err("missing"), Ok(&[]));
    assert_eq!(raster, Raster::direct(1.));
    assert!(note.is_some());
    let (raster, _) = Setting::MatchGame.resolve_raster(Ok(&borderless_at(1.2)), Ok(&[MONITOR]));
    let screen_point = [200., 150.];
    let frame_point = screen_point.map(|point| point / raster.stretch);
    let hit = to_interface(frame_point, raster.ui_scale);
    let direct = to_interface(screen_point, raster.screen_scale());
    assert!((0..2).all(|axis| (hit[axis] - direct[axis]).abs() < 1e-4));
}

/// The owner's borderless game on a 2560x1440 monitor at 125%.
pub(crate) fn borderless() -> Display {
    borderless_at(1.0)
}

fn borderless_at(gui_scale: f32) -> Display {
    Display {
        clamps: vec![Clamp {
            gui_scale: Some(gui_scale),
            size: Some([2048, 1152]),
            borderless: true,
        }],
        size: Some([2560, 1440]),
        min_gui: [1280, 720],
        mode: DisplayMode::Borderless,
        display_index: 0,
    }
}
pub(crate) const MONITOR: Desktop = Desktop {
    physical: [2560, 1440],
    dpi: 120,
};

#[test]
fn matching_the_game_multiplies_its_scale_by_its_stretch() {
    let (scale, note) = Setting::MatchGame.resolve(Ok(&borderless()), Ok(&[MONITOR]));
    assert_eq!((scale, note), (1.25, None));
    // At 100% the same game fills the monitor 1:1.
    let unscaled = Desktop { dpi: 96, ..MONITOR };
    assert_eq!(
        Setting::MatchGame.resolve(Ok(&borderless()), Ok(&[unscaled])),
        (1.0, None)
    );
    let (scale, _) = Setting::MatchGame.resolve(Ok(&borderless_at(1.2)), Ok(&[unscaled]));
    // 2560 frame pixels over 2133 interface pixels, truncated as the game does.
    assert_eq!(scale, 2560.0 / 2133.0);
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
    assert_eq!(Setting::Manual(2.0).stored(), Some(2.0));
}

#[test]
fn a_click_hits_exactly_what_the_scaled_quad_covers() {
    // A combat bubble laid out at (100, 100) 40 x 20 interface pixels.
    let hit = crate::combat::Hit {
        selection: crate::combat::Selection::Land(7),
        rect: [100.0, 100.0, 40.0, 20.0],
        layer: 0,
        order: 0,
        supported: true,
    };
    let [x0, y0, x1, y1] = crate::map::sprites::device_corners(hit.rect, 1.25);
    assert_eq!([x0, y0, x1, y1], [125.0, 125.0, 175.0, 150.0]);
    let click = |device: [f32; 2]| hit.contains(to_interface(device, 1.25));
    assert!(click([x0, y0]));
    assert!(click([x1 - 0.5, y1 - 0.5]));
    assert!(!click([x1, y0]));
    assert!(!click([x0, y1]));
    assert!(!click([x0 - 0.5, y0]));
    // Unscaled, the same device point that hits at 1.25 misses the bubble.
    assert!(!hit.contains([170.0, 140.0]));
    assert!(click([170.0, 140.0]));
}

#[test]
fn tooltip_lengths_follow_the_ui_scale_alone_whatever_the_display_scale() {
    // 14 interface pixels are 17.5 device pixels at 1.25 on a 100%, 150%
    // or 200% Windows display, as a 14-pixel sprite would be.
    for pixels_per_point in [1.0, 1.5, 2.0] {
        let device = points(14.0, 1.25, pixels_per_point) * pixels_per_point;
        assert!((device - 17.5).abs() < 1e-4, "{pixels_per_point}");
    }
}

#[test]
fn device_points_divide_into_interface_pixels() {
    assert_eq!(to_interface([1280.0, 720.0], 1.25), [1024.0, 576.0]);
    assert_eq!(extent([2560, 1440], 1.25), [2048.0, 1152.0]);
    assert_eq!(extent([2048, 1152], 1.0), [2048.0, 1152.0]);
}
