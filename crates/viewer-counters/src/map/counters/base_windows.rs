//! Air-base, rocket-site, gun-emplacement, headquarters and facility windows, on the
//! native map-icon pools.

use super::*;
use std::collections::HashMap;
use viewer_foundation::map::gui_resources::count_refused;
use viewer_foundation::map::gui_resources::hit_rectangle;
use viewer_foundation::map::gui_resources::viewport as gui_resources_viewport;
use viewer_foundation::map::icon_windows::PoolView;
use viewer_foundation::map::{
    icon_windows::{Owner, Plan},
    native_gui::{
        self, SpriteSetter,
        lifecycle::{Instance, MapIconPool},
    },
    native_resources::Context,
};

#[derive(Default)]
pub(super) struct PreparedBases {
    placed: [Vec<Placed>; FAMILIES],
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Family {
    AirBase,
    Headquarter,
    Facility,
    RocketSite,
    GunEmplacement,
    Radar,
}

/// How many [`Family`] kinds there are, one pool view each.
pub(super) const FAMILIES: usize = 6;

/// The radar icon's priority: the game's per-mode priority tables leave the
/// radar icon's entry zero in every mode, and no define sets it.
pub(super) const RADAR_PRIORITY: i8 = 0;

struct Placed {
    id: u32,
    province: u32,
    screen: [f32; 2],
    depth: f32,
    state: State,
    anchor: Option<IconAnchor>,
}

pub(super) struct State {
    pub frame: i32,
    pub enabled: bool,
    pub sprite: Option<String>,
}

impl PreparedBases {
    pub(super) fn offer(
        &mut self,
        inputs: &CounterInputs<'_>,
        family: Family,
        id: u32,
        province: u32,
        anchor: [f32; 3],
        state: State,
    ) {
        let Some(projected) = inputs.camera.project_map_icon(
            anchor,
            inputs.viewport,
            gui_resources_viewport(inputs.viewport, inputs.ui_scale),
        ) else {
            return;
        };
        let cutoff = match family {
            Family::AirBase | Family::RocketSite | Family::GunEmplacement => {
                inputs.content.defines.air_base_cutoff
            }
            Family::Radar => inputs.content.defines.radar_cutoff,
            Family::Headquarter | Family::Facility => inputs.content.defines.facility_cutoff,
        };
        self.placed[family as usize].push(Placed {
            id,
            province,
            screen: projected.position,
            depth: projected.depth,
            state,
            anchor: inputs.icon_anchor(IconFamily::Bases, anchor, cutoff),
        });
    }

