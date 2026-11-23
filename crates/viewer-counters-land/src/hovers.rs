
use content_reader_gui::gui::native;
use crate::land;
use viewer_counter_kit::{ABILITY_WINDOW, CounterInputs, GUN_WINDOW, LAND_WINDOW, bar_placement};
use viewer_native_gui::sprites;
use viewer_native_gui::counter_hovers::land::CounterControl;
use viewer_native_gui::sprites::{Batch, SpriteSheet};
use viewer_unit_reads::counter_status;
#[cfg(test)]
use content_reader_layers::ResolvedLayers;
use viewer_native_gui::counter_hovers::gui_authored::Authored;
#[cfg(test)]
use viewer_runtime::geometry::contains;
use viewer_native_gui::native_gui::Hit;
use viewer_native_gui::native_gui::Sheets;
use viewer_native_gui::native_gui::container_frame;
#[cfg(test)]
use viewer_runtime::test_support::test_tga_rgba8;

#[derive(Default)]
pub struct Sprite {
    pub origin: [f32; 2],
    pub stood: [i32; 2],
    pub priority: i8,
    pub painting: u64,
    pub batch: Batch,
    pub presented: Presented,
}

#[derive(Default)]
pub struct Presented {
    pub controls: Vec<(CounterControl, [f32; 4], Option<Authored>)>,
    pub refused: Vec<String>,
}

pub fn presented(
    inputs: &CounterInputs<'_>,
    stack: &land::Stack,
    coarse: bool,
    origin: [f32; 2],
    quads: &[sprites::Quad],
) -> Presented {
    let _observe_profile = viewer_runtime::profiling::span("dc_observe");
    let railway_gun = stack.gun.is_some();
    let window = if railway_gun { GUN_WINDOW } else { LAND_WINDOW };
    let Some(root) = inputs.windows.guis.template(window) else {
        return Presented::default();
    };
    let mut bounds = Vec::<(u64, [f32; 4])>::new();
    for quad in quads {
        let rect = quad.screen_bounds();
        if rect[2] <= 0.0 || rect[3] <= 0.0 {
            continue;
        }
        if let Some((_, held)) = bounds.iter_mut().find(|(order, _)| *order == quad.layer) {
            let right = (held[0] + held[2]).max(rect[0] + rect[2]);
            let bottom = (held[1] + held[3]).max(rect[1] + rect[3]);
            held[0] = held[0].min(rect[0]);
            held[1] = held[1].min(rect[1]);
            held[2] = right - held[0];
            held[3] = bottom - held[1];
        } else {
            bounds.push((quad.layer, rect));
        }
    }
    bounds.sort_unstable_by_key(|(order, _)| *order);
    let mut out = Presented {
        controls: Vec::with_capacity(bounds.len()),
        refused: Vec::new(),
    };
    for (order, mut rect) in bounds {
        let Some(element) = root.children.get(order as usize) else {
            continue;
        };
        if element.hide
            || element.icon().is_some_and(|icon| icon.always_transparent)
            || element
                .button()
                .is_some_and(|button| button.always_transparent)
            || element
                .text_box()
                .is_some_and(|text| text.always_transparent)
        {
            continue;
        }
        if element.is_named("active_abilities") {
            ability_controls(inputs, stack, origin, root, quads, order, &mut out);
            continue;
        }
        let control = control(element, railway_gun);
        if let Some(text) = element.text_box() {
            let layout = if railway_gun {
                inputs
                    .assets
                    .layouts
                    .gun
                    .as_ref()
                    .ok()
                    .map(|layout| &layout.count)
            } else {
                inputs
                    .assets
                    .layouts
                    .land
                    .as_ref()
                    .ok()
                    .map(|layout| &layout.count)
            };
            let Some(layout) = layout else {
                continue;
            };
            rect = [
                (origin[0] + layout.at[0]).trunc() + 1.0,
                (origin[1] + layout.at[1]).trunc() + 1.0,
                layout.max_width - 1.0,
                if text.max_height == 0 {
                    199.0
                } else {
                    text.max_height as i32 as f32 - 1.0
                },
            ];
        } else {
            let Some(quad) = quads.iter().find(|quad| quad.layer == order) else {
                continue;
            };
            let frame = match selected_frame(inputs, stack, coarse, element, quad) {
                Ok(frame) => frame,
                Err(reason) => {
                    out.refused
                        .push(format!("{window}.{} {reason}", element.name()));
                    continue;
                }
            };
            if !accept_hit(frame.hit, window, &element.name(), &mut out.refused) {
                continue;
            }
            let scale = element
                .icon()
                .map(|icon| icon.scale)
                .or_else(|| element.button().map(|button| button.scale))
                .unwrap_or(1.0);
            if let Some(icon) = element.icon() {
                if element.is_named("bar_org") || element.is_named("bar_str") {
                    match container_frame(root, None) {
                        Ok(container) => {
                            let anchor = viewer_native_gui::native_gui::control_anchor(
                                icon.orientation,
                                container.content_size,
                            );
                            let base = [
                                origin[0]
                                    + container.content_at[0] as f32
                                    + icon.position[0]
                                    + anchor[0] as f32,
                                origin[1]
                                    + container.content_at[1] as f32
                                    + icon.position[1]
                                    + anchor[1] as f32,
                            ];
                            let fill = if element.is_named("bar_org") {
                                counter_status::stack_fill(&stack.organisation)
                            } else {
                                counter_status::stack_fill(&stack.hit_points)
                            };
                            let unscaled = [
                                frame.texture[0] / frame.grid[0] as f32,
                                frame.texture[1] / frame.grid[1] as f32,
                            ];
                            let ([x, y], _, _) = bar_placement(base, unscaled, scale, fill);
                            rect[0] = x;
                            rect[1] = y;
                        }
                        Err(reason) => {
                            out.refused.push(format!(
                                "{window}.{} hover placement unavailable: {reason}",
                                element.name()
                            ));
                            continue;
                        }
                    }
                } else if icon.rotation != 0.0 {
                    out.refused.push(format!(
                        "{window}.{} rotated hover placement unavailable",
                        element.name()
                    ));
                    continue;
                }
            }
            [rect[2], rect[3]] = frame.scaled_size(scale);
        }
        if let Err(reason) = clip(root, origin, &mut rect) {
            out.refused.push(format!(
                "{window}.{} hover clip unavailable: {reason}",
                element.name()
            ));
            continue;
        }
        if rect[2] > 0.0 && rect[3] > 0.0 {
            if element.button().is_some_and(|button| !button.enabled) {
                out.refused.push(format!(
                    "{window}.{} disabled counter batch control is not admitted",
                    element.name()
                ));
                continue;
            }
            out.controls
                .push((control, rect, Authored::read(element, false)));
        }
    }
    out
}

