use super::*;

#[test]
fn province_binding_restarts_delayed_body_but_hour_refresh_does_not() {
    let start = Instant::now();
    let rect = [0.0, 0.0, 800.0, 600.0];
    let at = [20.0, 20.0];
    let point = Some(at);
    let timing = [0.05, 1.0, 0.05];
    let mut hover = HoverState::default();
    hover.bind_province(Some(7));
    hover.shown(
        Some(("Forest", Some("details"), None, rect)),
        point,
        start,
        timing,
    );
    hover.bind_province(Some(7));
    assert_eq!(
        hover.shown(
            Some(("Storm", Some("details"), None, rect)),
            point,
            start + Duration::from_millis(1001),
            timing
        ),
        Some(("Storm\n--------------\ndetails", at))
    );
    hover.bind_province(Some(8));
    assert_eq!(
        hover.shown(
            Some(("Hills", Some("details"), None, rect)),
            point,
            start + Duration::from_millis(1002),
            timing
        ),
        Some(("Hills", at))
    );
    hover.bind_province(None);
    assert_eq!(
        hover.shown(
            Some(("Raid", Some("raid body"), None, rect)),
            point,
            start + Duration::from_millis(1003),
            timing
        ),
        Some(("Raid", at))
    );
}

#[test]
fn tooltip_edge_reversal_uses_the_pointer_and_twenty_pixel_gap() {
    assert_eq!(
        position([100.9, 200.1], [300.0, 80.0], [1000.0, 600.0], [0; 4]),
        [121.0, 224.0]
    );
    assert_eq!(
        position([900.0, 550.0], [300.0, 80.0], [1000.0, 600.0], [0; 4]),
        [580.0, 450.0]
    );
    assert_eq!(
        position([5.0, 5.0], [300.0, 80.0], [100.0, 50.0], [0; 4]),
        [0.0, 0.0]
    );
}

const TIMING: [f32; 3] = [0.05, 1.0, 0.05];
const RECT: [f32; 4] = [1.0, 2.0, 3.0, 4.0];
const POINT: Option<[f32; 2]> = Some([2.0, 3.0]);

#[test]
fn naval_tooltip_width_uses_authored_fallback_and_counts_missing_or_wrong_types() {
    content_reader_script::test_game::enter();
    use crate::test_support::gui::{fixture, fixture_with};
    let read = content_reader_gui::gui::native::read;
    let fallback = fixture_with(read(
        br#"guiTypes={textBoxType={name=defaultTextBox maxWidth=375}}"#,
    ));
    assert_eq!(authored_wrap_width(&fallback).unwrap(), 375.);
    let preferred = fixture_with(read(
        br#"guiTypes={textBoxType={name=ToolTip maxWidth=500}
            textBoxType={name=defaultTextBox maxWidth=375}}"#,
    ));
    assert_eq!(authored_wrap_width(&preferred).unwrap(), 500.);
    let zero = fixture_with(read(br#"guiTypes={textBoxType={name=ToolTip maxWidth=0}}"#));
    assert_eq!(authored_wrap_width(&zero).unwrap(), 0.);

    let wrong = fixture_with(read(
        br#"guiTypes={containerWindowType={name=ToolTip size={width=500 height=20}}
            textBoxType={name=defaultTextBox maxWidth=375}}"#,
    ));
    for (resources, reason) in [
        (fixture(), "ToolTip and defaultTextBox are absent"),
        (wrong, "tooltip template is not a textBoxType"),
    ] {
        let mut batch = Batch::default();
        let refused = draw(
            &resources,
            &mut batch,
            "ship",
            TooltipPosition {
                point: [700., 300.],
                anchor: Some(Explicit::NavalWindow([600., 100., 900., 650.])),
            },
            [2400., 900.],
            &TooltipDefines {
                offsets: [0; 4],
                timing: [0.; 3],
            },
            0.,
        );
        assert_eq!(refused.len(), 1, "{refused:?}");
        let (text, count) = refused.first_key_value().unwrap();
        assert!(
            text.contains("authored tooltip wrap width is unavailable"),
            "{text}"
        );
        assert!(text.contains(reason), "{text}");
        assert_eq!(*count, 1);
        assert!(batch.quads.is_empty());
    }
}

#[test]
fn naval_ship_tooltip_anchors_to_the_free_window_side_at_y_plus_250() {
    let window = [600., 100., 900., 650.];
    assert_eq!(naval_ship_anchor(window, 700., 500., 2400.), [100., 350.]);
    assert_eq!(naval_ship_anchor(window, 1200., 500., 2400.), [1500., 350.]);
    assert_eq!(naval_ship_anchor(window, 1200., 500., 1800.), [100., 350.]);
    assert_eq!(
        naval_ship_anchor([500., -300., 900., 600.], 600., 500., 2400.),
        [1400., 0.]
    );
    let at = TooltipPosition {
        point: [700., 500.],
        anchor: Some(Explicit::NavalWindow(window)),
    };
    assert_eq!(
        placed(at, [420., 100.], [2400., 900.], [0; 4], 500.),
        [100., 350.]
    );
    assert_eq!(
        placed(at, [420., 700.], [2400., 900.], [0; 4], 500.),
        [100., 0.]
    );
    let right = TooltipPosition {
        point: [1200., 500.],
        ..at
    };
    assert_eq!(
        placed(right, [500., 100.], [2400., 900.], [0; 4], 500.),
        [1500., 350.]
    );
}

