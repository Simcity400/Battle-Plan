//! Air-base, headquarters and facility windows, on the native map-icon pools.

use super::*;
use crate::map::{
    icon_windows::{Owner, Plan},
    native_gui::{
        self, SpriteSetter,
        lifecycle::{Instance, MapIconPool},
    },
    native_resources::Context,
};

#[derive(Default)]
pub(super) struct PreparedBases {
    placed: [Vec<Placed>; 3],
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Family {
    AirBase,
    Headquarter,
    Facility,
}

struct Placed {
    id: u32,
    province: u32,
    screen: [f32; 2],
    depth: f32,
    state: State,
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
        let projected = inputs.camera.project_map_icon(
            anchor,
            inputs.viewport,
            gui_windows::viewport(inputs.viewport, inputs.ui_scale),
        );
        self.placed[family as usize].push(Placed {
            id,
            province,
            screen: projected.position,
            depth: projected.depth,
            state,
        });
    }

    pub(super) fn draw(
        &self,
        inputs: &CounterInputs<'_>,
        time: f32,
        countries: Option<&Countries>,
        batch: &mut Batch,
        report: &mut CounterReport,
    ) {
        let resources = gui_windows::GuiAssets::with_countries(inputs, countries);
        let context = Context {
            resources: &resources,
            windows: inputs.windows,
            viewport: gui_windows::viewport(inputs.viewport, inputs.ui_scale),
            time: Some(time),
        };
        let mut views = inputs
            .presentation
            .windows
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let schedule = inputs
            .presentation
            .schedule()
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let families = [
            (
                Family::AirBase,
                AIR_BASE_WINDOW,
                MapIconPool::AirBase,
                inputs.content.defines.air_base_priority,
            ),
            (
                Family::Headquarter,
                NAVAL_HEADQUARTER_WINDOW,
                MapIconPool::Headquarter,
                inputs.content.defines.naval_headquarter_priority,
            ),
            (
                Family::Facility,
                FACILITY_WINDOW,
                MapIconPool::Facility,
                inputs.content.defines.facility_priority,
            ),
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
            let placed: std::collections::HashMap<_, _> = self.placed[family as usize]
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
            let drawn = views.bases[family as usize].present(
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
        // CMapIconManagerImpl::update@140B2F270 sorts map icons by projected depth.
        layouts.sort_by(|a, b| b.3.depth.total_cmp(&a.3.depth));
        for (family, name, priority, _, drawn) in layouts {
            batch.at_layer(layer(priority, report.base_ordinal(), 0));
            match native_gui::draw(&drawn.layout, drawn.origin, &resources, batch) {
                Ok(()) => match family {
                    Family::AirBase => report.air_bases += 1,
                    Family::Headquarter => report.naval_headquarters += 1,
                    Family::Facility => report.facilities += 1,
                },
                Err(why) => gui_windows::count_refused(&mut report.icon_gui_notes, name, &[why]),
            }
        }
    }
}

struct Controls<'a> {
    button: &'a native::Element,
    air: Option<[&'a native::Element; 4]>,
}

fn bind(root: &native::Element, family: Family) -> Result<Controls<'_>, String> {
    // Air@1418B7F90 and HQ/facility@1418BA830 use recursive button lookups.
    let button = root
        .find_recursive(native::Kind::Button, "bg_btn")
        .ok_or("no button named bg_btn")?;
    let air = if family == Family::AirBase {
        let find = |kind, name: &str| {
            root.find_recursive(kind, name)
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
    if let Some([strategic, wings, permission_count, permission_bg]) = air {
        // Political branch of UpdateAirBaseMapIcon@1418B1C50.
        instance.hide(root, strategic)?;
        instance.set_frame(root, strategic, state.frame)?;
        instance.control(root, strategic)?.enabled = Some(state.enabled);
        for control in [wings, permission_count, permission_bg] {
            instance.hide(root, control)?;
        }
    }
    // Air@1418B1C50 and HQ@1418D70B0 write frames; facility@1418BC070 keeps it.
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
    use native_gui::{PartKind, lifecycle::Tick, tests::fixture};

    fn window(children: &str) -> native::Element {
        native::read(format!("guiTypes = {{ containerWindowType = {{ name = test size = {{ width = 50 height = 40 }} {children} }} }}").as_bytes())
            .template("test").unwrap().clone()
    }

    #[test]
    fn air_update_uses_nested_buttons_and_hides_the_air_mode_controls() {
        let root = window(
            r#"
            containerWindowType = { name = nested position = { x = 5 y = 6 }
                buttonType = { name = bg_btn quadTextureSprite = GFX_even }
                buttonType = { name = bg_strat_btn quadTextureSprite = GFX_even }
                instantTextBoxType = { name = wings_count font = Main_14 text = authored }
                instantTextBoxType = { name = permission_wings_count font = Main_14 text = authored }
                iconType = { name = permission_wings_bg spriteType = GFX_even }
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

    #[test]
    fn facility_setter_rebuilds_the_drawable_and_keeps_an_undefined_sprite() {
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
        let button = root.find_recursive(native::Kind::Button, "bg_btn").unwrap();
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
