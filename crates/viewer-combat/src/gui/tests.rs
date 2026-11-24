use super::*;
use std::sync::Arc;
use viewer_native_gui::test_support::gui::{Fixture, fixture, fixture_with, windows};

thread_local! {
    static ARRIVED: std::cell::Cell<(Option<native_gui::Texture>, u64)> = const { std::cell::Cell::new((None, 0)) };
    static LOOKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl CombatResources for Fixture {
    fn art_version(&self) -> u64 {
        ARRIVED.with(|arrived| arrived.get().1)
    }

    fn flag_index(&self, key: &str) -> Result<i32, String> {
        Err(format!("the fixture has no recorded flag index for {key}"))
    }

    fn flag_texture(&self, _: &str, _: bool) -> Option<native_gui::Texture> {
        LOOKS.with(|looks| looks.set(looks.get() + 1));
        ARRIVED.with(|arrived| arrived.get().0)
    }

    fn button_sounds(
        &self,
        _: &content_reader_gui::gui::native::Button,
    ) -> content_reader_gui::gui::native::ButtonSounds {
        Default::default()
    }
}

#[test]
fn retained_null_tactic_keeps_the_previous_drawable_and_initial_authored_image() {
    content_reader_script::test_game::enter();
    use super::super::{presentation::land_tactics, tactics::Content};
    use replay_data::rows::LandCombatSideDetails;
    let windows = windows(
        b"guiTypes={containerWindowType={name=view size={width=100 height=100}
        iconType={name=attacker_tactic spriteType=GFX_even}
        iconType={name=defender_tactic spriteType=GFX_even}
        iconType={name=attacker_countered_tactic_overlay spriteType=GFX_even}
        iconType={name=defender_countered_tactic_overlay spriteType=GFX_even}}}",
    );
    let root = windows.gui_type(b"view").unwrap();
    let mut resources = fixture();
    resources.sprites.insert(
        "combat_tactic:fixture",
        resources.sprites["GFX_odd"].clone(),
    );
    let content = Content::from_database(content_reader_military::combat_tactics::parse(
        b"fixture={picture=fixture}",
        &content_reader_script::key_names::KeyNames::default(),
    ));
    let null = LandCombatSideDetails {
        tactic_kind: 1,
        ..Default::default()
    };
    let real = LandCombatSideDetails {
        tactic_kind: 2,
        tactic_id: 1,
        tactic_token: b"fixture".to_vec(),
        ..Default::default()
    };
    for first in [null.clone(), real] {
        let mut retained = Retained::new("view", root, [800, 600], &resources).unwrap();
        let expected = if first.tactic_kind == 1 {
            b"GFX_even".as_slice()
        } else {
            b"combat_tactic:fixture".as_slice()
        };
        for (generation, side) in [first, null.clone()].into_iter().enumerate() {
            let mut bindings = Bindings::default();
            land_tactics(
                &mut bindings,
                &|name| resources.sprite(name.as_bytes()).is_some(),
                &content,
                false,
                &[side, null.clone()],
            );
            let mut refused = BTreeMap::new();
            let prepared = retained
                .prepare(
                    root,
                    &bindings.values,
                    Vec::new(),
                    &windows,
                    &resources,
                    Tick {
                        time: generation as f32,
                        dt: 1.0,
                    },
                    generation as u64,
                    false,
                    false,
                    true,
                    None,
                    &mut refused,
                )
                .unwrap();
            assert!(refused.is_empty(), "{refused:?}");
            let part = prepared
                .layout
                .parts
                .iter()
                .find(|part| part.path == "view.attacker_tactic")
                .unwrap();
            let native_gui::PartKind::Sprite { sprite, .. } = &part.kind else {
                panic!("tactic sprite");
            };
            assert_eq!(sprite, expected);
        }
    }
}

