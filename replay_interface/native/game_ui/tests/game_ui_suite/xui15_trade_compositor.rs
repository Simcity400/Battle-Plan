use game_ui::assets::AssetResolver;
use game_ui::dds::RgbaImage;
use game_ui::gfx::SpriteRegistry;
use game_ui::loc::Localisation;
use game_ui::trade_compositor::{
    compose_observer_trade_panel, AdmittedObserverTradePanel, ObserverTradeCatalogueRow,
    ObserverTradeCountryFilterRow,
};
use game_ui::trade_layout::{CountryTradeViewerState, TradeAuthoredLayout};
use game_ui::trade_state::{
    TradeData, TradeDataIssue, TradeFactoryHeader, TradePresentation, TradeResourceCard,
    TRADE_RESOURCE_ORDER,
};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture_root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("xui15-compositor-{label}-{nonce}"))
}

fn write_gui(root: &PathBuf) {
    let path = root.join("interface/countrytradeview.gui");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        r#"guiTypes = {
  containerWindowType = {
    name = "countrytradeview" show_position = { x = 5 y = 10 }
    size = { width = 700 height = 580 }
    containerWindowType = {
      name = "resources" position = { x = 10 y = 20 } size = { width = 600 height = 120 }
      gridBoxType = { name = "resources_grid" position = { x = 2 y = 3 } size = { width = 580 height = 100 } slotsize = { width = 70 height = 90 } format = UPPER_LEFT }
    }
    containerWindowType = {
      name = "filters" position = { x = 10 y = 145 } size = { width = 580 height = 55 }
      verticalScrollbar = "right_vertical_slider"
      gridBoxType = { name = "resources_grid" position = { x = 2 y = 3 } size = { width = 570 height = 45 } slotsize = { width = 74 height = 50 } format = UPPER_LEFT }
    }
    containerWindowType = {
      name = "country_filters" position = { x = 10 y = 205 } size = { width = 580 height = 55 }
      verticalScrollbar = "right_vertical_slider"
      gridBoxType = { name = "country_filters_grid" position = { x = 2 y = 3 } size = { width = 570 height = 45 } slotsize = { width = 168 height = 50 } max_slots_horizontal = 3 format = UPPER_LEFT }
    }
    containerWindowType = {
      name = "trade" position = { x = 10 y = 265 } size = { width = 660 height = 300 }
      verticalScrollbar = "right_vertical_slider"
      margin = { top = 1 bottom = 4 } scroll_wheel_factor = 45 smooth_scrolling = yes
      gridBoxType = { name = "trade_grid" position = { x = 2 y = 3 } size = { width = 650 height = 290 } slotsize = { width = 648 height = 45 } max_slots_horizontal = 1 format = UPPER_LEFT }
    }
    buttonType = { name = "sort_country" position = { x = 10 y = 250 } size = { width = 100 height = 15 } }
    buttonType = { name = "sort_export" position = { x = 120 y = 250 } size = { width = 100 height = 15 } }
    buttonType = { name = "sort_request" position = { x = 230 y = 250 } size = { width = 100 height = 15 } }
    buttonType = { name = "sort_convoys" position = { x = 340 y = 250 } size = { width = 100 height = 15 } }
    buttonType = { name = "close_button" position = { x = -20 y = 5 } size = { x = 15 y = 15 } orientation = UPPER_RIGHT }
    instantTextboxType = { name = "factories_total" position = { x = 600 y = 20 } maxWidth = 30 maxHeight = 18 }
    instantTextboxType = { name = "factories_used" position = { x = 600 y = 40 } maxWidth = 30 maxHeight = 18 }
    instantTextboxType = { name = "factories_available" position = { x = 600 y = 60 } maxWidth = 30 maxHeight = 18 }
  }
  containerWindowType = {
    name = "resources_info_entry" size = { width = 70 height = 90 }
    instantTextboxType = { name = "balance" position = { x = 1 y = 1 } maxWidth = 30 maxHeight = 10 }
    instantTextboxType = { name = "produced" position = { x = 1 y = 12 } maxWidth = 30 maxHeight = 10 }
    instantTextboxType = { name = "imported" position = { x = 1 y = 23 } maxWidth = 30 maxHeight = 10 }
    instantTextboxType = { name = "exported" position = { x = 1 y = 34 } maxWidth = 30 maxHeight = 10 }
    instantTextboxType = { name = "special_project" position = { x = 1 y = 45 } maxWidth = 30 maxHeight = 10 }
    instantTextboxType = { name = "production_line" position = { x = 1 y = 56 } maxWidth = 30 maxHeight = 10 }
    instantTextboxType = { name = "needed" position = { x = 1 y = 67 } maxWidth = 30 maxHeight = 10 }
    instantTextboxType = { name = "neededType" position = { x = 35 y = 67 } maxWidth = 30 maxHeight = 10 }
    buttonType = { name = "resources_icon" position = { x = 35 y = 1 } size = { width = 10 height = 10 } }
    buttonType = { name = "cancel_imports" position = { x = 50 y = 70 } size = { width = 10 height = 10 } }
  }
  containerWindowType = {
    name = "resources_filter_entry" size = { width = 74 height = 50 }
    buttonType = { name = "background" size = { width = 74 height = 50 } }
    iconType = { name = "icon" size = { width = 10 height = 10 } }
  }
  containerWindowType = {
    name = "filter_entry" size = { width = 168 height = 50 }
    background = { name = "Background" size = { width = 168 height = 50 } }
    instantTextboxType = { name = "name" position = { x = 2 y = 2 } maxWidth = 100 maxHeight = 15 }
  }
  containerWindowType = {
    name = "country_trade_entry" size = { width = 648 height = 45 }
    buttonType = { name = "country_flag" position = { x = 2 y = 2 } size = { width = 20 height = 12 } }
    instantTextboxType = { name = "country" position = { x = 25 y = 2 } maxWidth = 100 maxHeight = 15 }
    instantTextboxType = { name = "available_export" position = { x = 150 y = 2 } maxWidth = 80 maxHeight = 15 }
    instantTextboxType = { name = "exported" position = { x = 250 y = 2 } maxWidth = 80 maxHeight = 15 }
    instantTextboxType = { name = "convoys" position = { x = 350 y = 2 } maxWidth = 80 maxHeight = 15 }
  }
}"#,
    )
    .unwrap();
    fs::write(
        root.join("interface/core.gui"),
        r#"guiTypes = {
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
}"#,
    )
    .unwrap();
    fs::write(
        root.join("interface/xui15_scroll.gfx"),
        r#"spriteTypes = {
  spriteType = { name = "GFX_xui15_scroll_background" texturefile = "gfx/interface/xui15_scroll.tga" borderSize = { x = 1 y = 1 } }
  spriteType = { name = "GFX_xui15_scroll_slider" texturefile = "gfx/interface/xui15_slider.tga" }
  spriteType = { name = "GFX_xui15_scroll_track" texturefile = "gfx/interface/xui15_scroll.tga" borderSize = { x = 1 y = 1 } }
  spriteType = { name = "GFX_xui15_scroll_decrease" texturefile = "gfx/interface/xui15_scroll.tga" }
  spriteType = { name = "GFX_xui15_scroll_increase" texturefile = "gfx/interface/xui15_scroll.tga" }
}"#,
    )
    .unwrap();
    let mut tga = vec![0u8; 18];
    tga[2] = 2;
    tga[12..14].copy_from_slice(&2u16.to_le_bytes());
    tga[14..16].copy_from_slice(&2u16.to_le_bytes());
    tga[16] = 32;
    tga[17] = 0x20 | 8;
    tga.extend_from_slice(&[
        0, 0, 255, 255, 0, 255, 0, 255, 255, 0, 0, 255, 255, 255, 255, 255,
    ]);
    let texture = root.join("gfx/interface/xui15_scroll.tga");
    fs::create_dir_all(texture.parent().unwrap()).unwrap();
    fs::write(texture, tga).unwrap();
    let mut slider_tga = vec![0u8; 18];
    slider_tga[2] = 2;
    slider_tga[12..14].copy_from_slice(&1u16.to_le_bytes());
    slider_tga[14..16].copy_from_slice(&1u16.to_le_bytes());
    slider_tga[16] = 32;
    slider_tga[17] = 0x20 | 8;
    slider_tga.extend_from_slice(&[0, 0, 255, 255]);
    fs::write(root.join("gfx/interface/xui15_slider.tga"), slider_tga).unwrap();
}

