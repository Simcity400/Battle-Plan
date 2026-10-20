//! Air-base, rocket-site, gun-emplacement, headquarters and facility windows, on the
//! native map-icon pools.

use super::*;
use std::collections::HashMap;
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

#[derive(Clone, Default)]
pub(super) struct PreparedBases {
    offers: Arc<Vec<Offer>>,
    placed: [Vec<Placed>; FAMILIES],
    pub(super) air_grids: BTreeMap<(usize, u32), Result<Vec<air_grid::PreparedItem>, String>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Family {
    AirBase,
    Headquarter,
    Facility,
    RocketSite,
    GunEmplacement,
    Radar,
    StrategicLocation,
}
impl Family {
    pub(super) fn wing_grid(self) -> Option<(usize, &'static str)> {
        match self {
            Self::AirBase => Some((0, "wings_box")),
            Self::RocketSite => Some((1, "rockets_box")),
            _ => None,
        }
    }
}

/// How many [`Family`] kinds there are, one pool view each.
pub(super) const FAMILIES: usize = 7;

#[derive(Default, Debug)]
pub struct View {
    pools: [PoolView; FAMILIES],
    wings: [air_grid::View; 2],
}
impl View {
    pub fn retries(&self) -> Vec<viewer_foundation::map::icon_windows::Retry> {
        self.pools.iter().filter_map(PoolView::retry).collect()
    }
    pub fn take_running(&mut self) -> bool {
        let mut running = false;
        for wings in &mut self.wings {
            running |= wings.take_running();
        }
        for pool in &mut self.pools {
            running |= pool.take_running();
        }
        running
    }
}

/// The radar icon's priority: the game's per-mode priority tables leave the
/// radar icon's entry zero in every mode, and no define sets it.
pub(super) const RADAR_PRIORITY: i8 = 0;

#[derive(Clone)]
struct Placed {
    id: u32,
    province: u32,
    screen: [f32; 2],
    depth: f32,
    state: State,
    anchor: Option<IconAnchor>,
}

#[derive(Clone)]
pub(super) struct State {
    pub frame: i32,
    pub enabled: bool,
    pub sprite: Option<String>,
    pub air: Option<bases::AirBaseCount>,
}

#[derive(Clone)]
struct Offer {
    family: Family,
    id: u32,
    province: u32,
    world: [f32; 3],
    state: State,
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
        let offer = Offer {
            family,
            id,
            province,
            world: anchor,
            state,
        };
        Self::project(&mut self.placed, inputs, &offer);
        Arc::make_mut(&mut self.offers).push(offer);
    }

    fn project(placed: &mut [Vec<Placed>; FAMILIES], inputs: &CounterInputs<'_>, offer: &Offer) {
        let Some(projected) = inputs.camera.project_map_icon(
            offer.world,
            inputs.viewport,
            gui_resources_viewport(inputs.viewport, inputs.ui_scale),
        ) else {
            return;
        };
        let cutoff = match offer.family {
            Family::AirBase
            | Family::RocketSite
            | Family::GunEmplacement
            | Family::StrategicLocation => inputs.content.defines.air_base_cutoff,
            Family::Radar => inputs.content.defines.radar_cutoff,
            Family::Headquarter | Family::Facility => inputs.content.defines.facility_cutoff,
        };
        placed[offer.family as usize].push(Placed {
            id: offer.id,
            province: offer.province,
            screen: projected.position,
            depth: projected.depth,
            state: offer.state.clone(),
            anchor: inputs.icon_anchor(IconFamily::Bases, offer.world, cutoff),
        });
    }

    /// Semantic offers survive a camera change; all projection and paint inputs are current.
    pub(super) fn for_camera(&self, inputs: &CounterInputs<'_>) -> Self {
        let mut result = Self {
            offers: Arc::clone(&self.offers),
            air_grids: self.air_grids.clone(),
            placed: Default::default(),
        };
        for offer in self.offers.iter() {
            Self::project(&mut result.placed, inputs, offer);
        }
        result
    }