#[test]
fn result_controls_include_both_close_buttons_outside_panels_and_text_scrollbars() {
    content_reader_script::test_game::enter();
    let resources = fixture_with(content_reader_gui::gui::native::read(br#"guiTypes={
        scrollbarType={name=details_bar size={x=10 y=60} minValue=0 maxValue=1 stepSize=1
            rightButton=up leftButton=down track=track slider=slider
            guiButtonType={name=up spriteType=GFX_even}
            guiButtonType={name=down spriteType=GFX_even}
            guiButtonType={name=track spriteType=GFX_bar3x17}
            guiButtonType={name=slider spriteType=GFX_bar3x17}}
        containerWindowType={name=navalcombatresultswindow size={width=300 height=160} clipping=no
            buttonType={name=close_button position={x=5 y=5} quadTextureSprite=GFX_even}
            buttonType={name=dismiss_button position={x=80 y=5} quadTextureSprite=GFX_even}
            buttonType={name=open_details position={x=140 y=5} quadTextureSprite=GFX_even}
            containerWindowType={name=attacker_details position={x=-100 y=0} size={width=90 height=100}
                instantTextBoxType={name=text font=Main_14 maxWidth=50 maxHeight=20 scrollbarType=details_bar}}
            containerWindowType={name=defender_details position={x=310 y=0} size={width=90 height=100}}
        }}"#));
    let root = resources.gui_type(b"navalcombatresultswindow").unwrap();
    let text = root.path_named("attacker_details/text").unwrap();
    let mut instance = Instance::construct(root, [800, 600], &resources).unwrap();
    instance.gui_show(root, root).unwrap();
    instance
        .set_text(root, text, "aaaa\nbbbb\ncccc\ndddd", &resources)
        .unwrap();
    let layout = instance
        .layout_with_regions(root, &resources, None, true)
        .unwrap()
        .unwrap();
    let mut drawn = Drawn {
        rect: [100., 100., 300., 160.],
        ..Default::default()
    };
    controls(root, &layout.regions, [100, 100], &mut drawn, false);
    assert!(drawn.closes([106., 106.]));
    assert!(drawn.closes([181., 106.]));
    assert!(drawn.result_details_button.is_some());
    assert!(
        drawn.contains([11., 110.]),
        "the outside detail panel consumes map input"
    );
    let target = drawn.list_at([1., 101.]).unwrap();
    assert_eq!(target.path, "attacker_details/text");
    assert!(target.vertical);
    let directions: Vec<_> = drawn
        .scroll_buttons
        .iter()
        .map(|(_, target, direction)| (target.path.as_str(), *direction))
        .collect();
    assert_eq!(
        directions,
        [("attacker_details/text", -1), ("attacker_details/text", 1)]
    );
}

#[test]
fn retained_result_mixed_templates_keep_nested_air_loss_counts_and_tint() {
    content_reader_script::test_game::enter();
    use super::super::Widget;
    let windows = windows(
        br#"guiTypes={
        containerWindowType={name=ship size={width=30 height=20}
            iconType={name=fleet spriteType=GFX_even}}
        containerWindowType={name=air size={width=30 height=20}
            iconType={name=plane spriteType=GFX_even}}
        containerWindowType={name=air_count size={width=30 height=20}
            instantTextBoxType={name=number font=Main_14 maxWidth=30 maxHeight=20}}
        containerWindowType={name=air_loss size={width=60 height=20} clipping=no
            iconType={name=equipment spriteType=GFX_even}
            gridBoxType={name=lost_air_grid position={x=30 y=0} size={width=30 height=20}
                slotsize={width=30 height=20} max_slots_horizontal=1}}
        containerWindowType={name=view size={width=100 height=100}
            gridBoxType={name=survivors size={width=60 height=20}
                slotsize={width=30 height=20} max_slots_horizontal=2 add_horizontal=yes}
            gridBoxType={name=losses position={x=0 y=30} size={width=60 height=20}
                slotsize={width=60 height=20} max_slots_horizontal=1}}
    }"#,
    );
    let resources = fixture();
    let root = windows.gui_type(b"view").unwrap();
    let color = [0.1, 0.2, 0.3, 0.4];
    let values = Values::from([
        (
            "survivors".into(),
            Widget {
                entry_template: Some("ship".into()),
                entry_templates: BTreeMap::from([(1, "air".into())]),
                entries: vec![Values::new(), Values::new()],
                ..Default::default()
            },
        ),
        (
            "losses".into(),
            Widget {
                entry_template: Some("air_loss".into()),
                entries: vec![Values::from([
                    (
                        "equipment".into(),
                        Widget {
                            color: Some(color),
                            ..Default::default()
                        },
                    ),
                    (
                        "lost_air_grid".into(),
                        Widget {
                            entry_template: Some("air_count".into()),
                            entries: vec![Values::from([(
                                "number".into(),
                                Widget {
                                    text: Some("7".into()),
                                    ..Default::default()
                                },
                            )])],
                            ..Default::default()
                        },
                    ),
                ])],
                ..Default::default()
            },
        ),
    ]);
    let mut retained = Retained::new("view", root, [800, 600], &resources).unwrap();
    let mut refused = BTreeMap::new();
    let prepared = retained
        .prepare(
            root,
            &values,
            Vec::new(),
            &windows,
            &resources,
            Tick { time: 0.0, dt: 0.0 },
            1,
            true,
            true,
            true,
            None,
            &mut refused,
        )
        .unwrap();
    assert!(refused.is_empty(), "{refused:?}");
    let parts: Vec<_> = prepared.items.iter().flatten().collect();
    assert!(parts.iter().any(|part| part.path == "ship.fleet"));
    assert!(parts.iter().any(|part| part.path == "air.plane"));
    assert_eq!(
        parts
            .iter()
            .find(|part| part.path == "air_loss.equipment")
            .unwrap()
            .color,
        color
    );
    let number = parts
        .iter()
        .find(|part| part.path == "air_count.number")
        .unwrap();
    let native_gui::PartKind::Text { glyphs, .. } = &number.kind else {
        panic!("nested loss count");
    };
    assert_eq!(glyphs.len(), 1);
    assert_eq!(retained.entries["survivors/1"].name, "air");
}

