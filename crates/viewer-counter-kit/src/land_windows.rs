use crate::gui_windows as gui_windows;
use std::collections::HashMap;

use content_reader_core::gui::native;
use crate::combat_markers::LandMarker;
use std::collections::BTreeMap;
use super::{CounterInputs, CounterReport, Countries};
use viewer_foundation::mode_visibility::PriorityIcon;
use viewer_gpu::icon_tracks::IconAnchor;
use viewer_native_gui::native_windows::LAND_COMBAT_WINDOW;
use viewer_native_gui::sprites::{Batch, layer};
use viewer_engagements::combat_selection::Hit;
use viewer_engagements::combat_selection::Selection;
use viewer_native_gui::gui_resources::count_refused;
use viewer_native_gui::gui_resources::hit_rectangle;
use viewer_native_gui::gui_resources::viewport as gui_resources_viewport;
use viewer_native_gui::icon_windows::Retry;
#[cfg(test)]
use viewer_native_gui::icon_windows::visible_layout;
use viewer_native_gui::{
    icon_windows::{Owner, Plan, PoolView},
    native_gui::{self, SpriteSetter, lifecycle::{Instance, MapIconPool, SlotId}},
    native_resources::Context,
};

#[derive(Default)]
pub struct PreparedCombats {
    pub placed: Vec<Placed>,
}

pub struct Placed {
    pub marker: LandMarker,
    pub screen: [f32; 2],
    pub depth: f32,
    pub angle: f32,
    pub terrain: Option<String>,
    pub text: String,
    pub anchor: Option<IconAnchor>,
}

#[derive(Debug, Default)]
pub struct View {
    pool: PoolView<u64>,
    terrain: BTreeMap<SlotId, Option<String>>,
    painted: viewer_native_gui::icon_windows::Painted<u64, Option<IconAnchor>>,
}

impl View {
    pub fn retry(&self) -> Option<Retry> {
        self.pool.retry()
    }

    pub fn set_quiet(&mut self, quiet: bool) {
        self.pool.set_quiet(quiet);
    }

    pub fn running_visibly(&self) -> bool {
        self.pool.running_visibly()
    }

    pub fn take_running(&mut self) -> bool {
        self.pool.take_running()
    }
}

struct Controls<'a> {
    background: &'a native::Element,
    direction: &'a native::Element,
    air: &'a native::Element,
    frame: &'a native::Element,
    bombardment: &'a native::Element,
    progress: &'a native::Element,
}

fn controls(root: &native::Element) -> Result<Controls<'_>, String> {
    let find = |kind, name: &str| {
        root.find(kind, name)
            .ok_or_else(|| format!("no control named {name}"))
    };
    Ok(Controls {
        background: find(native::Kind::Button, "bg_btn")?,
        direction: find(native::Kind::Icon, "battle_indicator_direction")?,
        air: find(native::Kind::Icon, "air_superiority_icon")?,
        frame: find(native::Kind::Icon, "mapicon_land_combat_frame")?,
        bombardment: find(native::Kind::Icon, "mapicon_land_combat_bombardment")?,
        progress: find(native::Kind::TextBox, "combat_progress")?,
    })
}

fn update(
    instance: &mut Instance,
    root: &native::Element,
    placed: &Placed,
    previous: std::collections::btree_map::Entry<'_, impl Ord, Option<String>>,
    resources: &dyn native_gui::Resources,
) -> Result<(), String> {
    let c = controls(root)?;
    let marker = &placed.marker;
    use std::collections::btree_map::Entry;
    let changed = match &previous {
        Entry::Vacant(_) => true,
        Entry::Occupied(entry) => entry.get() != &placed.terrain,
    };
    if changed {
        let native::Fields::Button(button) = &c.background.fields else {
            return Err("bg_btn is not a standard button".into());
        };
        let mut variant = button.sprite.clone();
        if let Some(terrain) = &placed.terrain {
            variant.push(b'_');
            variant.extend_from_slice(terrain.as_bytes());
        }
        let sprite = if resources.sprite(&variant).is_some() {
            &variant
        } else {
            &button.sprite
        };
        instance.set_sprite(root, c.background, sprite, SpriteSetter::ButtonStandard)?;
        previous.insert_entry(placed.terrain.clone());
    }
    instance.hide(root, c.air)?;
    instance.show(root, c.progress)?;
    instance.set_text(root, c.progress, &placed.text, resources)?;
    let frame = match marker.frame {
        Some(frame) => frame.wrapping_add(1) as i32,
        None => {
            let native::Fields::Icon(icon) = &c.frame.fields else {
                return Err("mapicon_land_combat_frame is not a standard icon".into());
            };
            icon.frame
        }
    };
    instance.show(root, c.frame)?;
    instance.set_frame(root, c.frame, frame)?;
    if let Some(tint) = marker.tint {
        instance.show(root, c.background)?;
        instance.control(root, c.background)?.color = Some(tint);
        if marker.seen {
            instance.control(root, c.background)?.enabled = Some(true);
            instance.show(root, c.direction)?;
            let direction = instance.control(root, c.direction)?;
            direction.color = Some(tint);
            direction.rotation = Some(placed.angle);
        } else {
            instance.hide(root, c.direction)?;
        }
    } else {
        instance.hide(root, c.background)?;
        instance.hide(root, c.direction)?;
    }
    if marker.bombardment {
        instance.show(root, c.bombardment)?;
    } else {
        instance.hide(root, c.bombardment)?;
    }
    Ok(())
}

