//! Raid icon windows, bound by Reload@141E71BF0 and updated by SetFrameState@141E71F70
//! and ApplyOffers@141E75100. Preparing an hour never changes a presented instance.

use super::*;

// Rebuild@1418D83B0, literal@142A014B0.
const BUTTON_CONTAINER: &str = "raid_map_icon_button_container";
// Reload@141E71BF0, literal@142A014D0.
const FRAME: &str = "raid_map_icon_frame";
// Reload@141E71BF0, literal@142A729B8.
const GLYPH: &str = "raid_map_icon_button";
// Reload@141E71BF0, literal@142A729A8.
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
    // Rebuild@1418D83B0 uses FindChildContainerByName@1422A54E0; Reload@141E71BF0
    // uses recursive control@1422A4D60 and icon@1422A7170 lookups inside it.
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
    // vslot+0x2D8 in SetFrameState@141E71F70, then vslot+0xB0, one-based.
    instance.set_sprite(
        root,
        controls.frame,
        FRAME_SPRITE,
        SpriteSetter::ButtonStandard,
    )?;
    instance.set_frame(root, controls.frame, icon.frame.wrapping_add(1) as i32)?;
    if icon.frame == 0 {
        // ApplyOffers@141E75100 enables/disables the frame only.
        instance.control(root, controls.frame)?.enabled = Some(icon.enabled);
    } else {
        // Owned refresh@141E72980 and enemy refresh@141E770E0 enable the glyph.
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
        let drawn = view.present(
            context,
            &Plan {
                name: WINDOW,
                kind: MapIconPool::Raid,
                alignment: 0x0A,
                owners: &owners,
            },
            Box::new(|instance, root| {
                // Reload@141E71BF0 initially hides plus_icon and sets its hide bit.
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
            if let (Some(key), Some(frame)) = (&placed.icon.tooltip, frame) {
                for part in drawn
                    .layout
                    .parts
                    .iter()
                    .filter(|part| part.path == frame.path)
                {
                    if let native_gui::PartKind::Sprite { geometry, .. } = &part.kind {
                        hits.push(tooltips::Hit {
                            origin: [
                                geometry.rect[0] + drawn.origin[0] as f32,
                                geometry.rect[1] + drawn.origin[1] as f32,
                            ],
                            size: [geometry.rect[2], geometry.rect[3]],
                            key: key.clone(),
                        });
                    } else {
                        let why = native_gui::Refusal {
                            path: part.path.clone(),
                            kind: native_gui::RefusalKind::MissingInput,
                            detail: "raid-frame hover geometry for this drawable is not reproduced"
                                .into(),
                        };
                        gui_windows::count_refused(&mut report.reasons, WINDOW, &[why]);
                    }
                }
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
        hover_tooltip(false);
    }

    #[test]
    fn raid_tooltip_regression_disabled_offer_hover_loads_diagnostic_icon() {
        hover_tooltip(true);
    }

    #[test]
    fn native_raid_draw_and_hover_follow_the_map_anchor() {
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
    fn missing_native_raid_controls_are_named_and_counted_for_each_owner() {
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
        // CButtonType::vf10@14235A0D0 inherits the sprite only; the derived
        // control keeps its own default position and orientation.
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