#[test]
fn tactics_entry_provider_fills_child_hits_and_preserves_explicit_hovers() {
    content_reader_script::test_game::enter();
    use super::super::tooltips::set_hover;
    let windows = windows(
        b"guiTypes={containerWindowType={name=tactics_list_entry
        size={width=140 height=40}
        iconType={name=tactics_icon position={x=5 y=5} spriteType=GFX_even}
        iconType={name=child position={x=60 y=5} spriteType=GFX_even}
        instantTextBoxType={name=tactics_name position={x=30 y=5} font=Main_14
            maxWidth=100 maxHeight=14 alwaystransparent=yes}}}",
    );
    let root = windows.gui_type(b"tactics_list_entry").unwrap();
    let tactics_icon = root.path_named("tactics_icon").unwrap();
    let child_element = root.path_named("child").unwrap();
    let tactics_name = root.path_named("tactics_name").unwrap();
    let resources = fixture();
    let mut values = Values::new();
    set_hover(
        &mut values,
        "",
        Some("Tactic".into()),
        Some("Requirements".into()),
    );
    set_hover(&mut values, "child", Some("Own child".into()), None);
    values.entry("tactics_name".into()).or_default().text = Some("Name".into());
    let mut retained = Retained::new("tactics_list_entry", root, [800, 600], &resources).unwrap();
    let mut refused = BTreeMap::new();
    let prepared = retained
        .prepare(
            root,
            &values,
            Vec::new(),
            &windows,
            &resources,
            Tick { time: 0., dt: 0. },
            1,
            true,
            true,
            true,
            None,
            &mut refused,
        )
        .unwrap();
    assert!(refused.is_empty(), "{refused:?}");
    assert!(
        prepared
            .hovers
            .iter()
            .any(|hit| hit.region.path == tactics_icon.path
                && hit.text.as_deref() == Some("Tactic")
                && hit.description.as_deref() == Some("Requirements"))
    );
    assert!(
        prepared
            .hovers
            .iter()
            .any(|hit| hit.region.path == child_element.path
                && hit.text.as_deref() == Some("Own child")
                && hit.description.is_none())
    );
    assert!(
        !prepared
            .hovers
            .iter()
            .any(|hit| hit.region.path == tactics_name.path)
    );
    set_hover(&mut values, "child", None, None);
    let hovers = hover_regions(root, &prepared.layout.regions, &values);
    let child = hovers
        .iter()
        .find(|hit| hit.region.path == child_element.path)
        .unwrap();
    assert!(child.text.is_none() && child.description.is_none());
    let mut hits = Vec::new();
    append_hovers(&mut hits, hovers, [0; 2]);
    assert!(
        hits.iter()
            .rev()
            .find(|hit| super::super::contains(hit.rect, [65., 8.]))
            .unwrap()
            .text
            .is_none()
    );
}