impl PreparedCombats {
    pub fn draw(
        &self,
        inputs: &CounterInputs<'_>,
        view: &mut View,
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
        if !view.pool.same(&context) {
            view.terrain.clear();
        }
        let schedule = inputs.schedule.lock().unwrap_or_else(|p| p.into_inner());
        let placed: HashMap<_, _> = self.placed.iter().map(|p| (p.marker.id, p)).collect();
        let owners: Vec<_> = self
            .placed
            .iter()
            .map(|p| Owner {
                id: p.marker.id,
                screen: p.screen,
                dirty: schedule.dirty(p.marker.province),
            })
            .collect();
        view.pool.set_cleaning(self.placed.iter().any(|p| schedule.cleans(p.marker.province)));
        view.pool.present_recorded(
            &context,
            &Plan {
                name: LAND_COMBAT_WINDOW,
                kind: MapIconPool::LandCombat,
                alignment: 0x12,
                owners: &owners,
            },
            Box::new(|_, root| controls(root).map(|_| ())),
            |owner, slot, instance, root| {
                update(
                    instance,
                    root,
                    placed[&owner],
                    view.terrain.entry(slot),
                    &resources,
                )
            },
            |owner| {
                let placed = placed[owner];
                let marker = &placed.marker;
                (
                    placed.terrain.clone(),
                    placed.text.clone(),
                    placed.angle.to_bits(),
                    marker.frame,
                    marker.tint.map(|tint| tint.map(f32::to_bits)),
                    marker.seen,
                    marker.bombardment,
                )
            },
            false,
            &mut report.icon_gui_notes,
        );
        if view.pool.quiet() {
            return;
        }
        let mut drawn: Vec<_> = view.pool.drawn_generations().collect();
        drawn.sort_by(|(a, _), (b, _)| placed[&b.owner].depth.total_cmp(&placed[&a.owner].depth));
        let root = inputs.windows.gui_type(LAND_COMBAT_WINDOW.as_bytes());
        let painted = &mut view.painted;
        for (drawn, generation) in drawn {
            let marker = &placed[&drawn.owner].marker;
            let key = layer(
                inputs.priority(PriorityIcon::LandCombat),
                report.combat_ordinal(),
                0,
            );
            batch.at_layer(key);
            let anchor = placed[&drawn.owner].anchor;
            batch.at_anchor(anchor);
            let draw = |batch: &mut Batch| native_gui::draw(&drawn.layout, drawn.origin, &resources, batch);
            match painted.paint(
                viewer_native_gui::sprites::WindowKey::new(
                    viewer_native_gui::sprites::WindowPool::LandCombat,
                    0,
                    drawn.owner,
                ),
                drawn.owner, generation, drawn.origin, key, anchor, batch, draw) {
                Ok(()) => {
                    if let Some(root) = root
                        && let Some(button) = root.find(native::Kind::Button, "bg_btn")
                        && let Some(hit) = hit_rectangle(
                            button,
                            &drawn.layout,
                            drawn.origin,
                            &resources,
                            &mut report.icon_gui_notes,
                        )
                    {
                        report.combat_hits.push(Hit {
                            selection: Selection::Land(marker.id),
                            rect: hit.rect,
                            layer: key,
                            order: hit.order,
                            supported: hit.supported,
                        });
                    }
                    report.land_combats += 1;
                    report.no_intel_combats += usize::from(!marker.seen);
                }
                Err(why) => count_refused(&mut report.icon_gui_notes, LAND_COMBAT_WINDOW, &[why]),
            }
        }
        batch.at_anchor(None);
        painted.finish();
    }
}