fn scrollbar_registry(root: &PathBuf) -> SpriteRegistry {
    let mut registry = SpriteRegistry::new();
    registry
        .parse_content(&map_viewer::GameFileResolver::vanilla(&root).unwrap().read_required_bytes("interface/xui15_scroll.gfx", "fixture content").unwrap(), "interface/xui15_scroll.gfx")
        .expect("parse synthetic scrollbar sprites");
    registry
}

fn presentation() -> TradePresentation {
    let resources = TRADE_RESOURCE_ORDER
        .iter()
        .enumerate()
        .map(|(index, resource)| TradeResourceCard {
            resource: (*resource).into(),
            visible_order: index as i64,
            balance: index as f64,
            produced: 10.0,
            imported: 0.0,
            exported: 1.0,
            special_project: 0.0,
            production_line: 2.0,
            needed: 3.0,
            needed_current: None,
            needed_max: None,
            needed_type: String::new(),
        })
        .collect();
    TradePresentation {
        country_tag: "ENG".into(),
        resource_pools: TradeData::Unavailable(TradeDataIssue::NotRecorded),
        resource_cards: TradeData::Available(resources),
        deals: TradeData::Available(Vec::new()),
        market_stockpile: TradeData::Unavailable(TradeDataIssue::NotRecorded),
        equipment_purchase_contracts: TradeData::Unavailable(TradeDataIssue::NotRecorded),
        production_license_agreements: TradeData::Unavailable(TradeDataIssue::NotRecorded),
        factories: TradeData::Available(TradeFactoryHeader {
            total: 20,
            used: 5,
            available: 15,
        }),
        resource_totals: TradeData::Unavailable(TradeDataIssue::NotRecorded),
        deal_totals: TradeData::Unavailable(TradeDataIssue::NotRecorded),
    }
}