    pub(super) fn draw(
        &self,
        inputs: &CounterInputs<'_>,
        views: &mut [PoolView; FAMILIES],
        time: f32,
        countries: Option<&Countries>,
        batch: &mut Batch,
        report: &mut CounterReport,
    ) {
        let flags = gui_windows::window_flags(inputs, countries);
        let resources = gui_windows::gui_assets(inputs, flags.as_ref());
        let context = Context {
            resources: &resources,
            windows: inputs.windows,
            viewport: gui_resources_viewport(inputs.viewport, inputs.ui_scale),
            time: Some(time),
        };
        let schedule = inputs.schedule.lock().unwrap_or_else(|p| p.into_inner());
        let families = [
            (
                Family::AirBase,
                AIR_BASE_WINDOW,
                MapIconPool::AirBase,
                inputs.priority(PriorityIcon::AirBase),
            ),
            (
                Family::Headquarter,
                NAVAL_HEADQUARTER_WINDOW,
                MapIconPool::Headquarter,
                inputs.priority(PriorityIcon::NavalHeadquarter),
            ),
            (
                Family::Facility,
                FACILITY_WINDOW,
                MapIconPool::Facility,
                inputs.priority(PriorityIcon::Facility),
            ),
            (
                Family::RocketSite,
                ROCKET_SITE_WINDOW,
                MapIconPool::RocketSite,
                inputs.priority(PriorityIcon::RocketSite),
            ),
            (
                Family::GunEmplacement,
                GUN_EMPLACEMENT_WINDOW,
                MapIconPool::GunEmplacement,
                inputs.priority(PriorityIcon::GunEmplacement),
            ),
            (Family::Radar, RADAR_WINDOW, MapIconPool::Radar, RADAR_PRIORITY),
        ];
        let mut layouts = Vec::new();
        for &(family, name, kind, priority) in &families {
            for sprite in self.placed[family as usize]
                .iter()
                .filter_map(|icon| icon.state.sprite.as_ref())
            {
                if native_gui::Resources::sprite(&resources, sprite.as_bytes()).is_none() {
                    *report
                        .unresolved_facility_sprites
                        .entry(sprite.clone())
                        .or_default() += 1;
                }
            }
            let placed: HashMap<_, _> = self.placed[family as usize]
                .iter()
                .map(|icon| (icon.id, icon))
                .collect();
            let owners: Vec<_> = self.placed[family as usize]
                .iter()
                .map(|icon| Owner {
                    id: icon.id,
                    screen: icon.screen,
                    dirty: schedule.dirty(icon.province),
                })
                .collect();
            let drawn = views[family as usize].present(
                &context,
                &Plan {
                    name,
                    kind,
                    alignment: 0x12,
                    owners: &owners,
                },
                Box::new(move |_, root| bind(root, family).map(|_| ())),
                |owner, _, instance, root| update(instance, root, family, &placed[&owner].state),
                &mut report.icon_gui_notes,
            );
            layouts.extend(
                drawn
                    .into_iter()
                    .map(|drawn| (family, name, priority, placed[&drawn.owner], drawn)),
            );
        }
        // Sort map icons by projected depth.
        layouts.sort_by(|a, b| b.3.depth.total_cmp(&a.3.depth));
        for (family, name, priority, placed, drawn) in layouts {
            let key = layer(priority, report.base_ordinal(), 0);
            batch.at_layer(key);
            batch.at_anchor(placed.anchor);
            match native_gui::draw(&drawn.layout, drawn.origin, &resources, batch) {
                Ok(()) => {
                    if let Some(root) = inputs.windows.gui_type(name.as_bytes()) {
                        record_hit(root, &drawn.layout, drawn.origin, &resources, key, report);
                    }
                    match family {
                        Family::AirBase => report.air_bases += 1,
                        Family::Headquarter => report.naval_headquarters += 1,
                        Family::Facility => report.facilities += 1,
                        Family::RocketSite => report.rocket_sites += 1,
                        Family::GunEmplacement => report.gun_emplacements += 1,
                        Family::Radar => report.radars += 1,
                    }
                }
                Err(why) => count_refused(&mut report.icon_gui_notes, name, &[why]),
            }
        }
        batch.at_anchor(None);
    }
}

/// The binders find bg_btn locally; its actual drawn footprint intercepts unit input.
pub(super) fn record_hit(
    root: &native::Element,
    layout: &native_gui::Layout,
    origin: [i32; 2],
    resources: &dyn native_gui::Resources,
    layer: u64,
    report: &mut CounterReport,
) {
    let Some(button) = root.find(native::Kind::Button, "bg_btn") else {
        return;
    };
    if let Some(hit) = hit_rectangle(
        button,
        layout,
        origin,
        resources,
        &mut report.icon_gui_notes,
    ) {
        report
            .base_hits
            .push(viewer_foundation::map::counter_hovers::unit_hits::Blocker {
                rect: hit.rect,
                layer,
            });
    }
}

struct Controls<'a> {
    button: &'a native::Element,
    air: Option<[&'a native::Element; 4]>,
}

fn bind(root: &native::Element, family: Family) -> Result<Controls<'_>, String> {
    if family == Family::Radar {
        // The radar window's one control is its efficiency icon.
        let button = root
            .find(native::Kind::Icon, RADAR_WINDOW)
            .ok_or_else(|| format!("no icon named {RADAR_WINDOW}"))?;
        return Ok(Controls { button, air: None });
    }
    // Air-base, headquarters and facility controls use window-local lookup.
    let button = root
        .find(native::Kind::Button, "bg_btn")
        .ok_or("no button named bg_btn")?;
    let air = if family == Family::AirBase {
        let find = |kind, name: &str| {
            // Find buttons, instant text boxes and icons in the local window.
            root.find(kind, name)
                .ok_or_else(|| format!("no control named {name}"))
        };
        Some([
            find(native::Kind::Button, "bg_strat_btn")?,
            find(native::Kind::TextBox, "wings_count")?,
            find(native::Kind::TextBox, "permission_wings_count")?,
            find(native::Kind::Icon, "permission_wings_bg")?,
        ])
    } else {
        None
    };
    Ok(Controls { button, air })
}

