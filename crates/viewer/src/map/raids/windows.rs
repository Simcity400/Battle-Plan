//! Raid icon window bindings and updates. Preparing an hour never changes
//! a presented instance.

use super::*;

const BUTTON_CONTAINER: &str = "raid_map_icon_button_container";
const FRAME: &str = "raid_map_icon_frame";
const GLYPH: &str = "raid_map_icon_button";
const PLUS: &str = "plus_icon";

#[derive(Default)]
pub struct PreparedRaids {
    placed: Vec<Placed>,
    pub report: RaidReport,
}

struct Placed {
    screen: [f32; 2],
    icon: RaidIcon,
}

pub fn prepare(inputs: &RaidInputs<'_>, icons: &RaidIcons) -> PreparedRaids {
    let [near, far] = inputs.camera.defines.raid_icons;
    if !(near..=far).contains(&inputs.camera.height) {
        return PreparedRaids {
            report: icons.report.clone(),
            ..Default::default()
        };
    }
    let viewport = super::super::counters::gui_windows::viewport(inputs.viewport, inputs.ui_scale);
    let mut placed: Vec<_> = icons
        .icons
        .iter()
        .filter_map(|icon| {
            let world = (inputs.anchor)(icon.province)?;
            let projection = inputs
                .camera
                .project_map_icon(world, inputs.viewport, viewport);
            Some((
                projection.depth,
                Placed {
                    screen: projection.position,
                    icon: icon.clone(),
                },
            ))
        })
        .collect();
    placed.sort_by(|a, b| b.0.total_cmp(&a.0));
    PreparedRaids {
        placed: placed.into_iter().map(|(_, placed)| placed).collect(),
        report: icons.report.clone(),
    }
}

#[derive(Clone, Copy)]
struct Controls<'a> {
    frame: &'a native::Element,
    glyph: &'a native::Element,
    plus: &'a native::Element,
}

fn controls(root: &native::Element) -> Result<Controls<'_>, String> {
    // Find the named child container, then recursively find its controls and icons.
    let container = root
        .find_recursive(native::Kind::Container, BUTTON_CONTAINER.as_bytes())
        .ok_or_else(|| format!("no container named {BUTTON_CONTAINER}"))?;
    Ok(Controls {
        frame: container
            .find_recursive(native::Kind::Button, FRAME.as_bytes())
            .ok_or_else(|| format!("no button named {FRAME}"))?,
        glyph: container
            .find_recursive(native::Kind::Button, GLYPH.as_bytes())
            .ok_or_else(|| format!("no button named {GLYPH}"))?,
        plus: container
            .find_recursive(native::Kind::Icon, PLUS.as_bytes())
            .ok_or_else(|| format!("no icon named {PLUS}"))?,
    })
}

fn update(
    instance: &mut native_gui::lifecycle::Instance,
    root: &native::Element,
    icon: &RaidIcon,
    controls: Controls<'_>,
    resources: &dyn native_gui::Resources,
) -> Result<(), String> {
    // Set the frame and one-based sprite index in order.
    instance.set_sprite(
        root,
        controls.frame,
        FRAME_SPRITE,
        SpriteSetter::ButtonStandard,
    )?;
    instance.set_frame(root, controls.frame, icon.frame.wrapping_add(1) as i32)?;
    if icon.frame == 0 {
        // Offers enable or disable the frame only.
        instance.control(root, controls.frame)?.enabled = Some(icon.enabled);
    } else {
        // Own and incoming refreshes enable the glyph.
        instance.control(root, controls.glyph)?.enabled = Some(true);
    }
    let glyph = icon
        .glyph
        .as_deref()
        .ok_or_else(|| format!("{} has no decided raid glyph", controls.glyph.path))?;
    if !native_gui::sprite_resolves(glyph.as_bytes(), resources) {
        return Err(format!(
            "raid glyph {glyph} cannot replace {}",
            controls.glyph.path
        ));
    }
    instance.set_sprite(
        root,
        controls.glyph,
        glyph.as_bytes(),
        SpriteSetter::ButtonStandard,
    )?;
    if icon.plus {
        instance.set_hide_flag(root, controls.plus, false)?;
        instance.show(root, controls.plus)?;
    } else {
        instance.hide(root, controls.plus)?;
        instance.set_hide_flag(root, controls.plus, true)?;
    }
    Ok(())
}