struct Frame {
    hit: Hit,
    texture: [f32; 2],
    grid: [i32; 2],
}

impl Frame {
    fn scaled_size(&self, scale: f32) -> [f32; 2] {
        [0, 1].map(|axis| self.texture[axis] * scale * (1.0 / self.grid[axis] as f32))
    }
}

fn sprite_hit(inputs: &CounterInputs<'_>, name: &[u8]) -> Result<Hit, &'static str> {
    use viewer_native_gui::native_gui::Hit;
    inputs
        .assets
        .gui
        .sprite(name)
        .map(|info| info.hit)
        .or_else(|| {
            inputs
                .assets
                .gui
                .registry()
                .sprite(name)
                .and_then(|facts| facts.kind.plain())
                .map(|plain| {
                    if plain.always_transparent {
                        Hit::Through
                    } else if plain.transparence_check {
                        Hit::Alpha
                    } else {
                        Hit::Frame
                    }
                })
        })
        .ok_or("hover sprite metadata unavailable")
}

fn sheet_frame(inputs: &CounterInputs<'_>, sheet: &SpriteSheet) -> Result<Frame, &'static str> {
    Ok(Frame {
        hit: sprite_hit(inputs, sheet.name.as_bytes())?,
        texture: sheet.size.map(|value| value as f32),
        grid: [sheet.frames.max(1) as i32, 1],
    })
}

fn selected_frame(
    inputs: &CounterInputs<'_>,
    stack: &land::Stack,
    coarse: bool,
    element: &native::Element,
    quad: &sprites::Quad,
) -> Result<Frame, &'static str> {
    if element.is_named("type") {
        let name = inputs
            .assets
            .types
            .entries()
            .find_map(|((name, _), cell)| {
                (inputs.assets.types.texture == quad.texture
                    && inputs.assets.types.uv(*cell) == quad.uv)
                    .then_some(name)
            })
            .ok_or("hover type sprite identity unavailable")?;
        let (texture, grid) = inputs
            .assets
            .types
            .source_dimensions(name)
            .ok_or("hover type source dimensions unavailable")?;
        return Ok(Frame {
            hit: sprite_hit(inputs, name.as_bytes())?,
            texture: texture.map(|value| value as f32),
            grid,
        });
    }
    if element.is_named("general_portrait") {
        let sheet = stack
            .portrait
            .as_ref()
            .and_then(|portrait| inputs.assets.portrait(portrait))
            .ok_or("hover portrait sprite unavailable")?;
        return sheet_frame(inputs, &sheet);
    }
    let authored = element
        .icon()
        .map(|icon| icon.sprite.as_slice())
        .or_else(|| element.button().map(|button| button.sprite.as_slice()))
        .ok_or("unsupported child hover kind")?;
    let sheet = if element.is_named("count_bg") && stack.hq && !coarse && stack.gun.is_none() {
        inputs.assets.sheet("GFX_onmap_unit_counter_empty")
    } else if element.is_named("onmap_action_icon") {
        let action = if stack.gun.is_some() {
            &inputs.assets.gun_action
        } else {
            &inputs.assets.land_action
        };
        action
            .as_ref()
            .and_then(|action| action.sheet.as_ref().ok())
    } else {
        std::str::from_utf8(authored)
            .ok()
            .and_then(|name| inputs.assets.sheet(name))
            .or_else(|| Sheets::sheet(&inputs.assets.gui, authored))
    }
    .ok_or("hover sprite drawable unavailable")?;
    if sheet.texture != quad.texture {
        return Err("hover sprite binding differs from drawable");
    }
    sheet_frame(inputs, sheet)
}

fn accept_hit(hit: Hit, window: &str, name: &str, refused: &mut Vec<String>) -> bool {
    use viewer_native_gui::native_gui::Hit;
    match hit {
        Hit::Through => false,
        Hit::Frame => true,
        Hit::Alpha => {
            refused.push(format!("{window}.{name} hover needs native texture alpha"));
            false
        }
    }
}

fn clip(root: &native::Element, origin: [f32; 2], rect: &mut [f32; 4]) -> Result<(), String> {
    if let Some(container) = root.container()
        && container
            .clipping
            .unwrap_or(!container.backgrounds.is_empty())
    {
        let frame = container_frame(root, None)?;
        let left = origin[0] + frame.content_at[0] as f32;
        let top = origin[1] + frame.content_at[1] as f32;
        let right = (rect[0] + rect[2]).min(left + frame.content_size[0] as f32);
        let bottom = (rect[1] + rect[3]).min(top + frame.content_size[1] as f32);
        rect[0] = rect[0].max(left);
        rect[1] = rect[1].max(top);
        rect[2] = right - rect[0];
        rect[3] = bottom - rect[1];
    }
    Ok(())
}

fn ability_controls(
    inputs: &CounterInputs<'_>,
    stack: &land::Stack,
    origin: [f32; 2],
    root: &native::Element,
    quads: &[sprites::Quad],
    order: u64,
    out: &mut Presented,
) {
    let Some(item) = inputs.windows.guis.template(ABILITY_WINDOW) else {
        return;
    };
    let Some(element) = item.find(native::Kind::Icon, "active_ability") else {
        return;
    };
    let Some(icon) = element.icon() else {
        return;
    };
    if element.hide || icon.always_transparent {
        return;
    }
    let (Some(abilities), Some(grid)) = (&stack.abilities, &inputs.assets.layouts.abilities) else {
        return;
    };
    let item_frame = container_frame(item, None);
    if icon.rotation != 0.0
        || item.children.len() != 1
        || item_frame
            .as_ref()
            .map_or(true, |frame| frame.content_at != [0, 0])
    {
        out.refused.push(format!(
            "{LAND_WINDOW}.active_abilities unsupported item hover layout"
        ));
        return;
    }
    for (position, ability) in abilities.iter().enumerate() {
        let Some(sheet) = inputs.assets.ability_sheet(&ability.token) else {
            continue;
        };
        let mut rect = land::ability_rect(sheet, grid, origin, position);
        if !quads
            .iter()
            .any(|quad| quad.layer == order && quad.texture == sheet.texture && quad.rect == rect)
        {
            continue;
        }
        let frame = match sheet_frame(inputs, sheet) {
            Ok(frame) => frame,
            Err(reason) => {
                out.refused
                    .push(format!("{ABILITY_WINDOW}.active_ability {reason}"));
                continue;
            }
        };
        if !accept_hit(
            frame.hit,
            ABILITY_WINDOW,
            "active_ability",
            &mut out.refused,
        ) {
            continue;
        }
        [rect[2], rect[3]] = frame.scaled_size(grid.scale);
        let item_origin = [
            origin[0] + grid.at[0] + position as f32 * grid.slot[0],
            origin[1] + grid.at[1],
        ];
        if let Err(reason) =
            clip(item, item_origin, &mut rect).and_then(|()| clip(root, origin, &mut rect))
        {
            out.refused.push(format!(
                "{ABILITY_WINDOW}.active_ability hover clip unavailable: {reason}"
            ));
            continue;
        }
        if rect[2] > 0.0 && rect[3] > 0.0 {
            out.controls.push((
                CounterControl::Ability {
                    country: ability.country,
                    ordinal: ability.ordinal,
                },
                rect,
                Authored::read(element, false),
            ));
        }
    }
}

fn control(element: &native::Element, railway_gun: bool) -> CounterControl {
    for (name, control) in [
        ("no_orders", CounterControl::NoOrders),
        ("low_supply", CounterControl::Supply),
        ("onmap_action_icon", CounterControl::Action),
    ] {
        if element.is_named(name) {
            return control;
        }
    }
    if !railway_gun {
        for (name, control) in [
            ("invalid_orders", CounterControl::InvalidOrders),
            ("attrition", CounterControl::Attrition),
            ("defending_area", CounterControl::DefendingArea),
            ("onmap_digin_icon", CounterControl::DigIn),
            ("leader_enroute", CounterControl::LeaderEnroute),
        ] {
            if element.is_named(name) {
                return control;
            }
        }
    }
    CounterControl::Normal
}

#[cfg(test)]
mod tests {
    use content_reader_gfx::gfx::SpriteSet;
    use std::sync::Arc;
    use super::{
        Frame, Hit, ResolvedLayers, contains, land, native, presented, sprites, test_tga_rgba8,
    };
    use viewer_counter_kit::{Child, draw_bar};
    use viewer_foundation::templates;

    #[test]
    fn vertical_counter_hover_uses_the_native_grid_and_scale_order() {
        content_reader_script::test_game::enter();
        let root = std::env::temp_dir().join(format!(
            "hoi4_vertical_counter_hover_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(root.join("gfx")).unwrap();
        std::fs::write(root.join("gfx/type.tga"), test_tga_rgba8(12, 60)).unwrap();
        let layers = ResolvedLayers::from_folders(&[(root.as_path(), &[][..])]).unwrap();
        let sprites = SpriteSet::new(&[("interface/test.gfx".into(),
            native::sprites::read_sprites(b"spriteTypes={spriteType={name=GFX_div_templ_7_small texturefile=gfx/type.tga noOfFrames=2 framedirectiony=yes}}"))]);
        let (atlas, warnings) =
            templates::TypeAtlas::load(&layers, &sprites, Default::default(), &mut Vec::new())
                .unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let icon = templates::TypeIcon {
            sprite: "GFX_div_templ_7_small".into(),
            frame: 1,
        };
        assert!(atlas.cell(&icon).is_some());
        let (texture, grid) = atlas.source_dimensions(&icon.sprite).unwrap();
        let frame = Frame {
            hit: Hit::Frame,
            texture: texture.map(|edge| edge as f32),
            grid,
        };
        assert_eq!(frame.scaled_size(1.0), [12., 30.]);
        assert_eq!(frame.scaled_size(0.75), [9., 22.5]);
        let [w, h] = frame.scaled_size(0.75);
        let rect = [100., 200., w, h];
        assert!(contains(rect, [108., 222.]));
        assert!(!contains(rect, [109., 222.]));
        assert!(!contains(rect, [108., 222.5]));
        let mut batch = sprites::Batch::default();
        let _ = land::draw_type_icon(
            &atlas,
            &mut batch,
            &Child {
                sprite: icon.sprite.clone(),
                at: [0.; 2],
                scale: 0.75,
                frame: 1,
                rotation: 0.,
                centered: false,
                order: 0,
            },
            [100., 200.],
            Some(&icon),
            Some([1.; 4]),
        );
        assert_eq!(batch.quads[0].rect, rect);
        assert_eq!(batch.quads[0].uv, [0., 0.5, 1., 1.]);
        let fractional = Frame {
            texture: [12., 31.],
            grid: [1, 3],
            ..frame
        };
        assert_eq!(fractional.scaled_size(0.7)[1].to_bits(), 0x40e7_7777);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bar_hover_uses_authored_placement_at_180_degrees() {
        content_reader_script::test_game::enter();
        let gui = br#"guiTypes = {
            containerWindowType = {
                name = "naval_combat_mapicon"
                size = { width = 10 height = 10 }
            }
            containerWindowType = {
                name = "unit_counter_navy"
                size = { width = 10 height = 10 }
            }
            containerWindowType = {
                name = "unit_counter"
                size = { width = 76 height = 24 }
                iconType = { name = "bar_org" spriteType = "GFX_synthetic_overlay" position = { x = 14 y = 8 } rotation = 3.1415927 Orientation = "CENTER_LEFT" }
                iconType = { name = "bar_str" spriteType = "GFX_synthetic_overlay" position = { x = 14 y = 12 } Orientation = "CENTER_RIGHT" }
            }
        }"#;
        let guis = native::Guis::new(vec![(
            "test.gui".into(),
            native::read_at(gui, viewer_native_gui::test_support::VIEWPORT),
        )]);
        viewer_counter_kit::fixtures::with_counter_inputs_fixture(
            guis,
            |assets| {
                assets
                    .sheets
                    .get_mut("GFX_synthetic_overlay")
                    .unwrap()
                    .size = [17, 3];
            },
            |_| {},
            |inputs| {
                let sheet = inputs.assets.sheet("GFX_synthetic_overlay").unwrap();
                let child = Child {
                    sprite: "GFX_synthetic_overlay".into(),
                    at: [14.0, 8.0],
                    scale: 1.0,
                    frame: 0,
                    rotation: std::f32::consts::PI,
                    centered: false,
                    order: 0,
                };
                let mut batch = sprites::Batch::default();
                batch.at_layer(0);
                draw_bar(&mut batch, sheet, &child, [100.0, 200.0], 0.5);
                let second = Child {
                    at: [14.0, 12.0],
                    rotation: 0.0,
                    order: 1,
                    ..child
                };
                batch.at_layer(1);
                draw_bar(&mut batch, sheet, &second, [100.0, 200.0], 0.5);
                let stack = land::Stack {
                    selected: false,
                    owner: 0,
                    logical: Some(0),
                    plate: Ok([1.0; 4]),
                    flag_owner: 0,
                    coarse: false,
                    type_tint: None,
                    hq: false,
                    portrait: None,
                    hidden: false,
                    hit_points: Vec::new(),
                    organisation: Vec::new(),
                    bars: true,
                    status_recorded: true,
                    members: Vec::new(),
                    divisions: Arc::from([]),
                    timing: Arc::from([]),
                    template: 0,
                    alternate_template: 0,
                    disclosure: Ok(false),
                    grouping_unknown: None,
                    moving: None,
                    action: Ok(0),
                    marker: Ok(None),
                    leader_en_route: None,
                    defending_area: false,
                    gun: None,
                    abilities: None,
                };
                let out = presented(&inputs, &stack, false, [100.0, 200.0], &batch.quads);
                assert!(out.refused.is_empty(), "{:?}", out.refused);
                assert_eq!(out.controls.len(), 2);
                assert_eq!(out.controls[0].1, [114.0, 208.0, 17.0, 3.0]);
                assert_eq!(out.controls[1].1, [114.0, 212.0, 17.0, 3.0]);
            },
        );
    }

    #[test]
    fn bar_hover_shares_bar_step_origin_for_vertical_bars_at_half_fill() {
        content_reader_script::test_game::enter();
        let gui = br#"guiTypes = {
            containerWindowType = {
                name = "naval_combat_mapicon"
                size = { width = 10 height = 10 }
            }
            containerWindowType = {
                name = "unit_counter_navy"
                size = { width = 10 height = 10 }
            }
            containerWindowType = {
                name = "unit_counter"
                size = { width = 76 height = 30 }
                iconType = { name = "bar_org" spriteType = "GFX_synthetic_overlay" position = { x = 14 y = 8 } }
                iconType = { name = "bar_str" spriteType = "GFX_synthetic_overlay" position = { x = 14 y = 12 } }
            }
        }"#;
        let guis = native::Guis::new(vec![(
            "test.gui".into(),
            native::read_at(gui, viewer_native_gui::test_support::VIEWPORT),
        )]);
        viewer_counter_kit::fixtures::with_counter_inputs_fixture(
            guis,
            |assets| {
                assets
                    .sheets
                    .get_mut("GFX_synthetic_overlay")
                    .unwrap()
                    .size = [3, 17];
            },
            |_| {},
            |inputs| {
                let sheet = inputs.assets.sheet("GFX_synthetic_overlay").unwrap();
                let org = Child {
                    sprite: "GFX_synthetic_overlay".into(),
                    at: [14.0, 8.0],
                    scale: 1.0,
                    frame: 0,
                    rotation: 0.0,
                    centered: false,
                    order: 0,
                };
                let str_bar = Child {
                    at: [14.0, 12.0],
                    order: 1,
                    ..org.clone()
                };
                let mut batch = sprites::Batch::default();
                batch.at_layer(0);
                draw_bar(&mut batch, sheet, &org, [0.0, 0.0], 0.5);
                batch.at_layer(1);
                draw_bar(&mut batch, sheet, &str_bar, [0.0, 0.0], 0.5);
                let stack = land::Stack {
                    selected: false,
                    owner: 0,
                    logical: Some(0),
                    plate: Ok([1.0; 4]),
                    flag_owner: 0,
                    coarse: false,
                    type_tint: None,
                    hq: false,
                    portrait: None,
                    hidden: false,
                    hit_points: vec![50_000],
                    organisation: vec![50_000],
                    bars: true,
                    status_recorded: true,
                    members: Vec::new(),
                    divisions: Arc::from([]),
                    timing: Arc::from([]),
                    template: 0,
                    alternate_template: 0,
                    disclosure: Ok(false),
                    grouping_unknown: None,
                    moving: None,
                    action: Ok(0),
                    marker: Ok(None),
                    leader_en_route: None,
                    defending_area: false,
                    gun: None,
                    abilities: None,
                };
                let out = presented(&inputs, &stack, false, [0.0, 0.0], &batch.quads);
                assert!(out.refused.is_empty(), "{:?}", out.refused);
                assert_eq!(out.controls.len(), 2);
                assert_eq!(out.controls[0].1, [18.0, 8.0, 3.0, 17.0]);
                assert!(contains(out.controls[0].1, [19.0, 15.0]));
                assert!(!contains(out.controls[0].1, [15.0, 15.0]));
                assert_eq!(out.controls[1].1, [18.0, 12.0, 3.0, 17.0]);
                assert!(contains(out.controls[1].1, [19.0, 20.0]));
            },
        );
    }
}