fn update(
    instance: &mut Instance,
    root: &native::Element,
    family: Family,
    state: &State,
) -> Result<(), String> {
    let Controls { button, air } = bind(root, family)?;
    if matches!(family, Family::RocketSite | Family::GunEmplacement) {
        // The political-map branch: the large strategic button hides, and
        // both buttons take the frame and enabled state.
        let strategic = root
            .find(native::Kind::Button, "bg_strat_btn")
            .ok_or("no button named bg_strat_btn")?;
        instance.hide(root, strategic)?;
        instance.set_frame(root, strategic, state.frame)?;
        instance.control(root, strategic)?.enabled = Some(state.enabled);
        if family == Family::RocketSite
            && let Some(rockets) = root.find(native::Kind::Grid, "rockets_box")
        {
            instance.hide(root, rockets)?;
        }
    }
    if let Some([strategic, wings, permission_count, permission_bg]) = air {
        // Political-map visibility branch.
        instance.hide(root, strategic)?;
        instance.set_frame(root, strategic, state.frame)?;
        instance.control(root, strategic)?.enabled = Some(state.enabled);
        for control in [wings, permission_count, permission_bg] {
            instance.hide(root, control)?;
        }
    }
    if family == Family::Radar {
        // The game shows the icon in its default frame and writes nothing else.
        return instance.show(root, button);
    }
    // Air-base and headquarters updates write frames; facilities retain them.
    if family != Family::Facility {
        instance.set_frame(root, button, state.frame)?;
    }
    if let Some(sprite) = &state.sprite {
        instance.set_sprite(
            root,
            button,
            sprite.as_bytes(),
            SpriteSetter::ButtonStandard,
        )?;
    }
    instance.control(root, button)?.enabled = Some(state.enabled);
    instance.show(root, button)
}

#[cfg(test)]
mod tests {
    use super::*;
    use {
        native_gui::{PartKind, lifecycle::Tick},
        viewer_foundation::map::test_support::gui::fixture,
    };

    fn window(children: &str) -> native::Element {
        native::read(format!("guiTypes = {{ containerWindowType = {{ name = test size = {{ width = 50 height = 40 }} {children} }} }}").as_bytes())
            .template("test").unwrap().clone()
    }

