use super::*;
use crate::map::native_gui::tests::{Fixture, fixture, fixture_with, windows};

impl CombatResources for Fixture {
    fn flag_index(&self, key: &str) -> Result<i32, String> {
        Err(format!("the fixture has no recorded flag index for {key}"))
    }

    fn flag_texture(&self, _: &str, _: bool) -> Option<native_gui::Texture> {
        None
    }
}

#[test]
fn combat_mixed_grid_rows_are_counted_without_losing_compatible_entries() {
    let windows = windows(
        br#"guiTypes={
        containerWindowType={name=view size={width=400 height=200}
            gridBoxType={name=rows size={width=300 height=60}
                slotsize={width=30 height=20} max_slots_horizontal=3
                format=UPPER_LEFT add_horizontal=yes}}
        containerWindowType={name=entry size={width=20 height=10}
            iconType={name=icon spriteType=GFX_even}}
    }"#,
    );
    let resources = fixture();
    let root = windows.gui_type(b"view").unwrap();
    let values = Values::new();
    let entry_values = Values::from([("icon".into(), super::super::Widget::default())]);
    let modes = [
        None,
        Some((None, [20., 10.])),
        Some((Some([40., 20.]), [20., 10.])),
    ];
    for valid in modes {
        for invalid in modes.into_iter().filter(|mode| *mode != valid) {
            let mut retained = Retained::new("view", root, [800, 600], &resources).unwrap();
            let rows = [valid, invalid, invalid, valid]
                .into_iter()
                .enumerate()
                .map(|(index, explicit)| Row {
                    key: index.to_string(),
                    template: "entry",
                    values: &entry_values,
                    explicit,
                })
                .collect();
            let mut refused = BTreeMap::new();
            let prepared = retained
                .prepare(
                    root,
                    &values,
                    vec![Group {
                        path: "rows".into(),
                        rows,
                    }],
                    &windows,
                    &resources,
                    Tick { time: 0., dt: 0. },
                    1,
                    true,
                    true,
                    true,
                    &mut refused,
                )
                .unwrap();
            assert_eq!(refused.len(), 1, "{refused:?}");
            let (reason, count) = refused.first_key_value().unwrap();
            assert!(
                reason.contains(&format!(
                    "Combat GUI {}: mixed grid placements",
                    root.path_named("rows").unwrap().path
                )),
                "{reason}"
            );
            assert_eq!(*count, 2);
            assert_eq!(
                retained.places["rows"]
                    .iter()
                    .map(|(index, _)| *index)
                    .collect::<Vec<_>>(),
                [0, 3],
            );
            assert_eq!(prepared.items.len(), 1);
            assert_eq!(prepared.items[0].len(), 2);
        }
    }
}

