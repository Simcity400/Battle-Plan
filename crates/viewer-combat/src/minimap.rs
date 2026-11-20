//! The game's minimap (`mini_map_interface`), hosted through the same retained native GUI walk
//! as the picker.
//!
//! - `worldmap_bg` is a static picture of the world; a press on it moves the camera's target
//!   there, and so does the pointer while the press is held. The wheel over it zooms about the
//!   target ([`viewer_foundation::map::camera::motion::Motion::zoom_step`]).
//! - The `camera_quad_*` lines outline the camera's ground footprint, with wrap copies a map
//!   width over, inside `border_container`, which clips them to the picture.
//! - The handle (and its shortcut) slides the window by `MINIMAP_TOGGLE_SHIFT` over
//!   `MINIMAP_TOGGLE_DURATION`.
//! - The ping buttons arm ping mode, as Ctrl and Alt held do ([`ping_mode`]); a ping placed
//!   in it is a game command, which the replay never posts. The recording holds no pings, so
//!   the pinged handle and the ping circle never show, and the window names that refusal.

use super::{
    Drawn, HoverText, Values, Widget,
    binding::Bindings,
    gui::{self, Retained},
    gui_resources,
    picker::{Control as PickerControl, Placed},
};
use std::{collections::BTreeMap, io, sync::Arc};
use viewer_foundation::map::native_gui::{self, Resources, lifecycle, pointer::Input};


pub const WINDOW: &str = "mini_map_interface";
pub const TEMPLATES: &[&[u8]] = &[b"mini_map_interface"];

const WORLD: &str = "worldmap_bg";
const BORDER: &str = "border_container";
const WORLD_SPRITE: &[u8] = b"GFX_minimap_world";

/// Why the recorded pings are withheld.
pub const PINGS_REFUSED: &str = "the recording holds no pings, so the pinged handle and the ping      circle never show";

/// An executable constant: the game sets every minimap tooltip's anchor in code, 90 pixels above
/// the hovered control's top-left.
pub const TOOLTIP_ANCHOR: super::TooltipAnchor =
    super::TooltipAnchor::Authored(installed_content_core::gui::native::TooltipAnchor {
        set: true,
        x: 0,
        y: -90,
        relative: true,
        below: true,
    });

/// Which ping button is armed: the offensive one, the defensive one, or none.
pub type Armed = Option<bool>;

/// Whether the minimap is in ping mode, and if so whether the ping is offensive: a ping button
/// armed, or Ctrl and Alt held. Shift makes a held-key ping offensive, and an armed defensive
/// ping too.
pub fn ping_mode(armed: Armed, ctrl: bool, alt: bool, shift: bool) -> Option<bool> {
    let keys = ctrl && alt;
    if !keys && armed.is_none() {
        return None;
    }
    Some(armed == Some(true) || shift)
}

/// A control of the minimap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    /// The world picture.
    World,
    Handle,
    OffensivePing,
    DefensivePing,
}

impl Control {
    pub fn name(self) -> &'static str {
        match self {
            Control::World => "world",
            Control::Handle => "handle",
            Control::OffensivePing => "offensive_ping",
            Control::DefensivePing => "defensive_ping",
        }
    }
}

/// `NInterface.MINIMAP_TOGGLE_DURATION` and `MINIMAP_TOGGLE_SHIFT`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Defines {
    pub duration: f32,
    pub shift: i32,
}

impl Defines {
    pub fn load(defines: &installed_content_core::defines::Defines) -> io::Result<Self> {
        let duration = defines.number("NInterface", "MINIMAP_TOGGLE_DURATION")? as f32;
        let shift = defines.number("NInterface", "MINIMAP_TOGGLE_SHIFT")?;
        if !(duration.is_finite() && duration > 0.0) {
            return Err(super::invalid(
                "NDefines.NInterface.MINIMAP_TOGGLE_DURATION is not a positive duration",
            ));
        }
        Ok(Self {
            duration,
            shift: shift as i32,
        })
    }
}