#[test]
fn authored_anchor_takes_no_pointer_offset_and_flips_by_size_plus_twenty() {
    let at = |below| TooltipPosition {
        point: [1750., 1010.],
        anchor: Some(Explicit::At { at: [1542., 1000.], below }),
    };
    assert_eq!(placed(at(true), [300., 60.], [2400., 1100.], [0; 4], 500.), [1542., 1000.]);
    assert_eq!(placed(at(true), [300., 200.], [2400., 1100.], [0; 4], 500.), [1542., 780.]);
    assert_eq!(placed(at(true), [900., 60.], [2400., 1100.], [0; 4], 500.), [622., 1000.]);
    assert_eq!(placed(at(false), [300., 60.], [2400., 1100.], [0; 4], 500.), [1542., 940.]);
    assert_eq!(placed(at(false), [300., 60.], [2400., 1100.], [1600, 0, 0, 0], 500.), [1600., 940.]);
}

#[test]
fn hover_uses_strict_show_and_delayed_deadlines() {
    let start = Instant::now();
    let mut hover = HoverState::default();
    let hit = Some(("Header", Some("Body"), None, RECT));
    assert_eq!(hover.shown(hit, POINT, start, TIMING), None);
    assert!(hover.next_change(start, TIMING).is_some());
    assert_eq!(
        hover.shown(hit, POINT, start + Duration::from_millis(50), TIMING),
        None
    );
    assert_eq!(
        hover.shown(hit, POINT, start + Duration::from_millis(51), TIMING),
        Some(("Header", POINT.unwrap()))
    );
    assert_eq!(
        hover.shown(hit, POINT, start + Duration::from_secs(1), TIMING),
        Some(("Header", POINT.unwrap()))
    );
    assert_eq!(
        hover.shown(hit, POINT, start + Duration::from_millis(1001), TIMING),
        Some(("Header\n--------------\nBody", POINT.unwrap()))
    );
    assert_eq!(
        hover.next_change(start + Duration::from_millis(1001), TIMING),
        None
    );
}

#[test]
fn shortcut_only_hover_waits_for_show_delay_and_tail_follows_delayed_body() {
    let start = Instant::now();
    let shortcut = "Shortcut: \u{13}ESCAPE";
    let mut hover = HoverState::default();
    let hit = Some(("", None, Some(shortcut), RECT));
    assert_eq!(hover.shown(hit, POINT, start, TIMING), None);
    assert_eq!(
        hover.shown(hit, POINT, start + Duration::from_millis(50), TIMING),
        None
    );
    assert_eq!(
        hover.shown(hit, POINT, start + Duration::from_millis(51), TIMING),
        Some((shortcut, POINT.unwrap()))
    );
    let hit = Some(("Main", Some("Body"), Some(shortcut), RECT));
    assert_eq!(
        hover.shown(hit, POINT, start + Duration::from_millis(51), TIMING),
        Some((
            "Main\n--------------\nShortcut: \u{13}ESCAPE",
            POINT.unwrap()
        ))
    );
    assert_eq!(
        hover.shown(hit, POINT, start + Duration::from_millis(1001), TIMING),
        Some((
            "Main\n--------------\nBody\n--------------\nShortcut: \u{13}ESCAPE",
            POINT.unwrap()
        ))
    );
    assert_eq!(
        hover.shown(
            Some(("Next", None, None, RECT)),
            POINT,
            start + Duration::from_millis(1002),
            TIMING
        ),
        Some(("Next", POINT.unwrap()))
    );
}

#[test]
fn hover_preserves_authored_whitespace_in_tooltip_parts() {
    let clock = Instant::now();
    let start = clock + Duration::from_secs(1);
    let mut hover = HoverState::default();
    hover.shown(None, None, clock, [0.0; 3]);
    let hit = Some((" \n", Some(" x "), None, RECT));
    assert_eq!(hover.shown(hit, POINT, start, [0.0; 3]), None);
    assert_eq!(
        hover.shown(hit, POINT, start + Duration::from_millis(1), [0.0; 3]),
        Some((" \n\n--------------\n x ", POINT.unwrap()))
    );
}

#[test]
fn hovering_new_values_keeps_clock_but_a_new_binding_restarts_delayed_text() {
    let start = Instant::now();
    let mut hover = HoverState::default();
    hover.shown(
        Some(("10", Some("details"), None, RECT)),
        POINT,
        start,
        TIMING,
    );
    hover.shown(
        Some(("10", Some("details"), None, RECT)),
        POINT,
        start + Duration::from_millis(51),
        TIMING,
    );
    assert_eq!(
        hover.shown(
            Some(("11", Some("updated"), None, RECT)),
            POINT,
            start + Duration::from_millis(1001),
            TIMING
        ),
        Some(("11\n--------------\nupdated", POINT.unwrap()))
    );
    let next_rect = [5.0, 2.0, 3.0, 4.0];
    assert_eq!(
        hover.shown(
            Some(("New", Some("later"), None, next_rect)),
            POINT,
            start + Duration::from_millis(1100),
            TIMING
        ),
        Some(("New", POINT.unwrap()))
    );
}