#[test]
fn combat_box_hovers_pass_through_counts_and_keep_ship_icons_on_both_sides() {
    content_reader_script::test_game::enter();
    use super::super::{Widget, tooltips::set_hover};
    for (transparent, cover) in [(true, false), (false, false), (true, true)] {
        let mut source =
            String::from("guiTypes={containerWindowType={name=view size={width=640 height=120}");
        let mut values = Values::new();
        let mut groups = Vec::new();
        let mut entry_values = Values::new();
        set_hover(&mut entry_values, "icon", Some("Ship".into()), None);
        set_hover(&mut entry_values, "count", Some("Ship".into()), None);
        entry_values.entry("count".into()).or_default().text = Some("8".into());
        for side in 0..2 {
            for box_index in 0..4 {
                let key = format!("formation_{side}_{box_index}");
                let container = format!("box_{side}_{box_index}");
                let grid = format!("grid_{side}_{box_index}");
                let (x, y) = (box_index * 160, side * 60);
                let background = format!(
                    "containerWindowType={{name={container} position={{x={x} y={y}}} \
                     size={{width=140 height=40}} background={{name=tooltip \
                     spriteType=GFX_cornered pdx_tooltip={key}}}}}"
                );
                let holder = format!(
                    "gridBoxType={{name={grid} position={{x={x} y={y}}} \
                     size={{width=140 height=40}} slotsize={{width=140 height=40}} \
                     max_slots_horizontal=1 format=UPPER_LEFT}}"
                );
                for declaration in if cover {
                    [&holder, &background]
                } else {
                    [&background, &holder]
                } {
                    source.push_str(declaration);
                }
                values.insert(
                    format!("{container}/tooltip"),
                    Widget {
                        tooltip: Some(key.into()),
                        ..Default::default()
                    },
                );
                groups.push(Group {
                    path: grid,
                    rows: vec![Row {
                        naval_layout: None,
                        key: "0".into(),
                        template: "entry",
                        values: &entry_values,
                        explicit: None,
                    }],
                });
            }
        }
        source.push_str(&format!(
            "}} containerWindowType={{name=entry size={{width=140 height=40}} \
             iconType={{name=icon position={{x=5 y=5}} spriteType=GFX_even}} \
             instantTextBoxType={{name=count position={{x=30 y=5}} font=Main_14 \
             maxWidth=100 maxHeight=14 alwaystransparent={}}}}}}}",
            if transparent { "yes" } else { "no" }
        ));
        let windows = windows(source.as_bytes());
        let root = windows.gui_type(b"view").unwrap();
        let resources = fixture();
        let mut retained = Retained::new("view", root, [800, 600], &resources).unwrap();
        let mut refused = BTreeMap::new();
        let prepared = retained
            .prepare(
                root,
                &values,
                groups,
                &windows,
                &resources,
                Tick { time: 0., dt: 0. },
                1,
                true,
                true,
                true,
                None,
                &mut refused,
            )
            .unwrap();
        assert!(refused.is_empty(), "{refused:?}");
        let mut hits = Vec::new();
        append_hovers(&mut hits, prepared.hovers, [0; 2]);
        let hover = |point| {
            hits.iter()
                .rev()
                .find(|hit| super::super::contains(hit.rect, point))
                .and_then(|hit| hit.text.as_deref())
        };
        for side in 0..2 {
            for box_index in 0..4 {
                let key = format!("formation_{side}_{box_index}");
                let (x, y) = ((box_index * 160) as f32, (side * 60) as f32);
                assert_eq!(
                    hover([x + 10., y + 10.]),
                    Some(if cover { key.as_str() } else { "Ship" }),
                );
                assert_eq!(hover([x + 60., y + 30.]), Some(key.as_str()));
                assert_eq!(
                    hover([x + 60., y + 10.]),
                    Some(if transparent || cover {
                        key.as_str()
                    } else {
                        "Ship"
                    }),
                );
            }
        }
    }
}

#[test]
fn combat_mixed_grid_rows_are_counted_without_losing_compatible_entries() {
    content_reader_script::test_game::enter();
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
                    naval_layout: None,
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
                    None,
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
    content_reader_script::test_game::enter();
    let gui = content_reader_gui::gui::native::read(b"guiTypes={containerWindowType={name=view size={width=20 height=20} background={name=Background spriteType=bg} containerWindowType={name=inner size={width=5 height=5} iconType={name=icon spriteType=s}}}}");
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
    content_reader_script::test_game::enter();
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
                None,
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
fn combat_prepare_composes_nested_entries_clips_hovers_and_counts_missing_templates() {
    content_reader_script::test_game::enter();
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
            None,
            &mut refused,
        )
        .unwrap();
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert_eq!(
        refused["Combat GUI missing: the content stack declares no absent template"],
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
            None,
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
    assert!(Arc::ptr_eq(
        drawn.tooltips[0].text.as_ref().unwrap(),
        drawn.tooltips[1].text.as_ref().unwrap(),
    ));
}

#[test]
fn combat_controls_keep_native_scroll_paths_axes_and_popup_mask() {
    content_reader_script::test_game::enter();
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
    let resources = fixture_with(content_reader_gui::gui::native::read(document.as_bytes()));
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
    content_reader_script::test_game::enter();
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
    let resources = fixture_with(content_reader_gui::gui::native::read(source));
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
            None,
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
            None,
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
            None,
            &mut refused,
        )
        .unwrap();
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
            None,
            &mut refused,
        )
        .unwrap();
    assert_eq!(peek.layout.holders[0].origin, [0.0, -20.0]);
    assert!(refused.is_empty(), "{refused:?}");
}