/// The slide's ease: `(1 - (1 + 4t)^-2) / (1 - 5^-2)`, 0 at the start and 1 at the end.
pub fn ease(t: f32) -> f32 {
    (1.0 - (t * 4.0 + 1.0).powf(-2.0)) * (1.0 / (1.0 - 5.0f32.powf(-2.0)))
}

/// Whether the minimap is open, and the time its slide started.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slide {
    open: bool,
    start: Option<f32>,
}

impl Default for Slide {
    fn default() -> Self {
        Self {
            open: true,
            start: None,
        }
    }
}

impl Slide {
    pub fn open(&self) -> bool {
        self.open
    }

    /// Flips open and closed at `now`. A slide turned mid-way starts again from where it stands,
    /// its clock set back so the ease continues from the mirrored progress.
    pub fn toggle(&mut self, now: f32, duration: f32) {
        let progress = match self.start {
            None => 0.0,
            Some(start) => 1.0 - ease(((now - start) / duration).clamp(0.0, 1.0)),
        };
        self.open = !self.open;
        let progress = progress.clamp(0.0, 1.0);
        let left = (1.0 - progress / (1.0 / (1.0 - 5.0f32.powf(-2.0)))).sqrt();
        self.start = Some(now - (1.0 / left - 1.0) * 0.25 * duration);
    }

    /// How far right of its authored place the window stands at `now`, ending a slide that has
    /// run its course.
    pub fn offset(&mut self, now: f32, defines: Defines) -> i32 {
        let shift = defines.shift;
        let Some(start) = self.start else {
            return if self.open { 0 } else { shift };
        };
        let t = (now - start) / defines.duration;
        if t >= 1.0 {
            self.start = None;
            return if self.open { 0 } else { shift };
        }
        let f = ease(t);
        let moved = if self.open {
            (1.0 - f) * shift as f32
        } else {
            shift as f32 * f
        };
        moved as i32
    }

    pub fn sliding(&self) -> bool {
        self.start.is_some()
    }
}

/// One outline line: its centre in the window and its length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Line {
    pub at: [i32; 2],
    pub length: i32,
}

/// The camera outline: four lines and the wrap copies of the far vertical line and both
/// horizontal ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outline {
    pub left: Line,
    pub right: Line,
    pub top: Line,
    pub bottom: Line,
    pub wrap: Line,
    pub wrap_top: Line,
    pub wrap_bottom: Line,
}