#[test]
fn naval_loss_grid_uses_authored_slots_and_runtime_bounds() {
    use super::super::{Widget, binding::NavalEntry, naval_details::entry_offset};
    let windows = windows(
        br#"guiTypes={
        containerWindowType={name=navalcombatview size={width=400 height=200}
            gridBoxType={name=losses size={width=330 height=40}
                slotsize={width=100 height=27} max_slots_horizontal=3
                format=UPPER_LEFT add_horizontal=yes}}
        containerWindowType={name=navalcombat_ship_entry position={x=1 y=0}
            size={width=1 height=3} iconType={name=icon spriteType=GFX_even}}
    }"#,
    );
    let resources = fixture();
    let mut bindings = Bindings::default();
    for index in 0..4 {
        bindings.naval_entries.push(NavalEntry {
            side: 0,
            unknown: false,
            grid: "losses".into(),
            offset: entry_offset("sunk", 0.0, index as f32 * 23.0),
            size: [20.0, 30.0],
            values: Values::from([("icon".into(), Widget::default())]),
            battle_slots: vec![index],
        });
    }
    let mut gui = Windows::default();
    let drawn = gui
        .draw(
            &windows,
            &resources,
            &bindings,
            1,
            Selection::Naval(7),
            [0.0; 2],
            false,
            &mut None,
            [800.0, 600.0],
            true,
            Tick { time: 0.0, dt: 0.0 },
            false,
        )
        .unwrap();
    assert!(drawn.refused.is_empty(), "{:?}", drawn.refused);
    assert_eq!(
        drawn
            .batch
            .quads
            .iter()
            .map(|quad| quad.exact.as_ref().unwrap().corners[0])
            .collect::<Vec<_>>(),
        [[1., 0.], [101., 0.], [201., 0.], [1., 27.]]
    );
    let kept = gui.root.as_mut().unwrap();
    assert_eq!(
        kept.places["losses"]
            .iter()
            .map(|(_, at)| *at)
            .collect::<Vec<_>>(),
        [[1, 0], [101, 0], [201, 0], [1, 27]]
    );
    let root = windows.gui_type(b"navalcombatview").unwrap();
    // Runtime sprite size, not the tiny authored entry, determines overflow.
    let layout = kept
        .instance
        .layout_with_regions(root, &resources, None, true)
        .unwrap()
        .unwrap();
    let region = layout
        .regions
        .iter()
        .find(|region| region.path.ends_with(".losses"))
        .unwrap();
    assert_eq!(&region.rect[2..], &[330., 57.]);
}

#[test]
fn combat_button_enabled_binding_draws_native_disabled_state_and_resets() {
    use super::super::Widget;
    use crate::map::native_effects::Pixel;

    let windows = windows(
        br#"guiTypes={containerWindowType={name=view size={width=40 height=20}
        buttonType={name=disengage spriteType=GFX_four}
        buttonType={name=short_strip position={x=20 y=0} spriteType=GFX_odd}}}"#,
    );
    let resources = fixture();
    let view = windows.gui_type(b"view").unwrap();
    let mut instance = Instance::construct(view, [800, 600], &resources).unwrap();
    instance.gui_show(view, view).unwrap();
    for enabled in [Some(false), Some(true), Some(false), None] {
        let values = enabled
            .map(|enabled| {
                Values::from([
                    (
                        "disengage".into(),
                        Widget {
                            enabled: Some(enabled),
                            ..Default::default()
                        },
                    ),
                    (
                        "short_strip".into(),
                        Widget {
                            enabled: Some(enabled),
                            ..Default::default()
                        },
                    ),
                ])
            })
            .unwrap_or_default();
        let mut refused = BTreeMap::new();
        apply(&mut instance, view, &values, &resources, &mut refused);
        assert!(refused.is_empty(), "{refused:?}");
        instance
            .tick(view, &resources, Tick { time: 0., dt: 0. })
            .unwrap();
        let layout = instance.layout(view, &resources, None).unwrap().unwrap();
        assert_eq!(layout.parts.len(), 2);
        let disabled = enabled == Some(false);
        for (part, frames) in layout.parts.iter().zip([4., 3.]) {
            let native_gui::PartKind::Sprite { offsets, .. } = &part.kind else {
                panic!("expected a button sprite");
            };
            // DrawWithStateEffect@14222D0D0: disabled state 4 uses frame 4
            // when present, otherwise frame 1, and the Disable technique.
            assert_eq!(
                offsets[0],
                if disabled && frames == 4. {
                    3. / frames
                } else {
                    0.
                }
            );
            let Some((Pixel::Sprite(program), ..)) = part.shading.program else {
                panic!("expected the native button-state program");
            };
            assert_eq!(program.grey, disabled);
        }
        assert_eq!(
            instance
                .control(view, view.path_named("disengage").unwrap())
                .unwrap()
                .enabled,
            enabled
        );
    }
}