#[test]
fn combat_prepare_resolves_bound_control_and_holder_names_without_case() {
    content_reader_script::test_game::enter();
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
            None,
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

#[test]
fn authored_tooltip_anchors_resolve_from_the_controls_unclipped_corner() {
    content_reader_script::test_game::enter();
    use super::super::{TooltipAnchor, tooltips::set_hover};
    let windows = windows(
        b"guiTypes={containerWindowType={name=anchored size={width=300 height=60}
        iconType={name=relative position={x=200 y=10} spriteType=GFX_even
            pdx_tooltip_anchor={x=-158 y=0}}
        iconType={name=absolute position={x=10 y=10} spriteType=GFX_even
            pdx_tooltip_anchor={x=40 y=50 relative=no below=no}}
        iconType={name=plain position={x=100 y=10} spriteType=GFX_even}}}",
    );
    let root = windows.gui_type(b"anchored").unwrap();
    let resources = fixture();
    let mut values = Values::new();
    for name in ["relative", "absolute", "plain"] {
        set_hover(&mut values, name, Some(name.into()), None);
    }
    let mut retained = Retained::new("anchored", root, [800, 600], &resources).unwrap();
    let mut refused = BTreeMap::new();
    let prepared = retained
        .prepare(root, &values, Vec::new(), &windows, &resources, Tick { time: 0., dt: 0. }, 1, true, true, true, None, &mut refused)
        .unwrap();
    let mut hits = Vec::new();
    append_hovers(&mut hits, prepared.hovers, [1000, 500]);
    let anchor = |text: &str| hits.iter().find(|hit| hit.text.as_deref() == Some(text)).unwrap().anchor;
    assert_eq!(anchor("relative"), TooltipAnchor::At { x: 1000 + 200 - 158, y: 510, below: true });
    assert_eq!(anchor("absolute"), TooltipAnchor::At { x: 40, y: 50, below: false });
    assert_eq!(anchor("plain"), TooltipAnchor::Pointer);
}

#[test]
fn a_flag_whose_art_arrives_is_drawn_at_the_same_hour() {
    content_reader_script::test_game::enter();
    let windows = windows(
        br#"guiTypes={containerWindowType={name=countrystateview size={width=100 height=50}
            iconType={name=flag spriteType=GFX_even}}}"#,
    );
    let resources = fixture();
    let mut bindings = Bindings::default();
    bindings.values.insert("flag".into(), crate::Widget { flag: Some("GER".into()), ..Default::default() });
    let mut gui = Windows::default();
    let draw = |gui: &mut Windows| {
        gui.draw(&windows, &resources, &bindings, 1, Selection::State(9904), [0.0; 2], false, &mut None, [1920.0, 1080.0], false, Tick { time: 0.0, dt: 0.0 }, true, true, None)
            .unwrap()
            .batch
            .quads
            .len()
    };
    ARRIVED.with(|arrived| arrived.set((None, 0)));
    assert_eq!(draw(&mut gui), 0, "its flag is still loading: nothing drawn");
    ARRIVED.with(|arrived| arrived.set((Some(native_gui::Texture { texture: 0, uv: [0.0, 0.0, 1.0, 1.0], size: [10, 10] }), 1)));
    assert!(draw(&mut gui) > 0, "the flag arrived: drawn at the same hour and generation");
}

fn entry_flag_parts(retained: &mut Retained, windows: &NativeWindows, resources: &Fixture) -> String {
    let root = windows.gui_type(b"view").unwrap();
    let mut leaf = Values::new();
    leaf.insert("flag".into(), crate::Widget { flag: Some("GER".into()), ..Default::default() });
    let mut entry = Values::new();
    entry.insert("inner".into(), crate::Widget { entry_template: Some("leaf".into()), entries: vec![leaf], ..Default::default() });
    let groups = vec![Group { path: "grid".into(), rows: vec![Row { naval_layout: None, key: "0".into(), template: "entry", values: &entry, explicit: None }] }];
    let mut refused = BTreeMap::new();
    let prepared = retained
        .prepare(root, &Values::new(), groups, windows, resources, Tick { time: 0., dt: 0. }, 1, true, false, true, None, &mut refused)
        .unwrap();
    format!("{:?}", prepared.items)
}

