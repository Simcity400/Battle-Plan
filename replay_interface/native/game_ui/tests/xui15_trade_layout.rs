use game_ui::assets::AssetResolver;
use game_ui::gfx::SpriteRegistry;
use game_ui::trade_layout::{
    CountryTradeSort, CountryTradeViewerState, TradeAuthoredLayout, TradePointerAction,
    TradeScrollTarget, TRADE_SCROLL_FIXED_SCALE,
};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture(gui: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "xui15-authored-layout-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(root.join("interface")).unwrap();
    fs::write(root.join("interface/countrytradeview.gui"), gui).unwrap();
    fs::write(root.join("interface/core.gui"), CORE_GUI).unwrap();
    root
}

const CORE_GUI: &str = r#"
guiTypes = {
  extendedScrollbarType = {
    name = "right_vertical_slider" position = { x = -5 y = 0 }
    size = { width = 18 height = 18 } orientation = upper_right origo = upper_right
    smooth_scrolling = 0.25
    background = { name = "Background" size = { x = 18 y = 18 } quadTextureSprite = "GFX_xui15_scroll_background" }
    slider = { name = "Slider" position = { x = -2 y = -2 } size = { x = 12 y = 12 } quadTextureSprite = "GFX_xui15_scroll_slider" }
    track = { name = "Track" position = { x = 3 y = 0 } size = { x = 12 y = 12 } quadTextureSprite = "GFX_xui15_scroll_track" }
    decreaseButton = { name = "Decrease" position = { x = 2 y = 3 } size = { x = 12 y = 12 } quadTextureSprite = "GFX_xui15_scroll_decrease" }
    increaseButton = { name = "Increase" position = { x = -17 y = -19 } size = { x = 12 y = 12 } quadTextureSprite = "GFX_xui15_scroll_increase" }
  }
}
"#;

const GUI: &str = r#"
guiTypes = {
  containerWindowType = {
    name = "countrytradeview"
    position = { x = -304 y = 15 }
    show_position = { x = -4 y = 15 }
    size = { width = 300 height = 100%% }
    containerWindowType = {
      name = "resources" position = { x = 10 y = 20 } size = { width = 280 height = 100 }
      gridBoxType = { name = "resources_grid" position = { x = 20 y = 0 } size = { width = 200 height = 100%% } slotsize = { width = 31 height = 70 } max_slots = { x = 7 y = 1 } format = UPPER_LEFT }
    }
    containerWindowType = {
      name = "filters" position = { x = 20 y = 130 } size = { width = 200 height = 40 }
      verticalScrollbar = "right_vertical_slider"
      gridBoxType = { name = "resources_grid" position = { x = 5 y = 3 } size = { width = 160 height = 30 } slotsize = { width = 31 height = 30 } max_slots_vertical = 1 format = UPPER_LEFT }
    }
    containerWindowType = {
      name = "country_filters" position = { x = 30 y = 180 } size = { width = 250 height = 40 }
      verticalScrollbar = "right_vertical_slider"
      gridBoxType = { name = "country_filters_grid" position = { x = 7 y = 6 } size = { width = 240 height = 30 } slotsize = { width = 50 height = 25 } max_slots_horizontal = 3 format = UPPER_LEFT }
    }
    containerWindowType = {
      name = "trade" position = { x = 12 y = 220 } size = { width = 270 height = 100%% }
      verticalScrollbar = "right_vertical_slider"
      scroll_wheel_factor = 37 smooth_scrolling = yes margin = { top = 1 bottom = 9 }
      gridBoxType = { name = "trade_grid" position = { x = 0 y = 0 } size = { width = 100%% height = 100%% } slotsize = { width = 260 height = 29 } max_slots_horizontal = 1 format = UPPER_LEFT }
    }
    buttonType = { name = "sort_country" position = { x = 1 y = 190 } size = { x = 30 y = 10 } }
    buttonType = { name = "sort_export" position = { x = 31 y = 190 } size = { x = 40 y = 10 } }
    buttonType = { name = "sort_request" position = { x = 71 y = 190 } size = { x = 50 y = 10 } }
    buttonType = { name = "sort_convoys" position = { x = 121 y = 190 } size = { x = 60 y = 10 } }
    buttonType = { name = "close_button" position = { x = -20 y = 5 } size = { x = 15 y = 15 } orientation = UPPER_RIGHT }
  }
  containerWindowType = { name = "resources_info_entry" position = { x = 999 y = 999 } size = { width = 31 height = 75 } }
  containerWindowType = { name = "resources_filter_entry" position = { x = 888 y = 888 } size = { width = 29 height = 30 } }
  containerWindowType = { name = "filter_entry" position = { x = 777 y = 777 } size = { width = 48 height = 25 } }
  containerWindowType = { name = "country_trade_entry" position = { x = 666 y = 666 } size = { width = 260 height = 29 } }
}
"#;

