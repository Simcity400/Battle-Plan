use game_ui::assets::AssetResolver;
use game_ui::compose::{ComposeOptions, Compositor, Rect};
use game_ui::gfx::SpriteRegistry;
use game_ui::gui::{parse_gui_document, GuiElement};
use game_ui::observer_collection::{
    clone_rows_for_composition, compose_observer_row_layers, plan_observer_collection,
    AuthoredCollectionSource, AuthoredRowBinding, ObserverCollectionFamily,
    ObserverRowComposeInput, ObserverSemanticCollection, ObserverSemanticRow,
    StandardGridAuthority, ViewerCollectionState,
};
use game_ui::paradox_text::parse_str;

fn root(source: &str) -> GuiElement {
    let document = parse_str(&format!("guiTypes = {{ {source} }}"));
    parse_gui_document(&document)
        .into_iter()
        .next()
        .expect("one parsed GUI root")
}

fn semantic(rows: &[(&str, &str)]) -> ObserverSemanticCollection {
    ObserverSemanticCollection {
        collection_instance_token: "xui03:test-panel:hour-41".into(),
        frame_hour: 41,
        context_country: Some("FRA".into()),
        complete: true,
        rows: rows
            .iter()
            .enumerate()
            .map(|(ordinal, (id, subtype))| ObserverSemanticRow {
                stable_item_id: (*id).into(),
                source_ordinal: ordinal as u32,
                subtype_id: (*subtype).into(),
            })
            .collect(),
    }
}

fn source<'a>(element: &'a GuiElement) -> AuthoredCollectionSource<'a> {
    let is_grid = element.element_type.eq_ignore_ascii_case("gridBoxType");
    AuthoredCollectionSource {
        owning_window_id: if is_grid {
            "manifest:abc/interface/modded_designer.gui#countryequipmentdesignerview"
        } else {
            "interface/mod_panel.gui#00001"
        },
        root_source_identity: if is_grid {
            "manifest:abc/interface/modded_designer.gui#countryequipmentdesignerview"
        } else {
            "manifest:abc/interface/mod_panel.gui#00001"
        },
        collection_element_id: if is_grid {
            "manifest:abc/interface/modded_designer.gui#countryequipmentdesignerview/designer_intel_container/gridbox_container/acquired_variants_grid"
        } else {
            "interface/mod_panel.gui#collection-0001"
        },
        collection_element: element,
        effective_row_template_identity: None,
        effective_row_template: None,
        parent_rect: Rect::new(0, 0, 200, 120),
        grid_authority: is_grid
            .then_some(StandardGridAuthority::Xui13AcquiredVariantsStandardAutoAppend),
    }
}

fn viewer() -> ViewerCollectionState {
    ViewerCollectionState {
        scroll_offset_pixels: (0, 10),
        window_stack_ordinal: 4,
        root_paint_ordinal: 9,
        visible_clip: Rect::new(0, 0, 200, 75),
    }
}