#[test]
fn exile_template_override_belongs_to_its_grid_entry() {
    let values = Values::from([(
        "traits".into(),
        super::super::Widget {
            entry_template: Some("unit_leader_trait_entry".into()),
            entry_templates: [(1, "unit_leader_exile_trait_entry".into())]
                .into_iter()
                .collect(),
            entries: vec![Values::new(), Values::new()],
            ..Default::default()
        },
    )]);
    let groups = repeated(&values);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].path, "traits");
    assert_eq!(groups[0].rows[0].template, "unit_leader_trait_entry");
    assert_eq!(groups[0].rows[1].template, "unit_leader_exile_trait_entry");
    assert!(
        groups[0]
            .rows
            .iter()
            .all(|row| !row.values.contains_key(""))
    );
}

#[test]
fn hover_geometry_uses_native_container_clip() {
    let region = Region {
        path: "window.box.text".into(),
        rect: [-4.0, 2.0, 30.0, 20.0],
        clip: Some([0, 0, 12, 15]),
        ..Region::default()
    };
    assert_eq!(
        region_rect(&region, [30, 40]),
        Some([30.0, 42.0, 12.0, 13.0])
    );
}

#[test]
fn native_paths_include_container_backgrounds() {
    let gui = installed_content::gui::native::read(b"guiTypes={containerWindowType={name=view size={width=20 height=20} background={name=Background spriteType=bg} containerWindowType={name=inner size={width=5 height=5} iconType={name=icon spriteType=s}}}}");
    let root = gui
        .templates()
        .iter()
        .find(|root| root.is_named(b"view"))
        .unwrap();
    assert!(root.path_named("Background").is_some());
    assert!(root.path_named("inner/icon").is_some());
    assert!(root.path_named("INNER/ICON").is_some());
}

#[test]
fn unchanged_generation_keeps_runtime_controls_until_refresh() {
    let windows = windows(
        br#"guiTypes={containerWindowType={name=view size={width=20 height=20}
            iconType={name=icon spriteType=GFX_even}}}"#,
    );
    let resources = fixture();
    let view = windows.gui_type(b"view").unwrap();
    let icon = view.path_named("icon").unwrap();
    let values = Values::from([("icon".into(), super::super::Widget::default())]);
    let mut retained = Retained::new("view", view, [800, 600], &resources).unwrap();
    for generation in [1, 1, 2] {
        let mut refused = BTreeMap::new();
        retained
            .prepare(
                view,
                &values,
                Vec::new(),
                &windows,
                &resources,
                Tick { time: 0.0, dt: 0.0 },
                generation,
                false,
                false,
                true,
                &mut refused,
            )
            .unwrap();
        assert!(refused.is_empty(), "{refused:?}");
        let bound = retained.instance.control(view, icon).unwrap();
        if generation == 1 && bound.offset == Some([0, 0]) {
            bound.offset = Some([43, 7]);
        } else if generation == 1 {
            assert_eq!(bound.offset, Some([43, 7]));
        } else {
            assert_eq!(bound.offset, Some([0, 0]));
        }
    }
}