impl RaidAssets {
    pub(crate) fn gui_assets(&self) -> native_gui::Assets {
        let mut assets = native_gui::Assets::default();
        assets
            .sprites
            .extend(self.glyphs.iter().map(|name| name.as_bytes().to_vec()));
        assets.sprites.insert(FRAME_SPRITE.to_vec());
        assets
    }
}

impl PreparedRaids {
    pub fn draw(
        &self,
        context: &super::super::native_resources::Context<
            '_,
            impl native_gui::Resources + native_gui::Sheets,
        >,
        priority: i8,
        view: &mut crate::map::icon_windows::PoolView,
        batch: &mut Batch,
        hits: &mut Vec<tooltips::Hit>,
        report: &mut RaidReport,
    ) -> usize {
        use super::super::{
            counters::gui_windows,
            icon_windows::{Owner, Plan},
        };
        hits.clear();
        let owners: Vec<_> = self
            .placed
            .iter()
            .map(|placed| Owner {
                id: placed.icon.province,
                screen: placed.screen,
                dirty: false,
            })
            .collect();
        let placed: HashMap<_, _> = self
            .placed
            .iter()
            .enumerate()
            .map(|(ordinal, placed)| (placed.icon.province, (ordinal, placed)))
            .collect();
        let bindings = context.windows.gui_type(WINDOW.as_bytes()).map(controls);
        let frame = bindings
            .as_ref()
            .and_then(|bound| bound.as_ref().ok())
            .map(|bound| bound.frame);
        let mut drawn = view.present(
            context,
            &Plan {
                name: WINDOW,
                kind: MapIconPool::Raid,
                alignment: 0x0A,
                owners: &owners,
            },
            Box::new(|instance, root| {
                // Initially hide plus_icon and set its hide bit.
                let plus = controls(root)?.plus;
                instance.hide(root, plus)?;
                instance.set_hide_flag(root, plus, true)
            }),
            |owner, _, instance, root| {
                let controls = *bindings
                    .as_ref()
                    .expect("the helper found the template")
                    .as_ref()
                    .map_err(Clone::clone)?;
                update(
                    instance,
                    root,
                    &placed[&owner].1.icon,
                    controls,
                    context.resources,
                )
            },
            &mut report.reasons,
        );
        // Sort and reverse-attach visible windows before GUI painting; hit-test
        // the paint list backwards. Pool/live order can differ after camera moves
        // or slot reuse, so hover follows draw rank.
        drawn.sort_by_key(|drawn| placed[&drawn.owner].0);
        let mut count = 0;
        for drawn in drawn {
            let (ordinal, placed) = placed[&drawn.owner];
            batch.at_layer(sprites::layer(priority, ordinal as u32, 0));
            if let Err(why) =
                native_gui::draw(&drawn.layout, drawn.origin, context.resources, batch)
            {
                gui_windows::count_refused(&mut report.reasons, WINDOW, &[why]);
                continue;
            }
            count += 1;
            if let (Some(key), Some(frame)) = (&placed.icon.tooltip, frame)
                && let Some(hit) = gui_windows::hit_rectangle(
                    frame,
                    &drawn.layout,
                    drawn.origin,
                    context.resources,
                    &mut report.reasons,
                )
            {
                hits.push(tooltips::Hit {
                    origin: [hit.rect[0], hit.rect[1]],
                    size: [hit.rect[2], hit.rect[3]],
                    key: key.clone(),
                    layer: sprites::layer(priority, ordinal as u32, 0),
                    supported: hit.supported,
                });
            }
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{DRAW_GUI, assets, draw_resources};
    use super::*;
    use crate::map::native_gui::lifecycle::Instance;

    fn hover_tooltip(offer: bool) {
        use super::super::tests::{frame, put, registered_command_power};
        use installed_content::localisation::Localisation;
        use replay::rows::{RaidTargetRow, RaidTooltipOperandRow};

        let mut assets = assets();
        assets
            .tooltip
            .state_names
            .insert(10, Ok("example_place".into()));
        if !offer {
            assets.catalog.types[0].tooltip.target_loc_key = "example_target".into();
        }
        assets.catalog.types[0].tooltip.days_to_prepare = 2;
        assets.catalog.types[0].command_power_raw = 200_000;
        let mut frame = frame();
        let scope = RaidScope {
            state: 10,
            ..Default::default()
        };
        put::<tables::Raids>(
            &mut frame,
            vec![RaidRow {
                id: 7,
                reference_type: 84,
                phase: 2,
                preparation_hours: 24,
                ..Default::default()
            }],
        );
        put::<tables::RaidTargets>(
            &mut frame,
            vec![RaidTargetRow {
                id: 7,
                reference_type: 84,
                raid_type: 100,
                scope: scope.clone(),
                ..Default::default()
            }],
        );
        let raid = 7u64 << 32 | 84;
        put::<tables::RaidTooltipOperands>(
            &mut frame,
            [
                (engine::raid_tooltip_operands::REFERENCES, 0),
                (engine::raid_tooltip_operands::REFERENCES, 1),
                (
                    engine::raid_tooltip_operands::NAME,
                    engine::raid_tooltip_operands::ARMY_RESOLVED,
                ),
                (
                    engine::raid_tooltip_operands::NAME,
                    engine::raid_tooltip_operands::WING_RESOLVED,
                ),
            ]
            .map(|(kind, key)| RaidTooltipOperandRow {
                raid,
                kind,
                key,
                ..Default::default()
            })
            .into(),
        );
        put::<tables::CountryCommandPower>(&mut frame, registered_command_power([0, 1]));
        let loc: Localisation = [
            ("example_place", "Example"),
            ("example_target", "\u{a3}unpreloaded deposit $LOCATION$"),
            ("raid_type_oilfield", "Synthetic mission"),
            ("raid_inst_item_desc", "$TYPE$ -> $TARGET$"),
            ("RAID_INST_PHASE_PREPARING", "In progress"),
            ("raid_inst_progress_desc", "$PHASE$ $PROGRESS|%0$"),
            ("tooltip_raid_view_details_prompt", "Inspect"),
            (
                "raid_insufficient_command_power",
                "Missing \u{a3}unpreloaded $COST|0$",
            ),
        ]
        .map(|(key, text)| (key.into(), text.into()))
        .into_iter()
        .collect();
        let key = if offer {
            tooltips::Key::Offer(
                vec![offers::TooltipTarget {
                    scope,
                    types: vec![0],
                }]
                .into(),
            )
        } else {
            tooltips::Key::Raid {
                id: 7,
                reference_type: 84,
                incoming: false,
            }
        };
        let windows = native_gui::tests::windows(DRAW_GUI);
        let mut resources = native_gui::tests::fixture_with(native::read(br#"guiTypes = {
            textBoxType = { name = ToolTip font = Main_14 maxWidth = 400 borderSize = { x = 2 y = 2 } }
        }"#));
        resources.add_sprite("GFX_on_map_raids_icons_frame_strip", [52, 52], 6);
        resources.add_sprite("glyph", [32, 32], 1);
        resources.add_sprite("plus", [32, 32], 1);
        // GUI loading uses this production inventory. A declared sprite omitted
        // from it refuses the native text bake.
        let inventory = crate::map::counters::gui_assets(
            &windows,
            &native::resources::Registry::default(),
            &loc,
            native_gui::Assets::from_localisation(&loc),
        );
        if inventory.sprites.contains(b"GFX_unpreloaded".as_slice()) {
            resources.add_sprite("GFX_unpreloaded", [12, 12], 1);
        }
        let context = crate::map::native_resources::Context {
            resources: &resources,
            windows: &windows,
            viewport: [2048, 1152],
            time: Some(0.),
        };
        let prepared = PreparedRaids {
            placed: vec![Placed {
                screen: [100., 200.],
                icon: RaidIcon {
                    province: 1,
                    frame: if offer { 0 } else { 2 },
                    enabled: !offer,
                    glyph: Some("glyph".into()),
                    plus: false,
                    tooltip: Some(key.clone()),
                },
            }],
            report: Default::default(),
        };
        let mut hits = Vec::new();
        let mut report = RaidReport::default();
        let mut batch = Batch::default();
        assert_eq!(
            prepared.draw(
                &context,
                assets.priority,
                &mut crate::map::icon_windows::PoolView::default(),
                &mut batch,
                &mut hits,
                &mut report
            ),
            1
        );
        assert!(report.reasons.is_empty(), "{:?}", report.reasons);
        assert_eq!(hits[0].origin, [125., 175.]);
        assert_eq!(hits[0].size, [52., 52.]);
        // Interactive pointer conversion: undo monitor stretch and GUI scale.
        let raster = crate::map::ui_scale::Raster {
            ui_scale: 1.25,
            stretch: 1.5,
        };
        let cursor = [150., 200.].map(|pixel| pixel * raster.screen_scale());
        let point = crate::map::ui_scale::to_interface(
            cursor.map(|pixel| pixel / raster.stretch),
            raster.ui_scale,
        );
        let hit = hits
            .iter()
            .rev()
            .find(|hit| hit.contains(point))
            .expect("raid owns hover");
        assert_eq!(hit.key, key);
        let tooltip = tooltips::build_from(tooltips::Inputs {
            assets: &assets,
            frame: &frame,
            ours: if offer { 1 } else { 0 },
            nato: false,
            key: &hit.key,
            loc: &loc,
            recorded_language: &loc.language,
            scripted_content: &Err("no scripted substitutions in this fixture".into()),
            flag_exists: &|_| true,
        });
        assert!(!tooltip.title.is_empty(), "{:?}", tooltip.refused);
        assert!(tooltip.title.contains("\u{13}unpreloaded"), "{tooltip:?}");
        let layout = native_gui::text_sprites::tooltip(&resources, &tooltip.title, 1., true)
            .expect("hovered raid produces a native tooltip drawable");
        let bake = layout
            .parts
            .iter()
            .find_map(|part| match &part.kind {
                native_gui::PartKind::TextSprite { bake, .. } => Some(bake),
                _ => None,
            })
            .expect("native tooltip text bake");
        assert!(bake.size.iter().all(|size| *size > 0));
        assert!(!bake.glyphs.is_empty());
        assert_eq!(bake.inline.len(), 1);
    }

    #[test]
    fn raid_tooltip_regression_active_hover_loads_substituted_target_icon() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        hover_tooltip(false);
    }

    #[test]
    fn raid_tooltip_regression_disabled_offer_hover_loads_diagnostic_icon() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        hover_tooltip(true);
    }

    #[test]
    fn native_raid_draw_and_hover_follow_the_map_anchor() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let resources = draw_resources();
        let windows = native_gui::tests::windows(DRAW_GUI);
        let context = crate::map::native_resources::Context {
            resources: &resources,
            windows: &windows,
            viewport: [2048, 1152],
            time: Some(0.),
        };
        let prepared = PreparedRaids {
            placed: vec![Placed {
                screen: [100., 200.],
                icon: RaidIcon {
                    province: 1,
                    frame: 0,
                    enabled: true,
                    glyph: Some("glyph".into()),
                    plus: false,
                    tooltip: Some(tooltips::Key::Raid {
                        id: 1,
                        reference_type: 4,
                        incoming: false,
                    }),
                },
            }],
            report: Default::default(),
        };
        let mut batch = Batch::default();
        let mut hits = Vec::new();
        let mut report = RaidReport::default();
        assert_eq!(
            prepared.draw(
                &context,
                assets().priority,
                &mut crate::map::icon_windows::PoolView::default(),
                &mut batch,
                &mut hits,
                &mut report
            ),
            1
        );
        assert_eq!(
            batch.quads[0].exact.as_ref().unwrap().corners,
            [[125., 175.], [177., 175.], [177., 227.], [125., 227.]]
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].origin, [125., 175.]);
        assert_eq!(hits[0].size, [52., 52.]);
        assert!(report.reasons.is_empty());
    }

    #[test]
    fn raid_tooltip_regression_overlap_tracks_paint_order_after_camera_move() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let resources = draw_resources();
        let windows = native_gui::tests::windows(DRAW_GUI);
        let context = crate::map::native_resources::Context {
            resources: &resources,
            windows: &windows,
            viewport: [2048, 1152],
            time: Some(0.),
        };
        let camera = Camera {
            center: [3000., 1500.],
            height: 300.,
            map: crate::map::camera::MapSize::TEST,
            defines: crate::map::camera::MapDefines::test(),
        };
        let key = |id| tooltips::Key::Raid {
            id,
            reference_type: 4,
            incoming: false,
        };
        let icons = RaidIcons {
            icons: [1, 2]
                .map(|province| RaidIcon {
                    province,
                    frame: 0,
                    enabled: false,
                    glyph: Some("glyph".into()),
                    plus: false,
                    tooltip: Some(key(province)),
                })
                .into(),
            report: Default::default(),
            built_for: None,
        };
        let mut view = crate::map::icon_windows::PoolView::default();
        for swapped in [false, true] {
            let anchor = |province| {
                let north = (province == 1) == swapped;
                Some([3000., 10., if north { 1501. } else { 1499. }])
            };
            let prepared = prepare(
                &RaidInputs {
                    anchor: &anchor,
                    camera,
                    viewport: [2048, 1152],
                    ui_scale: 1.,
                },
                &icons,
            );
            let expected = if swapped { 2 } else { 1 };
            // Distinct projected depths; the southward eye draws the near icon last.
            assert_eq!(prepared.placed.last().unwrap().icon.province, expected);
            let mut batch = Batch::default();
            let mut hits = Vec::new();
            let mut report = RaidReport::default();
            assert_eq!(
                prepared.draw(
                    &context,
                    assets().priority,
                    &mut view,
                    &mut batch,
                    &mut hits,
                    &mut report,
                ),
                2
            );
            assert!(report.reasons.is_empty(), "{:?}", report.reasons);
            let near = hits.iter().find(|hit| hit.key == key(expected)).unwrap();
            let centre = std::array::from_fn(|i| near.origin[i] + near.size[i] / 2.);
            assert!(hits.iter().all(|hit| hit.contains(centre)));
            assert_eq!(
                hits.iter()
                    .rev()
                    .find(|hit| hit.contains(centre))
                    .unwrap()
                    .key,
                key(expected),
            );
            let top = batch.quads.iter().max_by_key(|quad| quad.layer).unwrap();
            assert_eq!(
                sprites::layer_icon(top.layer),
                1,
                "hover and the nearer icon's glyph use the same window rank",
            );
        }
    }

    #[test]
    fn raid_tooltip_regression_overlapping_buildings_keep_the_hovered_targets_native_state() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        use super::super::tests::{frame, put, registered_command_power};
        use installed_content::localisation::Localisation;
        use replay::rows::Recorded;

        let resources = draw_resources();
        let windows = native_gui::tests::windows(DRAW_GUI);
        let context = crate::map::native_resources::Context {
            resources: &resources,
            windows: &windows,
            viewport: [2048, 1152],
            time: Some(0.),
        };
        let mut assets = assets();
        assets.tooltip.state_names = [
            (10, Ok("near_state".into())),
            (20, Ok("other_state".into())),
        ]
        .into();
        assets
            .buildings
            .types
            .get_mut("dam")
            .unwrap()
            .need_detection = false;
        assets.buildings.types.insert(
            "facility".into(),
            installed_content::buildings::BuildingType {
                name: "facility".into(),
                ..Default::default()
            },
        );
        assets.catalog.types[0].command_power_raw = 200_000;
        let mut frame = frame();
        let mut tokens = frame.complete::<tables::LexicalTokens>().unwrap().to_vec();
        tokens.push(replay::rows::LexicalTokenRow {
            token: 201,
            literal: b"facility".to_vec(),
        });
        put::<tables::LexicalTokens>(&mut frame, tokens);
        put::<tables::CountryCommandPower>(&mut frame, registered_command_power([0, 1]));
        put::<tables::CountryAppearance>(
            &mut frame,
            [10, 11]
                .map(|country| replay::rows::CountryAppearanceRow {
                    country,
                    original_tag_reference: Recorded::Value(Some(0)),
                    original_tag: Recorded::Value(Some(Vec::new())),
                    ..Default::default()
                })
                .into(),
        );
        put::<tables::Politics>(
            &mut frame,
            [10, 11]
                .map(|country| replay::rows::PoliticsRow {
                    country,
                    ruling_party: Some(1),
                    ..Default::default()
                })
                .into(),
        );
        put::<tables::PoliticalParties>(
            &mut frame,
            [10, 11]
                .map(|country| replay::rows::PoliticalPartyRow {
                    country,
                    id: 1,
                    is_ruling: 1,
                    token: b"fixture_group".to_vec(),
                    tag: country,
                    ..Default::default()
                })
                .into(),
        );
        let key = |id| {
            tooltips::Key::Offer(
                vec![offers::TooltipTarget {
                    scope: RaidScope {
                        building_type: if id == 2 { 201 } else { 200 },
                        building_location: id,
                        building_flags: 5,
                        ..Default::default()
                    },
                    types: vec![0],
                }]
                .into(),
            )
        };
        let placed = |id| Placed {
            screen: match id {
                1 => [100., 200.],
                2 => [85., 195.],
                _ => [112., 202.],
            },
            icon: RaidIcon {
                province: id,
                frame: 0,
                enabled: false,
                glyph: Some("glyph".into()),
                plus: false,
                tooltip: Some(key(id)),
            },
        };
        let mut view = crate::map::icon_windows::PoolView::default();
        let mut hits = Vec::new();
        let mut report = RaidReport::default();
        // A retained pool's live order differs from the new near-to-far ranks.
        // The first fixture populates that history; the second is paint order.
        for order in [[1, 3, 2], [2, 3, 1]] {
            let prepared = PreparedRaids {
                placed: order.map(placed).into(),
                report: Default::default(),
            };
            let mut batch = Batch::default();
            assert_eq!(
                prepared.draw(
                    &context,
                    assets.priority,
                    &mut view,
                    &mut batch,
                    &mut hits,
                    &mut report,
                ),
                3
            );
        }
        assert!(report.reasons.is_empty(), "{:?}", report.reasons);
        let loc: Localisation = [
            ("near_state", "Native building state"),
            ("other_state", "Neighbouring building state"),
            ("VICTORY_POINTS_1", "Different province name"),
            ("dam", "Dam"),
            ("facility", "Facility"),
            ("RAID_TARGET_NAME", "$WHAT$ in $LOCATION$"),
            ("raid_type_oilfield", "Building mission"),
            (
                "TOOLTIP_POTENTIAL_TARGET_ENTRY_NOT_ALLOWED",
                "Target $TARGET_NAME$",
            ),
        ]
        .map(|(key, text)| (key.into(), text.into()))
        .into_iter()
        .collect();
        for point in [[164., 201.], [151., 201.]] {
            let hit = hits.iter().rev().find(|hit| hit.contains(point)).unwrap();
            let tooltip = tooltips::build_from(tooltips::Inputs {
                assets: &assets,
                frame: &frame,
                ours: 1,
                nato: false,
                key: &hit.key,
                loc: &loc,
                recorded_language: &loc.language,
                scripted_content: &Err("no scripted substitutions in this fixture".into()),
                flag_exists: &|_| true,
            });
            // Use the building's state. A building-only target does not query its
            // province's victory-point name.
            assert!(
                tooltip
                    .title
                    .starts_with("Target Dam in Native building state"),
                "{point:?}: {tooltip:?}",
            );
        }
        // A null ruling-party pointer cannot skip the earlier native
        // group-qualified flag rung, even with plain artwork available.
        let mut politics = frame.complete::<tables::Politics>().unwrap().to_vec();
        politics[0].ruling_party = None;
        put::<tables::Politics>(&mut frame, politics);
        let mut parties = frame
            .complete::<tables::PoliticalParties>()
            .unwrap()
            .to_vec();
        parties[0].is_ruling = 0;
        put::<tables::PoliticalParties>(&mut frame, parties);
        let tooltip = tooltips::build_from(tooltips::Inputs {
            assets: &assets,
            frame: &frame,
            ours: 1,
            nato: false,
            key: &key(1),
            loc: &loc,
            recorded_language: &loc.language,
            scripted_content: &Err("no scripted substitutions in this fixture".into()),
            flag_exists: &|_| true,
        });
        assert_eq!(
            tooltip.refused.get(
                "TOOLTIP_POTENTIAL_TARGET_ENTRY: country 10 flag has a null ruling-party pointer"
            ),
            Some(&1),
        );
    }

    #[test]
    fn missing_native_raid_controls_are_named_and_counted_for_each_owner() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let resources = draw_resources();
        let windows = native_gui::tests::windows(
            br#"guiTypes = { containerWindowType = {
            name = raid_map_icon containerWindowType = { name = raid_map_icon_button_container }
        } }"#,
        );
        let context = crate::map::native_resources::Context {
            resources: &resources,
            windows: &windows,
            viewport: [2048, 1152],
            time: Some(0.),
        };
        let prepared = PreparedRaids {
            placed: [1, 2]
                .map(|province| Placed {
                    screen: [100., 200.],
                    icon: RaidIcon {
                        province,
                        frame: 0,
                        enabled: true,
                        glyph: None,
                        plus: false,
                        tooltip: None,
                    },
                })
                .into(),
            report: Default::default(),
        };
        let mut batch = Batch::default();
        let mut report = RaidReport::default();
        assert_eq!(
            prepared.draw(
                &context,
                assets().priority,
                &mut crate::map::icon_windows::PoolView::default(),
                &mut batch,
                &mut Vec::new(),
                &mut report
            ),
            0
        );
        assert_eq!(report.reasons.values().sum::<usize>(), 2);
        assert!(
            report
                .reasons
                .keys()
                .any(|why| why.contains("raid_map_icon_frame"))
        );
        assert!(batch.quads.is_empty());
    }

    #[test]
    fn a_reused_raid_window_withholds_a_missing_or_undecided_glyph() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let resources = draw_resources();
        let windows = native_gui::tests::windows(DRAW_GUI);
        let context = crate::map::native_resources::Context {
            resources: &resources,
            windows: &windows,
            viewport: [2048, 1152],
            time: Some(1.),
        };
        let mut prepared = PreparedRaids {
            placed: vec![Placed {
                screen: [100., 200.],
                icon: RaidIcon {
                    province: 1,
                    frame: 0,
                    enabled: true,
                    glyph: Some("glyph".into()),
                    plus: false,
                    tooltip: None,
                },
            }],
            report: Default::default(),
        };
        let mut view = crate::map::icon_windows::PoolView::default();
        let mut batch = Batch::default();
        let mut report = RaidReport::default();
        assert_eq!(
            prepared.draw(
                &context,
                assets().priority,
                &mut view,
                &mut batch,
                &mut Vec::new(),
                &mut report
            ),
            1
        );
        for glyph in [Some("missing_glyph".into()), None] {
            prepared.placed[0].icon.glyph = glyph;
            batch = Batch::default();
            report = RaidReport::default();
            assert_eq!(
                prepared.draw(
                    &context,
                    assets().priority,
                    &mut view,
                    &mut batch,
                    &mut Vec::new(),
                    &mut report
                ),
                0
            );
            assert!(batch.is_empty());
            assert_eq!(report.reasons.values().sum::<usize>(), 1);
            assert!(report.reasons.keys().all(|why| why.contains("raid glyph")));
        }
    }

    #[test]
    fn native_raid_bindings_find_nested_inherited_controls() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let resources = draw_resources();
        let windows = native_gui::tests::windows(br#"guiTypes = { containerWindowType = {
          name = raid_map_icon size = { width = 100 height = 80 }
          containerWindowType = { name = wrapper containerWindowType = {
            name = raid_map_icon_button_container size = { width = 50 height = 60 }
            buttonType = { name = source spriteType = GFX_on_map_raids_icons_frame_strip
              orientation = lower_right position = { x = -5 y = -6 } }
            buttonType = { name = raid_map_icon_frame parent = source }
            buttonType = { name = raid_map_icon_button spriteType = glyph }
            containerWindowType = { name = nested iconType = { name = plus_icon spriteType = plus } }
          } }
        } }"#);
        let root = windows.gui_type(WINDOW.as_bytes()).unwrap();
        let mut instance = Instance::construct(root, [200, 100], &resources).unwrap();
        instance.gui_show(root, root).unwrap();
        let icon = RaidIcon {
            province: 1,
            frame: 0,
            enabled: false,
            glyph: Some("glyph".into()),
            plus: false,
            tooltip: None,
        };
        let controls = controls(root).unwrap();
        update(&mut instance, root, &icon, controls, &resources).unwrap();
        assert_eq!(
            instance.control(root, controls.frame).unwrap().enabled,
            Some(false)
        );
        let layout = instance.layout(root, &resources, None).unwrap().unwrap();
        let frame = layout
            .parts
            .iter()
            .find(|part| part.path == controls.frame.path)
            .unwrap();
        let native_gui::PartKind::Sprite { geometry, .. } = &frame.kind else {
            panic!("frame is a sprite");
        };
        // Inherit only the sprite; the derived control retains its own default
        // position and orientation.
        assert_eq!(&geometry.rect[..2], &[0., 0.]);
        assert!(
            layout
                .parts
                .iter()
                .all(|part| part.path != controls.plus.path)
        );
        update(
            &mut instance,
            root,
            &RaidIcon { plus: true, ..icon },
            controls,
            &resources,
        )
        .unwrap();
        let layout = instance.layout(root, &resources, None).unwrap().unwrap();
        assert!(
            layout
                .parts
                .iter()
                .any(|part| part.path == controls.plus.path)
        );
    }
}