#[test]
fn art_arriving_rebinds_the_window_and_its_entries_at_the_same_generation() {
    content_reader_script::test_game::enter();
    let windows = windows(
        br#"guiTypes={containerWindowType={name=view size={width=200 height=100}
            gridBoxType={name=grid size={width=200 height=50} slotsize={width=200 height=50}
                max_slots_horizontal=1 format=UPPER_LEFT}}
            containerWindowType={name=entry size={width=200 height=50}
                gridBoxType={name=inner size={width=200 height=50} slotsize={width=200 height=50}
                    max_slots_horizontal=1 format=UPPER_LEFT}}
            containerWindowType={name=leaf size={width=200 height=50}
                iconType={name=flag spriteType=GFX_even}}}"#,
    );
    let root = windows.gui_type(b"view").unwrap();
    let resources = fixture();
    let mut retained = Retained::new("view", root, [800, 600], &resources).unwrap();
    let texture = |texture| Some(native_gui::Texture { texture, uv: [0.0, 0.0, 1.0, 1.0], size: [10, 10] });
    ARRIVED.with(|arrived| arrived.set((None, 0)));
    assert!(!entry_flag_parts(&mut retained, &windows, &resources).contains("Cell {"), "a cold flag draws nothing");
    ARRIVED.with(|arrived| arrived.set((texture(7), 1)));
    assert!(entry_flag_parts(&mut retained, &windows, &resources).contains("texture: 7"), "its art arrived: the entry's flag draws at the same generation");
    ARRIVED.with(|arrived| arrived.set((texture(9), 2)));
    let variant = entry_flag_parts(&mut retained, &windows, &resources);
    assert!(variant.contains("texture: 9") && !variant.contains("texture: 7"), "the preferred variant replaced the fallback: {variant}");
    let looks = LOOKS.with(std::cell::Cell::get);
    let _ = entry_flag_parts(&mut retained, &windows, &resources);
    assert_eq!(LOOKS.with(std::cell::Cell::get), looks, "no art moved: nothing bound again");
}

#[test]
fn a_window_whose_template_is_missing_is_refused_by_name() {
    content_reader_script::test_game::enter();
    let windows = windows(br#"guiTypes={containerWindowType={name=another size={width=10 height=10}}}"#);
    let resources = fixture();
    let drawn = Windows::default()
        .draw(&windows, &resources, &Bindings::default(), 1, Selection::State(9904), [0.0; 2], false, &mut None, [1920.0, 1080.0], false, Tick { time: 0.0, dt: 0.0 }, true, true, None)
        .unwrap();
    assert!(drawn.batch.quads.is_empty());
    assert!(drawn.missing().is_some_and(|why| why.contains("countrystateview")), "{:?}", drawn.missing());
}

#[test]
fn a_window_takes_the_pointer_over_every_child_it_draws_beyond_its_own_size() {
    content_reader_script::test_game::enter();
    let windows = windows(
        br#"guiTypes={containerWindowType={name=countrystateview size={width=100 height=50}
            containerWindowType={name=lower position={x=10 y=48} size={width=60 height=30}
                iconType={name=terrain_picture spriteType=GFX_even}}
            buttonType={name=close_button position={x=-6 y=4} quadTextureSprite=GFX_even orientation=UPPER_RIGHT}}}"#,
    );
    let resources = fixture();
    let drawn = Windows::default()
        .draw(
            &windows,
            &resources,
            &Bindings::default(),
            1,
            Selection::State(9904),
            [0.0; 2],
            false,
            &mut None,
            [1920.0, 1080.0],
            false,
            Tick { time: 0.0, dt: 0.0 },
            true,
            false,
            None,
        )
        .unwrap();
    assert_eq!(drawn.rect, [0.0, 0.0, 100.0, 50.0]);
    let close = [110.0, 8.0];
    assert!(drawn.closes(close));
    assert!(drawn.contains(close), "the close button past the root's right edge");
    assert!(drawn.contains([40.0, 70.0]), "the lower panel below the root");
    assert!(!drawn.contains([90.0, 70.0]), "beside the panel, under nothing the window draws");
    let footprint = drawn.footprint();
    for (point, inside) in [(close, true), ([40.0, 70.0], true), ([90.0, 70.0], false)] {
        assert_eq!(footprint.contains(point), inside, "{point:?}");
    }
}
