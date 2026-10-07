//! Tooltip observers from the children actually presented by the land batch.

use super::*;

#[derive(Default)]
pub(super) struct Sprite {
    pub(super) origin: [f32; 2],
    pub(super) batch: Batch,
    pub(super) presented: Presented,
}

#[derive(Default)]
pub(super) struct Presented {
    pub(super) controls: Vec<(CounterControl, [f32; 4])>,
    pub(super) refused: Vec<String>,
}

/// Before `rank_children` replaces attachment indices with renderer layers,
/// retain only presented controls. The sprite retains its observers through
/// redraw caching. Normal siblings participate in the same reverse hit order.
pub(super) fn presented(
    inputs: &CounterInputs<'_>,
    stack: &land::Stack,
    coarse: bool,
    origin: [f32; 2],
    quads: &[sprites::Quad],
) -> Presented {
    let railway_gun = stack.gun.is_some();
    let window = if railway_gun { GUN_WINDOW } else { LAND_WINDOW };
    let Some(root) = inputs.windows.guis.template(window) else {
        return Presented::default();
    };
    let mut bounds = Vec::<(u64, [f32; 4])>::new();
    for quad in quads {
        let rect = quad.rect;
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
            // CIconType::object_factory@142358760 -> CIcon+154 bit5;
            // input_update@1422E6E90 skips CButton's observer dispatch.
            continue;
        }
        if element.is_named("active_abilities") {
            ability_controls(inputs, stack, origin, root, quads, order, &mut out);
            continue;
        }
        let control = control(element, railway_gun);
        if let Some(text) = element.text_box() {
            // HitTest@1422D8240 uses the authored +220/+224 layout, not ink.
            // The existing TextChild has 1422D7FD0's parent-orientation anchor;
            // native integer point comparisons exclude both lower edges.
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
            if let Some(icon) = element.icon()
                && icon.rotation != 0.0
            {
                // CSpriteType::ContainsPoint@14222C220 tests the unturned frame.
                // Undo the admitted bars' draw placement before using that frame.
                if element.is_named("bar_org") || element.is_named("bar_str") {
                    rect[1] -= (frame.drawn[1] - frame.drawn[0]) * scale * 0.5;
                } else {
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
            out.controls.push((control, rect));
        }
    }
    out
}

/// The selected drawable's metadata, not an arbitrary sprite sharing its texture.
struct Frame {
    hit: crate::map::native_gui::Hit,
    texture: [f32; 2],
    frames: u32,
    drawn: [f32; 2],
}

impl Frame {
    fn scaled_size(&self, scale: f32) -> [f32; 2] {
        // ContainsPoint@14222C220 multiplies the complete size by the scale,
        // then the reciprocal frame count, all in native float precision.
        [
            self.texture[0] * scale * (1.0 / self.frames as f32),
            self.texture[1] * scale,
        ]
    }
}

fn sprite_hit(
    inputs: &CounterInputs<'_>,
    name: &[u8],
) -> Result<crate::map::native_gui::Hit, &'static str> {
    use crate::map::native_gui::Hit;
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
        frames: sheet.frames.max(1),
        drawn: sheet.frame_size(),
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
        let name = inputs.assets.types.entries().find_map(|((name, _), cell)| {
            (inputs.assets.types.texture == quad.texture
                && inputs.assets.types.uv(*cell) == quad.uv)
                .then_some(name)
        });
        let name = name.ok_or("hover type sprite identity unavailable")?;
        let (texture, frames) = inputs
            .assets
            .types
            .source_dimensions(name)
            .ok_or("hover type source dimensions unavailable")?;
        return Ok(Frame {
            hit: sprite_hit(inputs, name.as_bytes())?,
            texture: texture.map(|value| value as f32),
            frames,
            drawn: inputs.assets.types.frame_size.map(|value| value as f32),
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
    // These are the same draw bindings used by draw_land_children/draw_gun_children.
    // HQ backgrounds and action sheets can differ from the authored placeholder.
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
            .or_else(|| crate::map::native_gui::Sheets::sheet(&inputs.assets.gui, authored))
    }
    .ok_or("hover sprite drawable unavailable")?;
    if sheet.texture != quad.texture {
        return Err("hover sprite binding differs from drawable");
    }
    sheet_frame(inputs, sheet)
}

fn accept_hit(
    hit: crate::map::native_gui::Hit,
    window: &str,
    name: &str,
    refused: &mut Vec<String>,
) -> bool {
    use crate::map::native_gui::Hit;
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
        let frame = crate::map::native_gui::container_frame(root, None)?;
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
    let item_frame = crate::map::native_gui::container_frame(item, None);
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
