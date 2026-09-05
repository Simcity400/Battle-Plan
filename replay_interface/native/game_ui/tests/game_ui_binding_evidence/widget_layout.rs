use game_ui::compose::{measure_to_pixels_for_parent, ComposeOptions, Orientation, Rect};
use game_ui::gui::{GuiElement, LayoutSet};
use game_ui::widget_layout::{
    gridbox_layout, listbox_layout, scrollbar_layout, GridBoxFormat, GridBoxLayout, ListBoxLayout,
    ListFlow, ScrollbarAxis, ScrollbarFamily,
};
use std::path::{Path, PathBuf};

fn game_root() -> PathBuf {
    replay_test_gates::require_vanilla_game_root()
}

fn parse_layout(root: &Path, rel: &str) -> LayoutSet {
    let mut layouts = LayoutSet::default();
    layouts
        .parse_content(&map_viewer::GameFileResolver::vanilla(&root).unwrap().read_required_bytes(rel, "fixture content").unwrap(), rel)
        .expect("parse vanilla gui");
    layouts
}

fn find<'a>(layouts: &'a LayoutSet, name: &str, kind: &str) -> &'a GuiElement {
    layouts
        .find_by_name(name)
        .into_iter()
        .find(|element| element.element_type.eq_ignore_ascii_case(kind))
        .unwrap_or_else(|| panic!("{kind} {name} missing"))
}

#[test]
#[cfg_attr(
    not(gate_hoi4_install),
    ignore = "the game install (set BATTLE_PLAN_GAME_ROOT / BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA, or install at the default Steam path)"
)]
fn backend_listbox_declares_vertical_rows_from_file_template_size() {
    let root = game_root();
    let layouts = parse_layout(&root, "interface/backend.gui");
    let kings = find(&layouts, "kings", "listboxType");
    let row_template = find(&layouts, "king_entry", "windowType");
    let options = ComposeOptions::new(1024, 768);

    let layout = listbox_layout(kings, Rect::new(0, 0, 1024, 768), &options).expect("listbox");
    assert_eq!(layout.rect, Rect::new(262, 184, 485, 460));
    assert_eq!(layout.flow, ListFlow::Vertical);
    assert_eq!(layout.spacing, 0);
    assert_eq!(
        layout.scrollbar_type.as_deref(),
        Some("standardlistbox_slider")
    );

    let row_size = row_template.size.as_ref().expect("king_entry size");
    let row_size = (
        measure_to_pixels_for_parent(&row_size.x, layout.rect.w()),
        measure_to_pixels_for_parent(&row_size.y, layout.rect.h()),
    );
    assert_eq!(row_size, (460, 20));
    assert_eq!(
        layout.declared_row_rect(0, row_size).expect("row 0"),
        Rect::new(262, 184, 460, 20)
    );
    assert_eq!(
        layout.declared_row_rect(1, row_size).expect("row 1"),
        Rect::new(262, 204, 460, 20)
    );
}

#[test]
#[cfg_attr(
    not(gate_hoi4_install),
    ignore = "the game install (set BATTLE_PLAN_GAME_ROOT / BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA, or install at the default Steam path)"
)]
fn achievements_gridbox_slots_follow_declared_slot_size() {
    let root = game_root();
    let layouts = parse_layout(&root, "interface/achievements.gui");
    let list = find(&layouts, "list", "gridBoxType");
    let options = ComposeOptions::new(760, 470).with_root_size(760, 470);

    let layout = gridbox_layout(list, Rect::new(0, 0, 760, 470), &options).expect("gridbox");
    assert_eq!(layout.rect, Rect::new(10, 10, 760, 470));
    assert_eq!(layout.slot_size, (760, 95));
    assert_eq!(layout.max_slots_x, Some(1));
    assert_eq!(layout.max_slots_y, None);
    assert_eq!(
        layout.slot_rect(0).expect("slot 0"),
        Rect::new(10, 10, 760, 95)
    );
    assert_eq!(
        layout.slot_rect(1).expect("slot 1"),
        Rect::new(10, 105, 760, 95)
    );
}

#[test]
fn listbox_row_index_overflow_is_a_typed_refusal() {
    let layout = ListBoxLayout {
        rect: Rect::new(0, 0, 100, 100),
        orientation: Orientation::UpperLeft,
        flow: ListFlow::Vertical,
        spacing: 0,
        scrollbar_type: None,
        scrollbar_offset: (0, 0),
        scrollbar_side: None,
    };
    let err = layout
        .declared_row_rect(usize::MAX, (10, 20))
        .expect_err("index beyond i32 must refuse");
    assert!(err.message.contains("UNTRACED"));
    assert!(err.message.contains("overflows pixel space"));
    let err = layout
        .declared_row_rect(i32::MAX as usize, (10, 20))
        .expect_err("pixel overflow must refuse");
    assert!(err.message.contains("overflows pixel space"));
    assert_eq!(
        layout.declared_row_rect(2, (10, 20)).expect("row 2"),
        Rect::new(0, 40, 10, 20)
    );
}

#[test]
fn gridbox_slot_index_overflow_is_a_typed_refusal() {
    let layout = GridBoxLayout {
        rect: Rect::new(0, 0, 100, 100),
        format: GridBoxFormat::UpperLeft,
        slot_size: (10, 20),
        max_slots_x: Some(3),
        max_slots_y: None,
        add_horizontal: Some(false),
    };
    let err = layout
        .slot_rect(usize::MAX)
        .expect_err("index beyond i32 must refuse");
    assert!(err.message.contains("UNTRACED"));
    assert!(err.message.contains("overflows pixel space"));
    let err = layout
        .slot_rect(i32::MAX as usize)
        .expect_err("pixel overflow must refuse");
    assert!(err.message.contains("overflows pixel space"));
    assert_eq!(layout.slot_rect(4).expect("slot 4"), Rect::new(10, 20, 10, 20));
}