    pub(super) fn draw(
        &self,
        inputs: &CounterInputs<'_>,
        views: &mut View,
        time: f32,
        countries: Option<&Countries>,
        batch: &mut Batch,
        report: &mut CounterReport,
    ) {
        let ticks: [_; 2] = std::array::from_fn(|at| views.wings[at].tick(time));
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
            (
                Family::Radar,
                RADAR_WINDOW,
                MapIconPool::Radar,
                RADAR_PRIORITY,
            ),
            (
                Family::StrategicLocation,
                STRATEGIC_LOCATION_WINDOW,
                MapIconPool::StrategicLocation,
                inputs.priority(PriorityIcon::StrategicLocation),
            ),
        ];
        let mut layouts = Vec::new();
        for &(family, name, kind, priority) in &families {
            // A strategic location's sprite is set only where the content
            // defines it; otherwise the button keeps its authored sprite.
            let named = family != Family::StrategicLocation;
            for sprite in self.placed[family as usize]
                .iter()
                .filter(|_| named)
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
            // A strategic location's icon fixes its location when its window is
            // acquired and keeps it while that window lives, as the game's icon
            // keeps the list it gathered when it was made.
            let view = &mut views.pools[family as usize];
            if let Some((at, _)) = family.wing_grid()
                && !view.same(&context)
            {
                views.wings[at].clear();
            }
            let mut acquired = std::mem::take(&mut view.acquired_sprites);
            let drawn = view.present(
                &context,
                &Plan {
                    name,
                    kind,
                    alignment: 0x12,
                    owners: &owners,
                },
                Box::new(move |_, root| bind(root, family).map(|_| ())),
                |owner, slot, instance, root| {
                    let state = &placed[&owner].state;
                    if family != Family::StrategicLocation {
                        update(instance, root, family, state)?;
                        if family == Family::AirBase {
                            air_text(instance, root, state, &resources)?;
                        }
                        if let Some((at, grid)) = family.wing_grid() {
                            views.wings[at].update(
                                owner,
                                slot,
                                instance,
                                root,
                                grid,
                                self.air_grids.get(&(at, owner)),
                                inputs,
                                countries,
                                &resources,
                            )?;
                        }
                        return Ok(());
                    }
                    let sprite = acquired_sprite(&mut acquired, owner, slot, &state.sprite);
                    let state = State {
                        air: None,
                        frame: state.frame,
                        enabled: state.enabled,
                        sprite,
                    };
                    update(instance, root, family, &state)
                },
                &mut report.icon_gui_notes,
            );
            forget_released(&mut acquired, &view.live());
            view.acquired_sprites = acquired;
            if let Some((at, _)) = family.wing_grid() {
                views.wings[at].retain(&view.live());
            }
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
            let composed = if let Some((at, grid)) = family.wing_grid() {
                views.wings[at].compose(
                    placed.id, &drawn, name, grid, inputs, &resources, ticks[at], key, report,
                )
            } else {
                Ok(vec![Vec::new(); drawn.layout.holders.len()])
            };
            let result = composed.and_then(|parts| {
                native_gui::draw_composed(&drawn.layout, &parts, drawn.origin, &resources, batch)
                    .map_err(|why| why.to_string())
            });
            match result {
                Ok(()) => {
                    if let Some(root) = inputs.windows.gui_type(name.as_bytes()) {
                        let first = report.base_hits.len();
                        record_hit(root, &drawn.layout, drawn.origin, &resources, key, report);
                        if family == Family::Radar {
                            for index in first..report.base_hits.len() {
                                report.radar_hits.insert(index, placed.province);
                            }
                        }
                        if family == Family::GunEmplacement {
                            let mut text = viewer_foundation::tooltip_text::Tooltip::default();
                            text.line(
                                false,
                                0,
                                "gun emplacement",
                                inputs
                                    .localisation
                                    .localise(b"GUN_EMPLACEMENT_BUILDING")
                                    .map_err(|why| why.to_string()),
                            );
                            let text = Arc::new(text);
                            for index in first..report.base_hits.len() {
                                report.air_grid_hits.insert(
                                    index,
                                    AirGridHover {
                                        members: None,
                                        text: Arc::clone(&text),
                                        family: IconFamily::Bases,
                                    },
                                );
                            }
                        }
                        if matches!(family, Family::AirBase | Family::RocketSite) {
                            for index in first..report.base_hits.len() {
                                report.air_base_hits.insert(
                                    index,
                                    (placed.province, i32::from(family == Family::RocketSite)),
                                );
                            }
                        }
                    }
                    match family {
                        Family::AirBase => report.air_bases += 1,
                        Family::Headquarter => report.naval_headquarters += 1,
                        Family::Facility => report.facilities += 1,
                        Family::RocketSite => report.rocket_sites += 1,
                        Family::GunEmplacement => report.gun_emplacements += 1,
                        Family::Radar => report.radars += 1,
                        Family::StrategicLocation => report.strategic_locations += 1,
                    }
                }
                Err(why) => {
                    *report
                        .icon_gui_notes
                        .entry(format!("{name} withheld: {why}"))
                        .or_default() += 1;
                }
            }
        }
        batch.at_anchor(None);
    }
}