#[test]
fn combat_prepare_draws_percentage_grid_slots_natively() {
    let windows = windows(
        br#"guiTypes={
            containerWindowType={name=entry size={width=20 height=10}
                iconType={name=icon spriteType=GFX_even}}
            containerWindowType={name=view size={width=200 height=100}
                gridBoxType={name=rows size={width=200 height=100}
                    slotsize={width=50% height=25%} max_slots_horizontal=2
                    format=UPPER_LEFT add_horizontal=yes}}
        }"#,
    );
    let resources = fixture();
    let view = windows.gui_type(b"view").unwrap();
    let entry = Values::from([(
        "icon".into(),
        super::super::Widget {
            tooltip: Some("same hover".into()),
            ..super::super::Widget::default()
        },
    )]);
    let values = Values::from([(
        "rows".into(),
        super::super::Widget {
            entry_template: Some("entry".into()),
            entries: vec![entry.clone(), entry.clone(), entry],
            ..super::super::Widget::default()
        },
    )]);
    let mut retained = Retained::new("view", view, [800, 600], &resources).unwrap();
    let mut refused = BTreeMap::new();
    let prepared = retained
        .prepare(
            view,
            &values,
            Vec::new(),
            &windows,
            &resources,
            Tick { time: 0.0, dt: 0.0 },
            1,
            true,
            true,
            true,
            &mut refused,
        )
        .unwrap();
    assert!(refused.is_empty(), "{refused:?}");
    let mut drawn = Drawn::default();
    assert!(draw_prepared(
        &prepared.layout,
        &prepared.items,
        [30, 40],
        &resources,
        &mut drawn
    ));
    assert_eq!(
        drawn
            .batch
            .quads
            .iter()
            .map(|quad| quad.exact.as_ref().unwrap().corners)
            .collect::<Vec<_>>(),
        [
            [[30.0, 40.0], [50.0, 40.0], [50.0, 50.0], [30.0, 50.0]],
            [[130.0, 40.0], [150.0, 40.0], [150.0, 50.0], [130.0, 50.0]],
            [[30.0, 65.0], [50.0, 65.0], [50.0, 75.0], [30.0, 75.0]],
        ]
    );
    append_hovers(&mut drawn.tooltips, prepared.hovers, [30, 40]);
    assert_eq!(drawn.tooltips.len(), 3);
}

#[test]
fn combat_prepare_composes_nested_entries_clips_hovers_and_counts_missing_templates() {
    let windows = windows(
        br#"guiTypes={
            containerWindowType={name=leaf size={width=20 height=10}
                iconType={name=icon spriteType=GFX_even position={x=0 y=2}}}
            containerWindowType={name=entry size={width=20 height=15} clipping=yes
                gridBoxType={name=kids size={width=20 height=10}
                    slotsize={width=20 height=10} max_slots_horizontal=1}}
            containerWindowType={name=view size={width=25 height=14} clipping=yes
                gridBoxType={name=rows size={width=40 height=30}
                    slotsize={width=20 height=15} max_slots_horizontal=2
                    format=UPPER_LEFT add_horizontal=yes}
                gridBoxType={name=missing size={width=20 height=10}
                    slotsize={width=20 height=10} max_slots_horizontal=1}}
        }"#,
    );
    let resources = fixture();
    let view = windows.gui_type(b"view").unwrap();
    let leaf = Values::from([(
        "icon".into(),
        super::super::Widget {
            tooltip: Some("same hover".into()),
            ..super::super::Widget::default()
        },
    )]);
    let entry = Values::from([(
        "kids".into(),
        super::super::Widget {
            entry_template: Some("leaf".into()),
            entries: vec![leaf],
            ..super::super::Widget::default()
        },
    )]);
    let values = Values::from([
        (
            "rows".into(),
            super::super::Widget {
                entry_template: Some("entry".into()),
                entries: vec![entry.clone(), entry],
                ..super::super::Widget::default()
            },
        ),
        (
            "missing".into(),
            super::super::Widget {
                entry_template: Some("absent".into()),
                entries: vec![Values::default(), Values::default()],
                ..super::super::Widget::default()
            },
        ),
    ]);
    let mut retained = Retained::new("view", view, [800, 600], &resources).unwrap();
    let mut refused = BTreeMap::new();
    let prepared = retained
        .prepare(
            view,
            &values,
            Vec::new(),
            &windows,
            &resources,
            Tick { time: 0.0, dt: 0.0 },
            1,
            true,
            true,
            true,
            &mut refused,
        )
        .unwrap();
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert_eq!(
        refused["Combat GUI missing: the content stack declares no absent template (142247660)"],
        2
    );
    let mut held_refused = BTreeMap::new();
    retained
        .prepare(
            view,
            &values,
            Vec::new(),
            &windows,
            &resources,
            Tick { time: 0.0, dt: 0.0 },
            1,
            false,
            true,
            true,
            &mut held_refused,
        )
        .unwrap();
    assert_eq!(held_refused, refused);
    let mut drawn = Drawn::default();
    assert!(draw_prepared(
        &prepared.layout,
        &prepared.items,
        [30, 40],
        &resources,
        &mut drawn
    ));
    assert_eq!(drawn.batch.quads.len(), 2);
    assert_eq!(
        drawn
            .batch
            .quads
            .iter()
            .map(|quad| quad.exact.as_ref().unwrap().corners)
            .collect::<Vec<_>>(),
        [
            [[30.0, 42.0], [50.0, 42.0], [50.0, 52.0], [30.0, 52.0]],
            [[50.0, 42.0], [70.0, 42.0], [70.0, 52.0], [50.0, 52.0]],
        ]
    );
    assert_eq!(
        drawn
            .batch
            .quads
            .iter()
            .map(|quad| quad.exact.as_ref().unwrap().scissor)
            .collect::<Vec<_>>(),
        [Some([30, 40, 50, 54]), Some([50, 40, 55, 54]),]
    );
    append_hovers(&mut drawn.tooltips, prepared.hovers, [30, 40]);
    assert_eq!(
        drawn
            .tooltips
            .iter()
            .map(|hit| hit.rect)
            .collect::<Vec<_>>(),
        [[30.0, 42.0, 20.0, 10.0], [50.0, 42.0, 5.0, 10.0],]
    );
    assert!(std::sync::Arc::ptr_eq(
        drawn.tooltips[0].text.as_ref().unwrap(),
        drawn.tooltips[1].text.as_ref().unwrap(),
    ));
}