#[test]
fn production_variable_height_grid_accumulates_resolved_child_heights() {
    let layout = GridBoxLayout {
        rect: Rect::new(7, 11, 83, 300),
        format: GridBoxFormat::UpperLeft,
        slot_size: (83, 1),
        max_slots_x: Some(1),
        max_slots_y: None,
        add_horizontal: Some(false),
    };
    assert_eq!(
        layout
            .production_variable_height_slot_rects(&[(71, 17), (73, 29), (79, 31)])
            .expect("exact-a729 production path"),
        [
            Rect::new(7, 11, 71, 17),
            Rect::new(7, 28, 73, 29),
            Rect::new(7, 57, 79, 31),
        ]
    );
}

#[test]
fn production_variable_height_grid_uses_modded_authored_grid_values() {
    let layout = GridBoxLayout {
        rect: Rect::new(7, 11, 200, 100),
        format: GridBoxFormat::UpperLeft,
        slot_size: (10, 3),
        max_slots_x: Some(4),
        max_slots_y: None,
        add_horizontal: Some(true),
    };
    assert_eq!(
        layout
            .production_variable_height_slot_rects(&[(71, 17), (73, 29)])
            .expect("direct production coordinates use authored grid values"),
        [Rect::new(7, 11, 71, 17), Rect::new(7, 62, 73, 29)]
    );
}

#[test]
fn production_variable_height_grid_follows_all_engine_format_cases() {
    let mut layout = GridBoxLayout {
        rect: Rect::new(7, 11, 200, 100),
        format: GridBoxFormat::Up,
        slot_size: (10, 3),
        max_slots_x: None,
        max_slots_y: None,
        add_horizontal: None,
    };
    let cases = [
        (GridBoxFormat::Up, (102, 62)),
        (GridBoxFormat::Down, (102, 57)),
        (GridBoxFormat::Left, (177, 59)),
        (GridBoxFormat::Right, (27, 59)),
        (GridBoxFormat::UpperLeft, (7, 62)),
        (GridBoxFormat::LowerLeft, (7, 57)),
        (GridBoxFormat::UpperRight, (197, 62)),
    ];
    for (format, expected_origin) in cases {
        layout.format = format;
        let rects = layout
            .production_variable_height_slot_rects(&[(71, 17), (73, 29)])
            .expect("exact engine format case");
        assert_eq!(
            (rects[1].x(), rects[1].y()),
            expected_origin,
            "{format:?}"
        );
        assert_eq!((rects[1].w(), rects[1].h()), (73, 29), "{format:?}");
    }
}

#[test]
fn gridbox_format_parser_matches_the_engine_table_and_default() {
    assert_eq!(GridBoxFormat::parse(Some("UP")), GridBoxFormat::Up);
    assert_eq!(GridBoxFormat::parse(Some("DOWN")), GridBoxFormat::Down);
    assert_eq!(GridBoxFormat::parse(Some("LEFT")), GridBoxFormat::Left);
    assert_eq!(GridBoxFormat::parse(Some("RIGHT")), GridBoxFormat::Right);
    assert_eq!(
        GridBoxFormat::parse(Some("UPPER_LEFT")),
        GridBoxFormat::UpperLeft
    );
    assert_eq!(
        GridBoxFormat::parse(Some("LOWER_LEFT")),
        GridBoxFormat::LowerLeft
    );
    assert_eq!(
        GridBoxFormat::parse(Some("UPPER_RIGHT")),
        GridBoxFormat::UpperRight
    );
    assert_eq!(GridBoxFormat::parse(None), GridBoxFormat::Up);
    assert_eq!(
        GridBoxFormat::parse(Some("unsupported_mod_value")),
        GridBoxFormat::Up
    );
}

#[test]
#[cfg_attr(
    not(gate_hoi4_install),
    ignore = "the game install (set BATTLE_PLAN_GAME_ROOT / BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA, or install at the default Steam path)"
)]
fn scrollbar_declared_parts_are_resolved_but_thumb_travel_fails_closed() {
    let root = game_root();
    let layouts = parse_layout(&root, "interface/airwingreorganization.gui");
    let split = find(&layouts, "split_slider", "scrollbarType");
    let options = ComposeOptions::new(1024, 768);

    let layout = scrollbar_layout(split, Rect::new(0, 0, 1024, 768), &options).expect("scrollbar");
    assert_eq!(layout.rect, Rect::new(23, 103, 230, 26));
    assert_eq!(layout.axis, ScrollbarAxis::Horizontal);
    assert_eq!(layout.family, ScrollbarFamily::Legacy);
    assert_eq!(layout.border_size, Some((16, 16)));
    assert_eq!(layout.min_value, Some(0.0));
    assert_eq!(layout.max_value, Some(100.0));
    assert_eq!(layout.step_size, Some(1.0));
    assert_eq!(layout.start_value, Some(50.0));
    assert_eq!(
        layout.slider.as_ref().map(|part| part.rect),
        Some(Rect::new(23, 103, 0, 0))
    );
    assert_eq!(
        layout.track.as_ref().map(|part| part.rect),
        Some(Rect::new(23, 123, 0, 0))
    );
    assert!(layout
        .thumb_rect_for_value(50.0)
        .expect_err("thumb travel is untraced")
        .message
        .contains("UNTRACED"));
}