impl Outline {
    fn lines(&self) -> [(&'static str, Line); 7] {
        [
            ("camera_quad_left", self.left),
            ("camera_quad_right", self.right),
            ("camera_quad_top", self.top),
            ("camera_quad_bottom", self.bottom),
            ("camera_quad_wrap", self.wrap),
            ("camera_quad_wrap_top", self.wrap_top),
            ("camera_quad_wrap_bottom", self.wrap_bottom),
        ]
    }
}

/// The outline of a camera whose ground `footprint` (top-left, top-right, bottom-left,
/// bottom-right and middle, as world `(x, z)`) is drawn over a picture at `image` (`x, y, w,
/// h` in the window) of a `map` sized world. `None` for a footprint less than a tenth of a unit
/// deep, for which the game leaves the outline as it was.
pub fn outline(footprint: [[f32; 2]; 5], map: [i32; 2], image: [i32; 4]) -> Option<Outline> {
    let [top_left, top_right, bottom_left, bottom_right, middle] = footprint;
    let top = (top_left[1] + top_right[1]) * 0.5;
    let bottom = (bottom_left[1] + bottom_right[1]) * 0.5;
    let left = (bottom_left[0] + top_left[0]) * 0.5;
    let right = (bottom_right[0] + top_right[0]) * 0.5;
    let depth = top - bottom;
    if depth.abs() < 0.1 {
        return None;
    }
    let [map_w, map_h] = map.map(|side| side as f32);
    let [_, y, w, h] = image;
    let [xf, yf, wf, hf] = image.map(|value| value as f32);
    let px = |world: f32| (world / map_w * wf + xf).round() as i32;
    let py = |world: f32| ((1.0 - world / map_h) * hf + yf).round() as i32;
    let mut x0 = px(left);
    let middle_y = py(middle[1]);
    let mut x1 = (px(right) as f32 - 1.0) as i32;
    let middle_x = px(middle[0]);
    let mut y0 = py(top);
    let mut y1 = (py(bottom) as f32 - 1.0) as i32;
    let least_width = ((right - left) * 6.0 / depth) as i32;
    if y1 - y0 < 6 {
        let grow = (y0 - y1) + 6;
        y0 -= grow / 2;
        y1 = (y1 - grow / 2) + grow;
    }
    if x1 - x0 < least_width {
        let grow = (x0 - x1) + least_width;
        x0 -= grow / 2;
        x1 = (x1 - grow / 2) + grow;
    }
    let shown_top = y0.max(y);
    let shown_bottom = y1.min(h + y - 2);
    let height = y1 - y0;
    let width = x1 - x0;
    let vertical = |x: i32| Line {
        at: [x, middle_y],
        length: height,
    };
    let horizontal = |x: i32, y: i32| Line {
        at: [x, y],
        length: width,
    };
    let (wrap, shift) = if left >= 0.0 {
        (vertical(x1 - w), -w)
    } else {
        (vertical(x0 + w), w)
    };
    Some(Outline {
        left: vertical(x0),
        right: vertical(x1),
        top: horizontal(middle_x, shown_top),
        bottom: horizontal(middle_x, shown_bottom),
        wrap,
        wrap_top: horizontal(middle_x + shift, shown_top),
        wrap_bottom: horizontal(middle_x + shift, shown_bottom),
    })
}

/// The handle's first authored shortcut, which toggles the minimap.
pub fn handle_shortcut(content: &super::content::Content, extent: [f32; 2]) -> io::Result<Option<String>> {
    let windows = content.window_cache.at(extent.map(|value| value as u32))?;
    let root = gui::template(&windows, WINDOW).map_err(super::invalid)?;
    Ok(root
        .path_named("handle")
        .and_then(|handle| handle.button()?.shortcuts.first().cloned())
        .and_then(|shortcut| String::from_utf8(shortcut).ok()))
}

/// The clip of the outline's container: the world picture at `image` (`x, y, w, h` in the
/// window), as left, top, right and bottom.
fn picture_clip([x, y, w, h]: [i32; 4]) -> [i32; 4] {
    [x, y, x + w, y + h]
}

/// The world `(x, z)` the pointer at interface point `pointer` stands for over a picture at
/// `image` (`x, y, w, h` on screen): across the picture wraps round the world, down it is held
/// within the map's rows.
pub fn world_point(pointer: [f32; 2], image: [i32; 4], map: [i32; 2]) -> [f32; 2] {
    let [x, y, w, h] = image.map(|value| value as f32);
    let mut u = ((pointer[0] as i32) as f32 - x) / w;
    let v = 1.0 - ((pointer[1] as i32) as f32 - y) / h;
    while u < 0.0 {
        u += 1.0;
    }
    while u > 1.0 {
        u -= 1.0;
    }
    let v = v.clamp(0.0, 1.0);
    [map[0] as f32 * u, map[1] as f32 * v]
}

/// What the minimap shows this frame.
pub struct State<'a> {
    /// The camera's ground footprint; `None` keeps the outline last drawn.
    pub footprint: Option<[[f32; 2]; 5]>,
    pub map: [i32; 2],
    /// How far right of its authored place the window stands.
    pub slide: i32,
    /// Each tooltip's text; `None` where the localisation refused it.
    pub handle_tooltip: Option<&'a str>,
    pub offensive_tooltip: [Option<&'a str>; 2],
    pub defensive_tooltip: [Option<&'a str>; 2],
    /// Ping mode, and whether its ping is offensive.
    pub ping: Option<bool>,
}

/// One frame of the minimap: its quads and tooltips, its controls in paint order, and where its
/// world picture landed on screen.
#[derive(Clone, Default)]
pub struct MinimapDrawn {
    pub controls: Vec<Placed>,
    pub clicked: Vec<(PickerControl, usize)>,
    /// The world picture, `x, y, w, h` in interface pixels.
    pub image: Option<[i32; 4]>,
}

/// The minimap's retained window; a new content or viewport rebuilds it.
#[derive(Default)]
pub struct Minimap {
    built: Option<(
        [i32; 2],
        viewer_foundation::map::native_windows::NativeWindows,
    )>,
    window: Option<Retained>,
    generation: u64,
    last: Option<(Option<Outline>, i32)>,
    outline: Option<Outline>,
    controls: BTreeMap<String, Control>,
}

fn hover(text: Option<&str>) -> Option<HoverText> {
    text.filter(|text| !text.is_empty()).map(Arc::from)
}

impl Minimap {
    pub fn holding(&self) -> bool {
        self.window.as_ref().is_some_and(Retained::holding)
    }

    pub fn pointing(&self) -> bool {
        self.window.as_ref().is_some_and(Retained::pointing)
    }

    /// Lays out and draws the minimap for `state`.
    #[allow(clippy::too_many_arguments, reason = "one frame's view, pointer and clock, as the picker's draw takes them")]
    pub fn draw(
        &mut self,
        content: &super::content::Content,
        state: &State<'_>,
        extent: [f32; 2],
        hovers: bool,
        time: f32,
        advance: bool,
        input: Option<Input>,
        drawn: &mut Drawn,
        refused: &mut BTreeMap<String, usize>,
    ) -> io::Result<MinimapDrawn> {
        let viewport = extent.map(|value| value as i32);
        let windows = content.window_cache.at(extent.map(|value| value as u32))?;
        if self
            .built
            .as_ref()
            .is_none_or(|(size, built)| *size != viewport || !built.same(&windows))
        {
            *self = Self {
                built: Some((viewport, (*windows).clone())),
                outline: self.outline,
                ..Self::default()
            };
        }
        let assets = content.assets.as_ref();
        let inventory = gui_resources::GuiAssets::new(assets, &Bindings::default());
        let resources = gui_resources::GuiResources::new(
            assets,
            &windows,
            &content.localisation,
            &content.small_flags,
            &content.large_flags,
            &inventory,
        );
        let mut out = MinimapDrawn::default();
        refused.insert(PINGS_REFUSED.to_owned(), 1);
        if let Some(why) = resources
            .sprite(WORLD_SPRITE)
            .map_or(Some("is not declared".to_owned()), |info| info.missing)
        {
            refused.insert(
                format!(
                    "the world picture GFX_minimap_world, art from the current \
                     installation, {why}; the minimap shows its frame without it"
                ),
                1,
            );
        }
        let root = gui::template(&windows, WINDOW).map_err(super::invalid)?;
        if self.controls.is_empty() {
            self.controls = gui::elements(root)
                .into_iter()
                .filter_map(|(path, element)| {
                    let control = match path.rsplit('/').next().unwrap_or(&path) {
                        "handle" => Control::Handle,
                        "offensive_ping" => Control::OffensivePing,
                        "defensive_ping" => Control::DefensivePing,
                        _ => return None,
                    };
                    Some((element.path.clone(), control))
                })
                .collect();
        }
        let retained = gui::ensure(&mut self.window, WINDOW, root, viewport, &resources)
            .map_err(super::invalid)?;
        let frame = native_gui::container_frame(root, Some(viewport)).map_err(super::invalid)?;
        let origin = [frame.at[0] + state.slide, frame.at[1]];
        retained
            .instance
            .set_position(root, root, origin)
            .map_err(super::invalid)?;
        let world = root
            .path_named(WORLD)
            .ok_or_else(|| super::invalid(format!("{WINDOW} has no {WORLD}")))?;
        let picture = world
            .icon()
            .map(|icon| icon.position.map(|value| value as i32))
            .unwrap_or_default();
        let size = retained
            .instance
            .drawable_extent(root, world, &resources)
            .map_err(|why| super::invalid(format!("{WORLD}: {why:?}")))?;
        let image = size.map(|size| [picture[0], picture[1], size[0], size[1]]);
        if let Some(image) = image {
            if let Some(footprint) = state.footprint
                && let Some(placed) = outline(footprint, state.map, image)
            {
                self.outline = Some(placed);
            }
            if let Some(border) = root.path_named(BORDER)
                && let Err(why) =
                    retained
                        .instance
                        .set_size(root, border, [image[2], image[3]], &resources)
            {
                viewer_foundation::map::gui_resources::count_refused(
                    refused,
                    BORDER,
                    &why,
                );
            }
            // As the game does each frame, the outline's container clips to the world picture, so
            // the lines and their seam copies show only on it.
            if let Some(border) = root.path_named(BORDER) {
                match retained.instance.control(root, border) {
                    Ok(bound) => bound.clip = Some(picture_clip(image)),
                    Err(why) => {
                        refused.insert(format!("{BORDER}: {why}"), 1);
                    }
                }
            }
        }
        let shown = (self.outline, state.slide);
        if self.last != Some(shown) {
            self.last = Some(shown);
            self.generation += 1;
        }
        let values = values(state, self.outline.as_ref());
        let tick = lifecycle::Tick { time, dt: 0.0 };
        let Some(prepared) = retained.prepare(
            root,
            &values,
            Vec::new(),
            &windows,
            &resources,
            tick,
            self.generation,
            advance,
            hovers,
            true,
            input
                .as_ref()
                .map(|input| input.local(origin.map(|value| value as f32))),
            refused,
        ) else {
            return Ok(out);
        };
        drawn.sounds.extend(prepared.sounds.iter().cloned());
        out.clicked.extend(
            prepared
                .clicks
                .iter()
                .filter(|(key, _, _)| key.is_none())
                .filter_map(|(_, path, order)| {
                    Some((PickerControl::Minimap(*self.controls.get(path)?), *order))
                }),
        );
        if !gui::draw_prepared(
            &prepared.layout,
            &prepared.items,
            origin,
            &resources,
            drawn,
        ) {
            return Ok(out);
        }
        if let Some([x, y, w, h]) = image {
            let screen = [origin[0] + x, origin[1] + y, w, h];
            out.image = Some(screen);
            out.controls.push(Placed::new(
                screen.map(|value| value as f32),
                PickerControl::Minimap(Control::World),
            ));
        }
        for region in &prepared.layout.regions {
            let Some(rect) = gui::region_rect(region, origin) else {
                continue;
            };
            if let Some(control) = self.controls.get(&region.path) {
                let click = gui::click_sound(root, &region.path, &resources);
                out.controls.extend(Placed::region(
                    rect,
                    PickerControl::Minimap(*control),
                    region,
                    origin,
                    click,
                ));
            }
        }
        if hovers {
            gui::append_hovers(&mut drawn.tooltips, prepared.hovers, origin);
        }
        Ok(out)
    }

    /// Whether a window of the minimap draws differently next frame without a call from the code.
    pub fn active(&self) -> bool {
        self.window.as_ref().is_some_and(Retained::active)
    }
}

/// The bindings: the outline lines, the plain handle, and the withheld ping controls.
fn values(state: &State<'_>, outline: Option<&Outline>) -> Values {
    let mut values = Values::new();
    let hidden = Widget {
        hidden: true,
        ..Default::default()
    };
    values.insert("handle_pinged".to_owned(), hidden.clone());
    values.insert(format!("{BORDER}/circle"), hidden.clone());
    values.insert(
        "handle".to_owned(),
        Widget {
            tooltip: hover(state.handle_tooltip),
            hover_owned: true,
            tooltip_anchor: TOOLTIP_ANCHOR,
            ..Default::default()
        },
    );
    for (name, [title, description], offensive) in [
        ("offensive_ping", state.offensive_tooltip, true),
        ("defensive_ping", state.defensive_tooltip, false),
    ] {
        values.insert(
            name.to_owned(),
            Widget {
                // The armed button shows its second frame.
                frame: Some(u32::from(state.ping == Some(offensive))),
                tooltip: hover(title),
                tooltip_description: hover(description),
                hover_owned: true,
                tooltip_anchor: TOOLTIP_ANCHOR,
                ..Default::default()
            },
        );
    }
    // Each line's stretch is half its length, as the game sets it on its two-pixel sprite.
    for (name, line) in outline
        .map(Outline::lines)
        .into_iter()
        .flatten()
    {
        values.insert(
            format!("{BORDER}/{name}"),
            Widget {
                offset: line.at.map(|value| value as f32),
                stretch: Some(line.length as f32 * 0.5),
                ..Default::default()
            },
        );
    }
    if outline.is_none() {
        for name in [
            "camera_quad_left",
            "camera_quad_right",
            "camera_quad_top",
            "camera_quad_bottom",
            "camera_quad_wrap",
            "camera_quad_wrap_top",
            "camera_quad_wrap_bottom",
        ] {
            values.insert(format!("{BORDER}/{name}"), hidden.clone());
        }
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A footprint of the box `left..right`, `bottom..top`, centred.
    fn footprint(left: f32, right: f32, bottom: f32, top: f32) -> [[f32; 2]; 5] {
        [
            [left, top],
            [right, top],
            [left, bottom],
            [right, bottom],
            [(left + right) * 0.5, (top + bottom) * 0.5],
        ]
    }

    const MAP: [i32; 2] = [5632, 2048];
    const IMAGE: [i32; 4] = [0, 0, 268, 97];

    #[test]
    fn a_centred_camera_gives_a_symmetric_outline_north_up() {
        let drawn = outline(footprint(2016.0, 3616.0, 512.0, 1536.0), MAP, IMAGE).unwrap();
        // x: 2016 / 5632 * 268 = 95.9, 3616 / 5632 * 268 = 172.1; y flips: z 1536 is row 24.
        // The middle row 48.5 rounds half away from zero.
        assert_eq!(drawn.left.at, [96, 49]);
        assert_eq!(drawn.right.at, [171, 49]);
        assert_eq!(drawn.top.at, [134, 24]);
        assert_eq!(drawn.bottom.at, [134, 72]);
        assert_eq!(drawn.left.length, drawn.right.length);
        assert_eq!(drawn.top.length, 75);
        assert_eq!(drawn.left.length, 48);
    }

    #[test]
    fn a_tiny_footprint_grows_to_the_least_height_and_its_aspect_width() {
        let drawn = outline(footprint(2800.0, 2810.0, 1000.0, 1005.0), MAP, IMAGE).unwrap();
        assert_eq!(drawn.left.length, 6);
        // Six pixels times the footprint's width over its depth.
        assert_eq!(drawn.top.length, 12);
        assert!(outline(footprint(2800.0, 2810.0, 1000.0, 1000.05), MAP, IMAGE).is_none());
    }

    #[test]
    fn the_wrap_copies_stand_a_picture_width_over() {
        let east = outline(footprint(5000.0, 6000.0, 512.0, 1536.0), MAP, IMAGE).unwrap();
        assert_eq!(east.wrap.at[0], east.right.at[0] - 268);
        assert_eq!(east.wrap_top.at[0], east.top.at[0] - 268);
        let west = outline(footprint(-400.0, 600.0, 512.0, 1536.0), MAP, IMAGE).unwrap();
        assert_eq!(west.wrap.at[0], west.left.at[0] + 268);
        assert_eq!(west.wrap_bottom.at[0], west.bottom.at[0] + 268);
    }

    #[test]
    fn the_outline_is_clipped_to_the_world_picture() {
        assert_eq!(picture_clip([4, 6, 268, 97]), [4, 6, 272, 103]);
    }

    #[test]
    fn the_pointer_wraps_across_and_holds_down_the_picture() {
        let image = [100, 50, 268, 97];
        assert_eq!(world_point([234.0, 98.5], image, MAP)[0], 5632.0 * 0.5);
        // Off the left edge wraps round to the east.
        let wrapped = world_point([90.0, 60.0], image, MAP);
        assert!((wrapped[0] - 5632.0 * (1.0 - 10.0 / 268.0)).abs() < 0.01);
        // Above the picture holds at the north edge, below at the south.
        assert_eq!(world_point([150.0, 0.0], image, MAP)[1], 2048.0);
        assert_eq!(world_point([150.0, 400.0], image, MAP)[1], 0.0);
        // Down the picture runs north to south.
        assert_eq!(world_point([150.0, 50.0], image, MAP)[1], 2048.0);
    }

    #[test]
    fn every_minimap_tooltip_takes_the_code_set_anchor() {
        let state = State {
            footprint: None,
            map: MAP,
            slide: 0,
            handle_tooltip: Some("handle"),
            offensive_tooltip: [Some("offensive"), Some("ping")],
            defensive_tooltip: [Some("defensive"), None],
            ping: None,
        };
        let values = values(&state, None);
        for name in ["handle", "offensive_ping", "defensive_ping"] {
            assert_eq!(values[name].tooltip_anchor, TOOLTIP_ANCHOR, "{name}");
        }
        assert_eq!(values["offensive_ping"].tooltip_description.as_deref(), Some("ping"));
        assert_eq!(values["defensive_ping"].tooltip_description, None, "a refused string shows nothing");
    }

    #[test]
    fn ping_mode_follows_the_armed_button_and_the_held_keys() {
        assert_eq!(ping_mode(None, false, false, true), None);
        assert_eq!(ping_mode(None, true, false, false), None);
        assert_eq!(ping_mode(Some(true), false, false, false), Some(true));
        assert_eq!(ping_mode(Some(false), false, false, false), Some(false));
        // Shift turns an armed defensive ping offensive, as the game reads its keys.
        assert_eq!(ping_mode(Some(false), false, false, true), Some(true));
        assert_eq!(ping_mode(None, true, true, false), Some(false));
        assert_eq!(ping_mode(None, true, true, true), Some(true));
        assert_eq!(ping_mode(Some(true), true, true, false), Some(true));
    }

    #[test]
    fn the_slide_eases_out_and_turns_back_from_where_it_stands() {
        let defines = Defines {
            duration: 0.5,
            shift: 270,
        };
        assert_eq!(ease(0.0), 0.0);
        assert!((ease(1.0) - 1.0).abs() < 1e-6);
        let mut slide = Slide::default();
        assert_eq!(slide.offset(10.0, defines), 0);
        slide.toggle(10.0, defines.duration);
        assert!(!slide.open());
        assert_eq!(slide.offset(10.0, defines), 0);
        let half = slide.offset(10.25, defines);
        assert_eq!(half, (270.0 * ease(0.5)) as i32);
        // Turned at the half way, it opens from where it stood.
        slide.toggle(10.25, defines.duration);
        assert!(slide.open());
        assert!((slide.offset(10.25, defines) - half).abs() <= 1);
        assert_eq!(slide.offset(11.0, defines), 0);
        assert!(!slide.sliding());
        slide.toggle(20.0, defines.duration);
        assert_eq!(slide.offset(21.0, defines), 270);
    }
}