#[test]
fn combat_prepare_sizes_escape_entries_from_the_native_drawable() {
    let windows = windows(
        br#"guiTypes={
            containerWindowType={name=entry size={width=1 height=3}
                iconType={name=icon spriteType=GFX_even}
                iconType={name=escape_progress spriteType=GFX_bar orientation=CENTER_UP}
                iconType={name=escape_icon_left spriteType=GFX_even orientation=CENTER_UP}}
            containerWindowType={name=view size={width=100 height=100}
                gridBoxType={name=rows size={width=100 height=100}
                    slotsize={width=1 height=1} max_slots_horizontal=1}}
        }"#,
    );
    let resources = fixture();
    let view = windows.gui_type(b"view").unwrap();
    let entry = windows.gui_type(b"entry").unwrap();
    let progress = entry.path_named("escape_progress").unwrap();
    let row = Values::from([
        (
            "escape_progress".into(),
            super::super::Widget {
                fill: Some(0.5),
                ..super::super::Widget::default()
            },
        ),
        ("escape_icon_left".into(), super::super::Widget::default()),
    ]);
    let values = Values::default();
    let mut retained = Retained::new("view", view, [800, 600], &resources).unwrap();
    let mut refused = BTreeMap::new();
    for _ in 0..2 {
        let groups = vec![Group {
            path: "rows".into(),
            rows: vec![Row {
                key: "0".into(),
                template: "entry",
                values: &row,
                explicit: Some((Some([0.0; 2]), [20.0, 10.0])),
            }],
        }];
        retained
            .prepare(
                view,
                &values,
                groups,
                &windows,
                &resources,
                Tick { time: 0.0, dt: 0.0 },
                1,
                true,
                true,
                true,
                &mut refused,
            )
            .unwrap();
        let kept = retained.entries.get_mut("rows/0").unwrap();
        // GFX_bar is fourteen pixels high; native resize overwrites the authored 1×3.
        assert_eq!(kept.instance.content_size(entry, entry).unwrap(), [20, 24]);
        assert_eq!(
            kept.instance.control(entry, progress).unwrap().offset,
            Some([10, 0])
        );
    }
    assert!(refused.is_empty(), "{refused:?}");
}