#[test]
fn effective_gui_controls_every_xui15_grid_and_scissor() {
    let root = fixture(GUI);
    let layout = TradeAuthoredLayout::load(&AssetResolver::new(vec![root.clone()]), 800, 600)
        .expect("admit synthetic effective countrytradeview");

    assert_eq!((layout.root_rect.x(), layout.root_rect.y()), (-4, 15));
    assert_eq!(
        (
            layout.resources.grid_layout.rect.x(),
            layout.resources.grid_layout.rect.y(),
            layout.resources.grid_layout.slot_size
        ),
        (26, 35, (31, 70))
    );
    assert_eq!(
        (
            layout.resource_filters.visible_clip.x(),
            layout.resource_filters.visible_clip.y(),
            layout.resource_filters.visible_clip.w(),
            layout.resource_filters.visible_clip.h()
        ),
        (21, 148, 160, 30)
    );
    assert_eq!(
        (
            layout.country_filters.grid_layout.rect.x(),
            layout.country_filters.grid_layout.rect.y(),
            layout.country_filters.grid_layout.slot_size
        ),
        (33, 201, (50, 25))
    );
    assert_eq!(
        (
            layout.catalogue.visible_clip.x(),
            layout.catalogue.visible_clip.y(),
            layout.catalogue.visible_clip.w(),
            layout.catalogue.visible_clip.h()
        ),
        (8, 236, 270, 355)
    );
    assert_eq!(layout.catalogue.scroll_wheel_pixels, 37);
    assert!(layout.catalogue.smooth_scrolling);
    assert_eq!(
        (
            layout.sort_country_rect.x(),
            layout.sort_country_rect.y(),
            layout.sort_convoys_rect.x(),
            layout.sort_convoys_rect.w()
        ),
        (-3, 205, 117, 60)
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn malformed_hierarchy_fails_closed_instead_of_falling_back_to_vanilla_geometry() {
    let root = fixture(&GUI.replace("name = \"trade_grid\"", "name = \"other_grid\""));
    let error = TradeAuthoredLayout::load(&AssetResolver::new(vec![root.clone()]), 800, 600)
        .expect_err("missing effective trade grid must reject");
    assert!(error
        .message
        .contains("expected exactly one direct trade_grid"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn viewer_click_sort_filter_and_wheel_state_are_replay_owned_and_clamped() {
    let root = fixture(GUI);
    let layout = TradeAuthoredLayout::load(&AssetResolver::new(vec![root.clone()]), 800, 600)
        .expect("admit synthetic effective countrytradeview");
    let mut viewer = CountryTradeViewerState::default();

    let close = layout.close_button_rect;
    assert_eq!(
        layout
            .hit_test(
                &viewer,
                (close.x() + 1, close.y() + 1),
                &[10, 11, 12, 13],
                7,
                4,
                30,
            )
            .expect("close hit test"),
        TradePointerAction::Close
    );

    let resource_scrollbar = layout
        .resource_filters
        .scrollbar
        .as_ref()
        .expect("resource-filter scrollbar");
    let action = layout
        .hit_test(
            &viewer,
            (
                resource_scrollbar.increase.rect.x() + 1,
                resource_scrollbar.increase.rect.y() + 1,
            ),
            &[10, 11, 12, 13],
            7,
            4,
            30,
        )
        .expect("resource scrollbar increase");
    assert_eq!(action, TradePointerAction::ScrollResourceFilters(1));
    layout
        .apply_pointer_action(&mut viewer, action, 7, 4, 30)
        .expect("apply resource scrollbar increase");
    assert_eq!(
        viewer.resource_filter_scroll.target_fixed,
        31 * TRADE_SCROLL_FIXED_SCALE
    );
    viewer.resource_filter_scroll = Default::default();

    let resource_grid = layout.resource_filters.grid_layout.rect;
    let action = layout
        .hit_test(
            &viewer,
            (resource_grid.x() + 3, resource_grid.y() + 3),
            &[10, 11, 12, 13],
            7,
            4,
            30,
        )
        .expect("resource hit test");
    assert_eq!(action, TradePointerAction::SelectResource(0));
    viewer.apply_pointer_action(action);
    assert_eq!(viewer.selected_resource_index, 0);

    viewer.resource_filter_scroll.displayed_fixed = layout
        .resource_filter_max_scroll_fixed(7)
        .expect("resource scroll range");
    assert_eq!(
        viewer.resource_filter_scroll.displayed_fixed,
        57 * TRADE_SCROLL_FIXED_SCALE
    );
    let action = layout
        .hit_test(&viewer, (153, 151), &[10, 11, 12, 13], 7, 4, 30)
        .expect("scrolled resource hit test");
    assert_eq!(action, TradePointerAction::SelectResource(6));
    viewer.apply_pointer_action(action);
    assert_eq!(viewer.selected_resource_index, 6);

    let action = layout
        .hit_test(&viewer, (35, 220), &[10, 11, 12, 13], 7, 4, 30)
        .expect("country-filter hit test");
    assert_eq!(action, TradePointerAction::ToggleCountryFilter(10));
    viewer.apply_pointer_action(action);
    assert_eq!(viewer.selected_country_filter_ids, vec![10]);

    assert_eq!(
        layout
            .wheel_action(&mut viewer, (35, 220), -1, 7, 4, 30)
            .expect("country-filter wheel"),
        TradePointerAction::None
    );
    assert_eq!(viewer.country_filter_scroll.target_fixed, 0);

    let action = layout
        .hit_test(&viewer, (40, 208), &[], 7, 4, 30)
        .expect("sort hit test");
    assert_eq!(
        action,
        TradePointerAction::Sort(CountryTradeSort::AvailableExport)
    );
    viewer.apply_pointer_action(action);
    assert_eq!(viewer.sort, CountryTradeSort::AvailableExport);
    assert!(!viewer.descending);
    viewer.apply_pointer_action(action);
    assert!(viewer.descending);

    assert_eq!(
        layout
            .wheel_action(&mut viewer, (20, 300), -100, 7, 4, 30)
            .expect("catalogue wheel"),
        TradePointerAction::ScrollCatalogue
    );
    assert_eq!(
        viewer.catalogue_scroll.target_fixed,
        layout
            .catalogue_max_scroll_fixed(30)
            .expect("catalogue scroll range")
    );
    layout
        .advance_smooth_scroll(&mut viewer)
        .expect("advance first exact smooth-scroll frame");
    let first_displayed = viewer.catalogue_scroll.displayed_fixed;
    layout
        .advance_smooth_scroll(&mut viewer)
        .expect("advance second exact smooth-scroll frame");
    let second_displayed = viewer.catalogue_scroll.displayed_fixed;
    assert!(first_displayed > 0);
    assert!(second_displayed > first_displayed);
    assert!(second_displayed < viewer.catalogue_scroll.target_fixed);

    viewer.catalogue_scroll = Default::default();
    let catalogue_scrollbar = layout.catalogue.scrollbar.as_ref().unwrap();
    let catalogue_max = layout.catalogue_max_scroll_fixed(30).unwrap();
    let slider = catalogue_scrollbar.slider_rect(0, catalogue_max).unwrap();
    let slider_point = (slider.x() + slider.w() / 2, slider.y() + slider.h() / 2);
    let action = layout
        .hit_test(&viewer, slider_point, &[], 7, 4, 30)
        .expect("catalogue slider hit test");
    assert_eq!(
        action,
        TradePointerAction::BeginScrollbarDrag(TradeScrollTarget::Catalogue, slider.h() / 2)
    );
    layout
        .apply_pointer_action(&mut viewer, action, 7, 4, 30)
        .expect("begin catalogue thumb drag");
    assert!(layout
        .drag_active_scrollbar(
            &mut viewer,
            catalogue_scrollbar.track.rect.y() + catalogue_scrollbar.track.rect.h(),
            7,
            4,
            30,
        )
        .expect("drag catalogue thumb"));
    assert_eq!(
        catalogue_scrollbar
            .slider_rect(viewer.catalogue_scroll.target_fixed, catalogue_max)
            .expect("dragged catalogue slider"),
        catalogue_scrollbar
            .slider_rect(catalogue_max, catalogue_max)
            .expect("maximum catalogue slider")
    );
    assert!(layout.release_active_scrollbar(&mut viewer));
    assert!(viewer.active_scrollbar_drag.is_none());

    fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg_attr(not(gate_hoi4_install), ignore = "requires the current the game install")]
fn shipped_a729_countrytradeview_resolves_without_coordinate_literals() {
    let game = replay_test_gates::require_vanilla_game_root();
    let resolver = AssetResolver::new(vec![game]);
    let mut registry = SpriteRegistry::new();
    for file in resolver
        .list_files_recursive("interface", Some("gfx"))
        .expect("effective gfx files")
    {
        registry
            .parse_file(&file.path, &file.rel_path)
            .expect("parse effective gfx");
    }
    let layout = TradeAuthoredLayout::load_with_registry(&resolver, &registry, 1920, 1080)
        .expect("admit shipped countrytradeview");
    assert_eq!(
        (
            layout.resources.grid_layout.rect.x(),
            layout.resources.grid_layout.rect.y(),
            layout.resources.grid_layout.slot_size
        ),
        (106, 193, (79, 156))
    );
    assert_eq!(
        (
            layout.catalogue.visible_clip.x(),
            layout.catalogue.visible_clip.y(),
            layout.catalogue.visible_clip.w(),
            layout.catalogue.visible_clip.h()
        ),
        (9, 541, 674, 526)
    );
    assert_eq!(layout.catalogue.scroll_wheel_pixels, 45);
    assert!(layout.catalogue.smooth_scrolling);
}