    #[test]
    fn radar_preparation_in_free_views_keeps_nearby_elevated_anchors() {
        use super::super::tests::{action_gui, wings_frame, with_counter_inputs_fixture};
        use replay::{
            frame::TableView,
            rows::{RadarCoverageRow, StateRow, TableId, TableRows},
        };
        use viewer_foundation::map::camera::{Free, Lens, Pose};

        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (mut frame, vision) =
            wings_frame(TableView::Complete(TableRows::StateFlags(Vec::new())));
        *frame.tables[TableId::RadarCoverage.index()] =
            TableView::Complete(TableRows::RadarCoverage(vec![RadarCoverageRow {
                country: 100,
                ordinal: 0,
                state: Some(7),
                provinces: Vec::new(),
            }]));
        *frame.tables[TableId::States.index()] =
            TableView::Complete(TableRows::States(vec![StateRow {
                id: 7,
                controller: 100,
                ..Default::default()
            }]));
        let anchor = [2.0, 50.0, 2.0];
        with_counter_inputs_fixture(
            action_gui(false),
            |_| {},
            |content| {
                content.defines.radar_cutoff = 100.0;
                Arc::make_mut(&mut content.site_icons)[building_facts::RADAR].insert(
                    9,
                    building_facts::SiteIcon {
                        state: 7,
                        anchor: Ok(building_facts::IconAnchor::Record(anchor)),
                    },
                );
            },
            |mut inputs| {
                inputs.camera.height = 120.0;
                inputs.camera.pose =
                    Pose::Free(Free::looking_at([2.0, 120.0, 2.0], anchor, Lens::MAP));
                assert_eq!(inputs.camera.icon_distance(anchor), 70.0);
                let mut out = CounterFrame::default();
                bases::radars(&inputs, &frame, &vision, &mut out).unwrap();
                let placed = &out.native_bases.placed[Family::Radar as usize];
                // An absolute-height gate loses this icon before the free view can show it.
                assert_eq!(placed.len(), 1);
                assert_eq!(
                    placed[0].anchor,
                    Some(IconAnchor {
                        world: anchor,
                        cutoff: 100.0,
                        family: IconFamily::Bases,
                    })
                );

                // Free presentation does not bypass the map mode's province-icon gate.
                inputs.visibility.province_icons = false;
                let mut out = CounterFrame::default();
                bases::radars(&inputs, &frame, &vision, &mut out).unwrap();
                assert!(out.native_bases.placed[Family::Radar as usize].is_empty());

                // The game's camera still removes radar at the cutoff itself.
                inputs.visibility.province_icons = true;
                inputs.camera.pose = Pose::Map;
                inputs.camera.height = 100.0;
                let mut out = CounterFrame::default();
                bases::radars(&inputs, &frame, &vision, &mut out).unwrap();
                assert!(out.native_bases.placed[Family::Radar as usize].is_empty());
            },
        );
    }

