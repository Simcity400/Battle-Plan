//! Per-view Air wing windows. Pool membership, clocks and selection are
//! presentation state; every simulation input comes from the prepared hour.
use super::super::*;
use super::{PreparedItem, prepared};
use std::collections::HashMap;
use viewer_foundation::map::{
    gui_resources::hit_rectangle,
    icon_windows::{self, Drawn},
    native_gui::{
        self,
        lifecycle::{Instance, SlotId, Tick},
    },
    native_windows::AIR_WING_WINDOW as WINDOW,
};

#[derive(Debug)]
struct Entry {
    instance: Instance,
    data: Option<PreparedItem>,
}
#[derive(Default, Debug)]
pub(crate) struct View {
    entries: HashMap<u32, (Option<SlotId>, Vec<Entry>)>,
    refused: HashMap<u32, String>,
    last: Option<f32>,
    active: bool,
}
/// One icon's composed wing grid: its parts, and the hits it takes once the icon draws.
#[derive(Debug)]
pub(crate) struct Composed {
    pub parts: Vec<Vec<native_gui::Part>>,
    hits: Vec<(
        viewer_foundation::map::counter_hovers::unit_hits::Blocker,
        AirGridHover,
    )>,
}
impl Composed {
    /// No wing grid: the icon's own holders, empty.
    pub fn empty(holders: usize) -> Self {
        Self {
            parts: vec![Vec::new(); holders],
            hits: Vec::new(),
        }
    }
    /// Publishes the grid's hits; only an icon that drew takes them.
    pub fn commit(self, report: &mut CounterReport) {
        for (blocker, hover) in self.hits {
            report.air_grid_hits.insert(report.base_hits.len(), hover);
            report.base_hits.push(blocker);
        }
    }
}
impl View {
    pub fn clear(&mut self) {
        self.entries.clear();
        self.refused.clear();
        self.last = None;
    }
    pub fn tick(&mut self, time: f32) -> Tick {
        self.active = false;
        icon_windows::frame(&mut self.last, time)
    }
    pub fn take_running(&mut self) -> bool {
        std::mem::take(&mut self.active)
    }
    /// Whether an item changed on its own at the last frame, not cleared.
    pub fn running(&self) -> bool {
        self.active
    }
    pub fn retain(&mut self, live: &[(u32, SlotId)]) {
        self.entries
            .retain(|owner, (slot, _)| slot.is_some_and(|slot| live.contains(&(*owner, slot))));
        self.refused
            .retain(|owner, _| live.iter().any(|(id, _)| id == owner));
    }
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        owner: u32,
        slot: impl Into<Option<SlotId>>,
        instance: &mut Instance,
        root: &native::Element,
        grid_name: &str,
        offered: Option<&Result<Vec<PreparedItem>, String>>,
        inputs: &CounterInputs<'_>,
        countries: Option<&Countries>,
        resources: &dyn native_gui::Resources,
    ) -> Result<(), String> {
        let slot = slot.into();
        self.refused.remove(&owner);
        let grid = root.find(native::Kind::Grid, grid_name);
        let values = match offered {
            Some(Ok(values)) => values.as_slice(),
            _ => {
                if let Some(Err(why)) = offered {
                    self.refused.insert(owner, why.clone());
                }
                if let Some(grid) = grid {
                    instance.hide(root, grid)?;
                }
                if let Some((_, entries)) = self.entries.get_mut(&owner)
                    && let Some(item) = inputs.windows.gui_type(WINDOW.as_bytes())
                {
                    for entry in entries {
                        entry.instance.hide(item, item)?;
                        entry.data = None;
                    }
                }
                return Ok(());
            }
        };
        let Some(grid) = grid else {
            self.refused
                .insert(owner, format!("Air base window has no {grid_name} grid"));
            return Ok(());
        };
        instance.show(root, grid)?;
        let item = inputs
            .windows
            .gui_type(WINDOW.as_bytes())
            .ok_or("Air wing counter template is absent")?;
        let viewport = gui_resources_viewport(inputs.viewport, inputs.ui_scale);
        let entry = self
            .entries
            .entry(owner)
            .or_insert_with(|| (slot, Vec::new()));
        if entry.0 != slot {
            entry.0 = slot;
            entry.1.clear();
        }
        while entry.1.len() < values.len() {
            entry.1.push(Entry {
                instance: Instance::construct(item, viewport, resources)?,
                data: None,
            });
        }
        for (index, entry) in entry.1.iter_mut().enumerate() {
            entry.data = values.get(index).cloned();
            if let Some(data) = &entry.data {
                update_item(
                    &mut entry.instance,
                    item,
                    data,
                    inputs,
                    countries,
                    resources,
                )?;
            } else {
                entry.instance.hide(item, item)?;
            }
        }
        let layout = instance
            .layout(root, resources, None)
            .map_err(|e| format!("{e:?}"))?;
        let Some(holder) = layout.as_ref().and_then(|layout| {
            layout
                .holders
                .iter()
                .find(|holder| holder.path == grid.path)
        }) else {
            return Ok(());
        };
        let placed = instance
            .finalize_grid(root, grid, holder, &vec![item; values.len()])
            .map_err(|e| format!("{e:?}"))?;
        for (entry, place) in entry.1.iter_mut().zip(placed.items) {
            entry.instance.set_position(item, item, place)?;
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub fn compose(
        &mut self,
        owner: u32,
        drawn: &Drawn,
        root_name: &str,
        grid_name: &str,
        inputs: &CounterInputs<'_>,
        resources: &dyn native_gui::Resources,
        tick: Tick,
        layer: u64,
        report: &mut CounterReport,
    ) -> Result<Composed, String> {
        self.compose_held(
            owner,
            drawn,
            root_name,
            grid_name,
            inputs,
            resources,
            tick,
            layer,
            None,
            IconFamily::Bases,
            report,
        )
    }
    /// Compose in the item's own coordinates; apply the outer holder to hit
    /// regions only. The caller places the composed drawing through that holder,
    /// and commits its hits once that drawing succeeds.
    #[allow(clippy::too_many_arguments)]
    pub fn compose_held(
        &mut self,
        owner: u32,
        drawn: &Drawn,
        root_name: &str,
        grid_name: &str,
        inputs: &CounterInputs<'_>,
        resources: &dyn native_gui::Resources,
        tick: Tick,
        layer: u64,
        outer: Option<(&native_gui::Holder, [i32; 2])>,
        family: IconFamily,
        report: &mut CounterReport,
    ) -> Result<Composed, String> {
        let mut composed = Composed::empty(drawn.layout.holders.len());
        if let Some(why) = self.refused.get(&owner) {
            *report
                .icon_gui_notes
                .entry(format!("{root_name} {grid_name} withheld: {why}"))
                .or_default() += 1;
        }
        let Some((_, entries)) = self.entries.get_mut(&owner) else {
            return Ok(composed);
        };
        let item = inputs
            .windows
            .gui_type(WINDOW.as_bytes())
            .ok_or("Air wing counter template is absent")?;
        let root = inputs
            .windows
            .gui_type(root_name.as_bytes())
            .ok_or("Air base counter template is absent")?;
        let Some(grid) = root.find(native::Kind::Grid, grid_name) else {
            return Ok(composed);
        };
        let holder = drawn
            .layout
            .holders
            .iter()
            .position(|holder| holder.path == grid.path);
        for entry in entries {
            entry.instance.tick(item, resources, tick)?;
            self.active |= entry.instance.active();
            let Some(data) = &entry.data else {
                continue;
            };
            for (why, count) in &data.refused {
                *report.icon_gui_notes.entry(why.clone()).or_default() += count;
            }
            let Some(at) = holder else {
                continue;
            };
            let Some(mut layout) = entry
                .instance
                .layout(item, resources, None)
                .map_err(|e| format!("{e:?}"))?
            else {
                continue;
            };
            let holder = &drawn.layout.holders[at];
            layout.parts = holder.place(entry.instance.position(), &layout);
            layout.regions = holder.place_regions(entry.instance.position(), &layout.regions);
            composed.parts[at].extend(layout.parts.iter().cloned());
            if let Some((outer, place)) = outer {
                layout.parts = outer.place(place, &layout);
                layout.regions = outer.place_regions(place, &layout.regions);
            }
            for (name, kind, text, members) in [
                (
                    "count_bg",
                    native::Kind::Button,
                    Arc::clone(&data.hover),
                    Some(Arc::new(data.item.members.clone())),
                ),
                (
                    "no_mission",
                    native::Kind::Icon,
                    Arc::clone(&data.marker_hovers[0]),
                    None,
                ),
                (
                    "no_region",
                    native::Kind::Icon,
                    Arc::clone(&data.marker_hovers[1]),
                    None,
                ),
                (
                    "training",
                    native::Kind::Icon,
                    Arc::clone(&data.marker_hovers[2]),
                    None,
                ),
            ] {
                let Some(control) = item.find(kind, name) else {
                    continue;
                };
                if let Some(hit) = hit_rectangle(
                    control,
                    &layout,
                    drawn.origin,
                    resources,
                    &mut report.icon_gui_notes,
                ) {
                    composed.hits.push((
                        viewer_foundation::map::counter_hovers::unit_hits::Blocker {
                            rect: hit.rect,
                            layer,
                        },
                        AirGridHover {
                            members,
                            text,
                            family,
                        },
                    ));
                }
            }
        }
        Ok(composed)
    }
}
fn visible(
    instance: &mut Instance,
    root: &native::Element,
    kind: native::Kind,
    name: &str,
    shown: bool,
) -> Result<(), String> {
    let control = root
        .find(kind, name)
        .ok_or_else(|| format!("Air wing counter has no {name}"))?;
    instance.set_hide_flag(root, control, !shown)?;
    if shown {
        instance.show(root, control)
    } else {
        instance.hide(root, control)
    }
}
fn update_item(
    instance: &mut Instance,
    root: &native::Element,
    data: &PreparedItem,
    inputs: &CounterInputs<'_>,
    countries: Option<&Countries>,
    resources: &dyn native_gui::Resources,
) -> Result<(), String> {
    use native::Kind::{Button, Icon, TextBox};
    let find = |kind, name: &str| {
        root.find(kind, name)
            .ok_or_else(|| format!("Air wing counter has no {name}"))
    };
    instance.set_frame(root, find(Icon, "wing_type")?, data.icon_frame)?;
    let count = find(TextBox, "count_txt")?;
    let text = if data.item.planes < 1000 {
        data.item.planes.to_string()
    } else {
        installed_content_localisation::localisation::format_scripted_variable(
            i64::from(data.item.planes) * 100_000,
            Some(b"*1"),
        )
    };
    instance.set_text(root, count, &text, resources)?;
    visible(instance, root, TextBox, "count_txt", true)?;
    visible(instance, root, Icon, "wing_type", true)?;
    visible(instance, root, Icon, "counter_overlay", true)?;
    visible(instance, root, Icon, "count_selected", data.item.selected)?;
    visible(
        instance,
        root,
        Icon,
        "group_marker",
        data.group_color.is_some(),
    )?;
    if let Some(color) = data.group_color {
        instance.control(root, find(Icon, "group_marker")?)?.color = Some(color);
    }
    visible(
        instance,
        root,
        Icon,
        "onmap_action_icon",
        data.action.is_some(),
    )?;
    if let Some(action) = data.action {
        instance.set_frame(root, find(Icon, "onmap_action_icon")?, action)?;
    }
    for (name, value) in [
        "no_mission",
        "no_region",
        "training",
        "training_will_stop_at_max_icon",
    ]
    .into_iter()
    .zip(data.markers)
    {
        visible(instance, root, Icon, name, value == Some(true))?;
    }
    let plate = countries
        .zip(data.country)
        .and_then(|(countries, country)| {
            countries.sides.get(&country).copied().and_then(|side| {
                counter_plate_color(
                    inputs.settings.color_mode,
                    &inputs.content.defines.tints,
                    countries,
                    side,
                    Some(country),
                )
                .ok()
            })
        });
    visible(instance, root, Button, "count_bg", plate.is_some())?;
    if let Some(color) = plate {
        instance.control(root, find(Button, "count_bg")?)?.color = Some(color);
    }
    let texture = countries.zip(data.country).and_then(|(countries, slot)| {
        use gui_windows::NavyArt;
        gui_windows::CounterArt {
            assets: inputs.assets,
            countries,
        }
        .flag(slot, false)
    });
    visible(instance, root, Icon, "flag", texture.is_some())?;
    if let Some(texture) = texture {
        instance.control(root, find(Icon, "flag")?)?.texture = Some(texture);
    }
    visible(instance, root, Icon, "bar_str", data.capacity.is_some())?;
    if let Some(capacity) = data.capacity {
        let bar = find(Icon, "bar_str")?;
        let fill = prepared::ratio(data.item.planes, capacity) as f32 / 100_000.0;
        let extent = native_gui::drawable_extent(bar, resources, &native_gui::Bindings::default())
            .map_err(|e| format!("{e:?}"))?;
        let offset = counter_inputs::bar_step(fill, extent.map_or(0, |size| size[1]) as f32) as i32;
        let bound = instance.control(root, bar)?;
        bound.stretch = Some(fill);
        bound.offset = Some([offset, 0]);
    }
    instance.gui_show(root, root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use viewer_foundation::map::test_support::gui::fixture;
    fn window_bytes() -> &'static [u8] {
        br#"guiTypes={containerWindowType={name=air_wing_counter size={width=58 height=27}
            buttonType={name=count_bg quadTextureSprite=GFX_even}
            iconType={name=wing_type spriteType=GFX_even}
            iconType={name=counter_overlay spriteType=GFX_even}
            iconType={name=count_selected spriteType=GFX_even}
            iconType={name=group_marker spriteType=GFX_even}
            iconType={name=onmap_action_icon spriteType=GFX_even}
            iconType={name=no_mission spriteType=GFX_even}
            iconType={name=no_region spriteType=GFX_even}
            iconType={name=training spriteType=GFX_even}
            iconType={name=training_will_stop_at_max_icon spriteType=GFX_even}
            iconType={name=flag spriteType=GFX_even}
            iconType={name=bar_str spriteType=GFX_even}
            instantTextboxType={name=count_txt font=Main_14 maxWidth=50 maxHeight=14}
        }}"#
    }
    fn window() -> native::Element {
        native::read(window_bytes())
            .template("air_wing_counter")
            .unwrap()
            .clone()
    }
    #[test]
    fn carrier_grid_composes_through_both_holders_and_hits_in_the_outer_coordinates() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let source = std::str::from_utf8(window_bytes()).unwrap().replacen(
            "guiTypes={",
            r#"guiTypes={
                    containerWindowType={name=naval_combat_mapicon}
                    containerWindowType={name=unit_counter_navy size={width=60 height=30}
                        gridboxtype={name=wings_box position={x=-20 y=35}
                            size={width=100% height=100%} slotsize={width=58 height=23}
                            max_slots_horizontal=1 format=UPPER_LEFT}}
                "#,
            1,
        );
        super::super::super::tests::with_counter_inputs_fixture(
            super::super::super::tests::action_gui(false),
            |assets| {
                assets.windows = SharedNativeWindows::read(source.as_bytes(), [400, 400]).unwrap();
            },
            |_| {},
            |mut inputs| {
                let windows = inputs.windows;
                inputs.settings.color_mode = CounterColorMode::Allegiance;
                let root = windows.navy();
                let item = windows.gui_type(WINDOW.as_bytes()).unwrap();
                let mut resources = fixture();
                let sprite = resources.sprites.get_mut("GFX_even").unwrap();
                sprite.frames = 4;
                sprite.texture = [80, 10];
                let mut countries = Countries::default();
                countries.sides.insert(1, Allegiance::Ours);
                let data = PreparedItem {
                    item: super::super::Item {
                        members: vec![7, 8],
                        selected: false,
                        archetype: Some(1),
                        group: 0,
                        planes: 20,
                    },
                    icon_frame: 1,
                    action: Some(2),
                    country: Some(1),
                    group_color: None,
                    markers: [Some(false); 4],
                    capacity: Some(100),
                    hover: Arc::default(),
                    marker_hovers: std::array::from_fn(|_| Arc::default()),
                    refused: Default::default(),
                };
                let mut instance = Instance::construct(root, [400, 400], &resources).unwrap();
                instance.gui_show(root, root).unwrap();
                let mut view = View::default();
                view.update(
                    0,
                    None,
                    &mut instance,
                    root,
                    "wings_box",
                    Some(&Ok(vec![data])),
                    &inputs,
                    Some(&countries),
                    &resources,
                )
                .unwrap();
                let tick = Tick { time: 1., dt: 1. };
                instance.tick(root, &resources, tick).unwrap();
                let drawn = Drawn {
                    owner: 0,
                    origin: [100, 200],
                    layout: instance.layout(root, &resources, None).unwrap().unwrap(),
                };
                let outer = native_gui::Holder {
                    path: "outer".into(),
                    child: None,
                    content: [400, 400],
                    origin: [10., 20.],
                    scale: 2.,
                    hit_scales: None,
                    logical: [10, 20],
                    clip: None,
                    opacity: 1.,
                    before: 0,
                    position: None,
                };
                let mut report = CounterReport::default();
                let grid = view
                    .compose_held(
                        0,
                        &drawn,
                        NAVY_WINDOW,
                        "wings_box",
                        &inputs,
                        &resources,
                        tick,
                        7,
                        Some((&outer, [3, 4])),
                        IconFamily::Columns,
                        &mut report,
                    )
                    .unwrap();
                let mut composed = drawn.layout.clone();
                composed.parts = drawn.layout.composed_parts(&grid.parts).cloned().collect();
                let placed = outer.place([3, 4], &composed);
                assert!(placed.iter().any(|part| part.path.ends_with("count_bg")));
                // Nothing takes a hit until its icon draws.
                assert!(report.base_hits.is_empty() && report.air_grid_hits.is_empty());
                grid.commit(&mut report);
                assert_eq!(report.air_grid_hits.len(), 1);
                let hit = &report.base_hits[0];
                let local = view.entries[&0].1[0].instance.position();
                let grid_holder = drawn
                    .layout
                    .holders
                    .iter()
                    .find(|holder| holder.path.ends_with("wings_box"))
                    .unwrap();
                let expected = [
                    100. + 10. + 2. * (3. + grid_holder.origin[0] + local[0] as f32),
                    200. + 20. + 2. * (4. + grid_holder.origin[1] + local[1] as f32),
                ];
                assert_eq!(&hit.rect[..2], &expected);
                let hover = &report.air_grid_hits[&0];
                assert_eq!(hover.family, IconFamily::Columns);
                assert_eq!(hover.members.as_deref().unwrap().as_slice(), &[7, 8]);
                assert_eq!(item.kind, native::Kind::Container);
            },
        );
    }

    /// A drawn icon's wing grid takes its hits after those already taken, each hover keyed by
    /// its own hit.
    #[test]
    fn a_committed_wing_grid_follows_the_hits_already_taken() {
        let blocker = |x| viewer_foundation::map::counter_hovers::unit_hits::Blocker {
            rect: [x, 0., 1., 1.],
            layer: 3,
        };
        let hover = AirGridHover {
            family: IconFamily::Bases,
            members: Some(Arc::new(vec![7])),
            text: Arc::default(),
        };
        let mut report = CounterReport::default();
        report.base_hits.push(blocker(0.));
        Composed {
            parts: Vec::new(),
            hits: vec![(blocker(1.), hover.clone()), (blocker(2.), hover.clone())],
        }
        .commit(&mut report);
        assert_eq!(report.base_hits, [blocker(0.), blocker(1.), blocker(2.)]);
        assert_eq!(
            report.air_grid_hits.into_iter().collect::<Vec<_>>(),
            [(1, hover.clone()), (2, hover)]
        );
    }

    #[test]
    fn air_item_draws_authored_controls_count_type_selection_flags_and_current_strength() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        super::super::super::tests::with_counter_inputs_fixture(
            super::super::super::tests::action_gui(false),
            |_| {},
            |_| {},
            |mut inputs| {
                inputs.settings.color_mode = CounterColorMode::Allegiance;
                let root = window();
                let mut resources = fixture();
                let sprite = resources.sprites.get_mut("GFX_even").unwrap();
                sprite.frames = 4;
                sprite.texture = [80, 10];
                let mut instance = Instance::construct(&root, [400, 400], &resources).unwrap();
                let mut countries = Countries::default();
                countries.sides.insert(1, Allegiance::Ours);
                let mut data = PreparedItem {
                    item: super::super::Item {
                        members: vec![7, 8],
                        selected: true,
                        archetype: Some(1),
                        group: 9,
                        planes: 1250,
                    },
                    icon_frame: 3,
                    action: Some(2),
                    country: Some(1),
                    group_color: Some([0.2, 0.3, 0.4, 0.5]),
                    markers: [Some(false), Some(false), Some(true), Some(true)],
                    capacity: Some(2000),
                    hover: Arc::default(),
                    marker_hovers: std::array::from_fn(|_| Arc::default()),
                    refused: Default::default(),
                };
                update_item(
                    &mut instance,
                    &root,
                    &data,
                    &inputs,
                    Some(&countries),
                    &resources,
                )
                .unwrap();
                let control = |kind, name| root.find(kind, name).unwrap();
                assert_eq!(
                    instance
                        .control(&root, control(native::Kind::TextBox, "count_txt"))
                        .unwrap()
                        .text
                        .as_deref(),
                    Some("1.2K")
                );
                assert_eq!(
                    instance
                        .bindings(&root, &resources)
                        .unwrap()
                        .bind(control(native::Kind::Icon, "wing_type"))
                        .frame,
                    Some(3)
                );
                assert_eq!(
                    instance
                        .bindings(&root, &resources)
                        .unwrap()
                        .bind(control(native::Kind::Icon, "onmap_action_icon"))
                        .frame,
                    Some(2)
                );
                assert_eq!(
                    instance
                        .control(&root, control(native::Kind::Icon, "bar_str"))
                        .unwrap()
                        .stretch,
                    Some(0.625)
                );
                assert_eq!(
                    instance
                        .control(&root, control(native::Kind::Icon, "group_marker"))
                        .unwrap()
                        .color,
                    Some([0.2, 0.3, 0.4, 0.5])
                );
                instance
                    .tick(&root, &resources, Tick { time: 0., dt: 0. })
                    .unwrap();
                let layout = instance.layout(&root, &resources, None).unwrap().unwrap();
                let shown = |name: &str| {
                    layout
                        .parts
                        .iter()
                        .any(|part| part.path.ends_with(&format!(".{name}")))
                };
                assert!(
                    shown("count_selected")
                        && shown("training")
                        && shown("training_will_stop_at_max_icon")
                );
                assert!(!shown("no_mission") && !shown("no_region"));
                data.capacity = None;
                data.action = None;
                data.markers = [None; 4];
                data.item.selected = false;
                update_item(
                    &mut instance,
                    &root,
                    &data,
                    &inputs,
                    Some(&countries),
                    &resources,
                )
                .unwrap();
                instance
                    .tick(&root, &resources, Tick { time: 1., dt: 1. })
                    .unwrap();
                let layout = instance.layout(&root, &resources, None).unwrap().unwrap();
                assert!(
                    layout
                        .parts
                        .iter()
                        .any(|part| part.path.ends_with(".count_txt"))
                );
                for name in ["onmap_action_icon", "bar_str", "training", "count_selected"] {
                    assert!(
                        !layout
                            .parts
                            .iter()
                            .any(|part| part.path.ends_with(&format!(".{name}")))
                    );
                }
            },
        );
    }
}