fn flag(rgb: [u8; 3]) -> RgbaImage {
    RgbaImage {
        width: 1,
        height: 1,
        pixels: vec![rgb[0], rgb[1], rgb[2], 255],
    }
}

#[test]
fn complete_panel_clones_all_four_authored_collections() {
    assert_country_name_panel(false);
}

#[test]
fn unavailable_country_name_preserves_the_panel_and_other_country_rows() {
    assert_country_name_panel(true);
}

fn assert_country_name_panel(unavailable_name: bool) {
    let root = fixture_root(if unavailable_name { "unavailable-name" } else { "complete" });
    write_gui(&root);
    let presentation = presentation();
    let mut catalogue = vec![
        ObserverTradeCatalogueRow {
            stable_country_id: "1".into(),
            display_ordinal: 0,
            country_name: "Germany".into(),
            country_name_issue: None,
            available_export_text: "12".into(),
            exported_text: "4".into(),
            convoys_text: "2/3".into(),
            flag: flag([0, 0, 0]),
        },
        ObserverTradeCatalogueRow {
            stable_country_id: "2".into(),
            display_ordinal: 1,
            country_name: "France".into(),
            country_name_issue: None,
            available_export_text: "9".into(),
            exported_text: "3".into(),
            convoys_text: "1/1".into(),
            flag: flag([0, 0, 255]),
        },
    ];
    for index in 2..10u32 {
        catalogue.push(ObserverTradeCatalogueRow {
            stable_country_id: (index + 1).to_string(),
            display_ordinal: index,
            country_name: format!("Country {index}"),
            country_name_issue: None,
            available_export_text: "1".into(),
            exported_text: "0".into(),
            convoys_text: "0/0".into(),
            flag: flag([index as u8, 0, 0]),
        });
    }
    let filters = vec![
        ObserverTradeCountryFilterRow {
            token_id: 10,
            source_ordinal: 0,
            display_name: "Allies".into(),
        },
        ObserverTradeCountryFilterRow {
            token_id: 20,
            source_ordinal: 1,
            display_name: "Neighbours".into(),
        },
    ];
    let viewer = CountryTradeViewerState {
        selected_resource_index: 1,
        selected_country_filter_ids: vec![20],
        catalogue_scroll: game_ui::trade_layout::TradeViewerScroll {
            displayed_fixed: 25 * game_ui::trade_layout::TRADE_SCROLL_FIXED_SCALE,
            target_fixed: 25 * game_ui::trade_layout::TRADE_SCROLL_FIXED_SCALE,
        },
        ..CountryTradeViewerState::default()
    };
    let issue = game_ui::text_law::TextIssue {
        kind: game_ui::text_law::TextIssueKind::CountryNameInputsUnavailable,
        token: "current leader subtype unavailable".into(),
    };
    if unavailable_name {
        catalogue[0].country_name.clear();
        catalogue[0].country_name_issue = Some(issue.clone());
    }
    let registry = scrollbar_registry(&root);
    let output = compose_observer_trade_panel(
        &AssetResolver::new(vec![root.clone()]),
        &registry,
        &Localisation::default(),
        AdmittedObserverTradePanel {
            frame_hour: 7,
            country_tag: "ENG",
            presentation: &presentation,
            catalogue_rows: &catalogue,
            country_filters: &filters,
        },
        &viewer,
        800,
        600,
    )
    .expect("compose complete XUI15 panel");
    let names = output
        .debug_rects
        .iter()
        .map(|rect| rect.name.as_str())
        .collect::<Vec<_>>();
    assert!(names.contains(&"resource:oil::resources_info_entry"));
    assert!(names.contains(&"resource-filter:oil::resources_filter_entry"));
    assert!(names.contains(&"country-filter:10::filter_entry"));
    assert!(names.contains(&"country:1::country_trade_entry"));
    assert!(names.contains(&"country:2::country_trade_entry"));
    assert_eq!(catalogue[1].country_name, "France");
    if unavailable_name {
        assert_eq!(output.report.unresolved_text_inputs.get("xui15:country-name:1"), Some(&vec![issue]));
    } else {
        assert!(!output.report.unresolved_text_inputs.contains_key("xui15:country-name:1"));
    }
    assert!(!output.report.unresolved_text_inputs.contains_key("xui15:country-name:2"));
    assert_eq!(
        output
            .debug_rects
            .iter()
            .find(|rect| rect.name == "country:1::country_trade_entry")
            .unwrap()
            .y,
        253
    );
    assert!(output.report.missing_sprites.is_empty());
    let layout = TradeAuthoredLayout::load_with_registry(
        &AssetResolver::new(vec![root.clone()]),
        &registry,
        800,
        600,
    )
    .expect("reload authored scrollbar geometry");
    let slider = layout
        .catalogue
        .scrollbar
        .as_ref()
        .expect("catalogue scrollbar")
        .slider_rect(
            viewer.catalogue_scroll.displayed_fixed,
            layout
                .catalogue_max_scroll_fixed(catalogue.len())
                .expect("catalogue range"),
        )
        .expect("mapped catalogue slider");
    let sample_x = usize::try_from(slider.x() + slider.w() / 2).unwrap();
    let sample_y = usize::try_from(slider.y() + slider.h() / 2).unwrap();
    let offset = (sample_y * output.image.width + sample_x) * 4;
    assert_eq!(&output.image.pixels[offset..offset + 4], &[255, 0, 0, 255]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn incomplete_catalogue_order_fails_before_any_asset_fallback() {
    let presentation = presentation();
    let catalogue = vec![ObserverTradeCatalogueRow {
        stable_country_id: "1".into(),
        display_ordinal: 1,
        country_name: "Germany".into(),
        country_name_issue: None,
        available_export_text: "12".into(),
        exported_text: "4".into(),
        convoys_text: "2/3".into(),
        flag: flag([0, 0, 0]),
    }];
    let error = compose_observer_trade_panel(
        &AssetResolver::new(Vec::new()),
        &SpriteRegistry::new(),
        &Localisation::default(),
        AdmittedObserverTradePanel {
            frame_hour: 7,
            country_tag: "ENG",
            presentation: &presentation,
            catalogue_rows: &catalogue,
            country_filters: &[],
        },
        &CountryTradeViewerState::default(),
        800,
        600,
    )
    .expect_err("noncontiguous catalogue cannot be painted");
    assert!(error.to_string().contains("order is not contiguous"));
}