    #[test]
    fn air_update_uses_local_buttons_and_hides_the_air_mode_controls() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let root = window(
            r#"
                buttonType = { name = bg_btn quadTextureSprite = GFX_even }
                buttonType = { name = bg_strat_btn quadTextureSprite = GFX_even }
                instantTextBoxType = { name = wings_count font = Main_14 text = authored }
                instantTextBoxType = { name = permission_wings_count font = Main_14 text = authored }
                iconType = { name = permission_wings_bg spriteType = GFX_even }
            containerWindowType = { name = nested position = { x = 5 y = 6 }
                iconType = { name = mod_decoration spriteType = GFX_even position = { x = 9 y = 10 } }
            }"#,
        );
        let resources = fixture();
        let mut instance = Instance::construct(&root, [800, 600], &resources).unwrap();
        instance.gui_show(&root, &root).unwrap();
        update(
            &mut instance,
            &root,
            Family::AirBase,
            &State {
                frame: 2,
                enabled: false,
                sprite: None,
            },
        )
        .unwrap();
        instance
            .tick(&root, &resources, Tick { time: 0., dt: 0. })
            .unwrap();
        let layout = instance.layout(&root, &resources, None).unwrap().unwrap();
        let names: Vec<_> = layout
            .parts
            .iter()
            .map(|p| p.path.rsplit('.').next().unwrap())
            .collect();
        assert_eq!(names, ["bg_btn", "mod_decoration"]);
        let button = bind(&root, Family::AirBase).unwrap().button;
        let control = instance.control(&root, button).unwrap();
        assert_eq!(control.frame, Some(2));
        assert_eq!(control.enabled, Some(false));
        assert!(
            matches!(&layout.parts[1].kind, PartKind::Sprite { geometry, .. } if geometry.rect[..2] == [14., 16.])
        );
    }

    /// The political-map branch of a rocket site or gun emplacement shows
    /// only `bg_btn`; both buttons take the frame and enabled state, and the
    /// rocket site's `rockets_box` hides. Before, neither icon was drawn.
    #[test]
    fn site_updates_show_the_plain_button_with_the_site_frame() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let root = window(
            r#"
                buttonType = { name = bg_btn quadTextureSprite = GFX_even }
                buttonType = { name = bg_strat_btn quadTextureSprite = GFX_even }
                gridboxtype = { name = rockets_box position = { x = -26 y = 35 } slotsize = { width = 58 height = 23 } }
            "#,
        );
        let resources = fixture();
        for family in [Family::RocketSite, Family::GunEmplacement] {
            let mut instance = Instance::construct(&root, [800, 600], &resources).unwrap();
            instance.gui_show(&root, &root).unwrap();
            update(
                &mut instance,
                &root,
                family,
                &State {
                    frame: 3,
                    enabled: false,
                    sprite: None,
                },
            )
            .unwrap();
            instance
                .tick(&root, &resources, Tick { time: 0., dt: 0. })
                .unwrap();
            let layout = instance.layout(&root, &resources, None).unwrap().unwrap();
            let names: Vec<_> = layout
                .parts
                .iter()
                .map(|p| p.path.rsplit('.').next().unwrap())
                .collect();
            assert_eq!(names, ["bg_btn"]);
            for name in ["bg_btn", "bg_strat_btn"] {
                let button = root.find(native::Kind::Button, name).unwrap();
                let control = instance.control(&root, button).unwrap();
                assert_eq!((control.frame, control.enabled), (Some(3), Some(false)));
            }
        }
        // A window without the strategic button is refused by name.
        let bare = window(r#"buttonType = { name = bg_btn quadTextureSprite = GFX_even }"#);
        let mut instance = Instance::construct(&bare, [800, 600], &resources).unwrap();
        let state = State {
            frame: 1,
            enabled: true,
            sprite: None,
        };
        assert!(
            update(&mut instance, &bare, Family::RocketSite, &state)
                .unwrap_err()
                .contains("bg_strat_btn")
        );
    }

    /// The radar window shows its one efficiency icon in its authored frame.
    #[test]
    fn radar_update_shows_its_icon() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let root = window(
            r#"iconType = { name = radar_mapicon spriteType = GFX_even frame = 2 }"#,
        );
        let resources = fixture();
        let mut instance = Instance::construct(&root, [800, 600], &resources).unwrap();
        instance.gui_show(&root, &root).unwrap();
        let state = State {
            frame: 0,
            enabled: true,
            sprite: None,
        };
        update(&mut instance, &root, Family::Radar, &state).unwrap();
        let layout = instance.layout(&root, &resources, None).unwrap().unwrap();
        assert_eq!(layout.parts.len(), 1);
        let icon = root.find(native::Kind::Icon, RADAR_WINDOW).unwrap();
        assert_eq!(instance.control(&root, icon).unwrap().frame, None);
        let bare = window(r#"buttonType = { name = bg_btn quadTextureSprite = GFX_even }"#);
        let mut instance = Instance::construct(&bare, [800, 600], &resources).unwrap();
        assert!(
            update(&mut instance, &bare, Family::Radar, &state)
                .unwrap_err()
                .contains(RADAR_WINDOW)
        );
    }

    #[test]
    fn base_hits_use_drawn_scale_clip_and_sprite_hit_mode() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let root = window(
            r#"buttonType = { name = bg_btn quadTextureSprite = GFX_even position = { x = 2 y = 3 } scale = 2 }"#,
        );
        let mut resources = fixture();
        let mut instance = Instance::construct(&root, [800, 600], &resources).unwrap();
        instance.gui_show(&root, &root).unwrap();
        let mut layout = instance.layout(&root, &resources, None).unwrap().unwrap();
        layout.parts[0].clip = Some([5, 4, 30, 15]);
        let key = layer(-3, 12, 0);
        for mode in [
            native_gui::Hit::Frame,
            native_gui::Hit::Through,
            native_gui::Hit::Alpha,
        ] {
            resources.sprites.get_mut("GFX_even").unwrap().hit = mode;
            let footprint = hit_rectangle(
                root.find(native::Kind::Button, "bg_btn").unwrap(),
                &layout,
                [100, 200],
                &resources,
                &mut BTreeMap::new(),
            );
            assert_eq!(footprint.is_some(), mode != native_gui::Hit::Through);
            if let Some(footprint) = footprint {
                assert_eq!(footprint.supported, mode == native_gui::Hit::Frame);
            }
            let mut report = CounterReport::default();
            record_hit(&root, &layout, [100, 200], &resources, key, &mut report);
            assert_eq!(
                report.base_hits.len(),
                usize::from(mode != native_gui::Hit::Through)
            );
            if let Some(hit) = report.base_hits.first() {
                assert_eq!(hit.rect, [105.0, 204.0, 25.0, 11.0]);
                assert_eq!(hit.layer, key);
            }
            assert_eq!(
                report.icon_gui_notes.len(),
                usize::from(mode == native_gui::Hit::Alpha)
            );
        }
        instance
            .hide(&root, root.find(native::Kind::Button, "bg_btn").unwrap())
            .unwrap();
        let layout = instance.layout(&root, &resources, None).unwrap().unwrap();
        let mut report = CounterReport::default();
        record_hit(&root, &layout, [100, 200], &resources, key, &mut report);
        assert!(report.base_hits.is_empty());
    }

    #[test]
    fn base_binders_do_not_bind_nested_controls() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let root = window(
            r#"containerWindowType={name=nested
            buttonType={name=bg_btn quadTextureSprite=GFX_even}}"#,
        );
        for family in [Family::AirBase, Family::Headquarter, Family::Facility] {
            assert!(bind(&root, family).err().unwrap().contains("bg_btn"));
        }
        assert!(
            naval_windows::update_naval(
                &mut Instance::construct(&root, [800, 600], &fixture()).unwrap(),
                &root,
                &NavalBaseIcon {
                    province: 1,
                    frame: 1,
                    enabled: true
                },
                1.0,
            )
            .unwrap_err()
            .contains("bg_btn")
        );
    }

    #[test]
    fn facility_setter_rebuilds_the_drawable_and_keeps_an_undefined_sprite() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let root = window(
            r#"buttonType = { name = bg_btn quadTextureSprite = GFX_even frame = 2 rotation = 1 }"#,
        );
        let resources = fixture();
        let mut instance = Instance::construct(&root, [800, 600], &resources).unwrap();
        instance.gui_show(&root, &root).unwrap();
        update(
            &mut instance,
            &root,
            Family::Facility,
            &State {
                frame: 0,
                enabled: true,
                sprite: Some("GFX_missing".into()),
            },
        )
        .unwrap();
        let kept = instance.layout(&root, &resources, None).unwrap().unwrap();
        assert!(
            matches!(&kept.parts[0].kind, PartKind::Sprite { geometry, .. } if geometry.rotation == 1.)
        );
        update(
            &mut instance,
            &root,
            Family::Facility,
            &State {
                frame: 0,
                enabled: true,
                sprite: Some("GFX_odd".into()),
            },
        )
        .unwrap();
        let rebuilt = instance.layout(&root, &resources, None).unwrap().unwrap();
        assert!(
            matches!(&rebuilt.parts[0].kind, PartKind::Sprite { uv, geometry, .. } if uv[0] == 0. && geometry.rotation == 0.)
        );
    }

    #[test]
    fn naval_update_overrides_authored_scale_and_hides_strategic_controls() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let root = window(
            r#"
            buttonType = { name = bg_btn quadTextureSprite = GFX_even scale = 3 }
            buttonType = { name = bg_strat_btn quadTextureSprite = GFX_even }
            instantTextBoxType = { name = txt_capacity font = Main_14 text = authored }
        "#,
        );
        let resources = fixture();
        let mut instance = Instance::construct(&root, [800, 600], &resources).unwrap();
        instance.gui_show(&root, &root).unwrap();
        naval_windows::update_naval(
            &mut instance,
            &root,
            &NavalBaseIcon {
                province: 1,
                frame: 10,
                enabled: false,
            },
            0.5,
        )
        .unwrap();
        let layout = instance.layout(&root, &resources, None).unwrap().unwrap();
        assert_eq!(layout.parts.len(), 1);
        let button = root.find(native::Kind::Button, "bg_btn").unwrap();
        assert!(
            matches!(&layout.parts[0].kind, PartKind::Sprite { uv, offsets, .. }
            if (offsets[0] / uv[2]).round() as i32 == 10)
        );
        assert_eq!(
            instance.drawable_extent(&root, button, &resources).unwrap(),
            Some([10, 5])
        );
    }
}