#[test]
fn combat_controls_keep_native_scroll_paths_axes_and_popup_mask() {
    let body = r#"size={width=300 height=160}
            containerWindowType={name=BattleReport size={width=100 height=80}
                verticalScrollbar=shared autohide_scrollbars=no}
            containerWindowType={name=FleetComposition position={x=120 y=0} size={width=100 height=80}
                verticalScrollbar=shared autohide_scrollbars=no}
            containerWindowType={name=LaterRows position={x=0 y=90} size={width=100 height=60}
                horizontalScrollbar=across autohide_scrollbars=no}"#;
    let document = format!(
        r#"guiTypes={{
            extendedScrollbarType={{name=shared size={{width=10 height=50}}
                background={{name=bg spriteType=GFX_even}}
                track={{name=track spriteType=GFX_bar3x17}}
                slider={{name=slider spriteType=GFX_bar3x17}}
                decreaseButton={{name=less spriteType=GFX_four}}
                increaseButton={{name=more spriteType=GFX_four position={{x=-10 y=-10}}}}}}
            extendedScrollbarType={{name=across horizontal=yes size={{width=50 height=10}}
                background={{name=bg spriteType=GFX_even}}
                track={{name=track spriteType=GFX_bar3x17}}
                slider={{name=slider spriteType=GFX_bar3x17}}
                decreaseButton={{name=less spriteType=GFX_four}}
                increaseButton={{name=more spriteType=GFX_four position={{x=-10 y=-10}}}}}}
            containerWindowType={{name=navalcombatview {body}}}
            containerWindowType={{name=navalcombatresultswindow {body}}}
            containerWindowType={{name=tactics_list_view {body}}}
        }}"#
    );
    let resources = fixture_with(installed_content::gui::native::read(document.as_bytes()));
    let popup = resources.gui_type(b"tactics_list_view").unwrap();
    let mut popup_instance = Instance::construct(popup, [800, 600], &resources).unwrap();
    popup_instance.gui_show(popup, popup).unwrap();
    let popup_layout = popup_instance
        .layout_with_regions(popup, &resources, None, true)
        .unwrap()
        .unwrap();
    for name in [b"navalcombatview".as_slice(), b"navalcombatresultswindow"] {
        let view = resources.gui_type(name).unwrap();
        let mut instance = Instance::construct(view, [800, 600], &resources).unwrap();
        instance.gui_show(view, view).unwrap();
        let layout = instance
            .layout_with_regions(view, &resources, None, true)
            .unwrap()
            .unwrap();
        let mut drawn = Drawn::default();
        controls(view, &layout.regions, [30, 40], &mut drawn, false);
        assert_eq!(
            drawn
                .lists
                .iter()
                .map(|(_, target)| (target.path.as_str(), target.vertical))
                .collect::<Vec<_>>(),
            [
                ("BattleReport", true),
                ("FleetComposition", true),
                ("LaterRows", false),
            ]
        );
        assert_eq!(
            drawn
                .scroll_buttons
                .iter()
                .map(|(_, target, direction)| (target.path.as_str(), target.vertical, *direction))
                .collect::<Vec<_>>(),
            [
                ("BattleReport", true, 1),
                ("BattleReport", true, -1),
                ("FleetComposition", true, 1),
                ("FleetComposition", true, -1),
                ("LaterRows", false, 1),
                ("LaterRows", false, -1),
            ]
        );
        let [x, y, _, _] = drawn.lists[0].0;
        let list_point = [x + 1.0, y + 1.0];
        let [x, y, w, h] = drawn.scroll_buttons[0].0;
        let button_point = [x + w / 2.0, y + h / 2.0];
        assert_eq!(drawn.list_at(list_point).unwrap().path, "BattleReport");
        assert_eq!(
            drawn.scroll_action(button_point).unwrap().0.path,
            "BattleReport"
        );
        drawn.tactics_rect = Some([30.0, 40.0, 300.0, 160.0]);
        assert!(drawn.list_at(list_point).is_none());
        assert!(drawn.scroll_action(button_point).is_none());
        controls(popup, &popup_layout.regions, [30, 40], &mut drawn, true);
        let target = drawn.list_at(list_point).unwrap();
        assert!(target.popup && target.vertical);
        assert_eq!(target.path, "BattleReport");
        let (target, direction) = drawn.scroll_action(button_point).unwrap();
        assert!(target.popup && target.vertical);
        assert_eq!((target.path.as_str(), direction), ("BattleReport", 1));
    }
}