#[test]
fn hide_delay_retains_last_text_and_reentry_cancels_hiding() {
    let start = Instant::now();
    let mut hover = HoverState::default();
    let hit = Some(("Header", None, None, RECT));
    hover.shown(hit, POINT, start, TIMING);
    hover.shown(hit, POINT, start + Duration::from_millis(51), TIMING);
    assert_eq!(
        hover.shown(None, None, start + Duration::from_millis(100), TIMING),
        Some(("Header", POINT.unwrap()))
    );
    assert_eq!(
        hover.shown(None, None, start + Duration::from_millis(150), TIMING),
        Some(("Header", POINT.unwrap()))
    );
    assert_eq!(
        hover.shown(hit, POINT, start + Duration::from_millis(150), TIMING),
        Some(("Header", POINT.unwrap()))
    );
    assert_eq!(
        hover.next_change(start + Duration::from_millis(150), TIMING),
        None
    );
    hover.shown(None, None, start + Duration::from_millis(200), TIMING);
    assert_eq!(
        hover.shown(None, None, start + Duration::from_millis(251), TIMING),
        None
    );
}

#[test]
fn negative_delays_follow_the_native_comparisons() {
    let defines = Defines::new().unwrap();
    defines
        .load(
            "common/defines/00_defines.lua",
            b"NDefines = {
                NInterface = { TOOLTIP_SCREEN_LEFT_OFFSET_X = 0, TOOLTIP_SCREEN_TOP_OFFSET_Y = 0,
                    TOOLTIP_SCREEN_RIGHT_OFFSET_X = 0, TOOLTIP_SCREEN_BOTTOM_OFFSET_Y = 0 },
                NGraphics = { TOOLTIP_SHOW_DELAY = -1, TOOLTIP_DELAYED_DELAY = -0.5, TOOLTIP_HIDE_DELAY = 0.05 } }",
        )
        .unwrap();
    let timing = TooltipDefines::read(&defines).unwrap().timing;
    assert_eq!(timing, [-1.0, -0.5, 0.05]);
    let clock = Instant::now();
    let start = clock + Duration::from_secs(2);
    let mut hover = HoverState::default();
    hover.shown(None, None, clock, timing);
    let hit = Some(("Header", Some("Body"), None, RECT));
    assert_eq!(
        hover.shown(hit, POINT, start, timing),
        Some(("Header\n--------------\nBody", POINT.unwrap()))
    );
    assert_eq!(hover.next_change(start, timing), None);
    let mut hover = HoverState::default();
    let nan = [0.0, f32::NAN, 0.05];
    hover.shown(None, None, clock, nan);
    hover.shown(hit, POINT, start, nan);
    assert_eq!(
        hover.shown(hit, POINT, start + Duration::from_secs(60), nan),
        Some(("Header", POINT.unwrap()))
    );
    assert_eq!(
        hover.next_change(start + Duration::from_secs(60), nan),
        None
    );
}

#[test]
fn a_deadline_at_or_below_zero_on_the_presentation_clock_is_unset() {
    let clock = Instant::now();
    let at = |millis| clock + Duration::from_millis(millis);
    let timing = [-1.0, 1.0, -1.0];
    let mut hover = HoverState::default();
    let hit = Some(("Header", None, None, RECT));
    assert_eq!(hover.shown(hit, POINT, clock, timing), None);
    assert_eq!(hover.shown(hit, POINT, at(500), timing), None);
    let next = hover.next_change(at(500), timing).unwrap();
    assert!(
        next > Duration::from_millis(499) && next < Duration::from_millis(501),
        "{next:?}"
    );
    assert_eq!(
        hover.shown(hit, POINT, at(1100), timing),
        Some(("Header", POINT.unwrap()))
    );
    assert_eq!(hover.shown(None, None, at(1200), timing), None);
    let timing = [0.05, 1.0, -1.0];
    let mut hover = HoverState::default();
    hover.shown(None, None, clock, timing);
    hover.shown(hit, POINT, at(100), timing);
    assert!(hover.shown(hit, POINT, at(200), timing).is_some());
    assert!(hover.shown(None, None, at(300), timing).is_some());
    assert_eq!(hover.shown(None, None, at(1100), timing), None);
}

#[test]
fn delayed_only_tooltip_waits_for_body_then_show_delay_without_separator() {
    let start = Instant::now();
    let mut hover = HoverState::default();
    let hit = Some(("", Some("Details"), None, RECT));
    assert_eq!(hover.shown(hit, POINT, start, TIMING), None);
    assert_eq!(
        hover.shown(hit, POINT, start + Duration::from_millis(1001), TIMING),
        None
    );
    assert_eq!(
        hover.shown(hit, POINT, start + Duration::from_millis(1052), TIMING),
        Some(("Details", POINT.unwrap()))
    );
}