/// The sprite an owner's window fixed when it was acquired: `offered` the
/// first time the owner holds `slot`, and that same choice while it keeps it.
pub(super) fn acquired_sprite<K: Eq + std::hash::Hash, S: PartialEq + Copy>(
    acquired: &mut HashMap<K, (S, Option<String>)>,
    owner: K,
    slot: S,
    offered: &Option<String>,
) -> Option<String> {
    match acquired.get(&owner) {
        Some((held, sprite)) if *held == slot => sprite.clone(),
        _ => {
            acquired.insert(owner, (slot, offered.clone()));
            offered.clone()
        }
    }
}

/// Drops the choices of windows no longer live, so an owner whose window
/// was released chooses afresh when it is acquired again.
pub(super) fn forget_released<K: Eq + std::hash::Hash, S: PartialEq + Copy>(
    acquired: &mut HashMap<K, (S, Option<String>)>,
    live: &[(K, S)],
) {
    acquired.retain(|owner, (slot, _)| live.iter().any(|(o, s)| o == owner && s == slot));
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
    for button in ["bg_btn", "bg_strat_btn"]
        .into_iter()
        .filter_map(|name| root.find(native::Kind::Button, name))
    {
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
        // Air uses the large strategic button. Both buttons always take the
        // frame and enabled state, including while hidden.
        let strategic = root
            .find(native::Kind::Button, "bg_strat_btn")
            .ok_or("no button named bg_strat_btn")?;
        if state.air.is_some() {
            instance.hide(root, button)?;
            instance.show(root, strategic)?;
        } else {
            instance.show(root, button)?;
            instance.hide(root, strategic)?;
        }
        instance.set_frame(root, strategic, state.frame)?;
        instance.control(root, strategic)?.enabled = Some(state.enabled);
        if family == Family::RocketSite
            && let Some(rockets) = root.find(native::Kind::Grid, "rockets_box")
        {
            instance.hide(root, rockets)?;
        }
    }
    if let Some([strategic, wings, permission_count, permission_bg]) = air {
        if state.air.is_some() {
            instance.hide(root, button)?;
            instance.show(root, strategic)?;
        } else {
            instance.show(root, button)?;
            instance.hide(root, strategic)?;
        }
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
    // Air-base and headquarters updates write frames; facilities and
    // strategic locations retain them.
    if !matches!(family, Family::Facility | Family::StrategicLocation) {
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
    if state.air.is_some() {
        Ok(())
    } else {
        instance.show(root, button)
    }
}

fn air_text(
    instance: &mut Instance,
    root: &native::Element,
    state: &State,
    resources: &dyn native_gui::Resources,
) -> Result<(), String> {
    let Some(count) = state.air.filter(|count| count.shown) else {
        return Ok(());
    };
    let wings = root
        .find(native::Kind::TextBox, "wings_count")
        .ok_or("no control named wings_count")?;
    let color = if count.planes > i64::from(count.capacity) {
        'R'
    } else {
        'W'
    };
    let control = viewer_foundation::map::native_text::COLOR;
    instance.set_text(
        root,
        wings,
        &format!(
            "{control}{color}{}/{}{control}!",
            count.planes, count.capacity
        ),
        resources,
    )?;
    instance.show(root, wings)?;
    if let Some((used, allowed)) = count.volunteers {
        let permission = root
            .find(native::Kind::TextBox, "permission_wings_count")
            .ok_or("no control named permission_wings_count")?;
        let background = root
            .find(native::Kind::Icon, "permission_wings_bg")
            .ok_or("no control named permission_wings_bg")?;
        let color = if used > allowed { 'R' } else { 'W' };
        instance.set_text(
            root,
            permission,
            &format!("{control}{color}{used}/{allowed}{control}!"),
            resources,
        )?;
        instance.show(root, permission)?;
        instance.show(root, background)?;
    }
    Ok(())
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
    fn air_site_style_uses_strategic_buttons_and_separate_rocket_wing_grid() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let root = window(
            r#"
            buttonType={name=bg_btn quadTextureSprite=GFX_even}
            buttonType={name=bg_strat_btn quadTextureSprite=GFX_even}
            gridboxtype={name=rockets_box slotsize={width=58 height=23}}
        "#,
        );
        let resources = fixture();
        for family in [Family::RocketSite, Family::GunEmplacement] {
            let mut instance = Instance::construct(&root, [800, 600], &resources).unwrap();
            instance.gui_show(&root, &root).unwrap();
            let mut state = State {
                frame: 2,
                enabled: true,
                sprite: None,
                air: Some(Default::default()),
            };
            update(&mut instance, &root, family, &state).unwrap();
            let shown = instance.layout(&root, &resources, None).unwrap().unwrap();
            assert!(
                shown
                    .parts
                    .iter()
                    .any(|part| part.path.ends_with("bg_strat_btn"))
            );
            assert!(!shown.parts.iter().any(|part| part.path.ends_with("bg_btn")));
            state.air = None;
            update(&mut instance, &root, family, &state).unwrap();
            let shown = instance.layout(&root, &resources, None).unwrap().unwrap();
            assert!(shown.parts.iter().any(|part| part.path.ends_with("bg_btn")));
            assert!(
                !shown
                    .parts
                    .iter()
                    .any(|part| part.path.ends_with("bg_strat_btn"))
            );
        }
        assert_eq!(Family::AirBase.wing_grid(), Some((0, "wings_box")));
        assert_eq!(Family::RocketSite.wing_grid(), Some((1, "rockets_box")));
        assert_eq!(Family::GunEmplacement.wing_grid(), None);
    }

    #[test]
    fn strategic_location_preparation_in_free_views_keeps_nearby_elevated_anchors() {
        use super::super::tests::{action_gui, wings_frame, with_counter_inputs_fixture};
        use replay_data::{
            rows::LexicalTokenRow, rows::ProvinceRow, rows::Recorded, rows::StateCoreRow,
            rows::TableId, rows::TableRows,
        };
        use replay_frame::frame::TableView;
        use viewer_foundation::map::camera::{Free, Lens, Pose};

        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (mut frame, _) = wings_frame(TableView::Complete(TableRows::StateFlags(Vec::new())));
        *frame.tables[TableId::Provinces.index()] =
            TableView::Complete(TableRows::Provinces(vec![ProvinceRow {
                id: 9,
                state: 7,
                controller: 100,
                ..Default::default()
            }]));
        *frame.tables[TableId::StateCores.index()] =
            TableView::Complete(TableRows::StateCores(vec![StateCoreRow {
                state: 7,
                strategic_locations: Recorded::Value(vec![(700u64 << 32) | 9]),
                province_strategic_locations: Recorded::Value(Vec::new()),
                ..Default::default()
            }]));
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 700,
                literal: b"synthetic_location".to_vec(),
            }]));
        let anchor = [2.0, 10.0, 2.0];
        with_counter_inputs_fixture(
            action_gui(false),
            |_| {},
            |content| {
                content.defines.air_base_cutoff = 50.0;
                Arc::make_mut(&mut content.anchors).insert((9, VICTORY_POINT), anchor);
            },
            |mut inputs| {
                let free = Pose::Free(Free::looking_at([2.0, 50.0, 2.0], anchor, Lens::MAP));
                inputs.camera.pose = free;
                inputs.camera.height = 50.0;
                assert_eq!(inputs.camera.icon_distance(anchor), 40.0);
                // Only the free view keeps this nearby icon at the absolute-height cutoff.
                // Province-icon mode suppression and the ordinary camera's strict cutoff remain.
                for (pose, height, province_icons, expected) in [
                    (free, 50.0, true, 1),
                    (free, 50.0, false, 0),
                    (Pose::Map, 50.0, true, 0),
                    (Pose::Map, 49.0, true, 1),
                ] {
                    inputs.camera.pose = pose;
                    inputs.camera.height = height;
                    inputs.visibility.province_icons = province_icons;
                    let mut out = CounterFrame::default();
                    bases::strategic_location_icons(&inputs, &frame, &mut out).unwrap();
                    let placed = &out.native_bases.placed[Family::StrategicLocation as usize];
                    assert_eq!(placed.len(), expected, "{pose:?}, height {height}");
                    if let Some(placed) = placed.first() {
                        assert_eq!(
                            placed.anchor,
                            Some(IconAnchor {
                                world: anchor,
                                cutoff: 50.0,
                                family: IconFamily::Bases,
                            })
                        );
                    }
                }
            },
        );
    }

    #[test]
    fn radar_preparation_in_free_views_keeps_nearby_elevated_anchors() {
        use super::super::tests::{action_gui, wings_frame, with_counter_inputs_fixture};
        use replay_data::{rows::RadarCoverageRow, rows::StateRow, rows::TableId, rows::TableRows};
        use replay_frame::frame::TableView;
        use viewer_foundation::map::camera::{Free, Lens, Pose};

        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (mut frame, vision) =
            wings_frame(TableView::Complete(TableRows::StateFlags(Vec::new())));
        *frame.tables[TableId::RadarCoverage.index()] =
            TableView::Complete(TableRows::RadarCoverage(vec![RadarCoverageRow {
                country: 100,
                ordinal: 0,
                state: Some(7),
                provinces: Vec::new(),
                radius: None,
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
        engine_executable::executable_tables::initialize_for_tests()
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
                air: None,
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
        engine_executable::executable_tables::initialize_for_tests()
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
                    air: None,
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
            air: None,
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

    /// A strategic location's window keeps the location it took when it was
    /// acquired: a later offer changes nothing while the window lives, and
    /// a released window chooses afresh when its owner acquires one again.
    /// Before, the sprite followed every frame's offer.
    #[test]
    fn a_strategic_location_keeps_its_sprite_for_its_window_s_lifetime() {
        let mut acquired: HashMap<u32, (u32, Option<String>)> = HashMap::new();
        let offer = |name: &str| Some(name.to_owned());
        assert_eq!(
            acquired_sprite(&mut acquired, 7, 1, &offer("A")),
            offer("A")
        );
        forget_released(&mut acquired, &[(7, 1)]);
        // A province token added while the icon is visible keeps the first.
        assert_eq!(
            acquired_sprite(&mut acquired, 7, 1, &offer("B")),
            offer("A")
        );
        // Released, then acquired again: the current choice.
        forget_released(&mut acquired, &[]);
        assert!(acquired.is_empty());
        assert_eq!(
            acquired_sprite(&mut acquired, 7, 1, &offer("B")),
            offer("B")
        );
        // A different slot is a new window.
        assert_eq!(
            acquired_sprite(&mut acquired, 7, 2, &offer("C")),
            offer("C")
        );
    }

    /// The radar window shows its one efficiency icon in its authored frame.
    #[test]
    fn radar_update_shows_its_icon() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let root = window(r#"iconType = { name = radar_mapicon spriteType = GFX_even frame = 2 }"#);
        let resources = fixture();
        let mut instance = Instance::construct(&root, [800, 600], &resources).unwrap();
        instance.gui_show(&root, &root).unwrap();
        let state = State {
            air: None,
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

    /// A strategic location sets its location's sprite on `bg_btn` and keeps
    /// the authored frame; an undefined sprite keeps the authored one.
    #[test]
    fn strategic_location_update_sets_the_location_sprite_and_keeps_the_frame() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let root =
            window(r#"buttonType = { name = bg_btn quadTextureSprite = GFX_even frame = 2 }"#);
        let resources = fixture();
        let mut instance = Instance::construct(&root, [800, 600], &resources).unwrap();
        instance.gui_show(&root, &root).unwrap();
        let button = root.find(native::Kind::Button, "bg_btn").unwrap();
        let state = |sprite: &str| State {
            air: None,
            frame: 0,
            enabled: true,
            sprite: Some(sprite.into()),
        };
        update(
            &mut instance,
            &root,
            Family::StrategicLocation,
            &state("GFX_missing"),
        )
        .unwrap();
        assert_eq!(instance.control(&root, button).unwrap().frame, None);
        update(
            &mut instance,
            &root,
            Family::StrategicLocation,
            &state("GFX_odd"),
        )
        .unwrap();
        let layout = instance.layout(&root, &resources, None).unwrap().unwrap();
        assert_eq!(layout.parts.len(), 1);
        assert_eq!(instance.control(&root, button).unwrap().frame, None);
    }

    #[test]
    fn base_hits_use_drawn_scale_clip_and_sprite_hit_mode() {
        engine_executable::executable_tables::initialize_for_tests()
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
        engine_executable::executable_tables::initialize_for_tests()
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
                    capacity: None,
                    capacity_refusal: None,
                    province: 1,
                    frame: 1,
                    highlighted: false,
                    enabled: true
                },
                1.0,
                false,
                &fixture()
            )
            .unwrap_err()
            .contains("bg_btn")
        );
    }

    #[test]
    fn facility_setter_rebuilds_the_drawable_and_keeps_an_undefined_sprite() {
        engine_executable::executable_tables::initialize_for_tests()
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
                air: None,
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
                air: None,
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
        engine_executable::executable_tables::initialize_for_tests()
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
                capacity: None,
                capacity_refusal: None,
                province: 1,
                frame: 10,
                highlighted: false,
                enabled: false,
            },
            0.5,
            false,
            &resources,
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
        naval_windows::update_naval(
            &mut instance,
            &root,
            &NavalBaseIcon {
                capacity: None,
                capacity_refusal: None,
                province: 1,
                frame: 10,
                highlighted: false,
                enabled: false,
            },
            0.5,
            true,
            &resources,
        )
        .unwrap();
        let naval = instance.layout(&root, &resources, None).unwrap().unwrap();
        assert_eq!(naval.parts.len(), 1);
        assert!(naval.parts[0].path.ends_with("bg_strat_btn"));
    }

    #[test]
    fn navy_mode_draws_repair_capacity_and_clears_it_when_intel_or_mode_hides_it() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let root = window(
            r#"
            buttonType={name=bg_btn quadTextureSprite=GFX_even}
            buttonType={name=bg_strat_btn quadTextureSprite=GFX_even}
            instantTextBoxType={name=txt_capacity font=Main_14 text=authored}
        "#,
        );
        let resources = fixture();
        let mut instance = Instance::construct(&root, [800, 600], &resources).unwrap();
        instance.gui_show(&root, &root).unwrap();
        let text = root.find(native::Kind::TextBox, "txt_capacity").unwrap();
        let mut glyph = NavalBaseIcon {
            province: 1,
            frame: 2,
            highlighted: false,
            enabled: true,
            capacity: Some(naval_repairs::Capacity {
                used: 2,
                level: 4,
                colour: Some('R'),
            }),
            capacity_refusal: None,
        };
        naval_windows::update_naval(&mut instance, &root, &glyph, 1.0, true, &resources).unwrap();
        assert_eq!(
            instance.control(&root, text).unwrap().text.as_deref(),
            Some(glyph.capacity.unwrap().text().as_str())
        );
        let shown = |instance: &Instance| {
            instance
                .layout(&root, &resources, None)
                .unwrap()
                .unwrap()
                .parts
                .iter()
                .any(|part| part.path.ends_with("txt_capacity"))
        };
        assert!(shown(&instance));
        naval_windows::update_naval(&mut instance, &root, &glyph, 1.0, false, &resources).unwrap();
        assert!(!shown(&instance));
        glyph.capacity = None;
        naval_windows::update_naval(&mut instance, &root, &glyph, 1.0, true, &resources).unwrap();
        assert!(!shown(&instance));
        glyph.highlighted = true;
        naval_windows::update_naval(&mut instance, &root, &glyph, 1.7, true, &resources).unwrap();
        // Ordered frame setters reach both drawables in the frame's clock phase,
        // including the hidden ordinary button.
        instance
            .tick_clocks(&root, &resources, Tick { time: 1.0, dt: 0.0 })
            .unwrap();
        for name in ["bg_btn", "bg_strat_btn"] {
            let button = root.find(native::Kind::Button, name).unwrap();
            let control = instance.control(&root, button).unwrap();
            assert_eq!(control.frame, Some(11));
            assert_eq!(control.scale, Some(1.7));
        }
        glyph.highlighted = false;
        naval_windows::update_naval(&mut instance, &root, &glyph, 0.8, true, &resources).unwrap();
        instance
            .tick_clocks(&root, &resources, Tick { time: 1.0, dt: 0.0 })
            .unwrap();
        assert_eq!(
            instance
                .control(
                    &root,
                    root.find(native::Kind::Button, "bg_strat_btn").unwrap()
                )
                .unwrap()
                .frame,
            Some(3)
        );
    }

    #[test]
    fn air_mode_uses_the_strategic_button_and_colors_its_aircraft_capacity_text() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let root = window(
            r#"
            buttonType={name=bg_btn quadTextureSprite=GFX_even}
            buttonType={name=bg_strat_btn quadTextureSprite=GFX_even}
            instantTextBoxType={name=wings_count font=Main_14 text=authored}
            instantTextBoxType={name=permission_wings_count font=Main_14 text=authored}
            iconType={name=permission_wings_bg spriteType=GFX_even}
        "#,
        );
        let resources = fixture();
        let mut instance = Instance::construct(&root, [800, 600], &resources).unwrap();
        instance.gui_show(&root, &root).unwrap();
        let color = viewer_foundation::map::native_text::COLOR;
        for (planes, shown, expected) in [
            (201, true, Some(format!("{color}R201/200{color}!"))),
            (200, true, Some(format!("{color}W200/200{color}!"))),
            (199, false, None),
        ] {
            let state = State {
                frame: 2,
                enabled: true,
                sprite: None,
                air: Some(bases::AirBaseCount {
                    planes,
                    capacity: 200,
                    shown,
                    ..Default::default()
                }),
            };
            update(&mut instance, &root, Family::AirBase, &state).unwrap();
            air_text(&mut instance, &root, &state, &resources).unwrap();
            let layout = instance.layout(&root, &resources, None).unwrap().unwrap();
            assert!(
                layout
                    .parts
                    .iter()
                    .any(|part| part.path.ends_with("bg_strat_btn"))
            );
            assert!(
                !layout
                    .parts
                    .iter()
                    .any(|part| part.path.ends_with("bg_btn"))
            );
            let wings = root.find(native::Kind::TextBox, "wings_count").unwrap();
            assert_eq!(
                layout
                    .parts
                    .iter()
                    .any(|part| part.path.ends_with("wings_count")),
                shown
            );
            if let Some(expected) = expected {
                assert_eq!(
                    instance.control(&root, wings).unwrap().text.as_deref(),
                    Some(expected.as_str())
                );
            }
        }
    }
    #[test]
    fn air_permission_count_and_background_follow_selection_and_native_warning_colour() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let root = window(
            r#"
            buttonType={name=bg_btn quadTextureSprite=GFX_even}
            buttonType={name=bg_strat_btn quadTextureSprite=GFX_even}
            instantTextBoxType={name=wings_count font=Main_14 text=authored}
            instantTextBoxType={name=permission_wings_count font=Main_14 text=authored}
            iconType={name=permission_wings_bg spriteType=GFX_even}
        "#,
        );
        let resources = fixture();
        let mut instance = Instance::construct(&root, [800, 600], &resources).unwrap();
        instance.gui_show(&root, &root).unwrap();
        let color = viewer_foundation::map::native_text::COLOR;
        for (shown, volunteers, text) in [
            (true, Some((2, 2)), Some(format!("{color}W2/2{color}!"))),
            (true, Some((3, 2)), Some(format!("{color}R3/2{color}!"))),
            (true, None, None),
            (false, Some((1, 2)), None),
        ] {
            let state = State {
                frame: 2,
                enabled: true,
                sprite: None,
                air: Some(bases::AirBaseCount {
                    shown,
                    volunteers,
                    capacity: 200,
                    ..Default::default()
                }),
            };
            update(&mut instance, &root, Family::AirBase, &state).unwrap();
            air_text(&mut instance, &root, &state, &resources).unwrap();
            let layout = instance.layout(&root, &resources, None).unwrap().unwrap();
            for name in ["permission_wings_count", "permission_wings_bg"] {
                assert_eq!(
                    layout.parts.iter().any(|part| part.path.ends_with(name)),
                    text.is_some()
                );
            }
            if let Some(text) = text {
                let control = root
                    .find(native::Kind::TextBox, "permission_wings_count")
                    .unwrap();
                assert_eq!(
                    instance.control(&root, control).unwrap().text.as_deref(),
                    Some(text.as_str())
                );
            }
        }
    }

    #[test]
    fn retained_base_offers_reproject_every_family_in_order_including_previously_hidden_anchors() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        use viewer_foundation::map::camera::{Free, Lens, Pose};
        crate::map::counters::tests::with_counter_inputs_fixture(
            crate::map::counters::tests::action_gui(false),
            |_| {},
            |_| {},
            |mut inputs| {
                let families = [
                    Family::AirBase,
                    Family::Headquarter,
                    Family::Facility,
                    Family::RocketSite,
                    Family::GunEmplacement,
                    Family::Radar,
                    Family::StrategicLocation,
                ];
                let offer = |target: &mut PreparedBases, inputs: &CounterInputs<'_>| {
                    for (at, family) in families.into_iter().enumerate() {
                        for side in 0..2 {
                            target.offer(
                                inputs,
                                family,
                                (at * 2 + side) as u32,
                                at as u32,
                                [2., 9.5, if side == 0 { -20. } else { 20. }],
                                State {
                                    frame: at as i32,
                                    enabled: side == 0,
                                    sprite: Some(format!("synthetic_{at}")),
                                    air: Some(bases::AirBaseCount {
                                        capacity: 7,
                                        ..Default::default()
                                    }),
                                },
                            );
                        }
                    }
                };
                inputs.camera.pose = Pose::Free(Free::new(0., 0., Lens::MAP));
                inputs.camera.height = 9.5;
                let mut retained = PreparedBases::default();
                offer(&mut retained, &inputs);
                assert_eq!(retained.offers.len(), families.len() * 2);
                assert!(retained.placed.iter().all(|placed| placed.len() == 1));
                for (step, yaw) in [std::f32::consts::PI, 0., std::f32::consts::PI, 0.]
                    .into_iter()
                    .enumerate()
                {
                    inputs.camera.pose = Pose::Free(Free::new(yaw, 0., Lens::MAP));
                    inputs.camera.center[0] += 0.125;
                    inputs.ui_scale = if step % 2 == 0 { 1.25 } else { 1. };
                    let cached = retained.for_camera(&inputs);
                    let mut fresh = PreparedBases::default();
                    offer(&mut fresh, &inputs);
                    for (a, b) in cached.placed.iter().zip(&fresh.placed) {
                        assert_eq!(a.len(), b.len());
                        for (a, b) in a.iter().zip(b) {
                            assert_eq!((a.id, a.province), (b.id, b.province));
                            assert_eq!(a.screen.map(f32::to_bits), b.screen.map(f32::to_bits));
                            assert_eq!(a.depth.to_bits(), b.depth.to_bits());
                            assert_eq!(a.anchor, b.anchor);
                            assert_eq!(
                                (a.state.frame, a.state.enabled, &a.state.sprite, a.state.air),
                                (b.state.frame, b.state.enabled, &b.state.sprite, b.state.air)
                            );
                        }
                    }
                    assert!(Arc::ptr_eq(&cached.offers, &retained.offers));
                }
            },
        );
    }
}