#[test]
fn combat_hit_prepare_does_not_tick_before_native_scroll_input() {
    let source = br#"guiTypes={
            extendedScrollbarType={name=sb size={width=10 height=50} smooth_scrolling=0.5
                background={name=bg spriteType=GFX_even}
                track={name=track spriteType=GFX_bar3x17}
                slider={name=slider spriteType=GFX_bar3x17}
                decreaseButton={name=less spriteType=GFX_four}
                increaseButton={name=more spriteType=GFX_four position={x=-10 y=-10}}}
            containerWindowType={name=entry size={width=60 height=40}
                iconType={name=icon spriteType=GFX_even}}
            containerWindowType={name=view size={width=100 height=80}
                background={spriteType=GFX_even} verticalScrollbar=sb
                vertical_scroll_step=40 smooth_scrolling=yes
                gridBoxType={name=rows size={width=100 height=80}
                    slotsize={width=100 height=40} max_slots_horizontal=1}}
        }"#;
    let windows = windows(source);
    let resources = fixture_with(installed_content::gui::native::read(source));
    let view = windows.gui_type(b"view").unwrap();
    let values = Values::from([(
        "rows".into(),
        super::super::Widget {
            entry_template: Some("entry".into()),
            entries: vec![Values::default(); 5],
            ..super::super::Widget::default()
        },
    )]);
    let mut retained = Retained::new("view", view, [800, 600], &resources).unwrap();
    let mut refused = BTreeMap::new();
    let tick = Tick {
        time: 1.0,
        dt: 0.016,
    };
    let initial = retained
        .prepare(
            view,
            &values,
            Vec::new(),
            &windows,
            &resources,
            tick,
            1,
            false,
            false,
            true,
            &mut refused,
        )
        .unwrap();
    assert_eq!(initial.layout.holders[0].origin, [0.0, 0.0]);
    assert_eq!(
        retained.instance.scroll_range(view, view, true).unwrap(),
        Some([0.0, 120.0])
    );
    retained
        .instance
        .set_scroll_target(view, view, true, 80.0, &resources)
        .unwrap();
    let hit = retained
        .prepare(
            view,
            &values,
            Vec::new(),
            &windows,
            &resources,
            tick,
            1,
            false,
            false,
            true,
            &mut refused,
        )
        .unwrap();
    assert_eq!(hit.layout.holders[0].origin, [0.0, 0.0]);
    retained.pending.push(ScrollAction {
        target: ScrollTarget {
            popup: false,
            path: String::new(),
            vertical: true,
        },
        direction: 1,
        wheel: false,
    });
    let drawn = retained
        .prepare(
            view,
            &values,
            Vec::new(),
            &windows,
            &resources,
            tick,
            1,
            true,
            false,
            true,
            &mut refused,
        )
        .unwrap();
    // Arrow input reads current zero, sets forty, then one smoothing tick reaches twenty.
    assert_eq!(drawn.layout.holders[0].origin, [0.0, -20.0]);
    assert!(retained.pending.is_empty());
    let peek = retained
        .prepare(
            view,
            &values,
            Vec::new(),
            &windows,
            &resources,
            tick,
            1,
            false,
            false,
            true,
            &mut refused,
        )
        .unwrap();
    assert_eq!(peek.layout.holders[0].origin, [0.0, -20.0]);
    assert!(refused.is_empty(), "{refused:?}");
}