#[cfg(test)]
mod tests {
    use viewer_native_gui::native_gui;
    use super::{BTreeMap, Instance, LAND_COMBAT_WINDOW, LandMarker, MapIconPool, Placed, controls, hit_rectangle, native, update, visible_layout};
    use native_gui::{PartKind, Resources};
    use viewer_native_gui::test_support::gui::fixture;

    fn window(font: &str) -> native::Element {
        native::read(r#"guiTypes = { containerWindowType = { name = test
            size = { width = 80 height = 40 }
                buttonType = { name = bg_btn quadTextureSprite = GFX_even }
                iconType = { name = battle_indicator_direction spriteType = GFX_even }
                iconType = { name = air_superiority_icon spriteType = GFX_even }
                iconType = { name = mapicon_land_combat_frame spriteType = GFX_even }
                iconType = { name = mapicon_land_combat_bombardment spriteType = GFX_even }
                instantTextBoxType = { name = combat_progress font = Main_14 maxWidth = 40 maxHeight = 20 }
            containerWindowType = { name = nested
                iconType = { name = decoration spriteType = GFX_even }
            }
        } }"#.replace("Main_14", font).as_bytes()).template("test").unwrap().clone()
    }

    fn placed() -> Placed {
        Placed {
            anchor: None,
            marker: LandMarker {
                id: 1,
                province: 1,
                attacker: None,
                seen: true,
                progress: Some(0),
                tint: Some([0.5; 4]),
                frame: Some(1),
                bombardment: false,
            },
            screen: [0.; 2],
            depth: 0.,
            angle: 0.25,
            terrain: Some("mud".into()),
            text: "x".into(),
        }
    }

    #[test]
    fn combat_binder_does_not_bind_nested_controls() {
        content_reader_core::test_game::enter();
        let mut root = window("Main_14");
        let mut background = root.children.remove(0);
        background.path = "test.nested.bg_btn".into();
        root.children.last_mut().unwrap().children.push(background);
        assert!(controls(&root).err().unwrap().contains("bg_btn"));
        let resources = fixture();
        let mut instance = Instance::construct(&root, [800, 600], &resources).unwrap();
        instance.gui_show(&root, &root).unwrap();
        let layout = instance.layout(&root, &resources, None).unwrap().unwrap();
        assert!(crate::gui_windows::background_footprint(&root, &layout).is_none());
    }

    #[test]
    fn unseen_frame_draws_authored_initial_or_last_seen_frame_with_native_children() {
        content_reader_core::test_game::enter();
        let mut resources = fixture();
        resources.add_sprite("GFX_synthetic_frame", [20, 10], 2);
        for authored in [1, 2] {
            let mut root = window("Main_14");
            let child = root
                .children
                .iter_mut()
                .find(|child| child.name_bytes() == b"mapicon_land_combat_frame")
                .unwrap();
            let native::Fields::Icon(icon) = &mut child.fields else {
                panic!("frame icon");
            };
            icon.sprite = b"GFX_synthetic_frame".to_vec();
            icon.frame = authored;
            let c = controls(&root).unwrap();
            let mut instance = Instance::construct(&root, [800, 600], &resources).unwrap();
            instance.gui_show(&root, &root).unwrap();
            let mut placed = placed();
            placed.marker.seen = false;
            placed.marker.progress = None;
            placed.marker.tint = Some([1.0; 4]);
            placed.text = "?".into();
            let mut previous = BTreeMap::new();
            for history in [None, Some(1), None, Some(0)] {
                placed.marker.frame = history;
                update(
                    &mut instance,
                    &root,
                    &placed,
                    previous.entry(0u32),
                    &resources,
                )
                .unwrap();
                let layout = instance.layout(&root, &resources, None).unwrap().unwrap();
                let frame = layout
                    .parts
                    .iter()
                    .find(|part| part.path == c.frame.path)
                    .expect("unseen frame child is drawn");
                let PartKind::Sprite { uv, offsets, .. } = &frame.kind else {
                    panic!("frame sprite");
                };
                assert_eq!(
                    offsets[0] / uv[2],
                    history.map_or((authored - 1) as f32, |v| v as f32)
                );
                assert_eq!(
                    instance.control(&root, c.background).unwrap().color,
                    Some([1.0; 4])
                );
                assert!(
                    layout
                        .parts
                        .iter()
                        .any(|part| part.path == c.background.path)
                );
                assert!(layout.parts.iter().any(|part| part.path == c.progress.path));
                assert!(!layout.parts.iter().any(|part| {
                    part.path == c.direction.path
                        || part.path == c.air.path
                        || part.path == c.bombardment.path
                }));
            }
        }
    }

    #[test]
    fn a_withheld_score_background_does_not_invent_a_window_hit() {
        content_reader_core::test_game::enter();
        let root = window("Main_14");
        let resources = fixture();
        let mut placed = placed();
        placed.marker.tint = None;
        let mut instance = Instance::construct(&root, [800, 600], &resources).unwrap();
        instance.gui_show(&root, &root).unwrap();
        update(
            &mut instance,
            &root,
            &placed,
            BTreeMap::new().entry(0u32),
            &resources,
        )
        .unwrap();
        let layout = instance.layout(&root, &resources, None).unwrap().unwrap();
        assert!(crate::gui_windows::background_footprint(&root, &layout).is_none());
        assert!(
            hit_rectangle(
                controls(&root).unwrap().background,
                &layout,
                [20, 30],
                &resources,
                &mut BTreeMap::new()
            )
            .is_none()
        );
    }

    #[test]
    fn unsupported_progress_font_is_refused_by_name_and_counted() {
        content_reader_core::test_game::enter();
        use native_gui::lifecycle::{Admission, Pool};
        let root = window("Unsupported_14");
        let resources = fixture();
        let mut pool = Pool::new(
            &root,
            [800, 600],
            MapIconPool::LandCombat,
            Box::new(|_, root| controls(root).map(|_| ())),
            &resources,
        )
        .unwrap();
        let slot = pool.request(1u64).unwrap();
        pool.update(&root, |_, _| Admission::Inside([20, 30]))
            .unwrap();
        update(
            pool.get(slot).unwrap(),
            &root,
            &placed(),
            BTreeMap::new().entry(0u32),
            &resources,
        )
        .unwrap();
        let mut notes = BTreeMap::new();
        let icon = pool.visible().next().unwrap();
        assert!(visible_layout(&icon, &root, LAND_COMBAT_WINDOW, &resources, &mut notes).is_none());
        assert_eq!(notes.values().sum::<usize>(), 1);
        let reason = notes.keys().next().unwrap();
        assert!(
            reason.contains("combat_progress") && reason.contains("Unsupported_14"),
            "{reason}"
        );
    }

    #[test]
    fn local_controls_use_authored_terrain_sprite_and_preserve_unchanged_drawable() {
        content_reader_core::test_game::enter();
        let root = window("Main_14");
        let mut resources = fixture();
        resources
            .sprites
            .insert("GFX_even_mud", resources.sprite(b"GFX_odd").unwrap());
        let placed = placed();
        let mut instance = Instance::construct(&root, [800, 600], &resources).unwrap();
        instance.gui_show(&root, &root).unwrap();
        let mut previous = BTreeMap::new();
        update(
            &mut instance,
            &root,
            &placed,
            previous.entry(0u32),
            &resources,
        )
        .unwrap();
        let c = controls(&root).unwrap();
        let layout = instance.layout(&root, &resources, None).unwrap().unwrap();
        assert!(layout.parts.iter().any(|part| part.path == c.background.path
            && matches!(&part.kind, PartKind::Sprite { sprite, .. } if sprite == b"GFX_even_mud")));
        assert!(
            !layout
                .parts
                .iter()
                .any(|part| part.path == c.air.path || part.path == c.bombardment.path)
        );
        assert!(
            layout
                .parts
                .iter()
                .any(|part| part.path.ends_with(".decoration"))
        );
        instance.control(&root, c.background).unwrap().rotation = Some(0.7);
        update(
            &mut instance,
            &root,
            &placed,
            previous.entry(0u32),
            &resources,
        )
        .unwrap();
        assert_eq!(
            instance.control(&root, c.background).unwrap().rotation,
            Some(0.7)
        );
        assert!(crate::gui_windows::background_footprint(&root, &layout).is_some());
    }
}
