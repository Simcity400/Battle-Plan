use game_ui::compose::{measure_to_pixels_for_parent, ComposeOptions, Rect};
use game_ui::gui::{GuiElement, LayoutSet};
use game_ui::widget_layout::{
    gridbox_layout, listbox_layout, scrollbar_layout, ListFlow, ScrollbarAxis, ScrollbarFamily,
};
use std::path::{Path, PathBuf};

const DEFAULT_GAME_ROOT: &str = r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV";

fn game_root() -> Option<PathBuf> {
    let root = std::env::var_os("BATTLE_PLAN_GAME_ROOT").or_else(|| std::env::var_os("HOI4_GAME_ROOT"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_GAME_ROOT));
    root.is_dir().then_some(root)
}

fn parse_layout(root: &Path, rel: &str) -> LayoutSet {
    let mut layouts = LayoutSet::default();
    layouts
        .parse_file(&root.join(rel), rel)
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
fn backend_listbox_declares_vertical_rows_from_file_template_size() {
    let Some(root) = game_root() else {
        eprintln!("SKIP backend_listbox_declares_vertical_rows_from_file_template_size: the game root missing");
        return;
    };
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
fn achievements_gridbox_slots_follow_declared_slot_size() {
    let Some(root) = game_root() else {
        eprintln!("SKIP achievements_gridbox_slots_follow_declared_slot_size: the game root missing");
        return;
    };
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
fn scrollbar_declared_parts_are_resolved_but_thumb_travel_fails_closed() {
    let Some(root) = game_root() else {
        eprintln!("SKIP scrollbar_declared_parts_are_resolved_but_thumb_travel_fails_closed: the game root missing");
        return;
    };
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