#[test]
fn combat_prepare_resolves_bound_control_and_holder_names_without_case() {
    let windows = windows(
        br#"guiTypes={
            containerWindowType={name=Entry size={width=100 height=40}
                iconType={name=Icon spriteType=GFX_even}
                instantTextBoxType={name=Title font=Main_14 text=EXAMPLE
                    position={x=5 y=3} maxWidth=40 maxHeight=20}}
            containerWindowType={name=View size={width=100 height=80}
                gridBoxType={name=Rows size={width=100 height=80}
                    slotsize={width=100 height=40} max_slots_horizontal=1}
                buttonType={name=Close_Button spriteType=GFX_even position={x=80 y=0}}
                buttonType={name=Tactics_List_Button spriteType=GFX_even position={x=80 y=20}}}
        }"#,
    );
    let resources = fixture();
    let view = windows.gui_type(b"view").unwrap();
    let entry = Values::from([
        (
            "icon".into(),
            super::super::Widget {
                sprite: Some("GFX_odd".into()),
                frame: Some(0),
                tooltip: Some("bound icon".into()),
                ..super::super::Widget::default()
            },
        ),
        (
            "title".into(),
            super::super::Widget {
                text: Some("abc".into()),
                tooltip: Some("bound title".into()),
                ..super::super::Widget::default()
            },
        ),
    ]);
    let values = Values::from([(
        "rows".into(),
        super::super::Widget {
            entry_template: Some("ENTRY".into()),
            entries: vec![entry.clone(), entry],
            ..super::super::Widget::default()
        },
    )]);
    let mut retained = Retained::new("view", view, [800, 600], &resources).unwrap();
    let mut refused = BTreeMap::new();
    let prepared = retained
        .prepare(
            view,
            &values,
            Vec::new(),
            &windows,
            &resources,
            Tick { time: 0.0, dt: 0.0 },
            1,
            true,
            true,
            true,
            &mut refused,
        )
        .unwrap();
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(prepared.items.len(), 1);
    assert_eq!(prepared.items[0].len(), 4);
    let texts: Vec<_> = prepared.items[0]
        .iter()
        .filter(|part| part.path == "Entry.Title")
        .collect();
    assert_eq!(texts.len(), 2);
    for part in texts {
        let native_gui::PartKind::Text { glyphs, .. } = &part.kind else {
            panic!("bound native text");
        };
        assert_eq!(glyphs.len(), 3);
    }
    let mut drawn = Drawn::default();
    assert!(draw_prepared(
        &prepared.layout,
        &prepared.items,
        [0; 2],
        &resources,
        &mut drawn
    ));
    assert_eq!(drawn.batch.quads.len(), 10);
    assert_eq!(
        drawn.batch.quads[0].exact.as_ref().unwrap().corners,
        [[0.0, 0.0], [15.0, 0.0], [15.0, 9.0], [0.0, 9.0]]
    );
    assert_eq!(
        drawn.batch.quads[4].exact.as_ref().unwrap().corners,
        [[0.0, 40.0], [15.0, 40.0], [15.0, 49.0], [0.0, 49.0]]
    );
    append_hovers(&mut drawn.tooltips, prepared.hovers, [0; 2]);
    assert_eq!(
        drawn
            .tooltips
            .iter()
            .map(|hit| hit.text.as_deref())
            .collect::<Vec<_>>(),
        [
            Some("bound icon"),
            Some("bound title"),
            Some("bound icon"),
            Some("bound title"),
        ]
    );
    controls(view, &prepared.layout.regions, [0; 2], &mut drawn, false);
    assert_eq!(drawn.close, Some([80.0, 0.0, 20.0, 10.0]));
    assert_eq!(drawn.tactics_button, Some([80.0, 20.0, 20.0, 10.0]));
}