#[test]
fn list_plan_uses_exact_spacing_border_and_variable_child_extents() {
    let list = root(
        r#"
        listBoxType = {
          name = "rows"
          position = { x = 10 y = 5 }
          size = { x = 100 y = 40 }
          spacing = 3
          borderSize = { x = 2 y = 99 }
          scrollbarType = "standardlistbox_slider"
        }
        "#,
    );
    let small = root(r#"containerWindowType = { name = "small" size = { x = 80 y = 20 } }"#);
    let large = root(r#"containerWindowType = { name = "large" size = { x = 90 y = 30 } }"#);
    let semantic = semantic(&[("row:a", "small"), ("row:b", "large")]);
    let bindings = [
        AuthoredRowBinding {
            stable_item_id: "row:a",
            subtype_id: "small",
            element_source_identity: "manifest:abc#small",
            instance_token: "xui03:test-panel:41:row:a",
            template: &small,
        },
        AuthoredRowBinding {
            stable_item_id: "row:b",
            subtype_id: "large",
            element_source_identity: "manifest:abc#large",
            instance_token: "xui03:test-panel:41:row:b",
            template: &large,
        },
    ];
    let plan = plan_observer_collection(
        41,
        &semantic,
        source(&list),
        &bindings,
        viewer(),
        &ComposeOptions::new(200, 120),
    )
    .expect("standard-list coordinates");
    assert_eq!(plan.family(), ObserverCollectionFamily::List);
    assert_eq!(plan.content_extent_pixels(), (92, 58));
    assert_eq!(
        plan.rows().map(|row| row.final_rect()).collect::<Vec<_>>(),
        vec![Rect::new(12, 0, 80, 20), Rect::new(12, 23, 90, 30)]
    );
}

#[test]
fn smooth_list_uses_its_separate_cumulative_axis_setter() {
    let list = root(
        r#"
        smoothListBoxType = {
          name = "rows"
          position = { x = 4 y = 7 }
          size = { x = 80 y = 50 }
          horizontal = yes
        }
        "#,
    );
    let first = root(r#"containerWindowType = { name = "first" size = { x = 11 y = 20 } }"#);
    let second = root(r#"containerWindowType = { name = "second" size = { x = 17 y = 25 } }"#);
    let semantic = semantic(&[("smooth:a", "first"), ("smooth:b", "second")]);
    let bindings = [
        AuthoredRowBinding {
            stable_item_id: "smooth:a",
            subtype_id: "first",
            element_source_identity: "manifest:abc#first",
            instance_token: "smooth:0",
            template: &first,
        },
        AuthoredRowBinding {
            stable_item_id: "smooth:b",
            subtype_id: "second",
            element_source_identity: "manifest:abc#second",
            instance_token: "smooth:1",
            template: &second,
        },
    ];
    let mut still = viewer();
    still.scroll_offset_pixels = (0, 0);
    let plan = plan_observer_collection(
        41,
        &semantic,
        source(&list),
        &bindings,
        still,
        &ComposeOptions::new(200, 120),
    )
    .expect("smooth-list coordinates");
    assert_eq!(plan.family(), ObserverCollectionFamily::SmoothList);
    assert_eq!(
        plan.rows().map(|row| row.final_rect()).collect::<Vec<_>>(),
        vec![Rect::new(4, 7, 11, 20), Rect::new(15, 7, 17, 25)]
    );
}

#[test]
fn malformed_list_layout_properties_fail_closed() {
    let template = root(r#"containerWindowType = { name = "item" size = { x = 20 y = 10 } }"#);
    let semantic = semantic(&[("item:a", "item")]);
    let bindings = [AuthoredRowBinding {
        stable_item_id: "item:a",
        subtype_id: "item",
        element_source_identity: "manifest:abc#item",
        instance_token: "item:0",
        template: &template,
    }];
    let mut still = viewer();
    still.scroll_offset_pixels = (0, 0);

    for (field, value, expected) in [
        ("horizontal", "sometimes", "horizontal"),
        ("spacing", "not_an_integer", "spacing"),
    ] {
        let list = root(&format!(
            r#"
            listBoxType = {{
              name = "bad_list"
              size = {{ x = 80 y = 20 }}
              {field} = {value}
            }}
            "#
        ));
        let error = plan_observer_collection(
            41,
            &semantic,
            source(&list),
            &bindings,
            still,
            &ComposeOptions::new(200, 120),
        )
        .expect_err("malformed list property must fail closed");
        assert!(error.message.contains(expected));
    }
}

#[test]
fn grid_plan_uses_modded_authored_axis_limits_format_and_template_sizes() {
    const MOD_TEMPLATE_ID: &str =
        "manifest:abc/interface/modded_designer_rows.gui#designer_equipment_entry";
    let grid = root(
        r#"
        gridBoxType = {
          name = "acquired_variants_grid"
          position = { x = 7 y = 11 }
          size = { x = 100 y = 80 }
          slotsize = { x = 17 y = 13 }
          max_slots_horizontal = 2
          max_slots_vertical = 0
          add_horizontal = yes
          format = UPPER_LEFT
        }
        "#,
    );
    let template = root(
        r#"containerWindowType = { name = "designer_equipment_entry" size = { x = 15 y = 9 } }"#,
    );
    let semantic = semantic(&[
        ("mod:a", "mod"),
        ("mod:b", "mod"),
        ("mod:c", "mod"),
        ("mod:d", "mod"),
    ]);
    let bindings = semantic
        .rows
        .iter()
        .enumerate()
        .map(|(ordinal, row)| AuthoredRowBinding {
            stable_item_id: row.stable_item_id.as_str(),
            subtype_id: row.subtype_id.as_str(),
            element_source_identity: MOD_TEMPLATE_ID,
            instance_token: match ordinal {
                0 => "mod:0",
                1 => "mod:1",
                2 => "mod:2",
                _ => "mod:3",
            },
            template: &template,
        })
        .collect::<Vec<_>>();
    let mut viewer = viewer();
    viewer.scroll_offset_pixels = (0, 0);
    viewer.visible_clip = Rect::new(0, 0, 200, 120);
    let mut unwitnessed = source(&grid);
    unwitnessed.collection_element_id =
        "manifest:abc/interface/modded_designer.gui#countryequipmentdesignerview/mod_grid";
    let error = plan_observer_collection(
        41,
        &semantic,
        unwitnessed,
        &bindings,
        viewer,
        &ComposeOptions::new(200, 120),
    )
    .expect_err("the XUI13 witness cannot be attached to an arbitrary mod grid");
    assert!(error.message.contains("does not match"));
    let mut modded_source = source(&grid);
    modded_source.effective_row_template_identity = Some(MOD_TEMPLATE_ID);
    modded_source.effective_row_template = Some(&template);
    let plan = plan_observer_collection(
        41,
        &semantic,
        modded_source,
        &bindings,
        viewer,
        &ComposeOptions::new(200, 120),
    )
    .expect("observer grid plan");
    assert_eq!(plan.family(), ObserverCollectionFamily::Grid);
    let first = plan.rows().next().expect("first modded designer row");
    assert_eq!(
        first.root_source_identity(),
        "manifest:abc/interface/modded_designer.gui#countryequipmentdesignerview"
    );
    assert_eq!(first.element_source_identity(), MOD_TEMPLATE_ID);
    assert_eq!(
        plan.rows().map(|row| row.final_rect()).collect::<Vec<_>>(),
        vec![
            Rect::new(7, 11, 15, 9),
            Rect::new(24, 11, 15, 9),
            Rect::new(7, 24, 15, 9),
            Rect::new(24, 24, 15, 9),
        ]
    );
    let clones = clone_rows_for_composition(&plan).expect("plan-owned row clones");
    assert_eq!(clones[0].1.name, "mod:0::designer_equipment_entry");
    assert_eq!(clones[3].1.name, "mod:3::designer_equipment_entry");
}

#[test]
fn measured_empty_grid_requires_an_exact_effective_template_pair() {
    let grid = root(
        r#"gridBoxType = {
          name = "acquired_variants_grid"
          size = { x = 100 y = 80 }
          slotsize = { x = 17 y = 13 }
          max_slots_horizontal = 2
        }"#,
    );
    let template = root(
        r#"containerWindowType = { name = "designer_equipment_entry" size = { x = 15 y = 9 } }"#,
    );
    let semantic = semantic(&[]);
    let mut authored = source(&grid);
    let mut still = viewer();
    still.scroll_offset_pixels = (0, 0);
    let error = plan_observer_collection(
        41,
        &semantic,
        authored,
        &[],
        still,
        &ComposeOptions::new(200, 120),
    )
    .expect_err("empty is not permission to omit row-template authority");
    assert!(error
        .message
        .contains("empty grid has no effective row-template authority"));

    authored.effective_row_template_identity =
        Some("manifest:abc/interface/modded_rows.gui#designer_equipment_entry");
    authored.effective_row_template = Some(&template);
    let plan = plan_observer_collection(
        41,
        &semantic,
        authored,
        &[],
        still,
        &ComposeOptions::new(200, 120),
    )
    .expect("measured-empty grid with exact template authority");
    assert_eq!(plan.rows().count(), 0);

    authored.effective_row_template_identity =
        Some("manifest:abc/interface/modded_rows.gui#wrong_template");
    assert!(plan_observer_collection(
        41,
        &semantic,
        authored,
        &[],
        still,
        &ComposeOptions::new(200, 120),
    )
    .is_err());
}

#[test]
fn designer_country_filter_uses_its_authenticated_template_and_hierarchy() {
    let grid = root(
        r#"gridBoxType = {
          name = "variant_country_filter_gridbox"
          size = { x = 100 y = 30 }
          slotsize = { x = 20 y = 20 }
          max_slots_horizontal = 4
        }"#,
    );
    let template = root(
        r#"containerWindowType = { name = "designer_creator_entry" size = { x = 18 y = 18 } }"#,
    );
    let semantic = semantic(&[("country:FRA", "creator")]);
    let bindings = [AuthoredRowBinding {
        stable_item_id: "country:FRA",
        subtype_id: "creator",
        element_source_identity:
            "manifest:abc/interface/modded_rows.gui#designer_creator_entry",
        instance_token: "creator:0",
        template: &template,
    }];
    let mut authored = source(&grid);
    authored.collection_element_id = "manifest:abc/interface/modded_designer.gui#countryequipmentdesignerview/designer_intel_container/country_filter_container/variant_country_filter_gridbox";
    authored.effective_row_template_identity =
        Some("manifest:abc/interface/modded_rows.gui#designer_creator_entry");
    authored.effective_row_template = Some(&template);
    authored.grid_authority =
        Some(StandardGridAuthority::Xui13VariantCountryFilterStandardAutoAppend);
    let mut still = viewer();
    still.scroll_offset_pixels = (0, 0);
    let plan = plan_observer_collection(
        41,
        &semantic,
        authored,
        &bindings,
        still,
        &ComposeOptions::new(200, 120),
    )
    .expect("authenticated country-filter grid");
    assert_eq!(
        plan.rows().next().unwrap().final_rect(),
        Rect::new(40, 0, 18, 18)
    );

    authored.collection_element_id = "manifest:abc/interface/modded_designer.gui#countryequipmentdesignerview/designer_intel_container/country_filter_container/not_the_authenticated_grid";
    assert!(plan_observer_collection(
        41,
        &semantic,
        authored,
        &bindings,
        still,
        &ComposeOptions::new(200, 120),
    )
    .is_err());
}

#[test]
fn xui19_grid_accepts_only_the_authored_decision_hierarchy_and_templates() {
    let grid = root(
        r#"
        gridBoxType = {
          name = "decision_grid"
          position = { x = 5 y = 9 }
          size = { x = 120 y = 80 }
          slotsize = { x = 60 y = 20 }
          max_slots_horizontal = 1
          add_horizontal = yes
          format = UPPER_LEFT
        }
        "#,
    );
    let template = root(
        r#"containerWindowType = { name = "decision_item" size = { x = 55 y = 18 } }"#,
    );
    let semantic = semantic(&[("decision:one", "decision_item")]);
    let binding = AuthoredRowBinding {
        stable_item_id: "decision:one",
        subtype_id: "decision_item",
        element_source_identity:
            "manifest:abc/interface/countrydecisionview.gui#decision_item",
        instance_token: "decision:one:41",
        template: &template,
    };
    let source = AuthoredCollectionSource {
        owning_window_id: "interface/countrydecisionview.gui#countrydecisionview",
        root_source_identity: "manifest:abc/interface/countrydecisionview.gui",
        collection_element_id:
            "interface/countrydecisionview.gui#countrydecisionview/body/decision_grid",
        collection_element: &grid,
        effective_row_template_identity: Some(
            "manifest:abc/interface/countrydecisionview.gui#decision_item",
        ),
        effective_row_template: Some(&template),
        parent_rect: Rect::new(0, 0, 200, 120),
        grid_authority: Some(StandardGridAuthority::Xui19DecisionStandardAutoAppend),
    };
    let mut still = viewer();
    still.scroll_offset_pixels = (0, 0);
    let plan = plan_observer_collection(
        41,
        &semantic,
        source,
        &[binding.clone()],
        still,
        &ComposeOptions::new(200, 120),
    )
    .expect("authenticated XUI19 decision grid");
    assert_eq!(
        plan.rows().next().unwrap().final_rect(),
        Rect::new(5, 9, 55, 18)
    );

    let mut wrong_binding = binding;
    wrong_binding.element_source_identity =
        "manifest:abc/interface/mod_panel.gui#decision_item";
    let error = plan_observer_collection(
        41,
        &semantic,
        source,
        &[wrong_binding],
        still,
        &ComposeOptions::new(200, 120),
    )
    .expect_err("a lookalike template from another file must be rejected");
    assert!(error.message.contains("four engine-resolved authored templates"));
}

#[test]
fn grid_omitted_add_horizontal_uses_exact_horizontal_first_runtime_default() {
    let grid = root(
        r#"
        gridBoxType = {
          name = "acquired_variants_grid"
          position = { x = 7 y = 11 }
          size = { x = 100 y = 80 }
          slotsize = { x = 17 y = 13 }
          max_slots_horizontal = 2
          format = UPPER_LEFT
        }
        "#,
    );
    let template = root(
        r#"containerWindowType = { name = "designer_equipment_entry" size = { x = 15 y = 9 } }"#,
    );
    let semantic = semantic(&[
        ("designer:a", "variant"),
        ("designer:b", "variant"),
        ("designer:c", "variant"),
        ("designer:d", "variant"),
    ]);
    let tokens = ["designer:0", "designer:1", "designer:2", "designer:3"];
    let bindings = semantic
        .rows
        .iter()
        .enumerate()
        .map(|(ordinal, row)| AuthoredRowBinding {
            stable_item_id: row.stable_item_id.as_str(),
            subtype_id: row.subtype_id.as_str(),
            element_source_identity:
                "manifest:abc/interface/equipmentdesignerview.gui#designer_equipment_entry",
            instance_token: tokens[ordinal],
            template: &template,
        })
        .collect::<Vec<_>>();
    let mut viewer = viewer();
    viewer.scroll_offset_pixels = (0, 0);
    viewer.visible_clip = Rect::new(0, 0, 200, 120);
    let plan = plan_observer_collection(
        41,
        &semantic,
        source(&grid),
        &bindings,
        viewer,
        &ComposeOptions::new(200, 120),
    )
    .expect("observer designer grid plan");
    assert_eq!(
        plan.rows().map(|row| row.final_rect()).collect::<Vec<_>>(),
        vec![
            Rect::new(7, 11, 15, 9),
            Rect::new(24, 11, 15, 9),
            Rect::new(7, 24, 15, 9),
            Rect::new(24, 24, 15, 9),
        ]
    );
}

#[test]
fn overlap_plan_compresses_to_file_extent_and_preserves_reverse_source_binding() {
    let overlap = root(
        r#"
        OverlappingElementsBoxType = {
          name = "portraits"
          position = { x = 0 y = 0 }
          size = { x = 50 y = 20 }
          format = right
          spacing = 5.0
          first_on_top = yes
          orientation = upper_left
        }
        "#,
    );
    let template = root(r#"containerWindowType = { name = "portrait" size = { x = 20 y = 10 } }"#);
    let semantic = semantic(&[
        ("p:a", "portrait"),
        ("p:b", "portrait"),
        ("p:c", "portrait"),
    ]);
    let bindings = semantic
        .rows
        .iter()
        .enumerate()
        .map(|(ordinal, row)| AuthoredRowBinding {
            stable_item_id: row.stable_item_id.as_str(),
            subtype_id: row.subtype_id.as_str(),
            element_source_identity: "manifest:abc#portrait",
            instance_token: match ordinal {
                0 => "portrait:0",
                1 => "portrait:1",
                _ => "portrait:2",
            },
            template: &template,
        })
        .collect::<Vec<_>>();
    let mut viewer = viewer();
    viewer.scroll_offset_pixels = (0, 0);
    viewer.visible_clip = Rect::new(0, 0, 200, 120);
    let plan = plan_observer_collection(
        41,
        &semantic,
        source(&overlap),
        &bindings,
        viewer,
        &ComposeOptions::new(200, 120),
    )
    .expect("overlap coordinates");
    assert_eq!(plan.family(), ObserverCollectionFamily::Overlap);
    assert_eq!(
        plan.rows().map(|row| row.final_rect()).collect::<Vec<_>>(),
        vec![
            Rect::new(60, 0, 20, 10),
            Rect::new(45, 0, 20, 10),
            Rect::new(30, 0, 20, 10),
        ]
    );
    assert_eq!(
        plan.rows()
            .map(|row| row.paint_order().local_paint_ordinal)
            .collect::<Vec<_>>(),
        vec![2, 1, 0]
    );
}

#[test]
fn overlap_malformed_spacing_and_first_on_top_fail_closed() {
    let template = root(r#"containerWindowType = { name = "item" size = { x = 20 y = 10 } }"#);
    let semantic = semantic(&[("item:a", "item")]);
    let bindings = [AuthoredRowBinding {
        stable_item_id: "item:a",
        subtype_id: "item",
        element_source_identity: "manifest:abc#item",
        instance_token: "item:0",
        template: &template,
    }];
    let mut still = viewer();
    still.scroll_offset_pixels = (0, 0);

    for (field, value, expected) in [
        ("spacing", "not_a_number", "spacing"),
        ("format", "{ bad = value }", "format"),
        ("first_on_top", "sometimes", "first_on_top"),
        ("first_on_top", "{ bad = value }", "first_on_top"),
    ] {
        let overlap = root(&format!(
            r#"
            OverlappingElementsBoxType = {{
              name = "bad_overlap"
              size = {{ x = 80 y = 20 }}
              {field} = {value}
            }}
            "#
        ));
        let error = plan_observer_collection(
            41,
            &semantic,
            source(&overlap),
            &bindings,
            still,
            &ComposeOptions::new(200, 120),
        )
        .expect_err("malformed overlap property must fail closed");
        assert!(error.message.contains(expected));
    }
}

#[test]
fn overlap_omitted_format_and_spacing_use_exact_left_and_one_defaults() {
    let overlap = root(
        r#"
        OverlappingElementsBoxType = {
          name = "default_overlap"
          position = { x = 4 y = 6 }
          size = { x = 80 y = 20 }
        }
        "#,
    );
    let template = root(r#"containerWindowType = { name = "item" size = { x = 20 y = 10 } }"#);
    let semantic = semantic(&[("item:a", "item"), ("item:b", "item")]);
    let bindings = [
        AuthoredRowBinding {
            stable_item_id: "item:a",
            subtype_id: "item",
            element_source_identity: "manifest:abc#item",
            instance_token: "item:0",
            template: &template,
        },
        AuthoredRowBinding {
            stable_item_id: "item:b",
            subtype_id: "item",
            element_source_identity: "manifest:abc#item",
            instance_token: "item:1",
            template: &template,
        },
    ];
    let mut viewer = viewer();
    viewer.scroll_offset_pixels = (0, 0);
    viewer.visible_clip = Rect::new(0, 0, 200, 120);
    let plan = plan_observer_collection(
        41,
        &semantic,
        source(&overlap),
        &bindings,
        viewer,
        &ComposeOptions::new(200, 120),
    )
    .expect("overlap defaults");
    assert_eq!(
        plan.rows().map(|row| row.final_rect()).collect::<Vec<_>>(),
        vec![Rect::new(4, 6, 20, 10), Rect::new(25, 6, 20, 10)]
    );
}

#[test]
fn overlap_singleton_zero_interval_compression_fails_only_on_the_active_axis() {
    let template = root(r#"containerWindowType = { name = "item" size = { x = 20 y = 10 } }"#);
    let semantic = semantic(&[("item:a", "item")]);
    let bindings = [AuthoredRowBinding {
        stable_item_id: "item:a",
        subtype_id: "item",
        element_source_identity: "manifest:abc#item",
        instance_token: "item:0",
        template: &template,
    }];
    let mut still = viewer();
    still.scroll_offset_pixels = (0, 0);

    for (format, width, height) in [("left", 10, 20), ("down", 20, 5)] {
        let overlap = root(&format!(
            r#"
            OverlappingElementsBoxType = {{
              name = "undersized"
              size = {{ x = {width} y = {height} }}
              format = {format}
            }}
            "#
        ));
        let error = plan_observer_collection(
            41,
            &semantic,
            source(&overlap),
            &bindings,
            still,
            &ComposeOptions::new(200, 120),
        )
        .expect_err("active-axis singleton compression must fail closed");
        assert!(error.message.contains("zero-interval compression"));
    }

    for (format, width, height) in [("left", 30, 5), ("down", 5, 20)] {
        let overlap = root(&format!(
            r#"
            OverlappingElementsBoxType = {{
              name = "inactive_axis_undersized"
              size = {{ x = {width} y = {height} }}
              format = {format}
            }}
            "#
        ));
        plan_observer_collection(
            41,
            &semantic,
            source(&overlap),
            &bindings,
            still,
            &ComposeOptions::new(200, 120),
        )
        .expect("inactive-axis undersize does not enter zero-interval compression");
    }
}

#[test]
fn incomplete_membership_subtype_drift_and_unauthenticated_scroll_fail_closed() {
    let list = root(r#"listBoxType = { name = "rows" size = { x = 100 y = 70 } spacing = 0 }"#);
    let template = root(r#"containerWindowType = { name = "row" size = { x = 80 y = 20 } }"#);
    let mut semantic = semantic(&[("row:a", "a")]);
    semantic.complete = false;
    let bindings = [AuthoredRowBinding {
        stable_item_id: "row:a",
        subtype_id: "a",
        element_source_identity: "manifest:abc#row",
        instance_token: "row:0",
        template: &template,
    }];
    let error = plan_observer_collection(
        41,
        &semantic,
        source(&list),
        &bindings,
        viewer(),
        &ComposeOptions::new(200, 120),
    )
    .expect_err("incomplete is not measured empty");
    assert!(error.message.contains("not explicitly complete"));

    semantic.complete = true;
    let subtype_drift = [AuthoredRowBinding {
        subtype_id: "different",
        ..bindings[0]
    }];
    let error = plan_observer_collection(
        41,
        &semantic,
        source(&list),
        &subtype_drift,
        viewer(),
        &ComposeOptions::new(200, 120),
    )
    .expect_err("subtype must bind exact template");
    assert!(error.message.contains("subtype differs"));

    let error = plan_observer_collection(
        41,
        &semantic,
        source(&list),
        &bindings,
        viewer(),
        &ComposeOptions::new(200, 120),
    )
    .expect_err("scroll beyond content must remain unavailable");
    assert!(error.message.contains("exceeds the content extent"));
}

#[test]
fn composition_is_bounded_and_authenticates_the_admitted_coordinate_space() {
    let list = root(
        r#"gridBoxType = {
          name = "acquired_variants_grid"
          size = { x = 100 y = 70 }
          slotsize = { x = 80 y = 20 }
          max_slots_horizontal = 1
          format = UPPER_LEFT
        }"#,
    );
    let template = root(
        r#"containerWindowType = { name = "designer_equipment_entry" size = { x = 80 y = 20 } }"#,
    );
    let semantic = semantic(&[("row:a", "a"), ("row:b", "a")]);
    let bindings = [
        AuthoredRowBinding {
            stable_item_id: "row:a",
            subtype_id: "a",
            element_source_identity:
                "manifest:abc/interface/equipmentdesignerview.gui#designer_equipment_entry",
            instance_token: "row:0",
            template: &template,
        },
        AuthoredRowBinding {
            stable_item_id: "row:b",
            subtype_id: "a",
            element_source_identity:
                "manifest:abc/interface/equipmentdesignerview.gui#designer_equipment_entry",
            instance_token: "row:1",
            template: &template,
        },
    ];
    let mut still = viewer();
    still.scroll_offset_pixels = (0, 0);
    let options = ComposeOptions::new(200, 120);
    let plan = plan_observer_collection(41, &semantic, source(&list), &bindings, still, &options)
        .expect("observer list plan");

    let resolver = AssetResolver::new(Vec::new());
    let registry = SpriteRegistry::new();
    let mut compositor = Compositor::new(&resolver, &registry);
    let inputs = [
        ObserverRowComposeInput {
            stable_item_id: "row:a",
            options: &options,
        },
        ObserverRowComposeInput {
            stable_item_id: "row:b",
            options: &options,
        },
    ];
    let composed = compose_observer_row_layers(&mut compositor, &plan, &inputs)
        .expect("bounded observer composition");
    assert_eq!(composed.output.image.width, 200);
    assert_eq!(composed.output.image.height, 120);
    assert_eq!(composed.output.image.pixels.len(), 200 * 120 * 4);
    assert_eq!(composed.painted_rows, vec!["row:a", "row:b"]);

    still.visible_clip = Rect::new(0, 0, 200, 15);
    let clipped_plan =
        plan_observer_collection(41, &semantic, source(&list), &bindings, still, &options)
            .expect("clipped observer list plan");
    let clipped = compose_observer_row_layers(&mut compositor, &clipped_plan, &inputs)
        .expect("fully clipped row is skipped");
    assert_eq!(clipped.painted_rows, vec!["row:a"]);

    let mismatched = ComposeOptions::new(201, 120);
    let bad_inputs = [
        ObserverRowComposeInput {
            stable_item_id: "row:a",
            options: &mismatched,
        },
        ObserverRowComposeInput {
            stable_item_id: "row:b",
            options: &mismatched,
        },
    ];
    let error = compose_observer_row_layers(&mut compositor, &plan, &bad_inputs)
        .expect_err("coordinate-space substitution must fail");
    assert!(error.message.contains("coordinate space differs"));
}

#[test]
fn composition_keeps_visible_descendants_that_extend_outside_the_row_root() {
    let grid = root(
        r#"gridBoxType = {
          name = "acquired_variants_grid"
          size = { x = 100 y = 70 }
          slotsize = { x = 80 y = 20 }
          max_slots_horizontal = 1
          format = UPPER_LEFT
        }"#,
    );
    let template = root(
        r#"containerWindowType = {
          name = "designer_equipment_entry"
          size = { x = 80 y = 20 }
          containerWindowType = {
            name = "visible_descendant"
            position = { x = 0 y = -15 }
            size = { x = 10 y = 10 }
          }
        }"#,
    );
    let semantic = semantic(&[("row:a", "a"), ("row:b", "a")]);
    let bindings = [
        AuthoredRowBinding {
            stable_item_id: "row:a",
            subtype_id: "a",
            element_source_identity:
                "manifest:abc/interface/equipmentdesignerview.gui#designer_equipment_entry",
            instance_token: "row:0",
            template: &template,
        },
        AuthoredRowBinding {
            stable_item_id: "row:b",
            subtype_id: "a",
            element_source_identity:
                "manifest:abc/interface/equipmentdesignerview.gui#designer_equipment_entry",
            instance_token: "row:1",
            template: &template,
        },
    ];
    let mut clipped = viewer();
    clipped.scroll_offset_pixels = (0, 0);
    clipped.visible_clip = Rect::new(0, 0, 200, 15);
    let options = ComposeOptions::new(200, 120);
    let plan = plan_observer_collection(41, &semantic, source(&grid), &bindings, clipped, &options)
        .expect("observer grid plan with out-of-root descendant");
    assert_eq!(
        plan.visible_rows()
            .map(|row| row.stable_item_id())
            .collect::<Vec<_>>(),
        vec!["row:a"],
        "root visibility is only a planning hint; composition must still inspect descendants",
    );

    let resolver = AssetResolver::new(Vec::new());
    let registry = SpriteRegistry::new();
    let mut compositor = Compositor::new(&resolver, &registry);
    let inputs = [
        ObserverRowComposeInput {
            stable_item_id: "row:a",
            options: &options,
        },
        ObserverRowComposeInput {
            stable_item_id: "row:b",
            options: &options,
        },
    ];
    let composed = compose_observer_row_layers(&mut compositor, &plan, &inputs)
        .expect("visible descendant keeps its row admitted");
    assert_eq!(composed.painted_rows, vec!["row:a", "row:b"]);
    assert!(composed
        .output
        .debug_rects
        .iter()
        .any(|debug| debug.name.ends_with("visible_descendant") && debug.y == 5));
}
