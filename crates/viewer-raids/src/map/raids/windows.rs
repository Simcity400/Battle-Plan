
#[cfg(test)]
use viewer_gpu::camera::MapDefines;
#[cfg(test)]
use viewer_gpu::camera::MapSize;
use viewer_native_gui::native_resources::Context;
#[cfg(test)]
use viewer_native_gui::test_support::gui;

use super::*;
use viewer_gpu::icon_tracks::{IconAnchor, IconFamily};
use viewer_native_gui::gui_resources::count_refused;
use viewer_native_gui::gui_resources::hit_rectangle;
use viewer_native_gui::gui_resources::viewport as gui_resources_viewport;
use viewer_native_gui::icon_windows::PoolView;

const BUTTON_CONTAINER: &str = "raid_map_icon_button_container";
const FRAME: &str = "raid_map_icon_frame";
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum FrameState {
    Own,
    Hovered,
    Selected,
}

impl FrameState {
    fn frame(self, own: u32) -> u32 {
        match self {
            Self::Own => own,
            Self::Hovered => 2,
            Self::Selected => 1,
        }
    }
}
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
    anchor: IconAnchor,
}

pub fn prepare(inputs: &RaidInputs<'_>, icons: &RaidIcons) -> PreparedRaids {
    let defines = &inputs.camera.defines;
    let [near, far] = defines.raid_icons;
    let far = match inputs.reach {
        Some(RaidReach::Normal) => Some(far),
        Some(RaidReach::RaidMode) => Some(defines.raid_icons_raid_mode),
        None => None,
    };
    let free = inputs.camera.free().is_some();
    let shown = far.is_some_and(|far| free || (near..=far).contains(&inputs.camera.height));
    let far = far.unwrap_or(near);
    if !shown {
        return PreparedRaids {
            report: icons.report.clone(),
            ..Default::default()
        };
    }
    let viewport = gui_resources_viewport(inputs.viewport, inputs.ui_scale);
    let mut placed: Vec<_> = icons
        .icons
        .iter()
        .filter_map(|icon| {
            let world = (inputs.anchor)(icon.province)?;
            if free && inputs.camera.icon_distance(world) < near {
                return None;
            }
            let projection = inputs
                .camera
                .project_map_icon(world, inputs.viewport, viewport)?;
            Some((
                projection.depth,
                Placed {
                    screen: projection.position,
                    icon: icon.clone(),
                    anchor: IconAnchor {
                        world,
                        cutoff: far,
                        family: IconFamily::Raids,
                        moves: None,
                    },
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
pub(crate) struct Controls<'a> {
    frame: &'a native::Element,
    glyph: &'a native::Element,
    plus: &'a native::Element,
}

pub(crate) fn controls(root: &native::Element) -> Result<Controls<'_>, String> {
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

pub(crate) fn update(
    instance: &mut native_gui::lifecycle::Instance,
    root: &native::Element,
    icon: &RaidIcon,
    state: FrameState,
    controls: Controls<'_>,
    resources: &dyn native_gui::Resources,
) -> Result<(), String> {
    instance.set_sprite(
        root,
        controls.frame,
        FRAME_SPRITE,
        SpriteSetter::ButtonStandard,
    )?;
    let frame = state.frame(icon.frame);
    instance.set_frame(root, controls.frame, frame.wrapping_add(1) as i32)?;
    if icon.frame == 0 {
        instance.control(root, controls.frame)?.enabled = Some(icon.enabled);
    } else {
        instance.control(root, controls.glyph)?.enabled = Some(true);
    }
    let glyph = icon
        .glyph
        .as_deref()
        .ok_or_else(|| format!("{} has no decided raid glyph", controls.glyph.path))?;
    if native_gui::sprite_resolves(glyph.as_bytes(), resources) {
        instance.set_sprite(
            root,
            controls.glyph,
            glyph.as_bytes(),
            SpriteSetter::ButtonStandard,
        )?;
    }
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
    pub fn gui_assets(&self) -> native_gui::Assets {
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
        context: &Context<'_, impl native_gui::Resources + native_gui::Sheets>,
        priority: i8,
        view: &mut PoolView,
        batch: &mut Batch,
        hits: &mut Vec<tooltips::Hit>,
        report: &mut RaidReport,
        details: Option<&(u32, std::sync::Arc<tooltips::Details>)>,
        hovered: Option<u32>,
    ) -> usize {
        use viewer_native_gui::icon_windows::{Owner, Plan};
        hits.clear();
        let owners: Vec<_> = self
            .placed
            .iter()
            .map(|placed| Owner {
                id: placed.icon.group,
                screen: placed.screen,
                dirty: false,
            })
            .collect();
        let placed: HashMap<_, _> = self
            .placed
            .iter()
            .enumerate()
            .map(|(ordinal, placed)| (placed.icon.group, (ordinal, placed)))
            .collect();
        let bindings = context.windows.gui_type(WINDOW.as_bytes()).map(controls);
        let state = |icon: &RaidIcon| {
            if !matches!(icon.tooltip, Some(tooltips::Key::Offer(_))) {
                FrameState::Own
            } else if details.is_some_and(|(province, _)| *province == icon.province) {
                FrameState::Selected
            } else if hovered == Some(icon.province) {
                FrameState::Hovered
            } else {
                FrameState::Own
            }
        };
        let frame = bindings
            .as_ref()
            .and_then(|bound| bound.as_ref().ok())
            .map(|bound| bound.frame);
        view.present_recorded(
            context,
            &Plan {
                name: WINDOW,
                kind: MapIconPool::Raid,
                alignment: 0x0A,
                owners: &owners,
            },
            Box::new(|instance, root| {
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
                let icon = &placed[&owner].1.icon;
                update(instance, root, icon, state(icon), controls, context.resources)
            },
            |owner| {
                let icon = &placed[owner].1.icon;
                (icon.frame, state(icon), icon.enabled, icon.glyph.clone(), icon.plus)
            },
            false,
            &mut report.reasons,
        );
        if view.quiet() {
            return 0;
        }
        let (mut drawn, painted) = view.painted();
        drawn.sort_by_key(|(drawn, _)| placed[&drawn.owner].0);
        let mut count = 0;
        for (drawn, generation) in drawn {
            let (ordinal, placed) = placed[&drawn.owner];
            let key = sprites::layer(priority, ordinal as u32, 0);
            batch.at_layer(key);
            batch.at_anchor(Some(placed.anchor));
            let draw = |batch: &mut Batch| native_gui::draw(&drawn.layout, drawn.origin, context.resources, batch);
            if let Err(why) =
                painted.paint(
                    sprites::WindowKey::new(sprites::WindowPool::Raid, 0, drawn.owner),
                    drawn.owner, generation, drawn.origin, key, Some(placed.anchor), batch, draw)
            {
                count_refused(&mut report.reasons, WINDOW, &[why]);
                continue;
            }
            count += 1;
            if let (Some(key), Some(frame)) = (&placed.icon.tooltip, frame)
                && let Some(hit) = hit_rectangle(
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
                    province: placed.icon.province,
                    layer: sprites::layer(priority, ordinal as u32, 0),
                    supported: hit.supported,
                });
            }
            if let Some((province, shown)) = details
                && placed.icon.province == *province
                && let Some(key @ tooltips::Key::Offer(_)) = &placed.icon.tooltip
            {
                let layer = sprites::layer(priority, ordinal as u32, 1);
                batch.at_layer(layer);
                if let Some((panels, buttons)) = super::details::draw(context, shown, drawn.origin, batch, &mut report.reasons) {
                    for rect in panels {
                        hits.push(tooltips::Hit {
                            origin: [rect[0], rect[1]],
                            size: [rect[2], rect[3]],
                            key: key.clone(),
                            province: *province,
                            layer,
                            supported: false,
                        });
                    }
                    for button in buttons {
                        let Some(target) = button.target(shown) else { continue };
                        hits.push(tooltips::Hit {
                            origin: [button.rect[0], button.rect[1]],
                            size: [button.rect[2], button.rect[3]],
                            key: tooltips::Key::Type(std::sync::Arc::new(target)),
                            province: *province,
                            layer,
                            supported: button.supported,
                        });
                    }
                }
            }
        }
        batch.at_anchor(None);
        painted.finish();
        count
    }
}

#[cfg(any(test, feature = "test-support"))]
impl PreparedRaids {
    pub fn test_icon(screen: [f32; 2], icon: RaidIcon) -> Self {
        Self {
            placed: vec![Placed {
                screen,
                icon,
                anchor: IconAnchor {
                    world: [0.0; 3],
                    cutoff: f32::INFINITY,
                    family: IconFamily::Raids,
                    moves: None,
                },
            }],
            report: RaidReport::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{DRAW_GUI, assets, draw_resources};
    use super::*;

    const UNANCHORED: IconAnchor = IconAnchor {
        world: [0.0; 3],
        cutoff: f32::INFINITY,
        family: IconFamily::Raids,
        moves: None,
    };
    use viewer_native_gui::native_gui::lifecycle::Instance;

    #[test]
    fn native_raid_draw_and_hover_follow_the_map_anchor() {
        content_reader_script::test_game::enter();
        let resources = draw_resources();
        let windows = gui::windows(DRAW_GUI);
        let context = Context {
            resources: &resources,
            windows: &windows,
            viewport: [2048, 1152],
            time: Some(0.),
        };
        let prepared = PreparedRaids {
            placed: vec![Placed {
                anchor: UNANCHORED,
                screen: [100., 200.],
                icon: RaidIcon {
                    group: 1,
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
                &mut PoolView::default(),
                &mut batch,
                &mut hits,
                &mut report, None, None
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
        content_reader_script::test_game::enter();
        let resources = draw_resources();
        let windows = gui::windows(DRAW_GUI);
        let context = Context {
            resources: &resources,
            windows: &windows,
            viewport: [2048, 1152],
            time: Some(0.),
        };
        let camera = Camera {
            center: [3000., 1500.],
            height: 300.,
            map: MapSize::TEST,
            defines: MapDefines::test(),
            pose: viewer_gpu::camera::Pose::Map,
        };
        let key = |id| tooltips::Key::Raid {
            id,
            reference_type: 4,
            incoming: false,
        };
        let icons = RaidIcons {
            icons: [1, 2]
                .map(|province| RaidIcon {
                    group: province,
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
        let mut view = PoolView::default();
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
                    reach: Some(RaidReach::Normal),
                },
                &icons,
            );
            let expected = if swapped { 2 } else { 1 };
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
                    &mut report, None, None,
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
        content_reader_script::test_game::enter();
        use super::super::test_support::{frame, put, registered_command_power};
        use content_reader_localisation::localisation::Localisation;
        use replay_data::rows::Recorded;

        let resources = draw_resources();
        let windows = gui::windows(DRAW_GUI);
        let context = Context {
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
            .trigger_content
            .buildings
            .types
            .get_mut("dam")
            .unwrap()
            .need_detection = false;
        assets.trigger_content.buildings.types.insert(
            "facility".into(),
            content_reader_key_names::buildings::BuildingType {
                name: "facility".into(),
                ..Default::default()
            },
        );
        assets.catalog.types[0].command_power_raw = 200_000;
        let mut frame = frame();
        let mut tokens = frame.complete::<tables::LexicalTokens>().unwrap().to_vec();
        tokens.push(replay_data::rows::LexicalTokenRow {
            token: 201,
            literal: b"facility".to_vec(),
        });
        put::<tables::LexicalTokens>(&mut frame, tokens);
        put::<tables::CountryCommandPower>(&mut frame, registered_command_power([0, 1]));
        put::<tables::CountryAppearance>(
            &mut frame,
            [10, 11]
                .map(|country| replay_data::rows::CountryAppearanceRow {
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
                .map(|country| replay_data::rows::PoliticsRow {
                    country,
                    ruling_party: Some(1),
                    ..Default::default()
                })
                .into(),
        );
        put::<tables::PoliticalParties>(
            &mut frame,
            [10, 11]
                .map(|country| replay_data::rows::PoliticalPartyRow {
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
            anchor: UNANCHORED,
            screen: match id {
                1 => [100., 200.],
                2 => [85., 195.],
                _ => [112., 202.],
            },
            icon: RaidIcon {
                group: id,
                province: id,
                frame: 0,
                enabled: false,
                glyph: Some("glyph".into()),
                plus: false,
                tooltip: Some(key(id)),
            },
        };
        let mut view = PoolView::default();
        let mut hits = Vec::new();
        let mut report = RaidReport::default();
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
                    &mut report, None, None,
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
            assert!(
                tooltip
                    .title
                    .starts_with("Target Dam in Native building state"),
                "{point:?}: {tooltip:?}",
            );
        }
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
        content_reader_script::test_game::enter();
        let resources = draw_resources();
        let windows = gui::windows(
            br#"guiTypes = { containerWindowType = {
            name = raid_map_icon containerWindowType = { name = raid_map_icon_button_container }
        } }"#,
        );
        let context = Context {
            resources: &resources,
            windows: &windows,
            viewport: [2048, 1152],
            time: Some(0.),
        };
        let prepared = PreparedRaids {
            placed: [1, 2]
                .map(|province| Placed {
                    anchor: UNANCHORED,
                    screen: [100., 200.],
                    icon: RaidIcon {
                        group: province,
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
                &mut PoolView::default(),
                &mut batch,
                &mut Vec::new(),
                &mut report, None, None
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
    fn groups_standing_at_one_province_keep_their_own_windows() {
        content_reader_script::test_game::enter();
        let resources = draw_resources();
        let windows = gui::windows(DRAW_GUI);
        let context = Context {
            resources: &resources,
            windows: &windows,
            viewport: [2048, 1152],
            time: Some(1.),
        };
        let prepared = PreparedRaids {
            placed: [(4, 2), (5, 4)]
                .map(|(group, frame)| Placed {
                    anchor: UNANCHORED,
                    screen: [100., 200.],
                    icon: RaidIcon {
                        group,
                        province: 5,
                        frame,
                        enabled: true,
                        glyph: Some("glyph".into()),
                        plus: false,
                        tooltip: None,
                    },
                })
                .into(),
            report: Default::default(),
        };
        let mut report = RaidReport::default();
        assert_eq!(
            prepared.draw(
                &context,
                assets().priority,
                &mut PoolView::default(),
                &mut Batch::default(),
                &mut Vec::new(),
                &mut report, None, None
            ),
            2,
            "{:?}",
            report.reasons
        );
    }

    #[test]
    fn a_reused_raid_window_keeps_an_undeclared_glyph_and_withholds_an_undecided_one() {
        content_reader_script::test_game::enter();
        let resources = draw_resources();
        let windows = gui::windows(DRAW_GUI);
        let context = Context {
            resources: &resources,
            windows: &windows,
            viewport: [2048, 1152],
            time: Some(1.),
        };
        let mut prepared = PreparedRaids {
            placed: vec![Placed {
                anchor: UNANCHORED,
                screen: [100., 200.],
                icon: RaidIcon {
                    group: 1,
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
        let mut view = PoolView::default();
        let mut batch = Batch::default();
        let mut report = RaidReport::default();
        assert_eq!(
            prepared.draw(
                &context,
                assets().priority,
                &mut view,
                &mut batch,
                &mut Vec::new(),
                &mut report, None, None
            ),
            1
        );
        for glyph in [Some("missing_glyph".into()), None] {
            let decided = glyph.is_some();
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
                    &mut report, None, None
                ),
                usize::from(decided)
            );
            assert_eq!(batch.is_empty(), !decided);
            assert_eq!(report.reasons.values().sum::<usize>(), usize::from(!decided));
            assert!(report.reasons.keys().all(|why| why.contains("raid glyph")));
        }
    }

    #[test]
    fn native_raid_bindings_find_nested_inherited_controls() {
        content_reader_script::test_game::enter();
        let resources = draw_resources();
        let windows = gui::windows(br#"guiTypes = { containerWindowType = {
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
            group: 1,
            province: 1,
            frame: 0,
            enabled: false,
            glyph: Some("glyph".into()),
            plus: false,
            tooltip: None,
        };
        let controls = controls(root).unwrap();
        let frame = |instance: &Instance| {
            let layout = instance.layout(root, &resources, None).unwrap().unwrap();
            let part = layout
                .parts
                .iter()
                .find(|part| part.path == controls.frame.path)
                .unwrap();
            let native_gui::PartKind::Sprite { uv, offsets, .. } = &part.kind else {
                panic!("frame is a sprite");
            };
            (offsets[0] / uv[2]).round() as i32
        };
        update(&mut instance, root, &icon, FrameState::Own, controls, &resources).unwrap();
        assert_eq!(
            instance.control(root, controls.frame).unwrap().enabled,
            Some(false)
        );
        assert_eq!(frame(&instance), 0);
        update(&mut instance, root, &icon, FrameState::Hovered, controls, &resources).unwrap();
        assert_eq!(frame(&instance), 2);
        update(&mut instance, root, &icon, FrameState::Own, controls, &resources).unwrap();
        let layout = instance.layout(root, &resources, None).unwrap().unwrap();
        let frame_part = layout
            .parts
            .iter()
            .find(|part| part.path == controls.frame.path)
            .unwrap();
        let native_gui::PartKind::Sprite { geometry, .. } = &frame_part.kind else {
            panic!("frame is a sprite");
        };
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
            FrameState::Selected,
            controls,
            &resources,
        )
        .unwrap();
        assert_eq!(frame(&instance), 1);
        assert_eq!(instance.control(root, controls.frame).unwrap().enabled, Some(false));
        let layout = instance.layout(root, &resources, None).unwrap().unwrap();
        assert!(
            layout
                .parts
                .iter()
                .any(|part| part.path == controls.plus.path)
        );
    }
}
